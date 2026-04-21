import { useEffect, useRef, useState } from "react";
import type { ContextMenuItem } from "./ContextMenu";

export type MenuDef = {
  label: string;
  items: ContextMenuItem[];
};

type Props = {
  menus: MenuDef[];
};

/**
 * Windows / Linux titlebar menu bar. Renders one button per menu; clicking
 * opens a dropdown anchored under that button. Hovering a sibling while a
 * dropdown is open switches to it (standard desktop UX). Outside click and
 * Escape close the menu. On macOS this component is never mounted — the
 * native menubar at the top of the screen fills that role.
 */
export default function TitlebarMenu({ menus }: Props) {
  const [openIndex, setOpenIndex] = useState<number | null>(null);
  const barRef = useRef<HTMLDivElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const buttonRefs = useRef<(HTMLButtonElement | null)[]>([]);

  useEffect(() => {
    if (openIndex === null) return;
    const onDocMouseDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (barRef.current?.contains(target)) return;
      if (dropdownRef.current?.contains(target)) return;
      setOpenIndex(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpenIndex(null);
    };
    document.addEventListener("mousedown", onDocMouseDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocMouseDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [openIndex]);

  const activeMenu = openIndex !== null ? menus[openIndex] : null;
  const activeBtn = openIndex !== null ? buttonRefs.current[openIndex] : null;

  let dropdownStyle: React.CSSProperties | undefined;
  if (activeBtn) {
    const rect = activeBtn.getBoundingClientRect();
    dropdownStyle = { left: rect.left, top: rect.bottom + 2 };
  }

  return (
    <div className="menu-items" ref={barRef} data-no-window-drag="true">
      {menus.map((m, i) => (
        <button
          key={m.label}
          type="button"
          ref={(el) => {
            buttonRefs.current[i] = el;
          }}
          className={openIndex === i ? "is-open" : undefined}
          onMouseDown={(e) => {
            e.stopPropagation();
            setOpenIndex((cur) => (cur === i ? null : i));
          }}
          onMouseEnter={() => {
            if (openIndex !== null && openIndex !== i) setOpenIndex(i);
          }}
        >
          {m.label}
        </button>
      ))}
      {activeMenu ? (
        <div
          ref={dropdownRef}
          className="ctx-menu titlebar-menu-dropdown"
          style={dropdownStyle}
          onMouseDown={(e) => e.stopPropagation()}
        >
          {activeMenu.items.map((item, idx) =>
            "divider" in item ? (
              <div key={`d-${idx}`} className="ctx-menu__divider" />
            ) : (
              <button
                key={`${item.label}-${idx}`}
                type="button"
                className="ctx-menu__item"
                disabled={item.disabled}
                onClick={() => {
                  setOpenIndex(null);
                  item.action();
                }}
              >
                <span>{item.label}</span>
                {item.shortcut ? (
                  <span className="ctx-menu__shortcut">{item.shortcut}</span>
                ) : null}
              </button>
            ),
          )}
        </div>
      ) : null}
    </div>
  );
}
