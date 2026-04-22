import type { RightTool, TabState } from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import { useEffect, useMemo, useRef, useState } from "react";
import * as cmd from "../lib/commands";
import { RIGHT_TOOL_META } from "../lib/rightToolMeta";
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
  /** Open the saved-connection editor — passed down to panels that need
   *  to recover from a "saved password missing" error. */
  onEditConnection: (index: number) => void;
  /** App-owned collapse state so the outer grid can reclaim right-panel width. */
  collapsed: boolean;
  onToggleCollapsed: () => void;
};

type SplashTool = "monitor" | "docker" | "mysql" | "postgres" | "redis" | "log" | "sftp";

function renderSplash(
  kind: SplashTool,
  t: (s: string) => string,
  onConnectSaved: (index: number) => void,
  onNewConnection: () => void,
) {
  const m = RIGHT_TOOL_META[kind];
  const Icon = m.icon;
  return (
    <ConnectSplash
      icon={<Icon size={22} strokeWidth={1.6} />}
      title={t(m.splashTitle ?? m.label)}
      subtitle={t(m.splashSubtitle ?? "")}
      tintVar={m.tintVar ?? "var(--accent)"}
      tagLabel={t("SSH")}
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
  isActive,
  onConnectSaved,
  onNewConnection,
  onEditConnection,
  t,
}: {
  tool: RightTool;
  tab: TabState | null;
  browserPath: string;
  markdownPath: string;
  unknownToolLabel: string;
  /** True when this slot is the visible right-side tool. Threaded into
   *  panels that do background polling so hidden (keep-alive) instances
   *  don't burn IPC. */
  isActive: boolean;
  onConnectSaved: (index: number) => void;
  onNewConnection: () => void;
  onEditConnection: (index: number) => void;
  t: (s: string) => string;
}) {
  const tabKey = tab?.id ?? "no-tab";
  switch (tool) {
    case "git":
      return <GitPanel key={tabKey} browserPath={browserPath} isActive={isActive} />;
    case "monitor":
      return tab
        ? <ServerMonitorPanel key={tab.id} tab={tab} onEditConnection={onEditConnection} />
        : renderSplash("monitor", t, onConnectSaved, onNewConnection);
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

export default function RightSidebar({
  activeTab,
  activeTool,
  browserPath,
  selectedMarkdownPath,
  onToolChange,
  onConnectSaved,
  onNewConnection,
  onEditConnection,
  collapsed,
  onToggleCollapsed,
}: Props) {
  const { t } = useI18n();
  const expanded = !collapsed;
  const branch = useStatusStore((s) => s.branch);
  const ahead = useStatusStore((s) => s.ahead);
  const behind = useStatusStore((s) => s.behind);

  // "Remote context" is true whenever the active tab carries SSH
  // addressing — either via the primary fields, the local-terminal
  // mirror after `ssh user@host`, or the nested-ssh overlay set
  // when `ssh user@host` is typed inside an existing SSH session.
  const activeSshTarget = activeTab ? effectiveSshTarget(activeTab) : null;
  const hasRemoteContext = activeSshTarget !== null;
  const unknownTool = t("Unknown tool.");

  // Keep-alive: once a tool has been opened for the current tab, its panel
  // stays mounted (hidden via CSS) so returning to it is instant — no
  // re-fetching git_panel_state / docker_overview / DB connects. Visited
  // resets when the active tab changes so we don't keep panels for stale
  // tabs alive; tab switches still cost exactly one mount.
  const tabKey = activeTab?.id ?? "no-tab";
  const [visited, setVisited] = useState<{ tabKey: string; tools: RightTool[] }>(
    { tabKey, tools: [activeTool] },
  );
  useEffect(() => {
    setVisited((prev) => {
      if (prev.tabKey !== tabKey) {
        return { tabKey, tools: [activeTool] };
      }
      if (prev.tools.includes(activeTool)) return prev;
      return { tabKey, tools: [...prev.tools, activeTool] };
    });
  }, [tabKey, activeTool]);

  const detectedEntry = useDetectedServicesStore((s) =>
    activeTab ? s.byTab[activeTab.id] : undefined,
  );
  const setPending = useDetectedServicesStore((s) => s.setPending);
  const setReady = useDetectedServicesStore((s) => s.setReady);
  const setError = useDetectedServicesStore((s) => s.setError);
  const clearDetectedTab = useDetectedServicesStore((s) => s.clearTab);

  // The detected-services cache is keyed by tabId, but the SSH target
  // for a tab can change (user typed `ssh user@otherhost` in a local
  // terminal, or nested ssh on a real ssh tab). Clear the entry when
  // the target host/user changes so the auto-detect effect below
  // re-runs against the new host instead of returning stale tools.
  const targetFingerprint = activeSshTarget
    ? `${activeSshTarget.user}@${activeSshTarget.host}:${activeSshTarget.port}`
    : "";
  const lastFingerprintRef = useRef<{ tabId: string; fp: string } | null>(null);
  useEffect(() => {
    if (!activeTab) return;
    const last = lastFingerprintRef.current;
    if (last?.tabId === activeTab.id && last.fp === targetFingerprint) return;
    if (last?.tabId === activeTab.id && last.fp !== targetFingerprint) {
      clearDetectedTab(activeTab.id);
    }
    lastFingerprintRef.current = { tabId: activeTab.id, fp: targetFingerprint };
  }, [activeTab, targetFingerprint, clearDetectedTab]);

  useEffect(() => {
    // Run detection any time we have an SSH target on the tab —
    // primary, local-mirror, or nested overlay. The store-entry
    // guard prevents re-running for already-detected tabs.
    if (!activeTab || !activeSshTarget) return;
    if (detectedEntry) return;
    setPending(activeTab.id);
    const tabId = activeTab.id;
    cmd
      .detectServices({
        host: activeSshTarget.host,
        port: activeSshTarget.port,
        user: activeSshTarget.user,
        authMode: activeSshTarget.authMode,
        password: activeSshTarget.password,
        keyPath: activeSshTarget.keyPath,
        savedConnectionIndex: activeSshTarget.savedConnectionIndex,
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
    // sshPassword / sshKeyPath are used inside but must NOT be reactive:
    // passwords arrive async via sshConnectionResolvePassword and would
    // otherwise re-trigger detection mid-flight. Detection is keyed on
    // connection identity (host/port/user/authMode) + the `detectedEntry`
    // guard, which is the right staleness signal.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    activeTab?.id,
    activeTab?.backend,
    activeSshTarget?.host,
    activeSshTarget?.port,
    activeSshTarget?.user,
    activeSshTarget?.authMode,
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
          {visited.tools.map((tool) => {
            const isActive = tool === activeTool;
            const useOuterShell = tool === "git" || tool === "markdown";
            const headerMeta = rightHeaderMeta(
              tool,
              browserPath,
              selectedMarkdownPath,
              branch,
              ahead,
              behind,
            );
            const HeaderIcon = useOuterShell ? RIGHT_TOOL_META[tool].icon : undefined;
            return (
              <div
                key={tool}
                className={"right-tool-slot" + (isActive ? "" : " is-hidden")}
                aria-hidden={!isActive}
              >
                {useOuterShell ? (
                  <>
                    <PanelHeader
                      className="is-right"
                      icon={HeaderIcon}
                      title={t(RIGHT_TOOL_META[tool].label)}
                      meta={headerMeta}
                    />
                    <div className="panel-body">
                      <ToolContent
                        tool={tool}
                        tab={activeTab}
                        browserPath={browserPath}
                        markdownPath={selectedMarkdownPath}
                        unknownToolLabel={unknownTool}
                        isActive={isActive}
                        onConnectSaved={onConnectSaved}
                        onNewConnection={onNewConnection}
                        onEditConnection={onEditConnection}
                        t={t}
                      />
                    </div>
                  </>
                ) : (
                  <ToolContent
                    tool={tool}
                    tab={activeTab}
                    browserPath={browserPath}
                    markdownPath={selectedMarkdownPath}
                    unknownToolLabel={unknownTool}
                    isActive={isActive}
                    onConnectSaved={onConnectSaved}
                    onNewConnection={onNewConnection}
                    onEditConnection={onEditConnection}
                    t={t}
                  />
                )}
              </div>
            );
          })}
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
