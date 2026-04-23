import { useEffect, useRef, useState } from "react";
import { CircleDot, GitBranch, Loader2, Terminal } from "lucide-react";
import type { RightTool, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useStatusStore } from "../stores/useStatusStore";
import { useTaskStore } from "../stores/useTaskStore";

type Props = {
  version?: string;
  coreInfo?: string;
  activeTab?: TabState | null;
  activeTool?: RightTool;
};

function rightToolLabel(activeTool: RightTool | undefined, tab: TabState | null | undefined): string {
  return (activeTool ?? tab?.rightTool ?? "markdown").toUpperCase();
}

export default function StatusBar({ version, coreInfo, activeTab, activeTool }: Props) {
  const { t } = useI18n();
  const showPerf = useSettingsStore((s) => s.performanceOverlay);
  const branch = useStatusStore((s) => s.branch);
  const ahead = useStatusStore((s) => s.ahead);
  const behind = useStatusStore((s) => s.behind);
  const tasks = useTaskStore((s) => s.tasks);
  const setTrayOpen = useTaskStore((s) => s.setTrayOpen);
  const runningCount = tasks.filter((t) => t.status === "running").length;
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
  const backendLabel = !activeTab
    ? t("local · zsh")
    : activeTab.backend === "ssh"
      ? t("ssh · russh")
      : activeTab.backend === "sftp"
        ? t("sftp · russh")
        : activeTab.backend === "markdown"
          ? t("markdown preview")
          : t("local · zsh");
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
        <span>{backendLabel}</span>
      </span>
      {sizeLabel ? (
        <span className="sb-item text-muted">{sizeLabel}</span>
      ) : null}
      <span className="sb-spacer" />
      {tasks.length > 0 && (
        <button
          type="button"
          className="sb-item"
          onClick={() => setTrayOpen(true)}
          style={{ background: "transparent", border: "none", cursor: "pointer", padding: 0, color: "inherit", display: "inline-flex", alignItems: "center", gap: 4 }}
          title={t("Open task tray")}
        >
          {runningCount > 0 ? (
            <Loader2 size={10} className="ftp-spin" color="var(--accent)" />
          ) : (
            <CircleDot size={10} color="var(--muted)" />
          )}
          <span>
            {runningCount > 0
              ? t("{running} running · {total} total", { running: runningCount, total: tasks.length })
              : t("{total} tasks", { total: tasks.length })}
          </span>
        </button>
      )}
      {showPerf && (
        <span className={`sb-item ${perfTone}`}>
          {t("{fps} FPS", { fps })}
        </span>
      )}
      <span className="sb-item">
        <span>{t("PANEL")} · {rightToolLabel(activeTool, activeTab)}</span>
      </span>
      <span className="sb-item">UTF-8</span>
      <span className="sb-item pos">
        <CircleDot size={10} />
        {t("Ready")}
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
