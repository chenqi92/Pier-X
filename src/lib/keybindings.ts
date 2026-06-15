/**
 * Single source of truth for the keyboard shortcuts the Settings
 * dialog's Keymap page lists AND for dispatching the global app
 * shortcuts in `App.tsx`.
 *
 * Two layers live here:
 *
 *  - The `KEYBINDINGS` catalog: every user-visible shortcut, grouped by
 *    scope. A binding with a `command` is user-rebindable (the global
 *    app commands); the rest are display-only documentation of keys
 *    owned by the OS / WebView, xterm, CodeMirror, or dev builds, and
 *    carry a `lockReason`.
 *
 *  - Chord helpers: normalize a `KeyboardEvent` into a comparable
 *    `Chord`, match an event against a chord, and render a chord into
 *    the glyph convention the Keymap page uses.
 *
 * Rebindable commands are dispatched in `App.tsx` through `matchChord`
 * against the *effective* chord (factory default merged with the user's
 * override from `useKeybindingsStore`). Adding a new global shortcut
 * means: add a handler in App.tsx's dispatch switch, add an entry here
 * with a `command` + `defaultChord`, and it becomes rebindable for free.
 */

export type KeybindingScope = "global" | "panel" | "git" | "terminal" | "editor";

/** Normalized, `KeyboardEvent`-comparable chord. `mod` is the primary
 *  accelerator — Cmd on macOS, Ctrl elsewhere — matching the
 *  `e.metaKey || e.ctrlKey` convention the app has always used. `key`
 *  is the normalized `KeyboardEvent.key` (single chars upper-cased). */
export type Chord = {
  key: string;
  mod: boolean;
  shift: boolean;
  alt: boolean;
};

/** Reserved accelerators that carry no single `defaultChord` — the
 *  numeric ranges (⌘1–9 / ⌘⌥1–9) and the multi-variant DevTools combos
 *  (⌘⌥I / Ctrl+Shift+I …). The recorder consults these so a rebind
 *  can't silently shadow them, because `App.tsx` dispatches the
 *  rebindable loop *before* its range branches. */
export type ReservedSpec =
  | { kind: "digits"; mod: boolean; alt: boolean; shift: boolean }
  | { kind: "chords"; chords: Chord[] };

export type Keybinding = {
  /** Stable id — also the override key in `useKeybindingsStore` and,
   *  for rebindable rows, the dispatch `command`. */
  id: string;
  /** User-facing label. Localized via `t()` in the consumer. */
  label: string;
  scope: KeybindingScope;
  /** Present ⇒ user-rebindable. The id `App.tsx` dispatches on. */
  command?: string;
  /** Factory chord for single-chord bindings. Used for dispatch (when
   *  rebindable), display (when not overridden), and conflict checks
   *  (even for locked rows like Copy, so a rebind can't silently
   *  shadow a reserved key). Absent for range / multi-key rows. */
  defaultChord?: Chord;
  /** Display glyphs for rows without a single `Chord` — numeric ranges
   *  ("⌘1 … ⌘9") and multi-key dev shortcuts ("⌘⌥I / F12"). */
  mac?: string;
  other?: string;
  /** Why a non-rebindable row is locked (tooltip on the lock icon).
   *  Localized via `t()` in the consumer. */
  lockReason?: string;
  /** Reserved chord(s) for a locked row that has no single
   *  `defaultChord`, so conflict detection can still block a rebind
   *  that would shadow it. */
  reserved?: ReservedSpec;
};

/** Terse chord constructor for the catalog. Every app accelerator
 *  includes the primary modifier, so `mod` defaults to true. */
const c = (key: string, opts?: { shift?: boolean; alt?: boolean }): Chord => ({
  key,
  mod: true,
  shift: opts?.shift ?? false,
  alt: opts?.alt ?? false,
});

