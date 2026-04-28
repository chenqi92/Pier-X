import {
  Check,
  ChevronDown,
  ChevronRight,
  Circle,
  Copy,
  Download,
  FileText,
  Info,
  Loader,
  MoreHorizontal,
  Package,
  Play,
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
import { writeClipboardText } from "../lib/clipboard";
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

  /** Install all packages in `bundle` sequentially. Skips ones
   *  that are already installed (so re-running a bundle on a
   *  partially-set-up host is fast). Per-package progress flows
   *  through the existing per-row activity log. */
  async function runBundle(bundle: SoftwareBundle) {
    if (!sshParams || !swKey || bundleRunning) return;
    setBundleRunning(bundle.id);
    try {
      for (const pkgId of bundle.packageIds) {
        const descriptor = registry.find((d) => d.id === pkgId);
        if (!descriptor) continue;
        const cur = statuses[pkgId];
        if (cur?.installed) continue;
        // Reuse the per-row install path so the activity log,
        // cancel button, and outcome handling stay consistent.
        // eslint-disable-next-line no-await-in-loop
        await runInstall(descriptor, "install");
        // Bail out early if the user cancelled mid-bundle.
        const after = useSoftwareStore.getState().get(swKey).statuses[pkgId];
        if (!after?.installed) {
          // Surface as a warning in the panel: the bundle stops
          // at the first failure so the user can react before
          // the next package's apt cycle.
          break;
        }
      }
    } finally {
      setBundleRunning(null);
    }
  }

  /** Reverse of `runBundle` — uninstall everything in `bundle` in
   *  reverse install order (services first, then their deps).
   *  Skips members that aren't installed. Uses safe defaults
   *  (no purge / no autoremove / no data-dir wipe / no upstream
   *  cleanup) so accidental clicks can't nuke postgres data. */
  async function runBundleUninstall(bundle: SoftwareBundle) {
    if (!sshParams || !swKey || bundleRunning) return;
    setBundleRunning(bundle.id);
    try {
      // Reverse-iterate so daemons go down before their CLI deps.
      for (let i = bundle.packageIds.length - 1; i >= 0; i--) {
        const pkgId = bundle.packageIds[i];
        const descriptor = registry.find((d) => d.id === pkgId);
        if (!descriptor) continue;
        const cur = statuses[pkgId];
        if (!cur?.installed) continue;
        // eslint-disable-next-line no-await-in-loop
        await runUninstall(descriptor, {
          purgeConfig: false,
          autoremove: false,
          removeDataDirs: false,
          removeUpstreamSource: false,
        });
        const after = useSoftwareStore.getState().get(swKey).statuses[pkgId];
        if (after?.installed) {
          // Stop on first failure — same reasoning as runBundle.
          break;
        }
      }
    } finally {
      setBundleRunning(null);
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
                <button
                  key={b.id}
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
                      {t("Installing bundle...")}
                    </div>
                  )}
                </button>
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
}: {
  target: SoftwareBundle | null;
  registry: SoftwareDescriptor[];
  statuses: Record<string, SoftwarePackageStatus>;
  onCancel: () => void;
  onInstall: () => void;
  onUninstall: () => void;
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
            <button
              type="button"
              className="btn is-primary is-compact"
              disabled={toInstall.length === 0}
              onClick={onInstall}
            >
              <Download size={10} /> {t("Install bundle")}
            </button>
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
      }
    | null;
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
      {activity && (activity.busy || activity.log.length > 0 || activity.error) && (
        <>
          {activity.error && (
            <div className="status-note status-note--error mono sw-row__error">
              {activity.error}
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

  useEffect(() => {
    if (!target) return;
    setMessage("");
    setPreviewId(null);
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
