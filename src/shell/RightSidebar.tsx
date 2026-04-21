import type { RightTool, TabState } from "../lib/types";
import { useEffect, useMemo } from "react";
import {
  Activity,
  Container,
  Database,
  FileText,
  FolderTree,
  GitBranch,
  Scroll,
  Zap,
} from "lucide-react";
import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { mapServiceToTool, useDetectedServicesStore } from "../stores/useDetectedServicesStore";
import { useStatusStore } from "../stores/useStatusStore";
import GitPanel from "../panels/GitPanel";
import MySqlPanel from "../panels/MySqlPanel";
import PostgresPanel from "../panels/PostgresPanel";
import SqlitePanel from "../panels/SqlitePanel";
import RedisPanel from "../panels/RedisPanel";
import DockerPanel from "../panels/DockerPanel";
import SftpPanel from "../panels/SftpPanel";
import ServerMonitorPanel from "../panels/ServerMonitorPanel";
import MarkdownPanel from "../panels/MarkdownPanel";
import LogViewerPanel from "../panels/LogViewerPanel";
import ToolStrip from "./ToolStrip";
import ConnectSplash from "../components/ConnectSplash";
import PanelHeader from "../components/PanelHeader";

type Props = {
  activeTab: TabState | null;
  /** Resolved right tool (falls back to app-level state when no tab is open). */
  activeTool: RightTool;
  browserPath: string;
  selectedMarkdownPath: string;
  onToolChange: (tool: RightTool) => void;
  onConnectSaved: (index: number) => void;
  onNewConnection: () => void;
  /** App-owned collapse state so the outer grid can reclaim right-panel width. */
  collapsed: boolean;
  onToggleCollapsed: () => void;
};

type SplashMeta = {
  icon: typeof Activity;
  title: string;
  subtitle: string;
  tintVar: string;
};

const SPLASH_META: Record<
  "monitor" | "docker" | "mysql" | "postgres" | "redis" | "log" | "sftp",
  SplashMeta
> = {
  monitor: {
    icon: Activity,
    title: "Server Monitor",
    subtitle: "Open a saved server to see live CPU, memory, disks, and top processes.",
    tintVar: "var(--svc-monitor)",
  },
  docker: {
    icon: Container,
    title: "Docker",
    subtitle: "Pick a host to list containers, images, networks, and compose stacks.",
    tintVar: "var(--svc-docker)",
  },
  mysql: {
    icon: Database,
    title: "MySQL",
    subtitle: "Connect through SSH to browse databases, run queries, and edit rows.",
    tintVar: "var(--svc-mysql)",
  },
  postgres: {
    icon: Database,
    title: "PostgreSQL",
    subtitle: "Connect through SSH to explore schemas, tables, and run SQL.",
    tintVar: "var(--svc-postgres)",
  },
  redis: {
    icon: Zap,
    title: "Redis",
    subtitle: "Tunnel into a host to browse keyspaces, inspect values, and tail keys.",
    tintVar: "var(--svc-redis)",
  },
  log: {
    icon: Scroll,
    title: "Log Viewer",
    subtitle: "Stream journal, nginx, or custom log tails from a saved server.",
    tintVar: "var(--svc-log)",
  },
  sftp: {
    icon: FolderTree,
    title: "SFTP",
    subtitle: "Browse a remote filesystem, preview files, and transfer in either direction.",
    tintVar: "var(--svc-sftp)",
  },
};

function renderSplash(
  kind: keyof typeof SPLASH_META,
  t: (s: string) => string,
  onConnectSaved: (index: number) => void,
  onNewConnection: () => void,
) {
  const m = SPLASH_META[kind];
  const Icon = m.icon;
  return (
    <ConnectSplash
      icon={<Icon size={22} strokeWidth={1.6} />}
      title={t(m.title)}
      subtitle={t(m.subtitle)}
      tintVar={m.tintVar}
      tagLabel={t("SSH")}
      onConnectSaved={onConnectSaved}
      onNewConnection={onNewConnection}
    />
  );
}

function toolTitle(tool: RightTool, t: (s: string) => string) {
  switch (tool) {
    case "git":
      return t("Git");
    case "markdown":
      return t("Markdown");
    case "monitor":
      return t("Server Monitor");
    case "docker":
      return t("Docker");
    case "mysql":
      return t("MySQL");
    case "postgres":
      return t("PostgreSQL");
    case "redis":
      return t("Redis");
    case "log":
      return t("Logs");
    case "sftp":
      return t("SFTP");
    case "sqlite":
      return t("SQLite");
    default:
      return String(tool);
  }
}

function ToolContent({
  tool,
  tab,
  browserPath,
  markdownPath,
  unknownToolLabel,
  onConnectSaved,
  onNewConnection,
  t,
}: {
  tool: RightTool;
  tab: TabState | null;
  browserPath: string;
  markdownPath: string;
  unknownToolLabel: string;
  onConnectSaved: (index: number) => void;
  onNewConnection: () => void;
  t: (s: string) => string;
}) {
  const tabKey = tab?.id ?? "no-tab";
  switch (tool) {
    case "git":
      return <GitPanel key={tabKey} browserPath={browserPath} />;
    case "monitor":
      return tab ? <ServerMonitorPanel key={tab.id} tab={tab} /> : renderSplash("monitor", t, onConnectSaved, onNewConnection);
    case "docker":
      return tab ? <DockerPanel key={tab.id} tab={tab} /> : renderSplash("docker", t, onConnectSaved, onNewConnection);
    case "mysql":
      return tab ? <MySqlPanel key={tab.id} tab={tab} /> : renderSplash("mysql", t, onConnectSaved, onNewConnection);
    case "postgres":
      return tab ? <PostgresPanel key={tab.id} tab={tab} /> : renderSplash("postgres", t, onConnectSaved, onNewConnection);
    case "redis":
      return tab ? <RedisPanel key={tab.id} tab={tab} /> : renderSplash("redis", t, onConnectSaved, onNewConnection);
    case "log":
      return tab ? <LogViewerPanel key={tab.id} tab={tab} /> : renderSplash("log", t, onConnectSaved, onNewConnection);
    case "sftp":
      return tab ? <SftpPanel key={tab.id} tab={tab} /> : renderSplash("sftp", t, onConnectSaved, onNewConnection);
    case "sqlite":
      return <SqlitePanel key={tabKey} />;
    case "markdown":
      return <MarkdownPanel key={markdownPath} filePath={markdownPath} />;
    default:
      return <div className="empty-note">{unknownToolLabel}</div>;
  }
}

