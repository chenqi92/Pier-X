/**
 * Single source of truth for the user-visible keyboard shortcuts the
 * Settings dialog's Keymap page lists. Mirrors the actual handlers
 * registered in `App.tsx` (`handleGlobalKeyDown`, titlebar menus,
 * command palette) and inside individual panels.
 *
 * The list is read-only for now — rebinding lands later. Until then,
 * adding a new shortcut means adding the handler **and** updating
 * this file so the Keymap page stays accurate.
 */

export type KeybindingScope = "global" | "panel" | "git" | "terminal" | "editor";

export type Keybinding = {
  /** Stable id — used by future rebind UI. */
  id: string;
  /** User-facing label. Localized via `t()` in the consumer. */
  label: string;
  /** Mac chord. The Settings page picks `mac` on macOS,
   *  `other` everywhere else. Use unicode key glyphs:
   *  ⌘ Cmd, ⌃ Ctrl, ⌥ Opt/Alt, ⇧ Shift, ↩ Return, ⏎ Enter,
   *  ⇥ Tab, ⌫ Backspace, ⎋ Esc. Multi-chord = space-separated. */
  mac: string;
  other: string;
  scope: KeybindingScope;
};

export const KEYBINDINGS: Keybinding[] = [
  // ── Global ──────────────────────────────────────────────────
  { id: "palette",         label: "Command palette",          mac: "⌘K",       other: "Ctrl+K",     scope: "global" },
  { id: "new-terminal",    label: "New local terminal",       mac: "⌘T",       other: "Ctrl+T",     scope: "global" },
  { id: "close-tab",       label: "Close tab",                mac: "⌘W",       other: "Ctrl+W",     scope: "global" },
  { id: "new-ssh",         label: "New SSH connection",       mac: "⌘N",       other: "Ctrl+N",     scope: "global" },
  { id: "settings",        label: "Settings",                 mac: "⌘,",       other: "Ctrl+,",     scope: "global" },
  { id: "switch-tab-1-9",  label: "Switch to tab 1–9",        mac: "⌘1 … ⌘9",  other: "Ctrl+1 … Ctrl+9", scope: "global" },

  // ── Panels ──────────────────────────────────────────────────
  { id: "toggle-git",      label: "Toggle Git panel",         mac: "⌘⇧G",      other: "Ctrl+Shift+G", scope: "panel" },

  // ── Editor (clipboard) ──────────────────────────────────────
  { id: "cut",             label: "Cut",                      mac: "⌘X",       other: "Ctrl+X",     scope: "editor" },
  { id: "copy",            label: "Copy",                     mac: "⌘C",       other: "Ctrl+C",     scope: "editor" },
  { id: "paste",           label: "Paste",                    mac: "⌘V",       other: "Ctrl+V",     scope: "editor" },
  { id: "select-all",      label: "Select all",               mac: "⌘A",       other: "Ctrl+A",     scope: "editor" },

  // ── SFTP / Log dialogs ──────────────────────────────────────
  { id: "find",            label: "Find",                     mac: "⌘F",       other: "Ctrl+F",     scope: "editor" },
  { id: "find-replace",    label: "Find & Replace",           mac: "⌘H",       other: "Ctrl+H",     scope: "editor" },
  { id: "save-file",       label: "Save file",                mac: "⌘S",       other: "Ctrl+S",     scope: "editor" },
  { id: "find-next",       label: "Next match",               mac: "⏎",        other: "Enter",      scope: "editor" },
  { id: "find-prev",       label: "Previous match",           mac: "⇧⏎",       other: "Shift+Enter", scope: "editor" },

  // ── SQL editor (DB panels) ──────────────────────────────────
  { id: "sql-run",         label: "Run query",                mac: "⌘↩",       other: "Ctrl+Enter", scope: "panel" },

  // ── DevTools (dev builds only) ──────────────────────────────
  { id: "devtools",        label: "Toggle DevTools",          mac: "⌘⌥I / F12", other: "Ctrl+Shift+I / F12", scope: "global" },
  { id: "devtools-console", label: "Open DevTools (Console)", mac: "⌘⌥J",      other: "Ctrl+Shift+J", scope: "global" },
];

/** Returns the key-chord display string for the active platform. */
export function chordFor(binding: Keybinding, isMac: boolean): string {
  return isMac ? binding.mac : binding.other;
}

/** Splits a chord string on whitespace. Each part becomes one rendered
 *  `<kbd>`. Word-form chords like "Ctrl+T" stay glued — the `+` is
 *  visible inside the cap, which is the conventional rendering. */
export function chordTokens(chord: string): string[] {
  return chord.split(/\s+/).filter(Boolean);
}