const REASON_OS = "Owned by the OS / terminal — can't be rebound here.";
const REASON_EDITOR = "Belongs to the file editor — can't be rebound here.";
const REASON_FIND = "Only active while the find box is focused.";
const REASON_SQL = "Belongs to the SQL editor — can't be rebound here.";
const REASON_RANGE = "Numeric ranges can't be rebound.";
const REASON_DEV = "Reserved for development builds.";

export const KEYBINDINGS: Keybinding[] = [
  // ── Global (rebindable) ─────────────────────────────────────
  { id: "palette",      label: "Command palette",    scope: "global", command: "palette",      defaultChord: c("K") },
  { id: "new-terminal", label: "New local terminal", scope: "global", command: "new-terminal", defaultChord: c("T") },
  { id: "close-tab",    label: "Close tab",          scope: "global", command: "close-tab",    defaultChord: c("W") },
  { id: "new-ssh",      label: "New SSH connection", scope: "global", command: "new-ssh",      defaultChord: c("N") },
  { id: "settings",     label: "Settings",           scope: "global", command: "settings",     defaultChord: c(",") },
  // ── Global (range — locked) ─────────────────────────────────
  { id: "switch-tab-1-9", label: "Switch to tab 1–9", scope: "global", mac: "⌘1 … ⌘9", other: "Ctrl+1 … Ctrl+9", lockReason: REASON_RANGE, reserved: { kind: "digits", mod: true, alt: false, shift: false } },

  // ── Panels (rebindable) ─────────────────────────────────────
  { id: "toggle-git", label: "Toggle Git panel", scope: "panel", command: "toggle-git", defaultChord: c("G", { shift: true }) },
  { id: "toggle-ai",  label: "Toggle AI panel",  scope: "panel", command: "toggle-ai",  defaultChord: c("A", { shift: true }) },
  // ── Panels (locked) ─────────────────────────────────────────
  { id: "switch-right-tool-1-9", label: "Switch right-side tool 1–9", scope: "panel", mac: "⌘⌥1 … ⌘⌥9", other: "Ctrl+Alt+1 … Ctrl+Alt+9", lockReason: REASON_RANGE, reserved: { kind: "digits", mod: true, alt: true, shift: false } },
  { id: "sql-run", label: "Run query", scope: "panel", mac: "⌘↩", other: "Ctrl+Enter", lockReason: REASON_SQL, reserved: { kind: "chords", chords: [c("Enter")] } },

  // ── Editor / clipboard (OS / WebView / xterm — locked) ──────
  { id: "cut",        label: "Cut",        scope: "editor", defaultChord: c("X"), lockReason: REASON_OS },
  { id: "copy",       label: "Copy",       scope: "editor", defaultChord: c("C"), lockReason: REASON_OS },
  { id: "paste",      label: "Paste",      scope: "editor", defaultChord: c("V"), lockReason: REASON_OS },
  { id: "select-all", label: "Select all", scope: "editor", defaultChord: c("A"), lockReason: REASON_OS },
  // ── SFTP / Log dialogs (editor-contextual — locked) ─────────
  { id: "find",         label: "Find",           scope: "editor", defaultChord: c("F"), lockReason: REASON_EDITOR },
  { id: "find-replace", label: "Find & Replace", scope: "editor", defaultChord: c("H"), lockReason: REASON_EDITOR },
  { id: "save-file",    label: "Save file",      scope: "editor", defaultChord: c("S"), lockReason: REASON_EDITOR },
  { id: "find-next",    label: "Next match",     scope: "editor", mac: "⏎",  other: "Enter",       lockReason: REASON_FIND },
  { id: "find-prev",    label: "Previous match", scope: "editor", mac: "⇧⏎", other: "Shift+Enter", lockReason: REASON_FIND },

  // ── DevTools (dev builds only — locked) ─────────────────────
  // Two distinct chords each: ⌘⌥I (mac) and Ctrl+Shift+I (other), per
  // the document listener in App.tsx — both reserved against rebinds.
  { id: "devtools",         label: "Toggle DevTools",         scope: "global", mac: "⌘⌥I / F12", other: "Ctrl+Shift+I / F12", lockReason: REASON_DEV, reserved: { kind: "chords", chords: [c("I", { alt: true }), c("I", { shift: true })] } },
  { id: "devtools-console", label: "Open DevTools (Console)", scope: "global", mac: "⌘⌥J",       other: "Ctrl+Shift+J",       lockReason: REASON_DEV, reserved: { kind: "chords", chords: [c("J", { alt: true }), c("J", { shift: true })] } },
];

