import { useState } from "react";
import { useI18n } from "../i18n/useI18n";
import { useThemeStore, TERMINAL_THEMES } from "../stores/useThemeStore";
import {
  useSettingsStore,
  UI_FONT_OPTIONS,
  MONO_FONT_OPTIONS,
} from "../stores/useSettingsStore";
import type { Locale } from "../stores/useSettingsStore";

type Props = {
  open: boolean;
  onClose: () => void;
};

type Page = "General" | "Appearance" | "Terminal";

const PAGES: Page[] = ["General", "Appearance", "Terminal"];

// ── Reusable sub-components ─────────────────────────────────────

function SectionTitle({ children }: { children: React.ReactNode }) {
  return <div className="settings__section-title">{children}</div>;
}

function SettingRow({ label, description, children }: { label: string; description?: string; children: React.ReactNode }) {
  return (
    <div className="settings__row">
      <div className="settings__row-label">
        <span className="settings__row-name">{label}</span>
        {description && <span className="settings__row-desc">{description}</span>}
      </div>
      <div className="settings__row-control">{children}</div>
    </div>
  );
}

function SegmentedControl({ options, value, onChange }: { options: { label: string; value: string | number }[]; value: string | number; onChange: (v: string | number) => void }) {
  return (
    <div className="settings__segmented">
      {options.map((opt) => (
        <button
          key={String(opt.value)}
          className={value === opt.value ? "settings__seg-btn settings__seg-btn--active" : "settings__seg-btn"}
          onClick={() => onChange(opt.value)}
          type="button"
        >
          {opt.label}
        </button>
      ))}
    </div>
  );
}

function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      className={checked ? "settings__toggle settings__toggle--on" : "settings__toggle"}
      onClick={() => onChange(!checked)}
      type="button"
    >
      <span className="settings__toggle-thumb" />
    </button>
  );
}

// ── Main dialog ─────────────────────────────────────────────────

