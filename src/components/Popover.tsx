import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import { createPortal } from "react-dom";

type Placement =
  | "bottom-start"
  | "bottom-end"
  | "bottom"
  | "top-start"
  | "top-end"
  | "top";

type Props = {
  open: boolean;
  anchor: HTMLElement | null;
  onClose: () => void;
  width?: number | "anchor" | "auto";
  placement?: Placement;
  closeOnScroll?: boolean;
  className?: string;
  children: ReactNode;
};

const VIEWPORT_MARGIN = 8;
const ANCHOR_OFFSET = 4;

function computePosition(
  anchor: HTMLElement,
  el: HTMLElement,
  placement: Placement,
): { left: number; top: number } {
  const rect = anchor.getBoundingClientRect();
  const w = el.offsetWidth;
  const h = el.offsetHeight;
  const vw = window.innerWidth;
  const vh = window.innerHeight;

  const wantsTop = placement.startsWith("top");
  const fitsBelow = rect.bottom + ANCHOR_OFFSET + h <= vh - VIEWPORT_MARGIN;
  const fitsAbove = rect.top - ANCHOR_OFFSET - h >= VIEWPORT_MARGIN;
  const placeAbove = wantsTop ? fitsAbove || !fitsBelow : !fitsBelow && fitsAbove;

  let top = placeAbove
    ? rect.top - ANCHOR_OFFSET - h
    : rect.bottom + ANCHOR_OFFSET;
  top = Math.max(VIEWPORT_MARGIN, Math.min(vh - h - VIEWPORT_MARGIN, top));

  let left: number;
  if (placement.endsWith("end")) {
    left = rect.right - w;
  } else if (placement === "bottom" || placement === "top") {
    left = rect.left + rect.width / 2 - w / 2;
  } else {
    left = rect.left;
  }
  left = Math.max(VIEWPORT_MARGIN, Math.min(vw - w - VIEWPORT_MARGIN, left));
  return { left, top };
}

export default function Popover({
  open,
  anchor,
  onClose,
  width,
  placement = "bottom-start",
  closeOnScroll = true,
  className,
  children,
}: Props) {
  const popoverRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    if (!open || !anchor || !popoverRef.current) {
      setPos(null);
      return;
    }
    const update = () => {
      if (!popoverRef.current || !anchor) return;
      setPos(computePosition(anchor, popoverRef.current, placement));
    };
    update();
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, [open, anchor, placement]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  useEffect(() => {
    if (!open || !closeOnScroll) return;
    const onScroll = (e: Event) => {
      const target = e.target as Node | null;
      if (popoverRef.current && target && popoverRef.current.contains(target)) return;
      onClose();
    };
    window.addEventListener("scroll", onScroll, true);
    return () => window.removeEventListener("scroll", onScroll, true);
  }, [open, closeOnScroll, onClose]);

  if (!open) return null;

  const widthStyle: CSSProperties = (() => {
    if (width === "anchor" && anchor) return { width: anchor.offsetWidth };
    if (typeof width === "number") return { width };
    return {};
  })();

  const style: CSSProperties = {
    ...widthStyle,
    left: pos?.left ?? -9999,
    top: pos?.top ?? -9999,
    visibility: pos ? "visible" : "hidden",
  };

  return createPortal(
    <div
      className="popover-layer"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={popoverRef}
        className={["popover", className ?? ""].filter(Boolean).join(" ")}
        style={style}
        onMouseDown={(e) => e.stopPropagation()}
      >
        {children}
      </div>
    </div>,
    document.body,
  );
}
