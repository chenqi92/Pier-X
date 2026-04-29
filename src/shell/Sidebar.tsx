import {
  ArrowLeft,
  ArrowUp,
  ChevronRight,
  Download,
  FileText,
  Folder,
  FolderPlus,
  FolderTree,
  GripVertical,
  HardDrive,
  Home,
  Key,
  Lock,
  Monitor,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Server,
  Shield,
  Star,
  Terminal,
  Trash2,
  Upload,
} from "lucide-react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type {
  DragEvent as ReactDragEvent,
  KeyboardEvent as ReactKeyboardEvent,
  MouseEvent as ReactMouseEvent,
} from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { effectiveSshTarget } from "../lib/types";
import type { CoreInfo, FileEntry, NestedSshTarget, SavedSshConnection, RightTool } from "../lib/types";
import { DRIVES_PATH } from "../lib/browserPath";
import { RIGHT_TOOL_META, SERVICE_CHIP_TOOLS, type LucideIcon } from "../lib/rightToolMeta";
import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useTabStore } from "../stores/useTabStore";
import { useDetectedServicesStore } from "../stores/useDetectedServicesStore";
import ContextMenu, { type ContextMenuItem } from "../components/ContextMenu";
import DismissibleNote from "../components/DismissibleNote";
import {
  DT_LOCAL_FILE,
  DT_SFTP_FILE,
  hasDragPayload,
  type LocalDragPayload,
  readDragPayload,
  type SftpDragPayload,
  writeDragPayload,
} from "../lib/sftpDrag";

type Props = {
  onOpenLocalTerminal: (path?: string) => void;
  onConnectSaved: (index: number) => void;
  onNewConnection: () => void;
  onEditConnection: (index: number) => void;
  onPathChange?: (path: string) => void;
  onFileSelect?: (entry: FileEntry) => void;
  selectedFilePath?: string;
  workspaceRoot?: string;
  /** Open the broadcast dialog with these saved-connection indices
   *  pre-selected (resolved to tab ids by the App layer). */
  onBroadcastToIndices?: (indices: number[]) => void;
};

type ServiceChip = {
  tool: RightTool;
  label: string;
  icon: LucideIcon;
  tintVar: string;
};

type FavoritePlace = {
  label: string;
  path: string;
  shortcut?: string;
};

type FolderVisitRecord = {
  path: string;
  count: number;
  lastVisitedAt: number;
};

const FOLDER_VISITS_STORAGE_KEY = "pierx:folder-visits";
const MAX_FOLDER_VISITS = 32;
const MAX_FREQUENT_FOLDERS = 5;

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

// `DRIVES_PATH` — the "no real directory, we're on the drive picker"
// sentinel — is defined in `lib/browserPath.ts` so other surfaces
// (GitPanel, App.openLocalTerminal) can check it without reaching into
// the Sidebar. Re-imported below alongside the rest of the sidebar's
// path helpers.

function isWindowsPath(path: string): boolean {
  return /^[A-Za-z]:[\\/]?/.test(path) || /^\\\\/.test(path);
}

/** Separator for joining a path's children — Windows keeps backslashes so
 *  `list_directory` receives native-shaped paths and the backend's
 *  `resolve_existing_path` won't silently fall back to cwd. */
function sepFor(path: string): string {
  return isWindowsPath(path) ? "\\" : "/";
}

/** Return the root prefix of a Windows path (e.g. `E:\`, `\\host\share\`)
 *  or empty string for POSIX paths. Roots always end with a separator so
 *  `fs::read_dir` on Windows accepts them. */
function windowsRoot(path: string): string {
  if (/^[A-Za-z]:/.test(path)) return path.slice(0, 2) + "\\";
  const unc = path.match(/^\\\\[^\\]+\\[^\\]+/);
  if (unc) return unc[0] + "\\";
  return "";
}

