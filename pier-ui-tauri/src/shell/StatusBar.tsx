import { useEffect, useRef, useState } from "react";
import { CircleDot, GitBranch, Terminal } from "lucide-react";
import type { TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { useSettingsStore } from "../stores/useSettingsStore";

type Props = {
  version?: string;
  coreInfo?: string;
  activeTab?: TabState | null;
};

function backendLabel(tab: TabState | null | undefined): string {
  if (!tab) return "local · zsh";
  switch (tab.backend) {
    case "ssh":
      return `ssh · ${tab.sshHost ?? ""}`;
    case "markdown":
      return "markdown preview";
    default:
      return "local · zsh";
  }
}

function rightToolLabel(tab: TabState | null | undefined): string {
  const tool = tab?.rightTool;
  if (!tool) return "GIT";
  return tool.toUpperCase();
}

export default function StatusBar({ version, coreInfo, activeTab }: Props) {
  const { t } = useI18n();
  const showPerf = useSettingsStore((s) => s.performanceOverlay);
  const [fps, setFps] = useState(0);

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

  const perfTone = fps >= 50 ? "is-pos" : fps >= 30 ? "is-accent" : "is-warn";

  return (
    <footer className="statusbar">
      <span className="statusbar__segment">
        <GitBranch size={10} />
        <span>{t("Ready")}</span>
      </span>
      <span className="statusbar__segment">
        <Terminal size={10} />
        <span>{backendLabel(activeTab)}</span>
      </span>
      <span className="statusbar__spacer" />
      {showPerf && (
        <span className={`statusbar__segment ${perfTone}`}>
          {t("{fps} FPS", { fps })}
        </span>
      )}
      <span className="statusbar__segment">
        <span>PANEL · {rightToolLabel(activeTab)}</span>
      </span>
      <span className="statusbar__segment">UTF-8</span>
      <span className="statusbar__segment is-pos">
        <CircleDot size={10} />
        READY
      </span>
      {version ? (
        <span className="statusbar__segment">
          Pier-X v{version}
          {coreInfo ? ` · ${coreInfo}` : ""}
        </span>
      ) : null}
    </footer>
  );
}
