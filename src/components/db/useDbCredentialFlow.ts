import { useEffect, useMemo, useRef, useState, type MutableRefObject } from "react";

import { localizeError } from "../../i18n/localizeMessage";
import type { I18nValue } from "../../i18n/useI18n";
import * as cmd from "../../lib/commands";
import { isDbAuthError } from "../../lib/dbAuthErrors";
import { closeTunnelSlot, ensureTunnelSlot, syncTunnelState } from "../../lib/sshTunnel";
import type {
  DbCredential,
  DbKind,
  DetectedDbInstance,
  TabState,
} from "../../lib/types";
import { effectiveSshTarget } from "../../lib/types";
import { useConnectionStore } from "../../stores/useConnectionStore";
import { useDetectedServicesStore } from "../../stores/useDetectedServicesStore";
import { useTabStore } from "../../stores/useTabStore";

/** Kinds that persist a `DbCredential` + open an SSH tunnel. SQLite is
 *  excluded because it authenticates by file path, not host/port/user. */
export type CredentialKind = Extract<DbKind, "mysql" | "postgres" | "redis">;

/**
 * Per-kind mapping from the generic "DB connection fields" the hook
 * operates on back to the flat `mysql*` / `pg*` / `redis*` slots on
 * `TabState`. Every panel ships a `const` instance of this (see
 * `mysqlCredentialAdapter` in MySqlPanel.tsx).
 */
export type DbCredentialFieldAdapter = {
  /** Read the current host from the given tab. */
  readHost: (tab: TabState) => string;
  /** Read the current port. */
  readPort: (tab: TabState) => number;
  /** Read the current user. */
  readUser: (tab: TabState) => string;
  /** Read the current password (transient — lives in memory only). */
  readPassword: (tab: TabState) => string;
  /** Read the current active-credential id. `null` = manual. */
  readActiveCredId: (tab: TabState) => string | null;
  /** Read the open tunnel id. `null` when no tunnel. */
  readTunnelId: (tab: TabState) => string | null;
  /** Read the open tunnel's local port. `null` when no tunnel. */
  readTunnelPort: (tab: TabState) => number | null;

  /** Patch fired when activating a saved credential — writes host / port /
   *  user + clears password + sets `*ActiveCredentialId` + nulls the
   *  tunnel. The patch is merged with `updateTab`. */
  patchFromCred: (cred: DbCredential) => Partial<TabState>;
  /** Patch fired after the Add Credential dialog saves. Mirrors `patchFromCred`
   *  but does not clear password (the user typed one). */
  patchFromSaved: (cred: DbCredential) => Partial<TabState>;
  /** Patch fired when the auto-browse effect resolves a password from
   *  the keyring and needs to cache it in tab state. */
  patchPassword: (password: string) => Partial<TabState>;
  /** Patch fired on successful password rotation. Usually identical to
   *  `patchPassword` but exists as a separate slot so kind-specific
   *  side effects (resetting a cred id, etc.) can fire if needed. */
  patchPasswordAfterRotate: (password: string) => Partial<TabState>;
};

export type UseDbCredentialFlowOpts = {
  tab: TabState;
  kind: CredentialKind;
  tunnelSlot: "mysql" | "postgres" | "redis";
  adapter: DbCredentialFieldAdapter;

  /**
   * Panel-provided browse callback. The hook wires it into the
   * auto-browse effect and the password-rotation flow. `passwordOverride`
   * lets the hook pass a freshly resolved keyring password that hasn't
   * made it into React state yet.
   */
  browse: (passwordOverride?: string) => Promise<void>;

  /** Whether the panel currently has live browser state — gates the
   *  auto-browse effect so it only runs on cold opens. */
  hasLiveState: boolean;

  /**
   * Called when the hook clears internal state — on `activateCredential`
   * and `disconnect`. The panel uses it to blank panel-local state like
   * `rowDetail`, `queryResult`, `queryError`, `notice`, etc., fixing the
   * stale-across-switches leak.
   */
  onReset: () => void;

  /** Panel error sink — hook writes into it when the auto-browse effect
   *  hits a recoverable keyring failure and wants the panel to surface it. */
  setError: (msg: string) => void;

  /** Focus target for "please enter password manually". */
  passwordInputRef: MutableRefObject<HTMLInputElement | null>;

  t: I18nValue["t"];
};

