import {
  ArrowLeft,
  ArrowUp,
  ChevronRight,
  FileText,
  Folder,
  FolderPlus,
  FolderTree,
  GripVertical,
  Home,
  Key,
  Lock,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Server,
  Shield,
  Terminal,
  Trash2,
} from "lucide-react";
import type { DragEvent as ReactDragEvent, KeyboardEvent as ReactKeyboardEvent } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { CoreInfo, FileEntry, SavedSshConnection, RightTool } from "../lib/types";
import { RIGHT_TOOL_META, SERVICE_CHIP_TOOLS, type LucideIcon } from "../lib/rightToolMeta";
import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useTabStore } from "../stores/useTabStore";
import { useDetectedServicesStore } from "../stores/useDetectedServicesStore";
import ContextMenu, { type ContextMenuItem } from "../components/ContextMenu";
import {
  DT_LOCAL_FILE,
  DT_SFTP_FILE,
  type LocalDragPayload,
  type SftpDragPayload,
} from "../panels/SftpPanel";

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

type ServiceChip = {
  tool: RightTool;
  label: string;
  icon: LucideIcon;
  tintVar: string;
};

const SERVICE_META: ServiceChip[] = SERVICE_CHIP_TOOLS.map((tool) => ({
  tool,
  label: RIGHT_TOOL_META[tool].label,
  icon: RIGHT_TOOL_META[tool].icon,
  tintVar: RIGHT_TOOL_META[tool].tintVar ?? "var(--accent)",
}));

/** Empty string = implicit "default" bucket. */
type GroupKey = string;

/** Derive the effective group label + display name for a connection.
 *  Prefers the explicit `group` field; falls back to legacy "Group/Name"
 *  slash-naming when `group` is missing so pre-migration data still
 *  shows clustered. */
function effectiveGroup(conn: SavedSshConnection): { group: GroupKey; display: string } {
  const explicit = (conn.group ?? "").trim();
  if (explicit) return { group: explicit, display: conn.name };
  const slash = conn.name.indexOf("/");
  if (slash > 0 && slash < conn.name.length - 1) {
    return {
      group: conn.name.slice(0, slash).trim(),
      display: conn.name.slice(slash + 1).trim(),
    };
  }
  return { group: "", display: conn.name };
}

type ConnectionGroup = {
  key: GroupKey;
  servers: Array<SavedSshConnection & { display: string }>;
};

/** Group connections preserving first-appearance order — the backend
 *  is responsible for keeping group members contiguous, so the display
 *  order matches the stored array order. */
function groupConnections(conns: SavedSshConnection[], query: string): ConnectionGroup[] {
  const q = query.trim().toLowerCase();
  const order: GroupKey[] = [];
  const byKey = new Map<GroupKey, ConnectionGroup>();
  for (const c of conns) {
    const { group, display } = effectiveGroup(c);
    if (q) {
      const hay = (c.name + c.host + c.user + group).toLowerCase();
      if (!hay.includes(q)) continue;
    }
    let entry = byKey.get(group);
    if (!entry) {
      entry = { key: group, servers: [] };
      byKey.set(group, entry);
      order.push(group);
    }
    entry.servers.push({ ...c, display });
  }
  return order.map((k) => byKey.get(k)!);
}

// ── Drag-drop helpers ─────────────────────────────────────────────

const DT_SERVER = "application/x-pier-server";
const DT_GROUP = "application/x-pier-group";

/** Compute a reorder that moves `srcIndex` adjacent to `targetIndex`
 *  (before or after depending on `position`) in the target group.
 *  Always keeps same-group members contiguous by re-inserting next to
 *  the target. */
function planServerMove(
  conns: SavedSshConnection[],
  srcIndex: number,
  targetIndex: number,
  position: "before" | "after",
  targetGroup: GroupKey,
): { order: number[]; groups: Array<string | null> } {
  const ids = conns.map((_, i) => i).filter((i) => i !== srcIndex);
  const groupOf = (i: number): GroupKey =>
    i === srcIndex ? targetGroup : effectiveGroup(conns[i]).group;
  // Find insertion slot: after filtering src out, locate target and
  // insert before/after.
  const tIdx = ids.indexOf(targetIndex);
  const slot = tIdx < 0
    ? ids.length
    : position === "before" ? tIdx : tIdx + 1;
  ids.splice(slot, 0, srcIndex);
  const order = ids;
  const groups: Array<string | null> = order.map((i) => {
    const g = groupOf(i);
    return g ? g : null;
  });
  return { order, groups };
}

