// ── SSH command parser ──────────────────────────────────────────
//
// When the user types `ssh user@host` (or a nested ssh inside an
// already-SSH session), we want the right sidebar to switch context
// to that target so the Server Monitor panel can reflect what they
// are actually connecting to. This module turns one line of shell
// input into the addressing bits we need (host / user / port / key).
//
// The parser is intentionally conservative — it only fires for
// cleanly-formed `ssh ...` invocations. Compound commands
// (`cd /tmp && ssh ...`, `for h in a b; do ssh $h; done`) and
// pipes are skipped: we'd rather no-op than misroute the right
// panel for an ambiguous input.

export type ParsedSshTarget = {
  host: string;
  /** Defaults to the OS user when the command omits `user@`. We
   *  return `""` in that case so the caller can decide whether to
   *  fall back to a saved-connection match or leave it blank. */
  user: string;
  /** `0` when the command did not pass `-p`. The caller normalizes
   *  to 22 if it needs an outbound port. */
  port: number;
  /** Path to an explicit `-i` identity, if one was given. */
  identityPath: string;
};

/**
 * Tokenize a single shell line into argv-style words. Honors single
 * and double quotes plus backslash escapes — enough to reconstruct
 * the kind of `ssh -p 2222 'user'@host -i "~/.ssh/id"` invocation a
 * developer would type at the prompt. Returns `null` on a quoting
 * error so the caller can decide to skip parsing rather than work
 * with garbage.
 */
function tokenize(line: string): string[] | null {
  const tokens: string[] = [];
  let current = "";
  let quote: '"' | "'" | null = null;
  let escaped = false;
  let any = false;

  for (const ch of line) {
    if (escaped) {
      current += ch;
      escaped = false;
      any = true;
      continue;
    }
    if (ch === "\\" && quote !== "'") {
      escaped = true;
      continue;
    }
    if (quote) {
      if (ch === quote) {
        quote = null;
      } else {
        current += ch;
        any = true;
      }
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      any = true;
      continue;
    }
    if (ch === " " || ch === "\t") {
      if (any) {
        tokens.push(current);
        current = "";
        any = false;
      }
      continue;
    }
    current += ch;
    any = true;
  }
  if (quote) return null;
  if (any) tokens.push(current);
  return tokens;
}

/**
 * Strip a leading `sudo`, `nohup`, or env-var prefix (`FOO=bar
 * BAR=baz`) so `sudo ssh root@host` and `LANG=C ssh user@host` are
 * recognized too. Stops at the first non-prefix token (which is
 * what we will inspect for `ssh`).
 */
function stripPrefixes(tokens: string[]): string[] {
  let i = 0;
  while (i < tokens.length) {
    const t = tokens[i];
    if (t === "sudo" || t === "nohup" || t === "doas") {
      i += 1;
      continue;
    }
    // VAR=value style env assignments are valid before any command.
    if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(t)) {
      i += 1;
      continue;
    }
    break;
  }
  return tokens.slice(i);
}

/**
 * Parse a single command line and return the SSH target it would
 * connect to, or `null` if it is not a recognizable `ssh` invocation.
 * Handles the common forms:
 *   ssh host
 *   ssh user@host
 *   ssh -p 2222 user@host
 *   ssh user@host -p 2222
 *   ssh -l user host
 *   ssh -i ~/.ssh/id_ed25519 user@host
 *
 * Compound commands (`&&`, `||`, `;`, `|`) and shell substitutions
 * (`$(…)`, backticks) are intentionally rejected — guessing the
 * SSH target out of a pipeline is more likely to mislead than help.
 */
