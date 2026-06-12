// ── Terminal column-width helpers ───────────────────────────────────
// The terminal grid is COLUMN-addressed: a CJK/fullwidth glyph
// occupies 2 cells (the emulator stores the glyph plus a `\0`
// continuation cell). Any frontend math that converts "typed text"
// into "grid columns" — the smart-mode caret/anchor positions, the
// syntax-overlay token widths — must use these, not `String.length`
// (UTF-16 code units, wrong for CJK and astral-plane chars).
//
// `charCols` is an EXACT mirror of `is_wide_char` in
// `pier-core/src/terminal/emulator.rs`. Keep the two in sync: if the
// tables disagree, the frontend's column math drifts from the cells
// the backend actually laid out. (Deliberately no zero-width class
// and emoji count as 1 — same as the emulator.)

/** Grid columns occupied by one code point (1 or 2). */
export function charCols(cp: number): 1 | 2 {
  if (
    (cp >= 0x1100 && cp <= 0x115f) || // Hangul Jamo
    (cp >= 0x2329 && cp <= 0x232a) || // Angle brackets
    (cp >= 0x2e80 && cp <= 0x303e) || // CJK Radicals, Kangxi, Ideographic Description
    (cp >= 0x3040 && cp <= 0x33bf) || // Hiragana, Katakana, Bopomofo, CJK Compat
    (cp >= 0x3400 && cp <= 0x4dbf) || // CJK Unified Ideographs Extension A
    (cp >= 0x4e00 && cp <= 0x9fff) || // CJK Unified Ideographs
    (cp >= 0xa000 && cp <= 0xa4cf) || // Yi Syllables and Radicals
    (cp >= 0xac00 && cp <= 0xd7af) || // Hangul Syllables
    (cp >= 0xf900 && cp <= 0xfaff) || // CJK Compatibility Ideographs
    (cp >= 0xfe10 && cp <= 0xfe6f) || // CJK Compatibility Forms, Small Forms
    (cp >= 0xff01 && cp <= 0xff60) || // Fullwidth Latin, Halfwidth Katakana boundary
    (cp >= 0xffe0 && cp <= 0xffe6) || // Fullwidth Signs
    (cp >= 0x20000 && cp <= 0x2ffff) || // CJK Extension B–F
    (cp >= 0x30000 && cp <= 0x3ffff) // CJK Extension G, H
  ) {
    return 2;
  }
  return 1;
}

/** Grid columns occupied by a string. */
export function textCols(s: string): number {
  let n = 0;
  for (const ch of s) {
    n += charCols(ch.codePointAt(0) ?? 0);
  }
  return n;
}