/** Move a server to the end of a group. If the group currently has no
 *  members, append at the end of the list. */
function planServerMoveToGroupEnd(
  conns: SavedSshConnection[],
  srcIndex: number,
  targetGroup: GroupKey,
): { order: number[]; groups: Array<string | null> } {
  const ids = conns.map((_, i) => i).filter((i) => i !== srcIndex);
  // Find the last index in ids whose group equals targetGroup.
  let lastMember = -1;
  for (let k = 0; k < ids.length; k++) {
    if (effectiveGroup(conns[ids[k]]).group === targetGroup) lastMember = k;
  }
  const slot = lastMember >= 0 ? lastMember + 1 : ids.length;
  ids.splice(slot, 0, srcIndex);
  const order = ids;
  const groups: Array<string | null> = order.map((i) => {
    const g = i === srcIndex ? targetGroup : effectiveGroup(conns[i]).group;
    return g ? g : null;
  });
  return { order, groups };
}

/** Reorder whole groups: move every member of `srcGroup` before or
 *  after the members of `targetGroup`. Groups themselves keep their
 *  labels. */
function planGroupMove(
  conns: SavedSshConnection[],
  srcGroup: GroupKey,
  targetGroup: GroupKey,
  position: "before" | "after",
): { order: number[]; groups: Array<string | null> } | null {
  if (srcGroup === targetGroup) return null;
  const srcIndices: number[] = [];
  const otherIndices: number[] = [];
  for (let i = 0; i < conns.length; i++) {
    if (effectiveGroup(conns[i]).group === srcGroup) srcIndices.push(i);
    else otherIndices.push(i);
  }
  if (srcIndices.length === 0) return null;
  // Find position in `otherIndices` to splice the src block in.
  let slot = -1;
  for (let k = 0; k < otherIndices.length; k++) {
    if (effectiveGroup(conns[otherIndices[k]]).group === targetGroup) {
      if (position === "before") {
        slot = k;
        break;
      }
      slot = k + 1; // keep advancing to last occurrence
    }
  }
  if (slot < 0) slot = otherIndices.length;
  const order = [
    ...otherIndices.slice(0, slot),
    ...srcIndices,
    ...otherIndices.slice(slot),
  ];
  const groups: Array<string | null> = order.map((i) => {
    const g = effectiveGroup(conns[i]).group;
    return g ? g : null;
  });
  return { order, groups };
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

export default function Sidebar({ onOpenLocalTerminal, onConnectSaved, onNewConnection, onEditConnection, onPathChange, onFileSelect, selectedFilePath }: Props) {
  const { t } = useI18n();
  const [section, setSection] = useState<0 | 1>(0);
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [currentPath, setCurrentPath] = useState("");
  const [homeDir, setHomeDir] = useState("");
  const [pathHistory, setPathHistory] = useState<string[]>([]);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const [searchText, setSearchText] = useState("");
  const { connections, refresh: refreshConnections, remove } = useConnectionStore();
  const [serverSearch, setServerSearch] = useState("");

  useEffect(() => {
    cmd.coreInfo().then((info: CoreInfo) => {
      setHomeDir(info.homeDir);
      const startPath = normalizePath(info.workspaceRoot || info.homeDir);
      if (!currentPath) {
        setCurrentPath(startPath);
        setPathHistory([startPath]);
        setHistoryIndex(0);
      }
    }).catch(() => {});
  }, []);

  useEffect(() => { if (!currentPath) return; cmd.listDirectory(currentPath).then(setEntries).catch(() => setEntries([])); setSearchText(""); }, [currentPath]);
  useEffect(() => {
    if (!currentPath) return;
    onPathChange?.(currentPath);
  }, [currentPath, onPathChange]);
  useEffect(() => { refreshConnections(); }, []);

  const filteredEntries = entries.filter((e) => !searchText.trim() || e.name.toLowerCase().includes(searchText.toLowerCase()));
  const segments = pathSegments(currentPath, homeDir);

  // ── Sidebar ↔ SFTP drag-drop ───────────────────────────────────
  //
  // The local file list is both a drag *source* (drop into SFTP
  // uploads the file) and a drag *target* (drop a remote file from
  // SFTP downloads into the current local directory). The drop uses
  // the active SSH tab's credentials — no extra IPC round-trip
  // needed since the SFTP cache is keyed by addressing, not secrets.
  const tabs = useTabStore((s) => s.tabs);
  const activeTabId = useTabStore((s) => s.activeTabId);
  const [sftpDropDepth, setSftpDropDepth] = useState(0);
  const sftpDropActive = sftpDropDepth > 0;

  function resolveSshTabForPayload(payload: SftpDragPayload) {
    // Prefer the active tab if it matches, so the download uses the
    // same cached session that just populated the SFTP panel.
    const active = tabs.find((tab) => tab.id === activeTabId);
    if (
      active &&
      active.backend === "ssh" &&
      active.sshHost === payload.host &&
      active.sshPort === payload.port &&
      active.sshUser === payload.user &&
      active.sshAuthMode === payload.authMode
    ) {
      return active;
    }
    return tabs.find(
      (tab) =>
        tab.backend === "ssh" &&
        tab.sshHost === payload.host &&
        tab.sshPort === payload.port &&
        tab.sshUser === payload.user &&
        tab.sshAuthMode === payload.authMode,
    );
  }

  async function handleSftpDropDownload(payload: SftpDragPayload) {
    const sshTab = resolveSshTabForPayload(payload);
    if (!sshTab) return;
    const dir = currentPath.trim().replace(/[\\/]+$/, "");
    if (!dir) return;
    const sep = /^[A-Za-z]:[\\/]|^\\\\/.test(dir) ? "\\" : "/";
    const localPath = `${dir}${sep}${payload.name}`;
    try {
      if (payload.isDir) {
        await cmd.sftpDownloadTree({
          host: sshTab.sshHost,
          port: sshTab.sshPort,
          user: sshTab.sshUser,
          authMode: sshTab.sshAuthMode,
          password: sshTab.sshPassword,
          keyPath: sshTab.sshKeyPath,
          savedConnectionIndex: sshTab.sshSavedConnectionIndex,
          remotePath: payload.path,
          localPath,
        });
      } else {
        await cmd.sftpDownload({
          host: sshTab.sshHost,
          port: sshTab.sshPort,
          user: sshTab.sshUser,
          authMode: sshTab.sshAuthMode,
          password: sshTab.sshPassword,
          keyPath: sshTab.sshKeyPath,
          savedConnectionIndex: sshTab.sshSavedConnectionIndex,
          remotePath: payload.path,
          localPath,
        });
      }
      // Refresh the file list so the newly-downloaded file shows up.
      cmd.listDirectory(currentPath).then(setEntries).catch(() => {});
    } catch (e) {
      // Swallow — the SFTP panel's own error bar is the right surface
      // for SFTP errors; the sidebar shouldn't grow its own toast
      // system just for drop feedback. Log for debugging.
      console.warn("sftp download from drop failed", localizeError(e, t));
    }
  }

  function handleFileListDragEnter(event: ReactDragEvent<HTMLDivElement>) {
    if (!Array.from(event.dataTransfer.types).includes(DT_SFTP_FILE)) return;
    event.preventDefault();
    setSftpDropDepth((d) => d + 1);
  }
  function handleFileListDragOver(event: ReactDragEvent<HTMLDivElement>) {
    if (!Array.from(event.dataTransfer.types).includes(DT_SFTP_FILE)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  }
  function handleFileListDragLeave(event: ReactDragEvent<HTMLDivElement>) {
    if (!Array.from(event.dataTransfer.types).includes(DT_SFTP_FILE)) return;
    event.preventDefault();
    setSftpDropDepth((d) => Math.max(0, d - 1));
  }
  function handleFileListDrop(event: ReactDragEvent<HTMLDivElement>) {
    setSftpDropDepth(0);
    const raw = event.dataTransfer.getData(DT_SFTP_FILE);
    if (!raw) return;
    event.preventDefault();
    try {
      const payload = JSON.parse(raw) as SftpDragPayload;
      if (payload.isDir) return;
      void handleSftpDropDownload(payload);
    } catch {
      /* malformed payload */
    }
  }

  function handleLocalRowDragStart(event: ReactDragEvent<HTMLDivElement>, entry: FileEntry) {
    // Both files and directories are draggable. The SFTP panel's
    // drop handler picks the single-file or recursive tree command
    // based on `isDir`.
    const payload: LocalDragPayload = {
      path: entry.path,
      name: entry.name,
      isDir: entry.kind === "directory",
    };
    event.dataTransfer.effectAllowed = "copy";
    event.dataTransfer.setData(DT_LOCAL_FILE, JSON.stringify(payload));
  }

  function pushPath(nextPath: string) {
    const normalized = normalizePath(nextPath);
    if (!normalized || normalized === currentPath) return;
    const nextHistory = pathHistory.slice(0, historyIndex + 1);
    nextHistory.push(normalized);
    setPathHistory(nextHistory);
    setHistoryIndex(nextHistory.length - 1);
    setCurrentPath(normalized);
  }

  function goBackPath() {
    if (historyIndex <= 0) return;
    const nextIndex = historyIndex - 1;
    const nextPath = pathHistory[nextIndex];
    if (!nextPath) return;
    setHistoryIndex(nextIndex);
    setCurrentPath(nextPath);
  }

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
              disabled={historyIndex <= 0}
              onClick={goBackPath}
              title={t("Back")}
              type="button"
            >
              <ArrowLeft />
            </button>
            <button
              className="mini-btn"
              disabled={!currentPath || currentPath === "/"}
              onClick={() => pushPath(goUp(currentPath))}
              title={t("Up")}
              type="button"
            >
              <ArrowUp />
            </button>
            <button
              className="mini-btn"
              disabled={!homeDir}
              onClick={() => pushPath(homeDir)}
              title={t("Home")}
              type="button"
            >
              <Home />
            </button>
            <div className="crumb">
              {segments.map((seg, i) => (
                <span key={seg.path} className="crumb-item">
                  {i > 0 && <span className="sep">/</span>}
                  <button
                    className={"seg" + (i === segments.length - 1 ? " last" : "")}
                    onClick={() => pushPath(seg.path)}
                    type="button"
                  >
                    {seg.name}
                  </button>
                </span>
              ))}
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

          <div
            className={"sidebar-list" + (sftpDropActive ? " is-drop" : "")}
            onDragEnter={handleFileListDragEnter}
            onDragOver={handleFileListDragOver}
            onDragLeave={handleFileListDragLeave}
            onDrop={handleFileListDrop}
          >
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
                    if (isDir) pushPath(entry.path);
                    else onFileSelect?.(entry);
                  }}
                  onDoubleClick={() => { if (isDir) onOpenLocalTerminal(entry.path); }}
                  role="button"
                  tabIndex={0}
                  draggable
                  onDragStart={(e) => handleLocalRowDragStart(e, entry)}
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
          onReorder={(order, groups) => useConnectionStore.getState().reorder(order, groups)}
          onRenameGroup={(from, to) => useConnectionStore.getState().renameGroup(from, to)}
        />
      )}
    </aside>
  );
}

