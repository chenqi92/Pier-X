// ── Shell input lexer ────────────────────────────────────────────
//
// Tokenises a single line of shell input for the smart-mode syntax
// overlay. Deliberately a *subset* of POSIX-shell — not a full bash
// parser — because the overlay only needs enough structure to colour
// each character, not to actually evaluate the line. Specifically:
//
// * we do not expand `$(...)`, backticks, brace expansion, here-docs,
//   process substitution, or arithmetic. They render as plain `text`
//   tokens. Real shells will still execute them; the highlight just
//   stops being interesting.
// * we do not track quote escaping inside double-quoted strings as
//   nested variables — `"hi $USER"` colours the whole thing as a
//   string. M5+ can split that if it ever matters visually.
// * paths are detected purely by their prefix (`/`, `./`, `../`,
//   `~/`). A bare `foo.txt` isn't called a path; that's correct
//   behaviour because shell only treats slash-bearing words specially.
//
// The output is a flat `Token[]` covering every character in the
// input — concatenating `tok.text` for every token reproduces the
// input byte-for-byte. The overlay relies on that invariant to
// align spans against the underlying terminal cells.

/** All token kinds the lexer can emit. Mapped 1:1 to a CSS class
 *  in `TerminalSyntaxOverlay.tsx` and `terminal-syntax.css`. */
export type ShellTokenKind =
  | "command" // first word in a command position (start, after pipe/sep)
  | "option" // `-x`, `--foo`, possibly with `=value`
  | "string" // anything inside `"..."` or `'...'`
  | "path" // word starting with `/`, `./`, `../`, `~/`
  | "variable" // `$NAME` or `${...}`
  | "operator" // `|`, `||`, `&&`, `;`, `&`, newline as separator
  | "redirect" // `>`, `>>`, `<`, `<<`, `2>`, `&>`, `2>&1` …
  | "comment" // `#...` to end of line (only at command position)
  | "whitespace" // runs of spaces / tabs — preserved for layout
  | "text"; // catch-all (positional arguments, numbers, …)

export type ShellToken = {
  kind: ShellTokenKind;
  /** Inclusive byte offset into the input string. */
  start: number;
  /** Exclusive byte offset into the input string. */
  end: number;
  text: string;
};

/**
 * Tokenise one line of shell input.
 *
 * Single-line by design — multi-line backslash continuation is M5+
 * scope. Pasting a multi-line string still works (the tokens just
 * include the literal `\n`), the colours simply don't reflow across
 * lines.
 */
