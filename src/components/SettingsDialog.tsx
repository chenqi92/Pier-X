import { Fragment, useState } from "react";
import {
  Check,
  FileText,
  Keyboard,
  Server,
  Settings as SettingsIcon,
  Sun,
  Terminal as TerminalIcon,
  X,
} from "lucide-react";
import type { ComponentType, SVGProps } from "react";
import IconButton from "./IconButton";
import { useI18n } from "../i18n/useI18n";
import {
  useThemeStore,
  TERMINAL_THEMES,
  type AccentName,
  type Density,
} from "../stores/useThemeStore";
import { useConnectionStore } from "../stores/useConnectionStore";
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

type Page = "Appearance" | "Typography" | "Terminal" | "Connections" | "General";

type NavEntry = {
  key: Page;
  icon: ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;
};
type NavGroup = { label: string; items: NavEntry[] };

const NAV_GROUPS: NavGroup[] = [
  {
    label: "General",
    items: [
      { key: "Appearance", icon: Sun },
      { key: "Typography", icon: FileText },
      { key: "Terminal", icon: TerminalIcon },
    ],
  },
  {
    label: "Integrations",
    items: [{ key: "Connections", icon: Server }],
  },
  {
    label: "System",
    items: [{ key: "General", icon: Keyboard }],
  },
];

// ── Reusable sub-components ─────────────────────────────────────

function SectionTitle({ children }: { children: React.ReactNode }) {
  return <div className="settings__section-title">{children}</div>;
}