function normalizePath(path: string): string {
  const value = String(path || "").trim().replace(/[\\/]+$/, "");
  return value || "/";
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
  onReorder,
  onRenameGroup,
}: {
  connections: SavedSshConnection[];
  serverSearch: string;
  onSearchChange: (s: string) => void;
  onConnect: (index: number) => void;
  onEdit: (index: number) => void;
  onRemove: (index: number) => void;
  onNew: () => void;
  onRefresh: () => void;
  onReorder: (order: number[], groups: Array<string | null>) => Promise<void>;
  onRenameGroup: (from: string, to: string | null) => Promise<void>;
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
  // Pending = user-created placeholder that only lives in this UI
  // session until the user confirms its name (and, for empty pending
  // groups, drags a server in). `attachServer` is set when the pending
  // group was created from a "Move to new group…" action — naming it
  // commits by reordering that server into the new group.
  const [pendingGroup, setPendingGroup] = useState<
    { name: string; editing: boolean; attachServer?: number } | null
  >(null);
  const [renamingGroup, setRenamingGroup] = useState<GroupKey | null>(null);
  // Drag state — keeps rendering light: we only store what's needed
  // for the drop-indicator, not the whole ghost.
  const [dragServer, setDragServer] = useState<number | null>(null);
  const [dragGroup, setDragGroup] = useState<GroupKey | null>(null);
  const [dropTargetRow, setDropTargetRow] = useState<
    { index: number; position: "before" | "after" } | null
  >(null);
  const [dropTargetGroup, setDropTargetGroup] = useState<
    { key: GroupKey; mode: "into" | "before" | "after" } | null
  >(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: ContextMenuItem[] } | null>(null);

  useEffect(() => {
    setExpanded((prev) => {
      const next = { ...prev };
      let changed = false;
      for (const g of groups) {
        if (next[g.key] === undefined) {
          next[g.key] = true;
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [groups]);

  // Pending group is rendered separately from `groups` so its
  // transient state (empty name while editing) doesn't collide with
  // the real default bucket (key === "").
  const pendingVisible =
    pendingGroup !== null &&
    (pendingGroup.editing ||
      (pendingGroup.name.length > 0 && !groups.some((g) => g.key === pendingGroup.name)));

  const shownCount = groups.reduce((acc, g) => acc + g.servers.length, 0);

  const clearDrag = () => {
    setDragServer(null);
    setDragGroup(null);
    setDropTargetRow(null);
    setDropTargetGroup(null);
  };

  const groupLabel = (key: GroupKey) => (key === "" ? t("Default") : key);

  const applyReorder = (
    order: number[],
    groupLabels: Array<string | null>,
  ) => {
    clearDrag();
    void onReorder(order, groupLabels).catch(() => {});
  };

  const openMoveMenu = (event: ReactDragEvent | React.MouseEvent, conn: SavedSshConnection) => {
    event.preventDefault();
    const items: ContextMenuItem[] = [];
    const currentGroup = effectiveGroup(conn).group;
    const seen = new Set<GroupKey>();
    for (const g of groups) {
      if (seen.has(g.key)) continue;
      seen.add(g.key);
      items.push({
        label: `${t("Move to group")}: ${groupLabel(g.key)}`,
        disabled: g.key === currentGroup,
        action: () => {
          const plan = planServerMoveToGroupEnd(connections, conn.index, g.key);
          applyReorder(plan.order, plan.groups);
        },
      });
    }
    if (currentGroup !== "") {
      items.push({
        label: t("Ungroup"),
        action: () => {
          const plan = planServerMoveToGroupEnd(connections, conn.index, "");
          applyReorder(plan.order, plan.groups);
        },
      });
    }
    items.push({ divider: true });
    items.push({
      label: t("Move to new group…"),
      action: () => {
        setPendingGroup({ name: "", editing: true, attachServer: conn.index });
      },
    });
    setMenu({ x: event.clientX, y: event.clientY, items });
  };

  const openGroupMenu = (event: React.MouseEvent, key: GroupKey, pending: boolean) => {
    event.preventDefault();
    const items: ContextMenuItem[] = [];
    if (!pending) {
      items.push({
        label: t("Rename group"),
        action: () => setRenamingGroup(key),
        disabled: key === "",
      });
      items.push({
        label: t("Delete group"),
        disabled: key === "",
        action: () => {
          void onRenameGroup(key, null).catch(() => {});
        },
      });
      items.push({ divider: true });
    }
    items.push({
      label: t("New group…"),
      action: () => {
        setPendingGroup({ name: "", editing: true });
      },
    });
    setMenu({ x: event.clientX, y: event.clientY, items });
  };

  const commitRename = (oldKey: GroupKey, nextName: string) => {
    setRenamingGroup(null);
    const trimmed = nextName.trim();
    if (!trimmed || trimmed === oldKey) return;
    void onRenameGroup(oldKey, trimmed).catch(() => {});
  };

  // User confirmed a pending group's name. If it was opened via
  // "Move to new group…", auto-commit by moving that server in.
  // Otherwise flip the pending row to named-but-empty-and-waiting.
  const commitPendingName = (name: string) => {
    const trimmed = name.trim();
    if (!trimmed) {
      setPendingGroup(null);
      return;
    }
    const existing = groups.find((g) => g.key === trimmed);
    if (existing) {
      // Name collides with a real group → just use that group as the target.
      if (pendingGroup?.attachServer !== undefined) {
        const plan = planServerMoveToGroupEnd(connections, pendingGroup.attachServer, trimmed);
        applyReorder(plan.order, plan.groups);
      }
      setPendingGroup(null);
      setExpanded((prev) => ({ ...prev, [trimmed]: true }));
      return;
    }
    if (pendingGroup?.attachServer !== undefined) {
      // Auto-commit the pending group by moving the attached server in.
      const plan = planServerMoveToGroupEnd(connections, pendingGroup.attachServer, trimmed);
      applyReorder(plan.order, plan.groups);
      setPendingGroup(null);
    } else {
      setPendingGroup({ name: trimmed, editing: false });
    }
    setExpanded((prev) => ({ ...prev, [trimmed]: true }));
  };

  return (
    <>
      <div className="sidebar-toolbar">
        <button className="mini-btn" onClick={onNew} title={t("New SSH connection")} type="button"><Plus /></button>
        <button
          className="mini-btn"
          onClick={() => setPendingGroup({ name: "", editing: true })}
          title={t("New group")}
          type="button"
        >
          <FolderPlus />
        </button>
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

      <div
        className="sidebar-list srv-list"
        onDragEnd={clearDrag}
        onDragLeave={(e) => {
          // Only clear if pointer actually left the list.
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
            setDropTargetRow(null);
            setDropTargetGroup(null);
          }
        }}
      >
        {groups.map((g) => {
          const open = expanded[g.key] ?? true;
          const onlineCount = g.servers.filter((s) => detectionByIndex.get(s.index)?.online).length;
          const draggable = g.key !== "";
          const isDragging = dragGroup === g.key;
          const dropClass =
            dropTargetGroup && dropTargetGroup.key === g.key
              ? " drop-" + dropTargetGroup.mode
              : "";
          return (
            <div
              key={`grp-${g.key || "__default__"}`}
              className={
                "srv-group" +
                (open ? " open" : "") +
                (isDragging ? " dragging" : "") +
                dropClass
              }
              onDragOver={(e) => {
                if (dragServer === null && dragGroup === null) return;
                e.preventDefault();
                if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
              }}
              onDrop={(e) => {
                if (dragServer !== null) {
                  e.preventDefault();
                  const plan = planServerMoveToGroupEnd(connections, dragServer, g.key);
                  applyReorder(plan.order, plan.groups);
                } else if (dragGroup !== null && dragGroup !== g.key) {
                  e.preventDefault();
                  const plan = planGroupMove(connections, dragGroup, g.key, "before");
                  if (plan) applyReorder(plan.order, plan.groups);
                }
              }}
            >
              <div
                className="srv-group-head"
                draggable={draggable}
                onDragStart={(e) => {
                  if (!draggable) return;
                  setDragGroup(g.key);
                  if (e.dataTransfer) {
                    e.dataTransfer.effectAllowed = "move";
                    e.dataTransfer.setData(DT_GROUP, g.key);
                  }
                }}
                onDragOver={(e) => {
                  if (dragServer !== null) {
                    e.preventDefault();
                    setDropTargetGroup({ key: g.key, mode: "into" });
                  } else if (dragGroup !== null && dragGroup !== g.key) {
                    e.preventDefault();
                    const rect = (e.currentTarget as HTMLDivElement).getBoundingClientRect();
                    const mid = rect.top + rect.height / 2;
                    setDropTargetGroup({
                      key: g.key,
                      mode: e.clientY < mid ? "before" : "after",
                    });
                  }
                }}
                onClick={() => {
                  if (renamingGroup === g.key) return;
                  setExpanded({ ...expanded, [g.key]: !open });
                }}
                onContextMenu={(e) => openGroupMenu(e, g.key, false)}
                role="button"
                tabIndex={0}
              >
                {draggable && <span className="srv-grip" aria-hidden><GripVertical size={10} /></span>}
                <span className="srv-chev"><ChevronRight size={10} /></span>
                {renamingGroup === g.key ? (
                  <GroupRenameInput
                    initial={g.key}
                    onCancel={() => setRenamingGroup(null)}
                    onCommit={(name) => commitRename(g.key, name)}
                  />
                ) : (
                  <span className="srv-group-name">{groupLabel(g.key)}</span>
                )}
                <span className="srv-group-meta">{onlineCount}/{g.servers.length}</span>
              </div>
              {open && g.servers.map((s) => {
                const det = detectionByIndex.get(s.index);
                const rowDrop = dropTargetRow && dropTargetRow.index === s.index
                  ? " drop-" + dropTargetRow.position
                  : "";
                return (
                  <ServerItem
                    key={s.index}
                    conn={s}
                    groupKey={g.key}
                    isOpen={openRow === s.index}
                    isDragging={dragServer === s.index}
                    dropClass={rowDrop}
                    online={det?.online ?? false}
                    detectedTools={det?.tools}
                    onToggle={() => setOpenRow((cur) => (cur === s.index ? null : s.index))}
                    onConnect={() => onConnect(s.index)}
                    onEdit={() => onEdit(s.index)}
                    onRemove={() => onRemove(s.index)}
                    onContextMenu={(e) => openMoveMenu(e, s)}
                    onDragStart={(e) => {
                      setDragServer(s.index);
                      if (e.dataTransfer) {
                        e.dataTransfer.effectAllowed = "move";
                        e.dataTransfer.setData(DT_SERVER, String(s.index));
                      }
                    }}
                    onDragOverRow={(e) => {
                      if (dragServer === null || dragServer === s.index) return;
                      e.preventDefault();
                      const rect = (e.currentTarget as HTMLDivElement).getBoundingClientRect();
                      const mid = rect.top + rect.height / 2;
                      setDropTargetRow({
                        index: s.index,
                        position: e.clientY < mid ? "before" : "after",
                      });
                      setDropTargetGroup(null);
                    }}
                    onDropRow={(e) => {
                      if (dragServer === null || dragServer === s.index) return;
                      e.preventDefault();
                      const rect = (e.currentTarget as HTMLDivElement).getBoundingClientRect();
                      const mid = rect.top + rect.height / 2;
                      const position: "before" | "after" = e.clientY < mid ? "before" : "after";
                      const plan = planServerMove(
                        connections,
                        dragServer,
                        s.index,
                        position,
                        g.key,
                      );
                      applyReorder(plan.order, plan.groups);
                    }}
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
        {pendingGroup && pendingVisible && (
          <div
            className={"srv-group open pending" + (dropTargetGroup && dropTargetGroup.key === pendingGroup.name ? " drop-into" : "")}
            onDragOver={(e) => {
              if (dragServer === null || !pendingGroup.name) return;
              e.preventDefault();
              if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
              setDropTargetGroup({ key: pendingGroup.name, mode: "into" });
            }}
            onDrop={(e) => {
              if (dragServer === null || !pendingGroup.name) return;
              e.preventDefault();
              const plan = planServerMoveToGroupEnd(connections, dragServer, pendingGroup.name);
              applyReorder(plan.order, plan.groups);
              setPendingGroup(null);
            }}
          >
            <div
              className="srv-group-head"
              onContextMenu={(e) => openGroupMenu(e, pendingGroup.name, true)}
            >
              <span className="srv-chev"><ChevronRight size={10} /></span>
              {pendingGroup.editing ? (
                <GroupRenameInput
                  initial={pendingGroup.name}
                  onCancel={() => setPendingGroup(null)}
                  onCommit={commitPendingName}
                />
              ) : (
                <span
                  className="srv-group-name"
                  onClick={() => setPendingGroup({ ...pendingGroup, editing: true })}
                  title={t("Rename group")}
                  role="button"
                  tabIndex={0}
                >
                  {pendingGroup.name}
                </span>
              )}
              <button
                className="mini-btn"
                onClick={() => setPendingGroup(null)}
                title={t("Cancel")}
                type="button"
                style={{ marginLeft: "auto" }}
              >
                <Trash2 />
              </button>
            </div>
            {!pendingGroup.editing && pendingGroup.name && (
              <div className="srv-group-empty">{t("Drag a server here")}</div>
            )}
          </div>
        )}
        {shownCount === 0 && !pendingVisible && (
          <div className="empty-note" style={{ padding: 12 }}>
            {totalCount === 0 ? t("No saved connections") : t("No matching connections")}
          </div>
        )}
      </div>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menu.items}
          onClose={() => setMenu(null)}
        />
      )}
    </>
  );
}

function GroupRenameInput({
  initial,
  onCommit,
  onCancel,
}: {
  initial: string;
  onCommit: (name: string) => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  return (
    <input
      ref={ref}
      className="srv-group-rename"
      defaultValue={initial}
      onBlur={(e) => onCommit(e.currentTarget.value)}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e: ReactKeyboardEvent<HTMLInputElement>) => {
        if (e.key === "Enter") onCommit(e.currentTarget.value);
        else if (e.key === "Escape") onCancel();
        e.stopPropagation();
      }}
    />
  );
}

function ServerItem({
  conn,
  groupKey,
  isOpen,
  isDragging,
  dropClass,
  online,
  detectedTools,
  onToggle,
  onConnect,
  onEdit,
  onRemove,
  onContextMenu,
  onDragStart,
  onDragOverRow,
  onDropRow,
  editLabel,
  deleteLabel,
  connectLabel,
  hintLabel,
  noneLabel,
  detectedLabel,
}: {
  conn: SavedSshConnection & { display: string };
  groupKey: GroupKey;
  isOpen: boolean;
  isDragging: boolean;
  dropClass: string;
  online: boolean;
  detectedTools?: Set<RightTool>;
  onToggle: () => void;
  onConnect: () => void;
  onEdit: () => void;
  onRemove: () => void;
  onContextMenu: (event: React.MouseEvent) => void;
  onDragStart: (event: ReactDragEvent<HTMLDivElement>) => void;
  onDragOverRow: (event: ReactDragEvent<HTMLDivElement>) => void;
  onDropRow: (event: ReactDragEvent<HTMLDivElement>) => void;
  editLabel: string;
  deleteLabel: string;
  connectLabel: string;
  hintLabel: string;
  noneLabel: string;
  detectedLabel: string;
}) {
  // groupKey isn't rendered directly — it's only here so the parent's
  // drag handler has the right context. Reference it to keep TS happy.
  void groupKey;
  const AuthIcon: LucideIcon = conn.authKind === "key" ? Key : conn.authKind === "agent" ? Shield : Lock;
  const { t } = useI18n();
  const addr = `${conn.user}@${conn.host}${conn.port !== 22 ? `:${conn.port}` : ""}`;
  const chips = detectedTools
    ? SERVICE_META.filter((m) => detectedTools.has(m.tool))
    : [];
  const authLabel =
    conn.authKind === "key"
      ? t("Key file")
      : conn.authKind === "agent"
        ? t("Agent")
        : t("Password");
  return (
    <div
      className={
        "srv-item" +
        (online ? "" : " offline") +
        (isDragging ? " dragging" : "") +
        dropClass
      }
    >
      <div
        className="srv-row"
        draggable
        onClick={onToggle}
        onContextMenu={onContextMenu}
        onDragStart={onDragStart}
        onDragOver={onDragOverRow}
        onDrop={onDropRow}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onToggle();
          }
        }}
        role="button"
        tabIndex={0}
        aria-expanded={isOpen}
        aria-label={`${conn.display} — ${addr}`}
      >
        <span className="srv-grip" aria-hidden>
          <GripVertical size={10} />
        </span>
        <span className={"srv-dot " + (online ? "on" : "off")} />
        <div className="srv-body">
          <div className="srv-name">{conn.display}</div>
          <div className="srv-addr">{addr}</div>
        </div>
        <span className="srv-auth" title={`${t("Authentication")}: ${authLabel}`}>
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
                      title={t(m.label)}
                    >
                      <Ic size={10} />
                      <span className="svc-name">{t(m.label)}</span>
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
