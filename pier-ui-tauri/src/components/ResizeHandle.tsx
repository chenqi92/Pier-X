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
};

export default function ResizeHandle({ direction, size, min, max, onResize }: Props) {
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
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
    };
  }, [direction, min, max, onResize]);

  return (
    <div
      className="resize-handle"
      onMouseDown={handleMouseDown}
    />
  );
}
