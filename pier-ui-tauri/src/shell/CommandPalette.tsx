import { useEffect, useRef, useState } from "react";
import { Search } from "lucide-react";
import type { ComponentType, SVGProps } from "react";
import { useI18n } from "../i18n/useI18n";

type LucideIcon = ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;

export type PaletteCommand = {
  title: string;
  shortcut?: string;
  section?: string;
  icon?: LucideIcon;
  action: () => void;
};

type Props = {
  open: boolean;
  onClose: () => void;
  commands: PaletteCommand[];
};

export default function CommandPalette({ open, onClose, commands }: Props) {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const filtered = commands.filter((cmd) =>
    cmd.title.toLowerCase().includes(query.toLowerCase()),
  );

  useEffect(() => {
    if (open) {
      setQuery("");
      setSelectedIndex(0);
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (filtered[selectedIndex]) {
        filtered[selectedIndex].action();
        onClose();
      }
    }
  }

  if (!open) return null;

  const grouped: { section: string | undefined; items: PaletteCommand[] }[] = [];
  for (const cmd of filtered) {
    const last = grouped[grouped.length - 1];
    if (last && last.section === cmd.section) {
      last.items.push(cmd);
    } else {
      grouped.push({ section: cmd.section, items: [cmd] });
    }
  }

  let runningIndex = 0;

  return (
    <div className="cmdp-overlay" onClick={onClose}>
      <div
        className="cmdp"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
      >
        <div className="cmdp-input">
          <Search size={15} />
          <input
            ref={inputRef}
            onChange={(e) => setQuery(e.currentTarget.value)}
            placeholder={t("Type a command or search…")}
            value={query}
          />
          <kbd>esc</kbd>
        </div>
        <div className="cmdp-list">
          {filtered.length > 0 ? (
            grouped.map((group, gi) => (
              <div key={`g-${gi}`}>
                {group.section ? (
                  <div className="cmdp-section">{group.section}</div>
                ) : null}
                {group.items.map((cmd) => {
                  const idx = runningIndex++;
                  const Icon = cmd.icon;
                  return (
                    <div
                      key={`${cmd.title}-${idx}`}
                      className={
                        idx === selectedIndex
                          ? "cmdp-item active"
                          : "cmdp-item"
                      }
                      onClick={() => {
                        cmd.action();
                        onClose();
                      }}
                      onMouseEnter={() => setSelectedIndex(idx)}
                      role="button"
                      tabIndex={0}
                    >
                      <span className="ci">{Icon ? <Icon size={14} /> : null}</span>
                      <span className="ct">{cmd.title}</span>
                      {cmd.shortcut && <span className="ck">{cmd.shortcut}</span>}
                    </div>
                  );
                })}
              </div>
            ))
          ) : (
            <div className="cmdp-item" style={{ color: "var(--muted)", cursor: "default" }}>
              {t("No matching commands")}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