export type DbCredentialFlow = {
  hasSsh: boolean;
  sshTarget: ReturnType<typeof effectiveSshTarget>;
  savedIndex: number | null;

  savedForKind: DbCredential[];
  detectedForKind: DetectedDbInstance[];

  /** Credential id the auto-browse effect is currently working on (the row
   *  the user just clicked Connect on). Cleared when the cycle resolves —
   *  success, error, or supersession by another click. */
  activating: string | null;
  /** Human-readable phase of the in-flight auto-browse: "resolving the
   *  saved password", "opening the SSH tunnel", "loading tables".
   *  `null` when nothing is in flight. Splash uses this so a click never
   *  looks like a no-op. */
  connectingStep: string | null;

  /** "scanning" while detection is pending, "error" on failure, else "idle". */
  probeState: "idle" | "scanning" | "error";
  /** `user@host` string shown in the splash probe line. `null` on local tabs. */
  probeTarget: string | null;
  /** Re-runs the `db_detect` command and refreshes the splash rows. */
  refreshDetection: () => Promise<void>;

  tunnelBusy: boolean;
  tunnelError: string;
  setTunnelError: (msg: string) => void;
  /** Opens (or returns the cached) tunnel. Falls back to direct on local tabs. */
  ensureConnectionTarget: (forceRebuild?: boolean) => Promise<{ host: string; port: number }>;
  rebuildTunnel: () => Promise<void>;
  closeTunnel: () => Promise<void>;

  addOpen: boolean;
  setAddOpen: (open: boolean) => void;
  adopting: DetectedDbInstance | null;
  setAdopting: (det: DetectedDbInstance | null) => void;
  pwUpdateOpen: boolean;
  setPwUpdateOpen: (open: boolean) => void;

  /** Switch to a saved credential. Writes the tab patch, nukes browser
   *  state via `onReset`, and lets the auto-browse effect pick it up. */
  activateCredential: (credId: string) => void;
  /** Fully disconnect: drop browser state, close the tunnel (best-effort). */
  disconnect: () => Promise<void>;
  /** Wire-up callback for the `DbAddCredentialDialog` `onSaved` prop. */
  handleCredentialAdded: (cred: DbCredential) => void;
  /** Resolves a newly-rotated password from the keyring and re-browses. */
  handlePasswordUpdated: () => Promise<void>;
  /** Returns true when the current error is an auth failure on a saved
   *  credential (i.e. the "Update password" affordance should be shown). */
  canUpdatePassword: (error: string) => boolean;
};

