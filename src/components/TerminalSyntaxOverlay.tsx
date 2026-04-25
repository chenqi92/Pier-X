// ── Terminal syntax overlay ───────────────────────────────────────
// Smart-mode (M2 + M3): paints a coloured copy of the in-progress
// input line on top of the underlying terminal grid. The grid still
// shows the same characters in the user's terminal-theme fg colour;
// the overlay sits at z-index 1 with a matching background so the
// two renderings do not double-print and the overlay's per-token
// colour wins visually.
//
// M3 adds typo highlighting: command tokens that resolve to
// `missing` (not a builtin, not on $PATH) get `--command-missing`
// instead of `--command`. Validation goes through
// `useTerminalSmartStore`, which caches results process-wide so a
// name only crosses the IPC boundary once. While a name is in
// flight the token stays styled as a normal command — switching to
// "missing" only after we know it's actually missing avoids a
// flash-of-typo on every keystroke during typing.
//
// Position is computed from the OSC 133;B prompt-end position
// (`promptEnd`) plus the cell metrics (`charWidth`, `rowHeight`)
// already measured by the parent for pty sizing. Because the
// containing block is `.terminal-screen` (which we mark
// `position: relative` for this purpose), the overlay's `top` /
// `left` are in screen-pixel space relative to the screen's content
// box.
//
// Multi-line input (long pasted lines, backslash continuation) is
// out of scope for M2 — we render a single row from `promptEnd` to
// the right edge of the grid. The overlay inherits the terminal's
// font metrics so the spans align with the underlying cells.

import { useEffect, useMemo } from "react";
import { tokenize, type ShellToken } from "../lib/shellLexer";
import { useTerminalSmartStore } from "../stores/useTerminalSmartStore";

type Props = {
  /**
   * The text the user has typed since the most recent prompt-end —
   * mirrored on the frontend in `smartLineBufferRef`. Empty string
   * is fine and renders nothing.
   */
  text: string;
  /**
   * Grid coordinate of OSC 133;B (`[row, col]`). When non-null and
   * `awaiting_input` is true, the overlay is visible from this cell
   * onward. Caller is responsible for not mounting the overlay when
   * it should be hidden (alt-screen, bracketed-paste, etc.) — the
   * component itself only handles layout and tokenisation.
   */
  promptEnd: [number, number];
  /** Pixel width of a single grid cell. Measured upstream from the
   *  font metrics of `.terminal-measure`. */
  charWidth: number;
  /** Pixel height of one grid row, matching `--terminal-row-h`. */
  rowHeight: number;
  /**
   * Background colour to paint behind each token span. Must match
   * the terminal's effective background so the overlay visually
   * "covers" the underlying row instead of producing a doubled-up
   * render with the terminal-segment text below.
   */
  bgColor: string;
  /**
   * M5 autosuggestion suffix — the bytes that would be appended if
   * the user accepted the current history match. Rendered in muted
   * gray after the tokenized text. Empty string = no suggestion;
   * the overlay still renders for the typed portion.
   */
  suggestionSuffix?: string;
};

export default function TerminalSyntaxOverlay({
  text,
  promptEnd,
  charWidth,
  rowHeight,
  bgColor,
  suggestionSuffix,
}: Props) {
  const tokens = useMemo<ShellToken[]>(() => tokenize(text), [text]);

  // Subscribe to the validation cache so a resolution arriving from
  // the backend triggers a re-render of just this overlay. The
  // selector returns the Map by reference; zustand re-renders when
  // the reference changes (the store always assigns a new Map on
  // update, see useTerminalSmartStore.ts).
  const cache = useTerminalSmartStore((s) => s.cache);
  const validateCommand = useTerminalSmartStore((s) => s.validateCommand);

  // Side-effect: ensure every command-position token has been
  // requested. The store deduplicates concurrent requests for the
  // same name and never re-requests a cached one, so this fires at
  // most once per unique name.
  useEffect(() => {
    for (const tok of tokens) {
      if (tok.kind === "command" && tok.text) {
        validateCommand(tok.text);
      }
    }
  }, [tokens, validateCommand]);

  // The overlay still renders when there's a suggestion but no
  // typed text (e.g. user pressed → on an empty line — rare, but
  // harmless). When both are empty there's nothing to draw.
  if ((!text || tokens.length === 0) && !suggestionSuffix) return null;

  const [row, col] = promptEnd;

  return (
    <div
      className="terminal-syntax-overlay"
      style={{
        top: row * rowHeight,
        left: col * charWidth,
        // Inline bg so each span inherits a solid colour matching
        // the user's terminal theme — the global stylesheet can't
        // know the runtime theme value, but the parent passed it in.
        background: bgColor,
        height: rowHeight,
        lineHeight: `${rowHeight}px`,
      }}
    >
      {tokens.map((tok, i) => {
        // Default class derived from token kind. Command tokens get
        // a flavour suffix once we know whether they resolve.
        let cls = `terminal-syntax terminal-syntax--${tok.kind}`;
        if (tok.kind === "command") {
          const resolved = cache.get(tok.text);
          if (resolved && resolved.kind === "missing") {
            cls = "terminal-syntax terminal-syntax--command-missing";
          }
        }
        return (
          <span key={i} className={cls}>
            {tok.text}
          </span>
        );
      })}
      {suggestionSuffix ? (
        <span className="terminal-syntax terminal-syntax--suggestion">
          {suggestionSuffix}
        </span>
      ) : null}
    </div>
  );
}
