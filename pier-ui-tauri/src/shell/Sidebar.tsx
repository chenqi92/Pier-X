import {
  Activity,
  ArrowLeft,
  ChevronRight,
  Container,
  Database,
  Download,
  FileText,
  Folder,
  FolderOpen,
  FolderTree,
  HardDrive,
  Home,
  Key,
  Lock,
  Monitor,
  MoreHorizontal,
  Pencil,
  Plus,
  RefreshCw,
  Scroll,
  Search,
  Server,
  Shield,
  Terminal,
  Trash2,
  Zap,
} from "lucide-react";
import type { ComponentType, SVGProps } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { CoreInfo, FileEntry, SavedSshConnection, RightTool } from "../lib/types";
import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useTabStore } from "../stores/useTabStore";
import { useDetectedServicesStore } from "../stores/useDetectedServicesStore";

type Props = {
  onOpenLocalTerminal: (path?: string) => void;
  onConnectSaved: (index: number) => void;
  onNewConnection: () => void;
  onEditConnection: (index: number) => void;
  onPathChange?: (path: string) => void;
  onFileSelect?: (entry: FileEntry) => void;
  selectedFilePath?: string;
  workspaceRoot?: string;
};

type LucideIcon = ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;

type ServiceChip = {
  tool: RightTool;
  label: string;
  icon: LucideIcon;
  tintVar: string;
};

const SERVICE_META: Record<string, ServiceChip> = {
  docker: { tool: "docker", label: "docker", icon: Container, tintVar: "var(--svc-docker)" },
  mysql: { tool: "mysql", label: "mysql", icon: Database, tintVar: "var(--svc-mysql)" },
  postgres: { tool: "postgres", label: "postgres", icon: Database, tintVar: "var(--svc-postgres)" },
  redis: { tool: "redis", label: "redis", icon: Zap, tintVar: "var(--svc-redis)" },
  monitor: { tool: "monitor", label: "monitor", icon: Activity, tintVar: "var(--svc-monitor)" },
  log: { tool: "log", label: "log", icon: Scroll, tintVar: "var(--svc-log)" },
  sftp: { tool: "sftp", label: "sftp", icon: FolderTree, tintVar: "var(--svc-sftp)" },
  sqlite: { tool: "sqlite", label: "sqlite", icon: HardDrive, tintVar: "var(--svc-sqlite)" },
};

function splitGroup(name: string): { group: string; display: string } {
  const slash = name.indexOf("/");
  if (slash > 0 && slash < name.length - 1) {
    return { group: name.slice(0, slash).trim() || "default", display: name.slice(slash + 1).trim() };
  }
  return { group: "default", display: name };
}

type ConnectionGroup = {
  name: string;
  servers: Array<SavedSshConnection & { display: string }>;
};

function groupConnections(conns: SavedSshConnection[], query: string): ConnectionGroup[] {
  const q = query.trim().toLowerCase();
  const byGroup = new Map<string, ConnectionGroup>();
  for (const c of conns) {
    const { group, display } = splitGroup(c.name);
    if (q) {
      const hay = (c.name + c.host + c.user).toLowerCase();
      if (!hay.includes(q)) continue;
    }
    const entry = byGroup.get(group);
    const augmented = { ...c, display };
    if (entry) entry.servers.push(augmented);
    else byGroup.set(group, { name: group, servers: [augmented] });
  }
  return Array.from(byGroup.values()).sort((a, b) => a.name.localeCompare(b.name));
}

function pathSegments(path: string, home: string): { name: string; path: string }[] {
  if (!path) return [];
  const segments: { name: string; path: string }[] = [];
  if (home && path.startsWith(home)) {
    segments.push({ name: "~", path: home });
    const parts = path.slice(home.length).split(/[\\/]+/).filter(Boolean);
    let acc = home;
    for (const part of parts) { acc += "/" + part; segments.push({ name: part, path: acc }); }
    return segments;
  }
  if (path === "/") return [{ name: "/", path: "/" }];
  segments.push({ name: "/", path: "/" });
  const parts = path.split(/[\\/]+/).filter(Boolean);
  let full = "";
  for (const part of parts) { full += "/" + part; segments.push({ name: part, path: full }); }
  return segments;
}

function goUp(currentPath: string): string {
  const trimmed = currentPath.replace(/[\\/]+$/, "");
  if (!trimmed || trimmed === "/") return "/";
  const slash = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  if (slash <= 0) return "/";
  return trimmed.slice(0, slash);
}

