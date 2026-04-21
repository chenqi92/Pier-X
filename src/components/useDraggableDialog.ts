import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties, HTMLAttributes, MouseEvent as ReactMouseEvent } from "react";

/**
 * Dialog drag behavior for the shared `.dlg` chrome.
 *
 * Returns a style to apply on the `.dlg` element and props to spread on
 * the drag handle (typically `.dlg-head`). Ignores drags that start on
 * interactive elements (buttons, inputs) inside the handle, so the close
 * button and title-bar search field keep working.
 */
export function useDraggableDialog(open: boolean) {
  const [pos, setPos] = useState<{ x: number; y: number }>({ x: 0, y: 0 });
  const dragRef = useRef<{ startX: number; startY: number; baseX: number; baseY: number } | null>(null);

  useEffect(() => {
    if (!open) setPos({ x: 0, y: 0 });
  }, [open]);

  const onMouseDown = useCallback((e: ReactMouseEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement;
    if (target.closest("button, input, textarea, select, a, [data-no-drag]")) return;
    dragRef.current = {
      startX: e.clientX,
      startY: e.clientY,
      baseX: pos.x,
      baseY: pos.y,
    };
    e.preventDefault();
  }, [pos.x, pos.y]);

  useEffect(() => {
    function move(e: MouseEvent) {
      const d = dragRef.current;
      if (!d) return;
      setPos({
        x: d.baseX + (e.clientX - d.startX),
        y: d.baseY + (e.clientY - d.startY),
      });
    }
    function up() {
      dragRef.current = null;
    }
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, []);

  const dialogStyle: CSSProperties =
    pos.x === 0 && pos.y === 0
      ? {}
      : { transform: `translate(${pos.x}px, ${pos.y}px)` };

  const handleProps: HTMLAttributes<HTMLDivElement> = {
    onMouseDown,
    style: { cursor: dragRef.current ? "grabbing" : "grab" },
  };

  return { dialogStyle, handleProps };
}