function pathSegments(path: string, home: string): { name: string; path: string }[] {
  if (!path || path === DRIVES_PATH) return [];
  const segments: { name: string; path: string }[] = [];
  if (home && path.startsWith(home)) {
    segments.push({ name: "~", path: home });
    const sep = sepFor(home);
    const parts = path.slice(home.length).split(/[\\/]+/).filter(Boolean);
    let acc = home.replace(/[\\/]+$/, "");
    for (const part of parts) { acc += sep + part; segments.push({ name: part, path: acc }); }
    return segments;
  }
  if (isWindowsPath(path)) {
    const root = windowsRoot(path);
    // Drive letter ("E:") or UNC head ("\\host\share") as the first crumb,
    // with the click target being the root path *with* trailing backslash.
    segments.push({ name: root.replace(/\\$/, ""), path: root });
    const rest = path.slice(root.length);
    const parts = rest.split(/[\\/]+/).filter(Boolean);
    let acc = root.replace(/\\$/, "");
    for (const part of parts) { acc += "\\" + part; segments.push({ name: part, path: acc }); }
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
  if (currentPath === DRIVES_PATH) return DRIVES_PATH;
  // POSIX root is terminal — no "This PC" above / on non-Windows.
  if (currentPath === "/") return "/";
  const trimmed = currentPath.replace(/[\\/]+$/, "");
  if (!trimmed) return DRIVES_PATH;
  // Windows drive root ("E:") → "This PC" (drives view).
  if (/^[A-Za-z]:$/.test(trimmed)) return DRIVES_PATH;
  const slash = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  if (slash < 0) return currentPath;
  // "E:\foo" → "E:\" (keep drive root with trailing separator).
  if (slash === 2 && /^[A-Za-z]:/.test(trimmed)) return trimmed.slice(0, 2) + "\\";
  if (slash === 0) return "/";
  return trimmed.slice(0, slash);
}

function pathFromHome(homeDir: string, leaf: string): string {
  const base = homeDir.replace(/[\\/]+$/, "");
  return `${base}${sepFor(homeDir)}${leaf}`;
}

function samePath(a: string, b: string): boolean {
  const left = normalizePath(a);
  const right = normalizePath(b);
  if (isWindowsPath(left) || isWindowsPath(right)) return left.toLowerCase() === right.toLowerCase();
  return left === right;
}

function addFavoritePlace(
  places: FavoritePlace[],
  seen: Set<string>,
  place: FavoritePlace,
) {
  const normalized = normalizePath(place.path);
  const key = isWindowsPath(normalized) ? normalized.toLowerCase() : normalized;
  if (!normalized || seen.has(key)) return;
  seen.add(key);
  places.push({ ...place, path: normalized });
}

function buildDefaultFavoritePlaces(
  homeDir: string,
  workspaceRoot: string,
  platform: CoreInfo["platform"] | "",
): FavoritePlace[] {
  const places: FavoritePlace[] = [];
  const seen = new Set<string>();
  if (homeDir) {
    addFavoritePlace(places, seen, { label: "Home", path: homeDir });

    if (platform === "windows") {
      addFavoritePlace(places, seen, { label: "Desktop", path: pathFromHome(homeDir, "Desktop") });
      addFavoritePlace(places, seen, { label: "Documents", path: pathFromHome(homeDir, "Documents") });
      addFavoritePlace(places, seen, { label: "Downloads", path: pathFromHome(homeDir, "Downloads") });
      addFavoritePlace(places, seen, { label: "Pictures", path: pathFromHome(homeDir, "Pictures") });
      addFavoritePlace(places, seen, { label: "Music", path: pathFromHome(homeDir, "Music") });
      addFavoritePlace(places, seen, { label: "Videos", path: pathFromHome(homeDir, "Videos") });
      addFavoritePlace(places, seen, { label: "This PC", path: DRIVES_PATH });
    } else if (platform === "macos") {
      addFavoritePlace(places, seen, { label: "Desktop", path: pathFromHome(homeDir, "Desktop") });
      addFavoritePlace(places, seen, { label: "Documents", path: pathFromHome(homeDir, "Documents") });
      addFavoritePlace(places, seen, { label: "Downloads", path: pathFromHome(homeDir, "Downloads") });
      addFavoritePlace(places, seen, { label: "Applications", path: "/Applications" });
      addFavoritePlace(places, seen, { label: "Pictures", path: pathFromHome(homeDir, "Pictures") });
      addFavoritePlace(places, seen, { label: "Movies", path: pathFromHome(homeDir, "Movies") });
    } else {
      addFavoritePlace(places, seen, { label: "Desktop", path: pathFromHome(homeDir, "Desktop") });
      addFavoritePlace(places, seen, { label: "Documents", path: pathFromHome(homeDir, "Documents") });
      addFavoritePlace(places, seen, { label: "Downloads", path: pathFromHome(homeDir, "Downloads") });
      addFavoritePlace(places, seen, { label: "Projects", path: pathFromHome(homeDir, "Projects") });
    }
  }

  if (workspaceRoot && (!homeDir || !samePath(workspaceRoot, homeDir))) {
    addFavoritePlace(places, seen, { label: "Workspace", path: workspaceRoot });
  }

  return places;
}

function readFolderVisitRecords(): FolderVisitRecord[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(FOLDER_VISITS_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return [];
    return Object.entries(parsed as Record<string, unknown>)
      .map(([path, value]) => {
        if (typeof value === "number") {
          return { path: normalizePath(path), count: value, lastVisitedAt: 0 };
        }
        if (!value || typeof value !== "object") return null;
        const next = value as Partial<FolderVisitRecord>;
        const count = Number(next.count ?? 0);
        const lastVisitedAt = Number(next.lastVisitedAt ?? 0);
        if (!Number.isFinite(count) || count <= 0) return null;
        return { path: normalizePath(path), count, lastVisitedAt };
      })
      .filter((record): record is FolderVisitRecord => !!record && !!record.path)
      .sort((a, b) => b.count - a.count || b.lastVisitedAt - a.lastVisitedAt)
      .slice(0, MAX_FOLDER_VISITS);
  } catch {
    return [];
  }
}

function writeFolderVisitRecords(records: FolderVisitRecord[]) {
  if (typeof window === "undefined") return;
  const payload: Record<string, Omit<FolderVisitRecord, "path">> = {};
  for (const record of records) {
    payload[record.path] = {
      count: record.count,
      lastVisitedAt: record.lastVisitedAt,
    };
  }
  window.localStorage.setItem(FOLDER_VISITS_STORAGE_KEY, JSON.stringify(payload));
}

function bumpFolderVisit(path: string): FolderVisitRecord[] {
  const normalized = normalizePath(path);
  if (!normalized || normalized === DRIVES_PATH) return readFolderVisitRecords();
  const visits = readFolderVisitRecords();
  const existing = visits.find((record) => samePath(record.path, normalized));
  const nextRecord: FolderVisitRecord = {
    path: normalized,
    count: (existing?.count ?? 0) + 1,
    lastVisitedAt: Date.now(),
  };
  const next = [
    nextRecord,
    ...visits.filter((record) => !samePath(record.path, normalized)),
  ]
    .sort((a, b) => b.count - a.count || b.lastVisitedAt - a.lastVisitedAt)
    .slice(0, MAX_FOLDER_VISITS);
  writeFolderVisitRecords(next);
  return next;
}

function shortPathLabel(path: string, homeDir: string): string {
  if (path === DRIVES_PATH) return "This PC";
  const normalized = normalizePath(path);
  if (homeDir && samePath(normalized, homeDir)) return "~";
  const homeBase = homeDir.replace(/[\\/]+$/, "");
  if (homeBase) {
    const comparablePath = isWindowsPath(normalized) ? normalized.toLowerCase() : normalized;
    const comparableHome = isWindowsPath(homeBase) ? homeBase.toLowerCase() : homeBase;
    if (comparablePath.startsWith(`${comparableHome}${sepFor(homeBase)}`)) {
      return `~${sepFor(homeBase)}${normalized.slice(homeBase.length + 1)}`;
    }
  }
  return normalized;
}

export default function Sidebar({ onOpenLocalTerminal, onConnectSaved, onNewConnection, onEditConnection, onPathChange, onFileSelect, selectedFilePath, workspaceRoot, onBroadcastToIndices }: Props) {
  const { t } = useI18n();
  const [section, setSection] = useState<0 | 1>(0);
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [currentPath, setCurrentPath] = useState("");
  const [homeDir, setHomeDir] = useState("");
  const [platform, setPlatform] = useState<CoreInfo["platform"] | "">("");
  const [coreWorkspaceRoot, setCoreWorkspaceRoot] = useState("");
  const [pathHistory, setPathHistory] = useState<string[]>([]);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const [searchText, setSearchText] = useState("");
  const [folderVisits, setFolderVisits] = useState<FolderVisitRecord[]>(readFolderVisitRecords);
  const pendingVisitRef = useRef<string | null>(null);
  const { connections, refresh: refreshConnections, remove } = useConnectionStore();
  const [serverSearch, setServerSearch] = useState("");

  // Transient error notice. Backend commands for connection / group
  // mutations used to silently swallow failures (`.catch(() => {})`),
  // which meant the user saw the UI state revert with no explanation.
  // This surface shows a localized message for a few seconds so the
  // failure is visible without needing devtools.
  const [notice, setNotice] = useState<string | null>(null);
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 4000);
    return () => window.clearTimeout(timer);
  }, [notice]);
  const reportError = (e: unknown) => setNotice(localizeError(e, t));

  useEffect(() => {
    cmd.coreInfo().then((info: CoreInfo) => {
      setHomeDir(info.homeDir);
      setPlatform(info.platform);
      setCoreWorkspaceRoot(info.workspaceRoot);
      const startPath = normalizePath(info.homeDir || info.workspaceRoot);
      if (!currentPath) {
        setCurrentPath(startPath);
        setPathHistory([startPath]);
        setHistoryIndex(0);
      }
    }).catch(reportError);
  }, []);

  useEffect(() => {
    if (!currentPath) return;
    // Cancellation guard: without this, switching from homeDir → This PC
    // while homeDir's listDirectory is still in flight lets the slower
    // homeDir promise resolve *after* listDrives and overwrite the
    // drive list with homeDir entries (observed in practice: breadcrumb
    // says "This PC" but rows are ~/.azure, ~/.bun, …).
    let cancelled = false;
    const loader =
      currentPath === DRIVES_PATH
        ? cmd.listDrives()
        : cmd.listDirectory(currentPath);
    loader
      .then((next) => {
        if (cancelled) return;
        setEntries(next);
        commitPendingVisit(currentPath);
      })
      .catch((e) => {
        if (cancelled) return;
        setEntries([]);
        clearPendingVisit(currentPath);
        reportError(e);
      });
    setSearchText("");
    return () => { cancelled = true; };
  }, [currentPath]);
  useEffect(() => {
    if (!currentPath) return;
    onPathChange?.(currentPath);
  }, [currentPath, onPathChange]);
  useEffect(() => { refreshConnections(); }, []);

  const filteredEntries = entries.filter((e) => !searchText.trim() || e.name.toLowerCase().includes(searchText.toLowerCase()));
  const segments = pathSegments(currentPath, homeDir);
  const effectiveWorkspaceRoot = workspaceRoot || coreWorkspaceRoot;
  const defaultFavoritePlaces = useMemo(
    () => buildDefaultFavoritePlaces(homeDir, effectiveWorkspaceRoot, platform),
    [homeDir, effectiveWorkspaceRoot, platform],
  );
  const frequentFavoritePlaces = useMemo(() => {
    const defaultPaths = new Set(
      defaultFavoritePlaces.map((place) => {
        const normalized = normalizePath(place.path);
        return isWindowsPath(normalized) ? normalized.toLowerCase() : normalized;
      }),
    );
    return folderVisits
      .filter((record) => {
        const key = isWindowsPath(record.path) ? record.path.toLowerCase() : record.path;
        return !defaultPaths.has(key) && record.path !== DRIVES_PATH;
      })
      .slice(0, MAX_FREQUENT_FOLDERS);
  }, [defaultFavoritePlaces, folderVisits]);

  // ── Sidebar ↔ SFTP drag-drop ───────────────────────────────────
  //
  // The local file list is both a drag *source* (drop into SFTP
  // uploads the file) and a drag *target* (drop a remote file from
  // SFTP downloads into the current local directory). The drop
  // resolves the source tab's effective SSH target, which keeps
  // primary and nested-SSH sessions on the same path.
  const tabs = useTabStore((s) => s.tabs);
  const activeTabId = useTabStore((s) => s.activeTabId);
  const [sftpDropDepth, setSftpDropDepth] = useState(0);
  const sftpDropActive = sftpDropDepth > 0;

  function sshTargetMatchesPayload(target: NestedSshTarget | null, payload: SftpDragPayload) {
    return (
      target !== null &&
      target.host === payload.host &&
      target.port === payload.port &&
      target.user === payload.user &&
      target.authMode === payload.authMode
    );
  }

  function resolveSshTargetForPayload(payload: SftpDragPayload): NestedSshTarget | null {
    if (payload.sourceTabId) {
      const sourceTab = tabs.find((tab) => tab.id === payload.sourceTabId);
      const sourceTarget = sourceTab ? effectiveSshTarget(sourceTab) : null;
      if (sshTargetMatchesPayload(sourceTarget, payload)) return sourceTarget;
    }

    // Prefer the active tab if it matches, so the download uses the
    // same cached session that just populated the SFTP panel.
    const active = tabs.find((tab) => tab.id === activeTabId);
    const activeTarget = active ? effectiveSshTarget(active) : null;
    if (sshTargetMatchesPayload(activeTarget, payload)) return activeTarget;

    for (const tab of tabs) {
      const target = effectiveSshTarget(tab);
      if (sshTargetMatchesPayload(target, payload)) return target;
    }

    return null;
  }

  async function handleSftpDropDownload(payload: SftpDragPayload) {
    const sshTarget = resolveSshTargetForPayload(payload);
    if (!sshTarget) {
      reportError(new Error(t("SSH connection required.")));
      return;
    }
    if (!currentPath || currentPath === DRIVES_PATH) {
      reportError(new Error(t("Choose a local folder before dropping SFTP files.")));
      return;
    }
    const localPath = localJoin(currentPath, payload.name);
    try {
      if (payload.isDir) {
        await cmd.sftpDownloadTree({
          host: sshTarget.host,
          port: sshTarget.port,
          user: sshTarget.user,
          authMode: sshTarget.authMode,
          password: sshTarget.password,
          keyPath: sshTarget.keyPath,
          savedConnectionIndex: sshTarget.savedConnectionIndex,
          remotePath: payload.path,
          localPath,
        });
      } else {
        await cmd.sftpDownload({
          host: sshTarget.host,
          port: sshTarget.port,
          user: sshTarget.user,
          authMode: sshTarget.authMode,
          password: sshTarget.password,
          keyPath: sshTarget.keyPath,
          savedConnectionIndex: sshTarget.savedConnectionIndex,
          remotePath: payload.path,
          localPath,
        });
      }
      // Refresh the file list so the newly-downloaded file shows up.
      cmd.listDirectory(currentPath).then(setEntries).catch(reportError);
    } catch (e) {
      reportError(e);
    }
  }

  function handleFileListDragEnter(event: ReactDragEvent<HTMLDivElement>) {
    if (!hasDragPayload(event.dataTransfer, DT_SFTP_FILE)) return;
    event.preventDefault();
    setSftpDropDepth((d) => d + 1);
  }
  function handleFileListDragOver(event: ReactDragEvent<HTMLDivElement>) {
    if (!hasDragPayload(event.dataTransfer, DT_SFTP_FILE)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  }
  function handleFileListDragLeave(event: ReactDragEvent<HTMLDivElement>) {
    if (!hasDragPayload(event.dataTransfer, DT_SFTP_FILE)) return;
    event.preventDefault();
    setSftpDropDepth((d) => Math.max(0, d - 1));
  }
  function handleFileListDrop(event: ReactDragEvent<HTMLDivElement>) {
    setSftpDropDepth(0);
    const payload = readDragPayload(event.dataTransfer, DT_SFTP_FILE, "sftp-file");
    if (!payload) return;
    event.preventDefault();
    void handleSftpDropDownload(payload);
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
    writeDragPayload(event.dataTransfer, DT_LOCAL_FILE, "local-file", payload);
  }

  // ── Local file context menu + mutations ────────────────────────
  // Mirror the SFTP panel's right-click surface on the local
  // sidebar: open / reveal / rename / delete / copy path + empty-
  // area new-file / new-folder / refresh. Kept inline in this
  // component because the state (entries, currentPath) is already
  // local here — lifting to a store would add churn without a
  // second consumer.
  type LocalCtxState =
    | { kind: "entry"; x: number; y: number; entry: FileEntry }
    | { kind: "empty"; x: number; y: number }
    | null;
  const [localCtxMenu, setLocalCtxMenu] = useState<LocalCtxState>(null);
  const [newLocalName, setNewLocalName] = useState("");
  const [newLocalKind, setNewLocalKind] = useState<"file" | "dir" | null>(null);

  // Quick-access folder dropdown — opens beneath the toolbar star
  // button. We anchor the ContextMenu to the button's bounding rect
  // instead of a mouse position so the menu lands in a predictable
  // place regardless of how the user triggered it.
  const favBtnRef = useRef<HTMLButtonElement>(null);
  const [favMenuPos, setFavMenuPos] = useState<{ x: number; y: number } | null>(null);

  function buildFavoriteItems(): ContextMenuItem[] {
    const items: ContextMenuItem[] = [];
    if (defaultFavoritePlaces.length > 0) {
      items.push({ section: t("Recommended folders") });
      for (const place of defaultFavoritePlaces) {
        items.push({
          label: t(place.label),
          shortcut: place.shortcut ?? (place.path === DRIVES_PATH ? undefined : shortPathLabel(place.path, homeDir)),
          action: () => pushPath(place.path),
        });
      }
      items.push({ divider: true });
    }
    if (frequentFavoritePlaces.length > 0) {
      items.push({ section: t("Frequent folders") });
      for (const record of frequentFavoritePlaces) {
        items.push({
          label: shortPathLabel(record.path, homeDir),
          shortcut: `${record.count}x`,
          action: () => pushPath(record.path),
        });
      }
      items.push({ divider: true });
    }
    items.push({
      label: t("Choose folder…"),
      action: () => {
        void (async () => {
          try {
            const picked = await openDialog({
              directory: true,
              multiple: false,
              title: t("Choose folder"),
            });
            if (typeof picked === "string" && picked) pushPath(picked);
          } catch (e) {
            reportError(e);
          }
        })();
      },
    });
    return items;
  }

  function markPendingVisit(path: string) {
    pendingVisitRef.current = normalizePath(path);
  }

  function clearPendingVisit(path: string) {
    if (pendingVisitRef.current === normalizePath(path)) pendingVisitRef.current = null;
  }

  function commitPendingVisit(path: string) {
    const normalized = normalizePath(path);
    if (pendingVisitRef.current !== normalized) return;
    pendingVisitRef.current = null;
    setFolderVisits(bumpFolderVisit(normalized));
  }

  function toggleFavMenu() {
    if (favMenuPos) {
      setFavMenuPos(null);
      return;
    }
    const rect = favBtnRef.current?.getBoundingClientRect();
    if (!rect) return;
    setFavMenuPos({ x: rect.left, y: rect.bottom + 4 });
  }

  function refreshLocalEntries() {
    if (!currentPath) return;
    const loader =
      currentPath === DRIVES_PATH
        ? cmd.listDrives()
        : cmd.listDirectory(currentPath);
    loader.then(setEntries).catch(reportError);
  }

  function localJoin(dir: string, leaf: string): string {
    const trimmed = dir.replace(/[\\/]+$/, "");
    const sep = /^[A-Za-z]:($|[\\/])|^\\\\/.test(trimmed) ? "\\" : "/";
    return `${trimmed}${sep}${leaf}`;
  }

  async function openLocalEntry(entry: FileEntry) {
    // Opener plugin handles files and directories differently on
    // each OS; for directories we prefer our own pushPath so the
    // sidebar follows the user in-place.
    if (entry.kind === "directory") {
      pushPath(entry.path);
      return;
    }
    try {
      const { openPath } = await import("@tauri-apps/plugin-opener");
      await openPath(entry.path);
    } catch (e) {
      console.warn("openPath failed", localizeError(e, t));
    }
  }

  async function revealLocalEntry(entry: FileEntry) {
    try {
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
      await revealItemInDir(entry.path);
    } catch (e) {
      console.warn("revealItemInDir failed", localizeError(e, t));
    }
  }

  async function renameLocalEntry(entry: FileEntry) {
    const next = window.prompt(
      t("Rename {name} to:", { name: entry.name }),
      entry.name,
    );
    if (!next || next.trim() === "" || next.trim() === entry.name) return;
    const trimmed = next.trim();
    const parent = entry.path.replace(/[\\/][^\\/]*$/, "");
    const to = localJoin(parent || currentPath, trimmed);
    try {
      await cmd.localRename(entry.path, to);
      refreshLocalEntries();
    } catch (e) {
      window.alert(localizeError(e, t));
    }
  }

  async function removeLocalEntry(entry: FileEntry) {
    const msg = entry.kind === "directory"
      ? t("Remove directory {name}? Contents will be deleted.", { name: entry.name })
      : t("Remove file {name}?", { name: entry.name });
    if (!window.confirm(msg)) return;
    try {
      await cmd.localRemove(entry.path, entry.kind === "directory");
      refreshLocalEntries();
    } catch (e) {
      window.alert(localizeError(e, t));
    }
  }

  async function copyLocalPath(entry: FileEntry) {
    const { writeClipboardText } = await import("../lib/clipboard");
    await writeClipboardText(entry.path);
  }

  async function commitNewLocal() {
    const leaf = newLocalName.trim();
    if (!leaf || !newLocalKind) return;
    const target = localJoin(currentPath, leaf);
    try {
      if (newLocalKind === "file") {
        await cmd.localCreateFile(target);
      } else {
        await cmd.localCreateDir(target);
      }
      setNewLocalKind(null);
      setNewLocalName("");
      refreshLocalEntries();
    } catch (e) {
      window.alert(localizeError(e, t));
    }
  }

  function buildLocalEntryMenu(entry: FileEntry): ContextMenuItem[] {
    const isDir = entry.kind === "directory";
    const items: ContextMenuItem[] = [];
    items.push({
      label: isDir ? t("Open") : t("Open externally"),
      action: () => void openLocalEntry(entry),
    });
    if (!isDir) {
      items.push({
        label: t("Reveal in file manager"),
        action: () => void revealLocalEntry(entry),
      });
    }
    items.push({ divider: true });
    items.push({ label: t("Rename…"), action: () => void renameLocalEntry(entry) });
    items.push({ label: t("Delete"), action: () => void removeLocalEntry(entry) });
    items.push({ divider: true });
    items.push({ label: t("Copy path"), action: () => void copyLocalPath(entry) });
    return items;
  }

  function buildLocalEmptyMenu(): ContextMenuItem[] {
    const canMutate = !!currentPath && currentPath !== DRIVES_PATH;
    return [
      {
        label: t("New file…"),
        action: () => { setNewLocalKind("file"); setNewLocalName(""); },
        disabled: !canMutate,
      },
      {
        label: t("New folder…"),
        action: () => { setNewLocalKind("dir"); setNewLocalName(""); },
        disabled: !canMutate,
      },
      { divider: true },
      { label: t("Refresh"), action: refreshLocalEntries },
    ];
  }

  function handleLocalRowContextMenu(
    event: ReactMouseEvent<HTMLDivElement>,
    entry: FileEntry,
  ) {
    event.preventDefault();
    event.stopPropagation();
    setLocalCtxMenu({ kind: "entry", x: event.clientX, y: event.clientY, entry });
  }

  function handleLocalListContextMenu(event: ReactMouseEvent<HTMLDivElement>) {
    const target = event.target as HTMLElement | null;
    if (target?.closest(".file-row")) return;
    event.preventDefault();
    setLocalCtxMenu({ kind: "empty", x: event.clientX, y: event.clientY });
  }

  function pushPath(nextPath: string) {
    const normalized = normalizePath(nextPath);
    if (!normalized || normalized === currentPath) return;
    const nextHistory = pathHistory.slice(0, historyIndex + 1);
    nextHistory.push(normalized);
    markPendingVisit(normalized);
    setPathHistory(nextHistory);
    setHistoryIndex(nextHistory.length - 1);
    setCurrentPath(normalized);
  }

  function goBackPath() {
    if (historyIndex <= 0) return;
    const nextIndex = historyIndex - 1;
    const nextPath = pathHistory[nextIndex];
    if (!nextPath) return;
    markPendingVisit(nextPath);
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

      {notice && (
        <DismissibleNote variant="status" tone="error" onDismiss={() => setNotice(null)}>
          {notice}
        </DismissibleNote>
      )}

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
              disabled={!currentPath || currentPath === "/" || currentPath === DRIVES_PATH}
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
            {platform === "windows" && (
              <button
                className={"mini-btn" + (currentPath === DRIVES_PATH ? " is-active" : "")}
                onClick={() => pushPath(DRIVES_PATH)}
                title={t("This PC")}
                type="button"
                aria-pressed={currentPath === DRIVES_PATH}
              >
                <Monitor />
              </button>
            )}
            <div className="crumb">
              {currentPath === DRIVES_PATH ? (
                <span className="crumb-item">
                  <span className="seg last">{t("This PC")}</span>
                </span>
              ) : (
                segments.map((seg, i) => (
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
                ))
              )}
            </div>
            <button
              ref={favBtnRef}
              className={"mini-btn" + (favMenuPos ? " is-active" : "")}
              onClick={toggleFavMenu}
              onMouseDown={(e) => {
                // ContextMenu closes on any document mousedown that
                // lands outside its ref — including the trigger
                // button. Swallow the mousedown so re-clicking the
                // button toggles cleanly instead of close-then-reopen.
                e.stopPropagation();
              }}
              title={t("Common folders")}
              type="button"
              aria-expanded={favMenuPos !== null}
            >
              <Star />
            </button>
            <button
              className="mini-btn"
              onClick={() => {
                const loader =
                  currentPath === DRIVES_PATH
                    ? cmd.listDrives()
                    : cmd.listDirectory(currentPath);
                loader.then(setEntries).catch(reportError);
              }}
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

          <div className="sidebar-header-row">
            <span className="col-icon" aria-hidden />
            <span className="col-name">{t("NAME")}</span>
            <span className="col-mod">{t("MOD")}</span>
            <span className="col-size">{t("SIZE")}</span>
          </div>

          <div
            className={"sidebar-list" + (sftpDropActive ? " is-drop" : "")}
            onDragEnter={handleFileListDragEnter}
            onDragOver={handleFileListDragOver}
            onDragLeave={handleFileListDragLeave}
            onDrop={handleFileListDrop}
            onContextMenu={handleLocalListContextMenu}
          >
            {newLocalKind && currentPath !== DRIVES_PATH && (
              <div className="sidebar-quickrow">
                <span className="sidebar-quickrow-label">
                  {newLocalKind === "file" ? t("New file") : t("New folder")}
                </span>
                <input
                  className="sidebar-quickrow-input"
                  value={newLocalName}
                  autoFocus
                  placeholder={newLocalKind === "file" ? t("notes.md") : t("logs")}
                  onChange={(e) => setNewLocalName(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void commitNewLocal();
                    if (e.key === "Escape") { setNewLocalKind(null); setNewLocalName(""); }
                  }}
                />
                <button
                  type="button"
                  className="mini-btn"
                  title={t("Create")}
                  disabled={!newLocalName.trim()}
                  onClick={() => void commitNewLocal()}
                >
                  <Plus />
                </button>
                <button
                  type="button"
                  className="mini-btn"
                  title={t("Cancel")}
                  onClick={() => { setNewLocalKind(null); setNewLocalName(""); }}
                >
                  <Trash2 />
                </button>
              </div>
            )}
            {filteredEntries.map((entry) => {
              const isSelected = entry.kind === "file" && selectedFilePath === entry.path;
              const isDir = entry.kind === "directory";
              const isDrive = currentPath === DRIVES_PATH;
              const isMd = entry.name.toLowerCase().endsWith(".md");
              const cls =
                "file-row" +
                (isDir ? " is-dir" : "") +
                (isMd ? " is-md" : "") +
                (isSelected ? " selected" : "");
              const icon = isDrive
                ? <HardDrive size={12} />
                : isDir
                  ? <Folder size={12} />
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
                  onContextMenu={isDrive ? undefined : (e) => handleLocalRowContextMenu(e, entry)}
                  role="button"
                  tabIndex={0}
                  draggable={!isDrive}
                  onDragStart={(e) => {
                    if (isDrive) {
                      e.preventDefault();
                      return;
                    }
                    handleLocalRowDragStart(e, entry);
                  }}
                >
                  <span className="fi">{icon}</span>
                  <span className="fname">{entry.name}</span>
                  <span className="fmod">{entry.modified}</span>
                  <span className="fsize">{entry.sizeLabel}</span>
                </div>
              );
            })}
            {filteredEntries.length === 0 && (
              <div className="empty-note" style={{ padding: "var(--sp-3)" }}>
                {searchText ? t("No matching files") : t("Empty directory")}
              </div>
            )}
          </div>
          {localCtxMenu && (
            <ContextMenu
              x={localCtxMenu.x}
              y={localCtxMenu.y}
              items={
                localCtxMenu.kind === "entry"
                  ? buildLocalEntryMenu(localCtxMenu.entry)
                  : buildLocalEmptyMenu()
              }
              onClose={() => setLocalCtxMenu(null)}
            />
          )}
          {favMenuPos && (
            <ContextMenu
              x={favMenuPos.x}
              y={favMenuPos.y}
              items={buildFavoriteItems()}
              onClose={() => setFavMenuPos(null)}
            />
          )}
        </>
      ) : (
        <ServersPane
          connections={connections}
          serverSearch={serverSearch}
          onSearchChange={setServerSearch}
          onConnect={onConnectSaved}
          onEdit={onEditConnection}
          onRemove={(index) => { void remove(index).catch(reportError); }}
          onNew={onNewConnection}
          onRefresh={() => { void refreshConnections(); }}
          onBroadcastToIndices={onBroadcastToIndices}
          onExport={async () => {
            try {
              const picked = await saveDialog({
                title: t("Export SSH connections"),
                defaultPath: "pier-x-ssh-connections.json",
                filters: [{ name: "JSON", extensions: ["json"] }],
              });
              if (typeof picked !== "string") return;
              // sshConnectionsList already strips passwords / keychain
              // ids on the way to the frontend; what we get here is
              // safe to share verbatim. envTag / group / databases (a
              // metadata-only projection) all flow through.
              const blob = JSON.stringify(
                {
                  _meta: {
                    exportedAt: new Date().toISOString(),
                    note:
                      "Passwords are NOT exported. Re-enter on import for password-auth entries.",
                  },
                  connections,
                },
                null,
                2,
              );
              await cmd.localWriteTextFile(picked, blob);
              setNotice(
                t("Exported {n} SSH connection(s).", { n: connections.length }),
              );
            } catch (e) {
              setNotice(e instanceof Error ? e.message : String(e));
            }
          }}
          onImport={async () => {
            try {
              const picked = await openDialog({
                title: t("Import SSH connections"),
                multiple: false,
                filters: [{ name: "JSON", extensions: ["json"] }],
              });
              if (!picked || typeof picked !== "string") return;
              const raw = await cmd.localReadTextFile(picked);
              const parsed = JSON.parse(raw);
              const incoming = Array.isArray(parsed?.connections)
                ? parsed.connections
                : Array.isArray(parsed)
                  ? parsed
                  : null;
              if (!incoming) {
                setNotice(t("File doesn't look like an SSH connection list."));
                return;
              }
              // De-dup by (host, port, user) so re-importing the same
              // file doesn't double up the sidebar.
              const have = new Set(
                connections.map((c) => `${c.user}@${c.host}:${c.port}`),
              );
              let added = 0;
              let skipped = 0;
              for (const c of incoming as SavedSshConnection[]) {
                const key = `${c.user}@${c.host}:${c.port}`;
                if (have.has(key)) {
                  skipped += 1;
                  continue;
                }
                try {
                  await useConnectionStore.getState().save({
                    name: c.name,
                    host: c.host,
                    port: c.port,
                    user: c.user,
                    authKind: c.authKind,
                    // Imported entries lose their password: keychain
                    // ids are local-only and DirectPassword fallbacks
                    // were never exported. Password-auth connections
                    // will re-prompt at first use.
                    password: "",
                    keyPath: c.keyPath ?? "",
                    group: c.group ?? null,
                    envTag: c.envTag ?? null,
                  });
                  have.add(key);
                  added += 1;
                } catch {
                  skipped += 1;
                }
              }
              setNotice(
                t("Imported {added} new ({skipped} skipped).", {
                  added,
                  skipped,
                }),
              );
            } catch (e) {
              setNotice(e instanceof Error ? e.message : String(e));
            }
          }}
          onReorder={async (order, groups) => {
            try {
              await useConnectionStore.getState().reorder(order, groups);
            } catch (e) {
              reportError(e);
            }
          }}
          onRenameGroup={async (from, to) => {
            try {
              await useConnectionStore.getState().renameGroup(from, to);
            } catch (e) {
              reportError(e);
            }
          }}
        />
      )}
    </aside>
  );
}

