import { Play, Search, Terminal } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import DbAddCredentialDialog from "../components/DbAddCredentialDialog";
import DbPasswordUpdateDialog from "../components/DbPasswordUpdateDialog";
import DbTunnelChip from "../components/DbTunnelChip";
import DismissibleNote from "../components/DismissibleNote";
import DbConnectSplash from "../components/db/DbConnectSplash";
import DbHeaderPicker, { type DbHeaderInstance } from "../components/db/DbHeaderPicker";
import RedisKeyDetail, { type RedisEdit } from "../components/db/RedisKeyDetail";
import RedisKeyList from "../components/db/RedisKeyList";
import type { DbSplashRowData } from "../components/db/DbSplashRow";
import { inferEnv } from "../components/db/dbTheme";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import * as cmd from "../lib/commands";
import { isDbAuthError } from "../lib/dbAuthErrors";
import { closeTunnelSlot, ensureTunnelSlot, syncTunnelState } from "../lib/sshTunnel";
import type {
  DetectedDbInstance,
  RedisBrowserState,
  RedisCommandResult,
  TabState,
} from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useDetectedServicesStore } from "../stores/useDetectedServicesStore";
import { useTabStore } from "../stores/useTabStore";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";

type Props = { tab: TabState };

export default function RedisPanel(props: Props) {
  const ready = useDeferredMount();
  const variant = props.tab.redisActiveCredentialId ? "grid" : "splash";
  return (
    <div className="panel-stage">
      {ready ? <RedisPanelBody {...props} /> : <PanelSkeleton variant={variant} rows={8} />}
    </div>
  );
}

