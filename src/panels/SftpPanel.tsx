import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Check,
  Edit,
  File as FileIcon,
  FileText,
  Folder,
  HardDrive,
  Home,
  Plus,
  RefreshCw,
  Server,
  Star,
  Terminal as TerminalIcon,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import type { DragEvent as ReactDragEvent, MouseEvent as ReactMouseEvent } from "react";
import type { ComponentType } from "react";
import * as cmd from "../lib/commands";
import { SFTP_PROGRESS_EVENT, type SftpProgressEvent } from "../lib/commands";
import type { SftpBrowseState, SftpEntryView, TabState } from "../lib/types";
import { effectiveSshTarget, isSshTargetReady } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import StatusDot from "../components/StatusDot";
import VirtualList from "../components/VirtualList";
import ContextMenu, { type ContextMenuItem } from "../components/ContextMenu";
import ChmodDialog from "../components/ChmodDialog";
import ConfirmDialog from "../components/ConfirmDialog";
import DismissibleNote from "../components/DismissibleNote";
import SftpEditorDialog from "../components/SftpEditorDialog";
import SftpNewEntryDialog from "../components/SftpNewEntryDialog";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";
import { writeClipboardText } from "../lib/clipboard";
import {
  hostKey,
  useSftpBookmarksStore,
  type SftpBookmark,
} from "../stores/useSftpBookmarksStore";

// Module-scope constant for "no bookmarks". Kept out of the
// zustand selector so two consecutive renders get the *same*
// reference — otherwise getSnapshot sees a new `[]` every time
// and React flags an infinite update loop.
const EMPTY_BOOKMARKS: SftpBookmark[] = [];
import { isEditableFilename, MAX_EDITOR_BYTES, modeToSymbolic } from "../lib/sftpEditor";
import {
  DT_LOCAL_FILE,
  DT_SFTP_FILE,
  hasDragPayload,
  readDragPayload,
  writeDragPayload,
  type LocalDragPayload,
  type SftpDragPayload,
} from "../lib/sftpDrag";

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

function joinLocalPath(dir: string, leaf: string): string {
  const trimmed = dir.trim().replace(/[\\/]+$/, "");
  const sep = /^[A-Za-z]:($|[\\/])|^\\\\/.test(trimmed) ? "\\" : "/";
  return `${trimmed}${sep}${leaf}`;
}

/** Parse an `ls -l` style permission string (e.g. `-rwxr-xr--`) back
 *  to the octal `u32` the chmod dialog wants as its seed. Returns
 *  `null` when the input isn't the 10-char symbolic form — callers
 *  then fall back to 0o644. */
