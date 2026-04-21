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
import { localizeError, localizeRuntimeMessage } from "../i18n/localizeMessage";
import DbConnRow from "../components/DbConnRow";
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

/**
 * Condense "CPU% · Mem" into one tight string for the list cell.
 *
 * `docker stats --format '{{.MemUsage}}'` returns `"used / limit"` (e.g.
 * `"48.5MiB / 1.94GiB"`). The list is too narrow for both halves, so we
 * keep the "used" portion and drop the limit — the full string is still
 * available in the inspect pane.
 */
function formatCpuMem(cpuPerc: string, memUsage: string): string {
  const cpu = cpuPerc.trim();
  const memUsed = memUsage.split("/")[0]?.trim() ?? "";
  if (!cpu && !memUsed) return "—";
  if (cpu && memUsed) return `${cpu} · ${memUsed}`;
  return cpu || memUsed;
}

type VolumeSort = "size" | "name" | "links";

function sortArrow(active: boolean, desc: boolean): string {
  if (!active) return "";
  return desc ? " ↓" : " ↑";
}

export default function DockerPanel({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
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
  const [volumeSort, setVolumeSort] = useState<VolumeSort>("size");
  const [volumeSortDesc, setVolumeSortDesc] = useState(true);

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
      setError(formatError(e));
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
      setNotice(`${shortId(id)}: ${localizeRuntimeMessage(result, t)}`);
      await refresh();
    } catch (e) {
      setError(formatError(e));
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
      setError(formatError(e));
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
      setError(formatError(e));
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
      setError(formatError(e));
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
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  function toggleVolumeSort(col: VolumeSort) {
    if (volumeSort === col) {
      setVolumeSortDesc((prev) => !prev);
    } else {
      setVolumeSort(col);
      // New column: size + links default to descending (big-first feels
      // right for both), name defaults to ascending.
      setVolumeSortDesc(col !== "name");
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
    const filtered = !n
      ? state.volumes
      : state.volumes.filter((v) =>
          v.name.toLowerCase().includes(n) || v.driver.toLowerCase().includes(n),
        );
    // Backend already sorts size-desc; re-sort only when the user picked
    // a different column or flipped the direction.
    const sorted = [...filtered];
    sorted.sort((a, b) => {
      let cmp = 0;
      if (volumeSort === "size") cmp = a.sizeBytes - b.sizeBytes;
      else if (volumeSort === "links") cmp = a.links - b.links;
      else cmp = a.name.localeCompare(b.name);
      if (cmp === 0) cmp = a.name.localeCompare(b.name);
      return volumeSortDesc ? -cmp : cmp;
    });
    return sorted;
  }, [state, search, volumeSort, volumeSortDesc]);

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
  const headerMeta = state
    ? t("{host} · {count} containers", { host: hostLabel, count: state.containers.length })
    : hostLabel;
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
      <PanelHeader icon={ContainerIcon} title={t("Docker")} meta={headerMeta} />
      <div className="dk">
        <DbConnRow
          icon={ContainerIcon}
          tint="var(--pos-dim)"
          iconTint="var(--pos)"
          name={hostLabel}
          sub={hostSub}
          tag={
            <>
              <StatusDot tone={state ? "pos" : "off"} />
              {state ? t("ready") : t("offline")}
            </>
          }
        />

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
            <div className="dk-scroll">
              <table className="dk-table">
                <thead>
                  <tr>
                    <th style={{ width: 20 }} />
                    <th>{t("NAME")}</th>
                    <th>{t("IMAGE")}</th>
                    <th style={{ width: 140 }}>{t("STATUS")}</th>
                    <th style={{ width: 88 }} className="text-right">{t("CPU/MEM")}</th>
                    <th style={{ width: 36 }} />
                  </tr>
                </thead>
                <tbody>
                  {filteredContainers.length === 0 ? (
                    <tr>
                      <td colSpan={6} className="dk-empty">{t("No containers found.")}</td>
                    </tr>
                  ) : (
                    filteredContainers.map((c) => {
                      const ds = dotState(c.state, c.running);
                      const badgeTone = ds === "running" ? "is-pos" : ds === "restarting" ? "is-warn" : "is-muted";
                      const isSel = c.id === selectedContainer;
                      return (
                        <tr
                          key={c.id}
                          className={isSel ? "selected" : undefined}
                          onClick={() => setSelectedContainer(c.id)}
                        >
                          <td><span className={"dk-dot " + ds} /></td>
                          <td className="mono">{c.names || shortId(c.id)}</td>
                          <td className="mono text-muted truncate">{c.image}</td>
                          <td>
                            <span className={"db-badge " + badgeTone}>{t(c.state)}</span>
                            {c.status && (
                              <span className="mono text-muted dk-td-sub">{c.status}</span>
                            )}
                          </td>
                          <td className="mono text-right text-muted">
                            {formatCpuMem(c.cpuPerc, c.memUsage)}
                          </td>
                          <td>
                            <button
                              className="mini-btn"
                              type="button"
                              title={t("Logs")}
                              onClick={(e) => {
                                e.stopPropagation();
                                openContainerLogs(c.id);
                              }}
                            >
                              <Scroll size={11} />
                            </button>
                          </td>
                        </tr>
                      );
                    })
                  )}
                </tbody>
              </table>
            </div>

            {selectedCtr && (
              <div className="dk-insp">
                <div className="dk-insp-head">
                  <span className="mono dk-insp-name">{selectedCtr.names || shortId(selectedCtr.id)}</span>
                  <span
                    className={
                      "db-badge " +
                      (dotState(selectedCtr.state, selectedCtr.running) === "running"
                        ? "is-pos"
                        : dotState(selectedCtr.state, selectedCtr.running) === "restarting"
                          ? "is-warn"
                          : "is-muted")
                    }
                  >
                    {t(selectedCtr.state)}
                  </span>
                  <div style={{ flex: 1 }} />
                  {selectedCtr.running ? (
                    <>
                      <button className="mini-btn" type="button" title={t("Stop")} disabled={actionBusy}
                        onClick={() => void containerAction(selectedCtr.id, "stop")}>
                        <ArrowDown size={11} />
                      </button>
                      <button className="mini-btn" type="button" title={t("Restart")} disabled={actionBusy}
                        onClick={() => void containerAction(selectedCtr.id, "restart")}>
                        <RefreshCw size={11} />
                      </button>
                    </>
                  ) : (
                    <>
                      <button className="mini-btn" type="button" title={t("Start")} disabled={actionBusy}
                        onClick={() => void containerAction(selectedCtr.id, "start")}>
                        <ArrowUp size={11} />
                      </button>
                      <button className="mini-btn is-destructive" type="button" title={t("Remove")} disabled={actionBusy}
                        onClick={() => void containerAction(selectedCtr.id, "remove")}>
                        <Trash2 size={11} />
                      </button>
                    </>
                  )}
                  <button className="mini-btn" type="button" title={t("Logs")}
                    onClick={() => openContainerLogs(selectedCtr.id)}>
                    <Scroll size={11} />
                  </button>
                  {hasSsh && (
                    <button className="mini-btn" type="button" title={t("Inspect")} disabled={actionBusy}
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
                  {selectedCtr.ports && (
                    <div className="dk-kv">
                      <span className="dk-kv-k">{t("Ports")}</span>
                      <span className="dk-kv-v mono">{selectedCtr.ports}</span>
                    </div>
                  )}
                  {(selectedCtr.cpuPerc || selectedCtr.memUsage) && (
                    <>
                      <div className="dk-kv">
                        <span className="dk-kv-k">{t("CPU")}</span>
                        <span className="dk-kv-v mono">{selectedCtr.cpuPerc || "—"}</span>
                      </div>
                      <div className="dk-kv">
                        <span className="dk-kv-k">{t("Memory")}</span>
                        <span className="dk-kv-v mono">
                          {selectedCtr.memUsage || "—"}
                          {selectedCtr.memPerc && (
                            <span className="text-muted"> · {selectedCtr.memPerc}</span>
                          )}
                        </span>
                      </div>
                    </>
                  )}
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
            <div className="dk-scroll">
              <table className="dk-table">
                <thead>
                  <tr>
                    <th>{t("REPOSITORY")}</th>
                    <th style={{ width: 120 }}>{t("TAG")}</th>
                    <th style={{ width: 72 }} className="text-right">{t("SIZE")}</th>
                    <th style={{ width: 72 }}>{t("AGE")}</th>
                    <th style={{ width: 36 }} />
                  </tr>
                </thead>
                <tbody>
                  {filteredImages.length === 0 ? (
                    <tr>
                      <td colSpan={5} className="dk-empty">{t("No images found.")}</td>
                    </tr>
                  ) : (
                    filteredImages.map((img) => {
                      const isSel = img.id === selectedImage;
                      return (
                        <tr
                          key={img.id}
                          className={isSel ? "selected" : undefined}
                          onClick={() => setSelectedImage(img.id)}
                        >
                          <td className="mono">{img.repository}</td>
                          <td className="mono text-muted">{img.tag}</td>
                          <td className="mono text-right">{img.size}</td>
                          <td className="mono text-muted">{img.created}</td>
                          <td>
                            {hasSsh ? (
                              <button
                                className="mini-btn is-destructive"
                                type="button"
                                title={t("Remove")}
                                disabled={actionBusy}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void removeImage(img.id);
                                }}
                              >
                                <Trash2 size={11} />
                              </button>
                            ) : (
                              <span className="dk-img-ic" aria-hidden>
                                <HardDrive size={11} />
                              </span>
                            )}
                          </td>
                        </tr>
                      );
                    })
                  )}
                </tbody>
              </table>
            </div>
          </div>
        )}

        {state && activeTab === "volumes" && (
          <div className="dk-body">
            <div className="dk-scroll">
              <table className="dk-table">
                <thead>
                  <tr>
                    <th>
                      <button
                        type="button"
                        className={"dk-sort" + (volumeSort === "name" ? " active" : "")}
                        onClick={() => toggleVolumeSort("name")}
                      >
                        {t("NAME")}{sortArrow(volumeSort === "name", volumeSortDesc)}
                      </button>
                    </th>
                    <th style={{ width: 100 }}>{t("DRIVER")}</th>
                    <th style={{ width: 64 }} className="text-right">
                      <button
                        type="button"
                        className={"dk-sort" + (volumeSort === "links" ? " active" : "")}
                        onClick={() => toggleVolumeSort("links")}
                      >
                        {t("USED BY")}{sortArrow(volumeSort === "links", volumeSortDesc)}
                      </button>
                    </th>
                    <th style={{ width: 80 }} className="text-right">
                      <button
                        type="button"
                        className={"dk-sort" + (volumeSort === "size" ? " active" : "")}
                        onClick={() => toggleVolumeSort("size")}
                      >
                        {t("SIZE")}{sortArrow(volumeSort === "size", volumeSortDesc)}
                      </button>
                    </th>
                    <th>{t("MOUNTPOINT")}</th>
                    <th style={{ width: 36 }} />
                  </tr>
                </thead>
                <tbody>
                  {filteredVolumes.length === 0 ? (
                    <tr>
                      <td colSpan={6} className="dk-empty">{t("No volumes found.")}</td>
                    </tr>
                  ) : (
                    filteredVolumes.map((v) => {
                      const isSel = v.name === selectedVolume;
                      return (
                        <tr
                          key={v.name}
                          className={isSel ? "selected" : undefined}
                          onClick={() => setSelectedVolume(v.name)}
                        >
                          <td className="mono">{v.name}</td>
                          <td className="mono text-muted">{v.driver}</td>
                          <td className="mono text-right text-muted">
                            {v.links >= 0 ? v.links : "—"}
                          </td>
                          <td className="mono text-right">{v.size || "—"}</td>
                          <td className="mono text-muted truncate">{v.mountpoint}</td>
                          <td>
                            {hasSsh ? (
                              <button
                                className="mini-btn is-destructive"
                                type="button"
                                title={t("Remove")}
                                disabled={actionBusy}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void removeVolume(v.name);
                                }}
                              >
                                <Trash2 size={11} />
                              </button>
                            ) : (
                              <span className="dk-img-ic" aria-hidden>
                                <Folder size={11} />
                              </span>
                            )}
                          </td>
                        </tr>
                      );
                    })
                  )}
                </tbody>
              </table>
            </div>
          </div>
        )}

        {state && activeTab === "networks" && (
          <div className="dk-body">
            <div className="dk-scroll">
              <table className="dk-table">
                <thead>
                  <tr>
                    <th>{t("NAME")}</th>
                    <th style={{ width: 100 }}>{t("DRIVER")}</th>
                    <th style={{ width: 80 }}>{t("SCOPE")}</th>
                    <th>{t("ID")}</th>
                    <th style={{ width: 36 }} />
                  </tr>
                </thead>
                <tbody>
                  {filteredNetworks.length === 0 ? (
                    <tr>
                      <td colSpan={5} className="dk-empty">{t("No networks found.")}</td>
                    </tr>
                  ) : (
                    filteredNetworks.map((n) => {
                      const isSel = n.id === selectedNetwork;
                      return (
                        <tr
                          key={n.id}
                          className={isSel ? "selected" : undefined}
                          onClick={() => setSelectedNetwork(n.id)}
                        >
                          <td className="mono">{n.name}</td>
                          <td className="mono text-muted">{n.driver}</td>
                          <td className="mono text-muted">{n.scope}</td>
                          <td className="mono text-muted">{shortId(n.id)}</td>
                          <td>
                            {hasSsh ? (
                              <button
                                className="mini-btn is-destructive"
                                type="button"
                                title={t("Remove")}
                                disabled={actionBusy}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void removeNetwork(n.name);
                                }}
                              >
                                <Trash2 size={11} />
                              </button>
                            ) : (
                              <span className="dk-img-ic" aria-hidden>
                                <Network size={11} />
                              </span>
                            )}
                          </td>
                        </tr>
                      );
                    })
                  )}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </div>
    </>
  );
}
