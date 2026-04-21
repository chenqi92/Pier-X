// ── Pier-X Shell Orchestrator ────────────────────────────────────
// Three-pane IDE layout: Sidebar | Center (TabBar + Content) | RightSidebar

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Activity,
  Container,
  Database,
  FileText,
  FolderTree,
  GitBranch,
  HardDrive,
  Moon,
  ScrollText,
  Server,
  Settings as SettingsIcon,
  SquareTerminal,
  X,
  Zap,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { I18nContext, makeI18n } from "./i18n/useI18n";
import * as cmd from "./lib/commands";
import type { CoreInfo, FileEntry, RightTool, SavedSshConnection } from "./lib/types";
import ResizeHandle from "./components/ResizeHandle";
import SettingsDialog from "./components/SettingsDialog";
import Stage from "./components/Stage";
import type { MenuDef } from "./components/TitlebarMenu";
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
import { useRecentConnectionsStore } from "./stores/useRecentConnectionsStore";
import { useSettingsStore } from "./stores/useSettingsStore";
import { useThemeStore as useThemeStoreRef } from "./stores/useThemeStore";
import "./styles/fonts.css";
import "./styles/tokens.css";
import "./styles/atoms.css";
import "./styles/shell.css";
import "./styles/pier-x.css";

const MARKDOWN_EXTENSIONS = /\.(md|markdown|mdown|mkdn|mkd|mdx)$/i;
const PANE_STORAGE_KEY = "pierx:pane-widths";
const TOOLSTRIP_W = 42;
const DEFAULT_SIDEBAR_W = 244;
const DEFAULT_RIGHT_W = 360 + TOOLSTRIP_W;

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
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    try {
      const stored = JSON.parse(window.localStorage.getItem(PANE_STORAGE_KEY) || "{}") as {
        sidebar?: number;
      };
      return stored.sidebar ?? DEFAULT_SIDEBAR_W;
    } catch {
      return DEFAULT_SIDEBAR_W;
    }
  });
  const [rightWidth, setRightWidth] = useState(() => {
    try {
      const stored = JSON.parse(window.localStorage.getItem(PANE_STORAGE_KEY) || "{}") as {
        right?: number;
      };
      return stored.right ?? DEFAULT_RIGHT_W;
    } catch {
      return DEFAULT_RIGHT_W;
    }
  });
  const [rightCollapsed, setRightCollapsed] = useState(() => {
    try {
      const stored = JSON.parse(window.localStorage.getItem(PANE_STORAGE_KEY) || "{}") as {
        rightCollapsed?: boolean;
      };
      return stored.rightCollapsed ?? false;
    } catch {
      return false;
    }
  });
  const [fallbackRightTool, setFallbackRightTool] = useState<RightTool>("markdown");
  const { tabs, activeTabId, addTab, closeTab } = useTabStore();
  const locale = useSettingsStore((s) => s.locale);
  const i18n = useMemo(() => makeI18n(locale), [locale]);

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? null;
  const activeRightTool = activeTab?.rightTool ?? fallbackRightTool;

  const isDev = import.meta.env.DEV;

  // Bootstrap
  useEffect(() => {
    cmd.coreInfo()
      .then((info) => {
        setCoreInfo(info);
        setBrowserPath(info.workspaceRoot || info.homeDir || "");
      })
      .catch(() => {});
    useConnectionStore.getState().refresh();
  }, []);

  useEffect(() => {
    try {
      window.localStorage.setItem(
        PANE_STORAGE_KEY,
        JSON.stringify({
          sidebar: sidebarWidth,
          right: rightWidth,
          rightCollapsed,
        }),
      );
    } catch {
      /* ignore persistence errors */
    }
  }, [rightWidth, sidebarWidth, rightCollapsed]);

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

    // Dev: F12 / Cmd+Opt+I / Ctrl+Shift+I toggles DevTools via Tauri IPC.
    // Prod: the same combinations are swallowed so they can't reach the
    // webview's built-in inspector.
    const onKeyDown = (e: KeyboardEvent) => {
      const isF12 = e.key === "F12";
      const isInspect =
        (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "i") ||
        (e.metaKey && e.altKey && e.key.toLowerCase() === "i");
      const isConsole =
        (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "j") ||
        (e.metaKey && e.altKey && e.key.toLowerCase() === "j");
      if (!(isF12 || isInspect || isConsole)) return;
      e.preventDefault();
      if (isDev) {
        cmd.devToggleDevtools().catch(() => {});
      }
    };
    document.addEventListener("keydown", onKeyDown);

    return () => {
      document.removeEventListener("contextmenu", preventCtxMenu);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [isDev]);

  // ── Tab creation helpers ────────────────────────────────────

  function openLocalTerminal(path?: string) {
    const targetPath = path ?? browserPath ?? coreInfo?.workspaceRoot ?? coreInfo?.homeDir ?? "";
    const fallbackTitle = i18n.t("Terminal");
    addTab({
      backend: "local",
      title: targetPath ? targetPath.split(/[\\/]/).pop() || fallbackTitle : fallbackTitle,
      startupCommand: targetPath ? `cd ${JSON.stringify(targetPath)}` : "",
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
    if (!conn) return;
    useRecentConnectionsStore.getState().touch(index);
    // Seed the tab synchronously so the terminal starts launching
    // via terminalCreateSshSaved (backend resolves password itself).
    const tabId = addTab({
      backend: "ssh",
      title: conn.name || `${conn.user}@${conn.host}`,
      sshHost: conn.host,
      sshPort: conn.port,
      sshUser: conn.user,
      sshAuthMode: conn.authKind,
      sshKeyPath: conn.keyPath,
      sshSavedConnectionIndex: conn.index,
      rightTool: "monitor",
    });
    // Prime the in-memory password from the keychain so non-terminal
    // commands (probe, detect, docker, db) that take an explicit
    // password parameter work for saved password connections.
    if (conn.authKind === "password") {
      cmd
        .sshConnectionResolvePassword(conn.index)
        .then((password) => {
          if (password) {
            useTabStore.getState().updateTab(tabId, { sshPassword: password });
          }
        })
        .catch(() => {
          /* fall through — backend terminal will still work via saved-index path */
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
    } else {
      setFallbackRightTool(tool);
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
      { section: i18n.t("Session"), icon: SquareTerminal, title: i18n.t("New local terminal"), shortcut: `${mod}T`, action: () => openLocalTerminal() },
      { section: i18n.t("Session"), icon: Server, title: i18n.t("New SSH connection"), shortcut: `${mod}N`, action: openNewConnectionDialog },
      { section: i18n.t("Session"), icon: X, title: i18n.t("Close tab"), shortcut: `${mod}W`, action: () => { if (activeTabId) closeTab(activeTabId); } },
      { section: i18n.t("Panels"), icon: GitBranch, title: i18n.t("Switch to Git"), action: () => handleToolChange("git") },
      { section: i18n.t("Panels"), icon: Activity, title: i18n.t("Switch to Server Monitor"), action: () => handleToolChange("monitor") },
      { section: i18n.t("Panels"), icon: Container, title: i18n.t("Switch to Docker"), action: () => handleToolChange("docker") },
      { section: i18n.t("Panels"), icon: Database, title: i18n.t("Switch to MySQL"), action: () => handleToolChange("mysql") },
      { section: i18n.t("Panels"), icon: Database, title: i18n.t("Switch to PostgreSQL"), action: () => handleToolChange("postgres") },
      { section: i18n.t("Panels"), icon: Zap, title: i18n.t("Switch to Redis"), action: () => handleToolChange("redis") },
      { section: i18n.t("Panels"), icon: FolderTree, title: i18n.t("Switch to SFTP"), action: () => handleToolChange("sftp") },
      { section: i18n.t("Panels"), icon: ScrollText, title: i18n.t("Switch to Log"), action: () => handleToolChange("log") },
      { section: i18n.t("Panels"), icon: HardDrive, title: i18n.t("Switch to SQLite"), action: () => handleToolChange("sqlite") },
      { section: i18n.t("Panels"), icon: FileText, title: i18n.t("Switch to Markdown"), action: () => handleToolChange("markdown") },
      { section: i18n.t("App"), icon: SettingsIcon, title: i18n.t("Settings"), shortcut: `${mod},`, action: () => setSettingsOpen(true) },
      { section: i18n.t("App"), icon: Moon, title: i18n.t("Toggle theme"), action: () => {
        const s = useThemeStoreRef.getState();
        s.setMode(s.resolvedDark ? "light" : "dark");
      } },
    ],
    [activeTabId, closeTab, i18n, mod],
  );

  // ── Titlebar menus (Windows / Linux only) ─────────────────────
  // macOS uses the OS-native global menu bar; on non-mac we render
  // these directly in the titlebar via TitlebarMenu.
  const titlebarMenus = useMemo<MenuDef[]>(() => {
    const focusedIsEditable = () => {
      const el = document.activeElement as HTMLElement | null;
      if (!el) return false;
      const tag = el.tagName;
      return tag === "INPUT" || tag === "TEXTAREA" || el.isContentEditable;
    };
    const exec = (cmdName: "copy" | "cut" | "paste" | "selectAll") => {
      try {
        document.execCommand(cmdName);
      } catch {
        /* no-op: some webviews disable execCommand('paste') */
      }
    };
    return [
      {
        label: i18n.t("File"),
        items: [
          { label: i18n.t("New local terminal"), shortcut: "Ctrl+T", action: () => openLocalTerminal() },
          { label: i18n.t("New SSH connection"), shortcut: "Ctrl+N", action: openNewConnectionDialog },
          { divider: true },
          { label: i18n.t("Close tab"), shortcut: "Ctrl+W", disabled: !activeTabId, action: () => { if (activeTabId) closeTab(activeTabId); } },
          { divider: true },
          { label: i18n.t("Settings"), shortcut: "Ctrl+,", action: () => setSettingsOpen(true) },
          { divider: true },
          { label: i18n.t("Exit"), action: () => { void getCurrentWindow().close(); } },
        ],
      },
      {
        label: i18n.t("Edit"),
        items: [
          { label: i18n.t("Cut"), shortcut: "Ctrl+X", disabled: !focusedIsEditable(), action: () => exec("cut") },
          { label: i18n.t("Copy"), shortcut: "Ctrl+C", action: () => exec("copy") },
          { label: i18n.t("Paste"), shortcut: "Ctrl+V", disabled: !focusedIsEditable(), action: () => exec("paste") },
          { divider: true },
          { label: i18n.t("Select all"), shortcut: "Ctrl+A", action: () => exec("selectAll") },
        ],
      },
      {
        label: i18n.t("View"),
        items: [
          { label: i18n.t("Command palette"), shortcut: "Ctrl+K", action: () => setPaletteOpen(true) },
          { divider: true },
          { label: i18n.t("Toggle theme"), action: () => {
            const s = useThemeStoreRef.getState();
            s.setMode(s.resolvedDark ? "light" : "dark");
          } },
          { label: rightCollapsed ? i18n.t("Show right panel") : i18n.t("Hide right panel"), action: () => setRightCollapsed((c) => !c) },
        ],
      },
      {
        label: i18n.t("Session"),
        items: [
          { label: i18n.t("New local terminal"), shortcut: "Ctrl+T", action: () => openLocalTerminal() },
          { label: i18n.t("New SSH connection"), shortcut: "Ctrl+N", action: openNewConnectionDialog },
          { divider: true },
          { label: i18n.t("Close tab"), shortcut: "Ctrl+W", disabled: !activeTabId, action: () => { if (activeTabId) closeTab(activeTabId); } },
        ],
      },
      {
        label: i18n.t("Help"),
        items: [
          { label: i18n.t("Keyboard shortcuts"), action: () => setPaletteOpen(true) },
          { divider: true },
          { label: i18n.t("Documentation"), action: () => { void openUrl("https://github.com/chenqi92/Pier-X#readme"); } },
          { label: i18n.t("Report an issue"), action: () => { void openUrl("https://github.com/chenqi92/Pier-X/issues/new"); } },
          { divider: true },
          { label: i18n.t("About Pier-X"), action: () => {
            const v = coreInfo?.version ?? "0.1.0";
            window.alert(`Pier-X ${v}\n\n${i18n.t("Cross-platform terminal / Git / SSH / database management tool.")}`);
          } },
        ],
      },
    ];
  }, [activeTabId, closeTab, coreInfo?.version, i18n, rightCollapsed]);

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

  const rightPanelW = rightCollapsed ? 0 : Math.max(rightWidth - TOOLSTRIP_W, 0);
  const isRightCollapsed = rightCollapsed || rightPanelW === 0;
  const appStyle: React.CSSProperties = {
    ["--sidebar-w" as never]: `${sidebarWidth}px`,
    ["--rightpanel-w" as never]: `${rightPanelW}px`,
  };

  return (
    <I18nContext.Provider value={i18n}>
      <Stage>
        <div
          className={`app${isRightCollapsed ? " is-right-collapsed" : ""}`}
          style={appStyle}
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
            menus={titlebarMenus}
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

          <div className="center">
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

          <RightSidebar
            activeTab={activeTab}
            activeTool={activeRightTool}
            browserPath={browserPath}
            selectedMarkdownPath={selectedMarkdownPath}
            onToolChange={handleToolChange}
            onConnectSaved={openSshSaved}
            onNewConnection={openNewConnectionDialog}
            collapsed={rightCollapsed}
            onToggleCollapsed={() => setRightCollapsed((c) => !c)}
          />

          <StatusBar
            version={coreInfo?.version}
            coreInfo={coreInfo?.profile}
            activeTab={activeTab}
            activeTool={activeRightTool}
          />

          <ResizeHandle
            className="resizer is-left"
            direction="left"
            size={sidebarWidth}
            min={180}
            max={420}
            onResize={setSidebarWidth}
          />
          {!isRightCollapsed && (
            <ResizeHandle
              className="resizer is-right"
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
            onConnectSaved={openSshSaved}
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
