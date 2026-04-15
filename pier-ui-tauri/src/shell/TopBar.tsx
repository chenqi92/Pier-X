import { Moon, Plus, Settings, Sun } from "lucide-react";
import { useI18n } from "../i18n/useI18n";
import { useThemeStore } from "../stores/useThemeStore";

type Props = {
  onNewTab: () => void;
  onSettings: () => void;
  onToggleTheme: () => void;
  version?: string;
};

const IS_MAC = navigator.platform.includes("Mac");

export default function TopBar({ onNewTab, onSettings, onToggleTheme, version }: Props) {
  const { t } = useI18n();
  const { resolvedDark, mode } = useThemeStore();

  return (
    <header className="topbar" data-tauri-drag-region>
      {/* macOS traffic light spacer */}
      {IS_MAC && <span className="topbar__traffic-spacer" />}

      {/* Brand */}
      <span className="topbar__brand" data-tauri-drag-region>Pier-X</span>

      {/* Drag region fills center */}
      <div className="topbar__drag" data-tauri-drag-region />

      {/* Right controls */}
      <div className="topbar__right">
        {version && <span className="topbar__version">v{version}</span>}
        <button className="topbar__icon-btn" onClick={onNewTab} title={t("New tab")} type="button">
          <Plus size={14} />
        </button>
        <button
          className="topbar__icon-btn"
          onClick={onToggleTheme}
          title={mode === "system" ? t("Follow system") : resolvedDark ? t("Light") : t("Dark")}
          type="button"
        >
          {resolvedDark ? <Sun size={14} /> : <Moon size={14} />}
        </button>
        <button className="topbar__icon-btn" onClick={onSettings} title={t("Settings")} type="button">
          <Settings size={14} />
        </button>
      </div>
    </header>
  );
}