function basename(path: string) {
  if (!path) return "";
  const index = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return index >= 0 ? path.slice(index + 1) : path;
}

function rightHeaderMeta(
  tool: RightTool,
  browserPath: string,
  selectedMarkdownPath: string,
  branch: string | null,
  ahead: number,
  behind: number,
) {
  if (tool === "markdown") {
    return selectedMarkdownPath ? basename(selectedMarkdownPath) : undefined;
  }
  if (tool === "git" && branch) {
    return `${branch}${ahead ? ` · ↑${ahead}` : ""}${behind ? ` · ↓${behind}` : ""}`;
  }
  if (tool === "git") {
    return basename(browserPath);
  }
  return undefined;
}

function rightHeaderIcon(tool: RightTool) {
  switch (tool) {
    case "git":
      return GitBranch;
    case "markdown":
      return FileText;
    default:
      return undefined;
  }
}

export default function RightSidebar({
  activeTab,
  activeTool,
  browserPath,
  selectedMarkdownPath,
  onToolChange,
  onConnectSaved,
  onNewConnection,
  collapsed,
  onToggleCollapsed,
}: Props) {
  const { t } = useI18n();
  const expanded = !collapsed;
  const branch = useStatusStore((s) => s.branch);
  const ahead = useStatusStore((s) => s.ahead);
  const behind = useStatusStore((s) => s.behind);

  const hasRemoteContext = activeTab?.backend === "ssh";
  const unknownTool = t("Unknown tool.");
  const useOuterShell = activeTool === "git" || activeTool === "markdown";
  const headerMeta = rightHeaderMeta(activeTool, browserPath, selectedMarkdownPath, branch, ahead, behind);
  const HeaderIcon = rightHeaderIcon(activeTool);

  const detectedEntry = useDetectedServicesStore((s) =>
    activeTab ? s.byTab[activeTab.id] : undefined,
  );
  const setPending = useDetectedServicesStore((s) => s.setPending);
  const setReady = useDetectedServicesStore((s) => s.setReady);
  const setError = useDetectedServicesStore((s) => s.setError);

  useEffect(() => {
    if (!activeTab || activeTab.backend !== "ssh") return;
    if (detectedEntry) return;
    if (!activeTab.sshHost) return;
    setPending(activeTab.id);
    const tabId = activeTab.id;
    cmd
      .detectServices({
        host: activeTab.sshHost,
        port: activeTab.sshPort,
        user: activeTab.sshUser,
        authMode: activeTab.sshAuthMode,
        password: activeTab.sshPassword,
        keyPath: activeTab.sshKeyPath,
      })
      .then((services) => {
        const tools: RightTool[] = [];
        for (const svc of services) {
          const tool = mapServiceToTool(svc.name);
          if (tool) tools.push(tool);
        }
        setReady(tabId, tools);
      })
      .catch(() => setError(tabId));
  }, [
    activeTab?.id,
    activeTab?.backend,
    activeTab?.sshHost,
    activeTab?.sshPort,
    activeTab?.sshUser,
    activeTab?.sshAuthMode,
    activeTab?.sshPassword,
    activeTab?.sshKeyPath,
    detectedEntry,
    setPending,
    setReady,
    setError,
  ]);

  const detectedTools = useMemo(
    () => detectedEntry?.tools ?? new Set<RightTool>(),
    [detectedEntry],
  );

  return (
    <div className="rightzone">
      {expanded && (
        <div className="rightpanel">
          {useOuterShell ? (
            <>
              <PanelHeader
                className="is-right"
                icon={HeaderIcon}
                title={toolTitle(activeTool, t)}
                meta={headerMeta}
              />
              <div className="panel-body">
                <ToolContent
                  tool={activeTool}
                  tab={activeTab}
                  browserPath={browserPath}
                  markdownPath={selectedMarkdownPath}
                  unknownToolLabel={unknownTool}
                  onConnectSaved={onConnectSaved}
                  onNewConnection={onNewConnection}
                  t={t}
                />
              </div>
            </>
          ) : (
            <ToolContent
              tool={activeTool}
              tab={activeTab}
              browserPath={browserPath}
              markdownPath={selectedMarkdownPath}
              unknownToolLabel={unknownTool}
              onConnectSaved={onConnectSaved}
              onNewConnection={onNewConnection}
              t={t}
            />
          )}
        </div>
      )}
      <ToolStrip
        activeTool={activeTool}
        hasRemoteContext={hasRemoteContext}
        detectedTools={detectedTools}
        onSelectTool={(tool) => {
          onToolChange(tool);
          if (collapsed) onToggleCollapsed();
        }}
        expanded={expanded}
        onToggleExpand={onToggleCollapsed}
      />
    </div>
  );
}
