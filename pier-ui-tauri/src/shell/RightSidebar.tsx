import type { RightTool, TabState } from "../lib/types";
import { useEffect, useMemo, useState } from "react";
import {
  Activity,
  Container,
  Database,
  FolderTree,
  Scroll,
  Zap,
} from "lucide-react";
import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { mapServiceToTool, useDetectedServicesStore } from "../stores/useDetectedServicesStore";
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

type Props = {
  activeTab: TabState | null;
  browserPath: string;
  selectedMarkdownPath: string;
  onToolChange: (tool: RightTool) => void;
  onConnectSaved: (index: number) => void;
  onNewConnection: () => void;
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
    title: "Server monitor",
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
    title: "Log viewer",
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
      tagLabel="ssh"
      onConnectSaved={onConnectSaved}
      onNewConnection={onNewConnection}
    />
  );
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

export default function RightSidebar({ activeTab, browserPath, selectedMarkdownPath, onToolChange, onConnectSaved, onNewConnection }: Props) {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(true);

  const activeTool: RightTool = activeTab?.rightTool ?? "markdown";
  const hasRemoteContext = activeTab?.backend === "ssh";
  const unknownTool = t("Unknown tool.");

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
      )}
      <ToolStrip
        activeTool={activeTool}
        hasRemoteContext={hasRemoteContext}
        detectedTools={detectedTools}
        onSelectTool={(tool) => {
          onToolChange(tool);
          if (!expanded) setExpanded(true);
        }}
        expanded={expanded}
        onToggleExpand={() => setExpanded((p) => !p)}
      />
    </div>
  );
}
