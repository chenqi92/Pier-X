import {
  AlertTriangle,
  BellRing,
  Check,
  ChevronDown,
  ChevronRight,
  Circle,
  Clock,
  Copy,
  Download,
  FileText,
  Info,
  Loader,
  MoreHorizontal,
  Package,
  Play,
  Plus,
  RefreshCw,
  RotateCw,
  Search,
  Server,
  Square,
  Trash2,
  Zap,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import * as cmd from "../lib/commands";
import type { SavedSshConnection } from "../lib/types";
import type {
  MirrorChoice,
  MirrorId,
  MirrorLatency,
  MirrorState,
  SoftwareBundle,
  SoftwareDescriptor,
  SoftwareInstallReport,
  SoftwarePackageDetail,
  SoftwarePackageStatus,
  SoftwareSearchHit,
  SoftwareServiceAction,
  SoftwareServiceActionReport,
  SoftwareUninstallReport,
  SshParams,
  UninstallOptions,
} from "../lib/commands";
import { describeInstallOutcome } from "../lib/softwareInstall";
import { buildRepoCleanupCommand } from "../lib/repoCleanup";
import { writeClipboardText } from "../lib/clipboard";
import { desktopNotify } from "../lib/notify";
import { toast } from "../stores/useToastStore";
import {
  buildCronLine,
  loadSchedules,
  saveSchedules,
  isDue,
  describeSchedule,
  makeScheduleId,
  type BundleSchedule,
} from "../lib/bundleSchedule";
import { effectiveSshTarget, type TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import {
  activePackageId,
  isVersionCacheFresh,
  softwareKeyForTab,
  useSoftwareStore,
  type SoftwareActivityKind,
} from "../stores/useSoftwareStore";
import { useUiActionsStore } from "../stores/useUiActionsStore";
import Dialog from "../components/Dialog";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";
import Popover from "../components/Popover";
import StatusDot from "../components/StatusDot";

type Props = { tab: TabState | null };

/** Stable order for the app-store sections — anything not in this
 *  list (or with an empty `category` field) lands in "Other" at the
 *  bottom. The id is the descriptor's `category` value; the label is
 *  the i18n key the panel translates. */
const CATEGORY_ORDER: { id: string; label: string }[] = [
  { id: "database", label: "Databases" },
  { id: "container", label: "Containers" },
  { id: "web", label: "Web servers" },
  { id: "runtime", label: "Languages & runtimes" },
  { id: "dev", label: "Build tools" },
  { id: "editor", label: "Editors" },
  { id: "terminal", label: "Shells & multiplexers" },
  { id: "network", label: "Network tools" },
  { id: "text", label: "Text & search" },
  { id: "system", label: "System utilities" },
];
const CATEGORY_OTHER = { id: "", label: "Other" };

function groupByCategory(
  rows: SoftwareDescriptor[],
): { id: string; label: string; entries: SoftwareDescriptor[] }[] {
  const buckets = new Map<string, SoftwareDescriptor[]>();
  for (const row of rows) {
    const key = row.category || "";
    const list = buckets.get(key) ?? [];
    list.push(row);
    buckets.set(key, list);
  }
  const out: { id: string; label: string; entries: SoftwareDescriptor[] }[] = [];
  for (const cat of CATEGORY_ORDER) {
    const entries = buckets.get(cat.id);
    if (entries && entries.length > 0) {
      out.push({ id: cat.id, label: cat.label, entries });
      buckets.delete(cat.id);
    }
  }
  // Anything left over (unknown / empty category) lands in Other.
  const leftover: SoftwareDescriptor[] = [];
  for (const list of buckets.values()) leftover.push(...list);
  if (leftover.length > 0) {
    out.push({
      id: CATEGORY_OTHER.id,
      label: CATEGORY_OTHER.label,
      entries: leftover,
    });
  }
  return out;
}

/** Whether `id` is one of the DB descriptors that supports the
 *  in-row metrics probe. */
function isDbDescriptor(id: string): boolean {
  return id === "postgres" || id === "mariadb" || id === "redis";
}

function matchesSearch(row: SoftwareDescriptor, query: string): boolean {
  if (!query) return true;
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return (
    row.displayName.toLowerCase().includes(q) ||
    row.id.toLowerCase().includes(q) ||
    (row.category ?? "").toLowerCase().includes(q) ||
    (row.notes ?? "").toLowerCase().includes(q)
  );
}

export default function SoftwarePanel(props: Props) {
  const ready = useDeferredMount();
  return (
    <div className="panel-stage">
      {ready ? (
        <SoftwarePanelBody {...props} />
      ) : (
        <PanelSkeleton variant="rows" rows={9} />
      )}
    </div>
  );
}

function SoftwarePanelBody({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);

  const sshTarget = tab ? effectiveSshTarget(tab) : null;
  const swKey = tab ? softwareKeyForTab(tab) : null;

  const snapshot = useSoftwareStore((s) => (swKey ? s.get(swKey) : null));
  const setProbeResult = useSoftwareStore((s) => s.setProbeResult);
  const setError = useSoftwareStore((s) => s.setError);
  const startActivity = useSoftwareStore((s) => s.startActivity);
  const appendLine = useSoftwareStore((s) => s.appendLine);
  const finishActivity = useSoftwareStore((s) => s.finishActivity);
  const setVersionCache = useSoftwareStore((s) => s.setVersionCache);

  /** Per-row user-selected version. `undefined` = "latest"; the
   *  install/update command goes out without a `version` and the
   *  package manager picks the default. State lives at the panel
   *  level so it survives row remounts (re-probe / activity end). */
  const [selectedVersions, setSelectedVersions] = useState<
    Record<string, string | undefined>
  >({});
  /** Per-row user-selected major-version variant (e.g. `"openjdk-21"`
   *  for Java). `undefined` = the descriptor's default install_packages.
   *  Only meaningful for descriptors that declare `versionVariants`. */
  const [selectedVariants, setSelectedVariants] = useState<
    Record<string, string | undefined>
  >({});
  /** Rows the user has expanded into the details pane. */
  const [expandedRows, setExpandedRows] = useState<Record<string, boolean>>({});
  /** Lazy-loaded details cache. Each entry is the loaded detail, the
   *  literal `"loading"` while a fetch is in flight, or `{ error }`
   *  if the last fetch failed. */
  const [details, setDetails] = useState<
    Record<string, SoftwarePackageDetail | "loading" | { error: string }>
  >({});
  /** In-flight version-list fetches, keyed by package id. The
   *  dropdown shows a spinner row while present. */
  const [versionsLoading, setVersionsLoading] = useState<Record<string, boolean>>(
    {},
  );
  const setCancelling = useSoftwareStore((s) => s.setCancelling);

  const [registry, setRegistry] = useState<SoftwareDescriptor[]>([]);
  const [enableService, setEnableService] = useState(true);
  const [probing, setProbing] = useState(false);
  /** App-store search filter — matches against displayName/id/category. */
  const [searchQuery, setSearchQuery] = useState("");
  /** Remote system-package search results (apt-cache search / dnf
   *  search / …). Populated 400ms after the user's last keystroke
   *  when the local registry has no matches. */
  const [searchHits, setSearchHits] = useState<SoftwareSearchHit[]>([]);
  const [searchPending, setSearchPending] = useState(false);
  /** Per-result busy / log state for ad-hoc installs from the
   *  search section. Keyed by package name. */
  const [arbitraryActivity, setArbitraryActivity] = useState<
    Record<string, { busy: boolean; log: string[]; error: string }>
  >({});
  /** Path of the user-extras JSON file shown in the panel footer
   *  so users discover where to add their own entries. `null` =
   *  src-tauri couldn't resolve a config dir on this OS. */
  const [extrasPath, setExtrasPath] = useState<string | null>(null);
  /** Editor dialog open flag. */
  const [extrasEditorOpen, setExtrasEditorOpen] = useState(false);
  /** Multi-host batch-action dialog open flag. */
  const [multiHostOpen, setMultiHostOpen] = useState(false);
  /** History dialog open flag. */
  const [historyOpen, setHistoryOpen] = useState(false);
  /** Webhooks settings dialog open flag. */
  const [webhooksOpen, setWebhooksOpen] = useState(false);
  /** When non-null, the Webhooks dialog opens with this initial
   *  tab. Cleared as soon as the dialog reads it (next mount /
   *  open) so the user's later manual tab-switching isn't
   *  overridden. */
  const [webhooksInitialTab, setWebhooksInitialTab] = useState<
    "endpoints" | "failures" | null
  >(null);
  // Listen for the App-level "open failures tab" signal so a
  // webhook-failed toast's Open Failures button works regardless
  // of where in the app the SoftwarePanel was last interacted
  // with. Counter-based pattern (same as recoveryRequestSeq) so a
  // single subscription captures every fire.
  const openFailuresSeq = useUiActionsStore(
    (s) => s.openWebhookFailuresSeq,
  );
  useEffect(() => {
    if (openFailuresSeq === 0) return;
    setWebhooksInitialTab("failures");
    setWebhooksOpen(true);
  }, [openFailuresSeq]);
  /** PostgreSQL quick-config dialog target descriptor (or null). */
  const [pgQuickTarget, setPgQuickTarget] = useState<SoftwareDescriptor | null>(null);
  /** MySQL/MariaDB quick-config dialog target. */
  const [mysqlQuickTarget, setMysqlQuickTarget] = useState<SoftwareDescriptor | null>(null);
  /** Redis quick-config dialog target. */
  const [redisQuickTarget, setRedisQuickTarget] = useState<SoftwareDescriptor | null>(null);
  /** Docker compose templates dialog target. */
  const [composeTarget, setComposeTarget] = useState<SoftwareDescriptor | null>(null);
  /** Clone-host dialog open flag. */
  const [cloneOpen, setCloneOpen] = useState(false);
  /** Live DB metrics, keyed by descriptor id. Populated by the
   *  per-row polling effect when the row is expanded AND
   *  descriptor.id is in the DB set. */
  const [metricsCache, setMetricsCache] = useState<Record<string, cmd.DbMetrics>>({});
  const metricsTimers = useRef<Record<string, ReturnType<typeof setInterval>>>({});
  /** Co-install graph dialog open flag. */
  const [graphOpen, setGraphOpen] = useState(false);
  /** Highlight pulse target id when user clicks a node — clears
   *  after ~1.5s. Drives the "scroll-to + flash" behaviour. */
  const [graphHighlight, setGraphHighlight] = useState<string | null>(null);
  /** "Record command as bundle" dialog open flag. */
  const [recordBundleOpen, setRecordBundleOpen] = useState(false);
  /** Co-install suggestion cache, keyed by descriptor id. We cache
   *  per session so a row that already showed chips doesn't refetch. */
  const [coInstallCache, setCoInstallCache] = useState<Record<string, string[]>>({});
  /** Rows the user has dismissed the suggestion chip on this session. */
  const [coInstallDismissed, setCoInstallDismissed] = useState<Set<string>>(new Set());
  /** Last-picked mirror, persisted across hosts. Suggested as the
   *  default on the next host where no mirror is yet detected. */
  const [preferredMirror, setPreferredMirror] = useState<MirrorId | null>(null);
  /** Curated bundle catalog from the backend. Empty until the
   *  initial load resolves; the panel just hides the section then. */
  const [bundles, setBundles] = useState<SoftwareBundle[]>([]);
  /** Open bundle-confirm dialog target, or `null` for closed. */
  const [bundleTarget, setBundleTarget] = useState<SoftwareBundle | null>(null);
  /** Bundle id whose install is currently running, or `null`. The
   *  card shows a spinner + the per-package activity arrives via
   *  the existing per-row event channel. */
  const [bundleRunning, setBundleRunning] = useState<string | null>(null);
  /** Live progress of the currently-running bundle. `null` when no
   *  bundle is in flight. The "skipped" counter advances when
   *  runBundle's loop sees an already-installed entry — so the
   *  banner shows e.g. `2/5 (1 skipped)` rather than only the
   *  literal-step count, which would make 5-package bundles that
   *  are mostly already installed look like they finished
   *  half-way. */
  const [bundleProgress, setBundleProgress] = useState<{
    current: number;
    total: number;
    skipped: number;
    packageId: string | null;
    /** Next not-yet-installed package id in the current order, or
     *  `null` when the iterator has nothing left to do. Surfaced
     *  in the bundle card during pause so the user can decide
     *  whether to resume or stop based on what's about to run. */
    nextPackageId: string | null;
    /** `"installing"` for runBundle, `"uninstalling"` for the
     *  reverse path. Drives banner copy. */
    mode: "installing" | "uninstalling";
  } | null>(null);
  /** Pause / skip / abort flags for the in-flight bundle. Refs
   *  rather than state because the loop reads them on each
   *  iteration without needing a re-render — we only `setState`
   *  when the user-facing status text needs to change. */
  const bundlePauseRef = useRef(false);
  const bundleSkipRef = useRef(false);
  const bundleAbortRef = useRef(false);
  /** Mirror of the pause flag for banner copy + button state.
   *  Kept as a separate piece of state so the banner re-renders
   *  the moment "暂停" is clicked, not on the next iteration. */
  const [bundlePaused, setBundlePaused] = useState(false);
  /** The installId of the package currently in flight inside a
   *  bundle run. Populated each iteration so the "跳过此包" /
   *  "停止整条链" buttons can call `software_install_cancel`
   *  against the right id. */
  const bundleCurrentInstallIdRef = useRef<string | null>(null);
  // Packages that crossed `not-installed → installed` during the
  // current `runBundle`. Used by the rollback CTA — when the bundle
  // bails on a real failure (not user-initiated skip / abort) we
  // offer to reverse-uninstall exactly these so partial state from
  // a half-finished run doesn't linger. Cleared on every fresh
  // bundle start.
  const bundleFreshlyInstalledRef = useRef<string[]>([]);
  const [pendingRollback, setPendingRollback] = useState<{
    bundleId: string;
    bundleName: string;
    packageIds: string[];
  } | null>(null);
  const [scheduleDialogOpen, setScheduleDialogOpen] = useState(false);
  const [schedules, setSchedules] = useState<BundleSchedule[]>(() =>
    loadSchedules(),
  );
  // Bundles are looked up by id. Keep the live `runBundle` reference
  // in a ref so the scheduler tick fires the latest closure (including
  // any state captured at fire time) rather than a stale one.
  const runBundleRef = useRef<((b: SoftwareBundle) => Promise<void>) | null>(
    null,
  );
  /** Detected mirror state for this host. `null` = not loaded yet. */
  const [mirrorState, setMirrorState] = useState<MirrorState | null>(null);
  const [mirrorCatalog, setMirrorCatalog] = useState<MirrorChoice[]>([]);
  const [mirrorDialogOpen, setMirrorDialogOpen] = useState(false);
  /** When a switch / restore is in flight we lock the dialog buttons. */
  const [mirrorBusy, setMirrorBusy] = useState<"set" | "restore" | "benchmark" | null>(
    null,
  );
  const [mirrorMessage, setMirrorMessage] = useState<string>("");
  /** Latency probe results, keyed by mirror id. `null` while
   *  probing or never run. */
  const [mirrorLatencies, setMirrorLatencies] = useState<Record<string, number | null>>(
    {},
  );
  /** Open uninstall-dialog target. The dialog reads dataDirs / id /
   *  displayName from this descriptor to decide which checkboxes
   *  appear and what name the user must type to confirm a wipe. */
  const [uninstallTarget, setUninstallTarget] = useState<SoftwareDescriptor | null>(null);
  /** Open log-dialog target. `null` = no dialog. The dialog owns its
   *  own fetch + refresh state; the panel just feeds it the descriptor
   *  + the SSH params it needs. */
  const [logTarget, setLogTarget] = useState<SoftwareDescriptor | null>(null);
  /** Open vendor-script confirm-dialog target. Distinct state from
   *  the uninstall dialog so a user can't have both open at once. The
   *  dialog reads `descriptor.vendorScript` to render the URL / risk
   *  notes / "I understand" gate. */
  const [vendorTarget, setVendorTarget] = useState<SoftwareDescriptor | null>(null);

  const sshParams = useMemo(() => {
    if (!sshTarget) return null;
    return {
      host: sshTarget.host,
      port: sshTarget.port,
      user: sshTarget.user,
      authMode: sshTarget.authMode,
      password: sshTarget.password,
      keyPath: sshTarget.keyPath,
      savedConnectionIndex: sshTarget.savedConnectionIndex,
    };
  }, [
    sshTarget?.host,
    sshTarget?.port,
    sshTarget?.user,
    sshTarget?.authMode,
    sshTarget?.password,
    sshTarget?.keyPath,
    sshTarget?.savedConnectionIndex,
  ]);

  // Pull the registry once. It's a static const on the backend so we
  // don't refetch when the host changes.
  useEffect(() => {
    let cancelled = false;
    cmd
      .softwareRegistry()
      .then((rows) => {
        if (!cancelled) setRegistry(rows);
      })
      .catch(() => {
        /* ignore — panel still renders skeleton on probe error */
      });
    cmd
      .softwareMirrorCatalog()
      .then((rows) => {
        if (!cancelled) setMirrorCatalog(rows);
      })
      .catch(() => {
        /* ignore */
      });
    cmd
      .softwareUserExtrasPath()
      .then((p) => {
        if (!cancelled) setExtrasPath(p);
      })
      .catch(() => {
        /* ignore */
      });
    cmd
      .softwarePreferencesGet()
      .then((p) => {
        if (!cancelled) setPreferredMirror(p.preferredMirrorId);
      })
      .catch(() => {
        /* ignore */
      });
    cmd
      .softwareBundles()
      .then((rows) => {
        if (!cancelled) setBundles(rows);
      })
      .catch(() => {
        /* ignore */
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function loadMirrorState() {
    if (!sshParams) return;
    try {
      const s = await cmd.softwareMirrorGet(sshParams);
      setMirrorState(s);
    } catch {
      setMirrorState(null);
    }
  }

  async function probe() {
    if (!sshParams || !swKey || probing) return;
    setProbing(true);
    try {
      const result = await cmd.softwareProbeRemote(sshParams);
      setProbeResult(swKey, result.env, result.statuses);
    } catch (e) {
      setError(swKey, formatError(e));
    } finally {
      setProbing(false);
    }
  }

  // Debounced remote search. Trigger only when the local registry
  // doesn't already have the answer (so common queries like "git"
  // don't fire a wasted apt-cache round-trip). 400ms delay keeps
  // typing responsive without flooding the SSH session.
  useEffect(() => {
    const q = searchQuery.trim();
    if (q.length < 3 || !sshParams) {
      setSearchHits([]);
      setSearchPending(false);
      return;
    }
    // If the local registry already shows results, hide the
    // remote section unless the user keeps typing past 4 chars
    // (heuristic: at that point they probably want a wider net).
    const localHits = registry.filter((d) => matchesSearch(d, q));
    if (localHits.length > 0 && q.length < 4) {
      setSearchHits([]);
      setSearchPending(false);
      return;
    }
    setSearchPending(true);
    const handle = setTimeout(async () => {
      try {
        const hits = await cmd.softwareSearchRemote({
          ...sshParams,
          query: q,
          limit: 30,
        });
        setSearchHits(hits);
      } catch {
        setSearchHits([]);
      } finally {
        setSearchPending(false);
      }
    }, 400);
    return () => {
      clearTimeout(handle);
      setSearchPending(false);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchQuery, sshParams, registry]);

  // Probe on host change. Also drop the details cache so a stale
  // snapshot from the previous host can't surface in this host's
  // expanded rows.
  useEffect(() => {
    if (!sshParams || !swKey) return;
    setDetails({});
    setExpandedRows({});
    setMirrorState(null);
    setMirrorMessage("");
    void probe();
    void loadMirrorState();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [swKey]);

  if (!tab) {
    return (
      <div className="panel-section panel-section--empty">
        <div className="panel-section__title mono">
          <Package size={12} /> {t("Software")}
        </div>
        <div className="status-note mono">{t("Open an SSH tab to manage installed software.")}</div>
      </div>
    );
  }

  if (!sshTarget) {
    return (
      <div className="panel-section panel-section--empty">
        <div className="panel-section__title mono">
          <Package size={12} /> {t("Software")}
        </div>
        <div className="status-note mono">
          {t("This tab has no SSH context — software management is remote-only.")}
        </div>
      </div>
    );
  }

  const env = snapshot?.env ?? null;
  const statuses = snapshot?.statuses ?? {};
  const activity = snapshot?.activity ?? {};
  const versionCache = snapshot?.versionCache ?? {};
  const busyPackageId = snapshot ? activePackageId(snapshot) : null;
  const canManage = env?.packageManager !== null && env?.packageManager !== undefined;
  /** pacman repos only carry the latest version, so the panel hides
   *  the dropdown trigger on Arch hosts. */
  const supportsVersionPick = canManage && env?.packageManager !== "pacman";

  /** Lazy-fetch the descriptor's version list. Skips the round-trip
   *  when a fresh cache entry exists (TTL = 5 min). The dropdown
   *  shows a "Loading versions..." row while in flight.
   *
   *  Re-keying by variant is intentionally NOT done — for v2.1 the
   *  cache is per-package; the picked variant influences which
   *  package the version query targets but the cache treats them as
   *  the same row. Switching variants will re-issue the query through
   *  the cache because the variant's package may not match the
   *  default's, but for now this is acceptable; can be revisited if
   *  users complain about stale "OpenJDK 8" rows showing for "OpenJDK 21". */
  async function loadVersionsForPackage(packageId: string) {
    if (!sshParams || !swKey || !snapshot) return;
    if (isVersionCacheFresh(snapshot, packageId)) return;
    if (versionsLoading[packageId]) return;
    setVersionsLoading((prev) => ({ ...prev, [packageId]: true }));
    try {
      const versions = await cmd.softwareVersionsRemote({
        ...sshParams,
        packageId,
        variantKey: selectedVariants[packageId] ?? null,
      });
      setVersionCache(swKey, packageId, versions);
    } catch {
      // Leave the cache untouched; the dropdown will show "no versions".
      // The user can retry by closing + reopening the dropdown after
      // the staleness window.
    } finally {
      setVersionsLoading((prev) => ({ ...prev, [packageId]: false }));
    }
  }

  /** Lazy-load the details pane for a row. Always re-fetches if the
   *  prior fetch errored. Cached results stay on the panel for the
   *  lifetime of the host snapshot. */
  async function loadDetailsForPackage(packageId: string, force = false) {
    if (!sshParams) return;
    const cur = details[packageId];
    if (cur === "loading") return;
    if (cur && typeof cur === "object" && "packageId" in cur && !force) return;
    setDetails((prev) => ({ ...prev, [packageId]: "loading" }));
    try {
      const detail = await cmd.softwareDetailsRemote({
        ...sshParams,
        packageId,
      });
      setDetails((prev) => ({ ...prev, [packageId]: detail }));
    } catch (e) {
      setDetails((prev) => ({
        ...prev,
        [packageId]: { error: formatError(e) },
      }));
    }
  }

  function toggleExpanded(packageId: string) {
    setExpandedRows((prev) => {
      const opening = !prev[packageId];
      if (opening) {
        void loadDetailsForPackage(packageId);
        if (isDbDescriptor(packageId) && statuses[packageId]?.installed) {
          startMetricsPoll(packageId);
        }
      } else {
        stopMetricsPoll(packageId);
      }
      return { ...prev, [packageId]: opening };
    });
  }

  /** Kick off a `systemctl <verb>` for one row's service. Mirrors the
   *  install / uninstall handlers' lifecycle exactly so the row UI
   *  (busy state, log streaming, post-action status flip) reuses the
   *  same code path. */
  async function runServiceAction(
    descriptor: SoftwareDescriptor,
    action: SoftwareServiceAction,
  ) {
    if (!sshParams || !swKey) return;
    const installId =
      typeof crypto !== "undefined" && "randomUUID" in crypto
        ? crypto.randomUUID()
        : `${Date.now()}-${Math.random()}`;
    const kind: SoftwareActivityKind = `service-${action}`;
    startActivity(swKey, descriptor.id, installId, kind);
    const unlisten = await cmd.subscribeSoftwareServiceAction(installId, (evt) => {
      if (evt.kind === "line") {
        appendLine(swKey, descriptor.id, evt.text);
      }
    });
    try {
      const report: SoftwareServiceActionReport = await cmd.softwareServiceActionRemote({
        ...sshParams,
        packageId: descriptor.id,
        installId,
        action,
      });
      const localized = describeServiceOutcome(report, t);
      // Flip just the serviceActive dot — version / installed are
      // unchanged by start / stop / restart / reload.
      const prior = statuses[descriptor.id] ?? null;
      const nextStatus: SoftwarePackageStatus | null = prior
        ? { ...prior, serviceActive: report.serviceActiveAfter }
        : null;
      finishActivity(
        swKey,
        descriptor.id,
        report.status === "ok" ? "" : localized,
        nextStatus,
      );
    } catch (e) {
      finishActivity(swKey, descriptor.id, formatError(e), null);
    } finally {
      unlisten();
    }
  }

  /** Kick off an install / update / vendor-script install for
   *  `descriptor`. Single owner of the install lifecycle so the row
   *  click and the vendor confirm-dialog both end up on the same
   *  code path. `action`:
   *
   *  - `"install"` — default apt / dnf / … path
   *  - `"update"` — re-install / upgrade via the same default path
   *  - `"install-vendor"` — v2: download + run the descriptor's
   *    `vendorScript` (e.g. get.docker.com). Only valid when the
   *    descriptor exposes a `vendorScript`. */
  async function runInstall(
    descriptor: SoftwareDescriptor,
    action: "install" | "update" | "install-vendor",
  ) {
    if (!sshParams || !swKey) return;
    const installId =
      typeof crypto !== "undefined" && "randomUUID" in crypto
        ? crypto.randomUUID()
        : `${Date.now()}-${Math.random()}`;
    // The store only knows three kinds of activity ("install" / "update"
    // / "uninstall"); collapse the vendor variant to "install" so the
    // existing "Installing…" label and busy-row dimming keep working.
    startActivity(
      swKey,
      descriptor.id,
      installId,
      action === "install-vendor" ? "install" : action,
    );
    // Cancel-vs-done race guard: when the user clicks Cancel, the
    // backend emits a `cancelled` event AND the awaited promise also
    // resolves with `status: "cancelled"`. Without this flag both code
    // paths would call finishActivity, the second overwriting the
    // first. Mirrors `runUninstall`'s guard.
    let cancelledSeen = false;
    const unlisten = await cmd.subscribeSoftwareInstall(installId, (evt) => {
      if (evt.kind === "line") {
        appendLine(swKey, descriptor.id, evt.text);
      } else if (evt.kind === "cancelled") {
        cancelledSeen = true;
        finishActivity(swKey, descriptor.id, t("Cancelled"), null);
      }
      // `done` / `failed` are handled by the promise resolve/reject
      // below — no extra work here.
    });
    try {
      const params = {
        ...sshParams,
        packageId: descriptor.id,
        installId,
        enableService,
        version: selectedVersions[descriptor.id],
        variantKey: selectedVariants[descriptor.id] ?? null,
        ...(action === "install-vendor" ? { viaVendorScript: true } : {}),
      };
      const report: SoftwareInstallReport =
        action === "update"
          ? await cmd.softwareUpdateRemote(params)
          : await cmd.softwareInstallRemote(params);
      // The `cancelled` event may have arrived first (most common —
      // event channel beats the awaited Tauri response) OR the report
      // itself may carry status="cancelled" (the response landed first).
      // Either way bail before letting the report overwrite the
      // "Cancelled" label the event handler already set.
      if (cancelledSeen) return;
      if (report.status === "cancelled") {
        finishActivity(swKey, descriptor.id, t("Cancelled"), null);
        return;
      }
      // Vendor-script runs end with an explicit "via {label} ({url})"
      // line in the activity log so the user can audit which channel
      // produced the install without reading the report struct.
      if (report.vendorScript) {
        appendLine(
          swKey,
          descriptor.id,
          t("via {label} ({url})", {
            label: report.vendorScript.label,
            url: report.vendorScript.url,
          }),
        );
      }
      const localized = describeInstallOutcome(report, t);
      const nextStatus: SoftwarePackageStatus = {
        id: descriptor.id,
        installed: report.status === "installed",
        version: report.installedVersion,
        serviceActive: report.serviceActive,
      };
      finishActivity(
        swKey,
        descriptor.id,
        report.status === "installed" ? "" : localized,
        nextStatus,
        report.repoWarnings ?? [],
      );
      // Append to the history journal. We log every terminal
      // outcome (success / failure) so the user can scan the dialog
      // and figure out what happened across multiple installs.
      void cmd.softwareHistoryLog({
        action: action === "update" ? "update" : "install",
        target: descriptor.id,
        host: `${sshTarget?.user}@${sshTarget?.host}:${sshTarget?.port}`,
        savedConnectionIndex: sshTarget?.savedConnectionIndex ?? null,
        outcome: report.status,
        note: report.status === "installed" ? "" : localized,
      });
      // Bust the details cache so a re-expanded row re-runs the
      // install-paths / candidate-version probes against the new state.
      if (report.status === "installed") {
        setDetails((prev) => {
          const next = { ...prev };
          delete next[descriptor.id];
          return next;
        });
      }
    } catch (e) {
      // Same guard on the failure path — if cancel already finished
      // the activity, don't replace its localized "Cancelled" label
      // with a raw error string from the unwound promise.
      if (cancelledSeen) return;
      finishActivity(swKey, descriptor.id, formatError(e), null);
    } finally {
      unlisten();
    }
  }

  /** Kick off an uninstall for `descriptor` with the dialog's options.
   *  Mirrors the install handler's lifecycle: generate an installId,
   *  start activity, subscribe to the per-installId stream, fire the
   *  command, mirror outcome into the store, then unsubscribe.
   *
   *  Cancellation race: if a `cancelled` event arrives during the
   *  await — either because the user clicked Cancel and the backend
   *  fanned the signal out, or pier-core observed the token mid-run —
   *  the listener writes the cancelled outcome and the post-await
   *  block early-returns so the resolved report can't overwrite it.
   *  This implements the "cancelled wins over done" rule. */
  async function runUninstall(
    descriptor: SoftwareDescriptor,
    options: UninstallOptions,
  ) {
    if (!sshParams || !swKey) return;
    setUninstallTarget(null);
    const installId =
      typeof crypto !== "undefined" && "randomUUID" in crypto
        ? crypto.randomUUID()
        : `${Date.now()}-${Math.random()}`;
    startActivity(swKey, descriptor.id, installId, "uninstall");
    let cancelledSeen = false;
    const unlisten = await cmd.subscribeSoftwareUninstall(installId, (evt) => {
      if (evt.kind === "line") {
        appendLine(swKey, descriptor.id, evt.text);
      } else if (evt.kind === "cancelled") {
        cancelledSeen = true;
        finishActivity(swKey, descriptor.id, t("Cancelled"), null);
      }
    });
    try {
      const report: SoftwareUninstallReport = await cmd.softwareUninstallRemote({
        ...sshParams,
        packageId: descriptor.id,
        installId,
        options,
      });
      if (cancelledSeen) return;
      if (report.status === "cancelled") {
        finishActivity(swKey, descriptor.id, t("Cancelled"), null);
        return;
      }
      const localized = describeUninstallOutcome(report, t);
      // Refresh status: when the package is gone, drop installed/version;
      // when the remove failed, leave the prior status untouched (the
      // panel will re-probe to recover ground truth).
      const nextStatus =
        report.status === "uninstalled" || report.status === "not-installed"
          ? ({
              id: descriptor.id,
              installed: false,
              version: null,
              serviceActive: null,
            } as SoftwarePackageStatus)
          : null;
      finishActivity(
        swKey,
        descriptor.id,
        report.status === "uninstalled" || report.status === "not-installed"
          ? ""
          : localized,
        nextStatus,
      );
      void cmd.softwareHistoryLog({
        action: "uninstall",
        target: descriptor.id,
        host: `${sshTarget?.user}@${sshTarget?.host}:${sshTarget?.port}`,
        savedConnectionIndex: sshTarget?.savedConnectionIndex ?? null,
        outcome: report.status,
        note:
          report.status === "uninstalled" || report.status === "not-installed"
            ? ""
            : localized,
      });
      if (report.status === "uninstalled") {
        setDetails((prev) => {
          const next = { ...prev };
          delete next[descriptor.id];
          return next;
        });
      }
    } catch (e) {
      if (cancelledSeen) return;
      finishActivity(swKey, descriptor.id, formatError(e), null);
    } finally {
      unlisten();
    }
  }

  /** Trigger the backend cancel for the row's in-flight activity.
   *  No-op when the row isn't busy or has already requested cancel.
   *  The backend may not be able to actually stop the remote process —
   *  see the disclaimer in the i18n string and PRODUCT-SPEC §5.11 v2. */
  async function cancelRow(packageId: string) {
    if (!swKey) return;
    const a = snapshot?.activity[packageId];
    if (!a || !a.busy || a.cancelling) return;
    setCancelling(swKey, packageId, true);
    try {
      await cmd.softwareInstallCancel(a.installId);
    } catch {
      // softwareInstallCancel resolves Ok even when the backend can't
      // find the install_id — any error here is an IPC failure, in
      // which case the cancelled event won't arrive and the user is
      // stuck. Reset the cancelling flag so they can retry.
      setCancelling(swKey, packageId, false);
    }
  }

  /** Install a package by name from the search-section list.
   *  Bypasses the registry entirely — the package may not have a
   *  descriptor. Streams output via the same SOFTWARE_INSTALL
   *  channel as descriptor-driven installs so the existing event
   *  wiring works. After success, re-probe so any rows that DO
   *  have descriptors update their installed flag. */
  async function installArbitrary(packageName: string) {
    if (!sshParams || arbitraryActivity[packageName]?.busy) return;
    const installId =
      typeof crypto !== "undefined" && "randomUUID" in crypto
        ? crypto.randomUUID()
        : `${Date.now()}-${Math.random()}`;
    setArbitraryActivity((prev) => ({
      ...prev,
      [packageName]: { busy: true, log: [], error: "" },
    }));
    const unlisten = await cmd.subscribeSoftwareInstall(installId, (evt) => {
      if (evt.kind === "line") {
        setArbitraryActivity((prev) => {
          const cur = prev[packageName];
          if (!cur) return prev;
          const log = [...cur.log, evt.text];
          if (log.length > 200) log.splice(0, log.length - 200);
          return { ...prev, [packageName]: { ...cur, log } };
        });
      }
    });
    try {
      const report = await cmd.softwareInstallArbitrary({
        ...sshParams,
        packageName,
        installId,
      });
      setArbitraryActivity((prev) => ({
        ...prev,
        [packageName]: {
          ...(prev[packageName] ?? { log: [], busy: false, error: "" }),
          busy: false,
          error:
            report.status === "installed"
              ? ""
              : describeInstallOutcome(report, t),
        },
      }));
      if (report.status === "installed") {
        // A registry row may have just become installed (e.g. user
        // searched "git" via apt-cache).
        void probe();
      }
    } catch (e) {
      setArbitraryActivity((prev) => ({
        ...prev,
        [packageName]: {
          ...(prev[packageName] ?? { log: [], busy: false, error: "" }),
          busy: false,
          error: formatError(e),
        },
      }));
    } finally {
      unlisten();
    }
  }

  /** Send `cd <path>` (followed by Enter) into this tab's
   *  terminal so the user lands in the install/config dir
   *  without retyping. Falls back to clipboard when the panel
   *  isn't attached to a live terminal session — i.e. the SSH
   *  session exists but no terminal tab has been opened yet. */
  async function sendCdToTerminal(path: string) {
    const sessionId = tab?.terminalSessionId ?? null;
    if (sessionId) {
      try {
        // Trailing \n triggers shell submission; spaces in `path`
        // are quoted so paths like `/etc/My App` survive verbatim.
        const safe = path.replace(/'/g, "'\\''");
        await cmd.terminalWrite(sessionId, ` cd '${safe}'\n`);
        return;
      } catch {
        // Fall through to clipboard.
      }
    }
    const safe = path.replace(/'/g, "'\\''");
    await writeClipboardText(`cd '${safe}'`);
  }

  /** Inject a "disable this stale third-party repo" command into the
   *  current SSH terminal tab. Unlike `sendCdToTerminal` this does
   *  NOT append `\n` — the user reviews the command (it touches
   *  /etc/apt/... or runs `dnf config-manager --set-disabled`) and
   *  presses Enter themselves. Falls back to clipboard when no
   *  terminal session is attached.
   *
   *  Per the user's "问题只在当前终端中解决" rule we never auto-execute,
   *  never modify the host on their behalf — we just hand them the
   *  one-liner so the friction of retyping it from the advisory is
   *  zero. */
  async function sendRepoCleanupToTerminal(warning: string) {
    const snippet = buildRepoCleanupCommand(warning);
    if (!snippet) {
      await writeClipboardText(warning);
      return;
    }
    const sessionId = tab?.terminalSessionId ?? null;
    if (sessionId) {
      try {
        // Leading space + no trailing \n: the space keeps the line
        // out of `HISTCONTROL=ignorespace` shell histories (matches
        // the OSC 7 / 133 init payload's convention) so a snippet
        // the user decides not to run doesn't pollute history.
        await cmd.terminalWrite(sessionId, ` ${snippet}`);
        return;
      } catch {
        // Fall through to clipboard.
      }
    }
    await writeClipboardText(snippet);
  }

  /** Install all packages in `bundle` sequentially. Skips ones
   *  that are already installed (so re-running a bundle on a
   *  partially-set-up host is fast). Per-package progress flows
   *  through the existing per-row activity log.
   *
   *  Topological reorder: before iterating we pass the bundle's
   *  declared `packageIds` through the backend's
   *  `softwareBundleInstallOrder`, which uses the static co-install
   *  map (e.g. docker → compose) as a "anchor before companion"
   *  constraint. This keeps install logs reading top-to-bottom
   *  even when the bundle JSON listed `compose` ahead of `docker`,
   *  and matches what the user expects when they read the panel's
   *  per-row activity stream. The reorder is best-effort — if the
   *  command fails for any reason we fall back to declared order
   *  rather than block the install. */
  async function runBundle(bundle: SoftwareBundle) {
    if (!sshParams || !swKey || bundleRunning) return;
    setBundleRunning(bundle.id);
    setBundleProgress({
      current: 0,
      total: bundle.packageIds.length,
      skipped: 0,
      packageId: null,
      nextPackageId: null,
      mode: "installing",
    });
    bundlePauseRef.current = false;
    bundleSkipRef.current = false;
    bundleAbortRef.current = false;
    bundleFreshlyInstalledRef.current = [];
    setPendingRollback(null);
    setBundlePaused(false);
    let bundleFailed = false;
    try {
      let order = bundle.packageIds;
      try {
        order = await cmd.softwareBundleInstallOrder(bundle.packageIds);
      } catch {
        // Reorder is a nicety — fall back to declared order so a
        // backend hiccup doesn't block the install entirely.
      }
      for (let i = 0; i < order.length; i++) {
        // Pause gate — block the loop until the user clicks
        // "继续". Polled at 200ms because we don't have a tokio-
        // style notify in the browser; a missed wake-up just
        // delays the resume by one tick. Abort during pause
        // exits the loop without running the next package.
        // eslint-disable-next-line no-await-in-loop
        while (bundlePauseRef.current && !bundleAbortRef.current) {
          // eslint-disable-next-line no-await-in-loop
          await new Promise((r) => setTimeout(r, 200));
        }
        if (bundleAbortRef.current) break;

        const pkgId = order[i];
        // Look ahead to the next package the loop will actually
        // try to install — skip past entries that are already
        // installed or have no descriptor in the registry, since
        // those don't materialise a fresh install attempt. Used
        // only for the pause-state preview banner.
        const nextPkgId = (() => {
          for (let j = i + 1; j < order.length; j++) {
            const id = order[j];
            const desc = registry.find((d) => d.id === id);
            if (!desc) continue;
            if (statuses[id]?.installed) continue;
            return id;
          }
          return null;
        })();
        const descriptor = registry.find((d) => d.id === pkgId);
        if (!descriptor) {
          setBundleProgress((p) =>
            p
              ? { ...p, current: i + 1, skipped: p.skipped + 1, nextPackageId: nextPkgId }
              : p,
          );
          continue;
        }
        const cur = statuses[pkgId];
        if (cur?.installed) {
          setBundleProgress((p) =>
            p
              ? {
                  ...p,
                  current: i + 1,
                  skipped: p.skipped + 1,
                  packageId: pkgId,
                  nextPackageId: nextPkgId,
                }
              : p,
          );
          continue;
        }
        setBundleProgress((p) =>
          p
            ? { ...p, current: i + 1, packageId: pkgId, nextPackageId: nextPkgId }
            : p,
        );
        bundleSkipRef.current = false;
        // Stash the activity's installId so the skip / abort
        // buttons can call softwareInstallCancel with the right
        // id. We read it from the store right after runInstall
        // creates the activity — there's a tiny window before
        // that where the buttons can't act, but it's <1 frame.
        // (We can't pre-allocate the id here because runInstall
        // generates its own.)
        // eslint-disable-next-line no-await-in-loop
        const installPromise = runInstall(descriptor, "install");
        // Capture the activity's installId once it's been
        // populated by runInstall's startActivity call.
        const captureId = () => {
          const live =
            useSoftwareStore.getState().get(swKey).activity[pkgId];
          if (live?.installId) {
            bundleCurrentInstallIdRef.current = live.installId;
          }
        };
        // First tick after the microtask queue drains.
        queueMicrotask(captureId);
        // Re-check shortly after — startActivity is sync but the
        // event listener subscribe path isn't, so a 50ms grace
        // catches the late case without slowing the happy path.
        const captureTimer = window.setTimeout(captureId, 50);
        // eslint-disable-next-line no-await-in-loop
        await installPromise;
        window.clearTimeout(captureTimer);
        bundleCurrentInstallIdRef.current = null;
        const after = useSoftwareStore.getState().get(swKey).statuses[pkgId];
        if (!after?.installed) {
          if (bundleSkipRef.current) {
            // User skipped this specific package — keep going.
            bundleSkipRef.current = false;
            continue;
          }
          // Real failure or unsolicited cancellation — bail out.
          bundleFailed = true;
          break;
        }
        // Fresh install (was not installed before, is now). Tracked
        // separately so the rollback CTA only touches packages this
        // run actually changed.
        if (!cur?.installed) {
          bundleFreshlyInstalledRef.current.push(pkgId);
        }
      }
    } finally {
      // If the bundle bailed mid-loop (real failure, not user abort)
      // and we'd already crossed at least one package into installed
      // state, surface a rollback CTA. The user can ignore it (close
      // the banner) or accept it to reverse-uninstall the partial
      // run. We only surface on *failure* — a successful bundle
      // should leave its installs in place.
      if (
        bundleFailed &&
        !bundleAbortRef.current &&
        bundleFreshlyInstalledRef.current.length > 0
      ) {
        setPendingRollback({
          bundleId: bundle.id,
          bundleName: bundle.displayName || bundle.id,
          packageIds: [...bundleFreshlyInstalledRef.current],
        });
      }
      setBundleRunning(null);
      setBundleProgress(null);
      bundlePauseRef.current = false;
      bundleSkipRef.current = false;
      bundleAbortRef.current = false;
      bundleCurrentInstallIdRef.current = null;
      bundleFreshlyInstalledRef.current = [];
      setBundlePaused(false);
    }
  }

  /** Roll back the partial state of a failed bundle by uninstalling
   *  exactly the packages this run installed, in reverse order.
   *  Differs from `runBundleUninstall` in that it operates on a
   *  caller-supplied list rather than the whole bundle membership,
   *  so packages that were already installed before the bundle ran
   *  stay put. */
  async function runBundleRollback(
    bundleId: string,
    bundleName: string,
    packageIds: string[],
  ) {
    if (!sshParams || !swKey || bundleRunning) return;
    setPendingRollback(null);
    setBundleRunning(bundleId);
    setBundleProgress({
      current: 0,
      total: packageIds.length,
      skipped: 0,
      packageId: null,
      nextPackageId: null,
      mode: "uninstalling",
    });
    bundlePauseRef.current = false;
    bundleSkipRef.current = false;
    bundleAbortRef.current = false;
    setBundlePaused(false);
    try {
      const reversed = [...packageIds].reverse();
      for (let i = 0; i < reversed.length; i++) {
        // eslint-disable-next-line no-await-in-loop
        while (bundlePauseRef.current && !bundleAbortRef.current) {
          // eslint-disable-next-line no-await-in-loop
          await new Promise((r) => setTimeout(r, 200));
        }
        if (bundleAbortRef.current) break;

        const pkgId = reversed[i];
        const nextPkgId = reversed[i + 1] ?? null;
        const descriptor = registry.find((d) => d.id === pkgId);
        if (!descriptor) {
          setBundleProgress((p) =>
            p
              ? { ...p, current: i + 1, skipped: p.skipped + 1, nextPackageId: nextPkgId }
              : p,
          );
          continue;
        }
        const cur = statuses[pkgId];
        if (!cur?.installed) {
          // Already gone (user uninstalled mid-rollback or it never
          // landed cleanly). Skip without counting as a failure.
          setBundleProgress((p) =>
            p
              ? { ...p, current: i + 1, skipped: p.skipped + 1, packageId: pkgId, nextPackageId: nextPkgId }
              : p,
          );
          continue;
        }
        setBundleProgress((p) =>
          p
            ? { ...p, current: i + 1, packageId: pkgId, nextPackageId: nextPkgId }
            : p,
        );
        bundleSkipRef.current = false;
        // eslint-disable-next-line no-await-in-loop
        const uninstallPromise = runUninstall(descriptor, {
          purgeConfig: false,
          autoremove: false,
          removeDataDirs: false,
          removeUpstreamSource: false,
        });
        const captureId = () => {
          const live =
            useSoftwareStore.getState().get(swKey).activity[pkgId];
          if (live?.installId) {
            bundleCurrentInstallIdRef.current = live.installId;
          }
        };
        queueMicrotask(captureId);
        const captureTimer = window.setTimeout(captureId, 50);
        // eslint-disable-next-line no-await-in-loop
        await uninstallPromise;
        window.clearTimeout(captureTimer);
        bundleCurrentInstallIdRef.current = null;
        const after =
          useSoftwareStore.getState().get(swKey).statuses[pkgId];
        if (after?.installed) {
          if (bundleSkipRef.current) {
            bundleSkipRef.current = false;
            continue;
          }
          break;
        }
      }
      desktopNotify(
        "info",
        t("Bundle rolled back: {name}", { name: bundleName }),
        t("{n} package(s) reverted", { n: packageIds.length }),
      );
    } finally {
      setBundleRunning(null);
      setBundleProgress(null);
      bundlePauseRef.current = false;
      bundleSkipRef.current = false;
      bundleAbortRef.current = false;
      bundleCurrentInstallIdRef.current = null;
      setBundlePaused(false);
    }
  }

  // Keep a fresh reference so the schedule ticker always invokes the
  // latest closure (state, refs, swKey).
  runBundleRef.current = runBundle;

  /** Persist schedules whenever they change. Debouncing isn't worth
   *  it — the dialog edits are user-paced. */
  useEffect(() => {
    saveSchedules(schedules);
  }, [schedules]);

  /** 60-second tick: scan schedules for the current swKey, fire any
   *  that are due. We mark `lastRunAt` BEFORE firing so a slow
   *  install doesn't trigger a second concurrent run on the next
   *  tick (`bundleRunning` would also gate it, but the timestamp
   *  bump is what prevents re-fire after the run finishes). */
  useEffect(() => {
    if (!swKey) return undefined;
    const tick = () => {
      const now = new Date();
      let fired: string | null = null;
      setSchedules((prev) => {
        const updated = prev.map((s) => ({ ...s }));
        for (const sched of updated) {
          if (sched.swKey !== swKey) continue;
          if (!isDue(sched, now)) continue;
          if (bundleRunning) continue;
          if (fired) continue; // one fire per tick to avoid a stampede
          const bundle = bundles.find((b) => b.id === sched.bundleId);
          if (!bundle) continue;
          sched.lastRunAt = now.getTime();
          fired = sched.bundleId;
          // Fire after the state update settles. Using the ref so we
          // always invoke the latest runBundle closure.
          queueMicrotask(() => {
            runBundleRef.current?.(bundle);
          });
        }
        return updated;
      });
    };
    // First tick after mount waits a moment so we don't run during
    // initial render churn.
    const initial = window.setTimeout(tick, 5_000);
    const timer = window.setInterval(tick, 60_000);
    return () => {
      window.clearTimeout(initial);
      window.clearInterval(timer);
    };
  }, [swKey, bundles, bundleRunning]);

  /** Toggle the bundle's pause flag. The next iteration of the
   *  loop blocks at the pause gate until the flag clears; the
   *  current package keeps running to completion (a half-cancelled
   *  apt is worse than waiting one more `apt-get install`).
   *
   *  Pause-on transitions also fire a desktop notification: long
   *  install runs are exactly the case where the user steps away
   *  to do something else, and clicking pause silently then
   *  forgetting about it is a real footgun. The notification
   *  carries the current step so they remember where the bundle
   *  was when they come back. */
  function togglePauseBundle() {
    const willPause = !bundlePauseRef.current;
    bundlePauseRef.current = willPause;
    setBundlePaused(willPause);
    if (willPause && bundleProgress) {
      const step = `${bundleProgress.current}/${bundleProgress.total}`;
      const next = bundleProgress.nextPackageId;
      desktopNotify(
        "info",
        t("Bundle paused at step {step}", { step }),
        next ? t("Next package: {pkg}", { pkg: next }) : undefined,
      );
    }
  }

  /** Cancel the in-flight package's install and continue with the
   *  next one. Different from the per-row Cancel button: that one
   *  stops the entire bundle (the loop sees the cancelled status
   *  and breaks). This sets a sticky flag the loop checks so the
   *  cancellation doesn't propagate as a fail. */
  function skipCurrentBundlePackage() {
    const id = bundleCurrentInstallIdRef.current;
    if (!id) return;
    bundleSkipRef.current = true;
    void cmd.softwareInstallCancel(id).catch(() => {
      // Same fallback as cancelRow — IPC failure resets the flag
      // so the user can retry.
      bundleSkipRef.current = false;
    });
  }

  /** Abort the entire bundle: cancel the in-flight package AND
   *  set the abort flag so the loop exits at the next gate. */
  function abortBundle() {
    bundleAbortRef.current = true;
    bundlePauseRef.current = false;
    setBundlePaused(false);
    const id = bundleCurrentInstallIdRef.current;
    if (id) {
      void cmd.softwareInstallCancel(id).catch(() => {
        /* see above */
      });
    }
  }

  /** Reverse of `runBundle` — uninstall everything in `bundle` in
   *  reverse install order (services first, then their deps).
   *  Skips members that aren't installed. Uses safe defaults
   *  (no purge / no autoremove / no data-dir wipe / no upstream
   *  cleanup) so accidental clicks can't nuke postgres data.
   *
   *  Same topo reorder as `runBundle`, then reverse: companions
   *  come down BEFORE their anchors so e.g. compose stops before
   *  docker (avoids "stop daemon used by N containers" warnings
   *  on the way out). Falls back to reverse declared order on
   *  reorder failure for the same reason as the install path. */
  async function runBundleUninstall(bundle: SoftwareBundle) {
    if (!sshParams || !swKey || bundleRunning) return;
    setBundleRunning(bundle.id);
    setBundleProgress({
      current: 0,
      total: bundle.packageIds.length,
      skipped: 0,
      packageId: null,
      nextPackageId: null,
      mode: "uninstalling",
    });
    bundlePauseRef.current = false;
    bundleSkipRef.current = false;
    bundleAbortRef.current = false;
    setBundlePaused(false);
    try {
      let baseOrder = bundle.packageIds;
      try {
        baseOrder = await cmd.softwareBundleInstallOrder(bundle.packageIds);
      } catch {
        // Same fallback contract as runBundle.
      }
      const reversed = [...baseOrder].reverse();
      for (let i = 0; i < reversed.length; i++) {
        // Same pause gate as runBundle.
        // eslint-disable-next-line no-await-in-loop
        while (bundlePauseRef.current && !bundleAbortRef.current) {
          // eslint-disable-next-line no-await-in-loop
          await new Promise((r) => setTimeout(r, 200));
        }
        if (bundleAbortRef.current) break;

        const pkgId = reversed[i];
        // Mirror image of the install-side lookahead: skip past
        // entries that aren't installed (no work to do) so the
        // preview shows the next package that'll actually be
        // touched.
        const nextPkgId = (() => {
          for (let j = i + 1; j < reversed.length; j++) {
            const id = reversed[j];
            const desc = registry.find((d) => d.id === id);
            if (!desc) continue;
            if (!statuses[id]?.installed) continue;
            return id;
          }
          return null;
        })();
        const descriptor = registry.find((d) => d.id === pkgId);
        if (!descriptor) {
          setBundleProgress((p) =>
            p
              ? { ...p, current: i + 1, skipped: p.skipped + 1, nextPackageId: nextPkgId }
              : p,
          );
          continue;
        }
        const cur = statuses[pkgId];
        if (!cur?.installed) {
          setBundleProgress((p) =>
            p
              ? {
                  ...p,
                  current: i + 1,
                  skipped: p.skipped + 1,
                  packageId: pkgId,
                  nextPackageId: nextPkgId,
                }
              : p,
          );
          continue;
        }
        setBundleProgress((p) =>
          p
            ? { ...p, current: i + 1, packageId: pkgId, nextPackageId: nextPkgId }
            : p,
        );
        bundleSkipRef.current = false;
        // eslint-disable-next-line no-await-in-loop
        const uninstallPromise = runUninstall(descriptor, {
          purgeConfig: false,
          autoremove: false,
          removeDataDirs: false,
          removeUpstreamSource: false,
        });
        const captureId = () => {
          const live =
            useSoftwareStore.getState().get(swKey).activity[pkgId];
          if (live?.installId) {
            bundleCurrentInstallIdRef.current = live.installId;
          }
        };
        queueMicrotask(captureId);
        const captureTimer = window.setTimeout(captureId, 50);
        // eslint-disable-next-line no-await-in-loop
        await uninstallPromise;
        window.clearTimeout(captureTimer);
        bundleCurrentInstallIdRef.current = null;
        const after = useSoftwareStore.getState().get(swKey).statuses[pkgId];
        if (after?.installed) {
          if (bundleSkipRef.current) {
            bundleSkipRef.current = false;
            continue;
          }
          break;
        }
      }
    } finally {
      setBundleRunning(null);
      setBundleProgress(null);
      bundlePauseRef.current = false;
      bundleSkipRef.current = false;
      bundleAbortRef.current = false;
      bundleCurrentInstallIdRef.current = null;
      setBundlePaused(false);
    }
  }

  /** Build the install/update command for `descriptor` (without
   *  running it) and copy it to the clipboard. Lets users vet the
   *  command before pasting it into their own SSH session. */
  async function copyInstallCommand(
    descriptor: SoftwareDescriptor,
    action: "install" | "update",
  ) {
    if (!sshParams) return;
    try {
      const preview = await cmd.softwareInstallPreview({
        ...sshParams,
        packageId: descriptor.id,
        version: selectedVersions[descriptor.id] ?? null,
        variantKey: selectedVariants[descriptor.id] ?? null,
        isUpdate: action === "update",
      });
      await writeClipboardText(preview.wrappedCommand);
    } catch (e) {
      // Surface as an inline note on the row's activity log so the
      // user knows nothing landed on the clipboard. No retry — the
      // backend's only failure mode here is "no detected package
      // manager" and that's not going to change without a re-probe.
      if (swKey) {
        appendLine(swKey, descriptor.id, formatError(e));
      }
    }
  }

  /** Switch the host's apt/dnf sources to one of the curated
   *  mirrors. On success, re-probe the registry so the candidate-
   *  version queries pick up the new mirror immediately. */
  async function applyMirror(mirrorId: MirrorId) {
    if (!sshParams || mirrorBusy) return;
    setMirrorBusy("set");
    setMirrorMessage("");
    try {
      const report = await cmd.softwareMirrorSet({ ...sshParams, mirrorId });
      setMirrorState(report.stateAfter);
      void cmd.softwareHistoryLog({
        action: "mirror-set",
        target: mirrorId,
        host: `${sshTarget?.user}@${sshTarget?.host}:${sshTarget?.port}`,
        savedConnectionIndex: sshTarget?.savedConnectionIndex ?? null,
        outcome: report.status,
      });
      if (report.status === "ok") {
        setPreferredMirror(mirrorId);
        setMirrorMessage(t("Mirror switched. Refreshing software status..."));
        // Drop cached version lists so the dropdown re-queries
        // against the new mirror.
        if (swKey) useSoftwareStore.setState((s) => {
          const prev = s.snapshots[swKey];
          if (!prev) return s;
          return {
            snapshots: {
              ...s.snapshots,
              [swKey]: { ...prev, versionCache: {} },
            },
          };
        });
        setDetails({});
        void probe();
      } else if (report.status === "sudo-requires-password") {
        setMirrorMessage(
          t(
            "sudo requires a password — connect as root or configure passwordless sudo.",
          ),
        );
      } else if (report.status === "unsupported-manager") {
        setMirrorMessage(
          t("Mirror switching is supported on apt and dnf hosts only."),
        );
      } else {
        setMirrorMessage(
          t("Mirror switch failed (exit {code})", { code: report.exitCode }),
        );
      }
    } catch (e) {
      setMirrorMessage(formatError(e));
    } finally {
      setMirrorBusy(null);
    }
  }

  /** Run a probe against each mirror; populate `mirrorLatencies`.
   *  When `from === "host"` the probe runs over SSH (curl HEAD);
   *  when `from === "client"` it's a TCP connect from this Pier-X
   *  process. The host probe is more accurate (measures the
   *  actual network the package manager will use); the client
   *  probe still works when the remote host is offline. */
  async function runMirrorBenchmark(from: "host" | "client" = "host") {
    if (mirrorBusy) return;
    if (from === "host" && !sshParams) return;
    setMirrorBusy("benchmark");
    setMirrorMessage(
      from === "host" ? t("Probing mirrors...") : t("Probing from this machine..."),
    );
    try {
      const results: MirrorLatency[] =
        from === "host"
          ? await cmd.softwareMirrorBenchmark(sshParams!)
          : await cmd.softwareMirrorBenchmarkClient();
      const map: Record<string, number | null> = {};
      for (const r of results) map[r.mirrorId] = r.latencyMs;
      setMirrorLatencies(map);
      const reachable = results.filter((r) => r.latencyMs !== null);
      if (reachable.length === 0) {
        setMirrorMessage(t("No mirror reachable from this host."));
      } else {
        const fastest = reachable.reduce((a, b) =>
          (a.latencyMs ?? 0) <= (b.latencyMs ?? 0) ? a : b,
        );
        setMirrorMessage(
          t("Fastest: {id} · {ms} ms", {
            id: fastest.mirrorId,
            ms: fastest.latencyMs ?? 0,
          }),
        );
      }
    } catch (e) {
      setMirrorMessage(formatError(e));
    } finally {
      setMirrorBusy(null);
    }
  }

  async function restoreMirror() {
    if (!sshParams || mirrorBusy) return;
    setMirrorBusy("restore");
    setMirrorMessage("");
    try {
      const report = await cmd.softwareMirrorRestore(sshParams);
      setMirrorState(report.stateAfter);
      if (report.status === "ok") {
        setMirrorMessage(t("Original sources restored."));
        if (swKey) useSoftwareStore.setState((s) => {
          const prev = s.snapshots[swKey];
          if (!prev) return s;
          return {
            snapshots: {
              ...s.snapshots,
              [swKey]: { ...prev, versionCache: {} },
            },
          };
        });
        setDetails({});
        void probe();
      } else if (report.status === "sudo-requires-password") {
        setMirrorMessage(
          t(
            "sudo requires a password — connect as root or configure passwordless sudo.",
          ),
        );
      } else {
        setMirrorMessage(
          t("Restore failed (exit {code})", { code: report.exitCode }),
        );
      }
    } catch (e) {
      setMirrorMessage(formatError(e));
    } finally {
      setMirrorBusy(null);
    }
  }

  /** Start a 5s metrics poll for `descriptorId`. Idempotent —
   *  re-calling for the same id is a no-op while a timer is
   *  already running. The `inflight` guard prevents a slow probe
   *  (auth-prompt timeout, bad network) from queueing successive
   *  ticks that pile up SSH traffic and starve other panels —
   *  notably the russh handshake of a freshly-opened terminal,
   *  which used to stall behind a backlog of metrics probes. */
  function startMetricsPoll(descriptorId: string) {
    if (!sshParams) return;
    if (!isDbDescriptor(descriptorId)) return;
    if (metricsTimers.current[descriptorId]) return;
    let inflight = false;
    const tick = async () => {
      if (inflight) return;
      inflight = true;
      try {
        const m = await cmd.softwareDbMetrics({
          ...sshParams,
          packageId: descriptorId,
        });
        setMetricsCache((prev) => ({ ...prev, [descriptorId]: m }));
      } catch {
        // Drop probe failures silently; UI shows "—" when
        // probe_ok stays false.
      } finally {
        inflight = false;
      }
    };
    void tick();
    metricsTimers.current[descriptorId] = setInterval(() => void tick(), 5000);
  }

  function stopMetricsPoll(descriptorId: string) {
    const handle = metricsTimers.current[descriptorId];
    if (handle) {
      clearInterval(handle);
      delete metricsTimers.current[descriptorId];
    }
  }

  // Stop every poll on unmount / host change so we don't leak
  // intervals across SSH tabs.
  useEffect(() => {
    return () => {
      for (const id of Object.keys(metricsTimers.current)) {
        clearInterval(metricsTimers.current[id]);
      }
      metricsTimers.current = {};
    };
  }, [swKey]);

  /** Run the inverse of a history entry. Reachable from the
   *  history dialog's "Undo" button. Resolves credentials via the
   *  saved-connection index recorded at log time. Reports per-step
   *  status to the dialog through `onProgress`. */
  async function runHistoryUndo(
    entry: cmd.SoftwareHistoryEntry,
    onProgress: (msg: string) => void,
  ): Promise<boolean> {
    if (entry.savedConnectionIndex === null || entry.savedConnectionIndex === undefined) {
      onProgress(t("Undo unavailable: no saved-connection index for this entry."));
      return false;
    }
    // Resolve the saved connection.
    const conns = await cmd.sshConnectionsList().catch(() => []);
    const match = conns.find((c) => c.index === entry.savedConnectionIndex);
    if (!match) {
      onProgress(t("Undo unavailable: saved connection no longer exists."));
      return false;
    }
    const params: SshParams = {
      host: match.host,
      port: match.port,
      user: match.user,
      authMode: match.authKind === "password" ? "password" : match.authKind,
      password: "",
      keyPath: match.keyPath,
      savedConnectionIndex: match.index,
    };
    try {
      switch (entry.action) {
        case "install": {
          // Inverse: uninstall — basic options, keep configs/data.
          const installId =
            typeof crypto !== "undefined" && "randomUUID" in crypto
              ? crypto.randomUUID()
              : `${Date.now()}-${Math.random()}`;
          const r = await cmd.softwareUninstallRemote({
            ...params,
            packageId: entry.target,
            installId,
            options: {
              purgeConfig: false,
              autoremove: false,
              removeDataDirs: false,
              removeUpstreamSource: false,
            },
          });
          onProgress(t("Undo: uninstall {pkg} → {status}", {
            pkg: entry.target,
            status: r.status,
          }));
          // Log the inverse action so the journal stays coherent.
          void cmd.softwareHistoryLog({
            action: "undo-install",
            target: entry.target,
            host: entry.host,
            outcome: r.status,
            savedConnectionIndex: entry.savedConnectionIndex,
          });
          return r.status === "uninstalled" || r.status === "not-installed";
        }
        case "update":
        case "uninstall": {
          // Inverse: re-install (no version pin).
          const installId =
            typeof crypto !== "undefined" && "randomUUID" in crypto
              ? crypto.randomUUID()
              : `${Date.now()}-${Math.random()}`;
          const r = await cmd.softwareInstallRemote({
            ...params,
            packageId: entry.target,
            installId,
            enableService: true,
          });
          onProgress(t("Undo: install {pkg} → {status}", {
            pkg: entry.target,
            status: r.status,
          }));
          void cmd.softwareHistoryLog({
            action: "undo-uninstall",
            target: entry.target,
            host: entry.host,
            outcome: r.status,
            savedConnectionIndex: entry.savedConnectionIndex,
          });
          return r.status === "installed";
        }
        case "mirror-set": {
          const r = await cmd.softwareMirrorRestore(params);
          onProgress(t("Undo: restore mirror → {status}", { status: r.status }));
          void cmd.softwareHistoryLog({
            action: "undo-mirror-set",
            target: entry.target,
            host: entry.host,
            outcome: r.status,
            savedConnectionIndex: entry.savedConnectionIndex,
          });
          return r.status === "ok";
        }
        default:
          onProgress(t("Undo not supported for action: {a}", { a: entry.action }));
          return false;
      }
    } catch (e) {
      onProgress(e instanceof Error ? e.message : String(e));
      return false;
    }
  }

  /** Fetch and cache co-install suggestions for `descriptorId`.
   *  Idempotent — once cached, subsequent calls are no-ops. */
  function ensureCoInstallSuggestions(descriptorId: string) {
    if (coInstallCache[descriptorId] !== undefined) return;
    void cmd
      .softwareCoInstallSuggestions(descriptorId)
      .then((rows) =>
        setCoInstallCache((prev) => ({ ...prev, [descriptorId]: rows })),
      )
      .catch(() =>
        setCoInstallCache((prev) => ({ ...prev, [descriptorId]: [] })),
      );
  }

  /** Install every co-install suggestion not already on the host.
   *  Sequential, reusing `runInstall`'s lifecycle. */
  async function installCoInstallSuggestions(descriptorId: string) {
    const suggestions = coInstallCache[descriptorId] ?? [];
    for (const id of suggestions) {
      const desc = registry.find((d) => d.id === id);
      if (!desc) continue;
      if (statuses[id]?.installed) continue;
      // eslint-disable-next-line no-await-in-loop
      await runInstall(desc, "install");
    }
    setCoInstallDismissed((prev) => {
      const next = new Set(prev);
      next.add(descriptorId);
      return next;
    });
  }

  /** Render one software row. Hoisted out of the JSX so the
   *  category-grouped rendering and a future "favorites pinned at
   *  the top" view can share the same prop wiring. */
  function renderRow(descriptor: SoftwareDescriptor) {
    return (
      <SoftwareRow
        key={descriptor.id}
        descriptor={descriptor}
        status={statuses[descriptor.id] ?? null}
        activity={activity[descriptor.id] ?? null}
        // PAUSED pill on the row the bundle currently points at —
        // matches the bundle card's badge so the user sees the
        // pause state on whichever row their eye lands on first.
        bundlePausedHere={
          bundlePaused && bundleProgress?.packageId === descriptor.id
        }
        disabledOtherBusy={!!busyPackageId && busyPackageId !== descriptor.id}
        canManage={canManage}
        enableService={enableService}
        supportsVersionPick={supportsVersionPick}
        availableVersions={versionCache[descriptor.id]?.versions ?? null}
        versionsLoading={!!versionsLoading[descriptor.id]}
        selectedVersion={selectedVersions[descriptor.id]}
        selectedVariant={selectedVariants[descriptor.id]}
        expanded={!!expandedRows[descriptor.id]}
        details={details[descriptor.id] ?? null}
        onToggleExpand={() => toggleExpanded(descriptor.id)}
        onLoadDetails={() => void loadDetailsForPackage(descriptor.id, true)}
        onSelectVariant={(variant) => {
          setSelectedVariants((prev) => ({
            ...prev,
            [descriptor.id]: variant,
          }));
          // Variant change invalidates the version dropdown's cache —
          // different variant likely has different package-manager versions.
          setSelectedVersions((prev) => ({
            ...prev,
            [descriptor.id]: undefined,
          }));
        }}
        onSelectVersion={(version) =>
          setSelectedVersions((prev) => ({
            ...prev,
            [descriptor.id]: version,
          }))
        }
        onLoadVersions={() => void loadVersionsForPackage(descriptor.id)}
        onUninstall={() => setUninstallTarget(descriptor)}
        onServiceAction={(action) => void runServiceAction(descriptor, action)}
        onViewLogs={() => setLogTarget(descriptor)}
        onCopyCommand={(action) => void copyInstallCommand(descriptor, action)}
        onCancel={() => void cancelRow(descriptor.id)}
        onVendorPick={() => setVendorTarget(descriptor)}
        onAction={(action) => void runInstall(descriptor, action)}
        onCdToPath={(p) => void sendCdToTerminal(p)}
        onCleanupRepo={(w) => void sendRepoCleanupToTerminal(w)}
        hasLiveTerminal={!!tab?.terminalSessionId}
        metrics={metricsCache[descriptor.id] ?? null}
        pulse={graphHighlight === descriptor.id}
        onPgQuickConfig={
          descriptor.id === "postgres"
            ? () => setPgQuickTarget(descriptor)
            : descriptor.id === "mariadb"
              ? () => setMysqlQuickTarget(descriptor)
              : descriptor.id === "redis"
                ? () => setRedisQuickTarget(descriptor)
                : descriptor.id === "docker"
                  ? () => setComposeTarget(descriptor)
                  : undefined
        }
        quickConfigLabel={
          descriptor.id === "postgres"
            ? t("PostgreSQL quick config...")
            : descriptor.id === "mariadb"
              ? t("MySQL/MariaDB quick config...")
              : descriptor.id === "redis"
                ? t("Redis quick config...")
                : descriptor.id === "docker"
                  ? t("Compose templates...")
                  : undefined
        }
        coInstallSuggestions={(coInstallCache[descriptor.id] ?? []).filter(
          (id) => !statuses[id]?.installed && registry.some((d) => d.id === id),
        )}
        coInstallDismissed={coInstallDismissed.has(descriptor.id)}
        onEnsureCoInstall={() => ensureCoInstallSuggestions(descriptor.id)}
        onInstallCoInstall={() =>
          void installCoInstallSuggestions(descriptor.id)
        }
        onDismissCoInstall={() =>
          setCoInstallDismissed((prev) => {
            const next = new Set(prev);
            next.add(descriptor.id);
            return next;
          })
        }
      />
    );
  }

  return (
    <div className="sw-panel">
      <div className="sw-panel__header">
        <div className="sw-panel__title mono">
          <Package size={12} /> {t("Software")} · {sshTarget.user}@{sshTarget.host}
        </div>
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={() => setMirrorDialogOpen(true)}
          disabled={!mirrorState || !mirrorState.packageManager}
          title={t("Switch package source mirror")}
        >
          <Server size={10} />{" "}
          {mirrorLabelOrFallback(mirrorState, mirrorCatalog, t)}
        </button>
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={() => setMultiHostOpen(true)}
          title={t("Run a batch action across multiple hosts")}
        >
          <Server size={10} /> {t("Batch hosts")}
        </button>
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={() => setHistoryOpen(true)}
          title={t("Recent software-panel actions")}
        >
          <FileText size={10} /> {t("History")}
        </button>
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={() => setRecordBundleOpen(true)}
          title={t("Parse a paste-in install command into a custom bundle")}
        >
          <Package size={10} /> {t("Record bundle")}
        </button>
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={() => setCloneOpen(true)}
          title={t("Replicate one host's package set onto others")}
        >
          <Copy size={10} /> {t("Clone hosts")}
        </button>
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={() => setGraphOpen(true)}
          title={t("Visualize co-install relationships")}
        >
          <Zap size={10} /> {t("Dep graph")}
        </button>
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={() => setWebhooksOpen(true)}
          title={t(
            "Configure HTTP webhooks fired after each install / update / uninstall.",
          )}
        >
          <BellRing size={10} /> {t("Webhooks")}
        </button>
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={() => setScheduleDialogOpen(true)}
          title={t(
            "Schedule a bundle install on an interval / daily / weekly timer (only fires while Pier-X is open).",
          )}
        >
          <Clock size={10} /> {t("Schedules")}
        </button>
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={() => void probe()}
          disabled={probing}
          title={t("Re-probe host")}
        >
          <RefreshCw size={10} /> {probing ? t("Probing...") : t("Refresh")}
        </button>
      </div>
      <div className="sw-panel__env mono">
        {env ? (
          <>
            {env.distroPretty || env.distroId || t("Unknown OS")}
            {" · "}
            {env.packageManager ?? t("no package manager detected")}
            {!env.isRoot && <> · {t("non-root (sudo -n)")}</>}
          </>
        ) : (
          t("Probing host...")
        )}
      </div>
      {snapshot?.error && (
        <div className="status-note status-note--error mono">{snapshot.error}</div>
      )}
      {!canManage && env && (
        <div className="status-note status-note--error mono">
          {t(
            "Distro \"{id}\" is not in the supported list. Install software manually for now.",
            { id: env.distroId || "?" },
          )}
        </div>
      )}
      {bundles.length > 0 && canManage && (
        <div className="sw-panel__bundles">
          <div className="sw-panel__bundles-title mono">
            {t("Quick bundles")}
          </div>
          <div className="sw-panel__bundles-grid">
            {bundles.map((b) => {
              const installedCount = b.packageIds.filter(
                (id) => statuses[id]?.installed,
              ).length;
              const total = b.packageIds.length;
              const running = bundleRunning === b.id;
              return (
                <div key={b.id} className="sw-panel__bundle-card-wrap">
                <button
                  type="button"
                  className="sw-panel__bundle-card"
                  disabled={!!bundleRunning || !!busyPackageId}
                  onClick={() => setBundleTarget(b)}
                  title={b.description}
                >
                  <div className="sw-panel__bundle-card-head">
                    <span className="sw-panel__bundle-card-label">
                      {b.displayName}
                    </span>
                    <span className="sw-panel__bundle-card-count mono">
                      {installedCount}/{total}
                    </span>
                  </div>
                  <div className="sw-panel__bundle-card-desc">
                    {b.description}
                  </div>
                  {running && (
                    <div className="sw-panel__bundle-card-running mono">
                      <Loader size={10} className="sw-row__spin" />{" "}
                      {bundleProgress
                        ? bundleProgress.mode === "installing"
                          ? bundleProgress.packageId
                            ? t(
                                "Installing {cur}/{total}: {pkg}",
                                {
                                  cur: bundleProgress.current,
                                  total: bundleProgress.total,
                                  pkg: bundleProgress.packageId,
                                },
                              )
                            : t("Resolving order…")
                          : bundleProgress.packageId
                            ? t(
                                "Uninstalling {cur}/{total}: {pkg}",
                                {
                                  cur: bundleProgress.current,
                                  total: bundleProgress.total,
                                  pkg: bundleProgress.packageId,
                                },
                              )
                            : t("Resolving order…")
                        : t("Installing bundle...")}
                      {bundleProgress && bundleProgress.skipped > 0 && (
                        <span className="muted">
                          {" "}
                          ·{" "}
                          {t("{n} already installed", {
                            n: bundleProgress.skipped,
                          })}
                        </span>
                      )}
                      {bundlePaused && bundleProgress?.nextPackageId && (
                        <span className="sw-panel__bundle-next mono">
                          {t("Next: {pkg}", {
                            pkg: bundleProgress.nextPackageId,
                          })}
                        </span>
                      )}
                    </div>
                  )}
                  {running && bundleProgress && bundleProgress.total > 0 && (
                    <div
                      className={
                        "sw-panel__bundle-card-progress" +
                        (bundlePaused
                          ? " sw-panel__bundle-card-progress--paused"
                          : "")
                      }
                      role="progressbar"
                      aria-valuemin={0}
                      aria-valuemax={bundleProgress.total}
                      aria-valuenow={bundleProgress.current}
                    >
                      <div
                        className="sw-panel__bundle-card-progress-fill"
                        style={{
                          width: `${Math.min(
                            100,
                            (bundleProgress.current / bundleProgress.total) *
                              100,
                          )}%`,
                        }}
                      />
                      {bundlePaused && (
                        <span
                          className="sw-panel__bundle-card-progress-badge"
                          aria-label={t("Paused")}
                        >
                          {t("PAUSED")}
                        </span>
                      )}
                    </div>
                  )}
                </button>
                {running && (
                  <div className="sw-panel__bundle-card-controls">
                    <button
                      type="button"
                      className="mini-button"
                      onClick={togglePauseBundle}
                      title={
                        bundlePaused
                          ? t(
                              "Resume the bundle — the next package will start as soon as the current one returns.",
                            )
                          : t(
                              "Pause after the current package finishes. Click again to resume.",
                            )
                      }
                    >
                      {bundlePaused ? t("Resume") : t("Pause")}
                    </button>
                    <button
                      type="button"
                      className="mini-button"
                      onClick={skipCurrentBundlePackage}
                      title={t(
                        "Cancel the in-flight package and continue with the next one.",
                      )}
                    >
                      {t("Skip current")}
                    </button>
                    <button
                      type="button"
                      className="mini-button mini-button--destructive"
                      onClick={abortBundle}
                      title={t(
                        "Cancel the in-flight package and stop the entire bundle.",
                      )}
                    >
                      {t("Stop")}
                    </button>
                  </div>
                )}
                {pendingRollback?.bundleId === b.id && !running && (
                  <div className="sw-panel__bundle-rollback mono">
                    <div className="sw-panel__bundle-rollback-text">
                      {t(
                        "Bundle failed after installing {n} package(s). Roll back?",
                        { n: pendingRollback.packageIds.length },
                      )}
                    </div>
                    <div className="sw-panel__bundle-rollback-actions">
                      <button
                        type="button"
                        className="mini-button"
                        onClick={() => setPendingRollback(null)}
                      >
                        {t("Keep installs")}
                      </button>
                      <button
                        type="button"
                        className="mini-button mini-button--destructive"
                        onClick={() =>
                          void runBundleRollback(
                            pendingRollback.bundleId,
                            pendingRollback.bundleName,
                            pendingRollback.packageIds,
                          )
                        }
                      >
                        {t("Roll back {n}", {
                          n: pendingRollback.packageIds.length,
                        })}
                      </button>
                    </div>
                  </div>
                )}
                </div>
              );
            })}
          </div>
        </div>
      )}
      <div className="sw-panel__search">
        <Search size={11} className="sw-panel__search-icon" />
        <input
          type="text"
          className="sw-panel__search-input mono"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.currentTarget.value)}
          placeholder={t("Filter by name, id, or category...")}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
        />
        {searchQuery && (
          <button
            type="button"
            className="icon-btn sw-panel__search-clear"
            title={t("Clear")}
            onClick={() => setSearchQuery("")}
          >
            <X size={10} />
          </button>
        )}
      </div>
      <div className="sw-panel__list">
        <label className="sw-panel__service-toggle mono">
          <input
            type="checkbox"
            checked={enableService}
            onChange={(e) => setEnableService(e.currentTarget.checked)}
          />
          {t("After install, also enable & start the systemd service")}
        </label>
        {(() => {
          const filtered = registry.filter((d) => matchesSearch(d, searchQuery));
          if (filtered.length === 0) {
            return (
              <div className="sw-panel__empty mono">
                {t("No software matches \"{query}\"", { query: searchQuery })}
              </div>
            );
          }
          const groups = groupByCategory(filtered);
          return groups.map((group) => (
            <div key={group.id || "other"} className="sw-panel__section">
              <div className="sw-panel__section-title mono">
                {t(group.label)}
                <span className="sw-panel__section-count">
                  {group.entries.length}
                </span>
              </div>
              {group.entries.map((descriptor) => renderRow(descriptor))}
            </div>
          ));
        })()}
        {(searchPending || searchHits.length > 0) && searchQuery.trim().length >= 3 && (
          <div className="sw-panel__section">
            <div className="sw-panel__section-title mono">
              {t("System packages")}
              <span className="sw-panel__section-count">
                {searchPending ? "…" : searchHits.length}
              </span>
            </div>
            {searchPending && (
              <div className="sw-panel__empty mono">
                <Loader size={10} className="sw-row__spin" />{" "}
                {t("Searching system catalog...")}
              </div>
            )}
            {!searchPending &&
              searchHits.map((hit) => (
                <SystemPackageRow
                  key={hit.name}
                  hit={hit}
                  activity={arbitraryActivity[hit.name] ?? null}
                  onInstall={() => void installArbitrary(hit.name)}
                />
              ))}
          </div>
        )}
        {extrasPath && (
          <div className="sw-panel__extras-note mono">
            <Info size={10} />{" "}
            {t("Add custom entries by editing")}{" "}
            <button
              type="button"
              className="sw-panel__extras-path"
              title={t("Copy path")}
              onClick={() => void writeClipboardText(extrasPath)}
            >
              {extrasPath}
            </button>
            <button
              type="button"
              className="btn is-ghost is-compact sw-panel__extras-edit"
              onClick={() => setExtrasEditorOpen(true)}
              title={t("Open extras editor")}
            >
              {t("Edit")}
            </button>
          </div>
        )}
      </div>
      <UninstallDialog
        target={uninstallTarget}
        onCancel={() => setUninstallTarget(null)}
        onConfirm={(opts) => {
          if (uninstallTarget) void runUninstall(uninstallTarget, opts);
        }}
      />
      <ServiceLogsDialog
        target={logTarget}
        sshParams={sshParams}
        onClose={() => setLogTarget(null)}
      />
      <VendorScriptConfirmDialog
        target={vendorTarget}
        onCancel={() => setVendorTarget(null)}
        onConfirm={() => {
          const target = vendorTarget;
          setVendorTarget(null);
          if (target) void runInstall(target, "install-vendor");
        }}
      />
      <ExtrasEditorDialog
        open={extrasEditorOpen}
        path={extrasPath}
        onClose={() => setExtrasEditorOpen(false)}
      />
      <MultiHostDialog
        open={multiHostOpen}
        onClose={() => setMultiHostOpen(false)}
        bundles={bundles}
        mirrorCatalog={mirrorCatalog}
      />
      <HistoryDialog
        open={historyOpen}
        onClose={() => setHistoryOpen(false)}
        onUndo={async (entry, onProgress) => {
          await runHistoryUndo(entry, onProgress);
        }}
      />
      <WebhooksDialog
        open={webhooksOpen}
        initialTab={webhooksInitialTab}
        onClose={() => {
          setWebhooksOpen(false);
          setWebhooksInitialTab(null);
        }}
      />
      <BundleSchedulesDialog
        open={scheduleDialogOpen}
        onClose={() => setScheduleDialogOpen(false)}
        swKey={swKey}
        bundles={bundles}
        schedules={schedules}
        onChange={setSchedules}
        sshParams={sshParams}
      />
      <PgQuickConfigDialog
        target={pgQuickTarget}
        sshParams={sshParams}
        onClose={() => setPgQuickTarget(null)}
      />
      <MysqlQuickConfigDialog
        target={mysqlQuickTarget}
        sshParams={sshParams}
        onClose={() => setMysqlQuickTarget(null)}
      />
      <RedisQuickConfigDialog
        target={redisQuickTarget}
        sshParams={sshParams}
        onClose={() => setRedisQuickTarget(null)}
      />
      <ComposeTemplatesDialog
        target={composeTarget}
        sshParams={sshParams}
        onClose={() => setComposeTarget(null)}
      />
      <CloneHostsDialog
        open={cloneOpen}
        onClose={() => setCloneOpen(false)}
      />
      <DepGraphDialog
        open={graphOpen}
        registry={registry}
        statuses={statuses}
        onClose={() => setGraphOpen(false)}
        onJump={(id) => {
          setGraphOpen(false);
          // Scroll the row into view + pulse it for visibility.
          const el = document.getElementById(`sw-row-${id}`);
          if (el) el.scrollIntoView({ behavior: "smooth", block: "center" });
          setGraphHighlight(id);
          setTimeout(() => setGraphHighlight(null), 1500);
        }}
      />
      <RecordBundleDialog
        open={recordBundleOpen}
        onClose={() => setRecordBundleOpen(false)}
      />
      <BundleConfirmDialog
        target={bundleTarget}
        registry={registry}
        statuses={statuses}
        onCancel={() => setBundleTarget(null)}
        onInstall={() => {
          const target = bundleTarget;
          setBundleTarget(null);
          if (target) void runBundle(target);
        }}
        onUninstall={() => {
          const target = bundleTarget;
          setBundleTarget(null);
          if (target) void runBundleUninstall(target);
        }}
        onDryRun={async () => {
          const target = bundleTarget;
          if (!target || !sshParams) return;
          // Resolve every package's wrapped install command without
          // running anything. Topo-order via the existing helper so
          // the script reflects the same order the real install
          // would use; on failure we fall back to the bundle's
          // declared order. Already-installed packages are skipped
          // so the script is a clean re-runnable transcript.
          let order = target.packageIds;
          try {
            order = await cmd.softwareBundleInstallOrder(target.packageIds);
          } catch {
            /* fallback to declared order */
          }
          const lines: string[] = [];
          lines.push(`#!/usr/bin/env bash`);
          lines.push(`# Dry-run preview for bundle: ${target.displayName}`);
          lines.push(`# Generated by Pier-X — review before running.`);
          lines.push(`set -e`);
          let included = 0;
          for (const pkgId of order) {
            const desc = registry.find((d) => d.id === pkgId);
            if (!desc) {
              lines.push(`# ${pkgId}: not in registry, skipped`);
              continue;
            }
            if (statuses[pkgId]?.installed) {
              lines.push(`# ${pkgId}: already installed, skipped`);
              continue;
            }
            try {
              const preview = await cmd.softwareInstallPreview({
                ...sshParams,
                packageId: pkgId,
              });
              lines.push("");
              lines.push(`# ── ${desc.displayName} (${pkgId}) ──`);
              lines.push(preview.wrappedCommand);
              included += 1;
            } catch (e) {
              lines.push(
                `# ${pkgId}: preview failed — ${
                  e instanceof Error ? e.message : String(e)
                }`,
              );
            }
          }
          if (included === 0) {
            toast.warn(
              t("Nothing to dry-run — every package is already installed."),
            );
            return;
          }
          const script = lines.join("\n");
          try {
            await writeClipboardText(script);
            toast.info(
              t(
                "Copied dry-run script for {n} package(s). Paste into a remote shell to review before running.",
                { n: included },
              ),
            );
          } catch (e) {
            toast.warn(e instanceof Error ? e.message : String(e));
          }
        }}
      />
      <MirrorDialog
        open={mirrorDialogOpen}
        onClose={() => setMirrorDialogOpen(false)}
        catalog={mirrorCatalog}
        state={mirrorState}
        preferred={preferredMirror}
        latencies={mirrorLatencies}
        busy={mirrorBusy}
        message={mirrorMessage}
        onApply={(id) => void applyMirror(id)}
        onRestore={() => void restoreMirror()}
        onBenchmark={(from) => void runMirrorBenchmark(from)}
      />
    </div>
  );
}

/** Resolve the host/url string the dialog renders next to a mirror
 *  label. apt/dnf always have a hostname; apk/pacman/zypper may be
 *  `null` if the catalog hasn't declared coverage. */
function mirrorHostForManager(m: MirrorChoice, manager: string): string {
  switch (manager) {
    case "apt":
      return m.aptHost;
    case "dnf":
    case "yum":
      return m.dnfHost;
    case "apk":
      return m.apkHost ?? "—";
    case "pacman":
      return m.pacmanUrl ?? "—";
    case "zypper":
      return m.zypperHost ?? "—";
    default:
      return m.aptHost;
  }
}

/** Pick the label shown on the "软件源" header button. Falls back
 *  to "Source" when nothing is loaded yet, "Official" when the
 *  detected hostname doesn't match any mirror in the catalog. */
function mirrorLabelOrFallback(
  state: MirrorState | null,
  catalog: MirrorChoice[],
  t: ReturnType<typeof useI18n>["t"],
): string {
  if (!state || !state.packageManager) return t("Mirror");
  if (!state.currentId) {
    return state.currentHost ? `${t("Mirror")} · ${state.currentHost}` : t("Mirror");
  }
  const choice = catalog.find((c) => c.id === state.currentId);
  return `${t("Mirror")} · ${choice?.label ?? state.currentId}`;
}

/** Bundle install confirmation. Lists each member's current status
 *  so the user can see what's already there + what will actually
 *  be installed. The bundle install runs sequentially via the same
 *  per-row install path; already-installed members are skipped. */
function BundleConfirmDialog({
  target,
  registry,
  statuses,
  onCancel,
  onInstall,
  onUninstall,
  onDryRun,
}: {
  target: SoftwareBundle | null;
  registry: SoftwareDescriptor[];
  statuses: Record<string, SoftwarePackageStatus>;
  onCancel: () => void;
  onInstall: () => void;
  onUninstall: () => void;
  /** Optional dry-run hook. When set, the install panel renders an
   *  extra "Dry run" button alongside Install — handy for users who
   *  want to vet the resolved commands before letting Pier-X execute
   *  them on prod. */
  onDryRun?: () => void | Promise<void>;
}) {
  const { t } = useI18n();
  // Default mode: "install" if anything is missing, "uninstall"
  // if everything's already there.
  const [mode, setMode] = useState<"install" | "uninstall">("install");
  // Reset the mode whenever a different bundle opens. The
  // initial value (above) takes effect on the first open; this
  // effect keeps subsequent re-opens in a sensible default.
  useEffect(() => {
    if (!target) return;
    const allInstalled = target.packageIds.every(
      (id) => !!statuses[id]?.installed,
    );
    setMode(allInstalled ? "uninstall" : "install");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target?.id]);
  if (!target) return null;
  const items = target.packageIds.map((id) => {
    const desc = registry.find((d) => d.id === id);
    const installed = !!statuses[id]?.installed;
    return {
      id,
      label: desc?.displayName ?? id,
      missing: !desc,
      installed,
    };
  });
  const toInstall = items.filter((i) => !i.missing && !i.installed);
  const toUninstall = items.filter((i) => !i.missing && i.installed);
  return (
    <Dialog
      open={!!target}
      title={
        mode === "install"
          ? t("Install bundle: {name}", { name: target.displayName })
          : t("Uninstall bundle: {name}", { name: target.displayName })
      }
      subtitle={target.description}
      size="sm"
      onClose={onCancel}
    >
      <div className="sw-bundle-form">
        <div className="sw-bundle-form__tabs">
          <button
            type="button"
            className={`sw-bundle-form__tab${
              mode === "install" ? " is-active" : ""
            }`}
            onClick={() => setMode("install")}
          >
            {t("Install")}
          </button>
          <button
            type="button"
            className={`sw-bundle-form__tab${
              mode === "uninstall" ? " is-active" : ""
            }`}
            onClick={() => setMode("uninstall")}
          >
            {t("Uninstall")}
          </button>
        </div>
        <ul className="sw-bundle-form__list">
          {items.map((it) => (
            <li
              key={it.id}
              className={`sw-bundle-form__item${
                it.installed ? " is-installed" : ""
              }${it.missing ? " is-missing" : ""}`}
            >
              {it.installed ? (
                <Check size={10} />
              ) : it.missing ? (
                <X size={10} />
              ) : (
                <Circle size={10} />
              )}
              <span>{it.label}</span>
              <span className="sw-bundle-form__id mono">{it.id}</span>
            </li>
          ))}
        </ul>
        <div className="sw-bundle-form__msg mono">
          {mode === "install"
            ? toInstall.length === 0
              ? t("Nothing to install — every member is already on this host.")
              : t("Will install {n} package(s) sequentially.", {
                  n: toInstall.length,
                })
            : toUninstall.length === 0
              ? t("Nothing to uninstall — none of these are installed.")
              : t(
                  "Will uninstall {n} package(s) in reverse order. Configs and data dirs are kept.",
                  { n: toUninstall.length },
                )}
        </div>
        <div className="sw-bundle-form__actions">
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={onCancel}
          >
            {t("Cancel")}
          </button>
          {mode === "install" ? (
            <>
              {onDryRun && (
                <button
                  type="button"
                  className="btn is-ghost is-compact"
                  disabled={toInstall.length === 0}
                  onClick={() => void onDryRun()}
                  title={t(
                    "Resolve the install commands without running them, copy as a shell script.",
                  )}
                >
                  {t("Dry run")}
                </button>
              )}
              <button
                type="button"
                className="btn is-primary is-compact"
                disabled={toInstall.length === 0}
                onClick={onInstall}
              >
                <Download size={10} /> {t("Install bundle")}
              </button>
            </>
          ) : (
            <button
              type="button"
              className="btn is-danger is-compact"
              disabled={toUninstall.length === 0}
              onClick={onUninstall}
            >
              <Trash2 size={10} /> {t("Uninstall bundle")}
            </button>
          )}
        </div>
      </div>
    </Dialog>
  );
}

/** Mirror picker dialog. Lists every entry in the catalog with the
 *  current selection highlighted. The "restore" button lives at the
 *  bottom and is disabled when no `.pier-bak` is on the host. */
function MirrorDialog({
  open,
  onClose,
  catalog,
  state,
  preferred,
  latencies,
  busy,
  message,
  onApply,
  onRestore,
  onBenchmark,
}: {
  open: boolean;
  onClose: () => void;
  catalog: MirrorChoice[];
  state: MirrorState | null;
  preferred: MirrorId | null;
  /** Per-mirror latency in ms; `null` = unreachable; missing entry
   *  = not probed yet. */
  latencies: Record<string, number | null>;
  busy: "set" | "restore" | "benchmark" | null;
  message: string;
  onApply: (id: MirrorId) => void;
  onRestore: () => void;
  onBenchmark: (from: "host" | "client") => void;
}) {
  const { t } = useI18n();
  if (!open) return null;
  const manager = state?.packageManager ?? "";
  const currentId = state?.currentId ?? null;
  const hostHint = state?.currentHost
    ? t("Currently pointing at {host}", { host: state.currentHost })
    : t("Currently pointing at the official upstream.");
  return (
    <Dialog
      open={open}
      title={t("Package source mirror")}
      subtitle={
        manager
          ? t("Manager: {pm}. {hint}", { pm: manager, hint: hostHint })
          : t("No supported package manager detected.")
      }
      size="sm"
      onClose={onClose}
    >
      <div className="sw-mirror-form">
        {(() => {
          const hasLatencies = Object.keys(latencies).length > 0;
          // Sort by latency when probed; reachable first, then
          // unreachable, then never-probed at the bottom. Otherwise
          // keep the catalog's natural order.
          const sorted = hasLatencies
            ? [...catalog].sort((a, b) => {
                const la = latencies[a.id];
                const lb = latencies[b.id];
                const va = la === undefined ? 999_999 : la === null ? 99_999 : la;
                const vb = lb === undefined ? 999_999 : lb === null ? 99_999 : lb;
                return va - vb;
              })
            : catalog;
          // Pick the fastest reachable for the "推荐" pill.
          const fastestId = hasLatencies
            ? (() => {
                let best: { id: string; ms: number } | null = null;
                for (const [id, ms] of Object.entries(latencies)) {
                  if (typeof ms !== "number") continue;
                  if (!best || ms < best.ms) best = { id, ms };
                }
                return best?.id ?? null;
              })()
            : null;
          return (
            <div className="sw-mirror-form__list">
              {sorted.map((m) => {
                const active = currentId === m.id;
                const isSuggested =
                  !active && currentId === null && preferred === m.id;
                const isFastest = fastestId === m.id;
                const lat = latencies[m.id];
                const host = mirrorHostForManager(m, manager);
                return (
                  <button
                    key={m.id}
                    type="button"
                    className={`sw-mirror-row${active ? " is-active" : ""}${
                      isSuggested ? " is-suggested" : ""
                    }${isFastest ? " is-fastest" : ""}`}
                    disabled={!!busy || !manager}
                    onClick={() => onApply(m.id)}
                  >
                    <span className="sw-mirror-row__label">
                      {m.label}
                      {isFastest && (
                        <span className="sw-mirror-row__suggest-pill">
                          {t("recommended")}
                        </span>
                      )}
                      {isSuggested && !isFastest && (
                        <span className="sw-mirror-row__suggest-pill">
                          {t("last used")}
                        </span>
                      )}
                    </span>
                    <span className="sw-mirror-row__host mono">
                      {typeof lat === "number" ? (
                        <span className="sw-mirror-row__lat">{lat} ms</span>
                      ) : lat === null ? (
                        <span className="sw-mirror-row__lat sw-mirror-row__lat--bad">
                          {t("unreachable")}
                        </span>
                      ) : null}{" "}
                      {host}
                    </span>
                    {active && <Check size={12} className="sw-mirror-row__check" />}
                  </button>
                );
              })}
            </div>
          );
        })()}
        {message && <div className="sw-mirror-form__msg mono">{message}</div>}
        <div className="sw-mirror-form__actions">
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={() => onBenchmark("host")}
            disabled={!!busy || !manager}
            title={t("Probe each mirror's latency from this host")}
          >
            <Zap size={10} />{" "}
            {busy === "benchmark"
              ? t("Probing mirrors...")
              : t("Benchmark from host")}
          </button>
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={() => onBenchmark("client")}
            disabled={!!busy}
            title={t("Probe each mirror from this Pier-X process")}
          >
            <Zap size={10} /> {t("Benchmark from client")}
          </button>
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={onRestore}
            disabled={!!busy || !state?.hasBackup}
            title={
              state?.hasBackup
                ? t("Restore the original sources from .pier-bak")
                : t("No backup found on this host")
            }
          >
            <RotateCw size={10} />{" "}
            {busy === "restore" ? t("Restoring...") : t("Restore original")}
          </button>
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={onClose}
            disabled={!!busy}
          >
            {t("Close")}
          </button>
        </div>
      </div>
    </Dialog>
  );
}

/** Pick the label shown on the primary install/update button. Encodes
 *  the busy states (install / update / uninstall / 4 service actions)
 *  via `busyLabel`; idle → "Install" or "Update", with the selected
 *  version appended when the user has pinned one. */
function primaryButtonLabel({
  t,
  action,
  busy,
  activityKind,
  selectedVersion,
  variantLabel,
}: {
  t: ReturnType<typeof useI18n>["t"];
  action: "install" | "update";
  busy: boolean;
  activityKind: SoftwareActivityKind | undefined;
  selectedVersion: string | undefined;
  variantLabel: string | undefined;
}): string {
  if (busy) return busyLabel(activityKind, action, t);
  if (selectedVersion) {
    return action === "update"
      ? t("Update to v{version}", { version: selectedVersion })
      : t("Install v{version}", { version: selectedVersion });
  }
  if (variantLabel) {
    return action === "update"
      ? t("Update {variant}", { variant: variantLabel })
      : t("Install {variant}", { variant: variantLabel });
  }
  return action === "update" ? t("Update") : t("Install");
}

function busyLabel(
  kind: SoftwareActivityKind | undefined,
  fallbackAction: "install" | "update",
  t: ReturnType<typeof useI18n>["t"],
): string {
  switch (kind) {
    case "uninstall":
      return t("Uninstalling...");
    case "update":
      return t("Updating...");
    case "install":
      return t("Installing...");
    case "service-start":
      return t("Starting...");
    case "service-stop":
      return t("Stopping...");
    case "service-restart":
      return t("Restarting...");
    case "service-reload":
      return t("Reloading...");
    default:
      return fallbackAction === "update" ? t("Updating...") : t("Installing...");
  }
}

function describeServiceOutcome(
  report: SoftwareServiceActionReport,
  t: ReturnType<typeof useI18n>["t"],
): string {
  switch (report.status) {
    case "ok":
      switch (report.action) {
        case "start":
          return t("Service started");
        case "stop":
          return t("Service stopped");
        case "restart":
          return t("Service restarted");
        case "reload":
          return t("Service reloaded");
      }
      return t("Done");
    case "sudo-requires-password":
      return t(
        "sudo requires a password — connect as root or configure passwordless sudo.",
      );
    case "failed":
      return t("Service action failed (exit {code})", { code: report.exitCode });
  }
}

// `describeInstallOutcome` (and the `cancelled` case for it) lives in
// `src/lib/softwareInstall.ts` — imported at the top of this file. The
// vendor-script-* cases ride along on the same install switch and need
// to be added there in a follow-up; for now they fall through and the
// row shows the generic install-failed wording.

function SoftwareRow({
  descriptor,
  status,
  activity,
  bundlePausedHere,
  disabledOtherBusy,
  canManage,
  enableService: _enableService,
  supportsVersionPick,
  availableVersions,
  versionsLoading,
  selectedVersion,
  selectedVariant,
  expanded,
  details,
  onSelectVersion,
  onSelectVariant,
  onToggleExpand,
  onLoadDetails,
  onLoadVersions,
  onAction,
  onUninstall,
  onServiceAction,
  onViewLogs,
  onCopyCommand,
  onCancel,
  onVendorPick,
  onCdToPath,
  onCleanupRepo,
  hasLiveTerminal,
  onPgQuickConfig,
  quickConfigLabel,
  coInstallSuggestions,
  coInstallDismissed,
  onEnsureCoInstall,
  onInstallCoInstall,
  onDismissCoInstall,
  metrics,
  pulse,
}: {
  descriptor: SoftwareDescriptor;
  status: SoftwarePackageStatus | null;
  activity:
    | {
        installId: string;
        kind: SoftwareActivityKind;
        log: string[];
        error: string;
        busy: boolean;
        cancelling: boolean;
        repoWarnings: string[];
      }
    | null;
  /** `true` when the parent bundle is paused AND this row is the
   *  package the bundle's progress pointer last touched. Drives a
   *  small PAUSED pill at the top of the activity area so the
   *  user spots the pause without having to scroll back to the
   *  bundle card. */
  bundlePausedHere: boolean;
  disabledOtherBusy: boolean;
  canManage: boolean;
  enableService: boolean;
  /** `false` on pacman / unsupported distros — the chevron-down half
   *  of the split button is suppressed because the manager can only
   *  install the latest. */
  supportsVersionPick: boolean;
  /** Cached version list (freshest first) or `null` when never
   *  fetched. The dropdown lazy-loads on open. */
  availableVersions: string[] | null;
  /** A `software_versions_remote` request is in flight for this row. */
  versionsLoading: boolean;
  /** User's pinned version, or `undefined` for "latest". */
  selectedVersion: string | undefined;
  /** User's picked major-version variant (e.g. `"openjdk-21"`).
   *  `undefined` = the descriptor's default packages. Only meaningful
   *  when `descriptor.versionVariants` is non-empty. */
  selectedVariant: string | undefined;
  /** `true` when the row is currently expanded into the details
   *  pane. Drives the chevron rotation and visibility of the pane. */
  expanded: boolean;
  /** Lazy-loaded details payload, sentinel `"loading"`, or
   *  `{ error }`. `null` = never fetched (the chevron click is what
   *  kicks off the fetch). */
  details: SoftwarePackageDetail | "loading" | { error: string } | null;
  /** Toggle the row's expanded state. The panel kicks off the
   *  details fetch on the first open. */
  onToggleExpand: () => void;
  /** Force a re-fetch of the details (used by the "刷新" button in
   *  the details pane on a cached or errored row). */
  onLoadDetails: () => void;
  onSelectVersion: (version: string | undefined) => void;
  /** Pick a variant. `undefined` = use the descriptor's default
   *  install packages (no variant pin). */
  onSelectVariant: (variant: string | undefined) => void;
  /** Trigger the lazy-load of versions for this descriptor. The
   *  panel skips the round-trip when the cache is fresh. */
  onLoadVersions: () => void;
  onAction: (action: "install" | "update") => Promise<void> | void;
  /** Open the uninstall dialog for this row. The panel owns the
   *  dialog state so only one dialog is ever mounted at a time. */
  onUninstall: () => void;
  /** Run `systemctl <verb>` against this row's service. Only ever
   *  called for descriptors where `hasService` is true (the menu
   *  hides the entries otherwise). */
  onServiceAction: (action: SoftwareServiceAction) => void;
  /** Open the journalctl viewer for this row. */
  onViewLogs: () => void;
  /** Synthesise + copy the install command for this row to the
   *  clipboard. Doesn't run anything on the host — the user can
   *  paste it into their own shell to vet before executing. */
  onCopyCommand: (action: "install" | "update") => void;
  /** Trigger backend cancel for the row's in-flight activity. */
  onCancel: () => void;
  /** Open the vendor-script confirm dialog. Only invoked from the
   *  install-channel chooser when the descriptor exposes a
   *  `vendorScript`. */
  onVendorPick: () => void;
  /** Inject `cd <path>` into the tab's terminal (or copy to
   *  clipboard when no terminal session is attached). */
  onCdToPath: (path: string) => void;
  /** Inject a "disable this stale repo" one-liner into the tab's
   *  terminal (or clipboard when no live terminal). Called from the
   *  advisory banner's per-row action button. */
  onCleanupRepo: (warning: string) => void;
  /** `true` when this tab has a live terminal session — the
   *  details pane uses this to label the cd-button as
   *  "→ 终端" vs "复制 cd 命令". */
  hasLiveTerminal: boolean;
  /** Open the service-level quick-config dialog for this row.
   *  Set on rows that have a service-specific helper (postgres,
   *  mariadb, redis); the menu hides the entry when undefined. */
  onPgQuickConfig?: () => void;
  /** Localized menu label for the quick-config entry. Pulled from
   *  the parent so the row stays generic (PG/MySQL/Redis share
   *  the same hook). */
  quickConfigLabel?: string;
  /** Curated "X is commonly installed alongside Y" ids, already
   *  filtered to those not yet installed. The parent populates
   *  this lazily after the row reports installed=true. */
  coInstallSuggestions: string[];
  /** User dismissed the chip strip for this row this session. */
  coInstallDismissed: boolean;
  /** Trigger the lazy co-install fetch. The row does this after
   *  it transitions to installed. */
  onEnsureCoInstall: () => void;
  /** Sequentially install every suggestion in `coInstallSuggestions`. */
  onInstallCoInstall: () => void;
  /** Hide the strip without installing — re-shows on next install. */
  onDismissCoInstall: () => void;
  /** Live DB metrics from the panel's polling effect, or `null`
   *  when descriptor.id isn't a DB / not yet polled. */
  metrics: cmd.DbMetrics | null;
  /** Set briefly to `true` when the dep-graph "jump to row" lands
   *  here. Drives a visual pulse + scroll-anchor target. */
  pulse: boolean;
}) {
  const { t } = useI18n();
  const logRef = useRef<HTMLPreElement>(null);
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const versionButtonRef = useRef<HTMLButtonElement>(null);
  const variantButtonRef = useRef<HTMLButtonElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [versionMenuOpen, setVersionMenuOpen] = useState(false);
  const [variantMenuOpen, setVariantMenuOpen] = useState(false);
  const channelButtonRef = useRef<HTMLButtonElement>(null);
  const [channelMenuOpen, setChannelMenuOpen] = useState(false);
  const hasVariants = descriptor.versionVariants.length > 0;
  const variantLabel = hasVariants
    ? descriptor.versionVariants.find((v) => v.key === selectedVariant)?.label
    : undefined;
  const installed = status?.installed ?? false;
  const version = status?.version ?? null;
  const serviceActive = status?.serviceActive ?? null;
  const busy = activity?.busy ?? false;
  const cancelling = activity?.cancelling ?? false;
  const action: "install" | "update" = installed ? "update" : "install";
  const buttonDisabled = busy || disabledOtherBusy || !canManage;
  const menuDisabled = busy || disabledOtherBusy;
  // Only offer service controls when (a) the descriptor declares a
  // service unit and (b) the package is actually installed. We don't
  // hide them on `serviceActive === null` (which can mean systemctl
  // isn't on the host) — the action itself will surface a clear
  // failure if it can't run.
  const showServiceControls = descriptor.hasService && installed;
  // Split-button chevron only shows on the install path. Once the
  // package is installed, "更新" goes straight through the apt path —
  // vendor scripts (get.docker.com) are install-only by design.
  const showChannelChooser =
    !installed && descriptor.vendorScript != null;

  // Auto-scroll the log to the latest line as it streams in.
  useEffect(() => {
    if (!activity || !logRef.current) return;
    logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [activity?.log.length]);

  // After the row reports installed, fetch curated co-install
  // suggestions exactly once. The parent caches the result so a
  // re-render won't re-trigger the round-trip.
  useEffect(() => {
    if (installed) onEnsureCoInstall();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [installed]);

  // When versions arrive for an already-installed package whose
  // installed version differs from the freshest available, default
  // the [Update] button to the latest one — without this, clicking
  // [Update] would just re-pull whatever the manager picks (often
  // already-installed = no-op). Only fires once per cache refresh
  // and never overrides an explicit user pick (which sets selectedVersion).
  useEffect(() => {
    if (
      action === "update" &&
      selectedVersion === undefined &&
      availableVersions &&
      availableVersions.length > 0 &&
      version &&
      availableVersions[0] !== version
    ) {
      onSelectVersion(availableVersions[0]);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [availableVersions]);

  const StatusIcon = busy ? Loader : installed ? Check : Circle;
  return (
    <div
      id={`sw-row-${descriptor.id}`}
      className={`sw-row${pulse ? " is-pulse" : ""}`}
    >
      <div className="sw-row__head">
        <button
          type="button"
          className="icon-btn sw-row__expand-btn"
          onClick={onToggleExpand}
          title={expanded ? t("Hide details") : t("Show details")}
          aria-expanded={expanded}
        >
          {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </button>
        <span
          className={`sw-row__status sw-row__status--${
            busy ? "busy" : installed ? "ok" : "missing"
          }`}
        >
          <StatusIcon size={12} className={busy ? "sw-row__spin" : undefined} />
        </span>
        <span className="sw-row__name">{descriptor.displayName}</span>
        <span className="sw-row__version mono">
          {installed && version ? `v ${version}` : ""}
          {installed && descriptor.hasService && serviceActive !== null && (
            <span
              className="sw-row__service-pill"
              title={
                serviceActive
                  ? t("service running")
                  : t("service stopped")
              }
            >
              <StatusDot tone={serviceActive ? "pos" : "neg"} />
            </span>
          )}
        </span>
        <span className="sw-row__actions">
          {busy ? (
            <button
              type="button"
              className="btn is-danger is-compact"
              disabled={cancelling}
              onClick={onCancel}
              title={t(
                "Cancel signal sent — the remote may still be running.",
              )}
            >
              <X size={10} />
              {cancelling ? t("Cancelling...") : t("Cancel")}
            </button>
          ) : (
            <span className="sw-row__split-btn">
              <button
                type="button"
                className={`btn is-primary is-compact${
                  supportsVersionPick ? " sw-row__split-btn-main" : ""
                }`}
                disabled={buttonDisabled}
                onClick={() => void onAction(action)}
              >
                <Download size={10} />
                {primaryButtonLabel({
                  t,
                  action,
                  busy,
                  activityKind: activity?.kind,
                  selectedVersion,
                  variantLabel,
                })}
              </button>
              {supportsVersionPick && (
                <button
                  ref={versionButtonRef}
                  type="button"
                  className="btn is-primary is-compact sw-row__split-btn-chevron"
                  disabled={buttonDisabled}
                  title={t("Pick version...")}
                  onClick={() => {
                    setVersionMenuOpen((cur) => {
                      const opening = !cur;
                      if (opening) onLoadVersions();
                      return opening;
                    });
                  }}
                >
                  <ChevronDown size={10} />
                </button>
              )}
              <Popover
                open={versionMenuOpen}
                anchor={versionButtonRef.current}
                onClose={() => setVersionMenuOpen(false)}
                placement="bottom-end"
                width={220}
                className="ctx-menu sw-row-version-menu"
              >
                <button
                  type="button"
                  className="ctx-menu__item"
                  onClick={() => {
                    onSelectVersion(undefined);
                    setVersionMenuOpen(false);
                  }}
                >
                  <span className="ctx-menu__label">
                    <span className="sw-row-version-menu__check">
                      {selectedVersion === undefined && <Check size={10} />}
                    </span>
                    {t("Latest")}
                  </span>
                </button>
                {versionsLoading && (
                  <div className="sw-row-version-menu__hint">
                    {t("Loading versions...")}
                  </div>
                )}
                {!versionsLoading &&
                  availableVersions !== null &&
                  availableVersions.length === 0 && (
                    <div className="sw-row-version-menu__hint">
                      {t("No specific versions available")}
                    </div>
                  )}
                {!versionsLoading &&
                  availableVersions?.map((v) => (
                    <button
                      key={v}
                      type="button"
                      className="ctx-menu__item"
                      onClick={() => {
                        onSelectVersion(v);
                        setVersionMenuOpen(false);
                      }}
                    >
                      <span className="ctx-menu__label">
                        <span className="sw-row-version-menu__check">
                          {selectedVersion === v && <Check size={10} />}
                        </span>
                        <span className="mono">{v}</span>
                      </span>
                    </button>
                  ))}
              </Popover>
            </span>
          )}
          {hasVariants && !busy && (
            <>
              <button
                ref={variantButtonRef}
                type="button"
                className="btn is-ghost is-compact sw-row__variant-btn"
                disabled={buttonDisabled}
                title={t("Pick major version")}
                onClick={() => setVariantMenuOpen((cur) => !cur)}
              >
                {variantLabel ?? t("Variant")}
                <ChevronDown size={10} />
              </button>
              <Popover
                open={variantMenuOpen}
                anchor={variantButtonRef.current}
                onClose={() => setVariantMenuOpen(false)}
                placement="bottom-end"
                width={200}
                className="ctx-menu sw-row-variant-menu"
              >
                <button
                  type="button"
                  className="ctx-menu__item"
                  onClick={() => {
                    onSelectVariant(undefined);
                    setVariantMenuOpen(false);
                  }}
                >
                  <span className="ctx-menu__label">
                    <span className="sw-row-version-menu__check">
                      {selectedVariant === undefined && <Check size={10} />}
                    </span>
                    {t("Default (recommended)")}
                  </span>
                </button>
                {descriptor.versionVariants.map((v) => (
                  <button
                    key={v.key}
                    type="button"
                    className="ctx-menu__item"
                    onClick={() => {
                      onSelectVariant(v.key);
                      setVariantMenuOpen(false);
                    }}
                  >
                    <span className="ctx-menu__label">
                      <span className="sw-row-version-menu__check">
                        {selectedVariant === v.key && <Check size={10} />}
                      </span>
                      {v.label}
                    </span>
                  </button>
                ))}
              </Popover>
            </>
          )}
          {showChannelChooser && (
            <button
              ref={channelButtonRef}
              type="button"
              className="btn is-primary is-compact sw-row__primary-chevron"
              onClick={() => setChannelMenuOpen((cur) => !cur)}
              disabled={buttonDisabled}
              title={t("Choose install channel")}
              aria-label={t("Choose install channel")}
            >
              <ChevronDown size={10} />
            </button>
          )}
          {showChannelChooser && (
            <Popover
              open={channelMenuOpen}
              anchor={channelButtonRef.current}
              onClose={() => setChannelMenuOpen(false)}
              placement="bottom-end"
              width={220}
              className="ctx-menu sw-channel-menu"
            >
              <button
                type="button"
                className="ctx-menu__item"
                onClick={() => {
                  setChannelMenuOpen(false);
                  void onAction(action);
                }}
              >
                <span className="ctx-menu__label">{t("Install via apt (default)")}</span>
              </button>
              <button
                type="button"
                className="ctx-menu__item sw-channel-menu__vendor"
                onClick={() => {
                  setChannelMenuOpen(false);
                  onVendorPick();
                }}
              >
                <span className="ctx-menu__label">
                  {t("Install via {label}", {
                    label: descriptor.vendorScript?.label ?? "",
                  })}
                </span>
              </button>
            </Popover>
          )}
          <button
            ref={menuButtonRef}
            type="button"
            className="icon-btn"
            onClick={() => setMenuOpen((cur) => !cur)}
            disabled={menuDisabled}
            title={t("More actions")}
          >
            <MoreHorizontal size={12} />
          </button>
          <Popover
            open={menuOpen}
            anchor={menuButtonRef.current}
            onClose={() => setMenuOpen(false)}
            placement="bottom-end"
            width={200}
            className="ctx-menu sw-row-menu"
          >
            {showServiceControls && (
              <>
                <button
                  type="button"
                  className="ctx-menu__item"
                  onClick={() => {
                    setMenuOpen(false);
                    onServiceAction("restart");
                  }}
                >
                  <span className="ctx-menu__label">
                    <RotateCw size={12} />
                    {t("Restart service")}
                  </span>
                </button>
                {descriptor.supportsReload && (
                  <button
                    type="button"
                    className="ctx-menu__item"
                    onClick={() => {
                      setMenuOpen(false);
                      onServiceAction("reload");
                    }}
                  >
                    <span className="ctx-menu__label">
                      <Zap size={12} />
                      {t("Reload (no downtime)")}
                    </span>
                  </button>
                )}
                {serviceActive === false ? (
                  <button
                    type="button"
                    className="ctx-menu__item"
                    onClick={() => {
                      setMenuOpen(false);
                      onServiceAction("start");
                    }}
                  >
                    <span className="ctx-menu__label">
                      <Play size={12} />
                      {t("Start service")}
                    </span>
                  </button>
                ) : (
                  <button
                    type="button"
                    className="ctx-menu__item"
                    onClick={() => {
                      setMenuOpen(false);
                      onServiceAction("stop");
                    }}
                  >
                    <span className="ctx-menu__label">
                      <Square size={12} />
                      {t("Stop service")}
                    </span>
                  </button>
                )}
                <button
                  type="button"
                  className="ctx-menu__item"
                  onClick={() => {
                    setMenuOpen(false);
                    onViewLogs();
                  }}
                >
                  <span className="ctx-menu__label">
                    <FileText size={12} />
                    {t("View logs")}
                  </span>
                </button>
                <div className="sw-row-menu__divider" role="separator" />
              </>
            )}
            {canManage && (
              <button
                type="button"
                className="ctx-menu__item"
                onClick={() => {
                  setMenuOpen(false);
                  onCopyCommand(action);
                }}
              >
                <span className="ctx-menu__label">
                  <Copy size={12} />
                  {action === "update"
                    ? t("Copy update command")
                    : t("Copy install command")}
                </span>
              </button>
            )}
            {installed && onPgQuickConfig && (
              <button
                type="button"
                className="ctx-menu__item"
                onClick={() => {
                  setMenuOpen(false);
                  onPgQuickConfig();
                }}
              >
                <span className="ctx-menu__label">
                  <Zap size={12} />
                  {quickConfigLabel ?? t("Quick config...")}
                </span>
              </button>
            )}
            <button
              type="button"
              className="ctx-menu__item sw-row-menu__danger"
              onClick={() => {
                setMenuOpen(false);
                onUninstall();
              }}
              disabled={!installed}
            >
              <span className="ctx-menu__label">
                <Trash2 size={12} />
                {t("Uninstall")}
              </span>
            </button>
            {!installed && (
              <div className="sw-row-menu__hint">
                {t("Install before you can uninstall.")}
              </div>
            )}
          </Popover>
        </span>
      </div>
      {descriptor.notes && (
        <div className="sw-row__note mono">{descriptor.notes}</div>
      )}
      {installed &&
        !coInstallDismissed &&
        coInstallSuggestions.length > 0 &&
        !busy && (
          <div className="sw-row__co-install mono">
            <span className="sw-row__co-install-label">
              {t("Commonly installed alongside:")}
            </span>
            {coInstallSuggestions.map((id) => (
              <span key={id} className="sw-record-bundle__chip">
                {id}
              </span>
            ))}
            <button
              type="button"
              className="btn is-primary is-compact sw-row__co-install-btn"
              onClick={onInstallCoInstall}
              disabled={disabledOtherBusy}
            >
              <Download size={10} /> {t("Install all")}
            </button>
            <button
              type="button"
              className="icon-btn"
              onClick={onDismissCoInstall}
              title={t("Dismiss suggestions")}
            >
              <X size={10} />
            </button>
          </div>
        )}
      {activity &&
        (activity.busy ||
          activity.log.length > 0 ||
          activity.error ||
          activity.repoWarnings.length > 0) && (
          <>
            {bundlePausedHere && (
              <div className="sw-row__paused mono">
                <span className="sw-row__paused-pill">{t("PAUSED")}</span>
                <span className="muted">
                  {t(
                    "Bundle is paused — current package will start as soon as you click Resume.",
                  )}
                </span>
              </div>
            )}
            {activity.error && (
              <div className="status-note status-note--error mono sw-row__error">
                {activity.error}
              </div>
            )}
            {activity.repoWarnings.length > 0 && (
              <div className="status-note status-note--warn mono sw-row__warn">
                <div className="sw-row__warn-title">
                  {t(
                    "Stale third-party repos detected — install proceeded against the cached index. Clean these up on the host to silence the warnings:",
                  )}
                </div>
                <ul className="sw-row__warn-list">
                  {activity.repoWarnings.map((w) => {
                    const cleanable =
                      buildRepoCleanupCommand(w).length > 0;
                    return (
                      <li key={w}>
                        <span className="sw-row__warn-ident">{w}</span>
                        {cleanable && (
                          <button
                            type="button"
                            className="mini-button sw-row__warn-action"
                            onClick={(e) => {
                              e.stopPropagation();
                              onCleanupRepo(w);
                            }}
                            title={
                              hasLiveTerminal
                                ? t(
                                    "Inject a disable command into the active terminal — review then press Enter to run.",
                                  )
                                : t(
                                    "Copy a disable command to the clipboard — paste into a terminal, review, then run.",
                                  )
                            }
                          >
                            {hasLiveTerminal
                              ? t("Disable in terminal")
                              : t("Copy disable command")}
                          </button>
                        )}
                      </li>
                    );
                  })}
                </ul>
              </div>
            )}
            {activity.log.length > 0 && (
              <pre ref={logRef} className="install-log mono sw-row__log">
                {activity.log.join("\n")}
              </pre>
            )}
          </>
        )}
      {expanded && (
        <SoftwareRowDetails
          descriptor={descriptor}
          status={status}
          details={details}
          onRefresh={onLoadDetails}
          onCdToPath={onCdToPath}
          hasLiveTerminal={hasLiveTerminal}
          metrics={metrics}
        />
      )}
    </div>
  );
}

/** Parse a pasted install command (`apt install foo bar` /
 *  `dnf install -y baz`) into a list of package ids; let the
 *  user name + describe the bundle and write it to
 *  `software-extras.json` so it appears in the panel's bundle
 *  cards on the next launch. */
function RecordBundleDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [raw, setRaw] = useState("");
  const [bundleId, setBundleId] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");

  useEffect(() => {
    if (!open) return;
    setRaw("");
    setBundleId("");
    setDisplayName("");
    setDescription("");
    setMessage("");
  }, [open]);

  // Strip flags, manager prefix, "install" verb; collect
  // everything left as package names. Handles:
  //   sudo apt install -y foo bar
  //   apt-get install --no-install-recommends foo
  //   dnf install -y foo
  //   pacman -S --noconfirm foo
  //   apk add foo
  //   yum install foo
  //   zypper install foo
  // Multiline / multiple commands on `&&` / `;` are concatenated.
  const parsed = useMemo<string[]>(() => {
    const STOP = new Set([
      "sudo",
      "apt",
      "apt-get",
      "dnf",
      "yum",
      "apk",
      "pacman",
      "zypper",
      "install",
      "add",
      "-S",
      "-y",
      "--no-install-recommends",
      "--noconfirm",
      "--non-interactive",
      "DEBIAN_FRONTEND=noninteractive",
    ]);
    const tokens = raw
      .split(/[\n;&]+/)
      .flatMap((cmd) => cmd.split(/\s+/))
      .map((t) => t.trim())
      .filter(Boolean);
    const out: string[] = [];
    for (const tok of tokens) {
      if (STOP.has(tok)) continue;
      if (tok.startsWith("-")) continue;
      // Skip "key=value" env-var prefixes that aren't in STOP.
      if (tok.includes("=") && !tok.includes("/")) continue;
      // De-dup while preserving order.
      if (!out.includes(tok)) out.push(tok);
    }
    return out;
  }, [raw]);

  if (!open) return null;
  const canSave =
    !busy &&
    parsed.length > 0 &&
    bundleId.trim().length > 0 &&
    displayName.trim().length > 0;

  async function handleSave() {
    setBusy(true);
    setMessage("");
    try {
      // Read existing extras (or treat empty file as starting fresh).
      const existing = await cmd.softwareUserExtrasRead();
      const trimmed = existing.trim();
      let wrapper: { packages?: unknown[]; bundles?: unknown[] };
      if (!trimmed) {
        wrapper = { packages: [], bundles: [] };
      } else {
        const parsedJson = JSON.parse(trimmed);
        if (Array.isArray(parsedJson)) {
          wrapper = { packages: parsedJson, bundles: [] };
        } else if (parsedJson && typeof parsedJson === "object") {
          wrapper = parsedJson as typeof wrapper;
        } else {
          throw new Error("extras root must be an array or object");
        }
      }
      const bundles = Array.isArray(wrapper.bundles) ? wrapper.bundles : [];
      bundles.push({
        id: bundleId.trim(),
        displayName: displayName.trim(),
        description: description.trim(),
        packageIds: parsed,
      });
      const next = { ...wrapper, bundles };
      await cmd.softwareUserExtrasWrite(JSON.stringify(next, null, 2));
      setMessage(t("Bundle saved. Restart Pier-X to see it in the cards."));
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={open}
      title={t("Record install command as bundle")}
      subtitle={t(
        "Paste a one-shot install command. Pier-X extracts the package names and writes a bundle entry to software-extras.json.",
      )}
      size="md"
      onClose={onClose}
    >
      <div className="sw-record-bundle">
        <textarea
          className="sw-record-bundle__textarea mono"
          placeholder={"sudo apt install -y nginx redis-server git curl"}
          value={raw}
          onChange={(e) => setRaw(e.currentTarget.value)}
          spellCheck={false}
          rows={4}
        />
        <div className="sw-record-bundle__parsed mono">
          {parsed.length === 0 ? (
            <span className="sw-record-bundle__parsed-empty">
              {t("Parsed packages will appear here.")}
            </span>
          ) : (
            <>
              {t("Parsed:")}{" "}
              {parsed.map((p) => (
                <span key={p} className="sw-record-bundle__chip">
                  {p}
                </span>
              ))}
            </>
          )}
        </div>
        <div className="sw-record-bundle__row">
          <input
            className="dlg-input"
            value={bundleId}
            onChange={(e) => setBundleId(e.currentTarget.value)}
            placeholder={t("bundle id (e.g. my-stack)")}
            spellCheck={false}
          />
          <input
            className="dlg-input"
            value={displayName}
            onChange={(e) => setDisplayName(e.currentTarget.value)}
            placeholder={t("display name")}
          />
        </div>
        <input
          className="dlg-input"
          value={description}
          onChange={(e) => setDescription(e.currentTarget.value)}
          placeholder={t("description (optional)")}
        />
        {message && <div className="sw-extras-editor__msg mono">{message}</div>}
        <div className="sw-extras-editor__actions">
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={onClose}
            disabled={busy}
          >
            {t("Close")}
          </button>
          <button
            type="button"
            className="btn is-primary is-compact"
            onClick={() => void handleSave()}
            disabled={!canSave}
          >
            {busy ? t("Saving...") : t("Save bundle")}
          </button>
        </div>
      </div>
    </Dialog>
  );
}

/** PostgreSQL quick-config dialog. Three independent forms:
 *  create role, create database, allow remote connections. Each
 *  form has its own outcome area so users can run them
 *  out of order. */
function PgQuickConfigDialog({
  target,
  sshParams,
  onClose,
}: {
  target: SoftwareDescriptor | null;
  sshParams: SshParams | null;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [pgUser, setPgUser] = useState("piertest");
  const [pgPass, setPgPass] = useState("");
  const [isSuper, setIsSuper] = useState(false);
  const [dbName, setDbName] = useState("");
  const [dbOwner, setDbOwner] = useState("piertest");
  const [busy, setBusy] = useState<"user" | "db" | "remote" | null>(null);
  const [userMsg, setUserMsg] = useState("");
  const [dbMsg, setDbMsg] = useState("");
  const [remoteMsg, setRemoteMsg] = useState("");

  // Reset every time a different target opens.
  useEffect(() => {
    if (!target) return;
    setUserMsg("");
    setDbMsg("");
    setRemoteMsg("");
  }, [target?.id]);

  if (!target || !sshParams) return null;

  function describePg(report: cmd.PostgresActionReport): string {
    if (report.status === "ok") return t("Done.");
    if (report.status === "sudo-requires-password") {
      return t(
        "sudo requires a password — connect as root or configure passwordless sudo.",
      );
    }
    return t("Failed (exit {code}). {tail}", {
      code: report.exitCode,
      tail: report.outputTail.split("\n").slice(-1)[0] ?? "",
    });
  }

  async function handleCreateUser() {
    if (busy || !pgUser.trim() || !pgPass) return;
    setBusy("user");
    setUserMsg("");
    try {
      const r = await cmd.postgresCreateUserRemote({
        ...sshParams!,
        pgUsername: pgUser,
        pgPassword: pgPass,
        isSuperuser: isSuper,
      });
      setUserMsg(describePg(r));
    } catch (e) {
      setUserMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function handleCreateDb() {
    if (busy || !dbName.trim() || !dbOwner.trim()) return;
    setBusy("db");
    setDbMsg("");
    try {
      const r = await cmd.postgresCreateDbRemote({
        ...sshParams!,
        dbName,
        owner: dbOwner,
      });
      setDbMsg(describePg(r));
    } catch (e) {
      setDbMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function handleOpenRemote() {
    if (busy) return;
    setBusy("remote");
    setRemoteMsg("");
    try {
      const r = await cmd.postgresOpenRemote(sshParams!);
      setRemoteMsg(describePg(r));
    } catch (e) {
      setRemoteMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <Dialog
      open={!!target}
      title={t("PostgreSQL quick config")}
      subtitle={t("Run common post-install setup tasks against the local cluster.")}
      size="md"
      onClose={onClose}
    >
      <div className="sw-pg-form">
        <fieldset className="sw-pg-form__section" disabled={busy !== null}>
          <legend>{t("Create role")}</legend>
          <div className="sw-pg-form__row">
            <input
              className="dlg-input"
              value={pgUser}
              onChange={(e) => setPgUser(e.currentTarget.value)}
              placeholder={t("username")}
              spellCheck={false}
              autoCorrect="off"
            />
            <input
              className="dlg-input"
              type="password"
              value={pgPass}
              onChange={(e) => setPgPass(e.currentTarget.value)}
              placeholder={t("password")}
              autoComplete="new-password"
            />
            <label className="sw-pg-form__check">
              <input
                type="checkbox"
                checked={isSuper}
                onChange={(e) => setIsSuper(e.currentTarget.checked)}
              />
              {t("superuser")}
            </label>
            <button
              type="button"
              className="btn is-primary is-compact"
              onClick={() => void handleCreateUser()}
              disabled={!pgUser.trim() || !pgPass}
            >
              {busy === "user" ? t("Running...") : t("Create / update")}
            </button>
          </div>
          {userMsg && <div className="sw-pg-form__msg mono">{userMsg}</div>}
        </fieldset>

        <fieldset className="sw-pg-form__section" disabled={busy !== null}>
          <legend>{t("Create database")}</legend>
          <div className="sw-pg-form__row">
            <input
              className="dlg-input"
              value={dbName}
              onChange={(e) => setDbName(e.currentTarget.value)}
              placeholder={t("database name")}
              spellCheck={false}
              autoCorrect="off"
            />
            <input
              className="dlg-input"
              value={dbOwner}
              onChange={(e) => setDbOwner(e.currentTarget.value)}
              placeholder={t("owner role")}
              spellCheck={false}
              autoCorrect="off"
            />
            <button
              type="button"
              className="btn is-primary is-compact"
              onClick={() => void handleCreateDb()}
              disabled={!dbName.trim() || !dbOwner.trim()}
            >
              {busy === "db" ? t("Running...") : t("Create")}
            </button>
          </div>
          {dbMsg && <div className="sw-pg-form__msg mono">{dbMsg}</div>}
        </fieldset>

        <fieldset className="sw-pg-form__section" disabled={busy !== null}>
          <legend>{t("Allow remote connections")}</legend>
          <div className="sw-pg-form__hint">
            {t(
              "Sets listen_addresses = '*' in postgresql.conf and appends 'host all all 0.0.0.0/0 md5' to pg_hba.conf, then reloads. Restart may be required for listen_addresses.",
            )}
          </div>
          <div className="sw-pg-form__row">
            <button
              type="button"
              className="btn is-danger is-compact"
              onClick={() => void handleOpenRemote()}
            >
              {busy === "remote" ? t("Running...") : t("Open to 0.0.0.0/0")}
            </button>
          </div>
          {remoteMsg && <div className="sw-pg-form__msg mono">{remoteMsg}</div>}
        </fieldset>
      </div>
    </Dialog>
  );
}

/** Co-install dependency graph. Renders the curated suggestion
 *  map as an SVG: nodes = registry entries, edges = "X often
 *  comes with Y". Click a node to dismiss the dialog and pulse
 *  that row in the panel. Layout is a simple circle — sufficient
 *  for ~20 nodes; a force-directed layout is overkill at this scale. */
function DepGraphDialog({
  open,
  registry,
  statuses,
  onClose,
  onJump,
}: {
  open: boolean;
  registry: SoftwareDescriptor[];
  statuses: Record<string, SoftwarePackageStatus>;
  onClose: () => void;
  onJump: (id: string) => void;
}) {
  const { t } = useI18n();
  const [edges, setEdges] = useState<{ from: string; to: string }[]>([]);
  const [nodeIds, setNodeIds] = useState<string[]>([]);

  // Fetch co-install map for every descriptor. Single batch on
  // open; deduplicate edges so undirected duplicates collapse.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    (async () => {
      const out: { from: string; to: string }[] = [];
      const ids = new Set<string>();
      for (const d of registry) {
        try {
          const sugg = await cmd.softwareCoInstallSuggestions(d.id);
          for (const s of sugg) {
            // Filter to suggestions that exist in the registry.
            if (!registry.some((r) => r.id === s)) continue;
            // Deduplicate undirected: store as sorted pair.
            const a = d.id < s ? d.id : s;
            const b = d.id < s ? s : d.id;
            if (!out.some((e) => e.from === a && e.to === b)) {
              out.push({ from: a, to: b });
            }
            ids.add(d.id);
            ids.add(s);
          }
        } catch {
          /* skip */
        }
      }
      if (!cancelled) {
        setEdges(out);
        setNodeIds([...ids].sort());
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, registry]);

  if (!open) return null;
  // Circular layout: place each node on a circle; edges are
  // straight lines through the centre. SVG viewBox is fixed so
  // the layout stays stable as the dialog resizes.
  const W = 540;
  const H = 460;
  const cx = W / 2;
  const cy = H / 2;
  const R = Math.min(W, H) / 2 - 60;
  const positions = new Map<string, { x: number; y: number }>();
  nodeIds.forEach((id, i) => {
    const angle = (i / nodeIds.length) * Math.PI * 2 - Math.PI / 2;
    positions.set(id, {
      x: cx + R * Math.cos(angle),
      y: cy + R * Math.sin(angle),
    });
  });

  return (
    <Dialog
      open={open}
      title={t("Co-install dependency graph")}
      subtitle={t(
        "Curated 'commonly installed alongside' edges. Click a node to jump to its row.",
      )}
      size="md"
      onClose={onClose}
    >
      <div className="sw-depgraph">
        {nodeIds.length === 0 ? (
          <div className="sw-panel__empty mono">
            {t("No co-install relationships found.")}
          </div>
        ) : (
          <svg
            viewBox={`0 0 ${W} ${H}`}
            className="sw-depgraph__svg"
            preserveAspectRatio="xMidYMid meet"
          >
            {edges.map((e, i) => {
              const a = positions.get(e.from);
              const b = positions.get(e.to);
              if (!a || !b) return null;
              return (
                <line
                  key={`e-${i}`}
                  x1={a.x}
                  y1={a.y}
                  x2={b.x}
                  y2={b.y}
                  className="sw-depgraph__edge"
                />
              );
            })}
            {nodeIds.map((id) => {
              const p = positions.get(id);
              if (!p) return null;
              const installed = !!statuses[id]?.installed;
              return (
                <g
                  key={id}
                  className="sw-depgraph__node"
                  transform={`translate(${p.x} ${p.y})`}
                  onClick={() => onJump(id)}
                >
                  <circle
                    r={14}
                    className={`sw-depgraph__circle${
                      installed ? " is-installed" : ""
                    }`}
                  />
                  <text className="sw-depgraph__label" y={28}>
                    {id}
                  </text>
                </g>
              );
            })}
          </svg>
        )}
      </div>
    </Dialog>
  );
}

/** Clone-host dialog: pick a source SSH connection, fetch its
 *  user-installed package set, filter to ones the registry knows
 *  how to install, then deploy that subset to one or more target
 *  hosts. Targets run sequentially with per-host progress. */
function CloneHostsDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [hosts, setHosts] = useState<SavedSshConnection[]>([]);
  const [sourceIdx, setSourceIdx] = useState<number | null>(null);
  const [plan, setPlan] = useState<cmd.ClonePlan | null>(null);
  const [planBusy, setPlanBusy] = useState(false);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [targets, setTargets] = useState<Set<number>>(new Set());
  const [running, setRunning] = useState(false);
  const [perTarget, setPerTarget] = useState<Record<number, string>>({});
  const [showAll, setShowAll] = useState(false);

  useEffect(() => {
    if (!open) return;
    setSourceIdx(null);
    setPlan(null);
    setPicked(new Set());
    setTargets(new Set());
    setPerTarget({});
    cmd
      .sshConnectionsList()
      .then(setHosts)
      .catch(() => setHosts([]));
  }, [open]);

  if (!open) return null;
  const sourceConn = hosts.find((h) => h.index === sourceIdx) ?? null;

  async function loadPlan() {
    if (!sourceConn) return;
    setPlanBusy(true);
    setPlan(null);
    try {
      const params: SshParams = {
        host: sourceConn.host,
        port: sourceConn.port,
        user: sourceConn.user,
        authMode:
          sourceConn.authKind === "password" ? "password" : sourceConn.authKind,
        password: "",
        keyPath: sourceConn.keyPath,
        savedConnectionIndex: sourceConn.index,
      };
      const p = await cmd.softwareClonePlan(params);
      setPlan(p);
      // Pre-pick all entries the registry resolved.
      setPicked(
        new Set(
          p.entries
            .filter((e) => e.descriptorId !== null)
            .map((e) => e.descriptorId as string),
        ),
      );
    } catch (e) {
      setPerTarget({ [-1]: e instanceof Error ? e.message : String(e) });
    } finally {
      setPlanBusy(false);
    }
  }

  async function runClone() {
    if (running || !plan || picked.size === 0 || targets.size === 0) return;
    setRunning(true);
    setPerTarget({});
    try {
      for (const tIdx of targets) {
        const target = hosts.find((h) => h.index === tIdx);
        if (!target) continue;
        setPerTarget((prev) => ({
          ...prev,
          [tIdx]: t("Probing target..."),
        }));
        const params: SshParams = {
          host: target.host,
          port: target.port,
          user: target.user,
          authMode: target.authKind === "password" ? "password" : target.authKind,
          password: "",
          keyPath: target.keyPath,
          savedConnectionIndex: target.index,
        };
        try {
          const probe = await cmd.softwareProbeRemote(params);
          const already = new Set(
            probe.statuses.filter((s) => s.installed).map((s) => s.id),
          );
          const todo = Array.from(picked).filter((id) => !already.has(id));
          if (todo.length === 0) {
            setPerTarget((prev) => ({
              ...prev,
              [tIdx]: t("Already complete."),
            }));
            continue;
          }
          let okCount = 0;
          for (const pkgId of todo) {
            const installId =
              typeof crypto !== "undefined" && "randomUUID" in crypto
                ? crypto.randomUUID()
                : `${Date.now()}-${Math.random()}`;
            // eslint-disable-next-line no-await-in-loop
            const r = await cmd.softwareInstallRemote({
              ...params,
              packageId: pkgId,
              installId,
              enableService: true,
            });
            if (r.status === "installed") okCount += 1;
            setPerTarget((prev) => ({
              ...prev,
              [tIdx]: t("{ok}/{n} installed", { ok: okCount, n: todo.length }),
            }));
          }
        } catch (e) {
          setPerTarget((prev) => ({
            ...prev,
            [tIdx]: e instanceof Error ? e.message : String(e),
          }));
        }
      }
    } finally {
      setRunning(false);
    }
  }

  return (
    <Dialog
      open={open}
      title={t("Clone packages across hosts")}
      subtitle={t(
        "Replicate one host's manually-installed package set onto one or more targets. Only registry-known packages are cloned.",
      )}
      size="md"
      onClose={onClose}
    >
      <div className="sw-multihost">
        <div className="sw-multihost__action">
          <label className="sw-multihost__action-label mono">
            {t("Source host")}:
          </label>
          <select
            className="dlg-input"
            value={sourceIdx ?? ""}
            onChange={(e) => {
              const v = e.currentTarget.value;
              setSourceIdx(v ? Number(v) : null);
              setPlan(null);
              setPicked(new Set());
            }}
            disabled={running || planBusy}
          >
            <option value="">{t("(select)")}</option>
            {hosts.map((h) => (
              <option key={h.index} value={h.index}>
                {h.name || `${h.user}@${h.host}`}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={() => void loadPlan()}
            disabled={!sourceConn || planBusy || running}
          >
            {planBusy ? t("Loading...") : t("Inspect")}
          </button>
        </div>

        {plan && (
          <>
            <div className="sw-clone__summary mono">
              {t("{n} explicitly installed; {k} known to Pier-X registry.", {
                n: plan.entries.length,
                k: plan.entries.filter((e) => e.descriptorId).length,
              })}
              <button
                type="button"
                className="btn is-ghost is-compact"
                onClick={() => setShowAll((v) => !v)}
              >
                {showAll ? t("Show known only") : t("Show all")}
              </button>
            </div>
            <div className="sw-clone__list">
              {plan.entries
                .filter((e) => showAll || e.descriptorId !== null)
                .map((e) => {
                  const id = e.descriptorId;
                  const checked = id !== null && picked.has(id);
                  return (
                    <label
                      key={e.package}
                      className={`sw-clone__row${
                        id === null ? " is-unresolved" : ""
                      }`}
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        disabled={id === null || running}
                        onChange={() => {
                          if (id === null) return;
                          setPicked((prev) => {
                            const next = new Set(prev);
                            if (next.has(id)) next.delete(id);
                            else next.add(id);
                            return next;
                          });
                        }}
                      />
                      <span className="sw-clone__pkg mono">{e.package}</span>
                      {id ? (
                        <span className="sw-clone__resolved mono">→ {id}</span>
                      ) : (
                        <span className="sw-clone__unresolved mono">
                          {t("(not in registry)")}
                        </span>
                      )}
                    </label>
                  );
                })}
            </div>
          </>
        )}

        {plan && (
          <>
            <div className="sw-multihost__hosts-head mono">
              {t("Target hosts")}
            </div>
            {hosts
              .filter((h) => h.index !== sourceIdx)
              .map((h) => {
                const status = perTarget[h.index];
                return (
                  <label key={h.index} className="sw-multihost__host">
                    <input
                      type="checkbox"
                      checked={targets.has(h.index)}
                      onChange={() => {
                        setTargets((prev) => {
                          const next = new Set(prev);
                          if (next.has(h.index)) next.delete(h.index);
                          else next.add(h.index);
                          return next;
                        });
                      }}
                      disabled={running}
                    />
                    <span className="sw-multihost__host-name">
                      {h.name || `${h.user}@${h.host}`}
                    </span>
                    <span className="sw-multihost__host-target mono">
                      {h.user}@{h.host}:{h.port}
                    </span>
                    <span></span>
                    {status && (
                      <span className="sw-multihost__host-status sw-multihost__host-status--running">
                        {status}
                      </span>
                    )}
                  </label>
                );
              })}
          </>
        )}

        <div className="sw-multihost__actions">
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={onClose}
            disabled={running}
          >
            {t("Close")}
          </button>
          <button
            type="button"
            className="btn is-primary is-compact"
            disabled={running || !plan || picked.size === 0 || targets.size === 0}
            onClick={() => void runClone()}
          >
            {running
              ? t("Running...")
              : t("Clone {k} package(s) to {n} host(s)", {
                  k: picked.size,
                  n: targets.size,
                })}
          </button>
        </div>
      </div>
    </Dialog>
  );
}

/** Docker Compose templates dialog. Lists curated stacks; each
 *  card has an "Apply" button (writes the YAML and runs
 *  `docker compose up -d`) and a "Down" button to tear it back
 *  down. Output of the most recent action shows under the cards. */
function ComposeTemplatesDialog({
  target,
  sshParams,
  onClose,
}: {
  target: SoftwareDescriptor | null;
  sshParams: SshParams | null;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [templates, setTemplates] = useState<cmd.ComposeTemplate[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [previewId, setPreviewId] = useState<string | null>(null);
  // Per-card K8s export state. We hold the most recent conversion
  // result keyed by template id so a user can flip back and forth
  // between cards without re-running the converter (it's ~free
  // either way, but skipping the round-trip keeps the UI snappy).
  const [k8sExportId, setK8sExportId] = useState<string | null>(null);
  const [k8sExports, setK8sExports] = useState<
    Record<string, cmd.ComposeK8sExport>
  >({});
  const [k8sNamespace, setK8sNamespace] = useState("");
  const [k8sIngressHost, setK8sIngressHost] = useState("");
  const [k8sIngressClass, setK8sIngressClass] = useState("");
  const [k8sIngressTls, setK8sIngressTls] = useState("");
  const [k8sLiftBindMounts, setK8sLiftBindMounts] = useState(false);
  const [k8sBusyId, setK8sBusyId] = useState<string | null>(null);

  useEffect(() => {
    if (!target) return;
    setMessage("");
    setPreviewId(null);
    setK8sExportId(null);
    setK8sExports({});
    setK8sNamespace("");
    setK8sIngressHost("");
    setK8sIngressClass("");
    setK8sIngressTls("");
    setK8sLiftBindMounts(false);
    setK8sBusyId(null);
    void cmd.softwareComposeTemplates().then(setTemplates).catch(() => setTemplates([]));
  }, [target?.id]);

  if (!target || !sshParams) return null;

  async function run(action: "apply" | "down", templateId: string) {
    if (busy) return;
    setBusy(`${action}:${templateId}`);
    setMessage("");
    try {
      const r =
        action === "apply"
          ? await cmd.softwareComposeApply({ ...sshParams!, templateId })
          : await cmd.softwareComposeDown({ ...sshParams!, templateId });
      setMessage(describeServiceReport(r, t));
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  /** Build a cron line for "periodic compose pull + up -d" and copy
   *  it to the clipboard. We default to a daily 3am refresh — the
   *  user can edit the `0 3 * * *` prefix to whatever cadence they
   *  want before pasting into `crontab -e`. The directory matches
   *  what `compose_apply` writes to: `$HOME/pier-x-stacks/<id>`.
   *  Doesn't try to be clever about resolving `$HOME` — most
   *  crontabs run with HOME populated, and if not the user can
   *  swap in an absolute path. */
  async function copyComposeCronLine(templateId: string) {
    const tpl = templates.find((x) => x.id === templateId);
    if (!tpl) {
      setMessage(t("Template not found."));
      return;
    }
    const dir = `$HOME/pier-x-stacks/${tpl.id}`;
    const command = `cd ${dir} && docker compose pull && docker compose up -d`;
    const line = `0 3 * * * ${command}  # pier-x:compose:${tpl.id}`;
    try {
      await writeClipboardText(line);
      setMessage(
        t(
          "Copied refresh cron line for {name}. Paste into `crontab -e` on the host (default cadence: daily at 03:00 — edit before saving to suit your needs).",
          { name: tpl.displayName },
        ),
      );
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    }
  }

  /** Toggle the K8s export pane for a template card. First open
   *  fires the conversion (cached on success); subsequent toggles
   *  just flip visibility without re-running. The namespace box at
   *  the top of the pane re-runs the conversion in place when
   *  edited so the user can see the namespace woven into each
   *  manifest's metadata without leaving the dialog. */
  function exportArgs(templateId: string) {
    return {
      templateId,
      namespace: k8sNamespace.trim() || undefined,
      ingressHost: k8sIngressHost.trim() || undefined,
      ingressClass: k8sIngressClass.trim() || undefined,
      ingressTlsSecret: k8sIngressTls.trim() || undefined,
      liftBindMounts: k8sLiftBindMounts,
    };
  }

  async function toggleK8sExport(templateId: string) {
    if (k8sExportId === templateId) {
      setK8sExportId(null);
      return;
    }
    if (k8sBusyId === templateId) return;
    if (k8sExports[templateId]) {
      setK8sExportId(templateId);
      return;
    }
    setK8sBusyId(templateId);
    try {
      const exp = await cmd.softwareComposeExportK8s(exportArgs(templateId));
      setK8sExports((prev) => ({ ...prev, [templateId]: exp }));
      setK8sExportId(templateId);
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setK8sBusyId(null);
    }
  }

  /** Re-run the conversion with the current namespace + ingress
   *  settings. Wired to each input's blur / Enter handler so a
   *  typo doesn't fire dozens of round-trips per keystroke. */
  async function refreshK8sExport(templateId: string) {
    setK8sBusyId(templateId);
    try {
      const exp = await cmd.softwareComposeExportK8s(exportArgs(templateId));
      setK8sExports((prev) => ({ ...prev, [templateId]: exp }));
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setK8sBusyId(null);
    }
  }

  async function copyK8sYaml(templateId: string) {
    const exp = k8sExports[templateId];
    if (!exp) return;
    await writeClipboardText(exp.k8sYaml);
    setMessage(t("Kubernetes manifest copied to clipboard."));
  }

  async function saveK8sYaml(templateId: string) {
    const exp = k8sExports[templateId];
    if (!exp) return;
    try {
      const dialog = await import("@tauri-apps/plugin-dialog");
      const picked = await dialog.save({
        defaultPath: `${templateId}-k8s.yaml`,
        filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
      });
      if (typeof picked !== "string") return;
      await cmd.localWriteTextFile(picked, exp.k8sYaml);
      setMessage(t("Saved manifest to {path}", { path: picked }));
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <Dialog
      open={!!target}
      title={t("Docker Compose templates")}
      subtitle={t(
        "One-click stacks. Each writes ~/pier-x-stacks/<id>/docker-compose.yml and runs docker compose up -d.",
      )}
      size="md"
      onClose={onClose}
    >
      <div className="sw-compose">
        <div className="sw-compose__list">
          {templates.map((tpl) => {
            const applyBusy = busy === `apply:${tpl.id}`;
            const downBusy = busy === `down:${tpl.id}`;
            const previewing = previewId === tpl.id;
            return (
              <div key={tpl.id} className="sw-compose__card">
                <div className="sw-compose__card-head">
                  <span className="sw-compose__card-label">
                    {tpl.displayName}
                  </span>
                  {tpl.publishedPorts.length > 0 && (
                    <span className="sw-compose__card-ports mono">
                      :{tpl.publishedPorts.join(" :")}
                    </span>
                  )}
                </div>
                <div className="sw-compose__card-desc">{tpl.description}</div>
                <div className="sw-compose__card-actions">
                  <button
                    type="button"
                    className="btn is-ghost is-compact"
                    onClick={() =>
                      setPreviewId(previewing ? null : tpl.id)
                    }
                  >
                    {previewing ? t("Hide YAML") : t("Show YAML")}
                  </button>
                  <button
                    type="button"
                    className="btn is-ghost is-compact"
                    onClick={() => void toggleK8sExport(tpl.id)}
                    disabled={k8sBusyId === tpl.id}
                  >
                    {k8sBusyId === tpl.id
                      ? t("Converting...")
                      : k8sExportId === tpl.id
                        ? t("Hide K8s YAML")
                        : t("Export K8s YAML")}
                  </button>
                  <button
                    type="button"
                    className="btn is-ghost is-compact"
                    onClick={() => void copyComposeCronLine(tpl.id)}
                    title={t(
                      "Copy a cron line that runs `docker compose pull && up -d` against this stack. Paste into `crontab -e` on the host so images stay refreshed even when Pier-X is closed.",
                    )}
                  >
                    <Copy size={10} /> {t("Refresh cron line")}
                  </button>
                  <button
                    type="button"
                    className="btn is-ghost is-compact"
                    onClick={() => void run("down", tpl.id)}
                    disabled={!!busy}
                  >
                    {downBusy ? t("Running...") : t("Down")}
                  </button>
                  <button
                    type="button"
                    className="btn is-primary is-compact"
                    onClick={() => void run("apply", tpl.id)}
                    disabled={!!busy}
                  >
                    {applyBusy ? t("Applying...") : t("Apply")}
                  </button>
                </div>
                {previewing && (
                  <pre className="sw-compose__yaml mono">{tpl.yaml}</pre>
                )}
                {k8sExportId === tpl.id && k8sExports[tpl.id] && (
                  <div className="sw-compose__k8s">
                    <div className="sw-compose__k8s-toolbar">
                      <label className="sw-compose__k8s-ns">
                        <span>{t("Namespace")}</span>
                        <input
                          type="text"
                          value={k8sNamespace}
                          onChange={(e) => setK8sNamespace(e.target.value)}
                          onBlur={() => void refreshK8sExport(tpl.id)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault();
                              void refreshK8sExport(tpl.id);
                            }
                          }}
                          placeholder={t("(cluster default)")}
                          disabled={k8sBusyId === tpl.id}
                        />
                      </label>
                      <span className="sw-compose__k8s-summary mono muted">
                        {t(
                          "{deps} Deployments · {svcs} Services · {pvcs} PVCs · {ings} Ingress · {cms} ConfigMaps · {secs} Secrets · {nps} NetworkPolicies",
                          {
                            deps: k8sExports[tpl.id].deploymentCount,
                            svcs: k8sExports[tpl.id].serviceCount,
                            pvcs: k8sExports[tpl.id].pvcCount,
                            ings: k8sExports[tpl.id].ingressCount,
                            cms: k8sExports[tpl.id].configmapCount,
                            secs: k8sExports[tpl.id].secretCount,
                            nps: k8sExports[tpl.id].networkpolicyCount,
                          },
                        )}
                      </span>
                      <div className="sw-compose__k8s-actions">
                        <button
                          type="button"
                          className="btn is-ghost is-compact"
                          onClick={() => void copyK8sYaml(tpl.id)}
                        >
                          {t("Copy")}
                        </button>
                        <button
                          type="button"
                          className="btn is-ghost is-compact"
                          onClick={() => void saveK8sYaml(tpl.id)}
                        >
                          {t("Save as…")}
                        </button>
                      </div>
                    </div>
                    <div className="sw-compose__k8s-ingress">
                      <label className="sw-compose__k8s-ns">
                        <span>{t("Ingress host")}</span>
                        <input
                          type="text"
                          value={k8sIngressHost}
                          onChange={(e) =>
                            setK8sIngressHost(e.target.value)
                          }
                          onBlur={() => void refreshK8sExport(tpl.id)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault();
                              void refreshK8sExport(tpl.id);
                            }
                          }}
                          placeholder={t("(skip Ingress)")}
                          disabled={k8sBusyId === tpl.id}
                        />
                      </label>
                      <label className="sw-compose__k8s-ns">
                        <span>{t("Ingress class")}</span>
                        <input
                          type="text"
                          value={k8sIngressClass}
                          onChange={(e) =>
                            setK8sIngressClass(e.target.value)
                          }
                          onBlur={() => void refreshK8sExport(tpl.id)}
                          placeholder={t("nginx, traefik, …")}
                          disabled={k8sBusyId === tpl.id}
                        />
                      </label>
                      <label className="sw-compose__k8s-ns">
                        <span>{t("TLS secret")}</span>
                        <input
                          type="text"
                          value={k8sIngressTls}
                          onChange={(e) =>
                            setK8sIngressTls(e.target.value)
                          }
                          onBlur={() => void refreshK8sExport(tpl.id)}
                          placeholder={t("(plain HTTP)")}
                          disabled={k8sBusyId === tpl.id}
                        />
                      </label>
                      <label
                        className="sw-compose__k8s-lift"
                        title={t(
                          "Convert Compose bind mounts (./local:/in) into placeholder ConfigMap resources so the manifest applies cleanly.",
                        )}
                      >
                        <input
                          type="checkbox"
                          checked={k8sLiftBindMounts}
                          onChange={(e) => {
                            setK8sLiftBindMounts(e.target.checked);
                            void refreshK8sExport(tpl.id);
                          }}
                          disabled={k8sBusyId === tpl.id}
                        />
                        <span>{t("Lift bind mounts → ConfigMap")}</span>
                      </label>
                    </div>
                    {k8sExports[tpl.id].warnings.length > 0 && (
                      <div className="status-note status-note--warn mono sw-compose__k8s-warn">
                        <div className="sw-compose__k8s-warn-title">
                          {t(
                            "Some Compose features can't translate cleanly — review before applying:",
                          )}
                        </div>
                        <ul>
                          {k8sExports[tpl.id].warnings.map((w, i) => (
                            <li key={i}>{w}</li>
                          ))}
                        </ul>
                      </div>
                    )}
                    <pre className="sw-compose__yaml mono">
                      {k8sExports[tpl.id].k8sYaml}
                    </pre>
                  </div>
                )}
              </div>
            );
          })}
        </div>
        {message && <div className="sw-pg-form__msg mono">{message}</div>}
      </div>
    </Dialog>
  );
}

function describeServiceReport(
  report: cmd.PostgresActionReport,
  t: ReturnType<typeof useI18n>["t"],
): string {
  if (report.status === "ok") return t("Done.");
  if (report.status === "sudo-requires-password") {
    return t(
      "sudo requires a password — connect as root or configure passwordless sudo.",
    );
  }
  return t("Failed (exit {code}). {tail}", {
    code: report.exitCode,
    tail: report.outputTail.split("\n").slice(-1)[0] ?? "",
  });
}

/** MySQL/MariaDB quick-config dialog. Mirror of PgQuickConfigDialog
 *  but with MySQL syntax + an optional "current root password"
 *  field for distros where root is already password-protected. */
function MysqlQuickConfigDialog({
  target,
  sshParams,
  onClose,
}: {
  target: SoftwareDescriptor | null;
  sshParams: SshParams | null;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [rootPass, setRootPass] = useState("");
  const [user, setUser] = useState("piertest");
  const [pass, setPass] = useState("");
  const [dbName, setDbName] = useState("piertest_db");
  const [busy, setBusy] = useState<"user" | "db" | "remote" | null>(null);
  const [userMsg, setUserMsg] = useState("");
  const [dbMsg, setDbMsg] = useState("");
  const [remoteMsg, setRemoteMsg] = useState("");

  useEffect(() => {
    if (!target) return;
    setUserMsg(""); setDbMsg(""); setRemoteMsg("");
  }, [target?.id]);

  if (!target || !sshParams) return null;
  const rootArg = rootPass ? rootPass : null;

  async function handleCreateUser() {
    if (busy || !user.trim() || !pass) return;
    setBusy("user"); setUserMsg("");
    try {
      const r = await cmd.mysqlCreateUserRemote({
        ...sshParams!,
        dbUsername: user,
        dbPassword: pass,
        dbName,
        rootPassword: rootArg,
      });
      setUserMsg(describeServiceReport(r, t));
    } catch (e) {
      setUserMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function handleCreateDb() {
    if (busy || !dbName.trim()) return;
    setBusy("db"); setDbMsg("");
    try {
      const r = await cmd.mysqlCreateDbRemote({
        ...sshParams!,
        dbName,
        rootPassword: rootArg,
      });
      setDbMsg(describeServiceReport(r, t));
    } catch (e) {
      setDbMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function handleOpenRemote() {
    if (busy) return;
    setBusy("remote"); setRemoteMsg("");
    try {
      const r = await cmd.mysqlOpenRemote(sshParams!);
      setRemoteMsg(describeServiceReport(r, t));
    } catch (e) {
      setRemoteMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <Dialog
      open={!!target}
      title={t("MySQL/MariaDB quick config")}
      subtitle={t(
        "Run common post-install setup tasks against the local MySQL/MariaDB cluster.",
      )}
      size="md"
      onClose={onClose}
    >
      <div className="sw-pg-form">
        <fieldset className="sw-pg-form__section" disabled={busy !== null}>
          <legend>{t("Root authentication")}</legend>
          <div className="sw-pg-form__hint">
            {t(
              "Fresh apt installs use auth_socket for root — sudo connects without a password. If you've set a root password, type it here.",
            )}
          </div>
          <div className="sw-pg-form__row">
            <input
              className="dlg-input"
              type="password"
              value={rootPass}
              onChange={(e) => setRootPass(e.currentTarget.value)}
              placeholder={t("root password (optional)")}
              autoComplete="current-password"
            />
          </div>
        </fieldset>

        <fieldset className="sw-pg-form__section" disabled={busy !== null}>
          <legend>{t("Create user + grant on database")}</legend>
          <div className="sw-pg-form__row">
            <input
              className="dlg-input"
              value={user}
              onChange={(e) => setUser(e.currentTarget.value)}
              placeholder={t("username")}
              spellCheck={false}
            />
            <input
              className="dlg-input"
              type="password"
              value={pass}
              onChange={(e) => setPass(e.currentTarget.value)}
              placeholder={t("password")}
              autoComplete="new-password"
            />
            <input
              className="dlg-input"
              value={dbName}
              onChange={(e) => setDbName(e.currentTarget.value)}
              placeholder={t("database (granted)")}
              spellCheck={false}
            />
            <button
              type="button"
              className="btn is-primary is-compact"
              onClick={() => void handleCreateUser()}
              disabled={!user.trim() || !pass}
            >
              {busy === "user" ? t("Running...") : t("Create / update")}
            </button>
          </div>
          {userMsg && <div className="sw-pg-form__msg mono">{userMsg}</div>}
        </fieldset>

        <fieldset className="sw-pg-form__section" disabled={busy !== null}>
          <legend>{t("Create database")}</legend>
          <div className="sw-pg-form__row">
            <input
              className="dlg-input"
              value={dbName}
              onChange={(e) => setDbName(e.currentTarget.value)}
              placeholder={t("database name")}
              spellCheck={false}
            />
            <button
              type="button"
              className="btn is-primary is-compact"
              onClick={() => void handleCreateDb()}
              disabled={!dbName.trim()}
            >
              {busy === "db" ? t("Running...") : t("Create")}
            </button>
          </div>
          {dbMsg && <div className="sw-pg-form__msg mono">{dbMsg}</div>}
        </fieldset>

        <fieldset className="sw-pg-form__section" disabled={busy !== null}>
          <legend>{t("Allow remote connections")}</legend>
          <div className="sw-pg-form__hint">
            {t(
              "Sets bind-address = 0.0.0.0 in mysqld.cnf / my.cnf and restarts the daemon. Make sure you have an account that grants from '%' before opening up.",
            )}
          </div>
          <div className="sw-pg-form__row">
            <button
              type="button"
              className="btn is-danger is-compact"
              onClick={() => void handleOpenRemote()}
            >
              {busy === "remote" ? t("Running...") : t("Open to 0.0.0.0")}
            </button>
          </div>
          {remoteMsg && <div className="sw-pg-form__msg mono">{remoteMsg}</div>}
        </fieldset>
      </div>
    </Dialog>
  );
}

/** Redis quick-config dialog. Two simple actions:
 *  - set requirepass
 *  - allow remote (bind 0.0.0.0 + protected-mode no) */
function RedisQuickConfigDialog({
  target,
  sshParams,
  onClose,
}: {
  target: SoftwareDescriptor | null;
  sshParams: SshParams | null;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [pwd, setPwd] = useState("");
  const [busy, setBusy] = useState<"pwd" | "remote" | null>(null);
  const [pwdMsg, setPwdMsg] = useState("");
  const [remoteMsg, setRemoteMsg] = useState("");

  useEffect(() => {
    if (!target) return;
    setPwdMsg(""); setRemoteMsg("");
  }, [target?.id]);

  if (!target || !sshParams) return null;

  async function handleSetPwd() {
    if (busy || !pwd) return;
    setBusy("pwd"); setPwdMsg("");
    try {
      const r = await cmd.redisSetPasswordRemote({
        ...sshParams!,
        redisPassword: pwd,
      });
      setPwdMsg(describeServiceReport(r, t));
    } catch (e) {
      setPwdMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function handleOpenRemote() {
    if (busy) return;
    setBusy("remote"); setRemoteMsg("");
    try {
      const r = await cmd.redisOpenRemote(sshParams!);
      setRemoteMsg(describeServiceReport(r, t));
    } catch (e) {
      setRemoteMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <Dialog
      open={!!target}
      title={t("Redis quick config")}
      subtitle={t("Set requirepass and toggle remote-network listen.")}
      size="sm"
      onClose={onClose}
    >
      <div className="sw-pg-form">
        <fieldset className="sw-pg-form__section" disabled={busy !== null}>
          <legend>{t("Set password (requirepass)")}</legend>
          <div className="sw-pg-form__row">
            <input
              className="dlg-input"
              type="password"
              value={pwd}
              onChange={(e) => setPwd(e.currentTarget.value)}
              placeholder={t("password")}
              autoComplete="new-password"
            />
            <button
              type="button"
              className="btn is-primary is-compact"
              onClick={() => void handleSetPwd()}
              disabled={!pwd}
            >
              {busy === "pwd" ? t("Running...") : t("Set password")}
            </button>
          </div>
          {pwdMsg && <div className="sw-pg-form__msg mono">{pwdMsg}</div>}
        </fieldset>

        <fieldset className="sw-pg-form__section" disabled={busy !== null}>
          <legend>{t("Allow remote connections")}</legend>
          <div className="sw-pg-form__hint">
            {t(
              "Sets bind 0.0.0.0 and protected-mode no in redis.conf. Use after setting a password.",
            )}
          </div>
          <div className="sw-pg-form__row">
            <button
              type="button"
              className="btn is-danger is-compact"
              onClick={() => void handleOpenRemote()}
            >
              {busy === "remote" ? t("Running...") : t("Open to 0.0.0.0")}
            </button>
          </div>
          {remoteMsg && <div className="sw-pg-form__msg mono">{remoteMsg}</div>}
        </fieldset>
      </div>
    </Dialog>
  );
}

/** Past-actions journal viewer. Reads `software-history.jsonl`,
 *  shows the most recent entries (default: last 24 hours up to 200
 *  rows) with a clear-all button. Tracking is append-only on the
 *  backend so an in-flight install can't trample a finished one. */
function HistoryDialog({
  open,
  onClose,
  onUndo,
}: {
  open: boolean;
  onClose: () => void;
  onUndo: (
    entry: cmd.SoftwareHistoryEntry,
    onProgress: (msg: string) => void,
  ) => Promise<void>;
}) {
  const { t } = useI18n();
  const [entries, setEntries] = useState<cmd.SoftwareHistoryEntry[]>([]);
  const [busy, setBusy] = useState(false);
  const [windowKind, setWindowKind] = useState<"24h" | "all">("24h");
  /** Per-entry undo running flag + last status message. Keyed by
   *  the entry's `ts + action + target` triple (closest thing to a
   *  unique id; logically a journal slot). */
  const [undoState, setUndoState] = useState<
    Record<string, { running: boolean; msg: string }>
  >({});

  function entryKey(e: cmd.SoftwareHistoryEntry): string {
    return `${e.ts}:${e.action}:${e.target}`;
  }

  function isUndoable(e: cmd.SoftwareHistoryEntry): boolean {
    if (
      e.savedConnectionIndex === null ||
      e.savedConnectionIndex === undefined
    ) {
      return false;
    }
    if (e.outcome !== "ok" && e.outcome !== "installed" && e.outcome !== "uninstalled") {
      return false;
    }
    return ["install", "update", "uninstall", "mirror-set"].includes(e.action);
  }

  async function load() {
    setBusy(true);
    try {
      const sinceTs =
        windowKind === "24h"
          ? Math.floor(Date.now() / 1000) - 24 * 60 * 60
          : 0;
      const rows = await cmd.softwareHistoryList({ sinceTs, limit: 500 });
      setEntries(rows);
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    if (open) void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, windowKind]);

  if (!open) return null;
  return (
    <Dialog
      open={open}
      title={t("Action history")}
      subtitle={t("Last {n} entries.", { n: entries.length })}
      size="md"
      onClose={onClose}
    >
      <div className="sw-history">
        <div className="sw-history__head">
          <div className="sw-bundle-form__tabs">
            <button
              type="button"
              className={`sw-bundle-form__tab${
                windowKind === "24h" ? " is-active" : ""
              }`}
              onClick={() => setWindowKind("24h")}
            >
              {t("Last 24 hours")}
            </button>
            <button
              type="button"
              className={`sw-bundle-form__tab${
                windowKind === "all" ? " is-active" : ""
              }`}
              onClick={() => setWindowKind("all")}
            >
              {t("All time")}
            </button>
          </div>
          <div style={{ flex: 1 }} />
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={() => void load()}
            disabled={busy}
          >
            <RefreshCw size={10} /> {t("Refresh")}
          </button>
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={busy || entries.length === 0}
            onClick={async () => {
              await cmd.softwareHistoryClear();
              await load();
            }}
          >
            <Trash2 size={10} /> {t("Clear all")}
          </button>
        </div>
        {entries.length === 0 ? (
          <div className="sw-panel__empty mono">
            {busy ? t("Loading...") : t("No history entries.")}
          </div>
        ) : (
          <div className="sw-history__list">
            {entries.map((e, i) => {
              const key = entryKey(e);
              const undoable = isUndoable(e);
              const u = undoState[key];
              return (
                <div
                  key={`${e.ts}-${i}`}
                  className={`sw-history__row sw-history__row--${
                    e.outcome === "ok" ||
                    e.outcome === "installed" ||
                    e.outcome === "uninstalled"
                      ? "ok"
                      : "fail"
                  }`}
                >
                  <span className="sw-history__ts mono">
                    {new Date(e.ts * 1000).toLocaleString()}
                  </span>
                  <span className="sw-history__action mono">{e.action}</span>
                  <span className="sw-history__target">{e.target}</span>
                  <span className="sw-history__host mono">{e.host}</span>
                  <span className="sw-history__outcome mono">{e.outcome}</span>
                  <button
                    type="button"
                    className="btn is-ghost is-compact sw-history__undo"
                    disabled={!undoable || u?.running}
                    title={
                      undoable
                        ? t("Run the inverse action")
                        : t("Undo unavailable for this entry")
                    }
                    onClick={async () => {
                      setUndoState((prev) => ({
                        ...prev,
                        [key]: { running: true, msg: "" },
                      }));
                      try {
                        await onUndo(e, (msg) => {
                          setUndoState((prev) => ({
                            ...prev,
                            [key]: { running: false, msg },
                          }));
                        });
                      } finally {
                        setUndoState((prev) => ({
                          ...prev,
                          [key]: { running: false, msg: prev[key]?.msg ?? "" },
                        }));
                        // Refresh the list so the inverse action's
                        // own log entry shows up.
                        void load();
                      }
                    }}
                  >
                    {u?.running ? (
                      <Loader size={10} className="sw-row__spin" />
                    ) : (
                      <RotateCw size={10} />
                    )}{" "}
                    {t("Undo")}
                  </button>
                  {(e.note || u?.msg) && (
                    <span className="sw-history__note">
                      {[e.note, u?.msg].filter(Boolean).join(" · ")}
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </Dialog>
  );
}

/** Per-host execution status used by [`MultiHostDialog`]. */
type HostRunState = "idle" | "running" | "ok" | "failed";

/** Batch-action dialog: pick saved SSH connections + an action +
 *  run the action against each host sequentially. Reuses the
 *  existing single-host commands client-side so we don't duplicate
 *  any pier-core surface.
 *
 *  Sequential (not parallel) so the user can see clear per-host
 *  progress without an SSH-multiplexer stampede on shared infra. */
function MultiHostDialog({
  open,
  onClose,
  bundles,
  mirrorCatalog,
}: {
  open: boolean;
  onClose: () => void;
  bundles: SoftwareBundle[];
  mirrorCatalog: MirrorChoice[];
}) {
  const { t } = useI18n();
  const [hosts, setHosts] = useState<SavedSshConnection[]>([]);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [action, setAction] = useState<"mirror" | "bundle">("mirror");
  const [mirrorPick, setMirrorPick] = useState<MirrorId | "">("");
  const [bundlePick, setBundlePick] = useState<string>("");
  const [busy, setBusy] = useState(false);
  /** Per-host run state, keyed by saved-connection index. */
  const [hostStates, setHostStates] = useState<
    Record<number, { state: HostRunState; message: string }>
  >({});
  /** Per-host action override. Empty string = use the dialog's
   *  default action; any other value (e.g. "bundle:devops" or
   *  "mirror:tsinghua") overrides it for just that host. */
  const [overrides, setOverrides] = useState<Record<number, string>>({});

  // Load saved connections each time the dialog opens.
  useEffect(() => {
    if (!open) return;
    setHostStates({});
    setOverrides({});
    setBusy(false);
    cmd
      .sshConnectionsList()
      .then((rows) => setHosts(rows))
      .catch(() => setHosts([]));
  }, [open]);

  // Default mirror pick = first catalog entry; bundle pick = first.
  useEffect(() => {
    if (mirrorCatalog.length > 0 && !mirrorPick) {
      setMirrorPick(mirrorCatalog[0].id);
    }
    if (bundles.length > 0 && !bundlePick) {
      setBundlePick(bundles[0].id);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mirrorCatalog.length, bundles.length]);

  if (!open) return null;
  const allSelected = hosts.length > 0 && selected.size === hosts.length;

  function toggleHost(idx: number) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  }

  function toggleAll() {
    if (allSelected) setSelected(new Set());
    else setSelected(new Set(hosts.map((h) => h.index)));
  }

  /** Resolve which action runs for `host`. Falls back to the
   *  dialog's default when the per-host override is unset or
   *  malformed. The override format is `<kind>:<id>` so we can
   *  encode `mirror:tsinghua` and `bundle:devops` in one
   *  string-keyed record. */
  function resolveAction(hostIndex: number): {
    kind: "mirror" | "bundle";
    id: string;
  } {
    const ov = overrides[hostIndex];
    if (ov) {
      const [kind, id] = ov.split(":", 2);
      if (kind === "mirror" || kind === "bundle") {
        return { kind, id: id ?? "" };
      }
    }
    return {
      kind: action,
      id: action === "mirror" ? mirrorPick : bundlePick,
    };
  }

  /** Run the resolved action against `host`. Returns whether it
   *  succeeded so the outer loop can decide to continue. */
  async function runOne(host: SavedSshConnection): Promise<boolean> {
    const sshParams = {
      host: host.host,
      port: host.port,
      user: host.user,
      authMode: host.authKind === "password" ? "password" : host.authKind,
      // Password / key get resolved server-side via savedConnectionIndex.
      password: "",
      keyPath: host.keyPath,
      savedConnectionIndex: host.index,
    };
    const resolved = resolveAction(host.index);
    setHostStates((prev) => ({
      ...prev,
      [host.index]: { state: "running", message: "" },
    }));
    try {
      if (resolved.kind === "mirror") {
        if (!resolved.id) throw new Error("no mirror picked");
        const report = await cmd.softwareMirrorSet({
          ...sshParams,
          mirrorId: resolved.id as MirrorId,
        });
        if (report.status === "ok") {
          setHostStates((prev) => ({
            ...prev,
            [host.index]: {
              state: "ok",
              message: t("Mirror set"),
            },
          }));
          return true;
        }
        setHostStates((prev) => ({
          ...prev,
          [host.index]: {
            state: "failed",
            message: report.status,
          },
        }));
        return false;
      }
      // bundle
      const bundle = bundles.find((b) => b.id === resolved.id);
      if (!bundle) throw new Error("no bundle picked");
      const probe = await cmd.softwareProbeRemote(sshParams);
      const installed = new Set(
        probe.statuses.filter((s) => s.installed).map((s) => s.id),
      );
      const todo = bundle.packageIds.filter((id) => !installed.has(id));
      if (todo.length === 0) {
        setHostStates((prev) => ({
          ...prev,
          [host.index]: { state: "ok", message: t("Already installed") },
        }));
        return true;
      }
      // Install each member sequentially.
      for (const pkgId of todo) {
        const installId =
          typeof crypto !== "undefined" && "randomUUID" in crypto
            ? crypto.randomUUID()
            : `${Date.now()}-${Math.random()}`;
        // eslint-disable-next-line no-await-in-loop
        const report = await cmd.softwareInstallRemote({
          ...sshParams,
          packageId: pkgId,
          installId,
          enableService: true,
        });
        if (report.status !== "installed") {
          setHostStates((prev) => ({
            ...prev,
            [host.index]: {
              state: "failed",
              message: t("{pkg}: {status}", {
                pkg: pkgId,
                status: report.status,
              }),
            },
          }));
          return false;
        }
      }
      setHostStates((prev) => ({
        ...prev,
        [host.index]: {
          state: "ok",
          message: t("{n} installed", { n: todo.length }),
        },
      }));
      return true;
    } catch (e) {
      setHostStates((prev) => ({
        ...prev,
        [host.index]: {
          state: "failed",
          message: e instanceof Error ? e.message : String(e),
        },
      }));
      return false;
    }
  }

  async function runAll() {
    if (busy || selected.size === 0) return;
    setBusy(true);
    const queue = hosts.filter((h) => selected.has(h.index));
    for (const h of queue) {
      // eslint-disable-next-line no-await-in-loop
      await runOne(h);
    }
    setBusy(false);
  }

  return (
    <Dialog
      open={open}
      title={t("Batch hosts")}
      subtitle={t(
        "Apply a mirror switch or a bundle install across multiple saved SSH connections.",
      )}
      size="md"
      onClose={onClose}
    >
      <div className="sw-multihost">
        <div className="sw-multihost__action">
          <label className="sw-multihost__action-label mono">
            {t("Action")}:
          </label>
          <select
            className="dlg-input"
            value={action}
            onChange={(e) => setAction(e.currentTarget.value as "mirror" | "bundle")}
            disabled={busy}
          >
            <option value="mirror">{t("Switch mirror")}</option>
            <option value="bundle">{t("Install bundle")}</option>
          </select>
          {action === "mirror" ? (
            <select
              className="dlg-input"
              value={mirrorPick}
              onChange={(e) => setMirrorPick(e.currentTarget.value as MirrorId)}
              disabled={busy}
            >
              {mirrorCatalog.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.label}
                </option>
              ))}
            </select>
          ) : (
            <select
              className="dlg-input"
              value={bundlePick}
              onChange={(e) => setBundlePick(e.currentTarget.value)}
              disabled={busy}
            >
              {bundles.map((b) => (
                <option key={b.id} value={b.id}>
                  {b.displayName}
                </option>
              ))}
            </select>
          )}
        </div>
        <div className="sw-multihost__hosts">
          <div className="sw-multihost__hosts-head mono">
            <label>
              <input
                type="checkbox"
                checked={allSelected}
                onChange={toggleAll}
                disabled={busy || hosts.length === 0}
              />{" "}
              {t("Select all ({n})", { n: hosts.length })}
            </label>
          </div>
          {hosts.length === 0 && (
            <div className="sw-panel__empty mono">
              {t("No saved SSH connections.")}
            </div>
          )}
          {hosts.map((h) => {
            const status = hostStates[h.index];
            const overrideValue = overrides[h.index] ?? "";
            return (
              <label key={h.index} className="sw-multihost__host">
                <input
                  type="checkbox"
                  checked={selected.has(h.index)}
                  onChange={() => toggleHost(h.index)}
                  disabled={busy}
                />
                <span className="sw-multihost__host-name">
                  {h.name || `${h.user}@${h.host}`}
                </span>
                <span className="sw-multihost__host-target mono">
                  {h.user}@{h.host}:{h.port}
                </span>
                <select
                  className="sw-multihost__host-override mono"
                  value={overrideValue}
                  disabled={busy || !selected.has(h.index)}
                  onChange={(e) => {
                    const v = e.currentTarget.value;
                    setOverrides((prev) => {
                      const next = { ...prev };
                      if (v) next[h.index] = v;
                      else delete next[h.index];
                      return next;
                    });
                  }}
                  title={t("Override the action for just this host")}
                >
                  <option value="">{t("(default)")}</option>
                  <optgroup label={t("Switch mirror")}>
                    {mirrorCatalog.map((m) => (
                      <option key={`m-${m.id}`} value={`mirror:${m.id}`}>
                        {m.label}
                      </option>
                    ))}
                  </optgroup>
                  <optgroup label={t("Install bundle")}>
                    {bundles.map((b) => (
                      <option key={`b-${b.id}`} value={`bundle:${b.id}`}>
                        {b.displayName}
                      </option>
                    ))}
                  </optgroup>
                </select>
                {status && (
                  <span
                    className={`sw-multihost__host-status sw-multihost__host-status--${status.state}`}
                  >
                    {status.state === "running" ? (
                      <Loader size={10} className="sw-row__spin" />
                    ) : status.state === "ok" ? (
                      <Check size={10} />
                    ) : status.state === "failed" ? (
                      <X size={10} />
                    ) : null}{" "}
                    {status.message}
                  </span>
                )}
              </label>
            );
          })}
        </div>
        <div className="sw-multihost__actions">
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={onClose}
            disabled={busy}
          >
            {t("Close")}
          </button>
          <button
            type="button"
            className="btn is-primary is-compact"
            disabled={busy || selected.size === 0}
            onClick={() => void runAll()}
          >
            {busy
              ? t("Running...")
              : t("Run on {n} host(s)", { n: selected.size })}
          </button>
        </div>
      </div>
    </Dialog>
  );
}

/** Webhook settings dialog. Lists, edits, saves, and test-fires
 *  user-configured HTTP webhooks. Persisted to
 *  `<app_config_dir>/webhooks.json` via the backend; new entries
 *  take effect immediately for the next install/uninstall in the
 *  current session (no restart required, unlike software-extras). */
function WebhookHeadersEditor({
  headers,
  onChange,
  t,
}: {
  headers: cmd.WebhookHeader[];
  onChange: (next: cmd.WebhookHeader[]) => void;
  t: (
    s: string,
    vars?: Record<string, string | number | null | undefined>,
  ) => string;
}) {
  const [open, setOpen] = useState(headers.length > 0);
  function update(i: number, patch: Partial<cmd.WebhookHeader>) {
    onChange(headers.map((h, idx) => (idx === i ? { ...h, ...patch } : h)));
  }
  function remove(i: number) {
    onChange(headers.filter((_, idx) => idx !== i));
  }
  function add() {
    onChange([...headers, { name: "", value: "" }]);
    setOpen(true);
  }
  return (
    <div className="sw-webhooks__headers">
      <button
        type="button"
        className="sw-webhooks__headers-toggle muted"
        onClick={() => setOpen((v) => !v)}
      >
        {open ? "▾" : "▸"} {t("Custom headers")} ({headers.length})
      </button>
      {open && (
        <div className="sw-webhooks__headers-body">
          {headers.length === 0 && (
            <span className="muted">
              {t("No extra headers. Content-Type is always application/json.")}
            </span>
          )}
          {headers.map((h, i) => (
            <div key={i} className="sw-webhooks__headers-row">
              <input
                className="sw-webhooks__headers-name mono"
                placeholder={t("Header name (e.g. Authorization)")}
                value={h.name}
                onChange={(ev) => update(i, { name: ev.target.value })}
                spellCheck={false}
              />
              <input
                className="sw-webhooks__headers-val mono"
                placeholder={t("Value (e.g. Bearer xyz123)")}
                value={h.value}
                onChange={(ev) => update(i, { value: ev.target.value })}
                spellCheck={false}
              />
              <button
                type="button"
                className="btn is-ghost is-compact"
                onClick={() => remove(i)}
                title={t("Remove")}
              >
                <X size={10} />
              </button>
            </div>
          ))}
          <button type="button" className="btn is-ghost is-compact" onClick={add}>
            <Plus size={10} /> {t("Add header")}
          </button>
        </div>
      )}
    </div>
  );
}

/** Header names treated as secret-bearing for export redaction.
 *  Match is case-insensitive and uses substring contains (so
 *  `X-Slack-Signature`, `Authorization`, `My-Bearer-Header` all
 *  count). Anything not matching here passes through verbatim. */
const SECRET_HEADER_PATTERNS = [
  "auth",
  "token",
  "key",
  "secret",
  "signature",
  "cookie",
  "session",
];

function isSecretHeaderName(name: string): boolean {
  const lower = name.trim().toLowerCase();
  if (!lower) return false;
  return SECRET_HEADER_PATTERNS.some((p) => lower.includes(p));
}

/** Strip every secret-looking field. Used by the default "redacted"
 *  export — the file only contains URL / label / events / templates,
 *  the receiver re-types secrets after import. Safest to share. */
function redactWebhookSecrets(e: cmd.WebhookEntry): cmd.WebhookEntry {
  const cleanedHeaders = (e.headers ?? []).map((h) =>
    isSecretHeaderName(h.name) ? { name: h.name, value: "" } : h,
  );
  const out: cmd.WebhookEntry = { ...e };
  out.headers = cleanedHeaders;
  out.hmacSecret = "";
  return out;
}

/** Base64-encode every secret-looking field with a `_pierx_b64:`
 *  prefix so the file isn't trivially readable when shared via
 *  email / Slack / a screenshot, but is still a 1:1 round-trip on
 *  import. NOT encryption — labelled as obfuscation in the dialog. */
function encodeWebhookSecrets(e: cmd.WebhookEntry): cmd.WebhookEntry {
  const wrap = (v: string): string =>
    v ? `_pierx_b64:${btoa(unescape(encodeURIComponent(v)))}` : "";
  const cleanedHeaders = (e.headers ?? []).map((h) =>
    isSecretHeaderName(h.name) ? { name: h.name, value: wrap(h.value) } : h,
  );
  const out: cmd.WebhookEntry = { ...e };
  out.headers = cleanedHeaders;
  if (e.hmacSecret) out.hmacSecret = wrap(e.hmacSecret);
  return out;
}

function decodeWebhookSecrets(e: cmd.WebhookEntry): cmd.WebhookEntry {
  const unwrap = (v: string): string => {
    if (typeof v !== "string") return v;
    if (!v.startsWith("_pierx_b64:")) return v;
    try {
      const raw = v.slice("_pierx_b64:".length);
      return decodeURIComponent(escape(atob(raw)));
    } catch {
      return v;
    }
  };
  const cleanedHeaders = (e.headers ?? []).map((h) => ({
    name: h.name,
    value: unwrap(h.value),
  }));
  const out: cmd.WebhookEntry = { ...e };
  out.headers = cleanedHeaders;
  if (e.hmacSecret) out.hmacSecret = unwrap(e.hmacSecret);
  return out;
}

/** Set of `{{name}}` placeholders the backend's `render_body` knows
 *  about. Keep in sync with `pier-core/src/services/webhook.rs`'s
 *  `pairs` array. */
const KNOWN_PLACEHOLDERS = new Set<string>([
  "event",
  "status",
  "package_id",
  "packageId",
  "host",
  "package_manager",
  "packageManager",
  "version",
  "fired_at",
  "firedAt",
  "text",
  "output_tail",
  "outputTail",
]);

function WebhooksDialog({
  open,
  initialTab,
  onClose,
}: {
  open: boolean;
  /** When set on open, jumps the dialog to this tab instead of
   *  the default "endpoints". Used by the failure-toast CTA so
   *  the user lands on the Failures viewer directly. */
  initialTab: "endpoints" | "failures" | null;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [entries, setEntries] = useState<cmd.WebhookEntry[]>([]);
  const [exportDialogOpen, setExportDialogOpen] = useState(false);
  const [path, setPath] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [testing, setTesting] = useState<number | null>(null);
  const [lastTest, setLastTest] = useState<{
    index: number;
    report: cmd.WebhookFireReport;
  } | null>(null);
  // Two tabs in the dialog: "Endpoints" (the existing config grid)
  // and "Failures" (a viewer for `webhook-failures.jsonl`). The
  // tab badge shows the failure count so a user with a fresh
  // failure spotting it from across the room is the goal.
  const [activeTab, setActiveTab] = useState<"endpoints" | "failures">(
    "endpoints",
  );
  const [failures, setFailures] = useState<cmd.WebhookFailureRecord[]>([]);
  const [failureBusy, setFailureBusy] = useState(false);
  const [replayingId, setReplayingId] = useState<string | null>(null);
  const [replayResult, setReplayResult] = useState<{
    id: string;
    report: cmd.WebhookFireReport;
  } | null>(null);

  async function refreshFailures() {
    setFailureBusy(true);
    try {
      const list = await cmd.softwareWebhooksFailuresList();
      setFailures(list);
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setFailureBusy(false);
    }
  }

  async function dismissFailure(id: string) {
    try {
      await cmd.softwareWebhooksFailuresDismiss(id);
      setFailures((prev) => prev.filter((f) => f.id !== id));
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    }
  }

  async function clearFailures() {
    try {
      await cmd.softwareWebhooksFailuresClear();
      setFailures([]);
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    }
  }

  /** Number of most-recent failures the batch-replay button will
   *  retry on click. 10 is a sweet spot: covers the common case
   *  of "a Slack outage just resolved" without sleeping the user
   *  for minutes if every replay times out. Backend caps at 50
   *  regardless. */
  const BATCH_REPLAY_LIMIT = 10;
  const [batchReplaying, setBatchReplaying] = useState(false);

  async function replayRecent() {
    if (failures.length === 0 || batchReplaying) return;
    setBatchReplaying(true);
    setMessage("");
    try {
      const rows = await cmd.softwareWebhooksReplayBatch(BATCH_REPLAY_LIMIT);
      // Backend already auto-dismisses the rows whose replay
      // landed; refresh from disk so the UI mirrors the new
      // truth (rather than us mutating the local list and
      // potentially diverging on a partial transport failure).
      await refreshFailures();
      const ok = rows.filter((r) => r.error === "").length;
      const fail = rows.length - ok;
      setMessage(
        t("Replayed {tried}: {ok} ok, {fail} still failing", {
          tried: rows.length,
          ok,
          fail,
        }),
      );
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBatchReplaying(false);
    }
  }

  async function replayFailure(rec: cmd.WebhookFailureRecord) {
    setReplayingId(rec.id);
    setReplayResult(null);
    try {
      const report = await cmd.softwareWebhooksReplay({
        url: rec.url,
        body: rec.body,
      });
      setReplayResult({ id: rec.id, report });
      // Auto-dismiss on success — the record is no longer
      // representative once the replay landed.
      if (report.error === "") {
        setFailures((prev) => prev.filter((f) => f.id !== rec.id));
        await cmd
          .softwareWebhooksFailuresDismiss(rec.id)
          .catch(() => {});
      }
    } catch (e) {
      setReplayResult({
        id: rec.id,
        report: {
          url: rec.url,
          statusCode: 0,
          latencyMs: 0,
          error: e instanceof Error ? e.message : String(e),
          attempts: 1,
        },
      });
    } finally {
      setReplayingId(null);
    }
  }

  useEffect(() => {
    if (!open) return;
    setMessage("");
    setLastTest(null);
    setReplayResult(null);
    // Honour the caller's initial tab on each open. The CTA from
    // the failure toast sets `initialTab="failures"`; manual
    // header-button clicks pass `null` and we default to the
    // last-seen tab (sticks during the session, resets on
    // remount).
    if (initialTab) setActiveTab(initialTab);
    void refreshFailures();
    void cmd
      .softwareWebhooksLoad()
      .then((cfg) => setEntries(cfg.entries))
      .catch((e) => setMessage(e instanceof Error ? e.message : String(e)));
    void cmd.softwareWebhooksPath().then(setPath).catch(() => setPath(null));
  }, [open, initialTab]);

  function updateEntry(idx: number, patch: Partial<cmd.WebhookEntry>) {
    setEntries((prev) =>
      prev.map((e, i) => (i === idx ? { ...e, ...patch } : e)),
    );
  }

  function toggleEvent(idx: number, kind: cmd.WebhookEventKindLabel) {
    setEntries((prev) =>
      prev.map((e, i) => {
        if (i !== idx) return e;
        const has = e.events.includes(kind);
        return {
          ...e,
          events: has
            ? e.events.filter((k) => k !== kind)
            : [...e.events, kind],
        };
      }),
    );
  }

  function addEntry() {
    setEntries((prev) => [
      ...prev,
      {
        url: "",
        label: "",
        events: [],
        disabled: false,
        bodyTemplate: "",
        maxRetries: 0,
        retryBackoffSecs: 0,
      },
    ]);
  }

  /** Curated body-template snippets keyed by destination. The
   *  Slack default is empty (the backend's default payload IS
   *  Slack-shaped). Discord and Teams want different top-level
   *  fields; the snippets just cover the minimum each platform
   *  needs to render the message. */
  const TEMPLATE_PRESETS: Array<{ id: string; label: string; template: string }> = [
    { id: "default", label: t("Default (Slack)"), template: "" },
    {
      id: "discord",
      label: t("Discord"),
      template: '{"content":"{{text}}"}',
    },
    {
      id: "teams",
      label: t("Microsoft Teams"),
      template:
        '{"@type":"MessageCard","@context":"https://schema.org/extensions","summary":"Pier-X","text":"{{text}}"}',
    },
    {
      id: "minimal",
      label: t("Minimal JSON"),
      template:
        '{"event":"{{event}}","package":"{{packageId}}","host":"{{host}}","status":"{{status}}","version":"{{version}}","firedAt":{{firedAt}}}',
    },
    {
      // Slack-shaped body that includes the install/uninstall
      // command's last ~60 lines of stdout+stderr so triage
      // happens in-channel without an SSH round-trip.
      id: "slack-with-tail",
      label: t("Slack with output tail"),
      template:
        '{"text":"{{text}}\\n```\\n{{outputTail}}\\n```"}',
    },
    {
      // Discord variant of the above — same code-block format,
      // but Discord uses `content:` instead of Slack's `text:`.
      id: "discord-with-tail",
      label: t("Discord with output tail"),
      template:
        '{"content":"{{text}}\\n```\\n{{outputTail}}\\n```"}',
    },
  ];

  function removeEntry(idx: number) {
    setEntries((prev) => prev.filter((_, i) => i !== idx));
  }

  /**
   * Lint a webhook body template against the known placeholder set and
   * basic JSON validity. Runs locally on every keystroke so the user
   * sees errors before they save. Empty templates are considered valid
   * (the backend renders the default Slack-shaped JSON).
   */
  function lintTemplate(template: string): {
    jsonError?: string;
    unknownPlaceholders: string[];
  } {
    if (!template.trim()) return { unknownPlaceholders: [] };
    // 1) Unknown-placeholder scan.
    const placeholderRe = /\{\{(\w+)\}\}/g;
    const unknown = new Set<string>();
    for (const m of template.matchAll(placeholderRe)) {
      const name = m[1];
      if (!KNOWN_PLACEHOLDERS.has(name)) unknown.add(name);
    }
    // 2) JSON validity. We substitute placeholders with dummy values
    //    of the same shape (string for strings, number for firedAt) so
    //    the preview round-trip mirrors what the server will emit.
    const stub: Record<string, string> = {
      event: "install",
      status: "ok",
      package_id: "git",
      packageId: "git",
      host: "host",
      package_manager: "apt",
      packageManager: "apt",
      version: "1.0",
      fired_at: "0",
      firedAt: "0",
      text: "Pier-X · git installed",
      output_tail: "",
      outputTail: "",
    };
    const rendered = template.replace(placeholderRe, (_, key: string) =>
      stub[key] ?? `{{${key}}}`,
    );
    try {
      JSON.parse(rendered);
      return { unknownPlaceholders: Array.from(unknown) };
    } catch (e) {
      return {
        jsonError: e instanceof Error ? e.message : String(e),
        unknownPlaceholders: Array.from(unknown),
      };
    }
  }

  async function save() {
    setBusy(true);
    setMessage("");
    try {
      // Trim url + label so an accidental trailing space doesn't
      // produce a "URL doesn't start with http://" error at fire-
      // time. Drop fully-empty rows so a user can clear a row by
      // emptying the URL instead of having to click the × button.
      const cleaned = entries
        .map((e) => ({ ...e, url: e.url.trim(), label: e.label.trim() }))
        .filter((e) => e.url.length > 0);
      await cmd.softwareWebhooksSave({ entries: cleaned });
      setEntries(cleaned);
      setMessage(t("Webhook configuration saved."));
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  /** Open the export confirmation dialog. The actual write happens
   *  inside the dialog so the user gets to choose redaction policy
   *  before any file lands on disk. */
  function exportEntries() {
    setMessage("");
    setExportDialogOpen(true);
  }

  /** Import a JSON exported from `exportEntries`. Merges into the
   *  current list — duplicates by URL are skipped so re-importing
   *  the same file doesn't double up. The import lives in dialog
   *  state until the user clicks Save (matches save semantics). */
  async function importEntries() {
    setMessage("");
    try {
      const dialog = await import("@tauri-apps/plugin-dialog");
      const picked = await dialog.open({
        title: t("Import webhook config"),
        multiple: false,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!picked || typeof picked !== "string") return;
      const raw = await cmd.localReadTextFile(picked);
      const parsed = JSON.parse(raw);
      const incoming = Array.isArray(parsed?.entries)
        ? (parsed.entries as cmd.WebhookEntry[])
        : Array.isArray(parsed)
          ? (parsed as cmd.WebhookEntry[])
          : null;
      if (!incoming) {
        setMessage(t("File doesn't look like a webhook config."));
        return;
      }
      const seen = new Set(entries.map((e) => e.url.trim()));
      // Decode any `_pierx_b64:` markers so import is the inverse of
      // export-with-secrets. Unknown shapes (raw plaintext, missing
      // marker) pass through untouched.
      const decoded = incoming.map(decodeWebhookSecrets);
      const fresh = decoded.filter((e) => {
        const url = (e.url ?? "").trim();
        if (!url) return false;
        if (seen.has(url)) return false;
        seen.add(url);
        return true;
      });
      setEntries((prev) => [...prev, ...fresh]);
      setMessage(
        t("Imported {n} new webhook(s) ({skipped} duplicate skipped).", {
          n: fresh.length,
          skipped: incoming.length - fresh.length,
        }),
      );
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    }
  }

  async function testFire(idx: number) {
    const entry = entries[idx];
    if (!entry || !entry.url.trim()) return;
    setTesting(idx);
    setLastTest(null);
    try {
      const report = await cmd.softwareWebhooksTestFire({
        url: entry.url.trim(),
        bodyTemplate: entry.bodyTemplate ?? "",
        headers: entry.headers ?? [],
        hmacSecret: entry.hmacSecret ?? "",
      });
      setLastTest({ index: idx, report });
    } catch (e) {
      setLastTest({
        index: idx,
        report: {
          url: entry.url,
          statusCode: 0,
          latencyMs: 0,
          error: e instanceof Error ? e.message : String(e),
          attempts: 1,
        },
      });
    } finally {
      setTesting(null);
    }
  }

  /** Track the preview pane's open/closed state per row + the
   *  rendered body the backend produced for the most recent
   *  template. Re-renders on template change so the user sees the
   *  wire shape live. Pure-CPU backend call — round-trip is
   *  microseconds. */
  const [previewOpen, setPreviewOpen] = useState<number | null>(null);
  const [previewBody, setPreviewBody] = useState<string>("");
  const [previewBusy, setPreviewBusy] = useState(false);

  async function refreshPreview(idx: number) {
    const entry = entries[idx];
    if (!entry) return;
    setPreviewBusy(true);
    try {
      const body = await cmd.softwareWebhooksPreviewBody(
        entry.bodyTemplate ?? "",
      );
      setPreviewBody(body);
    } catch (e) {
      setPreviewBody(e instanceof Error ? e.message : String(e));
    } finally {
      setPreviewBusy(false);
    }
  }

  async function togglePreview(idx: number) {
    if (previewOpen === idx) {
      setPreviewOpen(null);
      return;
    }
    setPreviewOpen(idx);
    await refreshPreview(idx);
  }

  /** Resolve the actual write of the export, called by the export
   *  dialog's primary action. Builds the redacted / obfuscated JSON
   *  per the user's choice and writes via the local FS bridge. */
  async function performExport(includeSecrets: boolean) {
    setExportDialogOpen(false);
    setMessage("");
    try {
      const dialog = await import("@tauri-apps/plugin-dialog");
      const picked = await dialog.save({
        title: t("Export webhook config"),
        defaultPath: includeSecrets
          ? "pier-x-webhooks-with-secrets.json"
          : "pier-x-webhooks-redacted.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (typeof picked !== "string") return;
      const cleaned = entries.map((e) =>
        includeSecrets ? encodeWebhookSecrets(e) : redactWebhookSecrets(e),
      );
      const blob = JSON.stringify(
        {
          _meta: {
            redacted: !includeSecrets,
            includesSecrets: includeSecrets,
            exportedAt: new Date().toISOString(),
          },
          entries: cleaned,
        },
        null,
        2,
      );
      await cmd.localWriteTextFile(picked, blob);
      setMessage(
        includeSecrets
          ? t(
              "Exported {n} webhook(s) — secrets are base64-obfuscated, NOT encrypted. Verify the recipient.",
              { n: entries.length },
            )
          : t(
              "Exported {n} webhook(s) with secrets stripped. The recipient must re-enter HMAC / auth values after import.",
              { n: entries.length },
            ),
      );
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <Dialog
      open={open}
      title={t("Webhooks")}
      subtitle={t(
        "Fire an HTTP POST after each install / update / uninstall — Slack, Discord, Teams, or your own monitoring inbox.",
      )}
      size="md"
      onClose={onClose}
    >
      <div className="sw-webhooks">
        <div className="sw-webhooks__tabs" role="tablist">
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === "endpoints"}
            className={`sw-webhooks__tab ${
              activeTab === "endpoints" ? "sw-webhooks__tab--active" : ""
            }`}
            onClick={() => setActiveTab("endpoints")}
          >
            {t("Endpoints")}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === "failures"}
            className={`sw-webhooks__tab ${
              activeTab === "failures" ? "sw-webhooks__tab--active" : ""
            }`}
            onClick={() => {
              setActiveTab("failures");
              void refreshFailures();
            }}
          >
            {t("Failures")}
            {failures.length > 0 && (
              <span className="sw-webhooks__tab-badge">{failures.length}</span>
            )}
          </button>
        </div>
        {activeTab === "failures" ? (
          <div className="sw-webhooks__failures">
            <div className="sw-webhooks__failures-toolbar">
              <span className="muted mono">
                {t("{n} stored", { n: failures.length })}
              </span>
              <div className="sw-webhooks__failures-actions">
                <button
                  type="button"
                  className="btn is-ghost is-compact"
                  onClick={() => void refreshFailures()}
                  disabled={failureBusy}
                >
                  {failureBusy ? t("Refreshing…") : t("Refresh")}
                </button>
                <button
                  type="button"
                  className="btn is-ghost is-compact"
                  onClick={() => void replayRecent()}
                  disabled={failures.length === 0 || batchReplaying}
                  title={t(
                    "Re-fire the {n} most recent failures sequentially. Successful replays auto-dismiss.",
                    { n: BATCH_REPLAY_LIMIT },
                  )}
                >
                  {batchReplaying
                    ? t("Replaying…")
                    : t("Replay last {n}", { n: BATCH_REPLAY_LIMIT })}
                </button>
                <button
                  type="button"
                  className="btn is-ghost is-compact"
                  onClick={() => void clearFailures()}
                  disabled={failures.length === 0}
                >
                  {t("Clear all")}
                </button>
              </div>
            </div>
            {failures.length === 0 ? (
              <div className="sw-webhooks__empty">
                {t("No webhook failures recorded.")}
              </div>
            ) : (
              <ul className="sw-webhooks__failure-list">
                {failures.map((f) => {
                  const date = new Date(f.failedAt * 1000);
                  return (
                    <li key={f.id} className="sw-webhooks__failure">
                      <div className="sw-webhooks__failure-head">
                        <span className="sw-webhooks__failure-event mono">
                          {f.event}
                        </span>
                        <span className="sw-webhooks__failure-pkg mono">
                          {f.packageId}
                        </span>
                        {f.host && (
                          <span className="muted mono">{f.host}</span>
                        )}
                        <span className="muted mono sw-webhooks__failure-when">
                          {date.toLocaleString()}
                        </span>
                      </div>
                      <div className="sw-webhooks__failure-url mono">
                        {f.label && (
                          <span className="sw-webhooks__failure-label">
                            {f.label}
                          </span>
                        )}
                        <span>{f.url}</span>
                      </div>
                      <div className="status-note status-note--error mono">
                        {t("attempt {n}: {err}", {
                          n: f.attempts,
                          err: f.error,
                        })}
                      </div>
                      {replayResult && replayResult.id === f.id && (
                        <div
                          className={`mono sw-webhooks__test-result ${
                            replayResult.report.error
                              ? "sw-webhooks__test-result--error"
                              : "sw-webhooks__test-result--ok"
                          }`}
                        >
                          {replayResult.report.error
                            ? `✗ ${replayResult.report.statusCode || "?"} · ${replayResult.report.error}`
                            : `✓ ${replayResult.report.statusCode} · ${replayResult.report.latencyMs} ms`}
                        </div>
                      )}
                      <div className="sw-webhooks__failure-actions">
                        <button
                          type="button"
                          className="btn is-ghost is-compact"
                          onClick={() => void replayFailure(f)}
                          disabled={replayingId === f.id}
                        >
                          {replayingId === f.id ? t("Replaying…") : t("Replay")}
                        </button>
                        <button
                          type="button"
                          className="btn is-ghost is-compact"
                          onClick={() => void dismissFailure(f.id)}
                        >
                          {t("Dismiss")}
                        </button>
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        ) : entries.length === 0 ? (
          <div className="sw-webhooks__empty">
            {t("No webhooks configured.")}
          </div>
        ) : (
          <ul className="sw-webhooks__list">
            {entries.map((e, idx) => (
              <li key={idx} className="sw-webhooks__row">
                <div className="sw-webhooks__row-main">
                  <input
                    type="text"
                    className="sw-webhooks__input"
                    placeholder={t("https://hooks.slack.com/services/...")}
                    value={e.url}
                    onChange={(ev) =>
                      updateEntry(idx, { url: ev.target.value })
                    }
                  />
                  <input
                    type="text"
                    className="sw-webhooks__input sw-webhooks__input--label"
                    placeholder={t("Label (optional)")}
                    value={e.label}
                    onChange={(ev) =>
                      updateEntry(idx, { label: ev.target.value })
                    }
                  />
                </div>
                <div className="sw-webhooks__row-meta">
                  <span className="muted">{t("Events:")}</span>
                  {(["install", "update", "uninstall"] as const).map((k) => (
                    <label key={k} className="sw-webhooks__chip">
                      <input
                        type="checkbox"
                        checked={
                          e.events.length === 0 || e.events.includes(k)
                        }
                        onChange={() => {
                          // First explicit click on a chip seeds
                          // the events array with just that kind;
                          // an empty events array meant "all
                          // events" and we want the toggle to feel
                          // intuitive. Subsequent clicks toggle
                          // membership normally.
                          if (e.events.length === 0) {
                            updateEntry(idx, {
                              events: ["install", "update", "uninstall"]
                                .filter((kk) => kk !== k) as cmd.WebhookEventKindLabel[],
                            });
                          } else {
                            toggleEvent(idx, k);
                          }
                        }}
                      />
                      <span>{t(k.charAt(0).toUpperCase() + k.slice(1))}</span>
                    </label>
                  ))}
                  <label className="sw-webhooks__chip">
                    <input
                      type="checkbox"
                      checked={e.disabled}
                      onChange={(ev) =>
                        updateEntry(idx, { disabled: ev.target.checked })
                      }
                    />
                    <span>{t("Disabled")}</span>
                  </label>
                  <label
                    className="sw-webhooks__chip sw-webhooks__chip--num"
                    title={t(
                      "Retry attempts after the first failure. Capped at 5; 0 disables retries.",
                    )}
                  >
                    <span className="muted">{t("Retries")}</span>
                    <input
                      type="number"
                      min={0}
                      max={5}
                      value={e.maxRetries ?? 0}
                      onChange={(ev) => {
                        const n = Math.max(
                          0,
                          Math.min(5, Number(ev.target.value) || 0),
                        );
                        updateEntry(idx, { maxRetries: n });
                      }}
                    />
                  </label>
                  <label
                    className="sw-webhooks__chip sw-webhooks__chip--num"
                    title={t(
                      "Base seconds for exponential backoff. 0 = use the default (5s, doubling).",
                    )}
                  >
                    <span className="muted">{t("Backoff")}</span>
                    <input
                      type="number"
                      min={0}
                      max={60}
                      value={e.retryBackoffSecs ?? 0}
                      onChange={(ev) => {
                        const n = Math.max(
                          0,
                          Math.min(60, Number(ev.target.value) || 0),
                        );
                        updateEntry(idx, { retryBackoffSecs: n });
                      }}
                    />
                    <span className="muted">{t("s")}</span>
                  </label>
                  <div className="sw-webhooks__row-actions">
                    <button
                      type="button"
                      className="btn is-ghost is-compact"
                      onClick={() => void testFire(idx)}
                      disabled={!e.url.trim() || testing !== null}
                    >
                      {testing === idx ? t("Sending…") : t("Send test")}
                    </button>
                    <button
                      type="button"
                      className="btn is-ghost is-compact"
                      onClick={() => removeEntry(idx)}
                    >
                      <X size={10} /> {t("Remove")}
                    </button>
                  </div>
                </div>
                <div className="sw-webhooks__row-template">
                  <label className="sw-webhooks__template-label">
                    <span className="muted">{t("Body template")}</span>
                    <select
                      className="sw-webhooks__preset"
                      value=""
                      onChange={(ev) => {
                        const preset = TEMPLATE_PRESETS.find(
                          (p) => p.id === ev.target.value,
                        );
                        if (preset) {
                          updateEntry(idx, {
                            bodyTemplate: preset.template,
                          });
                          if (previewOpen === idx) {
                            void refreshPreview(idx);
                          }
                        }
                      }}
                    >
                      <option value="">{t("Apply preset…")}</option>
                      {TEMPLATE_PRESETS.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.label}
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      className="btn is-ghost is-compact"
                      onClick={() => void togglePreview(idx)}
                    >
                      {previewOpen === idx ? t("Hide preview") : t("Preview")}
                    </button>
                  </label>
                  <textarea
                    className="sw-webhooks__template"
                    placeholder={t(
                      'Empty = default Slack-shaped JSON. Custom example: {"content":"{{text}}"}',
                    )}
                    value={e.bodyTemplate ?? ""}
                    onChange={(ev) => {
                      updateEntry(idx, { bodyTemplate: ev.target.value });
                    }}
                    onBlur={() => {
                      if (previewOpen === idx) void refreshPreview(idx);
                    }}
                    rows={3}
                    spellCheck={false}
                  />
                  {(() => {
                    const lint = lintTemplate(e.bodyTemplate ?? "");
                    if (!lint.jsonError && lint.unknownPlaceholders.length === 0) {
                      return null;
                    }
                    return (
                      <div className="sw-webhooks__lint mono">
                        {lint.jsonError && (
                          <div className="sw-webhooks__lint-err">
                            <AlertTriangle size={10} />{" "}
                            {t("Invalid JSON: {err}", { err: lint.jsonError })}
                          </div>
                        )}
                        {lint.unknownPlaceholders.length > 0 && (
                          <div className="sw-webhooks__lint-warn">
                            {t("Unknown placeholders: {names}", {
                              names: lint.unknownPlaceholders
                                .map((n) => `{{${n}}}`)
                                .join(", "),
                            })}
                          </div>
                        )}
                      </div>
                    );
                  })()}
                  {previewOpen === idx && (
                    <pre className="sw-webhooks__preview mono">
                      {previewBusy ? t("Rendering…") : previewBody}
                    </pre>
                  )}
                  <WebhookHeadersEditor
                    headers={e.headers ?? []}
                    onChange={(next) => updateEntry(idx, { headers: next })}
                    t={t}
                  />
                  <label className="sw-webhooks__hmac mono">
                    <span className="muted">{t("HMAC secret")}</span>
                    <input
                      type="password"
                      className="sw-webhooks__hmac-input"
                      placeholder={t(
                        "Empty = no signing. Set a shared secret to send X-Pier-Signature.",
                      )}
                      value={e.hmacSecret ?? ""}
                      onChange={(ev) =>
                        updateEntry(idx, { hmacSecret: ev.target.value })
                      }
                      spellCheck={false}
                      autoCorrect="off"
                    />
                  </label>
                </div>
                {lastTest && lastTest.index === idx && (
                  <div
                    className={`mono sw-webhooks__test-result ${
                      lastTest.report.error
                        ? "sw-webhooks__test-result--error"
                        : "sw-webhooks__test-result--ok"
                    }`}
                  >
                    {lastTest.report.error
                      ? `✗ ${lastTest.report.statusCode || "?"} · ${lastTest.report.error}`
                      : `✓ ${lastTest.report.statusCode} · ${lastTest.report.latencyMs} ms`}
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}

        {activeTab === "endpoints" && (
          <div className="sw-webhooks__actions">
            <button
              type="button"
              className="btn is-ghost is-compact"
              onClick={addEntry}
            >
              + {t("Add webhook")}
            </button>
            <button
              type="button"
              className="btn is-ghost is-compact"
              onClick={() => void exportEntries()}
              disabled={entries.length === 0}
              title={t(
                "Save the current webhook config to a JSON file — share or move between machines.",
              )}
            >
              {t("Export…")}
            </button>
            <button
              type="button"
              className="btn is-ghost is-compact"
              onClick={() => void importEntries()}
              title={t(
                "Load a webhook JSON exported from this dialog (merges into the current list).",
              )}
            >
              {t("Import…")}
            </button>
            <div className="sw-webhooks__actions-right">
              <button
                type="button"
                className="btn is-ghost is-compact"
                onClick={onClose}
                disabled={busy}
              >
                {t("Close")}
              </button>
              <button
                type="button"
                className="btn is-primary is-compact"
                onClick={() => void save()}
                disabled={busy}
              >
                {busy ? t("Saving…") : t("Save")}
              </button>
            </div>
          </div>
        )}
        {activeTab === "failures" && (
          <div className="sw-webhooks__actions">
            <span />
            <div className="sw-webhooks__actions-right">
              <button
                type="button"
                className="btn is-ghost is-compact"
                onClick={onClose}
              >
                {t("Close")}
              </button>
            </div>
          </div>
        )}

        {message && <div className="sw-pg-form__msg mono">{message}</div>}
        {path && (
          <div className="sw-webhooks__path muted mono">
            {t("Stored at {path}", { path })}
          </div>
        )}
      </div>
      {exportDialogOpen && (
        <WebhookExportConfirmDialog
          entries={entries}
          onCancel={() => setExportDialogOpen(false)}
          onConfirm={(includeSecrets) => void performExport(includeSecrets)}
        />
      )}
    </Dialog>
  );
}

function WebhookExportConfirmDialog({
  entries,
  onCancel,
  onConfirm,
}: {
  entries: cmd.WebhookEntry[];
  onCancel: () => void;
  onConfirm: (includeSecrets: boolean) => void;
}) {
  const { t } = useI18n();
  const [includeSecrets, setIncludeSecrets] = useState(false);

  // Quick survey of what would land in the file with secrets included
  // vs stripped. The user sees the count up front so they don't have
  // to inspect the JSON before sharing.
  const secretSummary = (() => {
    let secretHeaders = 0;
    let hmacCount = 0;
    for (const e of entries) {
      for (const h of e.headers ?? []) {
        if (isSecretHeaderName(h.name) && h.value) secretHeaders += 1;
      }
      if (e.hmacSecret) hmacCount += 1;
    }
    return { secretHeaders, hmacCount };
  })();

  return (
    <div className="dlg-overlay" onClick={onCancel}>
      <div className="dlg" onClick={(e) => e.stopPropagation()}>
        <div className="dlg-head">
          <span className="dlg-title">{t("Export webhook config")}</span>
        </div>
        <div className="dlg-body dlg-body--form">
          <div className="status-note mono">
            {t(
              "{n} webhook(s) selected. Secrets in this config: {hmac} HMAC, {hdr} auth-style headers.",
              {
                n: entries.length,
                hmac: secretSummary.hmacCount,
                hdr: secretSummary.secretHeaders,
              },
            )}
          </div>
          <label className="sw-webhooks__export-flag">
            <input
              type="checkbox"
              checked={includeSecrets}
              onChange={(e) => setIncludeSecrets(e.currentTarget.checked)}
            />
            <span>
              {t(
                "Include sensitive fields (HMAC secrets, auth headers).",
              )}
            </span>
          </label>
          {includeSecrets ? (
            <div className="status-note status-note--error mono">
              {t(
                "Secrets will be base64-encoded with a `_pierx_b64:` prefix — this is OBFUSCATION, not encryption. Anyone with the file can decode them. Verify the recipient before sharing.",
              )}
            </div>
          ) : (
            <div className="status-note mono">
              {t(
                "Sensitive fields will be stripped. The recipient re-types HMAC / auth values after importing.",
              )}
            </div>
          )}
        </div>
        <div className="dlg-foot">
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={onCancel}
          >
            {t("Cancel")}
          </button>
          <button
            type="button"
            className={
              "btn is-compact " +
              (includeSecrets ? "is-danger" : "is-primary")
            }
            onClick={() => onConfirm(includeSecrets)}
          >
            {includeSecrets ? t("Export with secrets") : t("Export redacted")}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Modal editor for `software-extras.json`. Loads the file on
 *  open, validates the user's input as JSON live (no schema
 *  validation — the backend's `validate_and_leak` does the strict
 *  pass on next startup), saves back via Tauri. The header shows
 *  a "重启生效" reminder because the running process keeps the
 *  catalog it built at startup. */
function ExtrasEditorDialog({
  open,
  path,
  onClose,
}: {
  open: boolean;
  path: string | null;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [content, setContent] = useState("");
  const [parseError, setParseError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");

  // Load the file each time the dialog opens. Reset state so the
  // user doesn't see leftover messages from a prior session.
  useEffect(() => {
    if (!open) return;
    setMessage("");
    setBusy(true);
    cmd
      .softwareUserExtrasRead()
      .then((s) => {
        setContent(s);
        setParseError(null);
      })
      .catch((e) => setMessage(String(e)))
      .finally(() => setBusy(false));
  }, [open]);

  // Live-parse on every change so the user sees the JSON error
  // before they hit Save.
  useEffect(() => {
    const trimmed = content.trim();
    if (!trimmed) {
      setParseError(null);
      return;
    }
    try {
      JSON.parse(trimmed);
      setParseError(null);
    } catch (e) {
      setParseError(e instanceof Error ? e.message : String(e));
    }
  }, [content]);

  if (!open) return null;
  const canSave = !busy && parseError === null;
  const isEmpty = content.trim().length === 0;

  async function handleSave() {
    setBusy(true);
    setMessage("");
    try {
      await cmd.softwareUserExtrasWrite(content);
      setMessage(t("Saved. Restart Pier-X to apply changes."));
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  function loadTemplate() {
    setContent(
      JSON.stringify(
        {
          packages: [
            {
              id: "my-tool",
              displayName: "My Tool",
              category: "system",
              binaryName: "my-tool",
              probeCommand:
                "command -v my-tool >/dev/null 2>&1 && my-tool --version 2>&1",
              installPackages: { apt: ["my-tool"], dnf: ["my-tool"] },
              configPaths: [],
              defaultPorts: [],
              dataDirs: [],
              notes: "",
            },
          ],
          bundles: [
            {
              id: "my-stack",
              displayName: "My Stack",
              description: "personal favourites",
              packageIds: ["docker", "git", "my-tool"],
            },
          ],
        },
        null,
        2,
      ),
    );
  }

  return (
    <Dialog
      open={open}
      title={t("software-extras.json")}
      subtitle={path ?? undefined}
      size="md"
      onClose={onClose}
    >
      <div className="sw-extras-editor">
        <div className="sw-extras-editor__hint mono">
          <Info size={10} /> {t("Changes take effect on the next Pier-X restart.")}
        </div>
        <textarea
          className="sw-extras-editor__textarea mono"
          value={content}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          onChange={(e) => setContent(e.currentTarget.value)}
          placeholder={t("Paste or type JSON here. Click 'Insert template' for a starter.")}
          rows={20}
        />
        {parseError && (
          <div className="status-note status-note--error mono">
            {t("JSON parse error: {err}", { err: parseError })}
          </div>
        )}
        {message && <div className="sw-extras-editor__msg mono">{message}</div>}
        <div className="sw-extras-editor__actions">
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={loadTemplate}
            disabled={busy}
          >
            {t("Insert template")}
          </button>
          <div style={{ flex: 1 }} />
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={onClose}
            disabled={busy}
          >
            {t("Close")}
          </button>
          <button
            type="button"
            className={`btn is-compact ${
              isEmpty ? "is-danger" : "is-primary"
            }`}
            onClick={handleSave}
            disabled={!canSave}
            title={
              isEmpty
                ? t("Empty file → deletes the extras file")
                : undefined
            }
          >
            {busy
              ? t("Saving...")
              : isEmpty
                ? t("Delete file")
                : t("Save")}
          </button>
        </div>
      </div>
    </Dialog>
  );
}

/** Compact row for an apt-cache / dnf-search hit. No descriptor =
 *  no version picker / variant / details pane — just name +
 *  one-liner summary + an Install button. Activity log is shown
 *  inline when an install is in flight. */
function SystemPackageRow({
  hit,
  activity,
  onInstall,
}: {
  hit: SoftwareSearchHit;
  activity: { busy: boolean; log: string[]; error: string } | null;
  onInstall: () => void;
}) {
  const { t } = useI18n();
  const busy = activity?.busy ?? false;
  return (
    <div className="sw-row sw-row--system">
      <div className="sw-row__head">
        <span className="sw-row__status sw-row__status--missing">
          {busy ? (
            <Loader size={12} className="sw-row__spin" />
          ) : (
            <Circle size={12} />
          )}
        </span>
        <span className="sw-row__name">{hit.name}</span>
        <span className="sw-row__actions">
          <button
            type="button"
            className="btn is-primary is-compact"
            disabled={busy}
            onClick={onInstall}
          >
            <Download size={10} /> {busy ? t("Installing...") : t("Install")}
          </button>
        </span>
      </div>
      {hit.summary && <div className="sw-row__note mono">{hit.summary}</div>}
      {activity && activity.log.length > 0 && (
        <pre className="install-log mono sw-row__log">
          {activity.log.join("\n")}
        </pre>
      )}
      {activity?.error && (
        <div className="status-note status-note--error mono sw-row__error">
          {activity.error}
        </div>
      )}
    </div>
  );
}

/** Renders a list of paths in the row's details pane. Each path
 *  is a button that injects `cd <path>` into the tab's terminal
 *  (or copies to clipboard when no terminal session is attached
 *  — communicated via `hasLiveTerminal`). */
function PathList({
  paths,
  onCd,
  hasLiveTerminal,
}: {
  paths: string[];
  onCd: (path: string) => void;
  hasLiveTerminal: boolean;
}) {
  const { t } = useI18n();
  return (
    <span className="sw-row__path-list">
      {paths.map((p, i) => (
        <span key={`${p}-${i}`} className="sw-row__path-item">
          <button
            type="button"
            className="sw-row__path-btn mono"
            title={
              hasLiveTerminal
                ? t("cd into this path in the terminal")
                : t("Copy 'cd <path>' to clipboard")
            }
            onClick={() => onCd(p)}
          >
            {p}
          </button>
          <button
            type="button"
            className="icon-btn sw-row__path-copy"
            title={t("Copy path")}
            onClick={() => void writeClipboardText(p)}
          >
            <Copy size={10} />
          </button>
        </span>
      ))}
    </span>
  );
}

/** Lazy-loaded details pane shown when the user clicks the row's
 *  expand chevron. Renders install path, config files (filtered to
 *  ones that exist), default + listening ports, candidate version,
 *  and per-variant install state. Owns its own loading / error UI;
 *  the panel hands in the cached payload (or the `"loading"`
 *  sentinel / error tuple). */
function SoftwareRowDetails({
  status,
  details,
  onRefresh,
  onCdToPath,
  hasLiveTerminal,
  metrics,
}: {
  descriptor: SoftwareDescriptor;
  status: SoftwarePackageStatus | null;
  details: SoftwarePackageDetail | "loading" | { error: string } | null;
  onRefresh: () => void;
  onCdToPath: (path: string) => void;
  hasLiveTerminal: boolean;
  metrics: cmd.DbMetrics | null;
}) {
  const { t } = useI18n();
  if (details === null || details === "loading") {
    return (
      <div className="sw-row__details mono">
        <Loader size={10} className="sw-row__spin" />{" "}
        {t("Loading details...")}
      </div>
    );
  }
  if ("error" in details) {
    return (
      <div className="sw-row__details mono">
        <div className="status-note status-note--error">{details.error}</div>
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={onRefresh}
        >
          <RefreshCw size={10} /> {t("Retry")}
        </button>
      </div>
    );
  }
  const installed = details.installed;
  const latestKnown = details.latestVersion;
  const installedVersion = details.installedVersion ?? status?.version ?? null;
  const updateAvailable =
    !!installed &&
    !!latestKnown &&
    !!installedVersion &&
    latestKnown !== installedVersion;
  return (
    <div className="sw-row__details mono">
      <div className="sw-row__details-row">
        <span className="sw-row__details-label">
          <Info size={10} /> {t("Latest available")}
        </span>
        <span className="sw-row__details-val">
          {latestKnown ?? t("(unknown)")}
          {updateAvailable && (
            <span className="sw-row__details-pill">
              {t("update available")}
            </span>
          )}
        </span>
      </div>
      {installed && installedVersion && (
        <div className="sw-row__details-row">
          <span className="sw-row__details-label">
            {t("Installed version")}
          </span>
          <span className="sw-row__details-val">{installedVersion}</span>
        </div>
      )}
      {details.installPaths.length > 0 && (
        <div className="sw-row__details-row">
          <span className="sw-row__details-label">{t("Install path")}</span>
          <span className="sw-row__details-val">
            <PathList
              paths={details.installPaths}
              onCd={onCdToPath}
              hasLiveTerminal={hasLiveTerminal}
            />
          </span>
        </div>
      )}
      {details.configPaths.length > 0 && (
        <div className="sw-row__details-row">
          <span className="sw-row__details-label">{t("Config files")}</span>
          <span className="sw-row__details-val">
            <PathList
              paths={details.configPaths}
              onCd={onCdToPath}
              hasLiveTerminal={hasLiveTerminal}
            />
          </span>
        </div>
      )}
      {details.defaultPorts.length > 0 && (
        <div className="sw-row__details-row">
          <span className="sw-row__details-label">{t("Ports")}</span>
          <span className="sw-row__details-val">
            {t("default {ports}", { ports: details.defaultPorts.join(", ") })}
            {details.listenProbeOk && (
              <>
                {" · "}
                {details.listeningPorts.length > 0
                  ? t("listening on {ports}", {
                      ports: details.listeningPorts.join(", "),
                    })
                  : t("none listening")}
              </>
            )}
          </span>
        </div>
      )}
      {details.serviceUnit && (
        <div className="sw-row__details-row">
          <span className="sw-row__details-label">{t("Service unit")}</span>
          <span className="sw-row__details-val">{details.serviceUnit}</span>
        </div>
      )}
      {details.variants.length > 0 && (
        <div className="sw-row__details-row">
          <span className="sw-row__details-label">{t("Variants")}</span>
          <span className="sw-row__details-val">
            {details.variants
              .map((v) =>
                v.installed ? `✓ ${v.label}` : `· ${v.label}`,
              )
              .join("   ")}
          </span>
        </div>
      )}
      {metrics && (
        <div className="sw-row__metrics mono">
          <span className="sw-row__details-label">
            {t("Live metrics")}
          </span>
          {metrics.probeOk ? (
            <span className="sw-row__metrics-vals">
              {metrics.connections !== null && (
                <span className="sw-row__metrics-pill">
                  {t("conns: {n}", { n: metrics.connections })}
                </span>
              )}
              {metrics.memoryMib !== null && (
                <span className="sw-row__metrics-pill">
                  {t("mem: {n} MiB", { n: metrics.memoryMib })}
                </span>
              )}
              {metrics.extra && (
                <span className="sw-row__metrics-extra">{metrics.extra}</span>
              )}
            </span>
          ) : (
            <span className="sw-row__metrics-vals">
              {t("(probe failed — daemon down or auth required)")}
            </span>
          )}
        </div>
      )}
      <div className="sw-row__details-actions">
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={onRefresh}
        >
          <RefreshCw size={10} /> {t("Refresh details")}
        </button>
      </div>
    </div>
  );
}

function describeUninstallOutcome(
  report: SoftwareUninstallReport,
  t: ReturnType<typeof useI18n>["t"],
): string {
  switch (report.status) {
    case "uninstalled":
      return report.dataDirsRemoved
        ? t("Uninstalled · {pm} (data wiped)", {
            pm: report.packageManager || "—",
          })
        : t("Uninstalled · {pm}", { pm: report.packageManager || "—" });
    case "not-installed":
      return t("Not installed — nothing to remove.");
    case "unsupported-distro":
      return t(
        "This distro ({id}) is not in the auto-install list — please uninstall manually.",
        { id: report.distroId || "?" },
      );
    case "sudo-requires-password":
      return t(
        "sudo requires a password — connect as root or configure passwordless sudo.",
      );
    case "package-manager-failed":
      return t("Uninstall failed via {pm} (exit {code})", {
        pm: report.packageManager || "—",
        code: report.exitCode,
      });
    case "cancelled":
      return t("Cancelled");
  }
}

/** Per-row uninstall confirmation dialog. Three independent options
 *  + a name-typed gate for the destructive data-dir wipe. */
function UninstallDialog({
  target,
  onCancel,
  onConfirm,
}: {
  target: SoftwareDescriptor | null;
  onCancel: () => void;
  onConfirm: (options: UninstallOptions) => void;
}) {
  const { t } = useI18n();
  const [purgeConfig, setPurgeConfig] = useState(false);
  const [autoremove, setAutoremove] = useState(false);
  const [removeData, setRemoveData] = useState(false);
  const [removeUpstream, setRemoveUpstream] = useState(false);
  const [confirmText, setConfirmText] = useState("");

  // Reset every time a new target opens so options from a prior
  // dialog session don't leak into the next.
  useEffect(() => {
    setPurgeConfig(false);
    setAutoremove(false);
    setRemoveData(false);
    setRemoveUpstream(false);
    setConfirmText("");
  }, [target?.id]);

  if (!target) return null;
  const hasDataDirs = target.dataDirs.length > 0;
  const hasUpstreamCleanup =
    target.vendorScript?.hasCleanupScripts ?? false;
  const dataConfirmed = !removeData || confirmText === target.id;

  return (
    <Dialog
      open={!!target}
      title={t("Uninstall {name}", { name: target.displayName })}
      subtitle={target.notes ?? undefined}
      size="sm"
      onClose={onCancel}
      footer={
        <>
          <div style={{ flex: 1 }} />
          <button type="button" className="btn" onClick={onCancel}>
            {t("Cancel")}
          </button>
          <button
            type="button"
            className="btn is-danger"
            disabled={!dataConfirmed}
            onClick={() =>
              onConfirm({
                purgeConfig,
                autoremove,
                removeDataDirs: removeData,
                removeUpstreamSource: removeUpstream,
              })
            }
          >
            {t("Uninstall")}
          </button>
        </>
      }
    >
      <div className="sw-uninstall-form">
        <label className="sw-check">
          <input
            type="checkbox"
            checked={purgeConfig}
            onChange={(e) => setPurgeConfig(e.target.checked)}
          />
          <span>
            <span className="sw-check__title">{t("Also remove configuration")}</span>
            <span className="sw-check__hint">
              {t("apt purge / pacman -Rn. Without this, package config files stay on disk.")}
            </span>
          </span>
        </label>
        <label className="sw-check">
          <input
            type="checkbox"
            checked={autoremove}
            onChange={(e) => setAutoremove(e.target.checked)}
          />
          <span>
            <span className="sw-check__title">{t("Also clean up dependencies")}</span>
            <span className="sw-check__hint">
              {t("apt autoremove / dnf autoremove / zypper --clean-deps / pacman -Rs. No-op on apk.")}
            </span>
          </span>
        </label>
        {hasUpstreamCleanup && target.vendorScript && (
          <label className="sw-check">
            <input
              type="checkbox"
              checked={removeUpstream}
              onChange={(e) => setRemoveUpstream(e.target.checked)}
            />
            <span>
              <span className="sw-check__title">
                {t("Also remove upstream source ({label})", {
                  label: target.vendorScript.label,
                })}
              </span>
              <span className="sw-check__hint">
                {t(
                  "Drops the upstream apt source / yum repo this descriptor adds (e.g. pgdg.list). The distro packages stay reachable.",
                )}
              </span>
            </span>
          </label>
        )}
        {hasDataDirs && (
          <label className="sw-check sw-check--danger">
            <input
              type="checkbox"
              checked={removeData}
              onChange={(e) => setRemoveData(e.target.checked)}
            />
            <span>
              <span className="sw-check__title">
                {t("Also delete data directories (irreversible)")}
              </span>
              <span className="sw-check__hint">{target.dataDirs.join(", ")}</span>
            </span>
          </label>
        )}
        {removeData && hasDataDirs && (
          <div className="sw-uninstall-confirm">
            <div className="sw-check__title">
              {t("Type {name} to confirm.", { name: target.id })}
            </div>
            <input
              className="dlg-input"
              value={confirmText}
              onChange={(e) => setConfirmText(e.target.value)}
              placeholder={target.id}
              autoComplete="off"
              spellCheck={false}
            />
          </div>
        )}
      </div>
    </Dialog>
  );
}

const LOG_TAIL_LINES = 200;

type LogSshParams = {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex: number | null | undefined;
};

/** Per-row journalctl viewer. One-shot fetch of the last N lines on
 *  open + manual refresh button. No live tail — that's the Log
 *  panel's job; this dialog is "what just happened to the service?". */
function ServiceLogsDialog({
  target,
  sshParams,
  onClose,
}: {
  target: SoftwareDescriptor | null;
  sshParams: LogSshParams | null;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);
  const [lines, setLines] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const preRef = useRef<HTMLPreElement>(null);

  const refresh = useCallback(async () => {
    if (!target || !sshParams) return;
    setLoading(true);
    setError("");
    try {
      const out = await cmd.softwareServiceLogsRemote({
        ...sshParams,
        packageId: target.id,
        lines: LOG_TAIL_LINES,
      });
      setLines(out);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setLoading(false);
    }
    // formatError closes over `t` which is stable across renders for
    // the same i18n instance; no need to add to deps.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target?.id, sshParams]);

  // Reset + first fetch each time the dialog opens for a new target.
  useEffect(() => {
    setLines([]);
    setError("");
    if (target) void refresh();
  }, [target?.id, refresh]);

  // Pin scroll to the bottom (newest entry) after each refresh.
  useEffect(() => {
    if (preRef.current) preRef.current.scrollTop = preRef.current.scrollHeight;
  }, [lines.length]);

  if (!target) return null;
  return (
    <Dialog
      open={!!target}
      title={t("Logs · {name}", { name: target.displayName })}
      subtitle={t("journalctl -u <unit> -n {n}", { n: LOG_TAIL_LINES })}
      size="lg"
      onClose={onClose}
      footer={
        <>
          <div style={{ flex: 1 }} />
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={() => void refresh()}
            disabled={loading}
            title={t("Re-fetch the latest entries")}
          >
            <RefreshCw size={10} />
            {loading ? t("Loading...") : t("Refresh")}
          </button>
          <button type="button" className="btn" onClick={onClose}>
            {t("Close")}
          </button>
        </>
      }
    >
      {error ? (
        <div className="status-note status-note--error mono">{error}</div>
      ) : lines.length === 0 ? (
        <div className="status-note mono">
          {loading ? t("Loading...") : t("No journal entries found.")}
        </div>
      ) : (
        <pre ref={preRef} className="install-log mono sw-logs__pre">
          {lines.join("\n")}
        </pre>
      )}
    </Dialog>
  );
}

/** Confirm dialog for the v2 vendor-script install path. The user
 *  must explicitly check the "I understand Pier-X does not verify the
 *  script signature" box before the destructive [Continue] button
 *  unlocks. Default-focused button is [Cancel] so a stray Enter from
 *  the row doesn't auto-confirm. */
function VendorScriptConfirmDialog({
  target,
  onCancel,
  onConfirm,
}: {
  target: SoftwareDescriptor | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useI18n();
  const [understood, setUnderstood] = useState(false);
  const [copied, setCopied] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);

  // Reset every time a new target opens so the prior session's
  // "understood" tick doesn't leak into a fresh confirmation.
  useEffect(() => {
    setUnderstood(false);
    setCopied(false);
    if (target) {
      // Default focus on Cancel — consistent with the rest of
      // Pier-X's destructive dialogs.
      const id = window.setTimeout(() => cancelRef.current?.focus(), 0);
      return () => window.clearTimeout(id);
    }
  }, [target?.id]);

  if (!target || !target.vendorScript) return null;
  const script = target.vendorScript;

  return (
    <Dialog
      open={!!target}
      title={t("Install {name} via official script", {
        name: target.displayName,
      })}
      size="sm"
      onClose={onCancel}
      footer={
        <>
          <div style={{ flex: 1 }} />
          <button ref={cancelRef} type="button" className="btn" onClick={onCancel}>
            {t("Cancel")}
          </button>
          <button
            type="button"
            className="btn is-danger"
            disabled={!understood}
            onClick={onConfirm}
          >
            {t("Continue install")}
          </button>
        </>
      }
    >
      <div className="sw-vendor-form">
        <div className="sw-vendor-form__row">
          <div className="sw-check__title">{t("Script source")}</div>
          <div className="sw-vendor-url mono">
            <span className="sw-vendor-url__text">{script.url}</span>
            <button
              type="button"
              className="icon-btn"
              onClick={() => {
                void writeClipboardText(script.url).then(() => {
                  setCopied(true);
                  window.setTimeout(() => setCopied(false), 1200);
                });
              }}
              title={t("Copy URL")}
              aria-label={t("Copy URL")}
            >
              <Copy size={11} />
            </button>
          </div>
          {copied && (
            <div className="sw-check__hint">{t("Copied to clipboard")}</div>
          )}
        </div>
        <div className="sw-vendor-form__row">
          <div className="sw-check__title">{t("Maintainer note")}</div>
          <div className="sw-check__hint">{script.notes}</div>
        </div>
        {script.conflictsWithApt && (
          <div className="sw-vendor-form__warning mono">
            {t(
              "This installer may conflict with the distro package. Uninstall the apt version first if it's already on this host.",
            )}
          </div>
        )}
        <label className="sw-check">
          <input
            type="checkbox"
            checked={understood}
            onChange={(e) => setUnderstood(e.target.checked)}
          />
          <span>
            <span className="sw-check__title">
              {t("I understand Pier-X does not verify the script signature.")}
            </span>
          </span>
        </label>
      </div>
    </Dialog>
  );
}

function BundleSchedulesDialog({
  open,
  onClose,
  swKey,
  bundles,
  schedules,
  onChange,
  sshParams,
}: {
  open: boolean;
  onClose: () => void;
  swKey: string | null;
  bundles: SoftwareBundle[];
  schedules: BundleSchedule[];
  onChange: (next: BundleSchedule[]) => void;
  sshParams: cmd.SshParams | null;
}) {
  const { t } = useI18n();
  // Schedules belonging to other hosts are hidden in this dialog —
  // each panel manages only its own. Stored across all hosts in one
  // localStorage blob so the user doesn't have to re-add them when
  // switching tabs back.
  const visibleSchedules = swKey
    ? schedules.filter((s) => s.swKey === swKey)
    : [];

  function update(idx: number, patch: Partial<BundleSchedule>) {
    const id = visibleSchedules[idx]?.id;
    if (!id) return;
    onChange(
      schedules.map((s) => (s.id === id ? { ...s, ...patch } : s)),
    );
  }

  function remove(idx: number) {
    const id = visibleSchedules[idx]?.id;
    if (!id) return;
    onChange(schedules.filter((s) => s.id !== id));
  }

  function add() {
    if (!swKey) return;
    const firstBundle = bundles[0];
    if (!firstBundle) return;
    const fresh: BundleSchedule = {
      id: makeScheduleId(),
      swKey,
      bundleId: firstBundle.id,
      kind: "daily",
      hour: 3,
      minute: 0,
      enabled: false,
      label: firstBundle.displayName,
    };
    onChange([...schedules, fresh]);
  }

  /** Resolve a bundle's package set into a single shell one-liner the
   *  remote crontab can run, then chain that onto the schedule's cron
   *  expression and copy the result to the clipboard. The user pastes
   *  the line into `crontab -e` on the target host. We use `&&` so a
   *  failed package aborts the rest — same fail-fast contract as the
   *  in-app bundle runner. */
  async function copyCronLine(s: BundleSchedule) {
    if (!sshParams) {
      toast.warn(t("Connect to a host first."));
      return;
    }
    const bundle = bundles.find((b) => b.id === s.bundleId);
    if (!bundle) {
      toast.warn(t("Bundle not found"));
      return;
    }
    try {
      const previews = await Promise.all(
        bundle.packageIds.map((pkgId) =>
          cmd
            .softwareInstallPreview({ ...sshParams, packageId: pkgId })
            .catch(() => null),
        ),
      );
      const commands = previews
        .filter((p): p is cmd.InstallCommandPreview => p !== null)
        .map((p) => p.wrappedCommand);
      if (commands.length === 0) {
        toast.warn(t("Could not build install command for any package."));
        return;
      }
      const joined = commands.join(" && ");
      const line = buildCronLine(s, joined);
      if (!line) {
        toast.warn(t("Schedule cannot be expressed as a cron line."));
        return;
      }
      await writeClipboardText(line);
      toast.info(t("Copied cron line — paste into `crontab -e` on the host."));
    } catch (e) {
      toast.warn(
        t("Could not build cron line: {err}", {
          err: e instanceof Error ? e.message : String(e),
        }),
      );
    }
  }

  return (
    <Dialog
      open={open}
      title={t("Bundle schedules")}
      subtitle={t(
        "Run a bundle on a timer. Only fires while Pier-X is open and this panel is mounted for the matching host.",
      )}
      size="md"
      onClose={onClose}
    >
      <div className="sw-schedules">
        {!swKey && (
          <div className="status-note mono">
            {t("Connect to a host to manage its schedules.")}
          </div>
        )}
        {swKey && bundles.length === 0 && (
          <div className="status-note mono">
            {t("No bundles configured for this host.")}
          </div>
        )}
        {swKey && visibleSchedules.length === 0 && bundles.length > 0 && (
          <div className="status-note mono">
            {t("No schedules yet — click \"Add schedule\".")}
          </div>
        )}
        {visibleSchedules.map((s, idx) => {
          const bundle = bundles.find((b) => b.id === s.bundleId);
          return (
            <div key={s.id} className="sw-schedules__row">
              <div className="sw-schedules__row-head">
                <label className="sw-schedules__chip">
                  <input
                    type="checkbox"
                    checked={s.enabled}
                    onChange={(e) =>
                      update(idx, { enabled: e.currentTarget.checked })
                    }
                  />
                  <span>
                    {s.enabled ? t("Schedule on") : t("Schedule off")}
                  </span>
                </label>
                <select
                  className="sw-schedules__bundle mono"
                  value={s.bundleId}
                  onChange={(e) => {
                    const next = bundles.find(
                      (b) => b.id === e.currentTarget.value,
                    );
                    update(idx, {
                      bundleId: e.currentTarget.value,
                      label: next?.displayName,
                    });
                  }}
                >
                  {bundles.map((b) => (
                    <option key={b.id} value={b.id}>
                      {b.displayName}
                    </option>
                  ))}
                </select>
                <select
                  className="sw-schedules__kind mono"
                  value={s.kind}
                  onChange={(e) =>
                    update(idx, {
                      kind: e.currentTarget.value as BundleSchedule["kind"],
                    })
                  }
                >
                  <option value="interval">{t("Every N min")}</option>
                  <option value="daily">{t("Daily")}</option>
                  <option value="weekly">{t("Weekly")}</option>
                </select>
                <button
                  type="button"
                  className="btn is-ghost is-compact"
                  onClick={() => void copyCronLine(s)}
                  title={t(
                    "Build a cron line for this schedule + bundle and copy it to the clipboard. Paste into `crontab -e` on the target host so the bundle runs even when Pier-X is closed.",
                  )}
                >
                  <Copy size={10} /> {t("Cron line")}
                </button>
                <button
                  type="button"
                  className="btn is-ghost is-compact"
                  onClick={() => remove(idx)}
                  title={t("Remove")}
                >
                  <X size={10} />
                </button>
              </div>
              <div className="sw-schedules__row-body mono">
                {s.kind === "interval" && (
                  <label className="sw-schedules__field">
                    <span className="muted">{t("Every")}</span>
                    <input
                      type="number"
                      min={5}
                      max={1440}
                      value={s.intervalMinutes ?? 60}
                      onChange={(e) =>
                        update(idx, {
                          intervalMinutes: Math.max(
                            5,
                            Number(e.currentTarget.value) || 60,
                          ),
                        })
                      }
                    />
                    <span className="muted">{t("min")}</span>
                  </label>
                )}
                {(s.kind === "daily" || s.kind === "weekly") && (
                  <>
                    {s.kind === "weekly" && (
                      <label className="sw-schedules__field">
                        <span className="muted">{t("Weekday")}</span>
                        <select
                          value={s.weekday ?? 0}
                          onChange={(e) =>
                            update(idx, {
                              weekday: Number(e.currentTarget.value),
                            })
                          }
                        >
                          <option value={0}>{t("Sun")}</option>
                          <option value={1}>{t("Mon")}</option>
                          <option value={2}>{t("Tue")}</option>
                          <option value={3}>{t("Wed")}</option>
                          <option value={4}>{t("Thu")}</option>
                          <option value={5}>{t("Fri")}</option>
                          <option value={6}>{t("Sat")}</option>
                        </select>
                      </label>
                    )}
                    <label className="sw-schedules__field">
                      <span className="muted">{t("At")}</span>
                      <input
                        type="number"
                        min={0}
                        max={23}
                        value={s.hour ?? 0}
                        onChange={(e) =>
                          update(idx, {
                            hour: Math.max(
                              0,
                              Math.min(23, Number(e.currentTarget.value) || 0),
                            ),
                          })
                        }
                      />
                      <span>:</span>
                      <input
                        type="number"
                        min={0}
                        max={59}
                        value={s.minute ?? 0}
                        onChange={(e) =>
                          update(idx, {
                            minute: Math.max(
                              0,
                              Math.min(59, Number(e.currentTarget.value) || 0),
                            ),
                          })
                        }
                      />
                    </label>
                  </>
                )}
                <span className="sw-schedules__summary muted">
                  {describeSchedule(s)}
                </span>
                {s.lastRunAt && (
                  <span className="sw-schedules__last muted">
                    {t("Last run: {when}", {
                      when: new Date(s.lastRunAt).toLocaleString(),
                    })}
                  </span>
                )}
                {!bundle && (
                  <span className="sw-schedules__warn">
                    {t("Bundle not found")}
                  </span>
                )}
              </div>
            </div>
          );
        })}
        {swKey && bundles.length > 0 && (
          <div className="sw-schedules__add">
            <button type="button" className="btn is-ghost is-compact" onClick={add}>
              <Plus size={10} /> {t("Add schedule")}
            </button>
          </div>
        )}
      </div>
    </Dialog>
  );
}