function RedisPanelBody({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const updateTab = useTabStore((s) => s.updateTab);

  // ── Connection state ────────────────────────────────────────
  const [host, setHost] = useState(tab.redisHost);
  const [port, setPort] = useState(String(tab.redisPort));
  const [db, setDb] = useState(String(tab.redisDb));
  const [user, setUser] = useState(tab.redisUser);
  const [password, setPassword] = useState(tab.redisPassword);
  const [pattern, setPattern] = useState("*");
  const [keyName, setKeyName] = useState("");
  // Tree mode collapses keys by their `:` separator. Keep it
  // local — small enough that wiring it through useTabStore
  // would only buy persistence for a value the user rarely
  // changes during a session.
  // Tree mode defaults to true: most Redis keyspaces use `:` as a
  // namespace separator (`user:42:profile`, `cache:home:v1`), so the
  // grouped view is the more useful default. Users can flip back to
  // a flat list with the toolbar toggle.
  const [treeMode, setTreeMode] = useState(true);
  const [command, setCommand] = useState("PING");
  const [state, setState] = useState<RedisBrowserState | null>(null);
  const [busy, setBusy] = useState(false);
  const [loadMoreBusy, setLoadMoreBusy] = useState(false);
  const [error, setError] = useState("");
  const [cmdResult, setCmdResult] = useState<RedisCommandResult | null>(null);
  const [cmdBusy, setCmdBusy] = useState(false);
  const [cmdError, setCmdError] = useState("");
  const [cliOpen, setCliOpen] = useState(false);
  const [tunnelBusy, setTunnelBusy] = useState(false);
  const [tunnelError, setTunnelError] = useState("");
  const [addOpen, setAddOpen] = useState(false);
  const [adopting, setAdopting] = useState<DetectedDbInstance | null>(null);
  // See useDbCredentialFlow — bumped from `activateCredential` so the
  // auto-browse effect re-fires on a re-click of the already-active cred.
  const [browseTrigger, setBrowseTrigger] = useState(0);
  const [pwUpdateOpen, setPwUpdateOpen] = useState(false);
  const [activating, setActivating] = useState<string | null>(null);
  const [connectingStep, setConnectingStep] = useState<string | null>(null);
  // Generation counter — every effect run takes a fresh ticket; later runs
  // supersede earlier ones. Replaces the `autoBrowseAttempted` flag whose
  // stale `true` swallowed every retry click.
  const browseGenRef = useRef(0);
  const passwordInputRef = useRef<HTMLInputElement | null>(null);

  // ── Derived ────────────────────────────────────────────────
  const hasSsh = effectiveSshTarget(tab) !== null;
  const sshTarget = effectiveSshTarget(tab);
  const savedIndex = sshTarget?.savedConnectionIndex ?? null;
  const p = Number.parseInt(port, 10);
  const d = Number.parseInt(db, 10);
  const canBrowse = host.trim() && Number.isFinite(p) && p > 0 && Number.isFinite(d);
  const canUpdatePassword =
    !!error &&
    isDbAuthError("redis", error) &&
    !!tab.redisActiveCredentialId &&
    savedIndex !== null;

  // ── Splash data ────────────────────────────────────────────
  const connection = useConnectionStore((s) =>
    savedIndex !== null ? s.connections.find((c) => c.index === savedIndex) ?? null : null,
  );
  const refreshConnections = useConnectionStore((s) => s.refresh);
  const savedForKind = useMemo(() => {
    // Mirror useDbCredentialFlow dedup so the splash never shows two
    // identical Redis rows when the YAML pre-dates the upsert fix.
    const all = (connection?.databases ?? []).filter((c) => c.kind === "redis");
    const seen = new Map<string, (typeof all)[number]>();
    for (const cred of all) {
      const sigKey =
        cred.source.kind === "detected" && cred.source.signature
          ? `sig:${cred.source.signature}`
          : null;
      const tupleKey = `tup:${cred.host}:${cred.port}:${cred.user}`;
      const key = sigKey ?? tupleKey;
      const prev = seen.get(key);
      if (!prev || (cred.favorite && !prev.favorite)) {
        seen.set(key, cred);
      }
    }
    return Array.from(seen.values());
  }, [connection]);
  const instancesEntry = useDetectedServicesStore((s) => s.instancesByTab[tab.id]);
  const setPending = useDetectedServicesStore((s) => s.setDbInstancesPending);
  const setInstances = useDetectedServicesStore((s) => s.setDbInstances);
  const setDetectionError = useDetectedServicesStore((s) => s.setDbInstancesError);
  const detectedForKind = useMemo(() => {
    const all = instancesEntry?.instances ?? [];
    const adopted = new Set(
      savedForKind
        .map((c) => (c.source.kind === "detected" ? c.source.signature : null))
        .filter((s): s is string => !!s),
    );
    return all.filter((d) => d.kind === "redis" && !adopted.has(d.signature));
  }, [instancesEntry, savedForKind]);

  // ── Sync tab → local ──────────────────────────────────────
  useEffect(() => {
    setHost((current) => (current === tab.redisHost ? current : tab.redisHost));
  }, [tab.redisHost]);
  useEffect(() => {
    const next = String(tab.redisPort);
    setPort((current) => (current === next ? current : next));
  }, [tab.redisPort]);
  useEffect(() => {
    const next = String(tab.redisDb);
    setDb((current) => (current === next ? current : next));
  }, [tab.redisDb]);
  useEffect(() => {
    setUser((current) => (current === tab.redisUser ? current : tab.redisUser));
  }, [tab.redisUser]);
  useEffect(() => {
    setPassword((current) => (current === tab.redisPassword ? current : tab.redisPassword));
  }, [tab.redisPassword]);

  useEffect(() => {
    if (!hasSsh || !tab.redisTunnelId) return;
    let cancelled = false;
    void syncTunnelState(tab, "redis", updateTab).then((info) => {
      if (cancelled || !info?.alive) return;
      setTunnelError("");
    });
    return () => {
      cancelled = true;
    };
  }, [hasSsh, tab.id, tab.redisTunnelId, tab.redisTunnelPort, updateTab]);

  useEffect(() => {
    if (!hasSsh || !tab.redisActiveCredentialId || savedIndex === null) {
      setActivating(null);
      setConnectingStep(null);
      return;
    }
    if (state) {
      setActivating(null);
      setConnectingStep(null);
      return;
    }
    if (!tab.redisHost.trim()) {
      setActivating(null);
      setConnectingStep(null);
      return;
    }

    const myGen = ++browseGenRef.current;
    let cancelled = false;
    const isCurrent = () => !cancelled && browseGenRef.current === myGen;

    /** Mirror of `useDbCredentialFlow.surfaceMissingKeyring`: the
     *  backend says this cred *had* a saved password (Keyring/Direct
     *  variant tag) but the actual lookup came back empty — most
     *  often because the macOS keychain entry was wiped, the cred
     *  was saved on another machine, or the Direct-fallback's
     *  in-memory cache is gone after an app restart. Pop the password
     *  update dialog instead of just stuffing a small banner the
     *  user can miss. */
    const surfaceMissingKeyring = () => {
      setError(
        t("Saved password is missing from the keyring. Re-enter it to reconnect."),
      );
      setPwUpdateOpen(true);
      setTimeout(() => passwordInputRef.current?.focus(), 0);
    };

    void (async () => {
      try {
        let effectivePw = tab.redisPassword;
        if (!effectivePw) {
          if (isCurrent()) setConnectingStep(t("Resolving saved password…"));
          try {
            const resolved = await cmd.dbCredResolve(
              savedIndex,
              tab.redisActiveCredentialId!,
            );
            if (!isCurrent()) return;
            effectivePw = resolved.password ?? "";
            if (effectivePw) {
              updateTab(tab.id, { redisPassword: effectivePw });
              setPassword(effectivePw);
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
        await browse(undefined, effectivePw);
      } catch (e) {
        if (isCurrent()) setError(formatError(e));
      } finally {
        // Clear both spinner and step. Success → splash unmounts so
        // the cleared step is invisible anyway. Failure → the panel's
        // error banner explains; a stale "Opening SSH tunnel…" next
        // to it would just confuse the user.
        if (isCurrent()) {
          setActivating(null);
          setConnectingStep(null);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.redisActiveCredentialId, savedIndex, hasSsh, browseTrigger]);

  // ── Actions ───────────────────────────────────────────────
  async function ensureConnectionTarget(forceTunnel = false) {
    if (!hasSsh) return { host: host.trim(), port: p };
    const info = await ensureTunnelSlot({
      tab,
      slot: "redis",
      remoteHost: host.trim(),
      remotePort: p,
      updateTab,
      force: forceTunnel,
    });
    setTunnelError("");
    return { host: info.localHost, port: info.localPort };
  }

  async function closeTunnel() {
    if (!hasSsh || !tab.redisTunnelId) return;
    setTunnelBusy(true);
    setTunnelError("");
    try {
      await closeTunnelSlot(tab, "redis", updateTab);
    } catch (e) {
      setTunnelError(formatError(e));
    } finally {
      setTunnelBusy(false);
    }
  }

  async function rebuildTunnel() {
    if (!hasSsh || !canBrowse) return;
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

  async function disconnect() {
    setState(null);
    setError("");
    setCmdResult(null);
    setCmdError("");
    if (hasSsh && tab.redisTunnelId) {
      try {
        await closeTunnelSlot(tab, "redis", updateTab);
      } catch {
        /* best-effort */
      }
    }
  }

  async function handlePasswordUpdated() {
    if (savedIndex === null || !tab.redisActiveCredentialId) return;
    setError("");
    setConnectingStep(t("Re-resolving password and reconnecting…"));
    try {
      const resolved = await cmd.dbCredResolve(savedIndex, tab.redisActiveCredentialId);
      const pw = resolved.password ?? "";
      updateTab(tab.id, { redisPassword: pw });
      setPassword(pw);
      setConnectingStep(
        hasSsh ? t("Opening SSH tunnel and querying…") : t("Connecting…"),
      );
      await browse(undefined, pw);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setConnectingStep(null);
    }
  }

  async function browse(nextKey = keyName, passwordOverride?: string) {
    setBusy(true);
    setError("");
    const pw = passwordOverride !== undefined ? passwordOverride : password;
    try {
      const target = await ensureConnectionTarget();
      const s = await cmd.redisBrowse({
        host: target.host,
        port: target.port,
        db: d,
        pattern: pattern.trim() || "*",
        key: nextKey.trim() || null,
        username: user.trim() || null,
        password: pw || null,
        cursor: null,
      });
      setState(s);
      setKeyName(s.keyName);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  // Append the next SCAN page using `state.nextCursor` — keeps the
  // already-fetched keys in place so the user doesn't lose scroll
  // position on long matches.
  async function loadMore() {
    if (!state || state.nextCursor === "0") return;
    setLoadMoreBusy(true);
    setError("");
    try {
      const target = await ensureConnectionTarget();
      const s = await cmd.redisBrowse({
        host: target.host,
        port: target.port,
        db: d,
        pattern: state.pattern,
        key: state.keyName || null,
        username: user.trim() || null,
        password: password || null,
        cursor: state.nextCursor,
      });
      // Merge: keep prior keys, append new ones, dedupe by key name
      // (SCAN guarantees no duplicates within a cursor walk but a
      // pattern change races between two concurrent calls).
      const seen = new Set(state.keys.map((k) => k.key));
      const appended = s.keys.filter((k) => !seen.has(k.key));
      setState({
        ...s,
        keys: [...state.keys, ...appended],
      });
    } catch (e) {
      setError(formatError(e));
    } finally {
      setLoadMoreBusy(false);
    }
  }

  // ── Key edit actions ─────────────────────────────────────
  // Both rename and delete go through their own confirm-guarded
  // backend command (`RENAMENX` / `DEL`). The panel reloads the
  // browse on success so the key list updates in place.
  const [keyActionBusy, setKeyActionBusy] = useState(false);

  async function renameKey(currentKey: string, nextKey: string) {
    setKeyActionBusy(true);
    setError("");
    try {
      const target = await ensureConnectionTarget();
      const ok = await cmd.redisRenameKey({
        host: target.host,
        port: target.port,
        db: d,
        from: currentKey,
        to: nextKey,
        username: user.trim() || null,
        password: password || null,
      });
      if (!ok) {
        setError(t("Cannot rename — a key named {key} already exists.", { key: nextKey }));
        return;
      }
      setKeyName(nextKey);
      await browse(nextKey);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setKeyActionBusy(false);
    }
  }

  /** Wrap a string for the Redis CLI tokenizer used by `redis_execute`:
   *  double-quote the value and backslash-escape interior quotes /
   *  backslashes. Keeps simple text values safe; binary data with
   *  embedded newlines is not supported (the tokenizer treats whitespace
   *  as a token separator inside / outside quotes — there's no
   *  way to express a literal `\n` byte). */
  function quoteRedisArg(s: string): string {
    return `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
  }

  async function editKey(op: RedisEdit) {
    if (!keyName) return;
    setKeyActionBusy(true);
    setError("");
    try {
      const target = await ensureConnectionTarget();
      const k = quoteRedisArg(keyName);
      let command: string;
      switch (op.kind) {
        case "string-set":
          command = `SET ${k} ${quoteRedisArg(op.value)}`;
          break;
        case "hash-set":
          command = `HSET ${k} ${quoteRedisArg(op.field)} ${quoteRedisArg(op.value)}`;
          break;
        case "hash-del":
          command = `HDEL ${k} ${quoteRedisArg(op.field)}`;
          break;
        case "list-set":
          command = `LSET ${k} ${op.index} ${quoteRedisArg(op.value)}`;
          break;
        case "list-push":
          command = `${op.side === "L" ? "LPUSH" : "RPUSH"} ${k} ${quoteRedisArg(op.value)}`;
          break;
        case "list-rem":
          command = `LREM ${k} ${op.count} ${quoteRedisArg(op.value)}`;
          break;
        case "set-add":
          command = `SADD ${k} ${quoteRedisArg(op.member)}`;
          break;
        case "set-rem":
          command = `SREM ${k} ${quoteRedisArg(op.member)}`;
          break;
        case "zset-add":
          command = `ZADD ${k} ${op.score} ${quoteRedisArg(op.member)}`;
          break;
        case "zset-rem":
          command = `ZREM ${k} ${quoteRedisArg(op.member)}`;
          break;
        case "ttl-set":
          command = op.seconds === null ? `PERSIST ${k}` : `EXPIRE ${k} ${op.seconds}`;
          break;
      }
      await cmd.redisExecute({
        host: target.host,
        port: target.port,
        db: d,
        command,
        username: user.trim() || null,
        password: password || null,
      });
      // Reload the key detail (preview + length + TTL) after the
      // mutation. Pass the same key so the selection sticks.
      await browse(keyName);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setKeyActionBusy(false);
    }
  }

  async function deleteKey(key: string) {
    setKeyActionBusy(true);
    setError("");
    try {
      const target = await ensureConnectionTarget();
      const existed = await cmd.redisDeleteKey({
        host: target.host,
        port: target.port,
        db: d,
        key,
        username: user.trim() || null,
        password: password || null,
      });
      // Clear the active selection so the detail pane returns
      // to the empty state. Browse picks the next key from the
      // refreshed list automatically.
      setKeyName("");
      await browse("");
      if (!existed) {
        // Surface this as a notice rather than an error — the
        // server agreed with the action; the key just wasn't
        // there. Reusing the cmdError lane since there's no
        // dedicated notice slot in the Redis panel.
        setCmdError(t("Key {key} did not exist.", { key }));
      }
    } catch (e) {
      setError(formatError(e));
    } finally {
      setKeyActionBusy(false);
    }
  }

  async function runCommand() {
    setCmdBusy(true);
    setCmdError("");
    try {
      const target = await ensureConnectionTarget();
      const r = await cmd.redisExecute({
        host: target.host,
        port: target.port,
        db: d,
        command,
        username: user.trim() || null,
        password: password || null,
      });
      setCmdResult(r);
    } catch (e) {
      setCmdResult(null);
      setCmdError(formatError(e));
    } finally {
      setCmdBusy(false);
    }
  }

  async function refreshDetection() {
    if (!sshTarget) return;
    setPending(tab.id);
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
      setInstances(tab.id, {
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

  function activateCredential(credId: string) {
    const cred = savedForKind.find((c) => c.id === credId);
    if (!cred) return;
    // Clear every panel-scoped result of the previous instance — error
    // banners, the CLI response, the key-name focus — so switching to
    // a fresh cred never inherits stale context.
    setError("");
    setState(null);
    setCmdResult(null);
    setCmdError("");
    setKeyName("");
    setActivating(credId);
    setConnectingStep(t("Starting…"));
    setHost(cred.host);
    setPort(String(cred.port));
    setUser(cred.user);
    setPassword("");
    updateTab(tab.id, {
      redisActiveCredentialId: cred.id,
      redisHost: cred.host,
      redisPort: cred.port,
      redisUser: cred.user,
      redisPassword: "",
      redisTunnelId: null,
      redisTunnelPort: null,
    });
    // Re-clicking the already-active cred would otherwise be a no-op:
    // the identity-only effect deps wouldn't change. Bump the trigger
    // so the auto-browse re-fires.
    setBrowseTrigger((n) => n + 1);
  }

  // ── Splash rows ───────────────────────────────────────────
  const viaLabel = sshTarget ? `${sshTarget.user}@${sshTarget.host}` : t("direct · localhost");
  const viaKind: DbSplashRowData["via"]["kind"] = hasSsh ? "tunnel" : "direct";
  const probeTarget = sshTarget ? `${sshTarget.user}@${sshTarget.host}` : null;
  const probeState =
    instancesEntry?.status === "pending"
      ? "scanning"
      : instancesEntry?.status === "error"
        ? "error"
        : "idle";

  const savedRows: DbSplashRowData[] = savedForKind.map((cred) => ({
    id: cred.id,
    name: cred.label || cred.id,
    env: inferEnv(cred.label),
    engine: t("Redis"),
    addr: `${cred.host}:${cred.port}`,
    via: { kind: viaKind, label: viaLabel },
    user: cred.user,
    authHint: cred.hasPassword ? t("keyring") : undefined,
    stats: <span className="sep">—</span>,
    lastUsed: null,
    status: "unknown",
    tintVar: "var(--svc-redis)",
    connectLabel: t("Connect"),
    onConnect: () => activateCredential(cred.id),
    pending: activating === cred.id,
  }));

  const detectedRows: DbSplashRowData[] = detectedForKind.map((det) => ({
    id: det.signature,
    name: det.label,
    env: inferEnv(det.label),
    engine: det.version ? `Redis ${det.version}` : t("Redis"),
    addr: `${det.host}:${det.port}`,
    via: {
      kind: det.source === "docker" ? "local" : "remote",
      label: det.source === "docker" ? det.image || t("docker container") : det.processName || t("systemd unit"),
    },
    stats: <span className="sep">—</span>,
    lastUsed: null,
    status: "up",
    tintVar: "var(--svc-redis)",
    connectLabel: t("Adopt & connect"),
    onConnect: () => {
      setAdopting(det);
      setAddOpen(true);
    },
  }));

  // ── Connected-state derived ───────────────────────────────
  const currentCred = tab.redisActiveCredentialId
    ? savedForKind.find((c) => c.id === tab.redisActiveCredentialId)
    : undefined;

  const currentInstance: DbHeaderInstance = {
    id: currentCred?.id ?? "adhoc",
    name: currentCred?.label || host || t("Redis"),
    addr: `${host}:${port}`,
    via: hasSsh ? t("SSH tunnel") : t("direct"),
    status: state ? "up" : "unknown",
    sub: <>{`${host}:${port} · db ${db}`}</>,
  };

  const otherInstances: DbHeaderInstance[] = savedForKind
    .filter((c) => c.id !== tab.redisActiveCredentialId)
    .map((c) => ({
      id: c.id,
      name: c.label || c.id,
      addr: `${c.host}:${c.port}`,
      via: "",
      status: "unknown",
    }));

  // ── Banner / dialogs ──────────────────────────────────────
  const banner = error ? (
    <DismissibleNote variant="status" tone="error" onDismiss={() => setError("")}>
      <div>{error}</div>
      {canUpdatePassword && (
        <div className="button-row" style={{ marginTop: 6 }}>
          <button className="mini-button" onClick={() => setPwUpdateOpen(true)} type="button">
            {t("Update password")}
          </button>
        </div>
      )}
    </DismissibleNote>
  ) : tunnelError ? (
    <DismissibleNote variant="status" tone="error" onDismiss={() => setTunnelError("")}>
      {tunnelError}
    </DismissibleNote>
  ) : null;

  const dialogs = (
    <>
      <DbAddCredentialDialog
        open={addOpen}
        onClose={() => setAddOpen(false)}
        kind="redis"
        savedConnectionIndex={savedIndex}
        adopting={adopting}
        tab={tab}
        onSaved={(cred) => {
          setActivating(cred.id);
          setConnectingStep(t("Starting…"));
          setState(null);
          setHost(cred.host);
          setPort(String(cred.port));
          setUser(cred.user);
          updateTab(tab.id, {
            redisActiveCredentialId: cred.id,
            redisHost: cred.host,
            redisPort: cred.port,
            redisUser: cred.user,
            redisTunnelId: null,
            redisTunnelPort: null,
          });
          // Mirror activateCredential: even when the saved cred id
          // collides with whatever was active before, force the
          // auto-browse to fire.
          setBrowseTrigger((n) => n + 1);
          void refreshConnections();
        }}
      />
      {tab.redisActiveCredentialId && savedIndex !== null && (
        <DbPasswordUpdateDialog
          open={pwUpdateOpen}
          onClose={() => setPwUpdateOpen(false)}
          savedConnectionIndex={savedIndex}
          credentialId={tab.redisActiveCredentialId}
          credentialLabel={host.trim() || t("Redis")}
          onUpdated={() => void handlePasswordUpdated()}
          onTest={async (pw) => {
            let liveHost = host.trim() || "127.0.0.1";
            let livePort = p;
            let tunnelId: string | null = null;
            let via = "direct";
            try {
              if (sshTarget) {
                const info = await cmd.sshTunnelOpen({
                  host: sshTarget.host,
                  port: sshTarget.port,
                  user: sshTarget.user,
                  authMode: sshTarget.authMode,
                  password: sshTarget.password,
                  keyPath: sshTarget.keyPath,
                  remoteHost: liveHost,
                  remotePort: livePort,
                  localPort: null,
                  savedConnectionIndex: sshTarget.savedConnectionIndex,
                });
                liveHost = info.localHost;
                livePort = info.localPort;
                tunnelId = info.tunnelId;
                via = "ssh-tunnel";
              }
              try {
                await cmd.redisBrowse({
                  host: liveHost,
                  port: livePort,
                  db: Number.isFinite(d) ? d : 0,
                  pattern: "*",
                  key: null,
                  username: user.trim() || null,
                  password: pw.length > 0 ? pw : null,
                });
                return { ok: true, via };
              } finally {
                if (tunnelId) {
                  await cmd.sshTunnelClose(tunnelId).catch(() => {});
                }
              }
            } catch (e) {
              return { ok: false, msg: formatError(e) };
            }
          }}
        />
      )}
    </>
  );

  if (!state) {
    return (
      <>
        {banner && <div className="db-panel-banner">{banner}</div>}
        <DbConnectSplash
          kind="redis"
          probeTarget={probeTarget}
          probeState={probeState}
          onReprobe={sshTarget ? () => void refreshDetection() : undefined}
          detected={detectedRows}
          saved={savedRows}
          onAddManual={() => {
            setAdopting(null);
            setAddOpen(true);
          }}
          footerHint={
            // Suppress when an error banner is showing — keeps the
            // splash from contradicting itself.
            error || tunnelError
              ? null
              : connectingStep ?? (busy ? t("Connecting...") : null)
          }
          description={
            hasSsh
              ? undefined
              : t("No SSH session on this tab — add a connection manually to connect directly.")
          }
        />
        {dialogs}
      </>
    );
  }

  const headerStats = [
    {
      label: (
        <>
          <Zap /> {t("{count} keys", { count: state.keys.length })}
        </>
      ),
    },
    { label: state.usedMemory || "—" },
    { label: state.serverVersion || t("Redis") },
    { label: t("{ms} ms", { ms: state.rttMs }) },
  ];

  return (
    <>
      {banner && <div className="db-panel-banner db-panel-banner--snug">{banner}</div>}
      <div className="rds">
        <div className="rds-head">
          <DbHeaderPicker
            kind="redis"
            current={currentInstance}
            others={otherInstances}
            onSwitch={activateCredential}
            onAdd={() => {
              setAdopting(null);
              setAddOpen(true);
            }}
            onDisconnect={() => void disconnect()}
          />
          <div className="db2-stats">
            {headerStats.map((s, i) => (
              <span key={i} className="db2-stat">
                {s.label}
              </span>
            ))}
          </div>
          <span className="rds-spacer" />
          {hasSsh && (
            <DbTunnelChip
              localPort={tab.redisTunnelPort}
              busy={tunnelBusy}
              hasError={!!tunnelError}
              onRebuild={() => void rebuildTunnel()}
              onClose={() => void closeTunnel()}
            />
          )}
        </div>

        <div className="rds-scan">
          <label>{t("PATTERN")}</label>
          <input
            className="rds-input"
            value={pattern}
            onChange={(e) => setPattern(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void browse("");
            }}
          />
          <label>{t("DB")}</label>
          <select
            className="rds-input rds-input--narrow"
            value={db}
            onChange={(e) => {
              const next = e.currentTarget.value;
              setDb(next);
              const n = Number.parseInt(next, 10);
              if (Number.isFinite(n)) {
                updateTab(tab.id, { redisDb: n });
              }
            }}
            title={t("Redis DB index (0-15 by default)")}
          >
            {(() => {
              // Standard Redis ships 16 DBs by default. If this profile
              // is talking to a server tuned with `databases > 16` and
              // the user already saved a higher value, surface it as an
              // extra leading option so the dropdown round-trips losslessly.
              const current = Number.parseInt(db, 10);
              const extras = Number.isFinite(current) && (current < 0 || current > 15)
                ? [current]
                : [];
              return [...extras, ...Array.from({ length: 16 }, (_, i) => i)].map((i) => (
                <option key={i} value={String(i)}>
                  {i}
                </option>
              ));
            })()}
          </select>
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={!canBrowse || busy}
            onClick={() => void browse("")}
          >
            <Search size={10} /> {t("Scan")}
          </button>
          <button
            type="button"
            className={"btn is-ghost is-compact" + (treeMode ? " active" : "")}
            onClick={() => setTreeMode((v) => !v)}
            title={
              treeMode
                ? t("Showing colon-separated tree; click to flatten")
                : t("Group keys by colon namespaces")
            }
          >
            {treeMode ? t("Tree") : t("Flat")}
          </button>
          <span className="rds-spacer" />
          <button
            type="button"
            className={"btn is-ghost is-compact" + (cliOpen ? " active" : "")}
            onClick={() => setCliOpen((v) => !v)}
          >
            <Terminal size={10} /> {t("CLI")}
          </button>
        </div>

        {cliOpen && (
          <div className="rds-scan" style={{ background: "var(--panel)" }}>
            <label>{t("COMMAND")}</label>
            <input
              className="rds-input"
              style={{ flex: 1, width: "auto" }}
              value={command}
              onChange={(e) => setCommand(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void runCommand();
              }}
              placeholder="PING"
            />
            <button
              type="button"
              className="btn is-primary is-compact"
              disabled={cmdBusy}
              onClick={() => void runCommand()}
            >
              <Play size={10} /> {cmdBusy ? t("Running...") : t("Run")}
            </button>
          </div>
        )}

        {cliOpen && (cmdResult || cmdError) && (
          <div className="rds-cli-out">
            {cmdError ? (
              <span className="rds-cli-out-err">{cmdError}</span>
            ) : cmdResult ? (
              <>
                <div className="rds-cli-out-summary">
                  {cmdResult.summary}
                  <span className="sep"> · {cmdResult.elapsedMs} ms</span>
                </div>
                <pre className="rds-cli-out-lines">{cmdResult.lines.join("\n")}</pre>
              </>
            ) : null}
          </div>
        )}

        <div className="rds-split">
          <div className="rds-browser">
            <RedisKeyList
              keys={state.keys}
              selected={state.keyName || null}
              selectedKind={state.details?.kind ?? null}
              onSelect={(key) => {
                setKeyName(key);
                void browse(key);
              }}
              hasMore={state.nextCursor !== "0"}
              onLoadMore={() => void loadMore()}
              loadMoreBusy={loadMoreBusy}
              treeMode={treeMode}
            />
          </div>
          <div className="rds-detail">
            <RedisKeyDetail
              details={state.details}
              onRename={(from, to) => void renameKey(from, to)}
              onDelete={(key) => void deleteKey(key)}
              onEdit={editKey}
              actionBusy={keyActionBusy}
            />
          </div>
        </div>
      </div>
      {dialogs}
    </>
  );
}

// Minimal local Zap wrapper so the import stays flat; the icon is
// tiny enough (10px) that we can't easily share sizing with the
// larger service glyphs.
function Zap() {
  return (
    <svg
      width={10}
      height={10}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2.4}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
    </svg>
  );
}