function normalizePath(path: string): string {
  const value = String(path || "").trim();
  if (!value) return "/";
  if (value === DRIVES_PATH) return DRIVES_PATH;
  // Windows drive root must keep its trailing separator — "E:" alone
  // is interpreted as the current directory of drive E, not its root.
  if (/^[A-Za-z]:[\\/]?$/.test(value)) return value.slice(0, 2) + "\\";
  // UNC root "\\host\share" has no trailing separator; strip any extras.
  if (/^\\\\[^\\]+\\[^\\]+[\\/]?$/.test(value)) return value.replace(/[\\/]+$/, "");
  const stripped = value.replace(/[\\/]+$/, "");
  return stripped || "/";
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
  onExport,
  onImport,
  onBroadcastToIndices,
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
  onExport: () => void;
  onImport: () => void;
  onBroadcastToIndices?: (indices: number[]) => void;
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
      // Connect-all: spawn one SSH tab per host in this group. Useful
      // for "open the whole prod fleet at once" workflows. We grab the
      // group's row indices off the latest groups snapshot rather than
      // scanning `connections` ourselves so the menu reflects the
      // displayed (possibly filtered) view.
      const grp = groups.find((g) => g.key === key);
      const rowCount = grp?.servers.length ?? 0;
      items.push({
        label: t("Open all ({n})", { n: rowCount }),
        disabled: rowCount === 0,
        action: () => {
          if (!grp) return;
          for (const row of grp.servers) onConnect(row.index);
        },
      });
      items.push({
        label: t("Probe all ({n})", { n: rowCount }),
        disabled: rowCount === 0,
        action: () => {
          if (!grp) return;
          // Run a single batched probe over the group's indices.
          // Failures are silent here — the Hosts dashboard / health
          // pulse on the sidebar pick up the freshly-stored result
          // on next render without a toast in the user's face.
          const indices = grp.servers.map((s) => s.index);
          void cmd
            .hostHealthProbe({ indices, timeoutMs: 3000 })
            .catch(() => {});
        },
      });
      if (onBroadcastToIndices) {
        items.push({
          label: t("Broadcast to group ({n})", { n: rowCount }),
          disabled: rowCount === 0,
          action: () => {
            if (!grp) return;
            onBroadcastToIndices(grp.servers.map((s) => s.index));
          },
        });
      }
      items.push({ divider: true });
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
          <span className="sep" style={{ marginLeft: "var(--sp-1-5)" }}>·</span>
          <span className="crumb-item">
            <span className="seg mono" style={{ fontSize: "var(--size-micro)" }}>{totalCount}</span>
          </span>
        </div>
        <button
          className="mini-btn"
          onClick={onImport}
          title={t("Import SSH connections from a JSON file")}
          type="button"
        >
          <Upload />
        </button>
        <button
          className="mini-btn"
          onClick={onExport}
          title={t("Export SSH connections to a JSON file (passwords stripped)")}
          type="button"
          disabled={connections.length === 0}
        >
          <Download />
        </button>
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
          <div className="empty-note" style={{ padding: "var(--sp-3)" }}>
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