export default function Sidebar({ onOpenLocalTerminal, onConnectSaved, onNewConnection, onEditConnection, onPathChange, onFileSelect, selectedFilePath, workspaceRoot }: Props) {
  const { t } = useI18n();
  const [section, setSection] = useState<0 | 1>(0);
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [currentPath, setCurrentPath] = useState("");
  const [homeDir, setHomeDir] = useState("");
  const [searchText, setSearchText] = useState("");
  const [placesOpen, setPlacesOpen] = useState(false);
  const { connections, refresh: refreshConnections, remove } = useConnectionStore();
  const [serverSearch, setServerSearch] = useState("");
  const placesRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    cmd.coreInfo().then((info: CoreInfo) => {
      setHomeDir(info.homeDir);
      if (!currentPath) setCurrentPath(info.homeDir);
    }).catch(() => {});
  }, []);

  useEffect(() => { if (!currentPath) return; cmd.listDirectory(currentPath).then(setEntries).catch(() => setEntries([])); setSearchText(""); }, [currentPath]);
  useEffect(() => {
    if (!currentPath) return;
    onPathChange?.(currentPath);
  }, [currentPath, onPathChange]);
  useEffect(() => { refreshConnections(); }, []);
  useEffect(() => {
    if (!placesOpen) return;
    const handler = (e: MouseEvent) => { if (placesRef.current && !placesRef.current.contains(e.target as Node)) setPlacesOpen(false); };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [placesOpen]);

  const filteredEntries = entries.filter((e) => !searchText.trim() || e.name.toLowerCase().includes(searchText.toLowerCase()));
  const segments = pathSegments(currentPath, homeDir);

  return (
    <aside className="sidebar">
      <div className="sidebar-tabs">
        <button
          className={section === 0 ? "sidebar-tab active" : "sidebar-tab"}
          onClick={() => setSection(0)}
          type="button"
        >
          <FolderTree size={12} />{t("Files")}
        </button>
        <button
          className={section === 1 ? "sidebar-tab active" : "sidebar-tab"}
          onClick={() => setSection(1)}
          type="button"
        >
          <Server size={12} />{t("Servers")}
        </button>
      </div>

      {section === 0 ? (
        <>
          <div className="sidebar-toolbar">
            <button
              className="mini-btn"
              disabled={!currentPath || currentPath === "/"}
              onClick={() => setCurrentPath(goUp(currentPath))}
              title={t("Up")}
              type="button"
            >
              <ArrowLeft />
            </button>
            <div className="crumb">
              {segments.map((seg, i) => (
                <span key={seg.path} className="crumb-item">
                  {i > 0 && <span className="sep">/</span>}
                  <button
                    className={"seg" + (i === segments.length - 1 ? " last" : "")}
                    onClick={() => setCurrentPath(seg.path)}
                    type="button"
                  >
                    {seg.name}
                  </button>
                </span>
              ))}
            </div>
            <div className="sidebar__places-wrap" ref={placesRef}>
              <button
                className="mini-btn"
                onClick={() => setPlacesOpen((p) => !p)}
                title={t("Places")}
                type="button"
              >
                <MoreHorizontal />
              </button>
              {placesOpen && (
                <div className="sidebar__places-menu">
                  <button className="sidebar__places-item" onClick={() => { setCurrentPath(homeDir); setPlacesOpen(false); }} type="button"><Home size={12} />{t("Home")}</button>
                  <button className="sidebar__places-item" onClick={() => { setCurrentPath(homeDir + "/Desktop"); setPlacesOpen(false); }} type="button"><Monitor size={12} />{t("Desktop")}</button>
                  <button className="sidebar__places-item" onClick={() => { setCurrentPath(homeDir + "/Documents"); setPlacesOpen(false); }} type="button"><FileText size={12} />{t("Documents")}</button>
                  <button className="sidebar__places-item" onClick={() => { setCurrentPath(homeDir + "/Downloads"); setPlacesOpen(false); }} type="button"><Download size={12} />{t("Downloads")}</button>
                  {workspaceRoot && workspaceRoot !== homeDir && (
                    <button className="sidebar__places-item" onClick={() => { setCurrentPath(workspaceRoot); setPlacesOpen(false); }} type="button"><FolderOpen size={12} />{t("Workspace")}</button>
                  )}
                  <div className="sidebar__places-divider" />
                  <button className="sidebar__places-item" onClick={() => { onOpenLocalTerminal(currentPath); setPlacesOpen(false); }} type="button"><Terminal size={12} />{t("Open terminal here")}</button>
                </div>
              )}
            </div>
            <button
              className="mini-btn"
              onClick={() => { cmd.listDirectory(currentPath).then(setEntries).catch(() => {}); }}
              title={t("Refresh")}
              type="button"
            >
              <RefreshCw />
            </button>
          </div>

          <div className="sidebar-search">
            <Search />
            <input
              onChange={(e) => setSearchText(e.currentTarget.value)}
              placeholder={t("Filter files…")}
              value={searchText}
            />
          </div>

          <div className="sidebar-list">
            <div className="sidebar-header-row">
              <span className="col-name">{t("NAME")}</span>
              <span className="col-mod">{t("MOD")}</span>
              <span className="col-size">{t("SIZE")}</span>
            </div>
            {filteredEntries.map((entry) => {
              const isSelected = entry.kind === "file" && selectedFilePath === entry.path;
              const isDir = entry.kind === "directory";
              const isMd = entry.name.toLowerCase().endsWith(".md");
              const cls =
                "file-row" +
                (isDir ? " is-dir" : "") +
                (isMd ? " is-md" : "") +
                (isSelected ? " selected" : "");
              const icon = isDir
                ? <Folder size={12} />
                : isMd
                  ? <FileText size={12} />
                  : <FileText size={12} />;
              return (
                <div
                  key={entry.path}
                  className={cls}
                  onClick={() => {
                    if (isDir) setCurrentPath(entry.path);
                    else onFileSelect?.(entry);
                  }}
                  onDoubleClick={() => { if (isDir) onOpenLocalTerminal(entry.path); }}
                  role="button"
                  tabIndex={0}
                >
                  <span className="fi">{icon}</span>
                  <span className="fname">{entry.name}</span>
                  <span className="fmod">{entry.modified}</span>
                  <span className="fsize">{entry.sizeLabel}</span>
                </div>
              );
            })}
            {filteredEntries.length === 0 && (
              <div className="empty-note" style={{ padding: 12 }}>
                {searchText ? t("No matching files") : t("Empty directory")}
              </div>
            )}
          </div>
        </>
      ) : (
        <ServersPane
          connections={connections}
          serverSearch={serverSearch}
          onSearchChange={setServerSearch}
          onConnect={onConnectSaved}
          onEdit={onEditConnection}
          onRemove={(index) => { void remove(index).catch(() => {}); }}
          onNew={onNewConnection}
          onRefresh={() => { void refreshConnections(); }}
        />
      )}
    </aside>
  );
}