export function useDbCredentialFlow(opts: UseDbCredentialFlowOpts): DbCredentialFlow {
  const { tab, kind, tunnelSlot, adapter, browse, hasLiveState, onReset, setError, passwordInputRef, t } = opts;
  const updateTab = useTabStore((s) => s.updateTab);
  const formatError = (e: unknown) => localizeError(e, t);

  const sshTarget = effectiveSshTarget(tab);
  const hasSsh = sshTarget !== null;
  const savedIndex = sshTarget?.savedConnectionIndex ?? null;

  const [tunnelBusy, setTunnelBusy] = useState(false);
  const [tunnelError, setTunnelError] = useState("");
  const [addOpen, setAddOpen] = useState(false);
  const [adopting, setAdopting] = useState<DetectedDbInstance | null>(null);
  const [pwUpdateOpen, setPwUpdateOpen] = useState(false);
  const [activating, setActivating] = useState<string | null>(null);
  const [connectingStep, setConnectingStep] = useState<string | null>(null);
  // Generation counter — every effect run takes a fresh ticket, every later
  // run supersedes earlier ones. Replaces the previous `attempted` ref,
  // which kept a stale `true` across StrictMode double-mounts and silently
  // swallowed every later click.
  const browseGenRef = useRef(0);
  // Bumped by `activateCredential` so the auto-browse effect re-fires
  // even when the user re-clicks the credential that's already active
  // (e.g. retrying after the seeded auto-browse failed). Without this
  // the effect's identity-only deps would consider the click a no-op
  // and the user would see no response at all.
  const [browseTrigger, setBrowseTrigger] = useState(0);

  // ── Saved creds + detected instances ────────────────────────
  const connection = useConnectionStore((s) =>
    savedIndex !== null ? s.connections.find((c) => c.index === savedIndex) ?? null : null,
  );
  const refreshConnections = useConnectionStore((s) => s.refresh);
  const savedForKind = useMemo<DbCredential[]>(
    () => (connection?.databases ?? []).filter((c) => c.kind === kind),
    [connection, kind],
  );

  const instancesEntry = useDetectedServicesStore((s) => s.instancesByTab[tab.id]);
  const setDetectionPending = useDetectedServicesStore((s) => s.setDbInstancesPending);
  const setDetectionReady = useDetectedServicesStore((s) => s.setDbInstances);
  const setDetectionError = useDetectedServicesStore((s) => s.setDbInstancesError);

  const detectedForKind = useMemo<DetectedDbInstance[]>(() => {
    const all = instancesEntry?.instances ?? [];
    const adopted = new Set(
      savedForKind
        .map((c) => (c.source.kind === "detected" ? c.source.signature : null))
        .filter((s): s is string => !!s),
    );
    return all.filter((d) => d.kind === kind && !adopted.has(d.signature));
  }, [instancesEntry, kind, savedForKind]);

  const probeTarget = sshTarget ? `${sshTarget.user}@${sshTarget.host}` : null;
  const probeState: "idle" | "scanning" | "error" =
    instancesEntry?.status === "pending"
      ? "scanning"
      : instancesEntry?.status === "error"
        ? "error"
        : "idle";

  async function refreshDetection() {
    if (!sshTarget) return;
    setDetectionPending(tab.id);
    try {
      const report = await cmd.dbDetect({
        host: sshTarget.host,
        port: sshTarget.port,
        user: sshTarget.user,
        authMode: sshTarget.authMode,
        password: sshTarget.password,
        keyPath: sshTarget.keyPath,
        savedConnectionIndex: sshTarget.savedConnectionIndex,
      });
      setDetectionReady(tab.id, {
        instances: report.instances,
        mysqlCli: report.mysqlCli,
        psqlCli: report.psqlCli,
        redisCli: report.redisCli,
        sqliteCli: report.sqliteCli,
      });
    } catch {
      setDetectionError(tab.id);
    }
  }

  // ── Tunnel helpers ──────────────────────────────────────────
  const tabHost = adapter.readHost(tab);
  const tabPort = adapter.readPort(tab);
  const tabTunnelId = adapter.readTunnelId(tab);
  const tabTunnelPort = adapter.readTunnelPort(tab);

  useEffect(() => {
    if (!hasSsh || !tabTunnelId) return;
    let cancelled = false;
    void syncTunnelState(tab, tunnelSlot, updateTab).then((info) => {
      if (cancelled || !info?.alive) return;
      setTunnelError("");
    });
    return () => {
      cancelled = true;
    };
  }, [hasSsh, tab.id, tabTunnelId, tabTunnelPort, tunnelSlot, updateTab]);

  async function ensureConnectionTarget(forceRebuild = false) {
    if (!hasSsh) {
      return { host: tabHost.trim(), port: tabPort };
    }
    const info = await ensureTunnelSlot({
      tab,
      slot: tunnelSlot,
      remoteHost: tabHost.trim(),
      remotePort: tabPort,
      updateTab,
      force: forceRebuild,
    });
    setTunnelError("");
    return { host: info.localHost, port: info.localPort };
  }

  async function rebuildTunnel() {
    if (!hasSsh) return;
    setTunnelBusy(true);
    setTunnelError("");
    try {
      await ensureConnectionTarget(true);
    } catch (e) {
      setTunnelError(formatError(e));
    } finally {
      setTunnelBusy(false);
    }
  }

  async function closeTunnel() {
    if (!hasSsh || !tabTunnelId) return;
    setTunnelBusy(true);
    setTunnelError("");
    try {
      await closeTunnelSlot(tab, tunnelSlot, updateTab);
    } catch (e) {
      setTunnelError(formatError(e));
    } finally {
      setTunnelBusy(false);
    }
  }

  // ── Auto-browse on saved-cred seeded tab open ───────────────
  //
  // Resolve the password from the keyring the first time a tab opens
  // with a saved `*ActiveCredentialId`, then call the panel-provided
  // `browse(pw)` directly. Passing the password explicitly is
  // deliberate: a preceding `setPassword(...)` is queued and `browse`'s
  // closure would otherwise auth with the pre-activation empty string,
  // surfacing a misleading "Access denied" even though the keyring is fine.
  const activeCredId = adapter.readActiveCredId(tab);
  const tabPassword = adapter.readPassword(tab);
  useEffect(() => {
    if (!hasSsh || !activeCredId || savedIndex === null) {
      // No work to do — drop any stale click-driven UI state so a
      // pending spinner doesn't hang forever.
      setActivating(null);
      setConnectingStep(null);
      return;
    }
    if (hasLiveState) {
      // Already connected — nothing for the splash to do.
      setActivating(null);
      setConnectingStep(null);
      return;
    }
    if (!tabHost.trim()) {
      setActivating(null);
      setConnectingStep(null);
      return;
    }

    const myGen = ++browseGenRef.current;
    let cancelled = false;
    const isCurrent = () => !cancelled && browseGenRef.current === myGen;

    /** Common recovery for "the keyring lookup didn't return a usable
     *  password but the YAML insists one was saved" — auto-pop the
     *  password-update dialog so the user has an obvious next action.
     *  The previous behaviour (small dismissible banner only) was easy
     *  to miss, especially when the message wasn't translated. */
    const surfaceMissingKeyring = () => {
      setError(t("Saved password is missing from the keyring. Re-enter it to reconnect."));
      setPwUpdateOpen(true);
      setTimeout(() => passwordInputRef.current?.focus(), 0);
    };

    void (async () => {
      try {
        let effectivePw = tabPassword;
        if (!effectivePw) {
          if (isCurrent()) setConnectingStep(t("Resolving saved password…"));
          try {
            const resolved = await cmd.dbCredResolve(savedIndex, activeCredId);
            if (!isCurrent()) return;
            effectivePw = resolved.password ?? "";
            if (effectivePw) {
              updateTab(tab.id, adapter.patchPassword(effectivePw));
            } else if (resolved.credential.hasPassword) {
              surfaceMissingKeyring();
              return;
            }
          } catch {
            if (!isCurrent()) return;
            surfaceMissingKeyring();
            return;
          }
        }
        if (!isCurrent()) return;
        setConnectingStep(
          hasSsh ? t("Opening SSH tunnel and querying…") : t("Connecting…"),
        );
        await browse(effectivePw);
      } catch (e) {
        if (isCurrent()) setError(formatError(e));
      } finally {
        // Clear both spinner and step. Success → splash unmounts so
        // the cleared step is invisible anyway. Failure → the panel's
        // own error banner (or `flow.tunnelError`) carries the real
        // diagnosis; keeping a stale "Opening SSH tunnel…" alongside
        // the error reads as a contradiction.
        if (isCurrent()) {
          setActivating(null);
          setConnectingStep(null);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
    // Intentionally narrow deps — we only want to kick off on
    // identity changes plus the explicit `browseTrigger` bump from
    // `activateCredential`, not on every `browse` closure rebuild.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeCredId, savedIndex, hasSsh, browseTrigger]);

  // ── Credential actions ──────────────────────────────────────
  function activateCredential(credId: string) {
    const cred = savedForKind.find((c) => c.id === credId);
    if (!cred) return;
    setTunnelError("");
    setActivating(credId);
    setConnectingStep(t("Starting…"));
    updateTab(tab.id, adapter.patchFromCred(cred));
    onReset();
    // Force the auto-browse effect to re-run even if the activated
    // credential id matches the one already on `tab` (the splash's
    // "Connect" button on a seeded-but-failed cred would otherwise be
    // a no-op).
    setBrowseTrigger((n) => n + 1);
  }

  async function disconnect() {
    onReset();
    if (hasSsh && tabTunnelId) {
      try {
        await closeTunnelSlot(tab, tunnelSlot, updateTab);
      } catch {
        /* best-effort — tab-close / reconnect will clean up */
      }
    }
  }

  function handleCredentialAdded(cred: DbCredential) {
    setActivating(cred.id);
    setConnectingStep(t("Starting…"));
    updateTab(tab.id, adapter.patchFromSaved(cred));
    onReset();
    // Mirror activateCredential: even when the saved cred id collides
    // with whatever was active before, force the auto-browse to fire.
    setBrowseTrigger((n) => n + 1);
    void refreshConnections();
  }

  async function handlePasswordUpdated() {
    if (savedIndex === null || !activeCredId) return;
    setError("");
    setConnectingStep(t("Re-resolving password and reconnecting…"));
    try {
      const resolved = await cmd.dbCredResolve(savedIndex, activeCredId);
      const pw = resolved.password ?? "";
      updateTab(tab.id, adapter.patchPasswordAfterRotate(pw));
      setConnectingStep(
        hasSsh ? t("Opening SSH tunnel and querying…") : t("Connecting…"),
      );
      await browse(pw);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setConnectingStep(null);
    }
  }

  function canUpdatePassword(error: string): boolean {
    return !!error && isDbAuthError(kind, error) && !!activeCredId && savedIndex !== null;
  }

  return {
    hasSsh,
    sshTarget,
    savedIndex,
    savedForKind,
    detectedForKind,
    activating,
    connectingStep,
    probeState,
    probeTarget,
    refreshDetection,
    tunnelBusy,
    tunnelError,
    setTunnelError,
    ensureConnectionTarget,
    rebuildTunnel,
    closeTunnel,
    addOpen,
    setAddOpen,
    adopting,
    setAdopting,
    pwUpdateOpen,
    setPwUpdateOpen,
    activateCredential,
    disconnect,
    handleCredentialAdded,
    handlePasswordUpdated,
    canUpdatePassword,
  };
}
