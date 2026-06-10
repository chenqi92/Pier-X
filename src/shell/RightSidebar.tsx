import type { RightTool, TabState } from "../lib/types";
import { effectiveSshTarget, isSshTargetReady } from "../lib/types";
import { isBrowsableRepoPath } from "../lib/browserPath";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import * as cmd from "../lib/shellCommands";
import { RIGHT_TOOL_META } from "../lib/rightToolMeta";
import { useI18n } from "../i18n/useI18n";
import { mapServiceToTool, useDetectedServicesStore } from "../stores/useDetectedServicesStore";
import { useStatusStore } from "../stores/useStatusStore";
import ToolStrip from "./ToolStrip";
import ConnectSplash from "../components/ConnectSplash";
import PanelHeader from "../components/PanelHeader";
import PanelSkeleton from "../components/PanelSkeleton";

const GitPanel = lazy(() => import("../panels/GitPanel"));
const MySqlPanel = lazy(() => import("../panels/MySqlPanel"));
const PostgresPanel = lazy(() => import("../panels/PostgresPanel"));
const SqlitePanel = lazy(() => import("../panels/SqlitePanel"));
const RedisPanel = lazy(() => import("../panels/RedisPanel"));
const DockerPanel = lazy(() => import("../panels/DockerPanel"));
const SftpPanel = lazy(() => import("../panels/SftpPanel"));
const ServerMonitorPanel = lazy(() => import("../panels/ServerMonitorPanel"));
const MarkdownPanel = lazy(() => import("../panels/MarkdownPanel"));
const LogViewerPanel = lazy(() => import("../panels/LogViewerPanel"));
const CodeSearchPanel = lazy(() => import("../panels/CodeSearchPanel"));
const FirewallPanel = lazy(() => import("../panels/FirewallPanel"));
const WebServerPanel = lazy(() => import("../panels/WebServerPanel"));
const SoftwarePanel = lazy(() => import("../panels/SoftwarePanel"));

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

type SplashTool = "monitor" | "docker" | "mysql" | "postgres" | "redis" | "log" | "search" | "sftp" | "firewall" | "webserver" | "software";

const DB_DETECTION_TOOLS = new Set<RightTool>(["mysql", "postgres", "redis"]);
const SERVICE_DETECTION_DELAY_MS = 2_000;
const EMPTY_DETECTED_TOOLS = new Set<RightTool>();

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
        ? <ServerMonitorPanel key={tab.id} tab={tab} isActive={isActive} onEditConnection={onEditConnection} />
        : renderSplash("monitor", t, onConnectSaved, onNewConnection);
    case "docker":
      return tab ? <DockerPanel key={tab.id} tab={tab} /> : renderSplash("docker", t, onConnectSaved, onNewConnection);
    case "firewall":
      return tab ? <FirewallPanel key={tab.id} tab={tab} isActive={isActive} /> : renderSplash("firewall", t, onConnectSaved, onNewConnection);
    case "mysql":
      return tab ? <MySqlPanel key={tab.id} tab={tab} /> : renderSplash("mysql", t, onConnectSaved, onNewConnection);
    case "postgres":
      return tab ? <PostgresPanel key={tab.id} tab={tab} /> : renderSplash("postgres", t, onConnectSaved, onNewConnection);
    case "redis":
      return tab ? <RedisPanel key={tab.id} tab={tab} /> : renderSplash("redis", t, onConnectSaved, onNewConnection);
    case "log":
      return tab ? <LogViewerPanel key={tab.id} tab={tab} isActive={isActive} /> : renderSplash("log", t, onConnectSaved, onNewConnection);
    case "sftp":
      return tab ? <SftpPanel key={tab.id} tab={tab} /> : renderSplash("sftp", t, onConnectSaved, onNewConnection);
    case "search":
      return tab ? <CodeSearchPanel key={tab.id} tab={tab} /> : renderSplash("search", t, onConnectSaved, onNewConnection);
    case "sqlite":
      return <SqlitePanel key={tabKey} tab={tab} />;
    case "webserver":
      return tab ? <WebServerPanel key={tab.id} tab={tab} /> : renderSplash("webserver", t, onConnectSaved, onNewConnection);
    case "software":
      return tab ? <SoftwarePanel key={tab.id} tab={tab} isActive={isActive} /> : renderSplash("software", t, onConnectSaved, onNewConnection);
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
    // Suppress the header subtitle on the drives sentinel — otherwise
    // it reads "pier:drives" literally, which is both ugly and
    // misleading (there is no directory).
    return isBrowsableRepoPath(browserPath) ? basename(browserPath) : undefined;
  }
  return undefined;
}

