// ── Pier-X Shell Orchestrator ────────────────────────────────────
// Three-pane IDE layout: Sidebar | Center (TabBar + Content) | RightSidebar

import { useCallback, useEffect, useMemo, useState } from "react";
import { I18nContext, makeI18n } from "./i18n/useI18n";
import * as cmd from "./lib/commands";
import type { CoreInfo, RightTool } from "./lib/types";
import SettingsDialog from "./components/SettingsDialog";
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
import "./styles/tokens.css";
import "./styles/shell.css";

function App() {
  const [coreInfo, setCoreInfo] = useState<CoreInfo | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [newConnOpen, setNewConnOpen] = useState(false);
  const { tabs, activeTabId, addTab, closeTab } = useTabStore();
  const locale = useSettingsStore((s) => s.locale);
  const i18n = useMemo(() => makeI18n(locale), [locale]);

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? null;

  // Bootstrap
  useEffect(() => {
    cmd.coreInfo().then(setCoreInfo).catch(() => {});
    useConnectionStore.getState().refresh();
  }, []);

  // ── Tab creation helpers ────────────────────────────────────

  function openLocalTerminal(path?: string) {
    addTab({
      backend: "local",
      title: path ? path.split(/[\\/]/).pop() || "Terminal" : "Terminal",
      startupCommand: path ? `cd ${path}` : "",
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

  function handleToolChange(tool: RightTool) {
    if (activeTab) {
      useTabStore.getState().setTabRightTool(activeTab.id, tool);
    }
  }

  // ── Command Palette commands ────────────────────────────────

  const isMac = navigator.platform.includes("Mac");
  const mod = isMac ? "\u2318" : "Ctrl+";

  const paletteCommands: PaletteCommand[] = useMemo(
    () => [
      { title: "New Local Terminal", shortcut: `${mod}T`, action: () => openLocalTerminal() },
      { title: "New SSH Connection", shortcut: `${mod}N`, action: () => setNewConnOpen(true) },
      { title: "Close Tab", shortcut: `${mod}W`, action: () => { if (activeTabId) closeTab(activeTabId); } },
      { title: "Settings", shortcut: `${mod},`, action: () => setSettingsOpen(true) },
      { title: "Toggle Theme", action: () => {
        const s = useThemeStoreRef.getState();
        s.setMode(s.resolvedDark ? "light" : "dark");
      } },
      { title: "Switch to Git", action: () => handleToolChange("git") },
      { title: "Switch to Docker", action: () => handleToolChange("docker") },
      { title: "Switch to MySQL", action: () => handleToolChange("mysql") },
      { title: "Switch to PostgreSQL", action: () => handleToolChange("postgres") },
      { title: "Switch to Redis", action: () => handleToolChange("redis") },
      { title: "Switch to SFTP", action: () => handleToolChange("sftp") },
      { title: "Switch to Server Monitor", action: () => handleToolChange("monitor") },
      { title: "Switch to SQLite", action: () => handleToolChange("sqlite") },
      { title: "Switch to Markdown", action: () => handleToolChange("markdown") },
    ],
    [activeTabId],
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
        setNewConnOpen(true);
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

  return (
    <I18nContext.Provider value={i18n}>
      <div className="app-shell">
        <TopBar
          onNewTab={openNewTab}
          onSettings={() => setSettingsOpen(true)}
          version={coreInfo?.version}
        />

        <div className="workspace">
          <Sidebar
            onOpenLocalTerminal={openLocalTerminal}
            onConnectSaved={openSshSaved}
            onNewConnection={() => setNewConnOpen(true)}
          />

          <div className="workspace__center">
            <TabBar onNewTab={openNewTab} />
            <div className="workspace__content">
              {tabs.length === 0 ? (
                <WelcomeView
                  onOpenLocalTerminal={() => openLocalTerminal()}
                  onNewSsh={() => setNewConnOpen(true)}
                  onConnectSaved={openSshSaved}
                  version={coreInfo?.version}
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
            browserPath={coreInfo?.workspaceRoot ?? ""}
            onToolChange={handleToolChange}
          />
        </div>

        <StatusBar
          version={coreInfo?.version}
          coreInfo={coreInfo?.profile}
        />

        {/* Overlays */}
        <CommandPalette
          open={paletteOpen}
          onClose={() => setPaletteOpen(false)}
          commands={paletteCommands}
        />
        <NewConnectionDialog
          open={newConnOpen}
          onClose={() => setNewConnOpen(false)}
          onConnect={openSshTab}
        />
        <SettingsDialog
          open={settingsOpen}
          onClose={() => setSettingsOpen(false)}
        />
      </div>
    </I18nContext.Provider>
  );
}

export default App;
