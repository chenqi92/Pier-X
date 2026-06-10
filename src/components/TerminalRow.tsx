import { memo } from "react";
import type { TerminalLine } from "../lib/types";

/**
 * Resolve a backend-emitted color tag against the user's selected terminal
 * theme palette. (Moved verbatim from TerminalPanel — this is now the only
 * consumer.)
 *
 * Backend tags (see `render_terminal_color` in `src-tauri/src/lib.rs`):
 * - `""` → default fg/bg (returns `undefined` to inherit from the parent
 *   `.terminal-screen`, painted with `termTheme.fg` / `termTheme.bg`).
 * - `"ansi:N"` → N in 0..=15 maps to the theme palette; 16..=231 is the
 *   6×6×6 cube, 232..=255 is grayscale (both computed, theme-independent).
 * - `"#rrggbb"` → truecolor (ANSI SGR 38/48;2;r;g;b), passed through.
 */
function resolveTerminalColor(tag: string, ansi: string[]): string | undefined {
  if (!tag) return undefined;
  if (tag.startsWith("ansi:")) {
    const n = Number.parseInt(tag.slice(5), 10);
    if (!Number.isFinite(n)) return undefined;
    if (n >= 0 && n < 16 && ansi[n]) return ansi[n];
    if (n >= 16 && n <= 231) {
      const value = n - 16;
      const steps = [0, 95, 135, 175, 215, 255];
      const r = steps[Math.floor(value / 36) % 6];
      const g = steps[Math.floor(value / 6) % 6];
      const b = steps[value % 6];
      return `rgb(${r},${g},${b})`;
    }
    if (n >= 232 && n <= 255) {
      const shade = 8 + (n - 232) * 10;
      return `rgb(${shade},${shade},${shade})`;
    }
    return undefined;
  }
  return tag;
}

/**
 * Per-render environment shared by every row. Kept referentially stable
 * (useMemo in TerminalPanel) so the memo comparator can compare it by
 * identity — it changes only on theme / cursor-setting / column-count
 * changes, not on every snapshot.
 */
export type TerminalRowEnv = {
  cursorStyle: number;
  cursorBlink: boolean;
  ansi: string[];
  fg: string;
  cols: number;
};

type Props = {
  line: TerminalLine;
  env: TerminalRowEnv;
  rowIndex: number;
};

function TerminalRowImpl({ line, env, rowIndex }: Props) {
  const { cursorStyle, cursorBlink, ansi, fg, cols } = env;
  const usedCols = line.segments.reduce((n, s) => n + s.cells, 0);
  const padCols = Math.max(0, cols - usedCols);
  return (
    <div className="terminal-row" data-terminal-row={rowIndex} style={{ color: fg }}>
      {line.segments.map((seg, j) => {
        const isCursor = seg.cursor;
        // Cursor style: 0=block (default), 1=beam, 2=underline. Blink
        // lives on a ::before overlay (see terminal-panel.css) so the
        // animation fades only the cursor block, not the glyph underneath.
        const baseCursorClass = isCursor
          ? cursorStyle === 1
            ? "terminal-segment terminal-segment--cursor-beam"
            : cursorStyle === 2
              ? "terminal-segment terminal-segment--cursor-underline"
              : "terminal-segment terminal-segment--cursor"
          : "terminal-segment";
        const cursorClass = isCursor && cursorBlink
          ? `${baseCursorClass} terminal-segment--cursor-blink`
          : baseCursorClass;
        const segBg = isCursor ? undefined : resolveTerminalColor(seg.bg, ansi);
        const segFg = isCursor ? undefined : resolveTerminalColor(seg.fg, ansi);
        return (
          <span
            className={cursorClass}
            data-terminal-cells={seg.cells}
            key={`seg-${j}`}
            style={{
              backgroundColor: segBg,
              color: segFg,
              fontWeight: seg.bold ? 510 : 400,
              textDecoration: seg.underline ? "underline" : "none",
            }}
          >
            {seg.text}
          </span>
        );
      })}
      {padCols > 0 && (
        <span
          className="terminal-segment terminal-segment--filler"
          data-terminal-cells={padCols}
          aria-hidden
        >
          {" ".repeat(padCols)}
        </span>
      )}
    </div>
  );
}

/**
 * Memoized terminal row. The `line` prop is a fresh object on every
 * snapshot (new IPC payload), so equality is decided by the backend
 * content `hash`, not object identity; `env` is compared by reference.
 */
export const TerminalRow = memo(
  TerminalRowImpl,
  (prev, next) =>
    prev.line.hash === next.line.hash && prev.env === next.env && prev.rowIndex === next.rowIndex,
);
