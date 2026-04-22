import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Check,
  Download,
  Edit,
  File as FileIcon,
  FileText,
  Folder,
  HardDrive,
  Home,
  Plus,
  RefreshCw,
  Server,
  Terminal as TerminalIcon,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import type { DragEvent as ReactDragEvent } from "react";
import type { ComponentType } from "react";
import * as cmd from "../lib/commands";
import { SFTP_PROGRESS_EVENT, type SftpProgressEvent } from "../lib/commands";
import { RIGHT_TOOL_META } from "../lib/rightToolMeta";
import type { SftpBrowseState, SftpEntryView, TabState } from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";
import VirtualList from "../components/VirtualList";

/** Row height for virtualized entries, matching `.ftp-row` in shell.css
 *  (12px font · 6px padding top+bottom · 1px border). Kept in sync
 *  manually — if that CSS changes, bump this. Mismatches show up as
 *  rows overlapping or whitespace gaps during scroll. */
const FTP_ROW_HEIGHT = 26;

/** A single row in the virtualized list, discriminated so the ".." parent
 *  pseudo-row and real entries share one renderer. */
type FtpListRow =
  | { kind: "parent" }
  | { kind: "entry"; entry: SftpEntryView };

/**
 * First-load placeholder that mimics `.ftp-row` layout so the transition
 * to the real virtualized list doesn't shift anything. Bar widths are
 * staggered so the stack doesn't read as identical rows — mirrors the
 * `DkSkeleton` pattern in the Docker panel.
 */
function FtpSkeleton({ rows = 8 }: { rows?: number }) {
  return (
    <div className="ftp-skeleton" aria-hidden>
      {Array.from({ length: rows }, (_, i) => {
        const nameWidth = 55 + ((i * 13) % 40); // 55..94%
        return (
          <div key={i} className="ftp-skeleton-row" style={{ height: FTP_ROW_HEIGHT }}>
            <span className="ftp-sk-bar ftp-sk-ic" />
            <span className="ftp-sk-bar ftp-sk-name" style={{ width: `${nameWidth}%` }} />
            <span className="ftp-sk-bar ftp-sk-perm" />
            <span className="ftp-sk-bar ftp-sk-size" />
            <span className="ftp-sk-bar ftp-sk-mod" />
          </div>
        );
      })}
    </div>
  );
}

/** DataTransfer MIME types for cross-panel drag-drop. The Sidebar
 *  file list writes `DT_LOCAL_FILE` when the user drags a local file,
 *  and the SFTP panel writes `DT_SFTP_FILE` when dragging a remote
 *  entry. Keep these constants in sync with src/shell/Sidebar.tsx. */
export const DT_LOCAL_FILE = "application/x-pier-localfile";
export const DT_SFTP_FILE = "application/x-pier-sftpfile";

export type LocalDragPayload = { path: string; name: string; isDir?: boolean };
export type SftpDragPayload = {
  path: string;
  name: string;
  isDir: boolean;
  size: number;
  /** SSH addressing — lets the sidebar's drop handler rebuild the
   *  cached session identity without fishing in the tab store. */
  host: string;
  port: number;
  user: string;
  authMode: string;
};

type Props = { tab: TabState };

type TransferDirection = "up" | "dn";
type TransferStatus = "active" | "done" | "failed";
type TransferItem = {
  id: string;
  direction: TransferDirection;
  name: string;
  remotePath: string;
  localPath: string;
  status: TransferStatus;
  startedAt: number;
  finishedAt?: number;
  error?: string;
  /** Latest bytes transferred, updated live from `sftp:progress`
   *  events. Zero until the first chunk arrives. */
  bytes?: number;
  /** Total file size in bytes, set on the first progress event. */
  total?: number;
};

function joinRemotePath(basePath: string, leaf: string) {
  const cleanLeaf = leaf.trim().replace(/^\/+/, "");
  if (!cleanLeaf) return basePath;
  const normalizedBase = basePath === "/" ? "/" : basePath.replace(/\/+$/, "");
  return normalizedBase === "/" ? `/${cleanLeaf}` : `${normalizedBase}/${cleanLeaf}`;
}

function remoteDirname(path: string) {
  const normalized = String(path || "").replace(/\/+$/, "");
  if (!normalized || normalized === "/") return "/";
  const index = normalized.lastIndexOf("/");
  if (index <= 0) return "/";
  return normalized.slice(0, index);
}

function localBaseName(path: string) {
  const normalized = String(path || "").replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/);
  return parts[parts.length - 1] || "";
}

function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let val = n;
  let u = 0;
  while (val >= 1024 && u < units.length - 1) {
    val /= 1024;
    u++;
  }
  return `${val < 10 && u > 0 ? val.toFixed(1) : Math.round(val)} ${units[u]}`;
}

