import type { ReactNode } from "react";
import { useRef, useState } from "react";

import SudoPasswordDialog from "../components/SudoPasswordDialog";
import { useI18n } from "../i18n/useI18n";
import { useSudoStore } from "../stores/useSudoStore";
import type { SshParams } from "./commands";

type SudoPromptState = {
  hostLabel: string;
  errorMessage?: string;
  resolve: (result: { password: string; remember: boolean } | null) => void;
} | null;

/**
 * Shared sudo-password retry flow for backend calls that may come back
 * with a `sudo-requires-password` outcome — software install / update /
 * uninstall / service action, the inline install CTA, and any future
 * right-side action that runs a root command over the host's SSH
 * session.
 *
 * The first attempt uses whatever password is cached for the host (or
 * `null`, which keeps the legacy non-interactive `sudo -n` path). A
 * `sudo-requires-password` report pops the modal {@link SudoPasswordDialog},
 * caches the user's input (optionally to the OS keychain via
 * `useSudoStore`), and retries — until the report is anything else, the
 * user dismisses the dialog, or a 4-attempt cap is hit so a stuck dialog
 * can't spin forever.
 *
 * Returns `withSudoRetry` to wrap a single attempt, plus `sudoDialog`,
 * which the caller must render somewhere in its tree.
 *
 * @param sshParams Resolved SSH target, or `null` while the host isn't
 *   ready (callers still call the hook unconditionally — Rules of Hooks).
 * @param hostLabel Subtitle shown in the dialog, e.g. `chenqi@10.0.0.3`.
 */
export function useSudoRetry(
  sshParams: SshParams | null,
  hostLabel: string,
): {
  withSudoRetry: <R extends { status: string }>(
    fn: (sudoPassword: string | null) => Promise<R>,
  ) => Promise<R>;
  sudoDialog: ReactNode;
} {
  const { t } = useI18n();
  const [sudoPrompt, setSudoPrompt] = useState<SudoPromptState>(null);
  const sudoPromptRef = useRef(sudoPrompt);
  sudoPromptRef.current = sudoPrompt;

  function requestSudoPassword(
    errorMessage?: string,
  ): Promise<{ password: string; remember: boolean } | null> {
    return new Promise((resolve) => {
      // Defensive: if a prior prompt is somehow still open, close it so
      // its awaiter sees `null` and bails before we replace the state.
      sudoPromptRef.current?.resolve(null);
      setSudoPrompt({ hostLabel, errorMessage, resolve });
    });
  }

  async function withSudoRetry<R extends { status: string }>(
    fn: (sudoPassword: string | null) => Promise<R>,
  ): Promise<R> {
    if (!sshParams) {
      // Unreachable in practice — callers gate on sshParams — but keep
      // the path total so the generic return type holds.
      return fn(null);
    }
    const cached = useSudoStore.getState().get(sshParams);
    let password: string | null = cached;
    let cachedRejectedThisRun = false;
    let lastReport: R | null = null;
    // 1 initial + 3 retries. The user can re-trigger from the row.
    for (let attempt = 0; attempt < 4; attempt++) {
      const report = await fn(password);
      lastReport = report;
      if (report.status !== "sudo-requires-password") return report;
      // First failure with a cached password → that cached value is
      // wrong; clear it and ask fresh. After that it's straight
      // "wrong password, try again" until the loop bottoms out.
      let errorMessage: string | undefined;
      if (cached && password === cached && !cachedRejectedThisRun) {
        useSudoStore.getState().clear(sshParams);
        cachedRejectedThisRun = true;
        errorMessage = t("Saved sudo password was rejected — please re-enter.");
      } else if (attempt > 0) {
        errorMessage = t("Wrong password — please try again.");
      }
      const fresh = await requestSudoPassword(errorMessage);
      if (fresh === null) return report;
      password = fresh.password;
      void useSudoStore
        .getState()
        .setPersistent(sshParams, fresh.password, fresh.remember);
    }
    return lastReport as R;
  }

  const sudoDialog = (
    <SudoPasswordDialog
      open={sudoPrompt !== null}
      hostLabel={sudoPrompt?.hostLabel ?? ""}
      errorMessage={sudoPrompt?.errorMessage}
      onSubmit={(password, remember) => {
        const cur = sudoPromptRef.current;
        setSudoPrompt(null);
        cur?.resolve({ password, remember });
      }}
      onCancel={() => {
        const cur = sudoPromptRef.current;
        setSudoPrompt(null);
        cur?.resolve(null);
      }}
    />
  );

  return { withSudoRetry, sudoDialog };
}
