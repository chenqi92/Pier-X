import {
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent as ReactDragEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
  type Ref,
} from "react";

type Props<T> = {
  items: readonly T[];
  /** Height of a single row, in pixels. All rows must be uniform height. */
  rowHeight: number;
  /** Caller-provided row renderer. Returned element must have `height: rowHeight` and
   *  its own key (caller is free to pick a stable id). */
  renderRow: (item: T, index: number) => ReactNode;
  /** Extra rows rendered off-screen above and below the viewport to make
   *  fast scrolling feel smoother. Default 6 ≈ ~150px for a 26px row. */
  overscan?: number;
  className?: string;
  style?: CSSProperties;
  /** Forwarded so callers can mount drag handlers on the scroll container
   *  (SftpPanel wants the whole list to accept dropped files). */
  onDragEnter?: (event: ReactDragEvent<HTMLDivElement>) => void;
  onDragOver?: (event: ReactDragEvent<HTMLDivElement>) => void;
  onDragLeave?: (event: ReactDragEvent<HTMLDivElement>) => void;
  onDrop?: (event: ReactDragEvent<HTMLDivElement>) => void;
  /** Forwarded so callers can mount right-click handlers on the scroll
   *  container — SftpPanel uses this to open the "empty area" context
   *  menu when the user right-clicks below the last row. */
  onContextMenu?: (event: ReactMouseEvent<HTMLDivElement>) => void;
  /** Optional ref to the scroll container for imperative use (scrollTo, etc.). */
  scrollRef?: Ref<HTMLDivElement>;
};

/**
 * Minimal fixed-height virtualized list.
 *
 * Keeps DOM size proportional to the viewport instead of to the full
 * `items.length`, so a 10k-entry SFTP directory costs the same to render
 * as a 30-entry one. Uses `ResizeObserver` for the viewport height so the
 * visible window recomputes when the panel resizes, and spacer divs above
 * / below the rendered slice keep the scrollbar thumb proportional.
 *
 * Non-goals: variable-height rows (use an item-key → measurement cache
 * library if that's ever needed), horizontal virtualization, focus
 * restoration. Keyboard focus inside visible rows still works because
 * React only unmounts rows that actually scroll out of view.
 */
export default function VirtualList<T>({
  items,
  rowHeight,
  renderRow,
  overscan = 6,
  className,
  style,
  onDragEnter,
  onDragOver,
  onDragLeave,
  onDrop,
  onContextMenu,
  scrollRef,
}: Props<T>) {
  const innerRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  // Initial height is 0 until we measure; with items.length > 0 this
  // temporarily renders zero rows. Acceptable — the layout effect resolves
  // it on the same frame.
  const [viewportHeight, setViewportHeight] = useState(0);

  useLayoutEffect(() => {
    const el = innerRef.current;
    if (!el) return;
    setViewportHeight(el.clientHeight);
    const ro = new ResizeObserver(() => setViewportHeight(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const total = items.length;
  const visibleCount = Math.max(1, Math.ceil(viewportHeight / rowHeight));
  const first = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const last = Math.min(total, first + visibleCount + overscan * 2);
  const topPad = first * rowHeight;
  const bottomPad = Math.max(0, (total - last) * rowHeight);

  const setRef = (node: HTMLDivElement | null) => {
    innerRef.current = node;
    if (typeof scrollRef === "function") scrollRef(node);
    else if (scrollRef && typeof scrollRef === "object") {
      (scrollRef as { current: HTMLDivElement | null }).current = node;
    }
  };

  return (
    <div
      ref={setRef}
      className={className}
      style={{ overflow: "auto", ...style }}
      onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
      onDragEnter={onDragEnter}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      onContextMenu={onContextMenu}
    >
      {topPad > 0 && <div style={{ height: topPad }} aria-hidden />}
      {items.slice(first, last).map((item, i) => renderRow(item, first + i))}
      {bottomPad > 0 && <div style={{ height: bottomPad }} aria-hidden />}
    </div>
  );
}
