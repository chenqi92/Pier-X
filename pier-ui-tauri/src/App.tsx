// ── Pier-X Shell Orchestrator ────────────────────────────────────
// Three-pane IDE layout: Sidebar | Center (TabBar + Content) | RightSidebar

import { useCallback, useEffect, useMemo, useState } from "react";
import { I18nContext, makeI18n } from "./i18n/useI18n";
import * as cmd from "./lib/commands";
import type { CoreInfo, FileEntry, RightTool, SavedSshConnection } from "./lib/types";
import ResizeHandle from "./components/ResizeHandle";
import SettingsDialog from "./components/SettingsDialog";
import Stage from "./components/Stage";
import TerminalPanel from "./panels/TerminalPanel";
import CommandPalette, { type PaletteCommand } from "./shell/CommandPalette";
import NewConnectionDialog from "./shell/NewConnectionDialog";
import TopBar from "./shell/TopBar";
import StatusBar from "./shell/StatusBar";
import Sidebar from "./shell/Sidebar";
import TabBar from "./shell/TabBar";
import WelcomeView from "./shell/WelcomeView";
import RightSidebar from "./shell/RightSidebar";
import { useTabStore } from "./stores/useTabStore";
import { useConnectionStore } from "./stores/useConnectionStore";
import { useSettingsStore } from "./stores/useSettingsStore";
import { useThemeStore as useThemeStoreRef } from "./stores/useThemeStore";
import "./styles/fonts.css";
import "./styles/tokens.css";
import "./styles/atoms.css";
import "./styles/shell.css";

const MARKDOWN_EXTENSIONS = /\.(md|markdown|mdown|mkdn|mkd|mdx)$/i;
function isMarkdownFile(name: string): boolean {
  return MARKDOWN_EXTENSIONS.test(name);
}