function lazyFallbackFor(tool: RightTool) {
  if (tool === "markdown") return <PanelSkeleton variant="prose" rows={8} />;
  if (tool === "monitor") return <PanelSkeleton variant="chrome" rows={4} />;
  if (tool === "mysql" || tool === "postgres" || tool === "redis" || tool === "sqlite") {
    return <PanelSkeleton variant="grid" rows={8} />;
  }
  if (tool === "software") return <PanelSkeleton variant="rows" rows={9} />;
  return <PanelSkeleton variant="rows" rows={8} />;
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
  const activeSshReady = isSshTargetReady(activeSshTarget);
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
  const setDbInstancesPending = useDetectedServicesStore((s) => s.setDbInstancesPending);
  const setDbInstances = useDetectedServicesStore((s) => s.setDbInstances);
  const setDbInstancesError = useDetectedServicesStore((s) => s.setDbInstancesError);

  // The detected-services cache is keyed by tabId, but the SSH target
  // for a tab can change (user typed `ssh user@otherhost` in a local
  // terminal, or nested ssh on a real ssh tab). Clear the entry when
  // the target host/user/port changes — and also when credentials
  // first land, so a detection that failed because we had no
  // password yet automatically re-runs once the password capture
  // (in TerminalPanel) populates `sshPassword`.
  const targetFingerprint = activeSshTarget
    ? [
        activeSshTarget.user,
        activeSshTarget.host,
        activeSshTarget.port,
        activeSshTarget.authMode,
        activeSshTarget.password ? "pw" : "no-pw",
      ].join("|")
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
    // guard prevents re-running for already-detected tabs. Delay
    // and skip while collapsed so first SSH paint / monitor probe
    // get priority over cosmetic service chips. For
    // real SSH-backend tabs we additionally wait for the terminal
    // session to come up so detect_services hits the cached
    // russh handle instead of racing the terminal's own handshake
    // — same reasoning as the gating in ServerMonitorPanel.
    if (!expanded) return;
    if (!activeTab || !activeSshTarget || !activeSshReady) return;
    if (detectedEntry) return;
    if (activeTab.backend === "ssh" && activeTab.terminalSessionId === null) return;
    const tabId = activeTab.id;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      if (cancelled) return;
      setPending(tabId);
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
          if (cancelled) return;
          const tools: RightTool[] = [];
          for (const svc of services) {
            const tool = mapServiceToTool(svc.name);
            if (tool) tools.push(tool);
          }
          setReady(tabId, tools);
        })
        .catch(() => {
          if (!cancelled) setError(tabId);
        });
    }, SERVICE_DETECTION_DELAY_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
    // The full password value is intentionally NOT in the deps —
    // the in-memory secret can come and go via async resolution and
    // we don't want detection re-firing mid-flight. We DO depend on
    // its presence (boolean) so the FIRST flip from "no password"
    // to "password available" triggers a re-detect after the
    // staleness clear above wipes the prior failure entry.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    activeTab?.id,
    activeTab?.backend,
    activeTab?.terminalSessionId !== null,
    activeSshTarget?.host,
    activeSshTarget?.port,
    activeSshTarget?.user,
    activeSshTarget?.authMode,
    activeSshReady,
    (activeSshTarget?.password.length ?? 0) > 0,
    detectedEntry,
    expanded,
    setPending,
    setReady,
    setError,
  ]);

  // Eager DB instance detection used to run immediately after every
  // SSH connect, alongside service chips and the monitor's first full
  // probe. That stacked three remote command batches on the first
  // handshake. Keep DB detection lazy: run it when a DB panel becomes
  // the visible tool, where its splash can show progress.
  useEffect(() => {
    if (!activeTab || !activeSshTarget || !activeSshReady) return;
    if (!DB_DETECTION_TOOLS.has(activeTool)) return;
    if (activeTab.backend === "ssh" && activeTab.terminalSessionId === null) return;
    const entry = useDetectedServicesStore.getState().instancesByTab[activeTab.id];
    const fresh = entry?.status === "ready" && Date.now() - entry.at < 60_000;
    if (fresh || entry?.status === "pending") return;
    const tabId = activeTab.id;
    let cancelled = false;
    setDbInstancesPending(tabId);
    const timer = window.setTimeout(() => {
      if (cancelled) return;
      cmd
        .dbDetect({
          host: activeSshTarget.host,
          port: activeSshTarget.port,
          user: activeSshTarget.user,
          authMode: activeSshTarget.authMode,
          password: activeSshTarget.password,
          keyPath: activeSshTarget.keyPath,
          savedConnectionIndex: activeSshTarget.savedConnectionIndex,
        })
        .then((report) => {
          if (cancelled) return;
          setDbInstances(tabId, {
            instances: report.instances,
            mysqlCli: report.mysqlCli,
            psqlCli: report.psqlCli,
            redisCli: report.redisCli,
            sqliteCli: report.sqliteCli,
          });
        })
        .catch(() => {
          if (!cancelled) setDbInstancesError(tabId);
        });
    }, 250);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
    // Mirrors the dep list of the service-chip effect above —
    // boolean presence of password, not its value, so mid-flight
    // re-renders don't re-fire detection.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    activeTab?.id,
    activeTab?.backend,
    activeTab?.terminalSessionId !== null,
    activeSshTarget?.host,
    activeSshTarget?.port,
    activeSshTarget?.user,
    activeSshTarget?.authMode,
    activeSshReady,
    (activeSshTarget?.password.length ?? 0) > 0,
    activeTool,
    setDbInstancesPending,
    setDbInstances,
    setDbInstancesError,
  ]);

  const detectedTools = useMemo(
    () => detectedEntry?.tools ?? EMPTY_DETECTED_TOOLS,
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
            const lazyFallback = lazyFallbackFor(tool);
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
                      <Suspense fallback={lazyFallback}>
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
                      </Suspense>
                    </div>
                  </>
                ) : (
                  <Suspense fallback={lazyFallback}>
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
                  </Suspense>
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
        collapsed={collapsed}
        onToggleCollapsed={onToggleCollapsed}
        onSelectTool={(tool) => {
          onToolChange(tool);
          if (collapsed) onToggleCollapsed();
        }}
      />
    </div>
  );
}
