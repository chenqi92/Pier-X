import { useEffect, useRef, useState } from "react";
import { Search } from "lucide-react";
import KeyCap from "../components/KeyCap";
import { useI18n } from "../i18n/useI18n";

export type PaletteCommand = {
  title: string;
  shortcut?: string;
  section?: string;
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

  // Group filtered commands by section while preserving order
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
    <div className="palette-backdrop" onClick={onClose}>
      <div
        className="palette"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
      >
        <div className="palette__input-row">
          <Search size={16} />
          <input
            ref={inputRef}
            className="palette__input"
            onChange={(e) => setQuery(e.currentTarget.value)}
            placeholder={t("Type a command…")}
            value={query}
          />
          <KeyCap>ESC</KeyCap>
        </div>
        <div className="palette__list">
          {filtered.length > 0 ? (
            grouped.map((group, gi) => (
              <div key={`g-${gi}`}>
                {group.section ? (
                  <div className="palette__section">{group.section}</div>
                ) : null}
                {group.items.map((cmd) => {
                  const idx = runningIndex++;
                  return (
                    <button
                      key={`${cmd.title}-${idx}`}
                      className={
                        idx === selectedIndex
                          ? "palette__item palette__item--selected"
                          : "palette__item"
                      }
                      onClick={() => {
                        cmd.action();
                        onClose();
                      }}
                      onMouseEnter={() => setSelectedIndex(idx)}
                      type="button"
                    >
                      <span>{cmd.title}</span>
                      {cmd.shortcut && (
                        <KeyCap className="palette__shortcut">{cmd.shortcut}</KeyCap>
                      )}
                    </button>
                  );
                })}
              </div>
            ))
          ) : (
            <div className="palette__empty">{t("No matching commands")}</div>
          )}
        </div>
      </div>
    </div>
  );
}