export default function SettingsDialog({ open, onClose }: Props) {
  const { t } = useI18n();
  const [page, setPage] = useState<Page>("General");
  const theme = useThemeStore();
  const settings = useSettingsStore();

  if (!open) return null;

  return (
    <div className="palette-backdrop" onClick={onClose}>
      <div className="dialog dialog--settings" onClick={(e) => e.stopPropagation()}>
        {/* Header */}
        <div className="dialog__header">
          <h2 className="dialog__title">{t("Settings")}</h2>
          <span className="dialog__subtitle">{t("Adjust appearance, terminal behavior, and saved connections.")}</span>
        </div>

        <div className="dialog__settings-body">
          {/* Left nav */}
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

          {/* Content */}
          <div className="dialog__content">
            {/* ── General ──────────────────────────────────── */}
            {page === "General" && (
              <div className="settings__page">
                <SectionTitle>{t("Theme")}</SectionTitle>
                <SettingRow label={t("Follow system")} description={t("Automatically match the operating system appearance.")}>
                  <Toggle checked={theme.mode === "system"} onChange={(on) => theme.setMode(on ? "system" : (theme.resolvedDark ? "dark" : "light"))} />
                </SettingRow>
                <SettingRow label={t("Color scheme")} description={t("Choose between dark and light mode.")}>
                  <SegmentedControl
                    options={[{ label: t("Dark"), value: "dark" }, { label: t("Light"), value: "light" }]}
                    value={theme.mode === "system" ? (theme.resolvedDark ? "dark" : "light") : theme.mode}
                    onChange={(v) => theme.setMode(v as "dark" | "light")}
                  />
                </SettingRow>

                <SectionTitle>{t("Language")}</SectionTitle>
                <SettingRow label={t("Interface language")} description={t("Changes apply immediately to all UI text.")}>
                  <SegmentedControl
                    options={[{ label: "English", value: "en" }, { label: "简体中文", value: "zh" }]}
                    value={settings.locale}
                    onChange={(v) => settings.setLocale(v as Locale)}
                  />
                </SettingRow>

                <SectionTitle>{t("Developer")}</SectionTitle>
                <SettingRow label={t("Performance overlay")} description={t("Show FPS and memory usage in the status bar.")}>
                  <Toggle checked={settings.performanceOverlay} onChange={settings.setPerformanceOverlay} />
                </SettingRow>
              </div>
            )}

            {/* ── Appearance ───────────────────────────────── */}
            {page === "Appearance" && (
              <div className="settings__page">
                <SectionTitle>{t("Typography")}</SectionTitle>
                <SettingRow label={t("UI font")} description={t("Primary font for interface elements.")}>
                  <select
                    className="settings__select"
                    value={settings.uiFontFamily}
                    onChange={(e) => settings.setUiFontFamily(e.currentTarget.value)}
                  >
                    {UI_FONT_OPTIONS.map((f) => <option key={f} value={f}>{f}</option>)}
                  </select>
                </SettingRow>

                <SettingRow
                  label={t("Interface text scale")}
                  description={t("{scale}% — affects all UI text.", {
                    scale: (settings.uiScale * 100).toFixed(0),
                  })}
                >
                  <input
                    className="settings__slider"
                    type="range"
                    min={0.9}
                    max={1.2}
                    step={0.05}
                    value={settings.uiScale}
                    onChange={(e) => settings.setUiScale(Number(e.currentTarget.value))}
                  />
                </SettingRow>

                <SettingRow label={t("Code / mono font")} description={t("Used in terminal, code blocks, and tables.")}>
                  <select
                    className="settings__select"
                    value={settings.monoFontFamily}
                    onChange={(e) => settings.setMonoFontFamily(e.currentTarget.value)}
                  >
                    {MONO_FONT_OPTIONS.map((f) => <option key={f} value={f}>{f}</option>)}
                  </select>
                </SettingRow>

                {/* Live preview */}
                <SectionTitle>{t("Preview")}</SectionTitle>
                <div className="settings__preview-card">
                  <p style={{ fontFamily: `"${settings.uiFontFamily}", system-ui`, fontSize: `${13 * settings.uiScale}px` }}>
                    {t("The quick brown fox jumps over the lazy dog — Bold text")}
                  </p>
                  <p style={{ fontFamily: `"${settings.monoFontFamily}", monospace`, fontSize: "13px", color: "var(--text-secondary)" }}>
                    {'const result = await query("SELECT * FROM users");'}
                  </p>
                </div>
              </div>
            )}

            {/* ── Terminal ─────────────────────────────────── */}
            {page === "Terminal" && (
              <div className="settings__page">
                <SectionTitle>{t("Terminal Theme")}</SectionTitle>
                <div className="settings__theme-grid">
                  {TERMINAL_THEMES.map((th, i) => (
                    <button
                      key={th.name}
                      className={theme.terminalThemeIndex === i ? "settings__theme-card settings__theme-card--selected" : "settings__theme-card"}
                      onClick={() => theme.setTerminalTheme(i)}
                      type="button"
                    >
                      <div className="settings__theme-preview" style={{ background: th.bg, color: th.fg }}>
                        <span style={{ color: th.ansi[2] }}>~</span>
                        <span style={{ color: th.ansi[4] }}> $ </span>
                        <span style={{ color: th.fg }}>echo </span>
                        <span style={{ color: th.ansi[3] }}>"hello"</span>
                      </div>
                      <span className="settings__theme-name">{th.name}</span>
                    </button>
                  ))}
                </div>

                <SectionTitle>{t("Font")}</SectionTitle>
                <SettingRow label={t("Font family")} description={t("Monospace font used in the terminal.")}>
                  <select
                    className="settings__select"
                    value={settings.monoFontFamily}
                    onChange={(e) => settings.setMonoFontFamily(e.currentTarget.value)}
                  >
                    {MONO_FONT_OPTIONS.map((f) => <option key={f} value={f}>{f}</option>)}
                  </select>
                </SettingRow>

                <SettingRow
                  label={t("Font size")}
                  description={t("{size}px", { size: settings.terminalFontSize })}
                >
                  <input
                    className="settings__slider"
                    type="range"
                    min={9}
                    max={24}
                    step={1}
                    value={settings.terminalFontSize}
                    onChange={(e) => settings.setTerminalFontSize(Number(e.currentTarget.value))}
                  />
                </SettingRow>

                <SectionTitle>{t("Cursor")}</SectionTitle>
                <SettingRow label={t("Cursor style")}>
                  <SegmentedControl
                    options={[
                      { label: t("Block"), value: 0 },
                      { label: t("Beam"), value: 1 },
                      { label: t("Underline"), value: 2 },
                    ]}
                    value={settings.cursorStyle}
                    onChange={(v) => settings.setCursorStyle(v as 0 | 1 | 2)}
                  />
                </SettingRow>

                <SettingRow label={t("Cursor blink")} description={t("Animate the cursor to attract attention.")}>
                  <Toggle checked={settings.cursorBlink} onChange={settings.setCursorBlink} />
                </SettingRow>

                <SectionTitle>{t("Scrollback")}</SectionTitle>
                <SettingRow
                  label={t("Buffer lines")}
                  description={t("{lines} lines of history kept in memory.", {
                    lines: settings.scrollbackLines.toLocaleString(),
                  })}
                >
                  <input
                    className="settings__number-input"
                    type="number"
                    min={1000}
                    max={100000}
                    step={1000}
                    value={settings.scrollbackLines}
                    onChange={(e) => settings.setScrollbackLines(Number(e.currentTarget.value))}
                  />
                </SettingRow>

                <SectionTitle>{t("Bell")}</SectionTitle>
                <SettingRow label={t("Visual bell")} description={t("Flash the terminal border on bell character.")}>
                  <Toggle checked={settings.visualBell} onChange={settings.setVisualBell} />
                </SettingRow>
                <SettingRow label={t("Audio bell")} description={t("Play a system sound on bell character.")}>
                  <Toggle checked={settings.audioBell} onChange={settings.setAudioBell} />
                </SettingRow>
              </div>
            )}

          </div>
        </div>

        {/* Footer */}
        <div className="dialog__footer">
          <button className="welcome__btn welcome__btn--primary" onClick={onClose} type="button">
            {t("Close")}
          </button>
        </div>
      </div>
    </div>
  );
}
