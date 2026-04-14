import { useEffect, useRef, useState } from "react";
import { useI18n } from "../i18n/useI18n";
import { useSettingsStore } from "../stores/useSettingsStore";

type Props = {
  version?: string;
  coreInfo?: string;
};

export default function StatusBar({ version, coreInfo }: Props) {
  const { t } = useI18n();
  const showPerf = useSettingsStore((s) => s.performanceOverlay);
  const [fps, setFps] = useState(0);

  // FPS counter
  const frameCountRef = useRef(0);
  const lastTimeRef = useRef(performance.now());

  useEffect(() => {
    if (!showPerf) return;
    let rafId: number;
    const tick = () => {
      frameCountRef.current++;
      const now = performance.now();
      if (now - lastTimeRef.current >= 1000) {
        setFps(frameCountRef.current);
        frameCountRef.current = 0;
        lastTimeRef.current = now;
      }
      rafId = requestAnimationFrame(tick);
    };
    rafId = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafId);
  }, [showPerf]);

  const fpsColor =
    fps >= 50 ? "var(--status-success)" : fps >= 30 ? "var(--status-info)" : "var(--status-warning)";

  return (
    <footer className="statusbar">
      <span className="statusbar__text">{t("Ready")}</span>
      <span className="statusbar__spacer" />
      {showPerf && (
        <span className="statusbar__meta" style={{ color: fpsColor }}>
          {t("{fps} FPS", { fps })}
        </span>
      )}
      {version ? (
        <span className="statusbar__meta">
          v{version}{coreInfo ? ` · ${coreInfo}` : ""}
        </span>
      ) : null}
    </footer>
  );
}
