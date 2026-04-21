import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Maximize2, Minimize2, Minus, X } from "lucide-react";
import { useI18n } from "../i18n/useI18n";

/**
 * Windows / Linux caption controls. macOS uses the OS's native traffic
 * lights (exposed through titleBarStyle="Overlay") so this component is
 * only mounted off-mac. Tauri's decoration flag is flipped to `false` on
 * Windows at runtime (see lib.rs) to keep the OS chrome from duplicating
 * these buttons.
 */
export default function WindowControls() {
  const { t } = useI18n();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const win = getCurrentWindow();
    void win.isMaximized().then(setMaximized).catch(() => {});
    const unlisten = win.onResized(() => {
      void win.isMaximized().then(setMaximized).catch(() => {});
    });
    return () => {
      void unlisten.then((fn) => fn()).catch(() => {});
    };
  }, []);

  const win = getCurrentWindow();

  return (
    <div className="winctl" data-no-window-drag="true">
      <button
        type="button"
        className="winctl__btn"
        title={t("Minimize")}
        onClick={() => void win.minimize().catch(() => {})}
      >
        <Minus size={14} />
      </button>
      <button
        type="button"
        className="winctl__btn"
        title={maximized ? t("Restore") : t("Maximize")}
        onClick={() => void win.toggleMaximize().catch(() => {})}
      >
        {maximized ? <Minimize2 size={12} /> : <Maximize2 size={12} />}
      </button>
      <button
        type="button"
        className="winctl__btn winctl__btn--close"
        title={t("Close")}
        onClick={() => void win.close().catch(() => {})}
      >
        <X size={14} />
      </button>
    </div>
  );
}
