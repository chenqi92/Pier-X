import {
  Check,
  ChevronDown,
  Circle,
  Copy,
  Download,
  FileText,
  Loader,
  MoreHorizontal,
  Package,
  Play,
  RefreshCw,
  RotateCw,
  Square,
  Trash2,
  Zap,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import * as cmd from "../lib/commands";
import type {
  SoftwareDescriptor,
  SoftwareInstallReport,
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
  /** In-flight version-list fetches, keyed by package id. The
   *  dropdown shows a spinner row while present. */
  const [versionsLoading, setVersionsLoading] = useState<Record<string, boolean>>(
    {},
  );
  const setCancelling = useSoftwareStore((s) => s.setCancelling);

  const [registry, setRegistry] = useState<SoftwareDescriptor[]>([]);
  const [enableService, setEnableService] = useState(true);
  const [probing, setProbing] = useState(false);
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
    return () => {
      cancelled = true;
    };
  }, []);

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

  // Probe on host change.
  useEffect(() => {
    if (!sshParams || !swKey) return;
    void probe();
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
   *  shows a "Loading versions..." row while in flight. */
  async function loadVersionsForPackage(packageId: string) {
    if (!sshParams || !swKey || !snapshot) return;
    if (isVersionCacheFresh(snapshot, packageId)) return;
    if (versionsLoading[packageId]) return;
    setVersionsLoading((prev) => ({ ...prev, [packageId]: true }));
    try {
      const versions = await cmd.softwareVersionsRemote({
        ...sshParams,
        packageId,
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
    // The store only knows two kinds of activity ("install" / "update" /
    // "uninstall"); collapse the vendor variant to "install" so the
    // existing "Installing…" label and busy-row dimming keep working.
    startActivity(
      swKey,
      descriptor.id,
      installId,
      action === "install-vendor" ? "install" : action,
    );
    const unlisten = await cmd.subscribeSoftwareInstall(installId, (evt) => {
      if (evt.kind === "line") {
        appendLine(swKey, descriptor.id, evt.text);
      }
    });
    try {
      const params = {
        ...sshParams,
        packageId: descriptor.id,
        installId,
        enableService,
        version: selectedVersions[descriptor.id],
        ...(action === "install-vendor" ? { viaVendorScript: true } : {}),
      };
      const report: SoftwareInstallReport =
        action === "update"
          ? await cmd.softwareUpdateRemote(params)
          : await cmd.softwareInstallRemote(params);
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
    } catch (e) {
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

  return (
    <div className="sw-panel">
      <div className="sw-panel__header">
        <div className="sw-panel__title mono">
          <Package size={12} /> {t("Software")} · {sshTarget.user}@{sshTarget.host}
        </div>
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
      <div className="sw-panel__list">
        <label className="sw-panel__service-toggle mono">
          <input
            type="checkbox"
            checked={enableService}
            onChange={(e) => setEnableService(e.currentTarget.checked)}
          />
          {t("After install, also enable & start the systemd service")}
        </label>
        {registry.map((descriptor) => (
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
            onCancel={() => void cancelRow(descriptor.id)}
            onVendorPick={() => setVendorTarget(descriptor)}
            onAction={(action) => void runInstall(descriptor, action)}
          />
        ))}
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
    </div>
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
}: {
  t: ReturnType<typeof useI18n>["t"];
  action: "install" | "update";
  busy: boolean;
  activityKind: SoftwareActivityKind | undefined;
  selectedVersion: string | undefined;
}): string {
  if (busy) return busyLabel(activityKind, action, t);
  if (selectedVersion) {
    return action === "update"
      ? t("Update to v{version}", { version: selectedVersion })
      : t("Install v{version}", { version: selectedVersion });
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
  onSelectVersion,
  onLoadVersions,
  onAction,
  onUninstall,
  onServiceAction,
  onViewLogs,
  onCancel,
  onVendorPick,
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
  onSelectVersion: (version: string | undefined) => void;
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
  /** Trigger backend cancel for the row's in-flight activity. */
  onCancel: () => void;
  /** Open the vendor-script confirm dialog. Only invoked from the
   *  install-channel chooser when the descriptor exposes a
   *  `vendorScript`. */
  onVendorPick: () => void;
}) {
  const { t } = useI18n();
  const logRef = useRef<HTMLPreElement>(null);
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const versionButtonRef = useRef<HTMLButtonElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [versionMenuOpen, setVersionMenuOpen] = useState(false);
  const channelButtonRef = useRef<HTMLButtonElement>(null);
  const [channelMenuOpen, setChannelMenuOpen] = useState(false);
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
  const [confirmText, setConfirmText] = useState("");

  // Reset every time a new target opens so options from a prior
  // dialog session don't leak into the next.
  useEffect(() => {
    setPurgeConfig(false);
    setAutoremove(false);
    setRemoveData(false);
    setConfirmText("");
  }, [target?.id]);

  if (!target) return null;
  const hasDataDirs = target.dataDirs.length > 0;
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
