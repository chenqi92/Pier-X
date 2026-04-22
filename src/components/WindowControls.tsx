import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useI18n } from "../i18n/useI18n";

const glyphProps = {
  width: 10,
  height: 10,
  viewBox: "0 0 10 10",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1,
  shapeRendering: "crispEdges" as const,
  "aria-hidden": true,
};

const MinimizeGlyph = () => (
  <svg {...glyphProps}>
    <line x1="0" y1="5" x2="10" y2="5" />
  </svg>
);

const MaximizeGlyph = () => (
  <svg {...glyphProps}>
    <rect x="0.5" y="0.5" width="9" height="9" />
  </svg>
);

const RestoreGlyph = () => (
  <svg {...glyphProps}>
    <rect x="0.5" y="2.5" width="7" height="7" />
    <path d="M2.5 2.5 V0.5 H9.5 V7.5 H7.5" />
  </svg>
);

const CloseGlyph = () => (
  <svg {...glyphProps}>
    <path d="M0 0 L10 10 M10 0 L0 10" />
  </svg>
);

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
        <MinimizeGlyph />
      </button>
      <button
        type="button"
        className="winctl__btn"
        title={maximized ? t("Restore") : t("Maximize")}
        onClick={() => void win.toggleMaximize().catch(() => {})}
      >
        {maximized ? <RestoreGlyph /> : <MaximizeGlyph />}
      </button>
      <button
        type="button"
        className="winctl__btn winctl__btn--close"
        title={t("Close")}
        onClick={() => void win.close().catch(() => {})}
      >
        <CloseGlyph />
      </button>
    </div>
  );
}