function parseSymbolicPermissions(s: string | null | undefined): number | null {
  if (!s || s.length < 10) return null;
  const core = s.slice(1, 10);
  let mode = 0;
  for (let i = 0; i < 9; i++) {
    if (core[i] !== "-") mode |= 1 << (8 - i);
  }
  return mode;
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

export default function SftpPanel(props: Props) {
  const ready = useDeferredMount();
  return (
    <div className="panel-stage">
      {ready ? <SftpPanelBody {...props} /> : <PanelSkeleton variant="rows" rows={10} />}
    </div>
  );
}

function SftpPanelBody({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const [state, setState] = useState<SftpBrowseState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  // Auto-dismiss the info notice after a few seconds so it doesn't
  // linger and cover the list below. Error stays put — the user
  // should explicitly acknowledge failures.
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(""), 3000);
    return () => window.clearTimeout(timer);
  }, [notice]);
  // Empty-string sentinel means "ask the backend to resolve a
  // sensible default" — it'll run `pwd` / `$HOME` on the remote and
  // return the user's home rather than dropping us at `/`. Once the
  // first browse lands, this is set to whatever `current_path` came
  // back (already canonicalised), and every subsequent browse
  // carries an explicit path.
  const [path, setPath] = useState("");
  const [selectedPath, setSelectedPath] = useState("");
  const [editingPath, setEditingPath] = useState(false);
  const [pathDraft, setPathDraft] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const [forward, setForward] = useState<string[]>([]);

  // Inline row rename. The previous implementation lived in a bottom
  // "inspector" strip; now the filename cell itself flips into an
  // editable input when the user picks Rename from the right-click
  // menu (or presses F2).
  const [renamingPath, setRenamingPath] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [actionBusy, setActionBusy] = useState(false);

  const [transfers, setTransfers] = useState<TransferItem[]>([]);
  const transferSeq = useRef(0);
  const [dropDepth, setDropDepth] = useState(0);
  const [osDropHover, setOsDropHover] = useState(false);
  const dropHover = dropDepth > 0 || osDropHover;
  const panelRef = useRef<HTMLDivElement | null>(null);

  // ── Context menu + extended actions ────────────────────────────
  type CtxState =
    | { kind: "entry"; x: number; y: number; entry: SftpEntryView }
    | { kind: "empty"; x: number; y: number }
    | null;
  const [ctxMenu, setCtxMenu] = useState<CtxState>(null);

  const [editorTarget, setEditorTarget] = useState<
    { path: string; name: string; size: number } | null
  >(null);

  const [chmodTarget, setChmodTarget] = useState<{ path: string; mode: number | null } | null>(null);

  // Unified "New entry" dialog — replaces the previous separate
  // inline quickrows for mkdir and createFile and offers a single
  // surface with a file/folder radio.
  const [newEntryOpen, setNewEntryOpen] = useState(false);
  const [newEntryKind, setNewEntryKind] = useState<"file" | "dir">("file");
  const [newEntryName, setNewEntryName] = useState("");

  const [propsTarget, setPropsTarget] = useState<SftpEntryView | null>(null);

  // Pending delete target for the themed ConfirmDialog. `window.confirm`
  // is not available in Tauri's webview (returns undefined), so the
  // previous `if (!window.confirm(...)) return` guard silently passed
  // through and deletions fired with no user prompt. Gate the actual
  // removal on this dialog instead. The anchor carries the click
  // position so the dialog opens near the cursor rather than jumping
  // to the viewport center.
  const [deleteTarget, setDeleteTarget] = useState<
    { entry: SftpEntryView; anchor?: { x: number; y: number } } | null
  >(null);

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
  const bookmarkHostKey = hasSsh ? hostKey(sshArgs.user, sshArgs.host, sshArgs.port) : "";
  // Select the raw store entry (which is a stable reference —
  // either a persistent array or `undefined`) and fall back to
  // the module-scope empty constant. Earlier we fabricated `[]`
  // inside the selector, which zustand's getSnapshot treated as
  // a fresh value every render and drove an infinite update loop.
  const bookmarksForHost = useSftpBookmarksStore(
    (s) => (bookmarkHostKey ? s.bookmarks[bookmarkHostKey] : undefined),
  );
  const bookmarks = bookmarksForHost ?? EMPTY_BOOKMARKS;
  const addBookmark = useSftpBookmarksStore((s) => s.add);
  const removeBookmark = useSftpBookmarksStore((s) => s.remove);
  const [bookmarksOpen, setBookmarksOpen] = useState(false);
  const currentIsBookmarked = useMemo(
    () => bookmarks.some((b) => b.path === (state?.currentPath || path || "/")),
    [bookmarks, state?.currentPath, path],
  );
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

  // OS-level drag-drop: Tauri intercepts file drops from the host
  // file manager (Finder / Explorer / Nautilus) and delivers absolute
  // paths via `onDragDropEvent`. We gate by the panel's bounding rect
  // in device pixels so dropping onto an adjacent panel (Git, Docker)
  // doesn't trigger an upload here. Only fires when SFTP is active.
  useEffect(() => {
    if (!hasSsh) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;

    function isOverPanel(px: number, py: number): boolean {
      const el = panelRef.current;
      if (!el) return false;
      const rect = el.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      const x = px / dpr;
      const y = py / dpr;
      return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
    }

    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === "enter" || p.type === "over") {
          setOsDropHover(isOverPanel(p.position.x, p.position.y));
        } else if (p.type === "leave") {
          setOsDropHover(false);
        } else if (p.type === "drop") {
          setOsDropHover(false);
          if (!isOverPanel(p.position.x, p.position.y)) return;
          const paths = p.paths ?? [];
          if (paths.length === 0) return;
          void uploadLocalFiles(paths, currentRemotePath);
        }
      })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      });

    return () => {
      disposed = true;
      unlisten?.();
      setOsDropHover(false);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasSsh, currentRemotePath, sshArgs.host, sshArgs.port, sshArgs.user, sshArgs.authMode]);

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
      setRenamingPath(null);
      setRenameDraft("");
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

  function openNewEntryDialog(kind: "file" | "dir") {
    setNewEntryKind(kind);
    setNewEntryName("");
    setNewEntryOpen(true);
  }

  /** Create the file or directory the user filled in via the "New…"
   *  dialog. Routes to `sftp_mkdir` or `sftp_create_file` based on
   *  `newEntryKind`, then refreshes the listing. */
  async function submitNewEntry() {
    const name = newEntryName.trim();
    if (!hasSsh || !name) return;
    const targetPath = joinRemotePath(currentRemotePath, name);
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      if (newEntryKind === "dir") {
        await cmd.sftpMkdir({ ...sshArgs, path: targetPath });
        setNotice(t("Created directory {path}.", { path: targetPath }));
      } else {
        await cmd.sftpCreateFile({ ...sshArgs, path: targetPath });
        setNotice(t("Created file {path}.", { path: targetPath }));
      }
      setNewEntryOpen(false);
      setNewEntryName("");
      await browse(currentRemotePath);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  function startInlineRename(entry: SftpEntryView) {
    setRenamingPath(entry.path);
    setRenameDraft(entry.name);
  }

  function cancelInlineRename() {
    setRenamingPath(null);
    setRenameDraft("");
  }

  async function commitInlineRename() {
    if (!hasSsh || !renamingPath) return;
    const draft = renameDraft.trim();
    const current = state?.entries.find((e) => e.path === renamingPath);
    if (!current) {
      cancelInlineRename();
      return;
    }
    if (!draft || draft === current.name) {
      cancelInlineRename();
      return;
    }
    const nextPath = joinRemotePath(remoteDirname(current.path), draft);
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      await cmd.sftpRename({ ...sshArgs, from: current.path, to: nextPath });
      setNotice(t("Renamed {from} to {to}.", { from: current.name, to: draft }));
      setRenamingPath(null);
      setRenameDraft("");
      await browse(currentRemotePath);
      if (selectedPath === current.path) setSelectedPath(nextPath);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  /** Actually issue the `sftp_remove` — called by the ConfirmDialog
   *  once the user confirms. The context-menu "Delete" action just
   *  stages the entry via `setDeleteTarget`. */
  async function performRemove(entry: SftpEntryView) {
    if (!hasSsh) return;
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      await cmd.sftpRemove({ ...sshArgs, path: entry.path, isDir: entry.isDir });
      setNotice(t("Removed {path}.", { path: entry.path }));
      await browse(currentRemotePath);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  /** Server-side copy via read+write (no atomic `cp` in SFTP). Gated
   *  on size because the whole file traverses the wire twice. */
  async function duplicateEntry(entry: SftpEntryView) {
    if (!hasSsh || entry.isDir) {
      setError(t("Duplicate only works on files."));
      return;
    }
    if (entry.size > MAX_EDITOR_BYTES) {
      setError(t("File is too large to duplicate in-place. Download and re-upload instead."));
      return;
    }
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      const res = await cmd.sftpReadText({
        ...sshArgs,
        path: entry.path,
        maxBytes: MAX_EDITOR_BYTES,
      });
      const baseName = entry.name.replace(/(\.[^./]+)?$/, (ext) => ` (copy)${ext ?? ""}`);
      const targetPath = joinRemotePath(remoteDirname(entry.path), baseName);
      await cmd.sftpWriteText({ ...sshArgs, path: targetPath, content: res.content });
      setNotice(t("Duplicated {from} to {to}.", { from: entry.name, to: baseName }));
      await browse(currentRemotePath);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function copyEntryPath(entry: SftpEntryView) {
    await writeClipboardText(entry.path);
    setNotice(t("Copied path to clipboard."));
  }

  async function applyChmod(path: string, mode: number) {
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      await cmd.sftpChmod({ ...sshArgs, path, mode });
      setNotice(t("Permissions changed to {mode}.", { mode: mode.toString(8).padStart(3, "0") }));
      setChmodTarget(null);
      await browse(currentRemotePath);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  /** Hand a remote file off to the OS default editor without going
   *  through the editor dialog at all — useful from the context menu
   *  for files the user knows they want to edit externally (e.g.
   *  binaries, multi-MB logs). The backend downloads to a temp path,
   *  spawns the system opener, and starts the auto-upload watcher.
   *  We don't surface watcher status here — that's the dialog's job;
   *  callers from the context menu just get a confirmation notice. */
  async function openEntryExternally(entry: SftpEntryView) {
    if (!hasSsh || entry.isDir) return;
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      await cmd.sftpOpenExternal({ ...sshArgs, path: entry.path });
      setNotice(t("Opened {name} in your system editor.", { name: entry.name }));
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  function openEditorFor(entry: SftpEntryView) {
    if (entry.isDir) {
      void browse(entry.path, { pushHistory: true });
      return;
    }
    // Always open the dialog — for too-large files it renders the
    // "open with system editor / download" branch instead of erroring
    // out silently. Pass `size` so the dialog can pick that branch
    // before issuing any sftp_read_text round-trip.
    setEditorTarget({ path: entry.path, name: entry.name, size: entry.size });
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
    const localPath = joinLocalPath(localDir, entry.name);
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

  /** Download `entry` to a user-chosen directory. Opens a native
   *  folder picker; no-op if the user cancels. Takes the entry
   *  explicitly so context-menu callers don't need to round-trip
   *  through `selectedEntry` state (which would be stale for a
   *  right-click on an unselected row). */
  async function downloadEntryPick(entry: SftpEntryView) {
    if (!hasSsh || entry.isDir) return;
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
      await downloadOne({ path: entry.path, name: entry.name }, picked);
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
  //
  // Deps are intentionally narrow: only the tab id and whether any
  // SSH target resolves. Credential-shape changes (password arriving
  // after a prompt, saved-index swap) used to re-fire this effect
  // while the first browse was still in flight; the `!state`/`!busy`
  // guards below made those re-fires no-ops but we still paid the
  // effect-scheduling cost and risked double calls under React
  // Strict Mode. The initial browse covers every case we care about.
  useEffect(() => {
    // Gate on credentials, not just `hasSsh`: the PTY watcher writes
    // host/user/port the moment it sees `ssh user@host`, but the
    // password lands later when the user types it. A premature browse
    // with empty password produces a misleading auth-rejected error.
    if (!isSshTargetReady(sshTarget)) return;
    if (state) return;
    if (busy) return;
    // Pass the current path (which is "" on the very first browse);
    // the backend resolves that sentinel to the remote user's $HOME
    // / login-shell `pwd`. Once we have state, subsequent browses
    // always carry an explicit path.
    void browse(path);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    tab.id,
    sshTarget?.host,
    sshTarget?.port,
    sshTarget?.user,
    sshTarget?.authMode,
    sshTarget?.password,
    sshTarget?.savedConnectionIndex,
  ]);

  function selectEntry(entry: SftpEntryView) {
    setSelectedPath(entry.path);
  }

  function openEntry(entry: SftpEntryView) {
    if (entry.isDir) {
      void browse(entry.path, { pushHistory: true });
      return;
    }
    // Double-click on a text-ish file → open the editor dialog. The
    // dialog handles the size split internally: under the inline
    // limit it mounts CodeMirror, otherwise it shows the "open with
    // system editor / download" card. Binary-looking files stay on
    // "select only" so a misclick doesn't dump 200MB through a
    // UTF-8 lossy read.
    if (isEditableFilename(entry.name)) {
      selectEntry(entry);
      openEditorFor(entry);
      return;
    }
    selectEntry(entry);
  }

  function buildEntryContextMenu(
    entry: SftpEntryView,
    anchor?: { x: number; y: number },
  ): ContextMenuItem[] {
    // Edit is now always available for non-directories — the dialog
    // handles the inline-vs-too-large split internally and shows
    // sensible fallbacks (open externally / download) for binaries
    // and oversized files.
    const items: ContextMenuItem[] = [];
    if (entry.isDir) {
      items.push({
        label: t("Open"),
        action: () => void browse(entry.path, { pushHistory: true }),
      });
    } else {
      items.push({
        label: t("Edit"),
        action: () => openEditorFor(entry),
      });
      items.push({
        label: t("Open with system editor"),
        action: () => void openEntryExternally(entry),
        disabled: actionBusy,
      });
      items.push({
        label: t("Download…"),
        action: () => void downloadEntryPick(entry),
        disabled: actionBusy,
      });
    }
    items.push({ divider: true });
    items.push({ label: t("Rename"), action: () => startInlineRename(entry) });
    if (!entry.isDir) {
      items.push({ label: t("Duplicate"), action: () => void duplicateEntry(entry) });
    }
    items.push({
      label: t("Delete"),
      action: () => setDeleteTarget({ entry, anchor }),
    });
    items.push({ divider: true });
    items.push({ label: t("New file…"), action: () => openNewEntryDialog("file") });
    items.push({ label: t("New folder…"), action: () => openNewEntryDialog("dir") });
    items.push({ divider: true });
    items.push({
      label: t("Change permissions…"),
      action: () => setChmodTarget({
        path: entry.path,
        mode: parseSymbolicPermissions(entry.permissions),
      }),
    });
    items.push({ label: t("Copy path"), action: () => void copyEntryPath(entry) });
    items.push({ divider: true });
    items.push({ label: t("Properties"), action: () => setPropsTarget(entry) });
    return items;
  }

  function buildEmptyContextMenu(): ContextMenuItem[] {
    return [
      { label: t("New file…"), action: () => openNewEntryDialog("file") },
      { label: t("New folder…"), action: () => openNewEntryDialog("dir") },
      { divider: true },
      { label: t("Upload…"), action: () => void uploadPick(), disabled: actionBusy },
      { label: t("Refresh"), action: () => void browse(currentRemotePath) },
    ];
  }

  function handleRowContextMenu(event: ReactMouseEvent<HTMLDivElement>, entry: SftpEntryView) {
    event.preventDefault();
    event.stopPropagation();
    selectEntry(entry);
    setCtxMenu({ kind: "entry", x: event.clientX, y: event.clientY, entry });
  }

  function handleEmptyContextMenu(event: ReactMouseEvent<HTMLDivElement>) {
    // Only fire for clicks on the list background — the per-row
    // handler stops propagation so this never sees row-level events.
    const target = event.target as HTMLElement | null;
    if (target?.closest(".ftp-row")) return;
    if (!hasSsh) return;
    event.preventDefault();
    setCtxMenu({ kind: "empty", x: event.clientX, y: event.clientY });
  }

  // ── Drag-drop between Sidebar ↔ SFTP ────────────────────────────
  //
  // The Sidebar writes `DT_LOCAL_FILE` when dragging a local file; we
  // read that on drop and upload into `currentRemotePath`. Internal
  // drags *out* of the SFTP panel (remote→local) set `DT_SFTP_FILE`
  // which is handled by the Sidebar on its side.
  function handleListDragEnter(event: ReactDragEvent<HTMLDivElement>) {
    if (!hasSsh) return;
    if (!hasDragPayload(event.dataTransfer, DT_LOCAL_FILE)) return;
    event.preventDefault();
    setDropDepth((d) => d + 1);
  }
  function handleListDragOver(event: ReactDragEvent<HTMLDivElement>) {
    if (!hasSsh) return;
    if (!hasDragPayload(event.dataTransfer, DT_LOCAL_FILE)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  }
  function handleListDragLeave(event: ReactDragEvent<HTMLDivElement>) {
    if (!hasDragPayload(event.dataTransfer, DT_LOCAL_FILE)) return;
    event.preventDefault();
    setDropDepth((d) => Math.max(0, d - 1));
  }
  function handleListDrop(event: ReactDragEvent<HTMLDivElement>) {
    setDropDepth(0);
    if (!hasSsh) return;
    const payload = readDragPayload(event.dataTransfer, DT_LOCAL_FILE, "local-file");
    if (!payload) return;
    event.preventDefault();
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
      sourceTabId: tab.id,
    };
    event.dataTransfer.effectAllowed = "copy";
    writeDragPayload(event.dataTransfer, DT_SFTP_FILE, "sftp-file", payload);
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
    const isRenaming = renamingPath === entry.path;
    return (
      <div
        key={entry.path}
        className={"ftp-row" + (isSel ? " sel" : "") + (entry.isDir ? " dir" : "")}
        style={{ height: FTP_ROW_HEIGHT }}
        onClick={() => { if (!isRenaming) selectEntry(entry); }}
        onDoubleClick={() => { if (!isRenaming) openEntry(entry); }}
        onContextMenu={(e) => handleRowContextMenu(e, entry)}
        onKeyDown={(e) => {
          if (isRenaming) return;
          if (e.key === "Enter") {
            e.preventDefault();
            openEntry(entry);
          } else if (e.key === " ") {
            e.preventDefault();
            selectEntry(entry);
          } else if (e.key === "F2") {
            e.preventDefault();
            startInlineRename(entry);
          }
        }}
        role="button"
        tabIndex={0}
        aria-selected={isSel}
        aria-label={entry.name}
        draggable={!isRenaming}
        onDragStart={(e) => handleRowDragStart(e, entry)}
      >
        <span className="ftp-ic"><Ic size={13} /></span>
        {isRenaming ? (
          <input
            className="ftp-rename-input"
            value={renameDraft}
            autoFocus
            onChange={(e) => setRenameDraft(e.currentTarget.value)}
            onClick={(e) => e.stopPropagation()}
            onDoubleClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") {
                e.preventDefault();
                void commitInlineRename();
              } else if (e.key === "Escape") {
                e.preventDefault();
                cancelInlineRename();
              }
            }}
            onBlur={() => void commitInlineRename()}
            onFocus={(e) => {
              // Preselect the stem (everything before the last dot) so
              // the user can retype the name without clobbering the
              // extension by accident — VS Code / Finder behavior.
              const v = e.currentTarget.value;
              const dot = v.lastIndexOf(".");
              if (dot > 0) e.currentTarget.setSelectionRange(0, dot);
              else e.currentTarget.select();
            }}
          />
        ) : (
          <span className="ftp-name" title={entry.name}>{entry.name}</span>
        )}
        <span className="ftp-perm mono">
          {entry.permissions || (entry.isDir ? "drwxr-xr-x" : "-rw-r--r--")}
        </span>
        <span
          className="ftp-owner mono"
          title={
            entry.owner && entry.group && entry.owner !== entry.group
              ? `${entry.owner}:${entry.group}`
              : entry.owner || entry.group || ""
          }
        >
          {entry.owner || "—"}
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
            <button type="button" className="btn is-primary is-compact" onClick={() => void browse(path)}>
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
        onContextMenu={handleEmptyContextMenu}
      />
    );
  };

  return (
    <>
      <div className="ftp" ref={panelRef}>
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
          <div style={{ position: "relative" }}>
            <button
              type="button"
              className="lg-ic"
              title={currentIsBookmarked ? t("Remove bookmark") : t("Bookmark this path")}
              disabled={!hasSsh || !state}
              onClick={() => setBookmarksOpen((o) => !o)}
            >
              {currentIsBookmarked ? (
                <Star size={12} fill="var(--accent)" color="var(--accent)" />
              ) : (
                <Star size={12} />
              )}
            </button>
            {bookmarksOpen && (
              <div
                className="cmdp-overlay"
                style={{ background: "transparent" }}
                onClick={() => setBookmarksOpen(false)}
              >
                <div
                  className="ftp-bookmarks-pop"
                  style={{
                    position: "absolute",
                    top: 32,
                    left: 0,
                    minWidth: 280,
                    maxWidth: 420,
                    maxHeight: 360,
                    overflowY: "auto",
                    background: "var(--elev)",
                    border: "1px solid var(--line)",
                    borderRadius: "var(--radius-sm)",
                    boxShadow: "var(--shadow-popover)",
                    padding: "var(--sp-2)",
                    zIndex: 100,
                  }}
                  onClick={(e) => e.stopPropagation()}
                >
                  <button
                    type="button"
                    className="mini-button"
                    style={{ width: "100%", justifyContent: "flex-start" }}
                    disabled={currentIsBookmarked}
                    onClick={() => {
                      addBookmark(bookmarkHostKey, { path: currentRemotePath });
                      setBookmarksOpen(false);
                    }}
                  >
                    <Star size={11} />
                    {t("Bookmark {path}", { path: currentRemotePath })}
                  </button>
                  {bookmarks.length > 0 && (
                    <div
                      style={{
                        marginTop: "var(--sp-2)",
                        borderTop: "1px solid var(--line)",
                        paddingTop: "var(--sp-2)",
                        display: "flex",
                        flexDirection: "column",
                        gap: "var(--sp-1)",
                      }}
                    >
                      {bookmarks.map((b) => (
                        <div key={b.path} style={{ display: "flex", gap: "var(--sp-1)", alignItems: "center" }}>
                          <button
                            type="button"
                            className="mini-button"
                            style={{ flex: 1, justifyContent: "flex-start", fontFamily: "var(--mono)", overflow: "hidden" }}
                            title={b.path}
                            onClick={() => {
                              setBookmarksOpen(false);
                              void browse(b.path, { pushHistory: true });
                            }}
                          >
                            <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                              {b.label || b.path}
                            </span>
                          </button>
                          <button
                            type="button"
                            className="mini-button mini-button--destructive"
                            title={t("Remove bookmark")}
                            onClick={() => removeBookmark(bookmarkHostKey, b.path)}
                          >
                            <Trash2 size={11} />
                          </button>
                        </div>
                      ))}
                    </div>
                  )}
                  {bookmarks.length === 0 && (
                    <div className="empty-note" style={{ marginTop: "var(--sp-2)" }}>
                      {t("No bookmarks for this host.")}
                    </div>
                  )}
                </div>
              </div>
            )}
          </div>
          <button
            type="button"
            className="lg-ic"
            title={t("New file or folder")}
            disabled={!hasSsh || !state}
            onClick={() => openNewEntryDialog("file")}
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

        <div className="ftp-col-head">
          <span>{t("NAME")}</span>
          <span className="ftp-perm">{t("PERM")}</span>
          <span className="ftp-owner">{t("OWNER")}</span>
          <span className="ftp-size">{t("SIZE")}</span>
          <span className="ftp-mod">{t("MODIFIED")}</span>
        </div>

        {renderListPane()}

        {(notice || error) && (
          <div className="ftp-notice-bar">
            {notice && (
              <DismissibleNote onDismiss={() => setNotice("")}>{notice}</DismissibleNote>
            )}
            {error && (
              <DismissibleNote tone="error" onDismiss={() => setError("")}>
                {error}
              </DismissibleNote>
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

        {ctxMenu && (
          <ContextMenu
            x={ctxMenu.x}
            y={ctxMenu.y}
            items={
              ctxMenu.kind === "entry"
                ? buildEntryContextMenu(ctxMenu.entry, { x: ctxMenu.x, y: ctxMenu.y })
                : buildEmptyContextMenu()
            }
            onClose={() => setCtxMenu(null)}
          />
        )}

        {chmodTarget && (
          <ChmodDialog
            open
            path={chmodTarget.path}
            initialMode={chmodTarget.mode}
            onSubmit={(mode) => applyChmod(chmodTarget.path, mode)}
            onClose={() => setChmodTarget(null)}
            busy={actionBusy}
          />
        )}

        {editorTarget && (
          <SftpEditorDialog
            open
            path={editorTarget.path}
            name={editorTarget.name}
            size={editorTarget.size}
            sshArgs={sshArgs}
            ownerLabel={sshArgs.user}
            onClose={() => setEditorTarget(null)}
            onSaved={() => void browse(currentRemotePath)}
          />
        )}

        <SftpNewEntryDialog
          open={newEntryOpen}
          kind={newEntryKind}
          name={newEntryName}
          parentPath={currentRemotePath}
          busy={actionBusy}
          onKindChange={setNewEntryKind}
          onNameChange={setNewEntryName}
          onSubmit={() => void submitNewEntry()}
          onClose={() => setNewEntryOpen(false)}
        />

        <ConfirmDialog
          open={deleteTarget !== null}
          tone="destructive"
          title={deleteTarget?.entry.isDir ? t("Remove directory") : t("Remove file")}
          message={
            deleteTarget
              ? deleteTarget.entry.isDir
                ? t("Remove directory {name}? It must be empty.", {
                    name: deleteTarget.entry.name,
                  })
                : t("Remove file {name}?", { name: deleteTarget.entry.name })
              : ""
          }
          confirmLabel={t("Delete")}
          anchor={deleteTarget?.anchor}
          onCancel={() => setDeleteTarget(null)}
          onConfirm={() => {
            const target = deleteTarget;
            setDeleteTarget(null);
            if (target) void performRemove(target.entry);
          }}
        />


        {propsTarget && (
          <div className="dlg-overlay" onClick={() => setPropsTarget(null)}>
            <div
              className="dlg dlg--props"
              onClick={(e) => e.stopPropagation()}
            >
              <div className="dlg-head">
                <span className="dlg-title">
                  <FileIcon size={13} />
                  {t("Properties")}
                </span>
                <div style={{ flex: 1 }} />
                <button type="button" className="lg-ic" onClick={() => setPropsTarget(null)} title={t("Close")}>
                  <X size={12} />
                </button>
              </div>
              <div className="dlg-body dlg-body--form">
                <div className="props-grid mono">
                  <span className="props-key">{t("Name")}</span>
                  <span className="props-val">{propsTarget.name}</span>
                  <span className="props-key">{t("Path")}</span>
                  <span className="props-val" title={propsTarget.path}>{propsTarget.path}</span>
                  <span className="props-key">{t("Type")}</span>
                  <span className="props-val">{propsTarget.isDir ? t("Directory") : t("File")}</span>
                  <span className="props-key">{t("Size")}</span>
                  <span className="props-val">{propsTarget.isDir ? "—" : formatBytes(propsTarget.size)}</span>
                  <span className="props-key">{t("Permissions")}</span>
                  <span className="props-val">
                    {propsTarget.permissions || (propsTarget.isDir ? "drwxr-xr-x" : "-rw-r--r--")}
                    {(() => {
                      const m = parseSymbolicPermissions(propsTarget.permissions);
                      return m != null ? ` · ${modeToSymbolic(m)} · ${m.toString(8).padStart(3, "0")}` : "";
                    })()}
                  </span>
                  <span className="props-key">{t("Modified")}</span>
                  <span className="props-val">{formatModifiedTooltip(propsTarget.modified) || "—"}</span>
                </div>
                <div className="chmod-actions">
                  <button
                    type="button"
                    className="btn is-ghost is-compact"
                    onClick={() => setPropsTarget(null)}
                  >
                    {t("Close")}
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}

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