function App() {
  const [coreInfo, setCoreInfo] = useState<CoreInfo | null>(null);
  const [browserPath, setBrowserPath] = useState("");
  const [selectedMarkdownPath, setSelectedMarkdownPath] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [newConnOpen, setNewConnOpen] = useState(false);
  const [editingConnection, setEditingConnection] = useState<SavedSshConnection | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(272);
  const [rightWidth, setRightWidth] = useState(400);
  const { tabs, activeTabId, addTab, closeTab } = useTabStore();
  const locale = useSettingsStore((s) => s.locale);
  const i18n = useMemo(() => makeI18n(locale), [locale]);

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? null;

  const isDev = import.meta.env.DEV;

  // Bootstrap
  useEffect(() => {
    cmd.coreInfo()
      .then((info) => {
        setCoreInfo(info);
        setBrowserPath(info.homeDir || info.workspaceRoot || "");
      })
      .catch(() => {});
    useConnectionStore.getState().refresh();
  }, []);

  // ── Desktop behaviors ───────────────────────────────────────
  useEffect(() => {
    // Disable default browser context menu (we provide our own)
    const preventCtxMenu = (e: MouseEvent) => {
      // Allow context menu in terminal viewport (handled there)
      // and in text inputs/textareas for native copy/paste
      const target = e.target as HTMLElement;
      if (target.closest(".terminal-viewport") || target.closest("input") || target.closest("textarea")) return;
      e.preventDefault();
    };
    document.addEventListener("contextmenu", preventCtxMenu);

    // Disable DevTools shortcut in production
    if (!isDev) {
      const blockDevTools = (e: KeyboardEvent) => {
        // Block F12, Ctrl+Shift+I, Cmd+Option+I
        if (e.key === "F12") { e.preventDefault(); return; }
        if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "i") { e.preventDefault(); return; }
        if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "j") { e.preventDefault(); return; }
      };
      document.addEventListener("keydown", blockDevTools);
      return () => { document.removeEventListener("contextmenu", preventCtxMenu); document.removeEventListener("keydown", blockDevTools); };
    }

    return () => document.removeEventListener("contextmenu", preventCtxMenu);
  }, [isDev]);

  // ── Tab creation helpers ────────────────────────────────────

  function openLocalTerminal(path?: string) {
    const fallbackTitle = i18n.t("Terminal");
    addTab({
      backend: "local",
      title: path ? path.split(/[\\/]/).pop() || fallbackTitle : fallbackTitle,
      startupCommand: path ? `cd ${JSON.stringify(path)}` : "",
    });
  }

  function openSshTab(params: {
    name: string;
    host: string;
    port: number;
    user: string;
    authKind: string;
    password: string;
    keyPath: string;
  }) {
    addTab({
      backend: "ssh",
      title: params.name || `${params.user}@${params.host}`,
      sshHost: params.host,
      sshPort: params.port,
      sshUser: params.user,
      sshAuthMode: params.authKind as "password" | "agent" | "key",
      sshPassword: params.password,
      sshKeyPath: params.keyPath,
      rightTool: "monitor",
    });
  }

  function openSshSaved(index: number) {
    const conn = useConnectionStore.getState().connections.find((c) => c.index === index);
    if (conn) {
      addTab({
        backend: "ssh",
        title: conn.name || `${conn.user}@${conn.host}`,
        sshHost: conn.host,
        sshPort: conn.port,
        sshUser: conn.user,
        sshAuthMode: conn.authKind,
        sshKeyPath: conn.keyPath,
        rightTool: "monitor",
      });
    }
  }

  function openNewTab() {
    openLocalTerminal();
  }

  function openNewConnectionDialog() {
    setEditingConnection(null);
    setNewConnOpen(true);
  }

  function openEditConnectionDialog(index: number) {
    const connection = useConnectionStore.getState().connections.find((entry) => entry.index === index) ?? null;
    setEditingConnection(connection);
    setNewConnOpen(true);
  }

  function handleToolChange(tool: RightTool) {
    if (activeTab) {
      useTabStore.getState().setTabRightTool(activeTab.id, tool);
    }
  }

  function handleFileSelect(entry: FileEntry) {
    if (!isMarkdownFile(entry.name)) return;
    setSelectedMarkdownPath(entry.path);
    if (activeTab && activeTab.rightTool !== "markdown") {
      useTabStore.getState().setTabRightTool(activeTab.id, "markdown");
    }
  }

  // ── Command Palette commands ────────────────────────────────

  const isMac = navigator.platform.includes("Mac");
  const mod = isMac ? "\u2318" : "Ctrl+";

  const paletteCommands: PaletteCommand[] = useMemo(
    () => [
      { title: i18n.t("New local terminal"), shortcut: `${mod}T`, action: () => openLocalTerminal() },
      { title: i18n.t("New SSH connection"), shortcut: `${mod}N`, action: openNewConnectionDialog },
      { title: i18n.t("Close tab"), shortcut: `${mod}W`, action: () => { if (activeTabId) closeTab(activeTabId); } },
      { title: i18n.t("Settings"), shortcut: `${mod},`, action: () => setSettingsOpen(true) },
      { title: i18n.t("Toggle theme"), action: () => {
        const s = useThemeStoreRef.getState();
        s.setMode(s.resolvedDark ? "light" : "dark");
      } },
      { title: i18n.t("Switch to Git"), action: () => handleToolChange("git") },
      { title: i18n.t("Switch to Docker"), action: () => handleToolChange("docker") },
      { title: i18n.t("Switch to MySQL"), action: () => handleToolChange("mysql") },
      { title: i18n.t("Switch to PostgreSQL"), action: () => handleToolChange("postgres") },
      { title: i18n.t("Switch to Redis"), action: () => handleToolChange("redis") },
      { title: i18n.t("Switch to SFTP"), action: () => handleToolChange("sftp") },
      { title: i18n.t("Switch to Server Monitor"), action: () => handleToolChange("monitor") },
      { title: i18n.t("Switch to SQLite"), action: () => handleToolChange("sqlite") },
      { title: i18n.t("Switch to Markdown"), action: () => handleToolChange("markdown") },
    ],
    [activeTabId, closeTab, i18n],
  );

  // ── Keyboard shortcuts ──────────────────────────────────────

  const handleGlobalKeyDown = useCallback(
    (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;

      // Cmd+K — Command palette
      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((p) => !p);
        return;
      }
      // Cmd+T — New tab
      if (mod && !e.shiftKey && e.key.toLowerCase() === "t") {
        e.preventDefault();
        openLocalTerminal();
        return;
      }
      // Cmd+W — Close tab
      if (mod && !e.shiftKey && e.key.toLowerCase() === "w") {
        e.preventDefault();
        if (activeTabId) closeTab(activeTabId);
        return;
      }
      // Cmd+N — New SSH
      if (mod && !e.shiftKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        openNewConnectionDialog();
        return;
      }
      // Cmd+, — Settings
      if (mod && e.key === ",") {
        e.preventDefault();
        setSettingsOpen((p) => !p);
        return;
      }
      // Cmd+Shift+G — Toggle Git panel
      if (mod && e.shiftKey && e.key.toLowerCase() === "g") {
        e.preventDefault();
        handleToolChange("git");
        return;
      }
    },
    [activeTabId],
  );

  useEffect(() => {
    window.addEventListener("keydown", handleGlobalKeyDown);
    return () => window.removeEventListener("keydown", handleGlobalKeyDown);
  }, [handleGlobalKeyDown]);

  const TOOLSTRIP_W = 42;
  const rightPanelW = Math.max(rightWidth - TOOLSTRIP_W, 0);
  const isRightCollapsed = rightPanelW === 0;
  const appShellStyle: React.CSSProperties = {
    ["--sidebar-w" as never]: `${sidebarWidth}px`,
    ["--rightpanel-w" as never]: `${rightPanelW}px`,
  };

  return (
    <I18nContext.Provider value={i18n}>
      <Stage>
        <div
          className={`app-shell${isRightCollapsed ? " is-right-collapsed" : ""}`}
          style={appShellStyle}
        >
          <TopBar
            onNewTab={openNewTab}
            onSettings={() => setSettingsOpen(true)}
            onToggleTheme={() => {
              const s = useThemeStoreRef.getState();
              s.setMode(s.resolvedDark ? "light" : "dark");
            }}
            version={coreInfo?.version}
            onCommandPalette={() => setPaletteOpen(true)}
          />

          <TabBar onNewTab={openNewTab} />

          <Sidebar
            onOpenLocalTerminal={openLocalTerminal}
            onConnectSaved={openSshSaved}
            onNewConnection={openNewConnectionDialog}
            onEditConnection={openEditConnectionDialog}
            onPathChange={setBrowserPath}
            onFileSelect={handleFileSelect}
            selectedFilePath={selectedMarkdownPath}
            workspaceRoot={coreInfo?.workspaceRoot}
          />

          <div className="workspace__center">
            <div className="workspace__content">
              {tabs.length === 0 ? (
                <WelcomeView
                  onOpenLocalTerminal={openLocalTerminal}
                  onNewSsh={openNewConnectionDialog}
                  onConnectSaved={openSshSaved}
                  onSettings={() => setSettingsOpen(true)}
                  onCommandPalette={() => setPaletteOpen(true)}
                  version={coreInfo?.version}
                  workspaceRoot={coreInfo?.workspaceRoot}
                />
              ) : (
                tabs.map((tab) => (
                  <TerminalPanel
                    key={tab.id}
                    tab={tab}
                    isActive={tab.id === activeTabId}
                  />
                ))
              )}
            </div>
          </div>

          <RightSidebar
            activeTab={activeTab}
            browserPath={browserPath}
            selectedMarkdownPath={selectedMarkdownPath}
            onToolChange={handleToolChange}
          />

          <StatusBar
            version={coreInfo?.version}
            coreInfo={coreInfo?.profile}
            activeTab={activeTab}
          />

          <ResizeHandle
            className="resize-handle--left"
            direction="left"
            size={sidebarWidth}
            min={180}
            max={420}
            onResize={setSidebarWidth}
          />
          {!isRightCollapsed && (
            <ResizeHandle
              className="resize-handle--right"
              direction="right"
              size={rightWidth}
              min={TOOLSTRIP_W + 220}
              max={900}
              onResize={setRightWidth}
            />
          )}

        {/* Overlays */}
        <CommandPalette
          open={paletteOpen}
          onClose={() => setPaletteOpen(false)}
          commands={paletteCommands}
        />
        <NewConnectionDialog
          open={newConnOpen}
          initialConnection={editingConnection}
          onClose={() => {
            setNewConnOpen(false);
            setEditingConnection(null);
          }}
          onConnect={openSshTab}
        />
        <SettingsDialog
          open={settingsOpen}
          onClose={() => setSettingsOpen(false)}
        />
        </div>
      </Stage>
    </I18nContext.Provider>
  );
}

export default App;