function iconForEntry(entry: SftpEntryView): ComponentType<{ size?: number }> {
  if (entry.isDir) return Folder;
  if (/\.(sh|js|ts|py|go|rb|rs|mjs)$/i.test(entry.name)) return TerminalIcon;
  if (/\.(md|log|txt|yml|yaml|toml|json|conf|ini)$/i.test(entry.name)) return FileText;
  if (/\.(tar|gz|zip|7z|xz|bz2|tgz|deb|rpm)$/i.test(entry.name)) return HardDrive;
  return FileIcon;
}

/** Render a Unix-seconds timestamp as a concrete local-time string.
 *  Recent entries (this year) show `MM-DD HH:mm`; older entries roll
 *  over to `YYYY-MM-DD`. Em-dash fallback if the server didn't report
 *  a modified time. */
function formatModifiedTime(unixSeconds: number | null | undefined): string {
  if (!unixSeconds || !Number.isFinite(unixSeconds)) return "—";
  const d = new Date(unixSeconds * 1000);
  const now = new Date();
  const pad = (n: number) => (n < 10 ? `0${n}` : `${n}`);
  const sameYear = d.getFullYear() === now.getFullYear();
  if (sameYear) {
    return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  }
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Long date/time used in tooltips. */
function formatModifiedTooltip(unixSeconds: number | null | undefined): string {
  if (!unixSeconds || !Number.isFinite(unixSeconds)) return "";
  return new Date(unixSeconds * 1000).toLocaleString();
}

export default function SftpPanel({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const [state, setState] = useState<SftpBrowseState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [path, setPath] = useState("/");
  const [selectedPath, setSelectedPath] = useState("");
  const [editingPath, setEditingPath] = useState(false);
  const [pathDraft, setPathDraft] = useState("/");
  const [history, setHistory] = useState<string[]>([]);
  const [forward, setForward] = useState<string[]>([]);

  const [renameOpen, setRenameOpen] = useState(false);
  const [renameTarget, setRenameTarget] = useState("");
  const [mkdirOpen, setMkdirOpen] = useState(false);
  const [mkdirName, setMkdirName] = useState("");
  const [actionBusy, setActionBusy] = useState(false);

  const [transfers, setTransfers] = useState<TransferItem[]>([]);
  const transferSeq = useRef(0);
  const [dropDepth, setDropDepth] = useState(0);
  const dropHover = dropDepth > 0;

  // SSH context can come from the tab being a real SSH tab, from a
  // local terminal where the user typed `ssh user@host`, or from a
  // nested-ssh overlay set on an SSH tab. `effectiveSshTarget`
  // collapses all three so this panel works in any of those modes.
  const sshTarget = effectiveSshTarget(tab);
  const hasSsh = sshTarget !== null;
  // Spread-friendly version of the SSH addressing for command calls.
  // Falls back to inert defaults when there's no target — every
  // call site is gated behind `hasSsh` / `sshTarget` first, so the
  // empty values never reach the backend.
  const sshArgs = {
    host: sshTarget?.host ?? "",
    port: sshTarget?.port ?? 22,
    user: sshTarget?.user ?? "",
    authMode: sshTarget?.authMode ?? "password",
    password: sshTarget?.password ?? "",
    keyPath: sshTarget?.keyPath ?? "",
    savedConnectionIndex: sshTarget?.savedConnectionIndex ?? null,
  };
  const sshRequired = t("SSH connection required.");
  const selectedEntry = useMemo(
    () => state?.entries.find((entry) => entry.path === selectedPath) ?? null,
    [state, selectedPath],
  );

  const currentRemotePath = state?.currentPath || path || "/";

  const crumbSegments = useMemo(() => {
    const segs = currentRemotePath.split("/").filter(Boolean);
    return ["/", ...segs];
  }, [currentRemotePath]);

  const activeTransfers = transfers.filter((t) => t.status === "active").length;
  const doneTransfers = transfers.filter((t) => t.status === "done").length;

  function pushTransfer(item: Omit<TransferItem, "id" | "startedAt" | "status">): string {
    // Namespace with tab id so progress events from concurrent tabs
    // can't cross-contaminate each other's transfer queues.
    const id = `xfer-${tab.id}-${++transferSeq.current}`;
    const entry: TransferItem = { ...item, id, status: "active", startedAt: Date.now() };
    setTransfers((prev) => [entry, ...prev].slice(0, 20));
    return id;
  }

  function finishTransfer(id: string, status: TransferStatus, errorMsg?: string) {
    setTransfers((prev) =>
      prev.map((t) =>
        t.id === id ? { ...t, status, finishedAt: Date.now(), error: errorMsg } : t,
      ),
    );
  }

  function clearFinishedTransfers() {
    setTransfers((prev) => prev.filter((t) => t.status === "active"));
  }

  // Subscribe to byte-level progress events from the backend. Each
  // upload/download command emits `sftp:progress` with its transfer
  // id on every 64 KiB chunk plus a final `done: true` emit. We
  // update only entries whose ids belong to this tab (the id is
  // prefixed with `xfer-${tab.id}-`) so multiple SFTP panels don't
  // clobber each other's queues.
  useEffect(() => {
    const tabIdPrefix = `xfer-${tab.id}-`;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<SftpProgressEvent>(SFTP_PROGRESS_EVENT, (event) => {
      const payload = event.payload;
      if (!payload?.id || !payload.id.startsWith(tabIdPrefix)) return;
      setTransfers((prev) =>
        prev.map((t) =>
          t.id === payload.id
            ? {
                ...t,
                bytes: payload.bytes,
                total: payload.total,
              }
            : t,
        ),
      );
    }).then((dispose) => {
      if (disposed) {
        dispose();
      } else {
        unlisten = dispose;
      }
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [tab.id]);

  async function browse(targetPath = path, opts: { pushHistory?: boolean } = {}) {
    if (!hasSsh) {
      setError(sshRequired);
      return;
    }
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const next = await cmd.sftpBrowse({
        ...sshArgs,
        path: targetPath,
      });
      if (opts.pushHistory && state?.currentPath && state.currentPath !== next.currentPath) {
        setHistory((h) => [...h, state.currentPath]);
        setForward([]);
      }
      setState(next);
      setPath(next.currentPath);
      setPathDraft(next.currentPath);
      setSelectedPath("");
      setRenameOpen(false);
    } catch (e) {
      setState(null);
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  async function goBack() {
    if (!history.length || !state) return;
    const prev = history[history.length - 1];
    setHistory((h) => h.slice(0, -1));
    setForward((f) => [...f, state.currentPath]);
    await browse(prev);
  }

  async function goForward() {
    if (!forward.length || !state) return;
    const next = forward[forward.length - 1];
    setForward((f) => f.slice(0, -1));
    setHistory((h) => [...h, state.currentPath]);
    await browse(next);
  }

  async function createDirectory() {
    if (!hasSsh || !mkdirName.trim()) return;
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      const targetPath = joinRemotePath(currentRemotePath, mkdirName);
      await cmd.sftpMkdir({
        ...sshArgs,
        path: targetPath,
      });
      setMkdirName("");
      setMkdirOpen(false);
      setNotice(t("Created directory {path}.", { path: targetPath }));
      await browse(currentRemotePath);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function renameSelected() {
    if (!hasSsh || !selectedEntry || !renameTarget.trim()) return;
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      const nextPath = joinRemotePath(remoteDirname(selectedEntry.path), renameTarget);
      await cmd.sftpRename({
        ...sshArgs,
        from: selectedEntry.path,
        to: nextPath,
      });
      setNotice(t("Renamed {from} to {to}.", { from: selectedEntry.name, to: renameTarget.trim() }));
      setRenameOpen(false);
      await browse(currentRemotePath);
      setSelectedPath(nextPath);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function removeSelected() {
    if (!hasSsh || !selectedEntry) return;
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      await cmd.sftpRemove({
        ...sshArgs,
        path: selectedEntry.path,
        isDir: selectedEntry.isDir,
      });
      setNotice(t("Removed {path}.", { path: selectedEntry.path }));
      await browse(currentRemotePath);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  /** Download a single remote file into `localDir` (an absolute local
   *  directory). Pushes a transfer queue entry and updates it on
   *  success/failure. Non-blocking for concurrent download fan-out —
   *  callers decide whether to await. */
  async function downloadOne(
    entry: { path: string; name: string },
    localDir: string,
  ): Promise<void> {
    if (!hasSsh) return;
    const trimmedDir = localDir.trim().replace(/[\\/]+$/, "");
    const sep = /^[A-Za-z]:[\\/]|^\\\\/.test(trimmedDir) ? "\\" : "/";
    const localPath = `${trimmedDir}${sep}${entry.name}`;
    const id = pushTransfer({
      direction: "dn",
      name: entry.name,
      remotePath: entry.path,
      localPath,
    });
    try {
      await cmd.sftpDownload({
        ...sshArgs,
        remotePath: entry.path,
        localPath,
        transferId: id,
      });
      finishTransfer(id, "done");
      setNotice(t("Downloaded {path}.", { path: entry.path }));
    } catch (e) {
      const msg = formatError(e);
      finishTransfer(id, "failed", msg);
      setError(msg);
    }
  }

  /** Download the currently-selected file to a user-chosen directory.
   *  Opens a native folder picker; no-op if the user cancels. */
  async function downloadSelectedPick() {
    if (!hasSsh || !selectedEntry || selectedEntry.isDir) return;
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: t("Select download folder"),
      });
      if (!picked || typeof picked !== "string") return;
      setActionBusy(true);
      setError("");
      setNotice("");
      await downloadOne({ path: selectedEntry.path, name: selectedEntry.name }, picked);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  /** Upload each local file into `remoteDir`. Fan-out is serialized
   *  to avoid flooding the single cached SSH session; swap to
   *  `Promise.all` later if pier-core's sftp channel grows concurrent
   *  transfer support. */
  async function uploadLocalFiles(localPaths: string[], remoteDir: string): Promise<void> {
    if (!hasSsh || localPaths.length === 0) return;
    setActionBusy(true);
    setError("");
    setNotice("");
    let okCount = 0;
    for (const localPath of localPaths) {
      const baseName = localBaseName(localPath);
      if (!baseName) continue;
      const remotePath = joinRemotePath(remoteDir, baseName);
      const id = pushTransfer({
        direction: "up",
        name: baseName,
        remotePath,
        localPath,
      });
      try {
        await cmd.sftpUpload({
          ...sshArgs,
          localPath,
          remotePath,
          transferId: id,
        });
        finishTransfer(id, "done");
        okCount++;
      } catch (e) {
        const msg = formatError(e);
        finishTransfer(id, "failed", msg);
        setError(msg);
      }
    }
    if (okCount > 0) {
      setNotice(t("Uploaded {count} file(s).", { count: okCount }));
      await browse(currentRemotePath);
    }
    setActionBusy(false);
  }

  /** Open a native file picker (multi-select) and upload the chosen
   *  files into the current remote directory. */
  async function uploadPick() {
    if (!hasSsh) return;
    try {
      const picked = await openDialog({
        directory: false,
        multiple: true,
        title: t("Select files to upload"),
      });
      if (!picked) return;
      const list = Array.isArray(picked) ? picked : [picked];
      if (list.length === 0) return;
      await uploadLocalFiles(list, currentRemotePath);
    } catch (e) {
      setError(formatError(e));
    }
  }

  // Auto-browse on mount / tab switch so SFTP works without the user
  // having to click "Browse". The backend reuses the SSH session that
  // the terminal already authenticated (seeded into the SFTP cache at
  // terminal-create time), so we don't gate this on credentials being
  // present in the tab — the cache + keychain resolution handle both
  // fresh and saved-password connections.
  useEffect(() => {
    if (!hasSsh) return;
    if (state) return;
    if (busy) return;
    void browse(path || "/");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    tab.id,
    tab.backend,
    sshTarget?.host,
    sshTarget?.port,
    sshTarget?.user,
    sshTarget?.authMode,
    tab.terminalSessionId,
    (sshTarget?.password.length ?? 0) > 0,
    sshTarget?.savedConnectionIndex,
  ]);

  function selectEntry(entry: SftpEntryView) {
    setSelectedPath(entry.path);
    setRenameTarget(entry.name);
    setRenameOpen(false);
  }

  function openEntry(entry: SftpEntryView) {
    if (entry.isDir) {
      void browse(entry.path, { pushHistory: true });
    } else {
      selectEntry(entry);
    }
  }

  // ── Drag-drop between Sidebar ↔ SFTP ────────────────────────────
  //
  // The Sidebar writes `DT_LOCAL_FILE` when dragging a local file; we
  // read that on drop and upload into `currentRemotePath`. Internal
  // drags *out* of the SFTP panel (remote→local) set `DT_SFTP_FILE`
  // which is handled by the Sidebar on its side.
  function handleListDragEnter(event: ReactDragEvent<HTMLDivElement>) {
    if (!hasSsh) return;
    if (!Array.from(event.dataTransfer.types).includes(DT_LOCAL_FILE)) return;
    event.preventDefault();
    setDropDepth((d) => d + 1);
  }
  function handleListDragOver(event: ReactDragEvent<HTMLDivElement>) {
    if (!hasSsh) return;
    if (!Array.from(event.dataTransfer.types).includes(DT_LOCAL_FILE)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  }
  function handleListDragLeave(event: ReactDragEvent<HTMLDivElement>) {
    if (!Array.from(event.dataTransfer.types).includes(DT_LOCAL_FILE)) return;
    event.preventDefault();
    setDropDepth((d) => Math.max(0, d - 1));
  }
  function handleListDrop(event: ReactDragEvent<HTMLDivElement>) {
    setDropDepth(0);
    if (!hasSsh) return;
    const raw = event.dataTransfer.getData(DT_LOCAL_FILE);
    if (!raw) return;
    event.preventDefault();
    try {
      const payload = JSON.parse(raw) as LocalDragPayload | LocalDragPayload[];
      const items = (Array.isArray(payload) ? payload : [payload]).filter(
        (p): p is LocalDragPayload => !!p && typeof p.path === "string",
      );
      if (items.length === 0) return;
      const files = items.filter((p) => !p.isDir);
      const dirs = items.filter((p) => p.isDir);
      if (files.length > 0) {
        void uploadLocalFiles(files.map((p) => p.path), currentRemotePath);
      }
      for (const dir of dirs) {
        void uploadLocalTree(dir);
      }
    } catch {
      // Malformed payload — silently ignore rather than scare the user.
    }
  }

  /** Recursively upload a local directory to the current remote
   *  directory. Creates a single transfer queue entry for the whole
   *  folder and lets the backend aggregate byte-level progress. */
  async function uploadLocalTree(dir: LocalDragPayload) {
    if (!hasSsh) return;
    const remotePath = joinRemotePath(currentRemotePath, dir.name);
    const id = pushTransfer({
      direction: "up",
      name: `${dir.name}/`,
      remotePath,
      localPath: dir.path,
    });
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      await cmd.sftpUploadTree({
        ...sshArgs,
        localPath: dir.path,
        remotePath,
        transferId: id,
      });
      finishTransfer(id, "done");
      setNotice(t("Uploaded folder {path}.", { path: dir.name }));
      await browse(currentRemotePath);
    } catch (e) {
      const msg = formatError(e);
      finishTransfer(id, "failed", msg);
      setError(msg);
    } finally {
      setActionBusy(false);
    }
  }

  function handleRowDragStart(event: ReactDragEvent<HTMLDivElement>, entry: SftpEntryView) {
    // Folders ARE draggable now — Sidebar dispatches a recursive
    // download via `sftp_download_tree`. The payload carries the
    // `isDir` flag so the receiving side picks the right command.
    const payload: SftpDragPayload = {
      path: entry.path,
      name: entry.name,
      isDir: entry.isDir,
      size: entry.size,
      host: sshArgs.host,
      port: sshArgs.port,
      user: sshArgs.user,
      authMode: sshArgs.authMode,
    };
    event.dataTransfer.effectAllowed = "copy";
    event.dataTransfer.setData(DT_SFTP_FILE, JSON.stringify(payload));
  }

  function crumbPath(index: number): string {
    if (index === 0) return "/";
    const segs = crumbSegments.slice(1, index + 1);
    return "/" + segs.join("/");
  }

  function commitPathDraft() {
    const next = pathDraft.trim() || "/";
    setEditingPath(false);
    void browse(next, { pushHistory: true });
  }

  const totalItems = state?.entries.length ?? 0;
  const hostName = sshTarget
    ? `${sshTarget.user}@${sshTarget.host}`
    : t("Not connected");
  const hostSub = sshTarget
    ? t("{user}@{host}:{port} · SFTP session", {
        user: sshTarget.user,
        host: sshTarget.host,
        port: sshTarget.port,
      })
    : t("Configure SSH connection to begin.");

  // Build one flat virtualized-list payload: the ".." parent row (if any)
  // followed by all entries. Memoized because `entries` is the usual
  // thousands-item case and we don't want to copy the array on every
  // render. Empty list when we're not connected / haven't browsed yet.
  const listRows = useMemo<FtpListRow[]>(() => {
    const rows: FtpListRow[] = [];
    if (state && currentRemotePath !== "/") rows.push({ kind: "parent" });
    if (state) {
      for (const entry of state.entries) rows.push({ kind: "entry", entry });
    }
    return rows;
  }, [state, currentRemotePath]);

  const renderListRow = (row: FtpListRow) => {
    if (row.kind === "parent") {
      return (
        <div
          key="__parent__"
          className="ftp-row dir"
          style={{ height: FTP_ROW_HEIGHT }}
          onClick={() => void browse(remoteDirname(currentRemotePath), { pushHistory: true })}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              void browse(remoteDirname(currentRemotePath), { pushHistory: true });
            }
          }}
          role="button"
          tabIndex={0}
          aria-label={t("Parent directory")}
        >
          <span className="ftp-ic"><ArrowUp size={13} /></span>
          <span className="ftp-name">..</span>
          <span className="ftp-perm mono">—</span>
          <span className="ftp-size mono">—</span>
          <span className="ftp-mod mono">—</span>
        </div>
      );
    }
    const entry = row.entry;
    const Ic = iconForEntry(entry);
    const isSel = selectedEntry?.path === entry.path;
    return (
      <div
        key={entry.path}
        className={"ftp-row" + (isSel ? " sel" : "") + (entry.isDir ? " dir" : "")}
        style={{ height: FTP_ROW_HEIGHT }}
        onClick={() => selectEntry(entry)}
        onDoubleClick={() => openEntry(entry)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            openEntry(entry);
          } else if (e.key === " ") {
            e.preventDefault();
            selectEntry(entry);
          }
        }}
        role="button"
        tabIndex={0}
        aria-selected={isSel}
        aria-label={entry.name}
        draggable
        onDragStart={(e) => handleRowDragStart(e, entry)}
      >
        <span className="ftp-ic"><Ic size={13} /></span>
        <span className="ftp-name" title={entry.name}>{entry.name}</span>
        <span className="ftp-perm mono">
          {entry.permissions || (entry.isDir ? "drwxr-xr-x" : "-rw-r--r--")}
        </span>
        <span className="ftp-size mono">{entry.isDir ? "—" : formatBytes(entry.size)}</span>
        <span className="ftp-mod mono" title={formatModifiedTooltip(entry.modified)}>
          {formatModifiedTime(entry.modified)}
        </span>
      </div>
    );
  };

  const renderListPane = () => {
    // Empty / loading states stay non-virtualized — they're single-line
    // hints, not lists. The virtualized list takes over as soon as we
    // have entries.
    if (!hasSsh) {
      return (
        <div
          className={"ftp-list" + (dropHover ? " is-drop" : "")}
          onDragEnter={handleListDragEnter}
          onDragOver={handleListDragOver}
          onDragLeave={handleListDragLeave}
          onDrop={handleListDrop}
        >
          <div className="lg-note">{sshRequired}</div>
        </div>
      );
    }
    if (!state && !busy) {
      return (
        <div
          className={"ftp-list" + (dropHover ? " is-drop" : "")}
          onDragEnter={handleListDragEnter}
          onDragOver={handleListDragOver}
          onDragLeave={handleListDragLeave}
          onDrop={handleListDrop}
        >
          <div className="lg-note">
            <button type="button" className="btn is-primary is-compact" onClick={() => void browse(path || "/")}>
              {t("Browse")}
            </button>
          </div>
        </div>
      );
    }
    // First load: no cached state yet. Show a shimmering skeleton so the
    // panel doesn't collapse to a single "Browsing..." line and the
    // layout pre-stamps the row grid before real data arrives.
    if (busy && !state) {
      return (
        <div
          className={"ftp-list" + (dropHover ? " is-drop" : "")}
          onDragEnter={handleListDragEnter}
          onDragOver={handleListDragOver}
          onDragLeave={handleListDragLeave}
          onDrop={handleListDrop}
        >
          <FtpSkeleton rows={10} />
        </div>
      );
    }
    // Refresh path (busy && state): keep the existing virtualized list
    // visible and dim it slightly instead of wiping it — avoids the
    // flicker the old "Browsing..." branch caused on every refresh.
    // The refresh icon's spin state in the toolbar telegraphs "in
    // flight" so the user still has a loading signal.
    return (
      <VirtualList<FtpListRow>
        className={
          "ftp-list" + (dropHover ? " is-drop" : "") + (busy ? " is-loading" : "")
        }
        items={listRows}
        rowHeight={FTP_ROW_HEIGHT}
        renderRow={renderListRow}
        onDragEnter={handleListDragEnter}
        onDragOver={handleListDragOver}
        onDragLeave={handleListDragLeave}
        onDrop={handleListDrop}
      />
    );
  };

  return (
    <>
      <PanelHeader icon={RIGHT_TOOL_META.sftp.icon} title={t("SFTP")} meta={currentRemotePath} />
      <div className="ftp">
        <div className="ftp-host-bar">
          <span className="ftp-host-ic"><Server size={12} /></span>
          <div className="ftp-host-meta">
            <div className="ftp-host-name">{hostName}</div>
            <div className="ftp-host-sub mono">{hostSub}</div>
          </div>
          <span className={"ftp-host-pill" + (hasSsh ? "" : " off")}>
            <StatusDot tone={hasSsh ? "pos" : "off"} />
            {hasSsh ? t("connected") : t("offline")}
          </span>
        </div>

        <div className="ftp-pathbar">
          <button
            type="button"
            className="lg-ic"
            title={t("Back")}
            disabled={!history.length || busy}
            onClick={() => void goBack()}
          >
            <ArrowLeft size={12} />
          </button>
          <button
            type="button"
            className="lg-ic"
            title={t("Forward")}
            disabled={!forward.length || busy}
            onClick={() => void goForward()}
          >
            <ArrowRight size={12} />
          </button>
          <button
            type="button"
            className="lg-ic"
            title={t("Up one level")}
            disabled={!state || currentRemotePath === "/" || busy}
            onClick={() => void browse(remoteDirname(currentRemotePath), { pushHistory: true })}
          >
            <ArrowUp size={12} />
          </button>
          {editingPath ? (
            <input
              className="ftp-path-input mono"
              autoFocus
              value={pathDraft}
              onChange={(e) => setPathDraft(e.currentTarget.value)}
              onBlur={commitPathDraft}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitPathDraft();
                if (e.key === "Escape") {
                  setPathDraft(currentRemotePath);
                  setEditingPath(false);
                }
              }}
            />
          ) : (
            <div
              className="ftp-crumb mono"
              onClick={() => { setPathDraft(currentRemotePath); setEditingPath(true); }}
            >
              {crumbSegments.map((s, i) => {
                const isLast = i === crumbSegments.length - 1;
                return (
                  <Fragment key={i}>
                    <span
                      className={"seg" + (isLast ? " last" : "")}
                      onClick={(e) => {
                        e.stopPropagation();
                        if (isLast) return;
                        void browse(crumbPath(i), { pushHistory: true });
                      }}
                    >
                      {s === "/" ? <Home size={11} /> : s}
                    </span>
                    {!isLast && i !== 0 && <span className="sep">/</span>}
                    {i === 0 && crumbSegments.length > 1 && <span className="sep">/</span>}
                  </Fragment>
                );
              })}
              <button
                type="button"
                className="ftp-path-edit"
                title={t("Edit path")}
                onClick={(e) => {
                  e.stopPropagation();
                  setPathDraft(currentRemotePath);
                  setEditingPath(true);
                }}
              >
                <Edit size={10} />
              </button>
            </div>
          )}
          <button
            type="button"
            className={"lg-ic" + (mkdirOpen ? " on" : "")}
            title={t("New folder")}
            disabled={!hasSsh || !state}
            onClick={() => setMkdirOpen((v) => !v)}
          >
            <Plus size={12} />
          </button>
          <button
            type="button"
            className="lg-ic"
            title={t("Upload from local")}
            disabled={!hasSsh || !state || actionBusy}
            onClick={() => void uploadPick()}
          >
            <Upload size={12} />
          </button>
          <button
            type="button"
            className="lg-ic"
            title={t("Refresh")}
            disabled={!hasSsh || busy}
            onClick={() => void browse(currentRemotePath)}
          >
            <RefreshCw size={12} className={busy ? "ftp-spin" : ""} />
          </button>
        </div>

        {mkdirOpen && (
          <div className="ftp-quickrow">
            <span className="ftp-quickrow-label mono">{t("New folder")}</span>
            <input
              className="field-input field-input--compact"
              value={mkdirName}
              onChange={(e) => setMkdirName(e.currentTarget.value)}
              placeholder={t("logs")}
              autoFocus
              onKeyDown={(e) => { if (e.key === "Enter") void createDirectory(); }}
            />
            <button
              type="button"
              className="btn is-primary is-compact"
              disabled={!mkdirName.trim() || actionBusy}
              onClick={() => void createDirectory()}
            >
              {t("Create")}
            </button>
            <button type="button" className="btn is-ghost is-compact" onClick={() => setMkdirOpen(false)}>{t("Cancel")}</button>
          </div>
        )}

        <div className="ftp-col-head">
          <span>{t("NAME")}</span>
          <span className="ftp-perm">{t("PERM")}</span>
          <span className="ftp-size">{t("SIZE")}</span>
          <span className="ftp-mod">{t("MODIFIED")}</span>
        </div>

        {renderListPane()}

        {(notice || error) && (
          <div className="ftp-notice-bar">
            {notice && <div className="lg-note">{notice}</div>}
            {error && <div className="lg-note lg-note--error">{error}</div>}
          </div>
        )}

        {selectedEntry && (
          <div className="ftp-inspector">
            <div className="ftp-inspector-head mono">
              <span className={"ftp-ic" + (selectedEntry.isDir ? " dir" : "")}>
                {(() => {
                  const Ic = iconForEntry(selectedEntry);
                  return <Ic size={12} />;
                })()}
              </span>
              <span className="ftp-inspector-name">{selectedEntry.name}</span>
              <span className="ftp-inspector-meta">
                {selectedEntry.permissions || (selectedEntry.isDir ? "drwxr-xr-x" : "-rw-r--r--")}
                {!selectedEntry.isDir ? ` · ${formatBytes(selectedEntry.size)}` : ""}
              </span>
              <div style={{ flex: 1 }} />
              <button
                type="button"
                className={"lg-ic" + (renameOpen ? " on" : "")}
                title={t("Rename")}
                onClick={() => setRenameOpen((v) => !v)}
              >
                <Edit size={11} />
              </button>
              {!selectedEntry.isDir && (
                <button
                  type="button"
                  className="lg-ic"
                  title={t("Download")}
                  disabled={actionBusy}
                  onClick={() => void downloadSelectedPick()}
                >
                  <Download size={11} />
                </button>
              )}
              <button
                type="button"
                className="lg-ic"
                title={t("Remove")}
                disabled={actionBusy}
                onClick={() => void removeSelected()}
              >
                <Trash2 size={11} />
              </button>
              <button type="button" className="lg-ic" title={t("Close")} onClick={() => setSelectedPath("")}>
                <X size={11} />
              </button>
            </div>
            {renameOpen && (
              <div className="ftp-quickrow">
                <span className="ftp-quickrow-label mono">{t("Rename")}</span>
                <input
                  className="field-input field-input--compact"
                  value={renameTarget}
                  onChange={(e) => setRenameTarget(e.currentTarget.value)}
                  autoFocus
                  onKeyDown={(e) => { if (e.key === "Enter") void renameSelected(); }}
                />
                <button
                  type="button"
                  className="btn is-primary is-compact"
                  disabled={!renameTarget.trim() || actionBusy}
                  onClick={() => void renameSelected()}
                >
                  {t("Save")}
                </button>
                <button type="button" className="btn is-ghost is-compact" onClick={() => setRenameOpen(false)}>{t("Cancel")}</button>
              </div>
            )}
          </div>
        )}

        <div className="ftp-disk mono">
          <HardDrive size={10} />
          <span>{t("SFTP session")}</span>
          <div style={{ flex: 1 }} />
          <span>
            {t("{n} items", { n: totalItems })}
            {selectedEntry ? ` · ${t("1 selected")}` : ""}
          </span>
        </div>

        {transfers.length > 0 && (
          <div className="ftp-queue">
            <div className="ftp-queue-head">
              <span className="ftp-queue-title">
                <Upload size={10} /> {t("TRANSFERS")}
              </span>
              <span className="ftp-queue-count mono">
                {t("{active} active · {done} done", { active: activeTransfers, done: doneTransfers })}
              </span>
              {doneTransfers > 0 && (
                <button
                  type="button"
                  className="lg-ic"
                  title={t("Clear completed")}
                  onClick={clearFinishedTransfers}
                >
                  <X size={11} />
                </button>
              )}
            </div>
            {transfers.map((item) => {
              const isActive = item.status === "active";
              const isDone = item.status === "done";
              const isFailed = item.status === "failed";
              const arrow = item.direction === "up" ? <ArrowRight size={10} /> : <ArrowLeft size={10} />;
              const destHint = item.direction === "up"
                ? `→ ${remoteDirname(item.remotePath)}/`
                : `→ ${item.localPath}`;
              const bytes = item.bytes ?? 0;
              const total = item.total ?? 0;
              const pct = total > 0 ? Math.min(100, Math.floor((bytes / total) * 100)) : null;
              const bytesLabel = total > 0
                ? `${formatBytes(bytes)} / ${formatBytes(total)}`
                : bytes > 0
                  ? formatBytes(bytes)
                  : null;
              return (
                <div
                  key={item.id}
                  className={"ftp-queue-item" + (isDone ? " done" : "") + (isFailed ? " failed" : "")}
                >
                  <span className={"ftp-queue-dir " + item.direction}>{arrow}</span>
                  <div className="ftp-queue-body">
                    <div className="ftp-queue-name mono">
                      {item.name} <span className="text-muted">{destHint}</span>
                    </div>
                    <div className="ftp-queue-meta mono">
                      {isActive && (
                        <>
                          {bytesLabel && <span>{bytesLabel}</span>}
                          {bytesLabel && <span className="sep">·</span>}
                          <span>{t("transferring…")}</span>
                        </>
                      )}
                      {isDone && (
                        <>
                          {total > 0 && <span>{formatBytes(total)}</span>}
                          {total > 0 && <span className="sep">·</span>}
                          <span className="text-pos">{t("✓ done")}</span>
                        </>
                      )}
                      {isFailed && <span className="text-neg">{item.error || t("failed")}</span>}
                    </div>
                    {isActive && (
                      <div className="ftp-queue-track">
                        {pct != null ? (
                          <div className="ftp-queue-fill" style={{ width: `${pct}%` }} />
                        ) : (
                          <div className="ftp-queue-fill ftp-queue-fill--anim" />
                        )}
                      </div>
                    )}
                  </div>
                  {isActive && pct != null && (
                    <span className="ftp-queue-pct ftp-queue-pct--active mono">{pct}%</span>
                  )}
                  {isDone && (
                    <span className="ftp-queue-pct mono">
                      <Check size={11} />
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </>
  );
}
