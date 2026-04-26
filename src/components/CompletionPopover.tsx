// ── Tab completion popover (M4) ────────────────────────────────────
//
// Smart-mode pop-up shown when the user presses Tab. Displays a
// scrollable list of candidates the user can navigate with ↑/↓ and
// accept with Enter/Tab. Esc dismisses without inserting.
//
// All chrome comes from the existing `.popover` / `.popover-item`
// atoms in `src/styles/atoms.css` — no new dialog colours, no new
// shadow shapes. The component itself only owns:
//   * scroll-into-view bookkeeping when the active row changes
//   * keyboard event consumption guards (so the underlying terminal
//     never sees the keys we care about)
//   * the icon mapping per `CompletionKind`
//
// Positioning is delegated to the upstream `Popover` wrapper which
// handles viewport clamping, `closeOnScroll`, and the click-outside
// dismissal pattern. Caller provides an `anchor` element that
// follows the cursor inside the terminal grid.

import { useEffect, useRef } from "react";
import {
  Box,
  FileText,
  Flag,
  Folder,
  GitBranch,
  Terminal as TerminalIcon,
} from "lucide-react";
import Popover from "./Popover";
import type { Completion } from "../lib/terminalSmart";

type Props = {
  /** Whether the popover is mounted. Caller manages this; we only
   *  render the inner content + portal. */
  open: boolean;
  /** DOM element the popover anchors to. Caller creates / positions
   *  it relative to the cursor cell inside the terminal screen. */
  anchor: HTMLElement | null;
  /** Filtered candidates currently displayed. Caller is responsible
   *  for filtering against the user's typing while the popover is
   *  open — we just render whatever this prop holds. */
  items: Completion[];
  /** Index of the currently highlighted row. -1 means "no
   *  selection yet"; the first ↓ press selects index 0. */
  selectedIndex: number;
  onSelect: (item: Completion, index: number) => void;
  onHighlight: (index: number) => void;
  onClose: () => void;
};

/** Lucide icons line up with the `CompletionKind` discriminator. */
function iconForKind(kind: Completion["kind"]) {
  switch (kind) {
    case "builtin":
      return TerminalIcon;
    case "binary":
      return Box;
    case "directory":
      return Folder;
    case "subcommand":
      return GitBranch;
    case "option":
      return Flag;
    case "file":
    default:
      return FileText;
  }
}

export default function CompletionPopover({
  open,
  anchor,
  items,
  selectedIndex,
  onSelect,
  onHighlight,
  onClose,
}: Props) {
  const listRef = useRef<HTMLDivElement>(null);

  // Keep the active row visible as the user arrows up/down. We
  // measure inside the popover's own scroll container, not the
  // viewport, so a long list still tracks the highlight cleanly.
  useEffect(() => {
    if (!open) return;
    const container = listRef.current;
    if (!container) return;
    const row = container.querySelector<HTMLElement>(
      `[data-completion-index="${selectedIndex}"]`,
    );
    row?.scrollIntoView({ block: "nearest", behavior: "auto" });
  }, [open, selectedIndex]);

  if (!open) return null;

  // When *any* row in the current list carries a description we
  // expand the popover and show the inline description on the row.
  // Pure file/binary lists (no descriptions) keep the compact
  // single-column shape so the popover doesn't grow gratuitously.
  const hasDescriptions = items.some((it) => !!it.description);
  const popoverWidth = hasDescriptions ? 480 : 320;

  return (
    <Popover
      open={open}
      anchor={anchor}
      onClose={onClose}
      placement="bottom-start"
      width={popoverWidth}
      closeOnScroll={false}
    >
      <div ref={listRef} className="completion-popover-list">
        {items.length === 0 ? (
          <div className="popover-section">No matches</div>
        ) : (
          items.map((item, idx) => {
            const Icon = iconForKind(item.kind);
            const className =
              idx === selectedIndex
                ? "popover-item is-active"
                : "popover-item";
            return (
              <button
                key={`${item.kind}:${item.value}:${idx}`}
                type="button"
                data-completion-index={idx}
                className={className}
                onMouseEnter={() => onHighlight(idx)}
                onMouseDown={(event) => {
                  // Prevent the terminal from losing focus before
                  // the click resolves; popover dismissal happens
                  // explicitly via onSelect → caller closes.
                  event.preventDefault();
                  onSelect(item, idx);
                }}
              >
                <span className="popover-item__icon">
                  <Icon size={12} />
                </span>
                <span className="popover-item__label">{item.display}</span>
                {/* Library-driven rows show their description inline
                  * to the right of the label (fish/warp style). When
                  * no description exists we fall back to the legacy
                  * `hint` slot used by binary path / "builtin"
                  * markers — both render in the same muted column. */}
                {item.description ? (
                  <span className="completion-popover-desc">
                    {item.description}
                  </span>
                ) : item.hint ? (
                  <span className="completion-popover-hint">{item.hint}</span>
                ) : null}
              </button>
            );
          })
        )}
      </div>
    </Popover>
  );
}