/** Rebindable bindings only — the ones `App.tsx` dispatches and the
 *  Keymap page exposes a recorder for. */
export const REBINDABLE: Keybinding[] = KEYBINDINGS.filter((b) => b.command);

/** A binding the user can rebind from the Keymap page. */
export function isRebindable(b: Keybinding): boolean {
  return b.command != null;
}

// ── Chord normalization & matching ──────────────────────────────

const MODIFIER_KEYS = new Set([
  "Shift",
  "Control",
  "Alt",
  "Meta",
  "OS",
  "AltGraph",
  "CapsLock",
]);

/** Normalize a `KeyboardEvent.key` for storage / matching. Single
 *  characters upper-case so Shift-state never changes a chord's
 *  identity; named keys ("Enter", "F1", ",") pass through verbatim. */
export function normalizeKey(key: string): string {
  return key.length === 1 ? key.toUpperCase() : key;
}

/** True for the bare modifier keys, which can't stand alone as a chord
 *  and should be ignored while recording. */
export function isModifierKey(key: string): boolean {
  return MODIFIER_KEYS.has(key);
}

/** Recover the base character of a physical key from
 *  `KeyboardEvent.code` for the keys we care about. Used only when
 *  Alt/Option is held, where macOS rewrites `e.key` to a composed
 *  glyph (⌥G → "©", ⌥I → a dead "ˆ"). Returns null for keys we don't
 *  special-case (function keys, Enter, …) so the caller falls back to
 *  `e.key`. */
function physicalKey(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  switch (code) {
    case "Comma": return ",";
    case "Period": return ".";
    case "Slash": return "/";
    case "Semicolon": return ";";
    case "Quote": return "'";
    case "BracketLeft": return "[";
    case "BracketRight": return "]";
    case "Backslash": return "\\";
    case "Minus": return "-";
    case "Equal": return "=";
    case "Backquote": return "`";
    default: return null;
  }
}

/** Normalized key identity for an event, recovering the base character
 *  behind a macOS Option dead-key so record / match / display stay
 *  consistent. Non-alt chords keep the logical `e.key` (matches the
 *  printed letter regardless of physical layout). */
function eventKey(e: KeyboardEvent): string {
  if (e.altKey) {
    const base = physicalKey(e.code);
    if (base) return base;
  }
  return normalizeKey(e.key);
}

/** Build a `Chord` from a live `KeyboardEvent` (used by the recorder). */
export function eventToChord(e: KeyboardEvent): Chord {
  return {
    key: eventKey(e),
    mod: e.metaKey || e.ctrlKey,
    shift: e.shiftKey,
    alt: e.altKey,
  };
}

/** Exact match — modifier state must match precisely so distinct
 *  chords (e.g. ⌘1 vs ⌘⌥1) never collide. */
export function matchChord(e: KeyboardEvent, chord: Chord): boolean {
  return (
    (e.metaKey || e.ctrlKey) === chord.mod &&
    e.shiftKey === chord.shift &&
    e.altKey === chord.alt &&
    eventKey(e) === chord.key
  );
}

export function chordEquals(a: Chord, b: Chord): boolean {
  return (
    a.key === b.key && a.mod === b.mod && a.shift === b.shift && a.alt === b.alt
  );
}

// ── Chord display ───────────────────────────────────────────────

const KEY_GLYPHS_MAC: Record<string, string> = {
  Enter: "↩",
  " ": "Space",
  Tab: "⇥",
  Backspace: "⌫",
  Delete: "⌦",
  Escape: "⎋",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
};

