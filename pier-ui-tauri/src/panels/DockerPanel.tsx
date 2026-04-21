import {
  ArrowDown,
  ArrowUp,
  Container as ContainerIcon,
  FileText,
  Folder,
  HardDrive,
  Network,
  RefreshCw,
  Scroll,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { useMemo, useState } from "react";
import * as cmd from "../lib/commands";
import { quoteCommandArg } from "../lib/commands";
import type { DockerOverview, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";
import { useTabStore } from "../stores/useTabStore";

type Props = { tab: TabState };

type DkTab = "containers" | "images" | "volumes" | "networks";

function shortId(id: string): string {
  if (!id) return "";
  const stripped = id.replace(/^sha256:/, "");
  return stripped.slice(0, 12);
}

function dotState(state: string, running: boolean): "running" | "restarting" | "exited" {
  const s = state.toLowerCase();
  if (s.includes("restart")) return "restarting";
  if (running) return "running";
  return "exited";
}

export default function DockerPanel({ tab }: Props) {
  const { t } = useI18n();
  const updateTab = useTabStore((s) => s.updateTab);
  const setTabRightTool = useTabStore((s) => s.setTabRightTool);
  const [state, setState] = useState<DockerOverview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [showAll, setShowAll] = useState(true);
  const [activeTab, setActiveTab] = useState<DkTab>("containers");
  const [actionBusy, setActionBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [inspectJson, setInspectJson] = useState("");
  const [search, setSearch] = useState("");
  const [selectedContainer, setSelectedContainer] = useState<string>("");
  const [selectedImage, setSelectedImage] = useState<string>("");
  const [selectedVolume, setSelectedVolume] = useState<string>("");
  const [selectedNetwork, setSelectedNetwork] = useState<string>("");

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();
  const isLocal = tab.backend === "local";
  const canRefresh = isLocal || hasSsh;

  async function refresh() {
    setBusy(true);
    setError("");
    setNotice("");
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
      if (!selectedContainer && overview.containers.length) {
        setSelectedContainer(overview.containers[0].id);
      }
    } catch (e) {
      setState(null);
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function containerAction(id: string, action: string) {
    if (actionBusy) return;
    setActionBusy(true);
    setNotice("");
    setError("");
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
      setNotice(`${shortId(id)}: ${result}`);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function inspectContainer(id: string) {
    if (!hasSsh || actionBusy) return;
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
      setNotice(t("Loaded container inspection for {id}.", { id: shortId(id) }));
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function removeImage(id: string) {
    if (!hasSsh || actionBusy) return;
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
      setNotice(t("Removed image {id}.", { id: shortId(id) }));
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function removeVolume(name: string) {
    if (!hasSsh || actionBusy) return;
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
    if (!hasSsh || actionBusy) return;
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
    updateTab(tab.id, { logCommand: `docker logs -f ${quoteCommandArg(id)}` });
    setTabRightTool(tab.id, "log");
  }

  const filteredContainers = useMemo(() => {
    const n = search.trim().toLowerCase();
    if (!state) return [];
    if (!n) return state.containers;
    return state.containers.filter((c) =>
      c.names.toLowerCase().includes(n) || c.image.toLowerCase().includes(n) || c.id.toLowerCase().includes(n),
    );
  }, [state, search]);

  const filteredImages = useMemo(() => {
    const n = search.trim().toLowerCase();
    if (!state) return [];
    if (!n) return state.images;
    return state.images.filter((i) =>
      `${i.repository}:${i.tag}`.toLowerCase().includes(n) || i.id.toLowerCase().includes(n),
    );
  }, [state, search]);

  const filteredVolumes = useMemo(() => {
    const n = search.trim().toLowerCase();
    if (!state) return [];
    if (!n) return state.volumes;
    return state.volumes.filter((v) =>
      v.name.toLowerCase().includes(n) || v.driver.toLowerCase().includes(n),
    );
  }, [state, search]);

  const filteredNetworks = useMemo(() => {
    const n = search.trim().toLowerCase();
    if (!state) return [];
    if (!n) return state.networks;
    return state.networks.filter((x) =>
      x.name.toLowerCase().includes(n) || x.driver.toLowerCase().includes(n),
    );
  }, [state, search]);

  const selectedCtr = useMemo(
    () => state?.containers.find((c) => c.id === selectedContainer) ?? null,
    [state, selectedContainer],
  );

  const hostLabel = hasSsh ? tab.sshHost : isLocal ? t("local") : "—";
  const headerMeta = state ? `${hostLabel} · ${state.containers.length} containers` : hostLabel;
  const hostSub = hasSsh
    ? `${tab.sshUser}@${tab.sshHost}:${tab.sshPort} · ${t("remote via SSH")}`
    : isLocal
      ? t("Local Docker socket")
      : t("Not connected");

  const tabCounts: Record<DkTab, number> = {
    containers: state?.containers.length ?? 0,
    images: state?.images.length ?? 0,
    volumes: state?.volumes.length ?? 0,
    networks: state?.networks.length ?? 0,
  };

  return (
    <>
      <PanelHeader icon={ContainerIcon} title="DOCKER" meta={headerMeta} />
      <div className="dk">
        <div className="dk-host">
          <span className="dk-host-ic"><ContainerIcon size={13} /></span>
          <div className="dk-host-body">
            <div className="dk-host-name">{hostLabel}</div>
            <div className="dk-host-sub mono">{hostSub}</div>
          </div>
          <span className={"dk-host-tag mono" + (state ? "" : " off")}>
            <StatusDot tone={state ? "pos" : "off"} />
            {state ? t("ready") : t("offline")}
          </span>
        </div>

        <div className="dk-tabs">
          {(["containers", "images", "volumes", "networks"] as DkTab[]).map((k) => (
            <button
              key={k}
              type="button"
              className={"dk-tab" + (activeTab === k ? " active" : "")}
              onClick={() => setActiveTab(k)}
            >
              {t(k.charAt(0).toUpperCase() + k.slice(1))}
              {state ? <span className="dk-tab-count">{tabCounts[k]}</span> : null}
            </button>
          ))}
        </div>

        <div className="dk-primary">
          <label className="dk-check mono">
            <input type="checkbox" checked={showAll} onChange={() => setShowAll((v) => !v)} />
            {t("all")}
          </label>
          <div className="dk-search">
            <Search size={10} />
            <input
              placeholder={
                activeTab === "containers"
                  ? t("Filter containers…")
                  : activeTab === "images"
                    ? t("repo:tag…")
                    : activeTab === "volumes"
                      ? t("Filter volumes…")
                      : t("Filter networks…")
              }
              value={search}
              onChange={(e) => setSearch(e.currentTarget.value)}
            />
            {search && (
              <button className="lg-x" type="button" onClick={() => setSearch("")}>
                <X size={10} />
              </button>
            )}
          </div>
          <button className="dk-ic" type="button" title={t("Refresh")} disabled={!canRefresh || busy} onClick={() => void refresh()}>
            <RefreshCw size={11} />
          </button>
        </div>

        {!canRefresh && <div className="lg-note">{t("SSH connection required for Docker.")}</div>}
        {canRefresh && !state && !busy && (
          <div className="lg-note">
            <button type="button" className="btn is-primary is-compact" onClick={() => void refresh()}>
              {t("Refresh Docker")}
            </button>
          </div>
        )}
        {busy && <div className="lg-note">{t("Loading...")}</div>}
        {notice && <div className="lg-note">{notice}</div>}
        {error && <div className="lg-note lg-note--error">{error}</div>}

        {state && activeTab === "containers" && (
          <div className="dk-body">
            <div className="dk-col-head">
              <span className="c-stat" />
              <span className="c-name">{t("NAME")}</span>
              <span className="c-status">{t("STATUS")}</span>
              <span className="c-more" />
            </div>
            <div className="dk-list">
              {filteredContainers.length === 0 ? (
                <div className="dk-empty">{t("No containers found.")}</div>
              ) : (
                filteredContainers.map((c) => {
                  const ds = dotState(c.state, c.running);
                  const isSel = c.id === selectedContainer;
                  return (
                    <div
                      key={c.id}
                      className={"dk-ctr" + (isSel ? " selected" : "")}
                      onClick={() => setSelectedContainer(c.id)}
                    >
                      <span className={"dk-dot " + ds} />
                      <div className="c-name">
                        <div className="dk-ctr-name mono">{c.names || shortId(c.id)}</div>
                        <div className="dk-ctr-img mono">{c.image}</div>
                      </div>
                      <div className="c-status">
                        <span className={"dk-chip " + ds}>{c.state}</span>
                      </div>
                      <div className="c-more" />
                    </div>
                  );
                })
              )}
            </div>

            {selectedCtr && (
              <div className="dk-insp">
                <div className="dk-insp-head">
                  <span className="mono dk-insp-name">{selectedCtr.names || shortId(selectedCtr.id)}</span>
                  <span className={"dk-chip " + dotState(selectedCtr.state, selectedCtr.running)}>
                    {selectedCtr.state}
                  </span>
                  <div style={{ flex: 1 }} />
                  {selectedCtr.running ? (
                    <>
                      <button className="lg-ic" type="button" title={t("Stop")} disabled={actionBusy}
                        onClick={() => void containerAction(selectedCtr.id, "stop")}>
                        <ArrowDown size={11} />
                      </button>
                      <button className="lg-ic" type="button" title={t("Restart")} disabled={actionBusy}
                        onClick={() => void containerAction(selectedCtr.id, "restart")}>
                        <RefreshCw size={11} />
                      </button>
                    </>
                  ) : (
                    <>
                      <button className="lg-ic" type="button" title={t("Start")} disabled={actionBusy}
                        onClick={() => void containerAction(selectedCtr.id, "start")}>
                        <ArrowUp size={11} />
                      </button>
                      <button className="lg-ic" type="button" title={t("Remove")} disabled={actionBusy}
                        onClick={() => void containerAction(selectedCtr.id, "remove")}>
                        <Trash2 size={11} />
                      </button>
                    </>
                  )}
                  <button className="lg-ic" type="button" title={t("Logs")}
                    onClick={() => openContainerLogs(selectedCtr.id)}>
                    <Scroll size={11} />
                  </button>
                  {hasSsh && (
                    <button className="lg-ic" type="button" title={t("Inspect")} disabled={actionBusy}
                      onClick={() => void inspectContainer(selectedCtr.id)}>
                      <FileText size={11} />
                    </button>
                  )}
                </div>
                <div className="dk-insp-body">
                  <div className="dk-kv">
                    <span className="dk-kv-k">{t("Image")}</span>
                    <span className="dk-kv-v mono">{selectedCtr.image}</span>
                  </div>
                  <div className="dk-kv">
                    <span className="dk-kv-k">{t("ID")}</span>
                    <span className="dk-kv-v mono">{shortId(selectedCtr.id)}</span>
                  </div>
                  <div className="dk-kv">
                    <span className="dk-kv-k">{t("Status")}</span>
                    <span className="dk-kv-v">{selectedCtr.status}</span>
                  </div>
                </div>
                {inspectJson && (
                  <div className="dk-insp-body">
                    <div className="dk-sub-h">{t("Inspect Output")}</div>
                    <pre className="dk-inspect-pre mono">{inspectJson}</pre>
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        {state && activeTab === "images" && (
          <div className="dk-body">
            <div className="dk-col-head dk-col-head--images">
              <span className="i-repo">{t("REPOSITORY · TAG")}</span>
              <span className="i-size">{t("SIZE")}</span>
              <span className="i-age">{t("AGE")}</span>
              <span className="c-more" />
            </div>
            <div className="dk-list">
              {filteredImages.length === 0 ? (
                <div className="dk-empty">{t("No images found.")}</div>
              ) : (
                filteredImages.map((img) => {
                  const isSel = img.id === selectedImage;
                  return (
                    <div
                      key={img.id}
                      className={"dk-img" + (isSel ? " selected" : "")}
                      onClick={() => setSelectedImage(img.id)}
                    >
                      <span className="dk-img-ic"><HardDrive size={11} /></span>
                      <div className="i-repo">
                        <div className="mono dk-img-repo">
                          {img.repository}<span className="text-muted">:</span>
                          <span className="c-accent">{img.tag}</span>
                        </div>
                        <div className="mono dk-img-id">{shortId(img.id)}</div>
                      </div>
                      <span className="i-size mono">{img.size}</span>
                      <span className="i-age mono text-muted">{img.created}</span>
                      {hasSsh && (
                        <button
                          className="dk-row-more"
                          type="button"
                          onClick={(e) => { e.stopPropagation(); void removeImage(img.id); }}
                          title={t("Remove")}
                          disabled={actionBusy}
                        >
                          <Trash2 size={11} />
                        </button>
                      )}
                    </div>
                  );
                })
              )}
            </div>
          </div>
        )}

        {state && activeTab === "volumes" && (
          <div className="dk-body">
            <div className="dk-col-head dk-col-head--volumes">
              <span className="v-name">{t("NAME")}</span>
              <span className="v-driver">{t("DRIVER")}</span>
              <span className="c-more" />
            </div>
            <div className="dk-list">
              {filteredVolumes.length === 0 ? (
                <div className="dk-empty">{t("No volumes found.")}</div>
              ) : (
                filteredVolumes.map((v) => {
                  const isSel = v.name === selectedVolume;
                  return (
                    <div
                      key={v.name}
                      className={"dk-vol" + (isSel ? " selected" : "")}
                      onClick={() => setSelectedVolume(v.name)}
                    >
                      <span className="dk-img-ic"><Folder size={11} /></span>
                      <div className="v-name">
                        <div className="mono dk-img-repo">{v.name}</div>
                        <div className="mono dk-img-id">{v.mountpoint}</div>
                      </div>
                      <span className="v-driver mono text-muted">{v.driver}</span>
                      {hasSsh && (
                        <button
                          className="dk-row-more"
                          type="button"
                          onClick={(e) => { e.stopPropagation(); void removeVolume(v.name); }}
                          title={t("Remove")}
                          disabled={actionBusy}
                        >
                          <Trash2 size={11} />
                        </button>
                      )}
                    </div>
                  );
                })
              )}
            </div>
          </div>
        )}

        {state && activeTab === "networks" && (
          <div className="dk-body">
            <div className="dk-col-head dk-col-head--networks">
              <span className="n-name">{t("NAME")}</span>
              <span className="n-driver">{t("DRIVER")}</span>
              <span className="n-scope">{t("SCOPE")}</span>
              <span className="c-more" />
            </div>
            <div className="dk-list">
              {filteredNetworks.length === 0 ? (
                <div className="dk-empty">{t("No networks found.")}</div>
              ) : (
                filteredNetworks.map((n) => {
                  const isSel = n.id === selectedNetwork;
                  return (
                    <div
                      key={n.id}
                      className={"dk-vol" + (isSel ? " selected" : "")}
                      onClick={() => setSelectedNetwork(n.id)}
                    >
                      <span className="dk-img-ic"><Network size={11} /></span>
                      <div className="n-name">
                        <div className="mono dk-img-repo">{n.name}</div>
                        <div className="mono dk-img-id">{shortId(n.id)}</div>
                      </div>
                      <span className="n-driver mono text-muted">{n.driver}</span>
                      <span className="n-scope mono text-muted">{n.scope}</span>
                      {hasSsh && (
                        <button
                          className="dk-row-more"
                          type="button"
                          onClick={(e) => { e.stopPropagation(); void removeNetwork(n.name); }}
                          title={t("Remove")}
                          disabled={actionBusy}
                        >
                          <Trash2 size={11} />
                        </button>
                      )}
                    </div>
                  );
                })
              )}
            </div>
          </div>
        )}
      </div>
    </>
  );
}
