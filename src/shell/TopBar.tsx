import { useEffect, useRef, type MouseEvent as ReactMouseEvent } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Command, Moon, Plus, Settings, Sun } from "lucide-react";
import { useI18n } from "../i18n/useI18n";
import { useThemeStore } from "../stores/useThemeStore";
import TitlebarMenu, { type MenuDef } from "../components/TitlebarMenu";
import WindowControls from "../components/WindowControls";

type Props = {
  onNewTab: () => void;
  onSettings: () => void;
  onToggleTheme: () => void;
  onCommandPalette?: () => void;
  version?: string;
  /** App-menu definitions. Rendered on Windows/Linux; hidden on macOS. */
  menus?: MenuDef[];
};

const IS_MAC = navigator.platform.includes("Mac");
const APP_WINDOW = isTauri() ? getCurrentWindow() : null;
const DRAG_THRESHOLD_PX = 4;

function shouldSkipWindowDrag(target: HTMLElement | null) {
  return !!target?.closest(
    "button, input, textarea, select, a, summary, [role='button'], [contenteditable='true'], [data-no-window-drag='true']",
  );
}

export default function TopBar({
  onNewTab,
  onSettings,
  onToggleTheme,
  onCommandPalette,
  version,
  menus,
}: Props) {
  const { t } = useI18n();
  const { resolvedDark, mode } = useThemeStore();
  const dragCleanupRef = useRef<(() => void) | null>(null);

  useEffect(() => () => dragCleanupRef.current?.(), []);

  function handleMouseDown(event: ReactMouseEvent<HTMLElement>) {
    if (!APP_WINDOW || event.button !== 0) return;
    const target = event.target instanceof HTMLElement ? event.target : null;
    if (shouldSkipWindowDrag(target) || event.detail > 1) return;

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
      if (dragStarted) return;
      const movedX = Math.abs(moveEvent.screenX - startX);
      const movedY = Math.abs(moveEvent.screenY - startY);
      if (Math.max(movedX, movedY) < DRAG_THRESHOLD_PX) return;
      dragStarted = true;
      cleanup();
      void APP_WINDOW.startDragging().catch(() => {});
    };

    dragCleanupRef.current = cleanup;
    window.addEventListener("mousemove", handleMouseMove, { passive: true });
    window.addEventListener("mouseup", cleanup, { once: true });
  }

  function handleDoubleClick(event: ReactMouseEvent<HTMLElement>) {
    if (!APP_WINDOW || event.button !== 0) return;
    const target = event.target instanceof HTMLElement ? event.target : null;
    if (shouldSkipWindowDrag(target)) return;
    dragCleanupRef.current?.();
    void APP_WINDOW.toggleMaximize().catch(() => {});
  }

  const themeTitle = mode === "system" ? t("Follow system") : resolvedDark ? t("Light") : t("Dark");

  return (
    <header
      className={`titlebar${IS_MAC ? " is-mac" : ""}`}
      data-tauri-drag-region
      onDoubleClick={handleDoubleClick}
      onMouseDown={handleMouseDown}
    >
      {/* macOS: titleBarStyle="Overlay" already renders the native traffic
       * lights; drawing our own on top caused a double-circle overlap. We
       * just reserve left padding via the `.is-mac` modifier below. */}
      <div className="brand">
        <span className="brand-mark">
          <img src="/pier-icon.png" alt="" width={18} height={18} draggable={false} />
        </span>
        <span>
          Pier-X {version ? <em>{version}</em> : null}
        </span>
      </div>

      {/* macOS uses the native global menu bar at the top of the screen;
       * on Windows/Linux there is no such bar so we draw our own. */}
      {!IS_MAC && menus && menus.length > 0 ? (
        <TitlebarMenu menus={menus} />
      ) : null}

      <div className="titlebar-spacer" data-tauri-drag-region />

      <div className="titlebar-actions">
        {onCommandPalette ? (
          <button
            className="icon-btn"
            onClick={onCommandPalette}
            title={t("Command palette (⌘K)")}
          >
            <Command size={13} />
          </button>
        ) : null}
        <button className="icon-btn" onClick={onNewTab} title={t("New tab")}>
          <Plus size={14} />
        </button>
        <button className="icon-btn" onClick={onToggleTheme} title={themeTitle}>
          {resolvedDark ? <Sun size={14} /> : <Moon size={14} />}
        </button>
        <button className="icon-btn" onClick={onSettings} title={t("Settings")}>
          <Settings size={14} />
        </button>
      </div>

      {/* macOS gets its caption controls from the OS (traffic lights on
       * the left); Windows/Linux get our own min/max/close on the right. */}
      {!IS_MAC ? <WindowControls /> : null}
    </header>
  );
}
