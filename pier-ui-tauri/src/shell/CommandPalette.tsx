import { useEffect, useRef, useState } from "react";
import { useI18n } from "../i18n/useI18n";

export type PaletteCommand = {
  title: string;
  shortcut?: string;
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

  return (
    <div className="palette-backdrop" onClick={onClose}>
      <div className="palette" onClick={(e) => e.stopPropagation()} onKeyDown={handleKeyDown}>
        <input
          ref={inputRef}
          className="palette__input"
          onChange={(e) => setQuery(e.currentTarget.value)}
          placeholder={t("Type a command…")}
          value={query}
        />
        <div className="palette__list">
          {filtered.length > 0 ? (
            filtered.map((cmd, i) => (
              <button
                key={cmd.title}
                className={i === selectedIndex ? "palette__item palette__item--selected" : "palette__item"}
                onClick={() => { cmd.action(); onClose(); }}
                onMouseEnter={() => setSelectedIndex(i)}
                type="button"
              >
                <span>{cmd.title}</span>
                {cmd.shortcut && <span className="palette__shortcut">{cmd.shortcut}</span>}
              </button>
            ))
          ) : (
            <div className="palette__empty">{t("No matching commands")}</div>
          )}
        </div>
      </div>
    </div>
  );
}
