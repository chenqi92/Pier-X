import { useState } from "react";
import * as cmd from "../lib/commands";
import type { DockerOverview, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";

type Props = { tab: TabState };

export default function DockerPanel({ tab }: Props) {
  const { t } = useI18n();
  const [state, setState] = useState<DockerOverview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [showAll, setShowAll] = useState(true);
  const [activeTab, setActiveTab] = useState<"containers" | "images" | "volumes" | "networks">("containers");
  const [actionBusy, setActionBusy] = useState(false);
  const [notice, setNotice] = useState("");

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();
  const sshRequired = t("SSH connection required.");

  async function refresh() {
    if (!hasSsh) { setError(sshRequired); return; }
    setBusy(true); setError("");
    try {
      const o = await cmd.dockerOverview({ host: tab.sshHost, port: tab.sshPort, user: tab.sshUser, authMode: tab.sshAuthMode, password: tab.sshPassword, keyPath: tab.sshKeyPath, all: showAll });
      setState(o);
    } catch (e) { setState(null); setError(String(e)); }
    finally { setBusy(false); }
  }

  async function containerAction(id: string, action: string) {
    if (actionBusy) return;
    setActionBusy(true); setNotice("");
    try {
      const r = await cmd.dockerContainerAction({ host: tab.sshHost, port: tab.sshPort, user: tab.sshUser, authMode: tab.sshAuthMode, password: tab.sshPassword, keyPath: tab.sshKeyPath, containerId: id, action });
      setNotice(`${id.slice(0, 12)}: ${r}`);
      await refresh();
    } catch (e) { setError(String(e)); }
    finally { setActionBusy(false); }
  }

  return (
    <div className="panel-scroll">
      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Docker")}</span></div>
        <div className="form-stack">
          <div className="button-row">
            <button className="mini-button" disabled={!hasSsh || busy} onClick={() => void refresh()} type="button">{busy ? t("Loading...") : t("Refresh Docker")}</button>
            <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text-secondary)" }}>
              <input type="checkbox" checked={showAll} onChange={() => setShowAll((p) => !p)} />{t("Show all containers")}
            </label>
          </div>
          {!hasSsh && <div className="inline-note">{t("SSH connection required for Docker.")}</div>}
          {notice && <div className="status-note">{notice}</div>}
          {error && <div className="status-note status-note--error">{error}</div>}
        </div>
      </section>

      {state && (
        <>
          <div className="surface-switcher" style={{ padding: "0 12px 8px" }}>
            {(["containers", "images", "volumes", "networks"] as const).map((tab) => (
              <button key={tab} className={activeTab === tab ? "surface-button surface-button--selected" : "surface-button"} onClick={() => setActiveTab(tab)} type="button">{t(tab.charAt(0).toUpperCase() + tab.slice(1))}</button>
            ))}
          </div>

          <section className="panel-section">
            {activeTab === "containers" ? (
              state.containers.length > 0 ? state.containers.map((c) => (
                <div className="connection-row" key={c.id}>
                  <div className="connection-row__head"><strong>{c.names || c.id.slice(0, 12)}</strong><span className="connection-pill">{c.state}</span></div>
                  <div className="connection-row__meta">{c.image} · {c.status}</div>
                  <div className="connection-row__actions">
                    {c.running ? (<>
                      <button className="mini-button" disabled={actionBusy} onClick={() => void containerAction(c.id, "stop")} type="button">{t("Stop")}</button>
                      <button className="mini-button" disabled={actionBusy} onClick={() => void containerAction(c.id, "restart")} type="button">{t("Restart")}</button>
                    </>) : (<>
                      <button className="mini-button" disabled={actionBusy} onClick={() => void containerAction(c.id, "start")} type="button">{t("Start")}</button>
                      <button className="mini-button" disabled={actionBusy} onClick={() => void containerAction(c.id, "remove")} type="button">{t("Remove")}</button>
                    </>)}
                  </div>
                </div>
              )) : <div className="empty-note">{t("No containers found.")}</div>
            ) : activeTab === "images" ? (
              state.images.length > 0 ? state.images.map((img) => (
                <div className="connection-row" key={img.id}><div className="connection-row__head"><strong>{img.repository}:{img.tag}</strong><span className="connection-pill">{img.size}</span></div><div className="connection-row__meta">{img.id.slice(0, 12)} · {img.created}</div></div>
              )) : <div className="empty-note">{t("No images found.")}</div>
            ) : activeTab === "volumes" ? (
              state.volumes.length > 0 ? state.volumes.map((v) => (
                <div className="connection-row" key={v.name}><div className="connection-row__head"><strong>{v.name}</strong><span className="connection-pill">{v.driver}</span></div><div className="connection-row__meta">{v.mountpoint}</div></div>
              )) : <div className="empty-note">{t("No volumes found.")}</div>
            ) : (
              state.networks.length > 0 ? state.networks.map((n) => (
                <div className="connection-row" key={n.id}><div className="connection-row__head"><strong>{n.name}</strong><span className="connection-pill">{n.driver}</span></div><div className="connection-row__meta">{n.scope} · {n.id.slice(0, 12)}</div></div>
              )) : <div className="empty-note">{t("No networks found.")}</div>
            )}
          </section>
        </>
      )}
    </div>
  );
}
