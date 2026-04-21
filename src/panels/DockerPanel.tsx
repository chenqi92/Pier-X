import {
  ChevronDown,
  ChevronRight,
  Container as ContainerIcon,
  Download,
  FileText,
  Folder,
  HardDrive,
  Network,
  Play,
  RefreshCw,
  RotateCw,
  Scroll,
  Search,
  Settings2,
  Sparkles,
  Square,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import * as cmd from "../lib/commands";
import { quoteCommandArg } from "../lib/commands";
import type { DockerOverview, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError, localizeRuntimeMessage } from "../i18n/localizeMessage";
import DbConnRow from "../components/DbConnRow";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";
import RegistryProxyDialog from "../shell/RegistryProxyDialog";
import RunContainerDialog from "../shell/RunContainerDialog";
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
  const [expandedVolume, setExpandedVolume] = useState<string>("");
  const [volumeFiles, setVolumeFiles] = useState<Record<string, string>>({});
  const [volumeFilesBusy, setVolumeFilesBusy] = useState<string>("");
  const [runDialogOpen, setRunDialogOpen] = useState(false);
  const [runDefaultImage, setRunDefaultImage] = useState("");
  const [proxyDialogOpen, setProxyDialogOpen] = useState(false);
  const [pullRef, setPullRef] = useState("");
  const [pullBusy, setPullBusy] = useState(false);
  const [pullLog, setPullLog] = useState("");
  const autoRefreshedRef = useRef(false);

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();
  const isLocal = tab.backend === "local";
  const canRefresh = isLocal || hasSsh;

  async function refresh() {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      // Fast path: base listings only (containers / images / volumes /
      // networks). Render immediately so the panel doesn't sit blank while
      // `docker stats` and `docker system df -v` crawl the daemon.
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
              savedConnectionIndex: tab.sshSavedConnectionIndex,
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

      // SSH: fire the slow enrichment in the background. These do not
      // block the first paint and their errors are swallowed (the rows
      // just keep the placeholder "—").
      if (hasSsh) {
        const sshParams = {
          host: tab.sshHost,
          port: tab.sshPort,
          user: tab.sshUser,
          authMode: tab.sshAuthMode,
          password: tab.sshPassword,
          keyPath: tab.sshKeyPath,
          savedConnectionIndex: tab.sshSavedConnectionIndex,
        };
        void cmd.dockerStats(sshParams).then((stats) => {
          setState((prev) => {
            if (!prev) return prev;
            const byId = new Map(stats.map((s) => [s.id, s]));
            return {
              ...prev,
              containers: prev.containers.map((c) => {
                const s = byId.get(c.id) ?? byId.get(c.id.slice(0, 12));
                return s
                  ? { ...c, cpuPerc: s.cpuPerc, memUsage: s.memUsage, memPerc: s.memPerc }
                  : c;
              }),
            };
          });
        }).catch(() => { /* stats unavailable — leave rows as-is */ });
        void cmd.dockerVolumeUsage(sshParams).then((usages) => {
          setState((prev) => {
            if (!prev) return prev;
            const byName = new Map(usages.map((u) => [u.name, u]));
            const next = prev.volumes.map((v) => {
              const u = byName.get(v.name);
              return u ? { ...v, size: u.size, sizeBytes: u.sizeBytes, links: u.links } : v;
            });
            next.sort((a, b) => b.sizeBytes - a.sizeBytes || a.name.localeCompare(b.name));
            return { ...prev, volumes: next };
          });
        }).catch(() => { /* system df unavailable */ });
      }
    } catch (e) {
      setState(null);
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  // Auto-refresh once the connection is usable so the user sees data on open
  // without having to click "Refresh Docker". Re-arms when the backend shifts
  // (e.g. ssh → local). Subsequent user-triggered refreshes go through the
  // toolbar button.
  useEffect(() => {
    if (!canRefresh) {
      autoRefreshedRef.current = false;
      return;
    }
    if (autoRefreshedRef.current) return;
    autoRefreshedRef.current = true;
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [canRefresh, tab.sshHost, tab.sshUser, tab.sshPort, tab.backend]);

  // "Show stopped containers" toggle should reflect immediately.
  useEffect(() => {
    if (!canRefresh || !autoRefreshedRef.current) return;
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showAll]);

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
            savedConnectionIndex: tab.sshSavedConnectionIndex,
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
        savedConnectionIndex: tab.sshSavedConnectionIndex,
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
        savedConnectionIndex: tab.sshSavedConnectionIndex,
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
        savedConnectionIndex: tab.sshSavedConnectionIndex,
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
        savedConnectionIndex: tab.sshSavedConnectionIndex,
      });
      setNotice(t("Removed network {name}.", { name }));
      await refresh();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  /**
   * Apply the tab's registry mirror to an image ref, but only when the
   * ref doesn't already target an explicit registry. A ref is considered
   * to already name a registry when its first path segment contains a
   * `.` (foo.io/bar), a `:` (localhost:5000/bar), or is `localhost`.
   */
  function applyRegistryMirror(ref: string): string {
    const mirror = tab.dockerRegistryMirror.trim().replace(/\/+$/, "");
    if (!mirror) return ref;
    const first = ref.split("/")[0] ?? "";
    const looksLikeRegistry = first.includes(".") || first.includes(":") || first === "localhost";
    if (looksLikeRegistry) return ref;
    return `${mirror}/${ref}`;
  }

  function pullEnv(): [string, string][] | null {
    const proxy = tab.dockerPullProxy.trim();
    if (!proxy) return null;
    return [
      ["HTTPS_PROXY", proxy],
      ["HTTP_PROXY", proxy],
      ["https_proxy", proxy],
      ["http_proxy", proxy],
    ];
  }

  async function pullImage(ref: string) {
    if (pullBusy || !ref.trim()) return;
    const rewritten = applyRegistryMirror(ref.trim());
    setPullBusy(true);
    setPullLog(t("Pulling {ref}…", { ref: rewritten }));
    setError("");
    try {
      const out = isLocal
        ? await cmd.localDockerPullImage(rewritten, pullEnv())
        : await cmd.dockerPullImage({
            host: tab.sshHost,
            port: tab.sshPort,
            user: tab.sshUser,
            authMode: tab.sshAuthMode,
            password: tab.sshPassword,
            keyPath: tab.sshKeyPath,
            imageRef: rewritten,
            envPrefix: pullEnv(),
            savedConnectionIndex: tab.sshSavedConnectionIndex,
          });
      const lastLine = out.trim().split("\n").pop() ?? "";
      setPullLog(lastLine || t("Pulled {ref}.", { ref: rewritten }));
      setNotice(t("Pulled {ref}.", { ref: rewritten }));
      await refresh();
    } catch (e) {
      setPullLog("");
      setError(formatError(e));
    } finally {
      setPullBusy(false);
    }
  }

  async function pruneVolumes() {
    if (actionBusy) return;
    setActionBusy(true);
    setError("");
    try {
      const out = isLocal
        ? await cmd.localDockerPruneVolumes()
        : await cmd.dockerPruneVolumes({
            host: tab.sshHost,
            port: tab.sshPort,
            user: tab.sshUser,
            authMode: tab.sshAuthMode,
            password: tab.sshPassword,
            keyPath: tab.sshKeyPath,
            savedConnectionIndex: tab.sshSavedConnectionIndex,
          });
      setNotice(out.trim().split("\n").pop() || t("Pruned unused volumes."));
      await refresh();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function toggleVolumeExpand(name: string, mountpoint: string) {
    if (expandedVolume === name) {
      setExpandedVolume("");
      return;
    }
    setExpandedVolume(name);
    if (volumeFiles[name] !== undefined) return; // cached
    setVolumeFilesBusy(name);
    try {
      const out = isLocal
        ? await cmd.localDockerVolumeFiles(mountpoint)
        : await cmd.dockerVolumeFiles({
            host: tab.sshHost,
            port: tab.sshPort,
            user: tab.sshUser,
            authMode: tab.sshAuthMode,
            password: tab.sshPassword,
            keyPath: tab.sshKeyPath,
            mountpoint,
            savedConnectionIndex: tab.sshSavedConnectionIndex,
          });
      setVolumeFiles((prev) => ({ ...prev, [name]: out || t("(empty directory)") }));
    } catch (e) {
      setVolumeFiles((prev) => ({ ...prev, [name]: formatError(e) }));
    } finally {
      setVolumeFilesBusy("");
    }
  }

  async function submitRunContainer(options: cmd.DockerRunOptions) {
    if (actionBusy) return;
    setActionBusy(true);
    setError("");
    try {
      const id = isLocal
        ? await cmd.localDockerRunContainer(options)
        : await cmd.dockerRunContainer({
            host: tab.sshHost,
            port: tab.sshPort,
            user: tab.sshUser,
            authMode: tab.sshAuthMode,
            password: tab.sshPassword,
            keyPath: tab.sshKeyPath,
            options,
            savedConnectionIndex: tab.sshSavedConnectionIndex,
          });
      setNotice(t("Started container {id}.", { id: id.slice(0, 12) }));
      setRunDialogOpen(false);
      await refresh();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  function openRunDialog(image?: string) {
    setRunDefaultImage(image ?? "");
    setRunDialogOpen(true);
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
    updateTab(tab.id, {
      logCommand: `docker logs -f ${quoteCommandArg(id)}`,
      logSource: {
        ...tab.logSource,
        mode: "system",
        systemPresetId: "docker-container",
        systemArg: id,
      },
    });
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
        {busy && !state && <div className="lg-note">{t("Loading...")}</div>}
        {notice && <div className="lg-note">{notice}</div>}
        {error && <div className="lg-note lg-note--error">{error}</div>}

        {state && activeTab === "containers" && (
          <div className="dk-body">
            <div className="dk-toolbar">
              <button
                type="button"
                className="btn is-primary is-compact"
                disabled={actionBusy}
                onClick={() => openRunDialog()}
              >
                <Play size={11} /> {t("Run container")}
              </button>
              <div style={{ flex: 1 }} />
              <span className="mono text-muted" style={{ fontSize: "var(--size-micro)" }}>
                {t("{count} total", { count: state.containers.length })}
              </span>
            </div>
            <div className="dk-card-list">
              {filteredContainers.length === 0 ? (
                <div className="dk-empty">{t("No containers found.")}</div>
              ) : (
                filteredContainers.map((c) => {
                  const ds = dotState(c.state, c.running);
                  const iconTone = ds === "running" ? "is-pos" : ds === "restarting" ? "is-warn" : "is-muted";
                  const isSel = c.id === selectedContainer;
                  return (
                    <div
                      key={c.id}
                      className={"dk-card" + (isSel ? " selected" : "")}
                      onClick={() => setSelectedContainer(c.id)}
                    >
                      <span className={"dk-card-ic " + iconTone}>
                        <ContainerIcon size={12} />
                      </span>
                      <div className="dk-card-body">
                        <div className="dk-card-title">{c.names || shortId(c.id)}</div>
                        <div className="dk-card-sub">{c.image}</div>
                      </div>
                      <div className="dk-card-meta">
                        {(c.cpuPerc || c.memUsage) && (
                          <span>{formatCpuMem(c.cpuPerc, c.memUsage)}</span>
                        )}
                      </div>
                      <div className="dk-card-actions" onClick={(e) => e.stopPropagation()}>
                        {c.running ? (
                          <>
                            <button className="mini-btn is-stop" type="button" title={t("Stop")}
                              disabled={actionBusy}
                              onClick={() => void containerAction(c.id, "stop")}>
                              <Square size={10} />
                            </button>
                            <button className="mini-btn is-info" type="button" title={t("Restart")}
                              disabled={actionBusy}
                              onClick={() => void containerAction(c.id, "restart")}>
                              <RotateCw size={11} />
                            </button>
                          </>
                        ) : (
                          <>
                            <button className="mini-btn is-start" type="button" title={t("Start")}
                              disabled={actionBusy}
                              onClick={() => void containerAction(c.id, "start")}>
                              <Play size={11} />
                            </button>
                            <button className="mini-btn is-destructive" type="button" title={t("Remove")}
                              disabled={actionBusy}
                              onClick={() => void containerAction(c.id, "remove")}>
                              <Trash2 size={11} />
                            </button>
                          </>
                        )}
                        <button className="mini-btn" type="button" title={t("Logs")}
                          onClick={() => openContainerLogs(c.id)}>
                          <Scroll size={11} />
                        </button>
                      </div>
                    </div>
                  );
                })
              )}
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
                      <button className="mini-btn is-stop" type="button" title={t("Stop")} disabled={actionBusy}
                        onClick={() => void containerAction(selectedCtr.id, "stop")}>
                        <Square size={10} />
                      </button>
                      <button className="mini-btn is-info" type="button" title={t("Restart")} disabled={actionBusy}
                        onClick={() => void containerAction(selectedCtr.id, "restart")}>
                        <RotateCw size={11} />
                      </button>
                    </>
                  ) : (
                    <>
                      <button className="mini-btn is-start" type="button" title={t("Start")} disabled={actionBusy}
                        onClick={() => void containerAction(selectedCtr.id, "start")}>
                        <Play size={11} />
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
            <div className="dk-toolbar">
              <div className="dk-pull">
                <Download size={11} />
                <input
                  placeholder={t("e.g. nginx:1.27-alpine")}
                  value={pullRef}
                  onChange={(e) => setPullRef(e.currentTarget.value)}
                  onKeyDown={(e) => { if (e.key === "Enter") void pullImage(pullRef); }}
                  disabled={pullBusy}
                />
                <button
                  type="button"
                  className="btn is-primary is-compact"
                  disabled={pullBusy || !pullRef.trim()}
                  onClick={() => void pullImage(pullRef)}
                >
                  {pullBusy ? t("Pulling…") : t("Pull")}
                </button>
              </div>
              <button
                type="button"
                className="mini-btn"
                title={t("Registry / proxy settings")}
                onClick={() => setProxyDialogOpen(true)}
              >
                <Settings2 size={11} />
              </button>
              <span className="mono text-muted" style={{ fontSize: "var(--size-micro)" }}>
                {t("{count} total", { count: state.images.length })}
              </span>
            </div>
            {pullLog && (
              <div className="lg-note mono" style={{ fontSize: "var(--size-micro)" }}>{pullLog}</div>
            )}
            <div className="dk-card-list">
              {filteredImages.length === 0 ? (
                <div className="dk-empty">{t("No images found.")}</div>
              ) : (
                filteredImages.map((img) => {
                  const isSel = img.id === selectedImage;
                  return (
                    <div
                      key={img.id}
                      className={"dk-card" + (isSel ? " selected" : "")}
                      onClick={() => setSelectedImage(img.id)}
                    >
                      <span className="dk-card-ic">
                        <HardDrive size={12} />
                      </span>
                      <div className="dk-card-body">
                        <div className="dk-card-title">{img.repository}</div>
                        <div className="dk-card-sub">
                          <span className="c-accent">{img.tag}</span>
                          <span className="text-muted"> · {img.size}</span>
                          {img.created && <span className="text-muted"> · {img.created}</span>}
                        </div>
                      </div>
                      <div className="dk-card-actions" onClick={(e) => e.stopPropagation()}>
                        <button
                          className="mini-btn is-start"
                          type="button"
                          title={t("Run container")}
                          onClick={() => openRunDialog(`${img.repository}:${img.tag}`)}
                        >
                          <Play size={11} />
                        </button>
                        <button
                          className="mini-btn is-info"
                          type="button"
                          title={t("Update (re-pull)")}
                          disabled={pullBusy}
                          onClick={() => void pullImage(`${img.repository}:${img.tag}`)}
                        >
                          <RotateCw size={11} />
                        </button>
                        {(hasSsh || isLocal) && (
                          <button
                            className="mini-btn is-destructive"
                            type="button"
                            title={t("Remove")}
                            disabled={actionBusy}
                            onClick={() => void removeImage(img.id)}
                          >
                            <Trash2 size={11} />
                          </button>
                        )}
                      </div>
                    </div>
                  );
                })
              )}
            </div>
          </div>
        )}

        {state && activeTab === "volumes" && (
          <div className="dk-body">
            <div className="dk-toolbar">
              <button
                type="button"
                className="btn is-compact"
                disabled={actionBusy}
                onClick={() => void pruneVolumes()}
                title={t("Remove unused volumes")}
              >
                <Sparkles size={11} /> {t("Prune unused")}
              </button>
              <div style={{ flex: 1 }} />
              <div className="dk-sort-group mono" style={{ fontSize: "var(--size-micro)", color: "var(--muted)" }}>
                <span>{t("Sort by:")}</span>
                <button type="button" className={"dk-sort" + (volumeSort === "size" ? " active" : "")}
                  onClick={() => toggleVolumeSort("size")}>
                  {t("size")}{sortArrow(volumeSort === "size", volumeSortDesc)}
                </button>
                <button type="button" className={"dk-sort" + (volumeSort === "links" ? " active" : "")}
                  onClick={() => toggleVolumeSort("links")}>
                  {t("used by")}{sortArrow(volumeSort === "links", volumeSortDesc)}
                </button>
                <button type="button" className={"dk-sort" + (volumeSort === "name" ? " active" : "")}
                  onClick={() => toggleVolumeSort("name")}>
                  {t("name")}{sortArrow(volumeSort === "name", volumeSortDesc)}
                </button>
              </div>
            </div>
            <div className="dk-card-list">
              {filteredVolumes.length === 0 ? (
                <div className="dk-empty">{t("No volumes found.")}</div>
              ) : (
                filteredVolumes.map((v) => {
                  const isSel = v.name === selectedVolume;
                  const isExp = expandedVolume === v.name;
                  return (
                    <div key={v.name}>
                      <div
                        className={"dk-card" + (isSel ? " selected" : "")}
                        onClick={() => setSelectedVolume(v.name)}
                      >
                        <span className="dk-card-ic">
                          <Folder size={12} />
                        </span>
                        <div className="dk-card-body">
                          <div className="dk-card-title">{v.name}</div>
                          <div className="dk-card-sub">
                            <span className="text-muted">{v.driver}</span>
                            {v.links >= 0 && (
                              <span className="text-muted">
                                {" · "}{t("{n} ref", { n: v.links })}
                              </span>
                            )}
                            {v.mountpoint && <span className="text-muted"> · {v.mountpoint}</span>}
                          </div>
                        </div>
                        <div className="dk-card-meta">
                          {v.size && <span>{v.size}</span>}
                        </div>
                        <div className="dk-card-actions" onClick={(e) => e.stopPropagation()}>
                          <button
                            className="mini-btn"
                            type="button"
                            title={isExp ? t("Collapse") : t("Show files")}
                            onClick={() => void toggleVolumeExpand(v.name, v.mountpoint)}
                          >
                            {isExp ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
                          </button>
                          {hasSsh && (
                            <button
                              className="mini-btn is-destructive"
                              type="button"
                              title={t("Remove")}
                              disabled={actionBusy}
                              onClick={() => void removeVolume(v.name)}
                            >
                              <Trash2 size={11} />
                            </button>
                          )}
                        </div>
                      </div>
                      {isExp && (
                        <pre
                          className={"dk-vol-files" + (volumeFilesBusy === v.name ? " is-busy" : "")}
                        >
                          {volumeFilesBusy === v.name
                            ? t("Loading...")
                            : volumeFiles[v.name] ?? ""}
                        </pre>
                      )}
                    </div>
                  );
                })
              )}
            </div>
          </div>
        )}

        <RunContainerDialog
          open={runDialogOpen}
          busy={actionBusy}
          defaultImage={runDefaultImage}
          onClose={() => setRunDialogOpen(false)}
          onSubmit={(opts) => submitRunContainer(opts)}
        />

        <RegistryProxyDialog
          open={proxyDialogOpen}
          mirror={tab.dockerRegistryMirror}
          proxy={tab.dockerPullProxy}
          onClose={() => setProxyDialogOpen(false)}
          onSave={(mirror, proxy) => {
            updateTab(tab.id, {
              dockerRegistryMirror: mirror.trim(),
              dockerPullProxy: proxy.trim(),
            });
            setProxyDialogOpen(false);
          }}
        />

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
