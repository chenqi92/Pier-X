// ── Remote-desktop keyboard mapping ──────────────────────────────────────
// Translates a browser `KeyboardEvent.code` (physical key, layout-independent)
// into the two wire forms our backends need:
//   * a PC/AT set-1 scancode (+ extended flag) for RDP, and
//   * an X11 keysym for VNC.
// Tables adapted from the canonical XT scancode set and X11 keysymdef; this
// covers the standard 104-key layout. International dead-keys / AltGr layouts
// are a follow-up (US physical positions are assumed for scancodes).

type KeyEntry = {
  /** PC set-1 scancode low byte (RDP). */
  sc: number;
  /** True when the scancode needs the 0xE0 extended prefix. */
  ext?: boolean;
  /** Explicit X11 keysym for non-printable keys (VNC). Printable keys
   *  derive their keysym from `KeyboardEvent.key` instead. */
  keysym?: number;
};

// X11 keysyms for common non-character keys.
const KS = {
  BackSpace: 0xff08,
  Tab: 0xff09,
  Return: 0xff0d,
  Escape: 0xff1b,
  Delete: 0xffff,
  Home: 0xff50,
  Left: 0xff51,
  Up: 0xff52,
  Right: 0xff53,
  Down: 0xff54,
  PageUp: 0xff55,
  PageDown: 0xff56,
  End: 0xff57,
  Insert: 0xff63,
  Menu: 0xff67,
  NumLock: 0xff7f,
  ScrollLock: 0xff14,
  ShiftL: 0xffe1,
  ShiftR: 0xffe2,
  ControlL: 0xffe3,
  ControlR: 0xffe4,
  CapsLock: 0xffe5,
  AltL: 0xffe9,
  AltR: 0xffea,
  SuperL: 0xffeb,
  SuperR: 0xffec,
  Space: 0x20,
  F1: 0xffbe,
} as const;

