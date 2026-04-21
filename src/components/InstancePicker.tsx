import { ChevronDown, Plus } from "lucide-react";
import type { CSSProperties, ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { useI18n } from "../i18n/useI18n";

export type InstanceOption = {
  id: string;
  name: string;
  /** Short secondary line — host:port, db name, compose project, etc. */
  meta: string;
  /** Optional short uppercase tag (e.g. "prod", "stage"). */
  tag?: string;
  /** Connection status — draws the coloured dot. */
  status?: "up" | "down" | "unknown";
};

type Props = {
  icon: ReactNode;
  instances: InstanceOption[];
  activeId: string;
  onSelect: (id: string) => void;
  onAdd?: () => void;
  /** CSS color expression for the icon tint (defaults to accent). */
  tintVar?: string;
  addLabel?: string;
};

export default function InstancePicker({
  icon,
  instances,
  activeId,
  onSelect,
  onAdd,
  tintVar = "var(--accent)",
  addLabel,
}: Props) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  const current = instances.find((i) => i.id === activeId) ?? instances[0];

  useEffect(() => {
    if (!open) return;
    function onClick(e: MouseEvent) {
      if (!rootRef.current) return;
      if (!rootRef.current.contains(e.target as Node)) setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", onClick);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onClick);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  if (!current) return null;

  const iconStyle: CSSProperties = {
    background: `color-mix(in srgb, ${tintVar} 22%, transparent)`,
    color: tintVar,
  };

  return (
    <div className="ins-picker" ref={rootRef}>
      <button
        type="button"
        className="ins-btn"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        <span className="ins-icon" style={iconStyle}>{icon}</span>
        <div className="ins-meta">
          <div className="ins-name">{current.name}</div>
          <div className="ins-sub mono">{current.meta}</div>
        </div>
        <span className={"cs-dot " + (current.status === "up" ? "on" : "off")} />
        <ChevronDown size={11} />
      </button>

      {open && (
        <div className="ins-menu" role="listbox">
          {instances.map((ins) => (
            <div
              key={ins.id}
              role="option"
              aria-selected={ins.id === activeId}
              className={"ins-menu-row" + (ins.id === activeId ? " sel" : "")}
              onClick={() => { onSelect(ins.id); setOpen(false); }}
            >
              <span className={"cs-dot " + (ins.status === "up" ? "on" : "off")} />
              <div className="ins-main">
                <div className="ins-name">{ins.name}</div>
                <div className="ins-sub mono">{ins.meta}</div>
              </div>
              {ins.tag ? <span className="ins-tag mono">{ins.tag}</span> : null}
            </div>
          ))}
          {onAdd && (
            <div className="ins-menu-foot">
              <button
                type="button"
                className="btn is-ghost is-compact"
                onClick={() => { onAdd(); setOpen(false); }}
              >
                <Plus size={10} /> {addLabel ?? t("Add connection")}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