const KEY_NAMES_OTHER: Record<string, string> = {
  " ": "Space",
};

function keyLabel(key: string, isMac: boolean): string {
  if (isMac && KEY_GLYPHS_MAC[key]) return KEY_GLYPHS_MAC[key];
  if (!isMac && KEY_NAMES_OTHER[key]) return KEY_NAMES_OTHER[key];
  return key; // already normalized (upper-case letter, ",", "F1"…)
}

/** Render a `Chord` into the same glyph convention `KEYBINDINGS.mac` /
 *  `.other` use: concatenated glyphs on macOS ("⌘⇧G"), "+"-joined
 *  words elsewhere ("Ctrl+Shift+G"). */
export function formatChord(chord: Chord, isMac: boolean): string {
  if (isMac) {
    let s = "";
    if (chord.mod) s += "⌘";
    if (chord.alt) s += "⌥";
    if (chord.shift) s += "⇧";
    return s + keyLabel(chord.key, true);
  }
  const parts: string[] = [];
  if (chord.mod) parts.push("Ctrl");
  if (chord.alt) parts.push("Alt");
  if (chord.shift) parts.push("Shift");
  parts.push(keyLabel(chord.key, false));
  return parts.join("+");
}

/** Effective display string for a binding given the user's override.
 *  Rebindable rows derive glyphs from the effective chord; locked rows
 *  fall back to their static `mac` / `other` strings. */
export function displayChord(
  binding: Keybinding,
  override: Chord | undefined,
  isMac: boolean,
): string {
  const chord = override ?? binding.defaultChord;
  if (chord) return formatChord(chord, isMac);
  return (isMac ? binding.mac : binding.other) ?? "";
}

/** Default display string for a binding (no override). Kept for
 *  callers that only render factory chords. */
export function chordFor(binding: Keybinding, isMac: boolean): string {
  return displayChord(binding, undefined, isMac);
}

/** Splits a chord string on whitespace. Each part becomes one rendered
 *  `<kbd>`. Word-form chords like "Ctrl+T" stay glued — the `+` is
 *  visible inside the cap, which is the conventional rendering. */
export function chordTokens(chord: string): string[] {
  return chord.split(/\s+/).filter(Boolean);
}

/** True if a candidate chord lands on a reserved accelerator (numeric
 *  range or an explicit reserved chord list). */
function matchReserved(chord: Chord, spec: ReservedSpec): boolean {
  if (spec.kind === "digits") {
    return (
      chord.mod === spec.mod &&
      chord.alt === spec.alt &&
      chord.shift === spec.shift &&
      /^[1-9]$/.test(chord.key)
    );
  }
  return spec.chords.some((rc) => chordEquals(rc, chord));
}

/** Find an existing binding (other than `selfId`) that `chord` would
 *  collide with. Used by the recorder to block duplicates. Scans every
 *  binding's effective chord — including locked ones like Copy — AND
 *  the reserved ranges / DevTools combos that carry no single chord, so
 *  a rebind can't silently shadow a reserved accelerator. */
export function findChordConflict(
  chord: Chord,
  selfId: string,
  overrides: Record<string, Chord>,
): Keybinding | undefined {
  return KEYBINDINGS.find((b) => {
    if (b.id === selfId) return false;
    const eff = overrides[b.id] ?? b.defaultChord;
    if (eff && chordEquals(eff, chord)) return true;
    if (b.reserved && matchReserved(chord, b.reserved)) return true;
    return false;
  });
}

// ── Recorder ↔ dispatcher handshake ─────────────────────────────
// While the Keymap recorder is capturing a chord, the global window
// listener in App.tsx must stand down so the captured keystroke isn't
// also dispatched as a command (it propagates to `window` regardless
// of React's `stopPropagation`). The recorder toggles this flag.

let recordingActive = false;

export function setKeybindingRecording(active: boolean): void {
  recordingActive = active;
}

export function isKeybindingRecording(): boolean {
  return recordingActive;
}
