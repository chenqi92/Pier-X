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
export type CompletionKind =
  | "builtin"
  | "binary"
  | "file"
  | "directory"
  | "subcommand"
  | "option"
  | "history";

/** One row in the completion popover. `value` is the full text the
 *  UI should produce when this row is selected; `display` is what
 *  to show in the row's main label; `hint` is the optional muted
 *  right-side annotation (resolved binary path, etc.); `description`
 *  is the localized side-panel text from the bundled command
 *  library (only set for `subcommand` / `option` rows). */
export type Completion = {
  kind: CompletionKind;
  value: string;
  display: string;
  hint?: string | null;
  description?: string | null;
};

/** Tab-completion candidates for the input line at `cursor`.
 *  Caller passes the shell's last-known `cwd` so the file branch
 *  resolves the right directory; pass `null` to fall back to the
 *  Pier-X process cwd inside pier-core. `locale` drives the
 *  language of subcommand / option descriptions returned for known
 *  commands (docker / git / kubectl / npm / ssh in the bundled
 *  pack); pass the active i18n locale, e.g. `"zh-CN"`. */
export const terminalCompletions = (
  line: string,
  cursor: number,
  cwd: string | null,
  locale: string | null = null,
) =>
  invoke<Completion[]>("terminal_completions", { line, cursor, cwd, locale });

/** A single option flag + its summary parsed from the man / --help
 *  output. Rendered as one row in the man-popover OPTIONS section. */
export type ManOption = {
  flag: string;
  summary: string;
};

/** Parsed man-page (or `--help` fallback) summary for one command. */
export type ManSynopsis = {
  synopsis: string;
  description: string;
  options: ManOption[];
  /** `"man"` when the data came from `man -P cat <cmd>`,
   *  `"help"` for the `<cmd> --help` fallback, `""` when neither
   *  was available. The popover shows this as a small muted hint
   *  so the user knows whether they're reading the canonical man
   *  page or a synthesised fallback. */
  source: string;
};

/** Look up the man-page summary for `command`. Resolves to `null`
 *  when neither `man` nor `--help` produced usable text — the
 *  popover renders an explicit "no documentation" message instead
 *  of treating that as an error. Genuine errors (invalid name, I/O
 *  failure) come back as a rejected promise. */
export const terminalManSynopsis = (command: string) =>
  invoke<ManSynopsis | null>("terminal_man_synopsis", { command });

/** Load `shell`'s persisted history file from disk. Resolves to
 *  `[]` for both "no file yet" and "no platform data dir"; the
 *  frontend ring then keeps the in-memory-only behaviour. */
export const terminalHistoryLoad = (shell: string) =>
  invoke<string[]>("terminal_history_load", { shell });

/** Append `command` to `shell`'s persisted history file. Backend
 *  silently drops lines that match the credential-keyword filter,
 *  so callers can fire-and-forget. */
export const terminalHistoryPush = (shell: string, command: string) =>
  invoke<void>("terminal_history_push", { shell, command });

/** Wipe `shell`'s persisted history file. Idempotent. */
export const terminalHistoryClear = (shell: string) =>
  invoke<void>("terminal_history_clear", { shell });

// ── Smart-mode command library ───────────────────────────────────
//
// Settings → Terminal → Command library. The library is a
// structured catalogue of commands with subcommand + option
// descriptions feeding the Tab completion popover. A small set
// ships bundled in the binary; users can install or update extra
// packs from disk (Phase D / E).

/** One row in the Settings library list. */
export type LibraryEntry = {
  command: string;
  toolVersion: string;
  /** `"bundled-seed"` (compiled-in), `"auto-imported"` (importer
   *  produced this from a CLI's `--help`/man/completion script),
   *  or `"user"` (hand-curated). */
  source: string;
  /** `"completion-zsh"` / `"man"` / `"help"` / `"hand-curated"`. */
  importMethod: string;
  /** `YYYY-MM-DD`. */
  importDate: string;
  subcommandCount: number;
  optionCount: number;
  /** Sorted list of locales present somewhere in the pack
   *  (e.g. `["en", "zh-CN"]`). */
  locales: string[];
};

/** Snapshot returned by the library Tauri commands. */
export type LibrarySnapshot = {
  entries: LibraryEntry[];
  /** Absolute path to the user pack directory; empty when the
   *  platform doesn't have an `app_data_dir`. */
  userDir: string;
};

/** List every loaded pack (bundled + user) for the Settings UI. */
export const completionLibraryList = () =>
  invoke<LibrarySnapshot>("completion_library_list");

/** Re-read user packs from disk and return the fresh snapshot. */
export const completionLibraryReload = () =>
  invoke<LibrarySnapshot>("completion_library_reload");

/** Install (or replace) a user pack. `body` is the raw JSON of a
 *  `CommandPack`; the backend validates schema + safe filename. */
export const completionLibraryInstallPack = (body: string) =>
  invoke<LibrarySnapshot>("completion_library_install_pack", { body });

/** Install a pack by reading the JSON file at the given absolute
 *  path. Returns the post-install snapshot. Used by the Settings
 *  "Import…" file picker so the frontend doesn't need to pull in a
 *  filesystem plugin just to forward the bytes. */
export const completionLibraryInstallPackFromPath = (path: string) =>
  invoke<LibrarySnapshot>("completion_library_install_pack_from_path", { path });

/** Remove a user pack by command name. Bundled packs are
 *  immutable; the UI hides the button for them. */
export const completionLibraryRemovePack = (command: string) =>
  invoke<LibrarySnapshot>("completion_library_remove_pack", { command });
