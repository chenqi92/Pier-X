import { useEffect, useRef, useState, type ReactNode } from "react";

import SudoPasswordDialog from "../components/SudoPasswordDialog";
import { sshElevationPreflight } from "./commands";
import { useI18n } from "../i18n/useI18n";
import { sudoKeyFor, useSudoStore } from "../stores/useSudoStore";
import { effectiveShellUser, effectiveSshTarget, type TabState } from "./types";

/** Best-effort "this failed because the SSH login user lacks privilege"
 *  detector — mirrors the backend `sudo::is_permission_denied` patterns.
 *  Shared by every right-side panel so they all decide to prompt for a
 *  sudo password (and elevate to the terminal's effective user) on the
 *  same signals. */
export function looksLikePermissionDenied(message: string): boolean {
  const m = message.toLowerCase();
  return (
    m.includes("permission denied") ||
    m.includes("eacces") ||
    m.includes("eperm") ||
    m.includes("operation not permitted") ||
    m.includes("is not in the sudoers file") ||
    m.includes("a password is required") ||
    m.includes("must be run from a terminal") ||
    m.includes("you must have a tty")
  );
}

/** Extra args every lane-aware backend command accepts so it can follow
 *  the terminal's effective identity. `sudoPassword` arms `sudo`;
 *  `effectiveUser` is the terminal's current shell user (`null` when it
 *  equals the SSH login user — i.e. no elevation needed), which the
 *  backend maps to plain `sudo` (root) or `sudo -u <user>`. */
export type ElevationArgs = {
  sudoPassword: string | null;
  effectiveUser: string | null;
};

export type SudoElevation = {
  /** Read the current elevation args **imperatively** (fresh from the
   *  store). Spread into every lane-aware command call — both the first
   *  attempt and the post-prompt retry — so the retry picks up the
   *  password that just landed without a stale closure. */
  getElevationArgs: () => ElevationArgs;
  /** Call from a catch block with the raw error string. When it looks
   *  like a privilege error, opens the sudo dialog and (once the
   *  password lands + React re-renders) runs `retry`. Returns `true`
   *  when it handled the error (so the caller can skip its own banner).
   *  Mutations that don't need a retry can omit `retry`; the cached
   *  password makes the user's next action succeed. */
  handlePermissionDenied: (raw: string, retry?: () => void) => boolean;
  /** Render this once in the panel's JSX. */
  dialog: ReactNode;
};

/**
 * Unified sudo-elevation for right-side panels. Centralizes:
 *  - reading/hydrating the per-host sudo password (`useSudoStore`,
 *    keyed by `user@host:port` — shared across panels so one prompt
 *    serves all of them),
 *  - deriving the elevation target from the terminal's effective user
 *    (`tab.currentShellUser`), so a panel mirrors `su root` / `sudo -i`
 *    / `su - deploy`,
 *  - the permission-denied → prompt → retry flow.
 *
 * Panels spread `getElevationArgs()` into their command calls and route
 * catch blocks through `handlePermissionDenied`, then render `dialog`.
 */
export function useSudoElevation(tab: TabState | null | undefined): SudoElevation {
  const { t } = useI18n();
  const sshTarget = tab ? effectiveSshTarget(tab) : null;
  const params = sshTarget
    ? {
        host: sshTarget.host,
        port: sshTarget.port,
        user: sshTarget.user,
        authMode: sshTarget.authMode,
        password: sshTarget.password,
        keyPath: sshTarget.keyPath,
        savedConnectionIndex: sshTarget.savedConnectionIndex,
      }
    : null;
  const storeKey = params ? sudoKeyFor(params) : "";
  // Reactive subscription drives the deferred retry + the dialog's
  // "password was rejected" hint. Reads at call time go through
  // `getElevationArgs` (imperative) so retries never use a stale value.
  const sudoPassword = useSudoStore((s) => (storeKey ? s.passwords[storeKey] ?? null : null));

  const loginUser = sshTarget?.user ?? "";
  const shellUser = tab ? effectiveShellUser(tab, sshTarget) : "";
  const effectiveUser = shellUser && shellUser !== loginUser ? shellUser : null;

  const [prompt, setPrompt] = useState<{ hostLabel: string; errorMessage?: string } | null>(null);
  const retryRef = useRef<(() => void) | null>(null);

  // Lift any persisted password from the OS keychain into the L1 cache
  // on host change, so a returning session elevates without re-prompting.
  useEffect(() => {
    if (params) void useSudoStore.getState().hydrate(params);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [storeKey]);

  // Deferred retry: once the password lands in the store (and React
  // re-renders), run the operation that hit permission-denied. The
  // retry re-reads the password via `getElevationArgs`, so it captures
  // the fresh value rather than the rendered-time null.
  useEffect(() => {
    if (!sudoPassword) return;
    const retry = retryRef.current;
    if (!retry) return;
    retryRef.current = null;
    retry();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sudoPassword]);

  function getElevationArgs(): ElevationArgs {
    return {
      sudoPassword: params ? useSudoStore.getState().get(params) : null,
      effectiveUser,
    };
  }

  function handlePermissionDenied(raw: string, retry?: () => void): boolean {
    if (!params || !looksLikePermissionDenied(raw)) return false;
    if (retry) retryRef.current = retry;
    setPrompt({
      hostLabel: `${shellUser || loginUser}@${sshTarget!.host}`,
      errorMessage: useSudoStore.getState().get(params)
        ? t("Saved sudo password was rejected — please re-enter.")
        : undefined,
    });
    return true;
  }

  const dialog = (
    <SudoPasswordDialog
      open={prompt !== null}
      hostLabel={prompt?.hostLabel ?? ""}
      errorMessage={prompt?.errorMessage}
      // Default to session-only (no keychain) — this prompt exists to
      // *follow the terminal's* elevation, which is itself ephemeral;
      // the user shouldn't have to opt out of persisting it. They can
      // still tick "remember" for a host they elevate often.
      defaultRemember={false}
      onSubmit={(password, remember) => {
        setPrompt(null);
        if (params) void useSudoStore.getState().setPersistent(params, password, remember);
      }}
      onCancel={() => {
        retryRef.current = null;
        setPrompt(null);
      }}
      onTest={
        params
          ? (pw) =>
              sshElevationPreflight({
                host: params.host,
                port: params.port,
                user: params.user,
                authMode: params.authMode,
                password: params.password,
                keyPath: params.keyPath,
                savedConnectionIndex: params.savedConnectionIndex,
                sudoPassword: pw,
                effectiveUser,
              })
          : undefined
      }
    />
  );

  return { getElevationArgs, handlePermissionDenied, dialog };
}
