import { Moon, Plus, Settings, Sun } from "lucide-react";
import { useI18n } from "../i18n/useI18n";
import { useThemeStore } from "../stores/useThemeStore";

type Props = {
  onNewTab: () => void;
  onSettings: () => void;
  version?: string;
};

export default function TopBar({ onNewTab, onSettings, version }: Props) {
  const { t } = useI18n();
  const { resolvedDark, setMode, mode } = useThemeStore();

  function toggleTheme() {
    setMode(resolvedDark ? "light" : "dark");
  }

  return (
    <header className="topbar">
      <div className="topbar__left">
        <span className="topbar__brand">Pier-X</span>
        <span className="topbar__divider" />
        <button className="topbar__menu-btn" type="button">{t("File")}</button>
        <button className="topbar__menu-btn" type="button">{t("Edit")}</button>
        <button className="topbar__menu-btn" type="button">{t("View")}</button>
        <button className="topbar__menu-btn" type="button">{t("Window")}</button>
        <button className="topbar__menu-btn" type="button">{t("Help")}</button>
      </div>
      <div className="topbar__drag" data-tauri-drag-region />
      <div className="topbar__right">
        {version ? <span className="topbar__version">v{version}</span> : null}
        <button className="topbar__icon-btn" onClick={onNewTab} title={t("New tab")} type="button">
          <Plus size={15} />
        </button>
        <button className="topbar__icon-btn" onClick={toggleTheme} title={mode === "system" ? t("Follow system") : resolvedDark ? t("Light") : t("Dark")} type="button">
          {resolvedDark ? <Sun size={15} /> : <Moon size={15} />}
        </button>
        <button className="topbar__icon-btn" onClick={onSettings} title={t("Settings")} type="button">
          <Settings size={15} />
        </button>
      </div>
    </header>
  );
}
