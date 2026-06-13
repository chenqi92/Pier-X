import { useCallback, useEffect, useRef } from "react";

type Props = {
  /** Which side of the handle is being resized */
  direction: "left" | "right";
  /** Current size in px of the target panel */
  size: number;
  /** Min width in px */
  min: number;
  /** Max width in px */
  max: number;
  /** Callback when size changes */
  onResize: (newSize: number) => void;
  /** Extra class for positioning (e.g. "resize-handle--left") */
  className?: string;
  /** Accessible label for keyboard and screen-reader users */
  ariaLabel?: string;
};

const KEYBOARD_STEP = 16;

export default function ResizeHandle({
  direction,
  size,
  min,
  max,
  onResize,
  className,
  ariaLabel,
}: Props) {
  const dragging = useRef(false);
  const startX = useRef(0);
  const startSize = useRef(0);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      dragging.current = true;
      startX.current = e.clientX;
      startSize.current = size;
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
      // Signals the pane-width transition to turn off so drag feels 1:1.
      document.body.classList.add("is-resizing");
    },
    [size],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      let next: number | null = null;
      if (e.key === "Home") {
        next = min;
      } else if (e.key === "End") {
        next = max;
      } else if (e.key === "ArrowLeft") {
        next = direction === "left" ? size - KEYBOARD_STEP : size + KEYBOARD_STEP;
      } else if (e.key === "ArrowRight") {
        next = direction === "left" ? size + KEYBOARD_STEP : size - KEYBOARD_STEP;
      }

      if (next === null) return;
      e.preventDefault();
      onResize(Math.max(min, Math.min(max, next)));
    },
    [direction, max, min, onResize, size],
  );

  useEffect(() => {
    // Coalesce mousemoves to one onResize per animation frame.
    // Without this, a fast drag fires ~120 mousemoves/s, each
    // setState re-renders the whole App tree (every panel, the
    // terminal viewport, sidebars, etc.). Under a heavy first-
    // connect (terminal handshake + monitor probe + sftp listing)
    // that storm of re-renders saturates WebView2's compositor
    // and the window goes white until the drag stops.
    let pendingX: number | null = null;
    let rafHandle: number | null = null;

    const flush = () => {
      rafHandle = null;
      if (pendingX === null || !dragging.current) return;
      const delta = pendingX - startX.current;
      pendingX = null;
      const newSize = direction === "left"
        ? startSize.current + delta
        : startSize.current - delta;
      onResize(Math.max(min, Math.min(max, newSize)));
    };

    const handleMouseMove = (e: MouseEvent) => {
      if (!dragging.current) return;
      pendingX = e.clientX;
      if (rafHandle !== null) return;
      rafHandle = window.requestAnimationFrame(flush);
    };

    const handleMouseUp = () => {
      if (!dragging.current) return;
      // Flush any pending move BEFORE clearing dragging — `flush`
      // bails out when dragging is false, so reversing this drops
      // the final pixel of the drag.
      if (rafHandle !== null) {
        window.cancelAnimationFrame(rafHandle);
        rafHandle = null;
      }
      if (pendingX !== null) flush();
      dragging.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      document.body.classList.remove("is-resizing");
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
      if (rafHandle !== null) window.cancelAnimationFrame(rafHandle);
    };
  }, [direction, min, max, onResize]);

  // Use prototype's `.resizer` class by default; caller may override.
  const cls = className || "resizer";
  return (
    <div
      className={cls}
      role="separator"
      aria-label={ariaLabel}
      aria-orientation="vertical"
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={Math.round(size)}
      tabIndex={0}
      onMouseDown={handleMouseDown}
      onKeyDown={handleKeyDown}
    />
  );
}