/** `KeyboardEvent.code` → scancode / keysym. */
export const KEY_TABLE: Record<string, KeyEntry> = {
  Escape: { sc: 0x01, keysym: KS.Escape },
  Digit1: { sc: 0x02 },
  Digit2: { sc: 0x03 },
  Digit3: { sc: 0x04 },
  Digit4: { sc: 0x05 },
  Digit5: { sc: 0x06 },
  Digit6: { sc: 0x07 },
  Digit7: { sc: 0x08 },
  Digit8: { sc: 0x09 },
  Digit9: { sc: 0x0a },
  Digit0: { sc: 0x0b },
  Minus: { sc: 0x0c },
  Equal: { sc: 0x0d },
  Backspace: { sc: 0x0e, keysym: KS.BackSpace },
  Tab: { sc: 0x0f, keysym: KS.Tab },
  KeyQ: { sc: 0x10 },
  KeyW: { sc: 0x11 },
  KeyE: { sc: 0x12 },
  KeyR: { sc: 0x13 },
  KeyT: { sc: 0x14 },
  KeyY: { sc: 0x15 },
  KeyU: { sc: 0x16 },
  KeyI: { sc: 0x17 },
  KeyO: { sc: 0x18 },
  KeyP: { sc: 0x19 },
  BracketLeft: { sc: 0x1a },
  BracketRight: { sc: 0x1b },
  Enter: { sc: 0x1c, keysym: KS.Return },
  ControlLeft: { sc: 0x1d, keysym: KS.ControlL },
  KeyA: { sc: 0x1e },
  KeyS: { sc: 0x1f },
  KeyD: { sc: 0x20 },
  KeyF: { sc: 0x21 },
  KeyG: { sc: 0x22 },
  KeyH: { sc: 0x23 },
  KeyJ: { sc: 0x24 },
  KeyK: { sc: 0x25 },
  KeyL: { sc: 0x26 },
  Semicolon: { sc: 0x27 },
  Quote: { sc: 0x28 },
  Backquote: { sc: 0x29 },
  ShiftLeft: { sc: 0x2a, keysym: KS.ShiftL },
  Backslash: { sc: 0x2b },
  KeyZ: { sc: 0x2c },
  KeyX: { sc: 0x2d },
  KeyC: { sc: 0x2e },
  KeyV: { sc: 0x2f },
  KeyB: { sc: 0x30 },
  KeyN: { sc: 0x31 },
  KeyM: { sc: 0x32 },
  Comma: { sc: 0x33 },
  Period: { sc: 0x34 },
  Slash: { sc: 0x35 },
  ShiftRight: { sc: 0x36, keysym: KS.ShiftR },
  NumpadMultiply: { sc: 0x37 },
  AltLeft: { sc: 0x38, keysym: KS.AltL },
  Space: { sc: 0x39, keysym: KS.Space },
  CapsLock: { sc: 0x3a, keysym: KS.CapsLock },
  F1: { sc: 0x3b, keysym: 0xffbe },
  F2: { sc: 0x3c, keysym: 0xffbf },
  F3: { sc: 0x3d, keysym: 0xffc0 },
  F4: { sc: 0x3e, keysym: 0xffc1 },
  F5: { sc: 0x3f, keysym: 0xffc2 },
  F6: { sc: 0x40, keysym: 0xffc3 },
  F7: { sc: 0x41, keysym: 0xffc4 },
  F8: { sc: 0x42, keysym: 0xffc5 },
  F9: { sc: 0x43, keysym: 0xffc6 },
  F10: { sc: 0x44, keysym: 0xffc7 },
  NumLock: { sc: 0x45, keysym: KS.NumLock },
  ScrollLock: { sc: 0x46, keysym: KS.ScrollLock },
  Numpad7: { sc: 0x47 },
  Numpad8: { sc: 0x48 },
  Numpad9: { sc: 0x49 },
  NumpadSubtract: { sc: 0x4a },
  Numpad4: { sc: 0x4b },
  Numpad5: { sc: 0x4c },
  Numpad6: { sc: 0x4d },
  NumpadAdd: { sc: 0x4e },
  Numpad1: { sc: 0x4f },
  Numpad2: { sc: 0x50 },
  Numpad3: { sc: 0x51 },
  Numpad0: { sc: 0x52 },
  NumpadDecimal: { sc: 0x53 },
  F11: { sc: 0x57, keysym: 0xffc8 },
  F12: { sc: 0x58, keysym: 0xffc9 },
  // ── Extended (0xE0-prefixed) keys ──
  NumpadEnter: { sc: 0x1c, ext: true, keysym: KS.Return },
  ControlRight: { sc: 0x1d, ext: true, keysym: KS.ControlR },
  NumpadDivide: { sc: 0x35, ext: true },
  AltRight: { sc: 0x38, ext: true, keysym: KS.AltR },
  Home: { sc: 0x47, ext: true, keysym: KS.Home },
  ArrowUp: { sc: 0x48, ext: true, keysym: KS.Up },
  PageUp: { sc: 0x49, ext: true, keysym: KS.PageUp },
  ArrowLeft: { sc: 0x4b, ext: true, keysym: KS.Left },
  ArrowRight: { sc: 0x4d, ext: true, keysym: KS.Right },
  End: { sc: 0x4f, ext: true, keysym: KS.End },
  ArrowDown: { sc: 0x50, ext: true, keysym: KS.Down },
  PageDown: { sc: 0x51, ext: true, keysym: KS.PageDown },
  Insert: { sc: 0x52, ext: true, keysym: KS.Insert },
  Delete: { sc: 0x53, ext: true, keysym: KS.Delete },
  MetaLeft: { sc: 0x5b, ext: true, keysym: KS.SuperL },
  MetaRight: { sc: 0x5c, ext: true, keysym: KS.SuperR },
  ContextMenu: { sc: 0x5d, ext: true, keysym: KS.Menu },
};

/** Derive an X11 keysym from `KeyboardEvent.key` for printable characters. */
function keysymFromChar(key: string): number | null {
  if (key.length !== 1) return null;
  const cp = key.codePointAt(0) ?? 0;
  // Latin-1 maps directly; everything else uses the Unicode keysym range.
  return cp < 0x100 ? cp : 0x0100_0000 + cp;
}

export type ResolvedKey = {
  scancode: number;
  extended: boolean;
  keysym: number;
};

/** Resolve a key event into scancode + keysym. Returns `null` for keys we
 *  cannot map (so the caller can ignore them rather than send garbage). */
export function resolveKey(e: KeyboardEvent): ResolvedKey | null {
  const entry = KEY_TABLE[e.code];
  if (entry) {
    const keysym = entry.keysym ?? keysymFromChar(e.key) ?? 0;
    return { scancode: entry.sc, extended: !!entry.ext, keysym };
  }
  // Unknown physical key: best-effort keysym only (VNC still works).
  const keysym = keysymFromChar(e.key);
  if (keysym == null) return null;
  return { scancode: 0, extended: false, keysym };
}