function ServersPane({
  connections,
  serverSearch,
  onSearchChange,
  onConnect,
  onEdit,
  onRemove,
  onNew,
  onRefresh,
}: {
  connections: SavedSshConnection[];
  serverSearch: string;
  onSearchChange: (s: string) => void;
  onConnect: (index: number) => void;
  onEdit: (index: number) => void;
  onRemove: (index: number) => void;
  onNew: () => void;
  onRefresh: () => void;
}) {
  const totalCount = connections.length;
  const { t } = useI18n();
  const groups = useMemo(() => groupConnections(connections, serverSearch), [connections, serverSearch]);

  const tabs = useTabStore((s) => s.tabs);
  const byTab = useDetectedServicesStore((s) => s.byTab);

  const detectionByIndex = useMemo(() => {
    const map = new Map<number, { online: boolean; tools: Set<RightTool> }>();
    for (const conn of connections) {
      let tab = tabs.find(
        (t) => t.backend === "ssh" && t.sshSavedConnectionIndex === conn.index,
      );
      if (!tab) {
        tab = tabs.find(
          (t) =>
            t.backend === "ssh" &&
            t.sshHost === conn.host &&
            t.sshPort === conn.port &&
            t.sshUser === conn.user,
        );
      }
      if (!tab) continue;
      const entry = byTab[tab.id];
      if (!entry) continue;
      map.set(conn.index, {
        online: entry.status === "ready",
        tools: entry.tools,
      });
    }
    return map;
  }, [connections, tabs, byTab]);

  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [openRow, setOpenRow] = useState<number | null>(null);

  useEffect(() => {
    setExpanded((prev) => {
      const next = { ...prev };
      let changed = false;
      for (const g of groups) {
        if (next[g.name] === undefined) {
          next[g.name] = true;
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [groups]);

  const shownCount = groups.reduce((acc, g) => acc + g.servers.length, 0);

  return (
    <>
      <div className="sidebar-toolbar">
        <button className="mini-btn" onClick={onNew} title={t("New SSH connection")} type="button"><Plus /></button>
        <div className="crumb">
          <span className="crumb-item">
            <span className="seg last">{t("SSH connections")}</span>
          </span>
          <span className="sep" style={{ marginLeft: 6 }}>·</span>
          <span className="crumb-item">
            <span className="seg" style={{ fontFamily: "var(--mono)", fontSize: 10 }}>{totalCount}</span>
          </span>
        </div>
        <button className="mini-btn" onClick={onRefresh} title={t("Refresh")} type="button"><RefreshCw /></button>
      </div>

      <div className="sidebar-search">
        <Search />
        <input
          onChange={(e) => onSearchChange(e.currentTarget.value)}
          placeholder={t("Filter connections…")}
          value={serverSearch}
        />
      </div>

      <div className="sidebar-list srv-list">
        {groups.map((g) => {
          const open = expanded[g.name] ?? true;
          const onlineCount = g.servers.filter((s) => detectionByIndex.get(s.index)?.online).length;
          return (
            <div key={g.name} className={"srv-group" + (open ? " open" : "")}>
              <button
                className="srv-group-head"
                onClick={() => setExpanded({ ...expanded, [g.name]: !open })}
                type="button"
              >
                <span className="srv-chev"><ChevronRight size={10} /></span>
                <span className="srv-group-name">{g.name}</span>
                <span className="srv-group-meta">{onlineCount}/{g.servers.length}</span>
              </button>
              {open && g.servers.map((s) => {
                const det = detectionByIndex.get(s.index);
                return (
                  <ServerItem
                    key={s.index}
                    conn={s}
                    isOpen={openRow === s.index}
                    online={det?.online ?? false}
                    detectedTools={det?.tools}
                    onToggle={() => setOpenRow((cur) => (cur === s.index ? null : s.index))}
                    onConnect={() => onConnect(s.index)}
                    onEdit={() => onEdit(s.index)}
                    onRemove={() => onRemove(s.index)}
                    editLabel={t("Edit")}
                    deleteLabel={t("Delete")}
                    connectLabel={t("Connect")}
                    hintLabel={t("Connect to discover services")}
                    noneLabel={t("No services detected")}
                    detectedLabel={t("Detected · click to open")}
                  />
                );
              })}
            </div>
          );
        })}
        {shownCount === 0 && (
          <div className="empty-note" style={{ padding: 12 }}>
            {totalCount === 0 ? t("No saved connections") : t("No matching connections")}
          </div>
        )}
      </div>
    </>
  );
}

function ServerItem({
  conn,
  isOpen,
  online,
  detectedTools,
  onToggle,
  onConnect,
  onEdit,
  onRemove,
  editLabel,
  deleteLabel,
  connectLabel,
  hintLabel,
  noneLabel,
  detectedLabel,
}: {
  conn: SavedSshConnection & { display: string };
  isOpen: boolean;
  online: boolean;
  detectedTools?: Set<RightTool>;
  onToggle: () => void;
  onConnect: () => void;
  onEdit: () => void;
  onRemove: () => void;
  editLabel: string;
  deleteLabel: string;
  connectLabel: string;
  hintLabel: string;
  noneLabel: string;
  detectedLabel: string;
}) {
  const AuthIcon: LucideIcon = conn.authKind === "key" ? Key : conn.authKind === "agent" ? Shield : Lock;
  const addr = `${conn.user}@${conn.host}${conn.port !== 22 ? `:${conn.port}` : ""}`;
  const chips = detectedTools
    ? Object.values(SERVICE_META).filter((m) => detectedTools.has(m.tool))
    : [];
  return (
    <div className={"srv-item" + (online ? "" : " offline")}>
      <div className="srv-row" onClick={onToggle}>
        <span className={"srv-dot " + (online ? "on" : "off")} />
        <div className="srv-body">
          <div className="srv-name">{conn.display}</div>
          <div className="srv-addr">{addr}</div>
        </div>
        <span className="srv-auth" title={"auth: " + conn.authKind}>
          <AuthIcon size={10} />
        </span>
        <div className="srv-actions" onClick={(e) => e.stopPropagation()}>
          <button className="mini-btn" onClick={onConnect} title={connectLabel} type="button">
            <Terminal />
          </button>
          <button className="mini-btn" onClick={onEdit} title={editLabel} type="button">
            <Pencil />
          </button>
          <button className="mini-btn" onClick={onRemove} title={deleteLabel} type="button">
            <Trash2 />
          </button>
        </div>
      </div>
      {isOpen && (
        <div className="srv-svcs">
          {!online && <div className="srv-svcs-empty">{hintLabel}</div>}
          {online && chips.length === 0 && <div className="srv-svcs-empty">{noneLabel}</div>}
          {online && chips.length > 0 && (
            <>
              <div className="srv-svcs-label">{detectedLabel}</div>
              <div className="srv-svcs-row">
                {chips.map((m) => {
                  const Ic = m.icon;
                  return (
                    <span
                      key={m.tool}
                      className="srv-svc"
                      style={{ ["--svc-tint" as string]: m.tintVar }}
                      title={m.label}
                    >
                      <Ic size={10} />
                      <span className="svc-name">{m.label}</span>
                    </span>
                  );
                })}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
