// Persistent user-defined terminal command snippets. Surfaced in the
// terminal's right-click menu so the user can drop a frequently-used
// command (`docker compose logs -f --tail=200`, `journalctl -u nginx -f`,
// etc.) into the active session with one click.
//
// Stored in localStorage under a single key — these are user-scoped
// preferences, not project state, so they don't belong in the SSH
// connection blob or in any per-host file.
//
// Behavior contract: if `runOnPaste` is true, the consumer terminates
// the pasted line with a literal newline (i.e. submits). Otherwise the
// command lands at the prompt for review/edit. Both shapes are useful;
// `df -h` is fine to auto-run, but a parameterized `kubectl scale` is
// safer to land and confirm.

const STORAGE_KEY = "pier-x:terminal-snippets";

export type TerminalSnippet = {
  id: string;
  /** Short label shown in the context menu and the manager dialog.
   *  Defaults to the first line of `command` when empty. */
  label: string;
  /** Command body. Pasted verbatim into the terminal session. */
  command: string;
  /** When true, append a `\n` after pasting so the command runs
   *  immediately. Defaults to `false` so users can review before
   *  hitting Enter — same default `crontab` / `kubectl edit` users
   *  expect. */
  runOnPaste?: boolean;
};

export function loadSnippets(): TerminalSnippet[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isValidSnippet);
  } catch {
    return [];
  }
}

export function saveSnippets(snippets: TerminalSnippet[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(snippets));
  } catch {
    // localStorage full / disabled — silently fall through. The
    // calling dialog reflects React state, so the user keeps editing
    // without realising persistence failed; that's acceptable for a
    // user-prefs feature where the worst case is "snippets disappear
    // next launch".
  }
}

function isValidSnippet(v: unknown): v is TerminalSnippet {
  if (!v || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  return (
    typeof o.id === "string" &&
    typeof o.label === "string" &&
    typeof o.command === "string"
  );
}

export function makeSnippetId(): string {
  return `snip-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 8)}`;
}

/** Clamp the displayed label so the context menu doesn't blow out
 *  to the edge of the viewport when a snippet has a multi-line body
 *  with no explicit label. */
export function snippetDisplayLabel(s: TerminalSnippet): string {
  const trimmed = s.label.trim();
  if (trimmed) return trimmed;
  const firstLine = s.command.split(/\r?\n/, 1)[0]?.trim() ?? "";
  if (firstLine.length > 60) return firstLine.slice(0, 57) + "…";
  return firstLine || "(empty snippet)";
}