export function tokenize(input: string): ShellToken[] {
  const out: ShellToken[] = [];
  if (!input) return out;

  // `commandPosition` is true at start of input and after every
  // operator that resets command context (pipe, semicolon, `&&`,
  // `||`, `&`). Whitespace doesn't reset it; the next non-whitespace
  // word in command position is a command.
  let commandPosition = true;
  let i = 0;
  const n = input.length;

  while (i < n) {
    const ch = input[i];

    // ── Whitespace ─────────────────────────────────────────────
    if (ch === " " || ch === "\t") {
      const start = i;
      while (i < n && (input[i] === " " || input[i] === "\t")) i++;
      out.push({ kind: "whitespace", start, end: i, text: input.slice(start, i) });
      continue;
    }

    // ── Comment (only at command position) ─────────────────────
    if (ch === "#" && commandPosition) {
      const start = i;
      while (i < n && input[i] !== "\n") i++;
      out.push({ kind: "comment", start, end: i, text: input.slice(start, i) });
      continue;
    }

    // ── Operators that flip command position ───────────────────
    // Order matters: longer prefixes (`&&`, `||`, `>>`) before short.
    if (ch === "|") {
      const start = i;
      i++;
      if (i < n && input[i] === "|") i++; // `||`
      out.push({ kind: "operator", start, end: i, text: input.slice(start, i) });
      commandPosition = true;
      continue;
    }
    if (ch === "&") {
      const start = i;
      i++;
      if (i < n && input[i] === "&") {
        i++;
        out.push({ kind: "operator", start, end: i, text: input.slice(start, i) });
        commandPosition = true;
        continue;
      }
      if (i < n && input[i] === ">") {
        // `&>` — bash redirect both stdout and stderr
        i++;
        if (i < n && input[i] === ">") i++; // `&>>`
        out.push({ kind: "redirect", start, end: i, text: input.slice(start, i) });
        continue;
      }
      // bare `&` — backgrounding operator
      out.push({ kind: "operator", start, end: i, text: input.slice(start, i) });
      commandPosition = true;
      continue;
    }
    if (ch === ";") {
      out.push({ kind: "operator", start: i, end: i + 1, text: ";" });
      i++;
      commandPosition = true;
      continue;
    }
    if (ch === "\n") {
      // Logical line break inside multi-line paste.
      out.push({ kind: "operator", start: i, end: i + 1, text: "\n" });
      i++;
      commandPosition = true;
      continue;
    }

    // ── Redirect operators ─────────────────────────────────────
    if (ch === ">" || ch === "<") {
      const start = i;
      i++;
      // `>>` / `<<`
      if (i < n && input[i] === ch) i++;
      out.push({ kind: "redirect", start, end: i, text: input.slice(start, i) });
      continue;
    }
    if ((ch === "1" || ch === "2") && input[i + 1] === ">") {
      // `2>`, `2>>`, `2>&1`, `1>`, …
      const start = i;
      i += 2; // digit + `>`
      if (i < n && input[i] === ">") i++; // append form `2>>`
      if (i < n && input[i] === "&") {
        i++;
        while (i < n && /[0-9]/.test(input[i])) i++; // `2>&1`
      }
      out.push({ kind: "redirect", start, end: i, text: input.slice(start, i) });
      continue;
    }

    // ── Strings ────────────────────────────────────────────────
    if (ch === '"' || ch === "'") {
      const quote = ch;
      const start = i;
      i++;
      // Inside double-quotes, backslash escapes the next char.
      // Inside single-quotes, only the closing quote ends the string —
      // shell doesn't interpret `\'` in single quotes at all.
      while (i < n && input[i] !== quote) {
        if (quote === '"' && input[i] === "\\" && i + 1 < n) {
          i += 2;
        } else {
          i++;
        }
      }
      if (i < n) i++; // consume closing quote (if present)
      out.push({ kind: "string", start, end: i, text: input.slice(start, i) });
      // After a string, we are no longer at command position — the
      // string was likely an argument. (A leading bare-quoted command
      // like `'/bin/ls'` is unusual enough that mis-classifying it as
      // an argument doesn't matter for highlight purposes.)
      commandPosition = false;
      continue;
    }

    // ── Variables ──────────────────────────────────────────────
    if (ch === "$") {
      const start = i;
      i++;
      if (i < n && input[i] === "{") {
        // `${...}` — span until matching `}`. We don't try to nest;
        // bash itself treats unbalanced `${` as an error so it's fine
        // to bail at end-of-input.
        i++;
        while (i < n && input[i] !== "}") i++;
        if (i < n) i++;
      } else if (i < n && /[A-Za-z_]/.test(input[i])) {
        while (i < n && /[A-Za-z0-9_]/.test(input[i])) i++;
      } else {
        // `$` followed by something non-identifier (e.g. `$5`, `$?`,
        // bare `$`) — accept one extra char if it looks like a special
        // parameter, otherwise leave the `$` as a one-char variable.
        if (i < n && /[0-9?#@*$!-]/.test(input[i])) i++;
      }
      out.push({ kind: "variable", start, end: i, text: input.slice(start, i) });
      commandPosition = false;
      continue;
    }

    // ── Word (command, option, path, or generic argument) ─────
    // Read until whitespace or a known terminator.
    const wordStart = i;
    while (
      i < n &&
      input[i] !== " " &&
      input[i] !== "\t" &&
      input[i] !== "\n" &&
      input[i] !== "|" &&
      input[i] !== "&" &&
      input[i] !== ";" &&
      input[i] !== ">" &&
      input[i] !== "<" &&
      input[i] !== '"' &&
      input[i] !== "'" &&
      input[i] !== "$"
    ) {
      i++;
    }
    if (i === wordStart) {
      // Defensive — should never happen, but guarantees forward
      // progress so we can't infinite-loop on a weird input.
      out.push({
        kind: "text",
        start: wordStart,
        end: wordStart + 1,
        text: input[wordStart] ?? "",
      });
      i = wordStart + 1;
      continue;
    }

    const text = input.slice(wordStart, i);
    const kind = classifyWord(text, commandPosition);
    out.push({ kind, start: wordStart, end: i, text });
    if (kind !== "option") commandPosition = false;
  }

  return out;
}

/**
 * Decide what flavour of word `text` is, given whether it's the
 * first word of a command. Options remain in command position so a
 * trailing word is still a command argument, not a second command —
 * matches what most shells actually do (`ls -la /tmp` has one
 * command, two args).
 */
function classifyWord(text: string, commandPosition: boolean): ShellTokenKind {
  if (commandPosition) {
    // Options before the command name are unusual but happen in
    // env-var-only lines like `FOO=bar cmd`. We don't model `FOO=bar`
    // specifically; it falls through to `text`.
    if (text.startsWith("-")) return "option";
    return "command";
  }
  if (text.startsWith("--") || text.startsWith("-")) {
    // `--foo`, `--foo=bar`, `-x`, `-xvf` — all options.
    // A standalone `-` (often meaning stdin) reads as text — most
    // shells render it plain too.
    if (text === "-") return "text";
    return "option";
  }
  if (
    text.startsWith("/") ||
    text.startsWith("./") ||
    text.startsWith("../") ||
    text.startsWith("~/") ||
    text === "~"
  ) {
    return "path";
  }
  return "text";
}