export function parseSshCommand(line: string): ParsedSshTarget | null {
  const trimmed = line.trim();
  if (!trimmed) return null;

  // Reject anything containing shell control characters that could
  // change which command actually runs. We look at the raw string
  // (not the tokens) so we catch them inside or outside quotes.
  if (/[`$|;]|&&|\|\|/.test(trimmed)) return null;
  // Redirections aren't fatal but signal the user is doing something
  // more involved — punt rather than half-parse.
  if (/[<>]/.test(trimmed)) return null;

  const tokens = tokenize(trimmed);
  if (!tokens || tokens.length < 2) return null;

  const stripped = stripPrefixes(tokens);
  if (stripped.length < 2) return null;
  if (stripped[0] !== "ssh") return null;

  let user = "";
  let host = "";
  let port = 0;
  let identityPath = "";

  // ssh argument flags that *consume the next token* as their value.
  // We don't try to model every option — just the ones we care about
  // (`-p`, `-l`, `-i`) plus a few common ones whose value would
  // otherwise be misread as the host (`-o`, `-J`, `-F`, `-c`, `-m`,
  // `-D`, `-L`, `-R`, `-W`, `-Q`, `-O`, `-S`, `-w`, `-b`, `-B`, `-E`,
  // `-I`).
  const flagsWithValue = new Set([
    "-p", "-l", "-i", "-o", "-J", "-F", "-c", "-m",
    "-D", "-L", "-R", "-W", "-Q", "-O", "-S", "-w", "-b", "-B", "-E", "-I",
  ]);

  let i = 1;
  while (i < stripped.length) {
    const arg = stripped[i];
    if (arg === "--") {
      i += 1;
      continue;
    }
    if (flagsWithValue.has(arg)) {
      const value = stripped[i + 1];
      if (value === undefined) return null;
      if (arg === "-p") {
        const parsed = Number.parseInt(value, 10);
        if (!Number.isFinite(parsed) || parsed <= 0 || parsed > 65535) return null;
        port = parsed;
      } else if (arg === "-l") {
        user = value;
      } else if (arg === "-i") {
        identityPath = value;
      } else if (arg === "-o") {
        // Honor `-o Port=2222` and `-o User=name` so `ssh -o Port=22 host`
        // still routes the right side correctly.
        const eq = value.indexOf("=");
        if (eq > 0) {
          const key = value.slice(0, eq).toLowerCase();
          const val = value.slice(eq + 1);
          if (key === "port") {
            const parsed = Number.parseInt(val, 10);
            if (Number.isFinite(parsed) && parsed > 0 && parsed <= 65535) {
              port = parsed;
            }
          } else if (key === "user") {
            user = val;
          } else if (key === "identityfile") {
            identityPath = val;
          }
        }
      }
      i += 2;
      continue;
    }
    // `-pNN`, `-lname`, `-iPATH` (no space) — short forms.
    if (arg.startsWith("-p") && arg.length > 2 && /^\d+$/.test(arg.slice(2))) {
      const parsed = Number.parseInt(arg.slice(2), 10);
      if (parsed > 0 && parsed <= 65535) port = parsed;
      i += 1;
      continue;
    }
    if (arg.startsWith("-l") && arg.length > 2) {
      user = arg.slice(2);
      i += 1;
      continue;
    }
    if (arg.startsWith("-i") && arg.length > 2) {
      identityPath = arg.slice(2);
      i += 1;
      continue;
    }
    // Boolean flags (`-A`, `-N`, `-T`, `-v`, etc.) — anything starting
    // with `-` we don't understand is treated as a no-arg flag and
    // skipped. Better to ignore an unknown option than refuse to
    // parse a command we mostly understand.
    if (arg.startsWith("-")) {
      i += 1;
      continue;
    }

    // Positional argument — the destination.
    const destination = arg;
    const at = destination.lastIndexOf("@");
    if (at >= 0) {
      const userPart = destination.slice(0, at);
      const hostPart = destination.slice(at + 1);
      if (userPart) user = userPart;
      host = hostPart;
    } else {
      host = destination;
    }

    // Anything after the destination is a remote command — we don't
    // care about it for routing the right panel. Stop scanning.
    break;
  }

  if (!host) return null;
  // Reject obvious garbage: `ssh -p 22` with no destination, or a
  // host like `--help` that slipped past the flag detector.
  if (host.startsWith("-")) return null;

  return { host, user, port, identityPath };
}
