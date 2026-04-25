// ── Smart-mode Tauri wrappers ─────────────────────────────────────
// Typed `invoke()` wrappers for the smart-mode commands surfaced by
// `src-tauri/src/terminal_smart.rs`. Lives in its own file so M3..M6
// additions (completions, history queries, man-page summaries) don't
// pollute the main commands.ts.
//
// Per CLAUDE.md Rule 4, panels never call `invoke()` directly — they
// go through these helpers, and these helpers are the only place
// command names appear as raw strings.

import { invoke } from "@tauri-apps/api/core";

/** What `terminal_validate_command` returns. `kind` discriminates:
 *  - `"builtin"` — POSIX/bash/zsh shell builtin (cd, echo, …)
 *  - `"binary"` — found on `$PATH`; `path` is the absolute path
 *  - `"missing"` — not a builtin, not on `$PATH`; `path` is null */
export type CommandValidation = {
  kind: "builtin" | "binary" | "missing";
  path: string | null;
};

/** Resolve a command name against shell builtins + `$PATH`. Used by
 *  the smart-mode syntax overlay to red-line typos. The frontend
 *  caches results per-session in `useTerminalSmartStore`, so each
 *  unique name only crosses the IPC boundary once. */
export const terminalValidateCommand = (name: string) =>
  invoke<CommandValidation>("terminal_validate_command", { name });

/** Discriminator for `Completion.kind`. */
export type CompletionKind = "builtin" | "binary" | "file" | "directory";

/** One row in the completion popover. `value` is the full text the
 *  UI should produce when this row is selected; `display` is what
 *  to show in the row's main label; `hint` is the optional muted
 *  right-side annotation (resolved binary path, etc.). */
export type Completion = {
  kind: CompletionKind;
  value: string;
  display: string;
  hint?: string | null;
};

/** Tab-completion candidates for the input line at `cursor`.
 *  Caller passes the shell's last-known `cwd` so the file branch
 *  resolves the right directory; pass `null` to fall back to the
 *  Pier-X process cwd inside pier-core. */
export const terminalCompletions = (
  line: string,
  cursor: number,
  cwd: string | null,
) =>
  invoke<Completion[]>("terminal_completions", { line, cursor, cwd });
