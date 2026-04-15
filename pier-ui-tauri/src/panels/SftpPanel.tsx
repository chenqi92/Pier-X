import { FolderTree } from "lucide-react";
import { useMemo, useState } from "react";
import * as cmd from "../lib/commands";
import type { SftpBrowseState, SftpEntryView, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";

type Props = { tab: TabState };

function joinRemotePath(basePath: string, leaf: string) {
  const cleanLeaf = leaf.trim().replace(/^\/+/, "");
  if (!cleanLeaf) {
    return basePath;
  }
  const normalizedBase = basePath === "/" ? "/" : basePath.replace(/\/+$/, "");
  return normalizedBase === "/" ? `/${cleanLeaf}` : `${normalizedBase}/${cleanLeaf}`;
}

function remoteDirname(path: string) {
  const normalized = String(path || "").replace(/\/+$/, "");
  if (!normalized || normalized === "/") {
    return "/";
  }
  const index = normalized.lastIndexOf("/");
  if (index <= 0) {
    return "/";
  }
  return normalized.slice(0, index);
}

function localBaseName(path: string) {
  const normalized = String(path || "").replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/);
  return parts[parts.length - 1] || "";
}

export default function SftpPanel({ tab }: Props) {
  const { t } = useI18n();
  const [state, setState] = useState<SftpBrowseState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [path, setPath] = useState("/");
  const [selectedPath, setSelectedPath] = useState("");
  const [mkdirName, setMkdirName] = useState("");
  const [renameTarget, setRenameTarget] = useState("");
  const [downloadLocalPath, setDownloadLocalPath] = useState("");
  const [uploadLocalPath, setUploadLocalPath] = useState("");
  const [uploadRemotePath, setUploadRemotePath] = useState("");
  const [actionBusy, setActionBusy] = useState(false);

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();
  const sshRequired = t("SSH connection required.");
  const selectedEntry = useMemo(
    () => state?.entries.find((entry) => entry.path === selectedPath) ?? null,
    [state, selectedPath],
  );

  async function browse(targetPath = path) {
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
      setState(next);
      setPath(next.currentPath);
      setSelectedPath("");
      setRenameTarget("");
      setDownloadLocalPath("");
    } catch (e) {
      setState(null);
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function createDirectory() {
    if (!hasSsh || !mkdirName.trim()) {
      return;
    }
    setActionBusy(true);
    setError("");
    setNotice("");
    try {
      const targetPath = joinRemotePath(path, mkdirName);
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
      setNotice(t("Created directory {path}.", { path: targetPath }));
      await browse(path);
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function renameSelected() {
    if (!hasSsh || !selectedEntry || !renameTarget.trim()) {
      return;
    }
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
      await browse(path);
      setSelectedPath(nextPath);
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function removeSelected() {
    if (!hasSsh || !selectedEntry) {
      return;
    }
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
      await browse(path);
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function downloadSelected() {
    if (!hasSsh || !selectedEntry || selectedEntry.isDir || !downloadLocalPath.trim()) {
      return;
    }
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
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function uploadFile() {
    if (!hasSsh || !uploadLocalPath.trim() || !uploadRemotePath.trim()) {
      return;
    }
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
      await browse(path);
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
  }

  return (
    <div className="panel-scroll">
      <section className="panel-section">
        <div className="panel-section__title"><FolderTree size={14} /><span>{t("SFTP")}</span></div>
        <div className="form-stack">
          <label className="field-stack">
            <span className="field-label">{t("Remote path")}</span>
            <div className="branch-row">
              <input className="field-input" onChange={(e) => setPath(e.currentTarget.value)} value={path} />
              <button className="mini-button" disabled={!hasSsh || busy} onClick={() => void browse()} type="button">{busy ? t("Browsing...") : t("Browse")}</button>
            </div>
          </label>
          {!hasSsh && <div className="inline-note">{sshRequired}</div>}
          {notice && <div className="status-note">{notice}</div>}
          {error && <div className="status-note status-note--error">{error}</div>}
        </div>
      </section>

      {state && (
        <>
          <section className="panel-section">
            <div className="panel-section__title"><span>{t("Operations")}</span></div>
            <div className="form-stack">
              <label className="field-stack">
                <span className="field-label">{t("Create directory")}</span>
                <div className="branch-row">
                  <input className="field-input" onChange={(e) => setMkdirName(e.currentTarget.value)} placeholder={t("logs")} value={mkdirName} />
                  <button className="mini-button" disabled={!mkdirName.trim() || actionBusy} onClick={() => void createDirectory()} type="button">{t("Create")}</button>
                </div>
              </label>
              <label className="field-stack">
                <span className="field-label">{t("Upload local file")}</span>
                <input
                  className="field-input"
                  onChange={(e) => {
                    const nextValue = e.currentTarget.value;
                    setUploadLocalPath(nextValue);
                    if (!uploadRemotePath.trim()) {
                      const baseName = localBaseName(nextValue);
                      setUploadRemotePath(baseName ? joinRemotePath(path, baseName) : "");
                    }
                  }}
                  placeholder={t("C:\\Users\\you\\Downloads\\bundle.tar.gz")}
                  value={uploadLocalPath}
                />
              </label>
              <label className="field-stack">
                <span className="field-label">{t("Remote upload path")}</span>
                <div className="branch-row">
                  <input className="field-input" onChange={(e) => setUploadRemotePath(e.currentTarget.value)} placeholder={joinRemotePath(path, "bundle.tar.gz")} value={uploadRemotePath} />
                  <button className="mini-button" disabled={!uploadLocalPath.trim() || !uploadRemotePath.trim() || actionBusy} onClick={() => void uploadFile()} type="button">{t("Upload")}</button>
                </div>
              </label>
            </div>
          </section>

          <section className="panel-section">
            <div className="panel-section__title"><span>{state.currentPath}</span></div>
            <div className="git-change-list">
              {state.currentPath !== "/" && (
                <button className="git-change-button" onClick={() => void browse(remoteDirname(state.currentPath))} type="button">
                  <span className="git-badge git-badge--staged">{t("DIR")}</span><span className="git-change-row__path">..</span>
                </button>
              )}
              {state.entries.map((entry) => (
                <div className={selectedEntry?.path === entry.path ? "connection-row connection-row--selected" : "connection-row"} key={entry.path}>
                  <div className="connection-row__head">
                    <button className="git-change-button" onClick={() => selectEntry(entry)} type="button">
                      <span className={entry.isDir ? "git-badge git-badge--staged" : "git-badge"}>{entry.isDir ? t("DIR") : t("FILE")}</span>
                      <span className="git-change-row__path">{entry.name}</span>
                    </button>
                    {entry.permissions ? <span className="connection-pill">{entry.permissions}</span> : null}
                  </div>
                  <div className="connection-row__meta">{entry.path}{!entry.isDir ? ` · ${t("{size} B", { size: entry.size })}` : ""}</div>
                  <div className="connection-row__actions">
                    {entry.isDir && <button className="mini-button" onClick={() => void browse(entry.path)} type="button">{t("Open")}</button>}
                    {!entry.isDir && <button className="mini-button" onClick={() => selectEntry(entry)} type="button">{t("Use")}</button>}
                  </div>
                </div>
              ))}
            </div>
          </section>

          {selectedEntry && (
            <section className="panel-section">
              <div className="panel-section__title"><span>{t("Selected Entry")}</span></div>
              <div className="form-stack">
                <div className="inline-note">{selectedEntry.path}</div>
                <label className="field-stack">
                  <span className="field-label">{t("Rename target")}</span>
                  <div className="branch-row">
                    <input className="field-input" onChange={(e) => setRenameTarget(e.currentTarget.value)} value={renameTarget} />
                    <button className="mini-button" disabled={!renameTarget.trim() || actionBusy} onClick={() => void renameSelected()} type="button">{t("Rename")}</button>
                  </div>
                </label>
                {!selectedEntry.isDir && (
                  <label className="field-stack">
                    <span className="field-label">{t("Download to local path")}</span>
                    <div className="branch-row">
                      <input className="field-input" onChange={(e) => setDownloadLocalPath(e.currentTarget.value)} placeholder={t("C:\\Users\\you\\Downloads\\artifact.log")} value={downloadLocalPath} />
                      <button className="mini-button" disabled={!downloadLocalPath.trim() || actionBusy} onClick={() => void downloadSelected()} type="button">{t("Download")}</button>
                    </div>
                  </label>
                )}
                <div className="button-row">
                  <button className="mini-button mini-button--destructive" disabled={actionBusy} onClick={() => void removeSelected()} type="button">{t("Remove")}</button>
                </div>
              </div>
            </section>
          )}
        </>
      )}
    </div>
  );
}
