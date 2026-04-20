import { Container } from "lucide-react";
import { useState } from "react";
import * as cmd from "../lib/commands";
import { quoteCommandArg } from "../lib/commands";
import type { DockerOverview, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import DbConnRow from "../components/DbConnRow";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";
import { useTabStore } from "../stores/useTabStore";

type Props = { tab: TabState };

export default function DockerPanel({ tab }: Props) {
  const { t } = useI18n();
  const updateTab = useTabStore((s) => s.updateTab);
  const setTabRightTool = useTabStore((s) => s.setTabRightTool);
  const [state, setState] = useState<DockerOverview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [showAll, setShowAll] = useState(true);
  const [activeTab, setActiveTab] = useState<"containers" | "images" | "volumes" | "networks">("containers");
  const [actionBusy, setActionBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [inspectJson, setInspectJson] = useState("");

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();
  const isLocal = tab.backend === "local";

  async function refresh() {
    setBusy(true);
    setError("");
    try {
      const overview = isLocal
        ? await cmd.localDockerOverview(showAll)
        : hasSsh
          ? await cmd.dockerOverview({
              host: tab.sshHost,
              port: tab.sshPort,
              user: tab.sshUser,
              authMode: tab.sshAuthMode,
              password: tab.sshPassword,
              keyPath: tab.sshKeyPath,
              all: showAll,
            })
          : null;
      if (!overview) {
        setError(t("No connection available."));
        return;
      }
      setState(overview);
    } catch (e) {
      setState(null);
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function containerAction(id: string, action: string) {
    if (actionBusy) {
      return;
    }
    setActionBusy(true);
    setNotice("");
    try {
      const result = isLocal
        ? await cmd.localDockerAction(id, action)
        : await cmd.dockerContainerAction({
            host: tab.sshHost,
            port: tab.sshPort,
            user: tab.sshUser,
            authMode: tab.sshAuthMode,
            password: tab.sshPassword,
            keyPath: tab.sshKeyPath,
            containerId: id,
            action,
          });
      setNotice(`${id.slice(0, 12)}: ${result}`);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function inspectContainer(id: string) {
    if (!hasSsh || actionBusy) {
      return;
    }
    setActionBusy(true);
    setError("");
    try {
      const output = await cmd.dockerInspect({
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        containerId: id,
      });
      setInspectJson(output);
      setNotice(t("Loaded container inspection for {id}.", { id: id.slice(0, 12) }));
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function removeImage(id: string) {
    if (!hasSsh || actionBusy) {
      return;
    }
    setActionBusy(true);
    setError("");
    try {
      await cmd.dockerRemoveImage({
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        imageId: id,
        force: false,
      });
      setNotice(t("Removed image {id}.", { id: id.slice(0, 12) }));
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function removeVolume(name: string) {
    if (!hasSsh || actionBusy) {
      return;
    }
    setActionBusy(true);
    setError("");
    try {
      await cmd.dockerRemoveVolume({
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        volumeName: name,
      });
      setNotice(t("Removed volume {name}.", { name }));
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function removeNetwork(name: string) {
    if (!hasSsh || actionBusy) {
      return;
    }
    setActionBusy(true);
    setError("");
    try {
      await cmd.dockerRemoveNetwork({
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        networkName: name,
      });
      setNotice(t("Removed network {name}.", { name }));
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  function openContainerLogs(id: string) {
    if (!hasSsh) {
      return;
    }
    updateTab(tab.id, {
      logCommand: `docker logs -f ${quoteCommandArg(id)}`,
    });
    setTabRightTool(tab.id, "log");
  }

  const canRefresh = isLocal || hasSsh;

  const headerMeta = hasSsh ? tab.sshHost : isLocal ? "local" : "—";
  const connName = hasSsh
    ? `${tab.sshUser}@${tab.sshHost}`
    : isLocal
      ? t("Local Docker")
      : t("Docker");
  const connSub = hasSsh ? `port ${tab.sshPort}` : isLocal ? t("Local socket") : t("Not connected");
  const connTag = (
    <>
      <StatusDot tone={state ? "pos" : "off"} />
      {state ? t("ready") : t("offline")}
    </>
  );

  return (
    <>
      <PanelHeader
        icon={Container}
        title="DOCKER"
        meta={headerMeta}
      />
      <DbConnRow
        icon={Container}
        tint="var(--pos-dim)"
        iconTint="var(--pos)"
        name={connName}
        sub={connSub}
        tag={connTag}
      />
      <div className="panel-scroll">
      <section className="panel-section">
        <div className="form-stack">
          <div className="button-row">
            <button className="mini-button" disabled={!canRefresh || busy} onClick={() => void refresh()} type="button">{busy ? t("Loading...") : t("Refresh Docker")}</button>
            <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text-secondary)" }}>
              <input type="checkbox" checked={showAll} onChange={() => setShowAll((current) => !current)} />{t("Show all containers")}
            </label>
          </div>
          {!canRefresh && <div className="inline-note">{t("SSH connection required for Docker.")}</div>}
          {hasSsh && <div className="inline-note">{t("Remote Docker includes inspect, cleanup, and log streaming shortcuts.")}</div>}
          {notice && <div className="status-note">{notice}</div>}
          {error && <div className="status-note status-note--error">{error}</div>}
        </div>
      </section>

      {state && (
        <>
          <div className="surface-switcher" style={{ padding: "0 12px 8px" }}>
            {(["containers", "images", "volumes", "networks"] as const).map((name) => (
              <button key={name} className={activeTab === name ? "surface-button surface-button--selected" : "surface-button"} onClick={() => setActiveTab(name)} type="button">{t(name.charAt(0).toUpperCase() + name.slice(1))}</button>
            ))}
          </div>

          <section className="panel-section">
            {activeTab === "containers" ? (
              state.containers.length > 0 ? state.containers.map((container) => (
                <div className="connection-row" key={container.id}>
                  <div className="connection-row__head"><strong>{container.names || container.id.slice(0, 12)}</strong><span className="connection-pill">{container.state}</span></div>
                  <div className="connection-row__meta">{container.image} · {container.status}</div>
                  <div className="connection-row__actions">
                    {container.running ? (
                      <>
                        <button className="mini-button" disabled={actionBusy} onClick={() => void containerAction(container.id, "stop")} type="button">{t("Stop")}</button>
                        <button className="mini-button" disabled={actionBusy} onClick={() => void containerAction(container.id, "restart")} type="button">{t("Restart")}</button>
                      </>
                    ) : (
                      <>
                        <button className="mini-button" disabled={actionBusy} onClick={() => void containerAction(container.id, "start")} type="button">{t("Start")}</button>
                        <button className="mini-button" disabled={actionBusy} onClick={() => void containerAction(container.id, "remove")} type="button">{t("Remove")}</button>
                      </>
                    )}
                    {hasSsh && (
                      <>
                        <button className="mini-button" disabled={actionBusy} onClick={() => void inspectContainer(container.id)} type="button">{t("Inspect")}</button>
                        <button className="mini-button" onClick={() => openContainerLogs(container.id)} type="button">{t("Logs")}</button>
                      </>
                    )}
                  </div>
                </div>
              )) : <div className="empty-note">{t("No containers found.")}</div>
            ) : activeTab === "images" ? (
              state.images.length > 0 ? state.images.map((image) => (
                <div className="connection-row" key={image.id}>
                  <div className="connection-row__head"><strong>{image.repository}:{image.tag}</strong><span className="connection-pill">{image.size}</span></div>
                  <div className="connection-row__meta">{image.id.slice(0, 12)} · {image.created}</div>
                  {hasSsh && (
                    <div className="connection-row__actions">
                      <button className="mini-button mini-button--destructive" disabled={actionBusy} onClick={() => void removeImage(image.id)} type="button">{t("Remove")}</button>
                    </div>
                  )}
                </div>
              )) : <div className="empty-note">{t("No images found.")}</div>
            ) : activeTab === "volumes" ? (
              state.volumes.length > 0 ? state.volumes.map((volume) => (
                <div className="connection-row" key={volume.name}>
                  <div className="connection-row__head"><strong>{volume.name}</strong><span className="connection-pill">{volume.driver}</span></div>
                  <div className="connection-row__meta">{volume.mountpoint}</div>
                  {hasSsh && (
                    <div className="connection-row__actions">
                      <button className="mini-button mini-button--destructive" disabled={actionBusy} onClick={() => void removeVolume(volume.name)} type="button">{t("Remove")}</button>
                    </div>
                  )}
                </div>
              )) : <div className="empty-note">{t("No volumes found.")}</div>
            ) : (
              state.networks.length > 0 ? state.networks.map((network) => (
                <div className="connection-row" key={network.id}>
                  <div className="connection-row__head"><strong>{network.name}</strong><span className="connection-pill">{network.driver}</span></div>
                  <div className="connection-row__meta">{network.scope} · {network.id.slice(0, 12)}</div>
                  {hasSsh && (
                    <div className="connection-row__actions">
                      <button className="mini-button mini-button--destructive" disabled={actionBusy} onClick={() => void removeNetwork(network.name)} type="button">{t("Remove")}</button>
                    </div>
                  )}
                </div>
              )) : <div className="empty-note">{t("No networks found.")}</div>
            )}
          </section>
        </>
      )}

      {inspectJson && (
        <section className="panel-section">
          <div className="panel-section__title"><span>{t("Inspect Output")}</span></div>
          <pre className="diff-viewer">{inspectJson}</pre>
        </section>
      )}
    </div>
    </>
  );
}
