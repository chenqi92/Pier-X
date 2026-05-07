import { useEffect, useMemo, useRef, useState } from "react";
import { CircleDot, GitBranch, Loader2, Network, Terminal } from "lucide-react";
import type { RightTool, TabState } from "../lib/types";
import { effectiveShellUser, effectiveSshTarget } from "../lib/types";
import { buildEgressChain, describeHop, formatChain } from "../lib/egressChain";
import { useI18n } from "../i18n/useI18n";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useEgressStore } from "../stores/useEgressStore";
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
  const egressProfiles = useEgressStore((s) => s.profiles);
  const refreshEgress = useEgressStore((s) => s.refresh);
  const connections = useConnectionStore((s) => s.connections);

  // The egress store is dialog-driven elsewhere (NewConnectionDialog,
  // EgressProfilesDialog). Pull once on mount so the badge has data
  // even when the user never opened those dialogs in this session.
  useEffect(() => {
    if (egressProfiles.length === 0) {
      void refreshEgress();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const egressBadge = useMemo(() => {
    if (!activeTab) return null;
    const sshTarget = effectiveSshTarget(activeTab);
    if (!sshTarget) return null;
    const conn = connections.find((c) => c.index === sshTarget.savedConnectionIndex);
    if (!conn) return null;

    // Active DB credential takes precedence over the SSH connection's
    // own egress: when a credential carries an egressId the DB connect
    // path skips the parent SSH tunnel and dials directly through the
    // credential's profile (see useDbCredentialFlow.ensureConnectionTarget).
    const credId =
      activeTab.pgActiveCredentialId ??
      activeTab.mysqlActiveCredentialId ??
      activeTab.redisActiveCredentialId ??
      null;
    let rootEgressId: string | null | undefined = conn.egressId;
    let targetLabel = `${effectiveShellUser(activeTab, sshTarget)}@${sshTarget.host}:${sshTarget.port}`;
    if (credId) {
      const cred = (conn.databases ?? []).find((d) => d.id === credId);
      if (cred?.egressId) {
        rootEgressId = cred.egressId;
        targetLabel = `${cred.host}:${cred.port}`;
      }
    }
    const hops = buildEgressChain(rootEgressId, egressProfiles, connections);
    if (hops.length === 0) return null;
    return {
      compact: formatChain(hops, targetLabel),
      tooltip: hops.map(describeHop).join("\n") + `\n→ ${targetLabel}`,
    };
  }, [activeTab, connections, egressProfiles]);

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
      {egressBadge ? (
        <span className="sb-item accent" title={egressBadge.tooltip}>
          <Network size={10} />
          <span>{egressBadge.compact}</span>
        </span>
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
