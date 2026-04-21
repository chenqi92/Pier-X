import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
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
import { Fragment, useMemo, useState } from "react";
import type { ComponentType } from "react";
import * as cmd from "../lib/commands";
import type { SftpBrowseState, SftpEntryView, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";

type Props = { tab: TabState };

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

export default function SftpPanel({ tab }: Props) {
  const { t } = useI18n();
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
      setError(String(e));
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
        path: targetPath,
      });
      setMkdirName("");
      setMkdirOpen(false);
      setNotice(t("Created directory {path}.", { path: targetPath }));
      await browse(currentRemotePath);
    } catch (e) {
      setError(String(e));
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
        from: selectedEntry.path,
        to: nextPath,
      });
      setNotice(t("Renamed {from} to {to}.", { from: selectedEntry.name, to: renameTarget.trim() }));
      setRenameOpen(false);
      await browse(currentRemotePath);
      setSelectedPath(nextPath);
    } catch (e) {
      setError(String(e));
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
        path: selectedEntry.path,
        isDir: selectedEntry.isDir,
      });
      setNotice(t("Removed {path}.", { path: selectedEntry.path }));
      await browse(currentRemotePath);
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function downloadSelected() {
    if (!hasSsh || !selectedEntry || selectedEntry.isDir || !downloadLocalPath.trim()) return;
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      await cmd.sftpDownload({
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        remotePath: selectedEntry.path,
        localPath: downloadLocalPath.trim(),
      });
      setNotice(t("Downloaded {path}.", { path: selectedEntry.path }));
      setDownloadOpen(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function uploadFile() {
    if (!hasSsh || !uploadLocalPath.trim() || !uploadRemotePath.trim()) return;
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      await cmd.sftpUpload({
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        localPath: uploadLocalPath.trim(),
        remotePath: uploadRemotePath.trim(),
      });
      setNotice(t("Uploaded {path}.", { path: uploadRemotePath.trim() }));
      setUploadOpen(false);
      setUploadLocalPath("");
      setUploadRemotePath("");
      await browse(currentRemotePath);
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

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
  const hostName = hasSsh ? tab.sshHost : t("Not connected");
  const hostSub = hasSsh
    ? `${tab.sshUser}@${tab.sshHost}:${tab.sshPort} · SFTP`
    : t("Configure SSH connection to begin.");

  return (
    <>
      <PanelHeader icon={FolderTree} title="SFTP" meta={currentRemotePath} />
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
            <div className="ftp-crumb2 mono" onClick={() => { setPathDraft(currentRemotePath); setEditingPath(true); }}>
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
                    {i === 0 && crumbSegments.length > 1 && <span className="sep" />}
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

        <div className="ftp-col-head2">
          <span />
          <span>{t("NAME")}</span>
          <span>{t("PERM")}</span>
          <span>{t("OWNER")}</span>
          <span className="ta-r">{t("SIZE")}</span>
          <span className="ta-r">{t("MODIFIED")}</span>
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
              className="ftp-row2 dir"
              onClick={() => void browse(remoteDirname(currentRemotePath), { pushHistory: true })}
            >
              <span className="ftp-ic"><ArrowUp size={13} /></span>
              <span className="ftp-name">..</span>
              <span className="ftp-perm mono">drwxr-xr-x</span>
              <span className="ftp-owner mono">—</span>
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
                className={"ftp-row2" + (isSel ? " sel" : "") + (entry.isDir ? " dir" : "")}
                onClick={() => selectEntry(entry)}
                onDoubleClick={() => openEntry(entry)}
              >
                <span className="ftp-ic"><Ic size={13} /></span>
                <span className="ftp-name">{entry.name}</span>
                <span className="ftp-perm mono">{entry.permissions || (entry.isDir ? "drwxr-xr-x" : "-rw-r--r--")}</span>
                <span className="ftp-owner mono">—</span>
                <span className="ftp-size mono">{entry.isDir ? "—" : formatBytes(entry.size)}</span>
                <span className="ftp-mod mono">—</span>
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
              <span className="ftp-inspector-meta">{selectedEntry.path}</span>
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
      </div>
    </>
  );
}