function SettingRow({
  label,
  description,
  children,
}: {
  label: string;
  description?: string;
  children: React.ReactNode;
}) {
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

function SegmentedControl({
  options,
  value,
  onChange,
}: {
  options: { label: string; value: string | number }[];
  value: string | number;
  onChange: (v: string | number) => void;
}) {
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

const ACCENT_OPTIONS: { name: AccentName; label: string; cls: string }[] = [
  { name: "blue", label: "Blue", cls: "swatch-blue" },
  { name: "green", label: "Green", cls: "swatch-green" },
  { name: "amber", label: "Amber", cls: "swatch-amber" },
  { name: "violet", label: "Violet", cls: "swatch-violet" },
  { name: "coral", label: "Coral", cls: "swatch-coral" },
];

function AccentSwatches({
  value,
  onChange,
}: {
  value: AccentName;
  onChange: (accent: AccentName) => void;
}) {
  return (
    <div className="swatches">
      {ACCENT_OPTIONS.map((opt) => (
        <button
          key={opt.name}
          type="button"
          title={opt.label}
          className={`${opt.cls}${value === opt.name ? " is-active" : ""}`}
          onClick={() => onChange(opt.name)}
        />
      ))}
    </div>
  );
}

// ── Main dialog ─────────────────────────────────────────────────

export default function SettingsDialog({ open, onClose }: Props) {
  const { t } = useI18n();
  const [page, setPage] = useState<Page>("Appearance");
  const theme = useThemeStore();
  const settings = useSettingsStore();
  const { connections, remove } = useConnectionStore();

  if (!open) return null;

  return (
    <div className="cmdp-overlay" onClick={onClose}>
      <div className="dlg dlg--settings" onClick={(e) => e.stopPropagation()}>
        {/* Header */}
        <div className="dlg-head">
          <span className="dlg-title">
            <SettingsIcon size={13} />
            {t("Settings")}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>

        <div className="dlg-body">
          <nav className="dlg-nav">
            {NAV_GROUPS.map((group) => (
              <Fragment key={group.label}>
                <div className="dlg-nav-group">{t(group.label)}</div>
                {group.items.map(({ key, icon: Icon }) => (
                  <button
                    key={key}
                    className={"dlg-nav-btn" + (page === key ? " active" : "")}
                    onClick={() => setPage(key)}
                    type="button"
                  >
                    <Icon size={13} />
                    <span>{t(key)}</span>
                  </button>
                ))}
              </Fragment>
            ))}
          </nav>

          <div className="dlg-pane">
            {/* ── Appearance ───────────────────────────────── */}
            {page === "Appearance" && (
              <div className="settings__page">
                <SectionTitle>{t("Theme")}</SectionTitle>
                <SettingRow
                  label={t("Color scheme")}
                  description={t("Dark is the native medium; light is a faithful mirror.")}
                >
                  <SegmentedControl
                    options={[
                      { label: t("Dark"), value: "dark" },
                      { label: t("Light"), value: "light" },
                      { label: t("System"), value: "system" },
                    ]}
                    value={theme.mode}
                    onChange={(v) => theme.setMode(v as "dark" | "light" | "system")}
                  />
                </SettingRow>

                <SettingRow
                  label={t("Accent")}
                  description={t("One chromatic accent — applies everywhere.")}
                >
                  <AccentSwatches value={theme.accent} onChange={theme.setAccent} />
                </SettingRow>

                <SettingRow
                  label={t("Density")}
                  description={t("Compact is the IDE default; Comfortable adds 2–4px of air.")}
                >
                  <SegmentedControl
                    options={[
                      { label: t("Compact"), value: "compact" },
                      { label: t("Comfortable"), value: "comfortable" },
                    ]}
                    value={theme.density}
                    onChange={(v) => theme.setDensity(v as Density)}
                  />
                </SettingRow>
              </div>
            )}

            {/* ── Typography ───────────────────────────────── */}
            {page === "Typography" && (
              <div className="settings__page">
                <SectionTitle>{t("Typography")}</SectionTitle>
                <SettingRow label={t("UI font")} description={t("Primary font for interface elements.")}>
                  <select
                    className="settings__select"
                    value={settings.uiFontFamily}
                    onChange={(e) => settings.setUiFontFamily(e.currentTarget.value)}
                  >
                    {UI_FONT_OPTIONS.map((f) => (
                      <option key={f} value={f}>
                        {f}
                      </option>
                    ))}
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
                    {MONO_FONT_OPTIONS.map((f) => (
                      <option key={f} value={f}>
                        {f}
                      </option>
                    ))}
                  </select>
                </SettingRow>

                <SectionTitle>{t("Preview")}</SectionTitle>
                <div className="settings__preview-card">
                  <p style={{ fontFamily: `"${settings.uiFontFamily}", var(--sans)`, fontSize: `${13 * settings.uiScale}px` }}>
                    {t("The quick brown fox jumps over the lazy dog — Bold text")}
                  </p>
                  <p
                    className="mono text-muted"
                    style={{ fontFamily: `"${settings.monoFontFamily}", var(--mono)`, fontSize: "13px" }}
                  >
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
                      className={
                        theme.terminalThemeIndex === i
                          ? "settings__theme-card settings__theme-card--selected"
                          : "settings__theme-card"
                      }
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
                    {MONO_FONT_OPTIONS.map((f) => (
                      <option key={f} value={f}>
                        {f}
                      </option>
                    ))}
                  </select>
                </SettingRow>

                <SettingRow label={t("Font size")} description={t("{size}px", { size: settings.terminalFontSize })}>
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

                <SectionTitle>{t("Display")}</SectionTitle>
                <SettingRow
                  label={t("Row separators")}
                  description={t("Draw a 1px divider between terminal rows — off by default.")}
                >
                  <Toggle
                    checked={settings.terminalRowSeparators}
                    onChange={settings.setTerminalRowSeparators}
                  />
                </SettingRow>
              </div>
            )}

            {/* ── Connections ──────────────────────────────── */}
            {page === "Connections" && (
              <div className="settings__page">
                <SectionTitle>
                  {t("Saved SSH connections")}
                  <span className="settings__badge">{connections.length}</span>
                </SectionTitle>
                {connections.length === 0 ? (
                  <div className="empty-note">
                    {t("No saved connections yet. Add one from the Servers sidebar.")}
                  </div>
                ) : (
                  <div className="settings__conn-list">
                    {connections.map((conn) => (
                      <div key={`${conn.index}-${conn.name}`} className="settings__conn-card">
                        <div className="settings__conn-header">
                          <strong>{conn.name}</strong>
                          <span className="settings__conn-auth">{conn.authKind}</span>
                        </div>
                        <div className="settings__conn-meta">
                          {conn.user}@{conn.host}:{conn.port}
                        </div>
                        <div className="settings__conn-actions">
                          <button
                            className="mini-button mini-button--destructive"
                            onClick={() => void remove(conn.index).catch(() => {})}
                            type="button"
                          >
                            {t("Remove")}
                          </button>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}

            {/* ── General ─────────────────────────────────── */}
            {page === "General" && (
              <div className="settings__page">
                <SectionTitle>{t("Language")}</SectionTitle>
                <SettingRow label={t("Interface language")} description={t("Changes apply immediately to all UI text.")}>
                  <SegmentedControl
                    options={[
                      { label: "English", value: "en" },
                      { label: "简体中文", value: "zh" },
                    ]}
                    value={settings.locale}
                    onChange={(v) => settings.setLocale(v as Locale)}
                  />
                </SettingRow>

                <SectionTitle>{t("Developer")}</SectionTitle>
                <SettingRow
                  label={t("Performance overlay")}
                  description={t("Show FPS and memory usage in the status bar.")}
                >
                  <Toggle
                    checked={settings.performanceOverlay}
                    onChange={settings.setPerformanceOverlay}
                  />
                </SettingRow>
              </div>
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="dlg-foot">
          <span className="dlg-foot-hint">
            <Check size={11} />
            {t("Changes save automatically")}
          </span>
          <div style={{ flex: 1 }} />
          <button className="gb-btn primary" onClick={onClose} type="button">
            {t("Done")}
          </button>
        </div>
      </div>
    </div>
  );
}
