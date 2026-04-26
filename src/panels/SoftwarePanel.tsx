import {
  Check,
  ChevronDown,
  Circle,
  Download,
  Loader,
  MoreHorizontal,
  Package,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import * as cmd from "../lib/commands";
import type {
  SoftwareDescriptor,
  SoftwareInstallReport,
  SoftwarePackageStatus,
  SoftwareUninstallReport,
  UninstallOptions,
} from "../lib/commands";
import { describeInstallOutcome } from "../lib/softwareInstall";
import { effectiveSshTarget, type TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import {
  activePackageId,
  isVersionCacheFresh,
  softwareKeyForTab,
  useSoftwareStore,
} from "../stores/useSoftwareStore";
import Dialog from "../components/Dialog";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";
import Popover from "../components/Popover";

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

  const [registry, setRegistry] = useState<SoftwareDescriptor[]>([]);
  const [enableService, setEnableService] = useState(true);
  const [probing, setProbing] = useState(false);
  /** Open uninstall-dialog target. The dialog reads dataDirs / id /
   *  displayName from this descriptor to decide which checkboxes
   *  appear and what name the user must type to confirm a wipe. */
  const [uninstallTarget, setUninstallTarget] = useState<SoftwareDescriptor | null>(null);

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

  /** Kick off an uninstall for `descriptor` with the dialog's options.
   *  Mirrors the install handler's lifecycle: generate an installId,
   *  start activity, subscribe to the per-installId stream, fire the
   *  command, mirror outcome into the store, then unsubscribe. */
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
    const unlisten = await cmd.subscribeSoftwareUninstall(installId, (evt) => {
      if (evt.kind === "line") {
        appendLine(swKey, descriptor.id, evt.text);
      }
    });
    try {
      const report: SoftwareUninstallReport = await cmd.softwareUninstallRemote({
        ...sshParams,
        packageId: descriptor.id,
        installId,
        options,
      });
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
      finishActivity(swKey, descriptor.id, formatError(e), null);
    } finally {
      unlisten();
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
            onAction={async (action) => {
              if (!sshParams || !swKey) return;
              const installId =
                typeof crypto !== "undefined" && "randomUUID" in crypto
                  ? crypto.randomUUID()
                  : `${Date.now()}-${Math.random()}`;
              startActivity(swKey, descriptor.id, installId, action);
              const unlisten = await cmd.subscribeSoftwareInstall(
                installId,
                (evt) => {
                  if (evt.kind === "line") {
                    appendLine(swKey, descriptor.id, evt.text);
                  }
                  // The `done` and `failed` paths are handled by the
                  // promise resolve/reject below — no extra work here.
                },
              );
              try {
                const params = {
                  ...sshParams,
                  packageId: descriptor.id,
                  installId,
                  enableService,
                  version: selectedVersions[descriptor.id],
                };
                const report: SoftwareInstallReport =
                  action === "update"
                    ? await cmd.softwareUpdateRemote(params)
                    : await cmd.softwareInstallRemote(params);
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
            }}
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
    </div>
  );
}

/** Pick the label shown on the primary install/update button. Encodes
 *  the four states: busy install / busy update / busy uninstall →
 *  "...ing"; idle → "Install" or "Update", with the selected version
 *  appended when the user has pinned one. */
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
  activityKind: "install" | "update" | "uninstall" | undefined;
  selectedVersion: string | undefined;
}): string {
  if (busy) {
    if (activityKind === "uninstall") return t("Uninstalling...");
    if (action === "update") return t("Updating...");
    return t("Installing...");
  }
  if (selectedVersion) {
    return action === "update"
      ? t("Update to v{version}", { version: selectedVersion })
      : t("Install v{version}", { version: selectedVersion });
  }
  return action === "update" ? t("Update") : t("Install");
}

// `describeOutcome` was duplicated here pre-rebase; the canonical
// implementation now lives in `src/lib/softwareInstall.ts` as
// `describeInstallOutcome` (already imported at the top of this file).

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
}: {
  descriptor: SoftwareDescriptor;
  status: SoftwarePackageStatus | null;
  activity:
    | {
        installId: string;
        kind: "install" | "update" | "uninstall";
        log: string[];
        error: string;
        busy: boolean;
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
}) {
  const { t } = useI18n();
  const logRef = useRef<HTMLPreElement>(null);
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const versionButtonRef = useRef<HTMLButtonElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [versionMenuOpen, setVersionMenuOpen] = useState(false);
  const installed = status?.installed ?? false;
  const version = status?.version ?? null;
  const busy = activity?.busy ?? false;
  const action: "install" | "update" = installed ? "update" : "install";
  const buttonDisabled = busy || disabledOtherBusy || !canManage;
  const menuDisabled = busy || disabledOtherBusy;

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
          {installed && status?.serviceActive === true && (
            <span className="sw-row__service-active"> · {t("service running")}</span>
          )}
        </span>
        <span className="sw-row__actions">
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
            width={180}
            className="ctx-menu sw-row-menu"
          >
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
