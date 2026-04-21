import { useEffect, useRef, useState } from "react";
import { CircleDot, GitBranch, Terminal } from "lucide-react";
import type { TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useStatusStore } from "../stores/useStatusStore";

type Props = {
  version?: string;
  coreInfo?: string;
  activeTab?: TabState | null;
};

function backendLabel(tab: TabState | null | undefined): string {
  if (!tab) return "local · zsh";
  switch (tab.backend) {
    case "ssh":
      return "ssh · russh";
    case "sftp":
      return "sftp · russh";
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
  const branch = useStatusStore((s) => s.branch);
  const ahead = useStatusStore((s) => s.ahead);
  const behind = useStatusStore((s) => s.behind);
  const terminalCols = useStatusStore((s) => s.terminalCols);
  const terminalRows = useStatusStore((s) => s.terminalRows);
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

  const perfTone = fps >= 50 ? "pos" : fps >= 30 ? "accent" : "warn";
  const branchLabel = branch ?? t("no repo");
  const sizeLabel =
    terminalCols != null && terminalRows != null
      ? `${terminalCols} × ${terminalRows}`
      : null;

  return (
    <footer className="statusbar">
      <span className="sb-item">
        <GitBranch size={10} />
        <span>{branchLabel}</span>
      </span>
      <span className="sb-item text-muted">
        {`↑${ahead} ↓${behind}`}
      </span>
      <span className="sb-item">
        <Terminal size={10} />
        <span>{backendLabel(activeTab)}</span>
      </span>
      {sizeLabel ? (
        <span className="sb-item text-muted">{sizeLabel}</span>
      ) : null}
      <span className="sb-spacer" />
      {showPerf && (
        <span className={`sb-item ${perfTone}`}>
          {t("{fps} FPS", { fps })}
        </span>
      )}
      <span className="sb-item">
        <span>PANEL · {rightToolLabel(activeTab)}</span>
      </span>
      <span className="sb-item">UTF-8</span>
      <span className="sb-item pos">
        <CircleDot size={10} />
        READY
      </span>
      {version ? (
        <span className="sb-item text-muted">
          Pier-X v{version}
          {coreInfo ? ` · ${coreInfo}` : ""}
        </span>
      ) : null}
    </footer>
  );
}
