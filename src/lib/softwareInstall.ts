// Shared helpers for the software-install flow. Used by SoftwarePanel
// (per-row install/update) and by `<InlineInstallCta />` (the per-panel
// "tool not installed" CTA on Docker / Redis / MySQL / PG / SQLite).
//
// `describeInstallOutcome` lives here so the localized status messages
// stay in one place — when we add a new install status to the backend,
// only one switch needs to grow.

import { useEffect } from "react";

import * as cmd from "./commands";
import type { SoftwareInstallReport, SshParams } from "./commands";
import type { useI18n } from "../i18n/useI18n";
import {
  type SoftwareSnapshot,
  useSoftwareStore,
} from "../stores/useSoftwareStore";

type Translator = ReturnType<typeof useI18n>["t"];

export function describeInstallOutcome(
  report: SoftwareInstallReport,
  t: Translator,
): string {
  switch (report.status) {
    case "installed":
      return t("Done · {pm} {ver}", {
        pm: report.packageManager || "—",
        ver: report.installedVersion ?? "?",
      });
    case "unsupported-distro":
      return t(
        "This distro ({id}) is not in the auto-install list — please install manually.",
        { id: report.distroId || "?" },
      );
    case "sudo-requires-password":
      return t(
        "sudo requires a password — connect as root or configure passwordless sudo.",
      );
    case "package-manager-failed":
      return t("Install failed via {pm} (exit {code})", {
        pm: report.packageManager || "—",
        code: report.exitCode,
      });
    case "cancelled":
      return t("Cancelled");
    case "vendor-script-download-failed":
      return t("Failed to download installer script (exit {code})", {
        code: report.exitCode,
      });
    case "vendor-script-failed":
      return t("Vendor installer script failed (exit {code})", {
        code: report.exitCode,
      });
  }
}

/**
 * Ensure the software-probe snapshot for the host behind `swKey` is
 * populated. Idempotent: callers in multiple panels (DockerPanel +
 * SoftwarePanel, etc.) will share the same in-flight probe via
 * `inFlight` on the store.
 *
 * Returns the snapshot subscription the caller can read off of. Returns
 * `null` when there's no SSH context.
 */
export function useSoftwareSnapshot(
  swKey: string | null,
  sshParams: SshParams | null,
): SoftwareSnapshot | null {
  const snapshot = useSoftwareStore((s) => (swKey ? s.get(swKey) : null));

  useEffect(() => {
    if (!swKey || !sshParams) return;
    // Read live state — the store mutation from a sibling effect this
    // tick may not yet show up via the subscribed `snapshot` value.
    const snap = useSoftwareStore.getState().get(swKey);
    if (snap.lastFetchedAt > 0) return;
    if (snap.inFlight) return;
    const setProbeResult = useSoftwareStore.getState().setProbeResult;
    const setError = useSoftwareStore.getState().setError;
    const setInFlight = useSoftwareStore.getState().setInFlight;
    let cancelled = false;
    const promise = (async () => {
      try {
        const result = await cmd.softwareProbeRemote(sshParams);
        if (cancelled) return;
        setProbeResult(swKey, result.env, result.statuses);
      } catch (e) {
        if (cancelled) return;
        setError(swKey, String(e));
      } finally {
        setInFlight(swKey, null);
      }
    })();
    setInFlight(swKey, promise);
    return () => {
      cancelled = true;
    };
  }, [swKey, sshParams?.host, sshParams?.port, sshParams?.user]);

  return snapshot;
}
