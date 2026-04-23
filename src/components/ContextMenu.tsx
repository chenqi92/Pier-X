import { useEffect, useRef } from "react";

export type ContextMenuItem =
  | {
      label: string;
      action: () => void;
      disabled?: boolean;
      shortcut?: string;
      iconColor?: string;
    }
  | { section: string }
  | { divider: true };

type Props = {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
};

export default function ContextMenu({ x, y, items, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const escHandler = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    document.addEventListener("mousedown", handler);
    document.addEventListener("keydown", escHandler);
    return () => { document.removeEventListener("mousedown", handler); document.removeEventListener("keydown", escHandler); };
  }, [onClose]);

  // Clamp position to viewport
  const menuWidth = 220;
  const menuHeight = items.length * 32;
  const clampedX = Math.min(x, window.innerWidth - menuWidth - 8);
  const clampedY = Math.min(y, window.innerHeight - menuHeight - 8);

  return (
    <div
      className="ctx-menu"
      ref={ref}
      style={{ left: Math.max(4, clampedX), top: Math.max(4, clampedY) }}
    >
      {items.map((item, i) =>
        "divider" in item ? (
          <div key={`d-${i}`} className="ctx-menu__divider" />
        ) : "section" in item ? (
          <div key={`s-${i}-${item.section}`} className="ctx-menu__section">
            {item.section}
          </div>
        ) : (
          <button
            key={item.label}
            className="ctx-menu__item"
            disabled={item.disabled}
            onClick={() => { item.action(); onClose(); }}
            type="button"
          >
            <span className="ctx-menu__label">
              {item.iconColor !== undefined && (
                <span
                  className="ctx-menu__swatch"
                  style={{ background: item.iconColor || "transparent" }}
                />
              )}
              {item.label}
            </span>
            {item.shortcut && <span className="ctx-menu__shortcut">{item.shortcut}</span>}
          </button>
        ),
      )}
    </div>
  );
}
