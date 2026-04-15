import { useEffect, useRef, type MouseEvent as ReactMouseEvent } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
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
const APP_WINDOW = isTauri() ? getCurrentWindow() : null;
const DRAG_THRESHOLD_PX = 4;

function shouldSkipWindowDrag(target: HTMLElement | null) {
  return !!target?.closest(
    "button, input, textarea, select, a, summary, [role='button'], [contenteditable='true'], [data-no-window-drag='true']",
  );
}

export default function TopBar({ onNewTab, onSettings, onToggleTheme, version }: Props) {
  const { t } = useI18n();
  const { resolvedDark, mode } = useThemeStore();
  const dragCleanupRef = useRef<(() => void) | null>(null);

  useEffect(() => () => dragCleanupRef.current?.(), []);

  function handleMouseDown(event: ReactMouseEvent<HTMLElement>) {
    if (!APP_WINDOW || event.button !== 0) {
      return;
    }

    const target = event.target instanceof HTMLElement ? event.target : null;
    if (shouldSkipWindowDrag(target) || event.detail > 1) {
      return;
    }

    dragCleanupRef.current?.();

    const startX = event.screenX;
    const startY = event.screenY;
    let dragStarted = false;

    const cleanup = () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", cleanup);
      dragCleanupRef.current = null;
    };

    const handleMouseMove = (moveEvent: MouseEvent) => {
      if (dragStarted) {
        return;
      }

      const movedX = Math.abs(moveEvent.screenX - startX);
      const movedY = Math.abs(moveEvent.screenY - startY);
      if (Math.max(movedX, movedY) < DRAG_THRESHOLD_PX) {
        return;
      }

      dragStarted = true;
      cleanup();
      void APP_WINDOW.startDragging().catch(() => {});
    };

    dragCleanupRef.current = cleanup;
    window.addEventListener("mousemove", handleMouseMove, { passive: true });
    window.addEventListener("mouseup", cleanup, { once: true });
  }

  function handleDoubleClick(event: ReactMouseEvent<HTMLElement>) {
    if (!APP_WINDOW || event.button !== 0) {
      return;
    }

    const target = event.target instanceof HTMLElement ? event.target : null;
    if (shouldSkipWindowDrag(target)) {
      return;
    }

    dragCleanupRef.current?.();
    void APP_WINDOW.toggleMaximize().catch(() => {});
  }

  return (
    <header
      className="topbar"
      data-tauri-drag-region
      onDoubleClick={handleDoubleClick}
      onMouseDown={handleMouseDown}
    >
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
