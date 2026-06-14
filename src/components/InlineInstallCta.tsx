import { Download, Loader } from "lucide-react";
import { useEffect, useRef } from "react";

import * as cmd from "../lib/commands";
import type { SoftwareInstallReport, SshParams } from "../lib/commands";
import { describeInstallOutcome } from "../lib/softwareInstall";
import { useSudoRetry } from "../lib/useSudoRetry";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import {
  activePackageId,
  useSoftwareStore,
} from "../stores/useSoftwareStore";
import { useSudoStore } from "../stores/useSudoStore";

/** Registry ids declared in `pier-core/src/services/package_manager.rs`. */
type PackageId = "docker" | "redis" | "mariadb" | "postgres" | "sqlite3";

type Props = {
  packageId: PackageId;
  sshParams: SshParams | null;
  /** softwareKeyForTab(tab) — required so log/activity survive panel
   *  remount and single-flight is enforced per host. */
  swKey: string | null;
  /** Whether to enable + start the systemd service after install. Docker
   *  panels pass true (dockerd is the point); DB panels pass false to
   *  avoid silently exposing a freshly-bound daemon on the SSH host. */
  enableService: boolean;
  /** One-line context above the button (e.g. "Docker not installed"). */
  hint?: string;
  /** Fires once when the install report comes back with status === "installed". */
  onInstalled?: () => void;
};

/**
 * Inline "Install <tool>" CTA. Reads probe state from `useSoftwareStore`
 * — the panel must have already kicked the probe (typically via
 * `useSoftwareSnapshot`). Renders nothing when:
 *   - no snapshot / no env yet (probe still in flight),
 *   - the host's distro isn't in the supported registry (renders a
 *     "manual install" note instead of a button).
 */
export default function InlineInstallCta({
  packageId,
  sshParams,
  swKey,
  enableService,
  hint,
  onInstalled,
}: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);

  const snapshot = useSoftwareStore((s) => (swKey ? s.get(swKey) : null));
  const startActivity = useSoftwareStore((s) => s.startActivity);
  const appendLine = useSoftwareStore((s) => s.appendLine);
  const finishActivity = useSoftwareStore((s) => s.finishActivity);

  const activity = snapshot?.activity[packageId] ?? null;
  const busyId = snapshot ? activePackageId(snapshot) : null;
  const otherBusy = !!busyId && busyId !== packageId;
  const env = snapshot?.env ?? null;
  const canManage = !!env?.packageManager;

  const logRef = useRef<HTMLPreElement>(null);
  // Guard against firing onInstalled twice when the store snapshot
  // re-emits during a parent re-render.
  const installedFiredRef = useRef<string | null>(null);

  // Same sudo flow the Software panel uses: cached/keychain password →
  // pass it to the install → on `sudo-requires-password`, prompt + retry.
  const hostLabel = sshParams
    ? `${sshParams.user}@${sshParams.host}`
    : t("the remote host");
  const { withSudoRetry, sudoDialog } = useSudoRetry(sshParams, hostLabel);

  // Lift any persisted elevation password from the OS keychain into the
  // in-memory cache the first time we see this host, so the retry path
  // can reuse it without a prompt round-trip.
  useEffect(() => {
    if (!sshParams) return;
    void useSudoStore.getState().hydrate(sshParams);
  }, [sshParams]);

  useEffect(() => {
    if (!activity || !logRef.current) return;
    logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [activity?.log.length]);

  async function runInstall() {
    if (!sshParams || !swKey || !canManage) return;
    if (activity?.busy || otherBusy) return;
    // Each sudo retry attempt owns its own installId + subscribe cycle so
    // the streamed log stays in sync with the run the backend emits
    // against. `startActivity` fires once; the captured installId from the
    // last attempt is what `onInstalled` dedupes on.
    let firstAttempt = true;
    let lastInstallId: string | null = null;
    try {
      const report = await withSudoRetry<SoftwareInstallReport>(
        async (sudoPassword) => {
          const installId =
            typeof crypto !== "undefined" && "randomUUID" in crypto
              ? crypto.randomUUID()
              : `${Date.now()}-${Math.random()}`;
          lastInstallId = installId;
          if (firstAttempt) {
            startActivity(swKey, packageId, installId, "install");
            firstAttempt = false;
          }
          const unlisten = await cmd.subscribeSoftwareInstall(
            installId,
            (evt) => {
              if (evt.kind === "line") appendLine(swKey, packageId, evt.text);
            },
          );
          try {
            return await cmd.softwareInstallRemote({
              ...sshParams,
              packageId,
              installId,
              enableService,
              sudoPassword: sudoPassword ?? null,
            });
          } finally {
            unlisten();
          }
        },
      );
      const localized = describeInstallOutcome(report, t);
      const nextStatus = {
        id: packageId,
        installed: report.status === "installed",
        version: report.installedVersion,
        serviceActive: report.serviceActive,
      };
      finishActivity(
        swKey,
        packageId,
        report.status === "installed" ? "" : localized,
        nextStatus,
      );
      if (
        report.status === "installed" &&
        lastInstallId &&
        installedFiredRef.current !== lastInstallId
      ) {
        installedFiredRef.current = lastInstallId;
        onInstalled?.();
      }
    } catch (e) {
      finishActivity(swKey, packageId, formatError(e), null);
    }
  }

  if (!snapshot || !env) return null;

  if (!canManage) {
    return (
      <div className="form-stack">
        {hint && <div className="status-note mono">{hint}</div>}
        <div className="status-note mono">
          {t(
            "Distro \"{id}\" is not in the supported list. Install software manually for now.",
            { id: env.distroId || "?" },
          )}
        </div>
      </div>
    );
  }

  const busy = activity?.busy ?? false;
  const buttonDisabled = !sshParams || busy || otherBusy;
  return (
    <div className="form-stack">
      {hint && <div className="status-note mono">{hint}</div>}
      <div className="branch-row">
        <button
          type="button"
          className="btn is-primary is-compact"
          disabled={buttonDisabled}
          onClick={() => void runInstall()}
        >
          {busy ? (
            <Loader size={10} className="sw-row__spin" />
          ) : (
            <Download size={10} />
          )}
          {busy ? t("Installing...") : t("Install")}
        </button>
        {otherBusy && (
          <span className="text-muted" style={{ fontSize: "var(--size-micro)" }}>
            {t("Another install is in progress on this host.")}
          </span>
        )}
      </div>
      {activity?.error && (
        <div className="status-note status-note--error mono">{activity.error}</div>
      )}
      {activity && activity.log.length > 0 && (
        <pre ref={logRef} className="install-log mono sw-row__log">
          {activity.log.join("\n")}
        </pre>
      )}
      {sudoDialog}
    </div>
  );
}
