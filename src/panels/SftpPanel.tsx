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
  FolderTree,
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
import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import type { ComponentType } from "react";
import * as cmd from "../lib/commands";
import type { SftpBrowseState, SftpEntryView, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";

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

function remoteBaseName(path: string) {
  const normalized = String(path || "").replace(/\/+$/, "");
  if (!normalized || normalized === "/") return "/";
  const idx = normalized.lastIndexOf("/");
  return idx < 0 ? normalized : normalized.slice(idx + 1) || "/";
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

/** Render a Unix-seconds timestamp as a compact "3m / 2h / 4d / 6w / 3mo / 2y"
 *  relative label matching the Remix design. Falls back to em-dash if the
 *  server didn't report a modified time. */
function formatRelativeTime(unixSeconds: number | null | undefined): string {
  if (!unixSeconds || !Number.isFinite(unixSeconds)) return "—";
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
  if (diff < 60) return `${diff}s`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  if (diff < 604800) return `${Math.floor(diff / 86400)}d`;
  if (diff < 2592000) return `${Math.floor(diff / 604800)}w`;
  if (diff < 31536000) return `${Math.floor(diff / 2592000)}mo`;
  return `${Math.floor(diff / 31536000)}y`;
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
  const [downloadOpen, setDownloadOpen] = useState(false);
  const [downloadLocalPath, setDownloadLocalPath] = useState("");
  const [mkdirOpen, setMkdirOpen] = useState(false);
  const [mkdirName, setMkdirName] = useState("");
  const [uploadOpen, setUploadOpen] = useState(false);
  const [uploadLocalPath, setUploadLocalPath] = useState("");
  const [uploadRemotePath, setUploadRemotePath] = useState("");
  const [actionBusy, setActionBusy] = useState(false);

  const [transfers, setTransfers] = useState<TransferItem[]>([]);
  const transferSeq = useRef(0);

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();
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
    const id = `xfer-${++transferSeq.current}`;
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
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        savedConnectionIndex: tab.sshSavedConnectionIndex,
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
      setDownloadOpen(false);
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
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        savedConnectionIndex: tab.sshSavedConnectionIndex,
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
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        savedConnectionIndex: tab.sshSavedConnectionIndex,
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
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        savedConnectionIndex: tab.sshSavedConnectionIndex,
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

  async function downloadSelected() {
    if (!hasSsh || !selectedEntry || selectedEntry.isDir || !downloadLocalPath.trim()) return;
    setActionBusy(true);
    setError("");
    setNotice("");
    const localPath = downloadLocalPath.trim();
    const id = pushTransfer({
      direction: "dn",
      name: selectedEntry.name,
      remotePath: selectedEntry.path,
      localPath,
    });
    try {
      await cmd.sftpDownload({
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        savedConnectionIndex: tab.sshSavedConnectionIndex,
        remotePath: selectedEntry.path,
        localPath,
      });
      finishTransfer(id, "done");
      setNotice(t("Downloaded {path}.", { path: selectedEntry.path }));
      setDownloadOpen(false);
    } catch (e) {
      const msg = formatError(e);
      finishTransfer(id, "failed", msg);
      setError(msg);
    } finally {
      setActionBusy(false);
    }
  }

  async function uploadFile() {
    if (!hasSsh || !uploadLocalPath.trim() || !uploadRemotePath.trim()) return;
    setActionBusy(true);
    setError("");
    setNotice("");
    const localPath = uploadLocalPath.trim();
    const remotePath = uploadRemotePath.trim();
    const id = pushTransfer({
      direction: "up",
      name: localBaseName(localPath) || remoteBaseName(remotePath),
      remotePath,
      localPath,
    });
    try {
      await cmd.sftpUpload({
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        savedConnectionIndex: tab.sshSavedConnectionIndex,
        localPath,
        remotePath,
      });
      finishTransfer(id, "done");
      setNotice(t("Uploaded {path}.", { path: remotePath }));
      setUploadOpen(false);
      setUploadLocalPath("");
      setUploadRemotePath("");
      await browse(currentRemotePath);
    } catch (e) {
      const msg = formatError(e);
      finishTransfer(id, "failed", msg);
      setError(msg);
    } finally {
      setActionBusy(false);
    }
  }

  // Auto-browse on mount / tab switch so SFTP works without the user
  // having to click "Browse". Password-auth saved tabs still work — the
  // backend resolves the keychain password via savedConnectionIndex.
  useEffect(() => {
    if (!hasSsh) return;
    if (state) return;
    if (busy) return;
    const ready =
      tab.sshAuthMode !== "password" ||
      tab.sshPassword.length > 0 ||
      tab.sshSavedConnectionIndex !== null;
    if (!ready) return;
    void browse(path || "/");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    tab.id,
    tab.backend,
    tab.sshHost,
    tab.sshPort,
    tab.sshUser,
    tab.sshAuthMode,
    tab.sshPassword.length > 0,
    tab.sshSavedConnectionIndex,
  ]);

  function selectEntry(entry: SftpEntryView) {
    setSelectedPath(entry.path);
    setRenameTarget(entry.name);
    setDownloadLocalPath(entry.isDir ? "" : localBaseName(entry.path));
    setRenameOpen(false);
    setDownloadOpen(false);
  }

  function openEntry(entry: SftpEntryView) {
    if (entry.isDir) {
      void browse(entry.path, { pushHistory: true });
    } else {
      selectEntry(entry);
    }
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
  const hostName = hasSsh ? `${tab.sshUser}@${tab.sshHost}` : t("Not connected");
  const hostSub = hasSsh
    ? t("{user}@{host}:{port} · SFTP session", {
        user: tab.sshUser,
        host: tab.sshHost,
        port: tab.sshPort,
      })
    : t("Configure SSH connection to begin.");

  return (
    <>
      <PanelHeader icon={FolderTree} title={t("SFTP")} meta={currentRemotePath} />
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
            onClick={() => { setMkdirOpen((v) => !v); setUploadOpen(false); }}
          >
            <Plus size={12} />
          </button>
          <button
            type="button"
            className={"lg-ic" + (uploadOpen ? " on" : "")}
            title={t("Upload from local")}
            disabled={!hasSsh || !state}
            onClick={() => { setUploadOpen((v) => !v); setMkdirOpen(false); }}
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
            <RefreshCw size={12} />
          </button>
        </div>

        {(mkdirOpen || uploadOpen) && (
          <div className="ftp-quickrow">
            {mkdirOpen && (
              <>
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
              </>
            )}
            {uploadOpen && (
              <>
                <span className="ftp-quickrow-label mono">{t("Upload")}</span>
                <input
                  className="field-input field-input--compact"
                  value={uploadLocalPath}
                  onChange={(e) => {
                    const nextValue = e.currentTarget.value;
                    setUploadLocalPath(nextValue);
                    if (!uploadRemotePath.trim()) {
                      const baseName = localBaseName(nextValue);
                      setUploadRemotePath(baseName ? joinRemotePath(currentRemotePath, baseName) : "");
                    }
                  }}
                  placeholder={t("Local path…")}
                />
                <input
                  className="field-input field-input--compact"
                  value={uploadRemotePath}
                  onChange={(e) => setUploadRemotePath(e.currentTarget.value)}
                  placeholder={joinRemotePath(currentRemotePath, "file")}
                />
                <button
                  type="button"
                  className="btn is-primary is-compact"
                  disabled={!uploadLocalPath.trim() || !uploadRemotePath.trim() || actionBusy}
                  onClick={() => void uploadFile()}
                >
                  {t("Upload")}
                </button>
                <button type="button" className="btn is-ghost is-compact" onClick={() => setUploadOpen(false)}>{t("Cancel")}</button>
              </>
            )}
          </div>
        )}

        <div className="ftp-col-head">
          <span>{t("NAME")}</span>
          <span className="ftp-size">{t("SIZE")}</span>
          <span className="ftp-mod">{t("MOD")}</span>
        </div>

        <div className="ftp-list">
          {!hasSsh && <div className="lg-note">{sshRequired}</div>}
          {hasSsh && !state && !busy && (
            <div className="lg-note">
              <button type="button" className="btn is-primary is-compact" onClick={() => void browse(path || "/")}>
                {t("Browse")}
              </button>
            </div>
          )}
          {busy && <div className="lg-note">{t("Browsing...")}</div>}
          {state && currentRemotePath !== "/" && (
            <div
              className="ftp-row dir"
              onClick={() => void browse(remoteDirname(currentRemotePath), { pushHistory: true })}
            >
              <span className="ftp-ic"><ArrowUp size={13} /></span>
              <span className="ftp-name">..</span>
              <span className="ftp-size mono">—</span>
              <span className="ftp-mod mono">—</span>
            </div>
          )}
          {state?.entries.map((entry) => {
            const Ic = iconForEntry(entry);
            const isSel = selectedEntry?.path === entry.path;
            return (
              <div
                key={entry.path}
                className={"ftp-row" + (isSel ? " sel" : "") + (entry.isDir ? " dir" : "")}
                onClick={() => selectEntry(entry)}
                onDoubleClick={() => openEntry(entry)}
              >
                <span className="ftp-ic"><Ic size={13} /></span>
                <span className="ftp-name">{entry.name}</span>
                <span className="ftp-size mono">{entry.isDir ? "—" : formatBytes(entry.size)}</span>
                <span className="ftp-mod mono" title={entry.modified ? new Date(entry.modified * 1000).toLocaleString() : ""}>
                  {formatRelativeTime(entry.modified)}
                </span>
              </div>
            );
          })}
        </div>

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
                onClick={() => { setRenameOpen((v) => !v); setDownloadOpen(false); }}
              >
                <Edit size={11} />
              </button>
              {!selectedEntry.isDir && (
                <button
                  type="button"
                  className={"lg-ic" + (downloadOpen ? " on" : "")}
                  title={t("Download")}
                  onClick={() => { setDownloadOpen((v) => !v); setRenameOpen(false); }}
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
            {downloadOpen && !selectedEntry.isDir && (
              <div className="ftp-quickrow">
                <span className="ftp-quickrow-label mono">{t("Download")}</span>
                <input
                  className="field-input field-input--compact"
                  value={downloadLocalPath}
                  onChange={(e) => setDownloadLocalPath(e.currentTarget.value)}
                  placeholder={t("C:\\Users\\you\\Downloads\\artifact.log")}
                  autoFocus
                />
                <button
                  type="button"
                  className="btn is-primary is-compact"
                  disabled={!downloadLocalPath.trim() || actionBusy}
                  onClick={() => void downloadSelected()}
                >
                  {t("Save")}
                </button>
                <button type="button" className="btn is-ghost is-compact" onClick={() => setDownloadOpen(false)}>{t("Cancel")}</button>
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
                      {isActive && <span>{t("transferring…")}</span>}
                      {isDone && <span className="text-pos">{t("✓ done")}</span>}
                      {isFailed && <span className="text-neg">{item.error || t("failed")}</span>}
                    </div>
                    {isActive && (
                      <div className="ftp-queue-track">
                        <div className="ftp-queue-fill ftp-queue-fill--anim" />
                      </div>
                    )}
                  </div>
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
