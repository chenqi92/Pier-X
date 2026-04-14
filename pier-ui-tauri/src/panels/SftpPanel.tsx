import { FolderTree } from "lucide-react";
import { useState } from "react";
import * as cmd from "../lib/commands";
import type { SftpBrowseState, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";

type Props = { tab: TabState };

export default function SftpPanel({ tab }: Props) {
  const { t } = useI18n();
  const [state, setState] = useState<SftpBrowseState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [path, setPath] = useState("/");

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();
  const sshRequired = t("SSH connection required.");

  async function browse(targetPath = path) {
    if (!hasSsh) { setError(sshRequired); return; }
    setBusy(true); setError("");
    try {
      const s = await cmd.sftpBrowse({ host: tab.sshHost, port: tab.sshPort, user: tab.sshUser, authMode: tab.sshAuthMode, password: tab.sshPassword, keyPath: tab.sshKeyPath, path: targetPath });
      setState(s); setPath(s.currentPath);
    } catch (e) { setState(null); setError(String(e)); }
    finally { setBusy(false); }
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
          {error && <div className="status-note status-note--error">{error}</div>}
        </div>
      </section>

      {state && (
        <section className="panel-section">
          <div className="panel-section__title"><span>{state.currentPath}</span></div>
          <div className="git-change-list">
            {state.currentPath !== "/" && (
              <button className="git-change-button" onClick={() => void browse(state.currentPath.replace(/\/[^/]+\/?$/, "") || "/")} type="button">
                <span className="git-badge git-badge--staged">{t("DIR")}</span><span className="git-change-row__path">..</span>
              </button>
            )}
            {state.entries.map((entry) => (
              <button key={entry.path} className="git-change-button" onClick={() => { if (entry.isDir) void browse(entry.path); }} type="button">
                <span className={entry.isDir ? "git-badge git-badge--staged" : "git-badge"}>{entry.isDir ? t("DIR") : t("FILE")}</span>
                <span className="git-change-row__path">{entry.name}</span>
                {!entry.isDir && <span className="inline-note">{t("{size} B", { size: entry.size })}</span>}
              </button>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
