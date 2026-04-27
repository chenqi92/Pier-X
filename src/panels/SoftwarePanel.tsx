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
import type {
  MirrorChoice,
  MirrorId,
  MirrorState,
  SoftwareBundle,
  SoftwareDescriptor,
  SoftwareInstallReport,
  SoftwarePackageDetail,
  SoftwarePackageStatus,
  SoftwareServiceAction,
  SoftwareServiceActionReport,
  SoftwareUninstallReport,
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
  /** Path of the user-extras JSON file shown in the panel footer
   *  so users discover where to add their own entries. `null` =
   *  src-tauri couldn't resolve a config dir on this OS. */
  const [extrasPath, setExtrasPath] = useState<string | null>(null);
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
  const [mirrorBusy, setMirrorBusy] = useState<"set" | "restore" | null>(null);
  const [mirrorMessage, setMirrorMessage] = useState<string>("");
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
      if (opening) void loadDetailsForPackage(packageId);
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
      <BundleConfirmDialog
        target={bundleTarget}
        registry={registry}
        statuses={statuses}
        onCancel={() => setBundleTarget(null)}
        onConfirm={() => {
          const target = bundleTarget;
          setBundleTarget(null);
          if (target) void runBundle(target);
        }}
      />
      <MirrorDialog
        open={mirrorDialogOpen}
        onClose={() => setMirrorDialogOpen(false)}
        catalog={mirrorCatalog}
        state={mirrorState}
        preferred={preferredMirror}
        busy={mirrorBusy}
        message={mirrorMessage}
        onApply={(id) => void applyMirror(id)}
        onRestore={() => void restoreMirror()}
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
  onConfirm,
}: {
  target: SoftwareBundle | null;
  registry: SoftwareDescriptor[];
  statuses: Record<string, SoftwarePackageStatus>;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useI18n();
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
  return (
    <Dialog
      open={!!target}
      title={t("Install bundle: {name}", { name: target.displayName })}
      subtitle={target.description}
      size="sm"
      onClose={onCancel}
    >
      <div className="sw-bundle-form">
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
          {toInstall.length === 0
            ? t("Nothing to install — every member is already on this host.")
            : t("Will install {n} package(s) sequentially.", {
                n: toInstall.length,
              })}
        </div>
        <div className="sw-bundle-form__actions">
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={onCancel}
          >
            {t("Cancel")}
          </button>
          <button
            type="button"
            className="btn is-primary is-compact"
            disabled={toInstall.length === 0}
            onClick={onConfirm}
          >
            <Download size={10} /> {t("Install bundle")}
          </button>
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
  busy,
  message,
  onApply,
  onRestore,
}: {
  open: boolean;
  onClose: () => void;
  catalog: MirrorChoice[];
  state: MirrorState | null;
  preferred: MirrorId | null;
  busy: "set" | "restore" | null;
  message: string;
  onApply: (id: MirrorId) => void;
  onRestore: () => void;
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
        <div className="sw-mirror-form__list">
          {catalog.map((m) => {
            const active = currentId === m.id;
            const isSuggested =
              !active && currentId === null && preferred === m.id;
            const host = mirrorHostForManager(m, manager);
            return (
              <button
                key={m.id}
                type="button"
                className={`sw-mirror-row${active ? " is-active" : ""}${
                  isSuggested ? " is-suggested" : ""
                }`}
                disabled={!!busy || !manager}
                onClick={() => onApply(m.id)}
              >
                <span className="sw-mirror-row__label">
                  {m.label}
                  {isSuggested && (
                    <span className="sw-mirror-row__suggest-pill">
                      {t("last used")}
                    </span>
                  )}
                </span>
                <span className="sw-mirror-row__host mono">{host}</span>
                {active && <Check size={12} className="sw-mirror-row__check" />}
              </button>
            );
          })}
        </div>
        {message && <div className="sw-mirror-form__msg mono">{message}</div>}
        <div className="sw-mirror-form__actions">
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
    <div className="sw-row">
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
        />
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
        <button
          key={`${p}-${i}`}
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
}: {
  descriptor: SoftwareDescriptor;
  status: SoftwarePackageStatus | null;
  details: SoftwarePackageDetail | "loading" | { error: string } | null;
  onRefresh: () => void;
  onCdToPath: (path: string) => void;
  hasLiveTerminal: boolean;
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
