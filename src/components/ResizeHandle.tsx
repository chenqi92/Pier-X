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
};

export default function ResizeHandle({ direction, size, min, max, onResize, className }: Props) {
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

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!dragging.current) return;
      const delta = e.clientX - startX.current;
      const newSize = direction === "left"
        ? startSize.current + delta
        : startSize.current - delta;
      onResize(Math.max(min, Math.min(max, newSize)));
    };

    const handleMouseUp = () => {
      if (!dragging.current) return;
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
    };
  }, [direction, min, max, onResize]);

  // Use prototype's `.resizer` class by default; caller may override.
  const cls = className || "resizer";
  return <div className={cls} onMouseDown={handleMouseDown} />;
}
