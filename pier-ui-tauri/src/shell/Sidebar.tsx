import {
  ArrowLeft,
  ChevronRight,
  Download,
  FileText,
  Folder,
  FolderOpen,
  FolderTree,
  Home,
  Monitor,
  MoreHorizontal,
  Plus,
  RefreshCw,
  Search,
  Server,
  SquareTerminal,
  Terminal,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import ContextMenu, { type ContextMenuItem } from "../components/ContextMenu";
import type { FileEntry } from "../lib/types";
import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { useConnectionStore } from "../stores/useConnectionStore";

type Props = {
  onOpenLocalTerminal: (path?: string) => void;
  onConnectSaved: (index: number) => void;
  onNewConnection: () => void;
  workspaceRoot?: string;
};

function pathSegments(path: string, home: string): { name: string; path: string }[] {
  if (!path) return [];
  const segments: { name: string; path: string }[] = [];

  if (home && path.startsWith(home)) {
    segments.push({ name: "~", path: home });
    const relative = path.slice(home.length);
    const parts = relative.split(/[\\/]+/).filter(Boolean);
    let accumulated = home;
    for (const part of parts) {
      accumulated += "/" + part;
      segments.push({ name: part, path: accumulated });
    }
    return segments;
  }

  if (path === "/") return [{ name: "/", path: "/" }];
  segments.push({ name: "/", path: "/" });
  const parts = path.split(/[\\/]+/).filter(Boolean);
  let full = "";
  for (const part of parts) {
    full += "/" + part;
    segments.push({ name: part, path: full });
  }
  return segments;
}

function goUp(currentPath: string): string {
  const trimmed = currentPath.replace(/[\\/]+$/, "");
  if (!trimmed || trimmed === "/") return "/";
  const slash = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  if (slash <= 0) return "/";
  return trimmed.slice(0, slash);
}

function formatAuthKind(authKind: string, t: (key: string) => string) {
  switch (authKind) {
    case "password": return t("Password");
    case "agent": return t("SSH Agent");
    case "key": return t("Key File");
    default: return authKind;
  }
}

export default function Sidebar({ onOpenLocalTerminal, onConnectSaved, onNewConnection, workspaceRoot }: Props) {
  const { t } = useI18n();
  const [section, setSection] = useState<0 | 1>(0);
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [currentPath, setCurrentPath] = useState(workspaceRoot ?? "");
  const [homePath, setHomePath] = useState("");
  const [searchText, setSearchText] = useState("");
  const [placesOpen, setPlacesOpen] = useState(false);
  const { connections, refresh: refreshConnections } = useConnectionStore();
  const [serverSearch, setServerSearch] = useState("");
  const placesRef = useRef<HTMLDivElement>(null);

  // Init home path from core_info
  useEffect(() => {
    cmd.coreInfo().then((info) => {
      const root = info.workspaceRoot;
      setHomePath(root.replace(/\/[^/]+$/, "") || "/");
      if (!currentPath) setCurrentPath(root);
    }).catch(() => {});
  }, []);

  // Update path when workspaceRoot changes
  useEffect(() => {
    if (workspaceRoot && !currentPath) setCurrentPath(workspaceRoot);
  }, [workspaceRoot]);

  // Fetch directory
  useEffect(() => {
    if (!currentPath) return;
    cmd.listDirectory(currentPath).then(setEntries).catch(() => setEntries([]));
    setSearchText("");
  }, [currentPath]);

  // Load connections
  useEffect(() => { refreshConnections(); }, []);

  // Close places menu on outside click
  useEffect(() => {
    if (!placesOpen) return;
    const handler = (e: MouseEvent) => {
      if (placesRef.current && !placesRef.current.contains(e.target as Node)) setPlacesOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [placesOpen]);

  const filteredEntries = entries.filter((e) => {
    if (!searchText.trim()) return true;
    return e.name.toLowerCase().includes(searchText.toLowerCase());
  });

  const filteredConnections = connections.filter((conn) => {
    if (!serverSearch.trim()) return true;
    const q = serverSearch.toLowerCase();
    return conn.name.toLowerCase().includes(q) || conn.host.toLowerCase().includes(q) || conn.user.toLowerCase().includes(q);
  });

  const segments = pathSegments(currentPath, homePath);
  const folderName = segments.length > 0 ? segments[segments.length - 1].name : t("Files");

  return (
    <aside className="sidebar">
      {/* Tab switcher */}
      <div className="sidebar__tabs">
        <button className={section === 0 ? "sidebar__tab sidebar__tab--active" : "sidebar__tab"} onClick={() => setSection(0)} type="button">
          <FolderTree size={13} />{t("Files")}
        </button>
        <button className={section === 1 ? "sidebar__tab sidebar__tab--active" : "sidebar__tab"} onClick={() => setSection(1)} type="button">
          <Server size={13} />{t("Servers")}
        </button>
      </div>

      {section === 0 ? (
        <div className="sidebar__pane">
          {/* ── Header: Up + Folder name + Places + Refresh ─ */}
          <div className="sidebar__file-header">
            <button className="sidebar__icon-btn" disabled={!currentPath || currentPath === "/"} onClick={() => setCurrentPath(goUp(currentPath))} title={t("Up")} type="button">
              <ArrowLeft size={14} />
            </button>
            <Folder size={14} className="sidebar__folder-icon" />
            <span className="sidebar__folder-name" title={currentPath}>{folderName}</span>
            <div className="sidebar__places-wrap" ref={placesRef}>
              <button className="sidebar__icon-btn" onClick={() => setPlacesOpen((p) => !p)} title={t("Places")} type="button">
                <MoreHorizontal size={14} />
              </button>
              {placesOpen && (
                <div className="sidebar__places-menu">
                  <button className="sidebar__places-item" onClick={() => { setCurrentPath(homePath); setPlacesOpen(false); }} type="button">
                    <Home size={13} />{t("Home")}
                  </button>
                  <button className="sidebar__places-item" onClick={() => { setCurrentPath(homePath + "/Desktop"); setPlacesOpen(false); }} type="button">
                    <Monitor size={13} />{t("Desktop")}
                  </button>
                  <button className="sidebar__places-item" onClick={() => { setCurrentPath(homePath + "/Documents"); setPlacesOpen(false); }} type="button">
                    <FileText size={13} />{t("Documents")}
                  </button>
                  <button className="sidebar__places-item" onClick={() => { setCurrentPath(homePath + "/Downloads"); setPlacesOpen(false); }} type="button">
                    <Download size={13} />{t("Downloads")}
                  </button>
                  {workspaceRoot && (
                    <button className="sidebar__places-item" onClick={() => { setCurrentPath(workspaceRoot); setPlacesOpen(false); }} type="button">
                      <FolderOpen size={13} />{t("Workspace")}
                    </button>
                  )}
                  <div className="sidebar__places-divider" />
                  <button className="sidebar__places-item" onClick={() => { onOpenLocalTerminal(currentPath); setPlacesOpen(false); }} type="button">
                    <Terminal size={13} />{t("Open terminal here")}
                  </button>
                </div>
              )}
            </div>
            <button className="sidebar__icon-btn" onClick={() => { cmd.listDirectory(currentPath).then(setEntries).catch(() => {}); }} title={t("Refresh")} type="button">
              <RefreshCw size={13} />
            </button>
          </div>

          {/* ── Breadcrumb ──────────────────────────────── */}
          <div className="sidebar__breadcrumb">
            {segments.map((seg, i) => (
              <span key={seg.path} className="sidebar__breadcrumb-item">
                {i > 0 && <ChevronRight size={10} className="sidebar__breadcrumb-sep" />}
                <button className="sidebar__crumb" onClick={() => setCurrentPath(seg.path)} type="button">{seg.name}</button>
              </span>
            ))}
          </div>

          {/* ── Search ──────────────────────────────────── */}
          <div className="sidebar__search">
            <Search size={12} />
            <input className="sidebar__search-input" onChange={(e) => setSearchText(e.currentTarget.value)} placeholder={t("Search files…")} value={searchText} />
          </div>

          {/* ── Column headers ──────────────────────────── */}
          <div className="sidebar__col-headers">
            <span className="sidebar__col-name">{t("Name")}</span>
            <span className="sidebar__col-size">{t("Size")}</span>
          </div>

          {/* ── File list ───────────────────────────────── */}
          <div className="sidebar__list">
            {filteredEntries.map((entry) => (
              <button
                key={entry.path}
                className="sidebar__file-row"
                onClick={() => {
                  if (entry.kind === "directory") setCurrentPath(entry.path);
                }}
                onDoubleClick={() => {
                  if (entry.kind === "directory") onOpenLocalTerminal(entry.path);
                }}
                type="button"
              >
                {entry.kind === "directory" ? (
                  <Folder size={14} className="sidebar__entry-icon sidebar__entry-icon--dir" />
                ) : (
                  <FileText size={14} className="sidebar__entry-icon" />
                )}
                <span className="sidebar__file-name">{entry.name}</span>
                <span className="sidebar__file-size">{entry.sizeLabel}</span>
              </button>
            ))}
            {filteredEntries.length === 0 && (
              <div className="empty-note" style={{ padding: "16px" }}>
                {searchText ? t("No matching files") : t("Empty directory")}
              </div>
            )}
          </div>
        </div>
      ) : (
        /* ── Servers Pane ───────────────────────────── */
        <div className="sidebar__pane">
          <div className="sidebar__pane-header">
            <span className="sidebar__pane-label">{connections.length} {t("Connections")}</span>
            <button className="sidebar__icon-btn" onClick={onNewConnection} title={t("New SSH connection")} type="button">
              <Plus size={14} />
            </button>
          </div>

          <div className="sidebar__search">
            <Search size={12} />
            <input className="sidebar__search-input" onChange={(e) => setServerSearch(e.currentTarget.value)} placeholder={t("Search servers…")} value={serverSearch} />
          </div>

          <div className="sidebar__list">
            {filteredConnections.map((conn) => (
              <button key={`${conn.index}-${conn.name}`} className="sidebar__server-row" onClick={() => onConnectSaved(conn.index)} type="button">
                <SquareTerminal size={14} />
                <div className="sidebar__server-info">
                  <strong>{conn.name}</strong>
                  <span>{conn.user}@{conn.host}:{conn.port}</span>
                </div>
                <span className="sidebar__auth-pill">{formatAuthKind(conn.authKind, t)}</span>
              </button>
            ))}
            {filteredConnections.length === 0 && (
              <div className="empty-note" style={{ padding: "16px" }}>
                {connections.length === 0 ? t("No saved connections") : t("No matching connections")}
              </div>
            )}
          </div>
        </div>
      )}
    </aside>
  );
}
