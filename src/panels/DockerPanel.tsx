import {
  ChevronDown,
  ChevronRight,
  Container as ContainerIcon,
  Download,
  ExternalLink,
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
import { createPortal } from "react-dom";
import * as cmd from "../lib/commands";
import type { DockerOverview, TabState } from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError, localizeRuntimeMessage } from "../i18n/localizeMessage";
import DbConnRow from "../components/DbConnRow";
import DismissibleNote from "../components/DismissibleNote";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";
import ContainerLogsDialog from "../shell/ContainerLogsDialog";
import RegistryProxyDialog from "../shell/RegistryProxyDialog";
import RunContainerDialog from "../shell/RunContainerDialog";
import { dockerKeyForTab, useDockerStore, type DockerSection } from "../stores/useDockerStore";
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

/**
 * Shimmer rows rendered inside `.dk-card-list` while the first fetch for
 * a new remote is in flight. Keeps the panel chrome visible and gives
 * the body something to show instead of "No containers found" before
 * data has ever arrived.
 */
function DkSkeleton({ rows = 3 }: { rows?: number }) {
  return (
    <div className="dk-skeleton" aria-hidden>
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="dk-skeleton-row" />
      ))}
    </div>
  );
}

export default function DockerPanel({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const updateTab = useTabStore((s) => s.updateTab);
  // ── Store-backed cache ─────────────────────────────────────────
  // Overview, volume-file listings, and last-fetched time persist across
  // panel remounts (tool switching + StrictMode). The panel becomes a
  // thin view over the store; local `useState` is reserved for
  // ephemeral UI state (search, dialog open, selection).
  const dockerKey = dockerKeyForTab(tab);
  const snapshot = useDockerStore((s) => s.snapshots[dockerKey]);
  const dockerRefresh = useDockerStore((s) => s.refresh);
  const dockerLoadSection = useDockerStore((s) => s.loadSection);
  const dockerMerge = useDockerStore((s) => s.mergeOverview);
  const dockerSetVolumeFile = useDockerStore((s) => s.setVolumeFile);
  const dockerSetError = useDockerStore((s) => s.setError);
  /** Write an error message onto the current snapshot — replaces the old
   *  `setError(string)` local-state setter so action handlers keep working
   *  after the move to store-backed state. */
  const setError = (msg: string) => dockerSetError(dockerKey, msg);
  const state: DockerOverview | null = snapshot?.overview ?? null;
  const error = snapshot?.error ?? "";
  const busy = !!snapshot?.inFlight && !snapshot?.overview;
  const [showAll, setShowAll] = useState(true);
  const [activeTab, setActiveTab] = useState<DkTab>("containers");
  const activeSectionBusy = !!snapshot?.sectionInFlight?.[activeTab];
  const [actionBusy, setActionBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [inspectJson, setInspectJson] = useState("");
  // Container id being displayed in the inspect modal. When non-null
  // the `<pre>` JSON is rendered inside a dialog over the panel instead
  // of inline under the container details, so the details pane doesn't
  // balloon the moment the user asks for inspect output.
  const [inspectCtrId, setInspectCtrId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [selectedContainer, setSelectedContainer] = useState<string>("");
  const [selectedImage, setSelectedImage] = useState<string>("");
  const [selectedVolume, setSelectedVolume] = useState<string>("");
  const [selectedNetwork, setSelectedNetwork] = useState<string>("");
  const [volumeSort, setVolumeSort] = useState<VolumeSort>("size");
  const [volumeSortDesc, setVolumeSortDesc] = useState(true);
  const [expandedVolume, setExpandedVolume] = useState<string>("");
  // Volume file listings live in the store so expanding a volume once
  // (which can be slow over SSH) survives panel remounts.
  const volumeFiles = snapshot?.volumeFiles ?? {};
  const [volumeFilesBusy, setVolumeFilesBusy] = useState<string>("");
  const [runDialogOpen, setRunDialogOpen] = useState(false);
  const [runDefaultImage, setRunDefaultImage] = useState("");
  const [proxyDialogOpen, setProxyDialogOpen] = useState(false);
  const [logsDialog, setLogsDialog] = useState<{ id: string; name: string } | null>(null);
  const [pullRef, setPullRef] = useState("");
  const [pullBusy, setPullBusy] = useState(false);
  const [pullLog, setPullLog] = useState("");
  const [statsBusy, setStatsBusy] = useState(false);
  const [volumeUsageBusy, setVolumeUsageBusy] = useState(false);
  const statsAttemptKeyRef = useRef("");
  const volumeUsageAttemptKeyRef = useRef("");

  // SSH context can be inferred from a local terminal where the user
  // typed `ssh user@host`, or from a nested-ssh overlay on an SSH
  // tab. `effectiveSshTarget` collapses both with the primary fields.
  const sshTarget = effectiveSshTarget(tab);
  const hasSsh = sshTarget !== null;
  // Treat as "local docker" only when the tab has neither an SSH
  // backend nor an inferred SSH target.
  const isLocal = tab.backend === "local" && !hasSsh;
  const canRefresh = isLocal || hasSsh;
  const sshArgs = {
    host: sshTarget?.host ?? "",
    port: sshTarget?.port ?? 22,
    user: sshTarget?.user ?? "",
    authMode: sshTarget?.authMode ?? "password",
    password: sshTarget?.password ?? "",
    keyPath: sshTarget?.keyPath ?? "",
    savedConnectionIndex: sshTarget?.savedConnectionIndex ?? null,
  };

  const sshParams = {
    host: sshArgs.host,
    port: sshArgs.port,
    user: sshArgs.user,
    authMode: sshArgs.authMode,
    password: sshArgs.password,
    keyPath: sshArgs.keyPath,
    savedConnectionIndex: sshArgs.savedConnectionIndex,
  };

  /**
   * Refresh via the store so concurrent callers (StrictMode's double
   * mount, user clicks) coalesce into one fetch. This path intentionally
   * loads containers only; the other Docker tabs fetch their own data
   * when opened.
   */
  async function refresh(force = false) {
    setNotice("");
    if (force) statsAttemptKeyRef.current = "";
    await dockerRefresh(
      dockerKey,
      {
        fetchOverview: async () => {
          const overview = isLocal
            ? await cmd.localDockerOverview(showAll)
            : hasSsh
              ? await cmd.dockerOverview({
                  host: sshArgs.host,
                  port: sshArgs.port,
                  user: sshArgs.user,
                  authMode: sshArgs.authMode,
                  password: sshArgs.password,
                  keyPath: sshArgs.keyPath,
                  all: showAll,
                  savedConnectionIndex: sshArgs.savedConnectionIndex,
                })
              : null;
          if (!overview) {
            throw new Error(t("No connection available."));
          }
          return overview;
        },
        loaded: ["containers"],
      },
      force,
    ).catch(() => { /* error stored on snapshot */ });
  }

  async function loadDockerSection(section: DockerSection, force = false) {
    if (!canRefresh || section === "containers") return;
    if (force && section === "volumes") volumeUsageAttemptKeyRef.current = "";
    await dockerLoadSection(
      dockerKey,
      section,
      async () => {
        if (section === "images") {
          const images = isLocal
            ? await cmd.localDockerImages()
            : hasSsh
              ? await cmd.dockerImages(sshParams)
              : [];
          return { images };
        }
        if (section === "volumes") {
          const volumes = isLocal
            ? await cmd.localDockerVolumes()
            : hasSsh
              ? await cmd.dockerVolumes(sshParams)
              : [];
          return { volumes };
        }
        const networks = isLocal
          ? await cmd.localDockerNetworks()
          : hasSsh
            ? await cmd.dockerNetworks(sshParams)
            : [];
        return { networks };
      },
      force,
    ).catch(() => { /* error stored on snapshot */ });
  }

  async function refreshActiveTab(force = true) {
    if (activeTab === "containers") {
      await refresh(force);
    } else {
      await loadDockerSection(activeTab, force);
    }
  }

  async function fetchContainerStats() {
    if (statsBusy || (!hasSsh && !isLocal)) return;
    setStatsBusy(true);
    try {
      const stats = hasSsh ? await cmd.dockerStats(sshParams) : await cmd.localDockerStats();
      const byId = new Map(stats.map((s) => [s.id, s]));
      dockerMerge(dockerKey, (prev) => ({
        ...prev,
        containers: prev.containers.map((c) => {
          const s = byId.get(c.id) ?? byId.get(c.id.slice(0, 12));
          return s
            ? { ...c, cpuPerc: s.cpuPerc, memUsage: s.memUsage, memPerc: s.memPerc }
            : c;
        }),
      }));
    } catch {
      // stats unavailable
    } finally {
      setStatsBusy(false);
    }
  }

  async function fetchVolumeUsage() {
    if (volumeUsageBusy || (!hasSsh && !isLocal)) return;
    setVolumeUsageBusy(true);
    try {
      const usages = hasSsh ? await cmd.dockerVolumeUsage(sshParams) : await cmd.localDockerVolumeUsage();
      const byName = new Map(usages.map((u) => [u.name, u]));
      dockerMerge(dockerKey, (prev) => {
        const next = prev.volumes.map((v) => {
          const u = byName.get(v.name);
          return u
            ? { ...v, size: u.size, sizeBytes: u.sizeBytes, links: u.links }
            : v;
        });
        next.sort((a, b) => b.sizeBytes - a.sizeBytes || a.name.localeCompare(b.name));
        return { ...prev, volumes: next };
      });
    } catch {
      // system df unavailable
    } finally {
      setVolumeUsageBusy(false);
    }
  }

  // On mount (or when the remote changes): render from cache immediately
  // and kick off a background refresh if the cache is stale. The store's
  // in-flight guard collapses StrictMode's double-invoke into one fetch.
  useEffect(() => {
    if (!canRefresh) return;
    void refresh(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dockerKey, canRefresh]);

  useEffect(() => {
    if (!canRefresh || activeTab === "containers") return;
    void loadDockerSection(activeTab, false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTab, dockerKey, canRefresh]);

  useEffect(() => {
    if (!canRefresh || activeTab !== "containers" || statsBusy) return;
    if (!(snapshot?.loaded?.containers ?? false)) return;
    const containers = state?.containers ?? [];
    if (!containers.length) return;
    if (containers.some((c) => c.cpuPerc || c.memUsage)) return;
    const key = `${dockerKey}:${containers.map((c) => `${c.id}:${c.status}`).join("|")}`;
    if (statsAttemptKeyRef.current === key) return;
    statsAttemptKeyRef.current = key;
    const timer = window.setTimeout(() => {
      void fetchContainerStats();
    }, 600);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTab, dockerKey, canRefresh, snapshot?.loaded?.containers, state?.containers, statsBusy]);

  useEffect(() => {
    if (!canRefresh || activeTab !== "volumes" || volumeUsageBusy) return;
    if (!(snapshot?.loaded?.volumes ?? false)) return;
    const volumes = state?.volumes ?? [];
    if (!volumes.length) return;
    if (volumes.some((v) => v.size || v.links >= 0)) return;
    const key = `${dockerKey}:${volumes.map((v) => v.name).join("|")}`;
    if (volumeUsageAttemptKeyRef.current === key) return;
    volumeUsageAttemptKeyRef.current = key;
    const timer = window.setTimeout(() => {
      void fetchVolumeUsage();
    }, 300);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTab, dockerKey, canRefresh, snapshot?.loaded?.volumes, state?.volumes, volumeUsageBusy]);

  // Default-select the first container once data is available.
  useEffect(() => {
    if (!selectedContainer && state?.containers.length) {
      setSelectedContainer(state.containers[0].id);
    }
  }, [state, selectedContainer]);

  // "Show stopped containers" forces a containers-only refresh with the
  // new `all` flag. Other Docker tabs stay cached; their data is
  // independent of this container filter.
  //
  // Skips the initial mount — otherwise this effect and the mount effect
  // above both fire on first render.
  const prevShowAllRef = useRef(showAll);
  useEffect(() => {
    if (!canRefresh) return;
    if (prevShowAllRef.current === showAll) return;
    prevShowAllRef.current = showAll;
    void refresh(true);
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
            host: sshArgs.host,
            port: sshArgs.port,
            user: sshArgs.user,
            authMode: sshArgs.authMode,
            password: sshArgs.password,
            keyPath: sshArgs.keyPath,
            containerId: id,
            action,
            savedConnectionIndex: sshArgs.savedConnectionIndex,
          });
      setNotice(`${shortId(id)}: ${localizeRuntimeMessage(result, t)}`);
      await refresh(true);
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
        host: sshArgs.host,
        port: sshArgs.port,
        user: sshArgs.user,
        authMode: sshArgs.authMode,
        password: sshArgs.password,
        keyPath: sshArgs.keyPath,
        containerId: id,
        savedConnectionIndex: sshArgs.savedConnectionIndex,
      });
      setInspectJson(output);
      setInspectCtrId(id);
      setNotice(t("Loaded container inspection for {id}.", { id: shortId(id) }));
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function removeImage(id: string, label?: string) {
    if (!hasSsh || actionBusy) return;
    if (!window.confirm(t("Remove image {id}?", { id: label ?? shortId(id) }))) return;
    setActionBusy(true);
    setError("");
    try {
      await cmd.dockerRemoveImage({
        host: sshArgs.host,
        port: sshArgs.port,
        user: sshArgs.user,
        authMode: sshArgs.authMode,
        password: sshArgs.password,
        keyPath: sshArgs.keyPath,
        imageId: id,
        force: false,
        savedConnectionIndex: sshArgs.savedConnectionIndex,
      });
      setNotice(t("Removed image {id}.", { id: shortId(id) }));
      await loadDockerSection("images", true);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function removeVolume(name: string) {
    if (!hasSsh || actionBusy) return;
    if (!window.confirm(t("Remove volume {name}? Any data stored in the volume will be lost.", { name }))) return;
    setActionBusy(true);
    setError("");
    try {
      await cmd.dockerRemoveVolume({
        host: sshArgs.host,
        port: sshArgs.port,
        user: sshArgs.user,
        authMode: sshArgs.authMode,
        password: sshArgs.password,
        keyPath: sshArgs.keyPath,
        volumeName: name,
        savedConnectionIndex: sshArgs.savedConnectionIndex,
      });
      setNotice(t("Removed volume {name}.", { name }));
      await loadDockerSection("volumes", true);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function removeNetwork(name: string) {
    if (!hasSsh || actionBusy) return;
    if (!window.confirm(t("Remove network {name}?", { name }))) return;
    setActionBusy(true);
    setError("");
    try {
      await cmd.dockerRemoveNetwork({
        host: sshArgs.host,
        port: sshArgs.port,
        user: sshArgs.user,
        authMode: sshArgs.authMode,
        password: sshArgs.password,
        keyPath: sshArgs.keyPath,
        networkName: name,
        savedConnectionIndex: sshArgs.savedConnectionIndex,
      });
      setNotice(t("Removed network {name}.", { name }));
      await loadDockerSection("networks", true);
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
            host: sshArgs.host,
            port: sshArgs.port,
            user: sshArgs.user,
            authMode: sshArgs.authMode,
            password: sshArgs.password,
            keyPath: sshArgs.keyPath,
            imageRef: rewritten,
            envPrefix: pullEnv(),
            savedConnectionIndex: sshArgs.savedConnectionIndex,
          });
      const lastLine = out.trim().split("\n").pop() ?? "";
      setPullLog(lastLine || t("Pulled {ref}.", { ref: rewritten }));
      setNotice(t("Pulled {ref}.", { ref: rewritten }));
      await loadDockerSection("images", true);
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
            host: sshArgs.host,
            port: sshArgs.port,
            user: sshArgs.user,
            authMode: sshArgs.authMode,
            password: sshArgs.password,
            keyPath: sshArgs.keyPath,
            savedConnectionIndex: sshArgs.savedConnectionIndex,
          });
      setNotice(out.trim().split("\n").pop() || t("Pruned unused volumes."));
      await loadDockerSection("volumes", true);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setActionBusy(false);
    }
  }

  async function pruneImages() {
    if (actionBusy) return;
    if (!window.confirm(t("Remove all unused images? This runs `docker image prune -a -f`."))) return;
    setActionBusy(true);
    setError("");
    try {
      const out = isLocal
        ? await cmd.localDockerPruneImages()
        : await cmd.dockerPruneImages({
            host: sshArgs.host,
            port: sshArgs.port,
            user: sshArgs.user,
            authMode: sshArgs.authMode,
            password: sshArgs.password,
            keyPath: sshArgs.keyPath,
            savedConnectionIndex: sshArgs.savedConnectionIndex,
          });
      setNotice(out.trim().split("\n").pop() || t("Pruned unused images."));
      await loadDockerSection("images", true);
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
    if (volumeFiles[name] !== undefined) return; // cache hit (store-backed)
    setVolumeFilesBusy(name);
    try {
      const out = isLocal
        ? await cmd.localDockerVolumeFiles(mountpoint)
        : await cmd.dockerVolumeFiles({
            host: sshArgs.host,
            port: sshArgs.port,
            user: sshArgs.user,
            authMode: sshArgs.authMode,
            password: sshArgs.password,
            keyPath: sshArgs.keyPath,
            mountpoint,
            savedConnectionIndex: sshArgs.savedConnectionIndex,
          });
      dockerSetVolumeFile(dockerKey, name, out || t("(empty directory)"));
    } catch (e) {
      dockerSetVolumeFile(dockerKey, name, formatError(e));
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
            host: sshArgs.host,
            port: sshArgs.port,
            user: sshArgs.user,
            authMode: sshArgs.authMode,
            password: sshArgs.password,
            keyPath: sshArgs.keyPath,
            options,
            savedConnectionIndex: sshArgs.savedConnectionIndex,
          });
      setNotice(t("Started container {id}.", { id: id.slice(0, 12) }));
      setRunDialogOpen(false);
      await refresh(true);
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
    const ctr = state?.containers.find((c) => c.id === id);
    setLogsDialog({ id, name: ctr?.names || id.slice(0, 12) });
  }

  // Route the same container-logs stream into the right-side Log panel
  // instead of a modal dialog. Lets the user keep browsing the rest of
  // the UI while the stream runs. Reuses the "docker-container" system
  // preset so LogViewerPanel's existing source pipeline handles it.
  function openContainerLogsInPanel(id: string) {
    const current = tab.logSource ?? {
      mode: "system" as const,
      filePath: "",
      fileDir: "",
      systemPresetId: "docker-container",
      systemArg: "",
      customCommand: "",
    };
    updateTab(tab.id, {
      logSource: {
        ...current,
        mode: "system",
        systemPresetId: "docker-container",
        systemArg: id,
      },
    });
    useTabStore.getState().setTabRightTool(tab.id, "log");
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

  const hostLabel = hasSsh ? sshArgs.host : isLocal ? t("local") : "—";
  const headerMeta = state
    ? t("{host} · {count} containers", { host: hostLabel, count: state.containers.length })
    : hostLabel;
  const hostSub = hasSsh
    ? `${sshArgs.user}@${sshArgs.host}:${sshArgs.port} · ${t("remote via SSH")}`
    : isLocal
      ? t("Local Docker socket")
      : t("Not connected");

  const tabCounts: Record<DkTab, number> = {
    containers: state?.containers.length ?? 0,
    images: state?.images.length ?? 0,
    volumes: state?.volumes.length ?? 0,
    networks: state?.networks.length ?? 0,
  };
  const tabLoaded = (section: DkTab) => snapshot?.loaded?.[section] ?? false;
  const tabBusy = (section: DkTab) => !!snapshot?.sectionInFlight?.[section];

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
              {state && tabLoaded(k) ? <span className="dk-tab-count">{tabCounts[k]}</span> : null}
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
          <button className="dk-ic" type="button" title={t("Refresh")} disabled={!canRefresh || busy || activeSectionBusy} onClick={() => void refreshActiveTab(true)}>
            <RefreshCw size={11} />
          </button>
        </div>

        {!canRefresh && <div className="lg-note">{t("SSH connection required for Docker.")}</div>}
        {/* First-load spinner belongs inside each tab body as a skeleton
            (see <DkSkeleton/>), not at the panel top — a centered banner
            here used to cover the toolbar and felt like a "whole-area"
            loading state. */}
        {notice && (
          <DismissibleNote onDismiss={() => setNotice("")}>{notice}</DismissibleNote>
        )}
        {error && (
          <DismissibleNote tone="error" onDismiss={() => setError("")}>
            {error}
          </DismissibleNote>
        )}

        {activeTab === "containers" && (
          <div className="dk-body">
            <div className="dk-toolbar">
              <button
                type="button"
                className="btn is-primary is-compact"
                disabled={actionBusy || !state}
                onClick={() => openRunDialog()}
              >
                <Play size={11} /> {t("Run container")}
              </button>
              <div style={{ flex: 1 }} />
              <span className="mono text-muted" style={{ fontSize: "var(--size-micro)" }}>
                {t("{count} total", { count: state?.containers.length ?? 0 })}
              </span>
            </div>
            <div className="dk-card-list">
              {filteredContainers.length === 0 ? (
                busy && !tabLoaded("containers")
                  ? <DkSkeleton rows={3} />
                  : <div className="dk-empty">{t("No containers found.")}</div>
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
                        <button className="mini-btn" type="button" title={t("Open in Log panel")}
                          onClick={() => openContainerLogsInPanel(c.id)}>
                          <ExternalLink size={11} />
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
                  <button className="mini-btn" type="button" title={t("Open in Log panel")}
                    onClick={() => openContainerLogsInPanel(selectedCtr.id)}>
                    <ExternalLink size={11} />
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
              </div>
            )}
          </div>
        )}

        {activeTab === "images" && (
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
              <button
                type="button"
                className="btn is-compact"
                disabled={actionBusy || (!hasSsh && !isLocal)}
                onClick={() => void pruneImages()}
                title={t("Remove all unused images (docker image prune -a)")}
              >
                <Sparkles size={11} /> {t("Prune unused")}
              </button>
              <span className="mono text-muted" style={{ fontSize: "var(--size-micro)" }}>
                {tabLoaded("images") ? t("{count} total", { count: state?.images.length ?? 0 }) : ""}
              </span>
            </div>
            {pullLog && (
              <div className="lg-note mono" style={{ fontSize: "var(--size-micro)" }}>{pullLog}</div>
            )}
            <div className="dk-card-list">
              {filteredImages.length === 0 ? (
                !tabLoaded("images") || tabBusy("images")
                  ? <DkSkeleton rows={3} />
                  : <div className="dk-empty">{t("No images found.")}</div>
              ) : (
                filteredImages.map((img) => {
                  // Docker returns one row per tag, so a single sha256
                  // id can appear multiple times (e.g. nginx:latest +
                  // nginx:1.27 share an id). Key and select on the
                  // full (id, repo, tag) tuple to keep rows distinct.
                  const rowKey = `${img.id}|${img.repository}:${img.tag}`;
                  const isSel = rowKey === selectedImage;
                  return (
                    <div
                      key={rowKey}
                      className={"dk-card" + (isSel ? " selected" : "")}
                      onClick={() => setSelectedImage(rowKey)}
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
                            onClick={() => void removeImage(img.id, `${img.repository}:${img.tag}`)}
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

        {activeTab === "volumes" && (
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
                !tabLoaded("volumes") || tabBusy("volumes")
                  ? <DkSkeleton rows={3} />
                  : <div className="dk-empty">{t("No volumes found.")}</div>
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

        <ContainerLogsDialog
          open={logsDialog !== null}
          tab={tab}
          containerId={logsDialog?.id ?? ""}
          containerName={logsDialog?.name}
          onClose={() => setLogsDialog(null)}
          onOpenInLogPanel={
            logsDialog ? () => openContainerLogsInPanel(logsDialog.id) : undefined
          }
        />

        {inspectCtrId && inspectJson && createPortal(
          <div
            className="dlg-overlay"
            onClick={() => {
              setInspectCtrId(null);
              setInspectJson("");
            }}
          >
            <div
              className="dlg dlg--inspect"
              onClick={(e) => e.stopPropagation()}
            >
              <div className="dlg-head">
                <span className="dlg-title">
                  <FileText size={13} />
                  {t("Inspect Output")}
                  <span className="text-muted mono" style={{ marginLeft: "var(--sp-2)" }}>
                    {shortId(inspectCtrId)}
                  </span>
                </span>
                <div style={{ flex: 1 }} />
                <button
                  type="button"
                  className="mini-btn"
                  title={t("Close")}
                  onClick={() => {
                    setInspectCtrId(null);
                    setInspectJson("");
                  }}
                >
                  <X size={12} />
                </button>
              </div>
              <div className="dlg-body dlg-body--inspect">
                <pre className="dk-inspect-pre mono">{inspectJson}</pre>
              </div>
            </div>
          </div>,
          document.body,
        )}

        {activeTab === "networks" && (
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
                      <td colSpan={5} className="dk-empty">
                        {!tabLoaded("networks") || tabBusy("networks") ? t("Loading...") : t("No networks found.")}
                      </td>
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
