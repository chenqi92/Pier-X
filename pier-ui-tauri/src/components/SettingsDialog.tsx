import { useState } from "react";
import { useI18n } from "../i18n/useI18n";
import { useThemeStore, TERMINAL_THEMES } from "../stores/useThemeStore";
import { useSettingsStore } from "../stores/useSettingsStore";
import type { Locale } from "../stores/useSettingsStore";

type Props = {
  open: boolean;
  onClose: () => void;
};

const PAGES = ["General", "Terminal"] as const;

export default function SettingsDialog({ open, onClose }: Props) {
  const { t } = useI18n();
  const [page, setPage] = useState<(typeof PAGES)[number]>("General");
  const { mode, setMode, terminalThemeIndex, setTerminalTheme } = useThemeStore();
  const settings = useSettingsStore();

  if (!open) return null;

  return (
    <div className="palette-backdrop" onClick={onClose}>
      <div className="dialog dialog--wide" onClick={(e) => e.stopPropagation()}>
        <div className="dialog__header">
          <h2 className="dialog__title">{t("Settings")}</h2>
        </div>
        <div className="dialog__settings-body">
          <nav className="dialog__nav">
            {PAGES.map((p) => (
              <button
                key={p}
                className={page === p ? "dialog__nav-item dialog__nav-item--active" : "dialog__nav-item"}
                onClick={() => setPage(p)}
                type="button"
              >
                {t(p)}
              </button>
            ))}
          </nav>
          <div className="dialog__content">
            {page === "General" ? (
              <div className="form-stack">
                <div className="panel-section__title"><span>{t("Theme")}</span></div>
                <div className="button-row">
                  {(["system", "dark", "light"] as const).map((m) => (
                    <button
                      key={m}
                      className={mode === m ? "surface-button surface-button--selected" : "surface-button"}
                      onClick={() => setMode(m)}
                      type="button"
                    >
                      {m === "system" ? t("Follow system") : m === "dark" ? t("Dark") : t("Light")}
                    </button>
                  ))}
                </div>

                <div className="panel-section__title"><span>{t("Language")}</span></div>
                <div className="button-row">
                  {(["en", "zh"] as Locale[]).map((l) => (
                    <button
                      key={l}
                      className={settings.locale === l ? "surface-button surface-button--selected" : "surface-button"}
                      onClick={() => settings.setLocale(l)}
                      type="button"
                    >
                      {l === "en" ? "English" : "中文"}
                    </button>
                  ))}
                </div>
              </div>
            ) : (
              <div className="form-stack">
                <div className="panel-section__title"><span>Terminal Theme</span></div>
                <div className="token-list">
                  {TERMINAL_THEMES.map((theme, i) => (
                    <button
                      key={theme.name}
                      className={terminalThemeIndex === i ? "token-button token-button--selected" : "token-button"}
                      onClick={() => setTerminalTheme(i)}
                      type="button"
                    >
                      <span style={{ display: "inline-block", width: 10, height: 10, borderRadius: 2, background: theme.bg, border: "1px solid var(--border-default)", marginRight: 4 }} />
                      {theme.name}
                    </button>
                  ))}
                </div>

                <div className="panel-section__title"><span>Font Size</span></div>
                <div className="branch-row">
                  <input
                    className="field-input field-input--narrow"
                    type="number"
                    min={10}
                    max={24}
                    value={settings.terminalFontSize}
                    onChange={(e) => settings.setTerminalFontSize(Number(e.currentTarget.value))}
                  />
                  <span className="inline-note">{settings.terminalFontSize}px</span>
                </div>

                <div className="panel-section__title"><span>Cursor Style</span></div>
                <div className="button-row">
                  {([0, 1, 2] as const).map((s) => (
                    <button
                      key={s}
                      className={settings.cursorStyle === s ? "surface-button surface-button--selected" : "surface-button"}
                      onClick={() => settings.setCursorStyle(s)}
                      type="button"
                    >
                      {s === 0 ? "Block" : s === 1 ? "Beam" : "Underline"}
                    </button>
                  ))}
                </div>

                <div className="panel-section__title"><span>Cursor Blink</span></div>
                <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13, color: "var(--text-secondary)", cursor: "pointer" }}>
                  <input type="checkbox" checked={settings.cursorBlink} onChange={(e) => settings.setCursorBlink(e.currentTarget.checked)} />
                  Enable cursor blink
                </label>

                <div className="panel-section__title"><span>Scrollback Lines</span></div>
                <input
                  className="field-input field-input--narrow"
                  type="number"
                  min={1000}
                  max={100000}
                  step={1000}
                  value={settings.scrollbackLines}
                  onChange={(e) => settings.setScrollbackLines(Number(e.currentTarget.value))}
                />
              </div>
            )}
          </div>
        </div>
        <div className="dialog__footer">
          <button className="welcome__btn welcome__btn--primary" onClick={onClose} type="button">{t("Close")}</button>
        </div>
      </div>
    </div>
  );
}
