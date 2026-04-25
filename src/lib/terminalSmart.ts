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
