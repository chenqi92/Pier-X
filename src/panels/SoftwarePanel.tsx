import {
  Check,
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
  const busyPackageId = snapshot ? activePackageId(snapshot) : null;
  const canManage = env?.packageManager !== null && env?.packageManager !== undefined;

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

function SoftwareRow({
  descriptor,
  status,
  activity,
  disabledOtherBusy,
  canManage,
  enableService: _enableService,
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
  onAction: (action: "install" | "update") => Promise<void> | void;
  /** Open the uninstall dialog for this row. The panel owns the
   *  dialog state so only one dialog is ever mounted at a time. */
  onUninstall: () => void;
}) {
  const { t } = useI18n();
  const logRef = useRef<HTMLPreElement>(null);
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
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
          <button
            type="button"
            className="btn is-primary is-compact"
            disabled={buttonDisabled}
            onClick={() => void onAction(action)}
          >
            <Download size={10} />
            {busy
              ? activity?.kind === "uninstall"
                ? t("Uninstalling...")
                : action === "update"
                  ? t("Updating...")
                  : t("Installing...")
              : action === "update"
                ? t("Update")
                : t("Install")}
          </button>
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
