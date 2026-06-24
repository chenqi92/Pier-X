import {
  Boxes,
  Copy,
  Network,
  Play,
  Plus,
  RefreshCw,
  RotateCw,
  Server,
  Square,
  Terminal,
  Trash2,
} from "lucide-react";
import { Fragment, useEffect, useRef, useState } from "react";
import * as cmd from "../lib/commands";
import type {
  NanoLinkCommandReport,
  NanoLinkServerAgent,
  NanoLinkServerSummary,
  NanoLinkStatus,
  SshParams,
} from "../lib/commands";
import { effectiveSshTarget, isSshTargetReady, type TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import PanelHeader from "../components/PanelHeader";
import IconButton from "../components/IconButton";
import Badge from "../components/Badge";
import StatusDot from "../components/StatusDot";
import Select from "../components/Select";
import SudoPasswordDialog from "../components/SudoPasswordDialog";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";
import { writeClipboardText } from "../lib/clipboard";
import { sudoKeyFor, useSudoStore } from "../stores/useSudoStore";
import { nanolinkKeyForTab, useNanoLinkStore } from "../stores/useNanoLinkStore";
import "../styles/nanolink-panel.css";

type Props = { tab: TabState; isActive?: boolean };

/** Default server gRPC port shown in the agent-install / add-server forms.
 *  The upstream silent-install examples use 39100 even though a standalone
 *  server's own default is 9200 — we surface 39100 as the prefill and let
 *  the user correct it. */
const DEFAULT_AGENT_PORT = "39100";

export default function NanoLinkPanel(props: Props) {
  const ready = useDeferredMount();
  return (
    <div className="panel-stage">
      {ready ? <NanoLinkPanelBody {...props} /> : <PanelSkeleton variant="rows" rows={5} />}
    </div>
  );
}

function NanoLinkPanelBody({ tab, isActive }: Props) {
  const { t } = useI18n();
  const fmtErr = (e: unknown) => localizeError(e, t);

  // ── SSH target + sudo, mirroring DockerPanel ───────────────────
  const sshTarget = effectiveSshTarget(tab);
  const canProbe = isSshTargetReady(sshTarget);

  const sudoStoreKey = sshTarget
    ? sudoKeyFor({
        host: sshTarget.host,
        port: sshTarget.port,
        user: sshTarget.user,
        authMode: sshTarget.authMode,
        password: sshTarget.password,
        keyPath: sshTarget.keyPath,
        savedConnectionIndex: sshTarget.savedConnectionIndex,
      })
    : "";
  const sudoPassword = useSudoStore((s) => (sudoStoreKey ? s.passwords[sudoStoreKey] ?? null : null));

  const sshArgs: SshParams = {
    host: sshTarget?.host ?? "",
    port: sshTarget?.port ?? 22,
    user: sshTarget?.user ?? "",
    authMode: sshTarget?.authMode ?? "password",
    password: sshTarget?.password ?? "",
    keyPath: sshTarget?.keyPath ?? "",
    savedConnectionIndex: sshTarget?.savedConnectionIndex ?? null,
    sudoPassword: sudoPassword ?? null,
  };

  // ── Status cache ───────────────────────────────────────────────
  const key = nanolinkKeyForTab(tab);
  const snapshot = useNanoLinkStore((s) => s.snapshots[key]);
  const refreshStore = useNanoLinkStore((s) => s.refresh);
  const status: NanoLinkStatus | null = snapshot?.status ?? null;
  const error = snapshot?.error ?? "";
  const loading = !!snapshot?.inFlight && !status;

  // Probe on mount + when the effective host changes. `key` encodes
  // user@host:port, so a nested-ssh repoint re-runs it. Hydrate the
  // sudo slot from the keychain like the other privileged panels.
  useEffect(() => {
    if (!canProbe) return;
    void useSudoStore.getState().hydrate(sshArgs);
    void refreshStore(key, () => cmd.nanolinkStatus(sshArgs));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, canProbe]);

  const reprobe = (force = true) => {
    if (!canProbe) return;
    void refreshStore(key, () => cmd.nanolinkStatus(sshArgs), force);
  };

  // ── Sudo-retry plumbing shared by all control actions ──────────
  const [sudoOpen, setSudoOpen] = useState(false);
  const [sudoBusy, setSudoBusy] = useState(false);
  const [sudoErr, setSudoErr] = useState("");
  const pendingActionRef = useRef<((args: SshParams) => Promise<NanoLinkCommandReport>) | null>(
    null,
  );
  const pendingOkRef = useRef<(() => void) | null>(null);
  const [actionMsg, setActionMsg] = useState("");

  /** Run a privileged NanoLink action; pop the sudo dialog + retry on
   *  `sudo-requires-password`. Returns the report so callers can show
   *  output. */
  const runControl = async (
    action: (args: SshParams) => Promise<NanoLinkCommandReport>,
    onOk?: () => void,
  ): Promise<NanoLinkCommandReport | null> => {
    setActionMsg("");
    try {
      const report = await action(sshArgs);
      if (report.status === "sudo-requires-password") {
        pendingActionRef.current = action;
        pendingOkRef.current = onOk ?? null;
        setSudoErr("");
        setSudoOpen(true);
        return report;
      }
      if (report.status === "ok") onOk?.();
      else setActionMsg(report.output || t("Command failed"));
      return report;
    } catch (e) {
      setActionMsg(fmtErr(e));
      return null;
    }
  };

  const onSudoSubmit = async (pw: string, remember: boolean) => {
    const action = pendingActionRef.current;
    if (!action) {
      setSudoOpen(false);
      return;
    }
    setSudoBusy(true);
    setSudoErr("");
    try {
      useSudoStore.getState().setPersistent(sshArgs, pw, remember);
      const report = await action({ ...sshArgs, sudoPassword: pw });
      if (report.status === "sudo-requires-password") {
        setSudoErr(t("Incorrect password, try again"));
        return;
      }
      setSudoOpen(false);
      if (report.status === "ok") pendingOkRef.current?.();
      else setActionMsg(report.output || t("Command failed"));
    } catch (e) {
      setSudoErr(fmtErr(e));
    } finally {
      setSudoBusy(false);
    }
  };

  // ── "both" role: which sub-view is showing ─────────────────────
  const [roleTab, setRoleTab] = useState<"server" | "client">("server");
  useEffect(() => {
    if (status?.role === "server") setRoleTab("server");
    else if (status?.role === "client") setRoleTab("client");
  }, [status?.role]);

  const role = status?.role ?? "none";
  const headerActions = (
    <IconButton
      variant="mini"
      title={t("Refresh")}
      onClick={() => reprobe(true)}
      aria-label={t("Refresh")}
    >
      <RefreshCw size={13} />
    </IconButton>
  );
  const headerMeta = status ? <RoleBadge role={role} t={t} /> : null;

  return (
    <div className="nl-panel">
      <PanelHeader icon={Network} title="NanoLink" meta={headerMeta} actions={headerActions} />

      {!canProbe && (
        <div className="nl-empty">
          <Network size={26} className="nl-empty__icon" />
          <p className="nl-empty__title">{t("Waiting for an SSH connection")}</p>
          <p className="nl-empty__sub">{t("NanoLink manages a remote host over SSH.")}</p>
        </div>
      )}

      {canProbe && loading && <PanelSkeleton variant="rows" rows={4} />}

      {canProbe && !loading && error && !status && (
        <div className="nl-error">
          <p>{error}</p>
          <button type="button" className="btn is-ghost is-compact" onClick={() => reprobe(true)}>
            {t("Retry")}
          </button>
        </div>
      )}

      {canProbe && status && (
        <div className="nl-body">
          {role === "none" && (
            <InstallView sshArgs={sshArgs} t={t} fmtErr={fmtErr} onInstalled={() => reprobe(true)} />
          )}

          {role === "both" && (
            <div className="nl-tabs">
              <button
                type="button"
                className={"nl-tab" + (roleTab === "server" ? " is-active" : "")}
                onClick={() => setRoleTab("server")}
              >
                <Server size={13} /> {t("Server")}
              </button>
              <button
                type="button"
                className={"nl-tab" + (roleTab === "client" ? " is-active" : "")}
                onClick={() => setRoleTab("client")}
              >
                <Boxes size={13} /> {t("Agent")}
              </button>
            </div>
          )}

          {(role === "server" || (role === "both" && roleTab === "server")) && (
            <ServerView sshArgs={sshArgs} status={status} t={t} fmtErr={fmtErr} isActive={!!isActive} />
          )}

          {(role === "client" || (role === "both" && roleTab === "client")) && (
            <ClientView
              sshArgs={sshArgs}
              status={status}
              t={t}
              fmtErr={fmtErr}
              runControl={runControl}
              onChanged={() => reprobe(true)}
              actionMsg={actionMsg}
            />
          )}
        </div>
      )}

      <SudoPasswordDialog
        open={sudoOpen}
        hostLabel={sshTarget ? `${sshTarget.user}@${sshTarget.host}` : ""}
        errorMessage={sudoErr || undefined}
        busy={sudoBusy}
        onSubmit={onSudoSubmit}
        onCancel={() => setSudoOpen(false)}
      />
    </div>
  );
}

function RoleBadge({ role, t }: { role: string; t: (k: string) => string }) {
  if (role === "server") return <Badge tone="info">{t("Server")}</Badge>;
  if (role === "client") return <Badge tone="info">{t("Agent")}</Badge>;
  if (role === "both") return <Badge tone="pos">{t("Server + Agent")}</Badge>;
  return <Badge tone="muted">{t("Not installed")}</Badge>;
}

// ── Install view (role = none) ────────────────────────────────────

function InstallView({
  sshArgs,
  t,
  fmtErr,
  onInstalled,
}: {
  sshArgs: SshParams;
  t: (k: string) => string;
  fmtErr: (e: unknown) => string;
  onInstalled: () => void;
}) {
  const [serverUrl, setServerUrl] = useState("");
  const [token, setToken] = useState("");
  const [permission, setPermission] = useState("0");
  const [useTls, setUseTls] = useState(true);
  const [hostname, setHostname] = useState("");
  const [busy, setBusy] = useState(false);
  const [output, setOutput] = useState("");
  const [msg, setMsg] = useState("");

  const canSubmit = serverUrl.trim() !== "" && token.trim() !== "" && !busy;

  const submit = async () => {
    setBusy(true);
    setMsg("");
    setOutput("");
    try {
      const report = await cmd.softwareProvisionApply({
        ...sshArgs,
        id: "nanolink",
        values: {
          server_url: serverUrl.trim(),
          token: token.trim(),
          permission,
          use_tls: useTls ? "true" : "false",
          hostname: hostname.trim(),
        },
      });
      setOutput(report.outputTail);
      if (report.status === "ok") {
        setMsg(t("NanoLink agent installed."));
        onInstalled();
      } else if (report.status === "sudo-requires-password") {
        setMsg(t("This host needs a sudo password — set it in another panel and retry."));
      } else {
        setMsg(t("Install failed."));
      }
    } catch (e) {
      setMsg(fmtErr(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="nl-install">
      <div className="nl-install__head">
        <Network size={22} className="nl-empty__icon" />
        <div>
          <p className="nl-empty__title">{t("NanoLink not installed")}</p>
          <p className="nl-empty__sub">
            {t("Install the agent to report this host's metrics to a NanoLink server.")}
          </p>
        </div>
      </div>

      <label className="nl-field">
        <span>{t("Server address")}</span>
        <input
          className="dlg-input"
          value={serverUrl}
          onChange={(e) => setServerUrl(e.currentTarget.value)}
          placeholder={`host:${DEFAULT_AGENT_PORT}`}
          spellCheck={false}
          autoCorrect="off"
        />
      </label>

      <label className="nl-field">
        <span>{t("Auth token")}</span>
        <input
          className="dlg-input"
          type="password"
          value={token}
          onChange={(e) => setToken(e.currentTarget.value)}
          autoComplete="new-password"
          spellCheck={false}
        />
      </label>

      <label className="nl-field">
        <span>{t("Permission level")}</span>
        <Select
          value={permission}
          onChange={setPermission}
          items={[
            { value: "0", label: `0 — ${t("Read only")}` },
            { value: "1", label: `1 — ${t("Basic write")}` },
            { value: "2", label: `2 — ${t("Service control")}` },
            { value: "3", label: `3 — ${t("System admin")}` },
          ]}
        />
      </label>

      <label className="nl-field nl-field--row">
        <input type="checkbox" checked={useTls} onChange={(e) => setUseTls(e.currentTarget.checked)} />
        <span>{t("Use TLS to the server")}</span>
      </label>

      <label className="nl-field">
        <span>{t("Report hostname (optional)")}</span>
        <input
          className="dlg-input"
          value={hostname}
          onChange={(e) => setHostname(e.currentTarget.value)}
          placeholder={t("system hostname")}
          spellCheck={false}
          autoCorrect="off"
        />
      </label>

      <div className="nl-install__actions">
        <button type="button" className="btn is-primary is-compact" disabled={!canSubmit} onClick={() => void submit()}>
          {busy ? t("Installing…") : t("Install agent")}
        </button>
        {msg && <span className="nl-msg">{msg}</span>}
      </div>

      {output && <pre className="nl-output">{output}</pre>}
    </div>
  );
}

// ── Client / agent view ───────────────────────────────────────────

function ClientView({
  sshArgs,
  status,
  t,
  fmtErr,
  runControl,
  onChanged,
  actionMsg,
}: {
  sshArgs: SshParams;
  status: NanoLinkStatus;
  t: (k: string) => string;
  fmtErr: (e: unknown) => string;
  runControl: (
    action: (args: SshParams) => Promise<NanoLinkCommandReport>,
    onOk?: () => void,
  ) => Promise<NanoLinkCommandReport | null>;
  onChanged: () => void;
  actionMsg: string;
}) {
  const [text, setText] = useState("");
  const [servers, setServers] = useState<cmd.NanoLinkAgentServer[]>([]);
  const [busy, setBusy] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);

  const reload = async () => {
    setBusy(true);
    try {
      const [txt, srv] = await Promise.all([
        cmd.nanolinkAgentStatus(sshArgs),
        cmd.nanolinkAgentServers(sshArgs).catch(() => [] as cmd.NanoLinkAgentServer[]),
      ]);
      setText(txt);
      setServers(srv);
    } catch (e) {
      setText(fmtErr(e));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sshArgs.host, sshArgs.user, sshArgs.port]);

  const service = (action: "start" | "stop" | "restart") =>
    void runControl(
      (args) => cmd.nanolinkAgentService({ ...args, action }),
      () => {
        onChanged();
        void reload();
      },
    );

  const removeServer = (host: string, port: number) =>
    void runControl(
      (args) => cmd.nanolinkAgentRemoveServer({ ...args, targetHost: host, targetPort: port }),
      () => {
        onChanged();
        void reload();
      },
    );

  const tlsLabel = (s: cmd.NanoLinkAgentServer) =>
    !s.tlsEnabled ? t("no TLS") : s.tlsVerify ? "TLS ✓" : t("TLS (no verify)");

  return (
    <div className="nl-client">
      <div className="nl-stat-row">
        <StatusDot tone={status.agentRunning ? "pos" : "off"} />
        <span className="nl-stat-label">
          {status.agentRunning ? t("Agent running") : t("Agent stopped")}
        </span>
        {status.agentVersion && <Badge tone="muted">v{status.agentVersion}</Badge>}
        <span className="nl-spacer" />
        <IconButton variant="mini" title={t("Start")} onClick={() => service("start")}>
          <Play size={13} />
        </IconButton>
        <IconButton variant="mini" title={t("Restart")} onClick={() => service("restart")}>
          <RotateCw size={13} />
        </IconButton>
        <IconButton variant="mini" title={t("Stop")} onClick={() => service("stop")}>
          <Square size={13} />
        </IconButton>
      </div>

      {status.agentConfigPath && (
        <div className="nl-kv">
          <span className="nl-kv__k">{t("Config")}</span>
          <span className="nl-kv__v nl-mono">{status.agentConfigPath}</span>
        </div>
      )}

      <div className="nl-section-head">
        <span>{t("Configured servers")}</span>
        <span className="nl-spacer" />
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={() => {
            setAddOpen((v) => !v);
            setRemoveOpen(false);
          }}
        >
          <Plus size={13} /> {t("Add server")}
        </button>
        {servers.length === 0 && (
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={() => {
              setRemoveOpen((v) => !v);
              setAddOpen(false);
            }}
          >
            <Trash2 size={13} /> {t("Remove server")}
          </button>
        )}
        <IconButton variant="mini" title={t("Refresh")} onClick={() => void reload()}>
          <RefreshCw size={13} />
        </IconButton>
      </div>

      {addOpen && (
        <AddServerForm
          t={t}
          onCancel={() => setAddOpen(false)}
          onSubmit={(form) =>
            void runControl(
              (args) =>
                cmd.nanolinkAgentAddServer({
                  ...args,
                  targetHost: form.host,
                  targetPort: form.port,
                  token: form.token,
                  permission: form.permission,
                  noTls: !form.useTls,
                }),
              () => {
                setAddOpen(false);
                onChanged();
                void reload();
              },
            )
          }
        />
      )}

      {servers.length > 0 ? (
        <div className="nl-srv-list">
          {servers.map((s) => (
            <div className="nl-srv-row" key={`${s.host}:${s.port}`}>
              <span className="nl-mono">
                {s.host}:{s.port}
              </span>
              <Badge tone="muted">{s.permissionName || `P${s.permission}`}</Badge>
              <span className="nl-srv-tls">{tlsLabel(s)}</span>
              <span className="nl-spacer" />
              <IconButton
                variant="mini"
                destructive
                title={t("Remove")}
                onClick={() => removeServer(s.host, s.port)}
              >
                <Trash2 size={13} />
              </IconButton>
            </div>
          ))}
        </div>
      ) : (
        removeOpen && (
          <RemoveServerForm
            t={t}
            onCancel={() => setRemoveOpen(false)}
            onSubmit={(form) =>
              void runControl(
                (args) =>
                  cmd.nanolinkAgentRemoveServer({
                    ...args,
                    targetHost: form.host,
                    targetPort: form.port,
                  }),
                () => {
                  setRemoveOpen(false);
                  onChanged();
                  void reload();
                },
              )
            }
          />
        )
      )}

      {actionMsg && <div className="nl-msg nl-msg--block">{actionMsg}</div>}

      <details className="nl-details">
        <summary>{t("Agent status output")}</summary>
        <pre className="nl-output nl-output--tall">
          {busy ? t("Loading…") : text || t("No status output.")}
        </pre>
      </details>
    </div>
  );
}

function AddServerForm({
  t,
  onCancel,
  onSubmit,
}: {
  t: (k: string) => string;
  onCancel: () => void;
  onSubmit: (form: { host: string; port: number; token: string; permission: number; useTls: boolean }) => void;
}) {
  const [host, setHost] = useState("");
  const [port, setPort] = useState(DEFAULT_AGENT_PORT);
  const [token, setToken] = useState("");
  const [permission, setPermission] = useState("0");
  const [useTls, setUseTls] = useState(true);
  const valid = host.trim() !== "" && token.trim() !== "" && Number(port) > 0;

  return (
    <div className="nl-addform">
      <label className="nl-field">
        <span>{t("Host")}</span>
        <input className="dlg-input" value={host} onChange={(e) => setHost(e.currentTarget.value)} spellCheck={false} autoCorrect="off" />
      </label>
      <label className="nl-field">
        <span>{t("Port")}</span>
        <input className="dlg-input" type="number" value={port} onChange={(e) => setPort(e.currentTarget.value)} />
      </label>
      <label className="nl-field">
        <span>{t("Auth token")}</span>
        <input className="dlg-input" type="password" value={token} onChange={(e) => setToken(e.currentTarget.value)} autoComplete="new-password" spellCheck={false} />
      </label>
      <label className="nl-field">
        <span>{t("Permission level")}</span>
        <Select
          value={permission}
          onChange={setPermission}
          items={[
            { value: "0", label: `0 — ${t("Read only")}` },
            { value: "1", label: `1 — ${t("Basic write")}` },
            { value: "2", label: `2 — ${t("Service control")}` },
            { value: "3", label: `3 — ${t("System admin")}` },
          ]}
        />
      </label>
      <label className="nl-field nl-field--row">
        <input type="checkbox" checked={useTls} onChange={(e) => setUseTls(e.currentTarget.checked)} />
        <span>{t("Use TLS to the server")}</span>
      </label>
      <div className="nl-install__actions">
        <button
          type="button"
          className="btn is-primary is-compact"
          disabled={!valid}
          onClick={() =>
            onSubmit({ host: host.trim(), port: Number(port), token: token.trim(), permission: Number(permission), useTls })
          }
        >
          {t("Add")}
        </button>
        <button type="button" className="btn is-ghost is-compact" onClick={onCancel}>
          {t("Cancel")}
        </button>
      </div>
    </div>
  );
}

function RemoveServerForm({
  t,
  onCancel,
  onSubmit,
}: {
  t: (k: string) => string;
  onCancel: () => void;
  onSubmit: (form: { host: string; port: number }) => void;
}) {
  const [host, setHost] = useState("");
  const [port, setPort] = useState(DEFAULT_AGENT_PORT);
  const valid = host.trim() !== "" && Number(port) > 0;

  return (
    <div className="nl-addform">
      <label className="nl-field">
        <span>{t("Host")}</span>
        <input className="dlg-input" value={host} onChange={(e) => setHost(e.currentTarget.value)} spellCheck={false} autoCorrect="off" />
      </label>
      <label className="nl-field">
        <span>{t("Port")}</span>
        <input className="dlg-input" type="number" value={port} onChange={(e) => setPort(e.currentTarget.value)} />
      </label>
      <div className="nl-install__actions">
        <button
          type="button"
          className="btn is-ghost is-compact"
          disabled={!valid}
          onClick={() => onSubmit({ host: host.trim(), port: Number(port) })}
        >
          <Trash2 size={13} /> {t("Remove")}
        </button>
        <button type="button" className="btn is-ghost is-compact" onClick={onCancel}>
          {t("Cancel")}
        </button>
      </div>
    </div>
  );
}

// ── Server / collector view ───────────────────────────────────────

function ServerView({
  sshArgs,
  status,
  t,
  fmtErr,
  isActive,
}: {
  sshArgs: SshParams;
  status: NanoLinkStatus;
  t: (k: string) => string;
  fmtErr: (e: unknown) => string;
  isActive: boolean;
}) {
  const port = status.httpPort || 8080;
  const busyRef = useRef(false);
  const [jwt, setJwt] = useState("");
  const [needLogin, setNeedLogin] = useState(status.authEnabled);
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [summary, setSummary] = useState<NanoLinkServerSummary | null>(null);
  const [agents, setAgents] = useState<NanoLinkServerAgent[]>([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [showAdd, setShowAdd] = useState(false);
  const [cmdAgentId, setCmdAgentId] = useState<string | null>(null);

  const load = async (bearer: string) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setErr("");
    try {
      const [s, a] = await Promise.all([
        cmd.nanolinkServerSummary({ ...sshArgs, nlPort: port, jwt: bearer }),
        cmd.nanolinkServerAgents({ ...sshArgs, nlPort: port, jwt: bearer }),
      ]);
      setSummary(s);
      setAgents(a);
    } catch (e) {
      setErr(fmtErr(e));
    } finally {
      setBusy(false);
      busyRef.current = false;
    }
  };

  // Light auto-poll while the tool is visible and authed — we replaced
  // the upstream WS stream with polling (PRODUCT-SPEC §5.15). Pauses when
  // another tool is active so hidden (kept-alive) panels don't keep
  // hitting the host; the busyRef guard drops ticks that overlap a fetch.
  useEffect(() => {
    if (!isActive || needLogin) return;
    const id = window.setInterval(() => void load(jwt), 5000);
    return () => window.clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isActive, needLogin, jwt]);

  // Auth disabled → load straight away. Auth enabled → wait for login.
  useEffect(() => {
    if (!status.authEnabled) void load("");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sshArgs.host, sshArgs.user, port, status.authEnabled]);

  const login = async () => {
    setBusy(true);
    setErr("");
    try {
      const token = await cmd.nanolinkServerLogin({
        ...sshArgs,
        nlPort: port,
        nlUsername: username,
        nlPassword: password,
      });
      setJwt(token);
      setNeedLogin(false);
      await load(token);
    } catch (e) {
      setErr(fmtErr(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="nl-server">
      <div className="nl-stat-row">
        <StatusDot tone={status.serverRunning ? "pos" : "off"} />
        <span className="nl-stat-label">
          {status.serverRunning ? t("Server running") : t("Server stopped")}
        </span>
        {status.serverVersion && <Badge tone="muted">v{status.serverVersion}</Badge>}
        <Badge tone="muted">:{port}</Badge>
        <span className="nl-spacer" />
        <IconButton variant="mini" title={t("Refresh")} onClick={() => void load(jwt)} disabled={busy || needLogin}>
          <RefreshCw size={13} />
        </IconButton>
      </div>

      {needLogin && (
        <div className="nl-login">
          <p className="nl-empty__sub">{t("This server requires login to read its dashboard.")}</p>
          <label className="nl-field">
            <span>{t("Username")}</span>
            <input className="dlg-input" value={username} onChange={(e) => setUsername(e.currentTarget.value)} spellCheck={false} autoCorrect="off" />
          </label>
          <label className="nl-field">
            <span>{t("Password")}</span>
            <input className="dlg-input" type="password" value={password} onChange={(e) => setPassword(e.currentTarget.value)} autoComplete="new-password" />
          </label>
          <div className="nl-install__actions">
            <button type="button" className="btn is-primary is-compact" disabled={busy || !password} onClick={() => void login()}>
              {busy ? t("Logging in…") : t("Log in")}
            </button>
          </div>
        </div>
      )}

      {err && <div className="nl-msg nl-msg--block">{err}</div>}

      {!needLogin && (
        <>
          <div className="nl-cards">
            <SummaryCard label={t("Agents")} value={summary ? String(summary.connectedAgents) : "—"} />
            <SummaryCard label={t("Avg CPU")} value={summary ? pct(summary.avgCpuPercent) : "—"} />
            <SummaryCard label={t("Memory")} value={summary ? pct(summary.memoryPercent) : "—"} />
            <SummaryCard label={t("Disk")} value={summary ? pct(summary.diskPercent) : "—"} />
          </div>

          <div className="nl-section-head">
            <span>{t("Add a monitored machine")}</span>
            <span className="nl-spacer" />
            <button type="button" className="btn is-ghost is-compact" onClick={() => setShowAdd((v) => !v)}>
              <Plus size={13} /> {showAdd ? t("Hide") : t("Generate config")}
            </button>
          </div>
          {showAdd && (
            <AddMachineCard
              sshArgs={sshArgs}
              port={port}
              jwt={jwt}
              defaultServerUrl={`${sshArgs.host}:${status.grpcPort || 39100}`}
              t={t}
              fmtErr={fmtErr}
            />
          )}

          <div className="nl-section-head">
            <span>{t("Connected agents")}</span>
          </div>

          {busy && !agents.length ? (
            <PanelSkeleton variant="rows" rows={3} />
          ) : agents.length === 0 ? (
            <div className="nl-empty__sub nl-pad">{t("No agents connected.")}</div>
          ) : (
            <table className="nl-table">
              <thead>
                <tr>
                  <th>{t("Host")}</th>
                  <th>{t("OS")}</th>
                  <th>{t("Version")}</th>
                  <th>{t("Perm")}</th>
                  <th>{t("Last seen")}</th>
                  <th />
                  <th />
                </tr>
              </thead>
              <tbody>
                {agents.map((a) => (
                  <Fragment key={a.id || a.hostname}>
                    <tr>
                      <td className="nl-mono">{a.hostname || a.id}</td>
                      <td>{[a.os, a.arch].filter(Boolean).join(" / ")}</td>
                      <td className="nl-mono">{a.version || "—"}</td>
                      <td>{a.permissionLevel}</td>
                      <td className="nl-mono">{shortStamp(a.lastHeartbeat)}</td>
                      <td>
                        <StatusDot tone={a.online ? "pos" : "off"} />
                      </td>
                      <td>
                        <IconButton
                          variant="mini"
                          title={t("Send command")}
                          active={cmdAgentId === a.id}
                          disabled={!a.id}
                          onClick={() => setCmdAgentId((cur) => (cur === a.id ? null : a.id))}
                        >
                          <Terminal size={13} />
                        </IconButton>
                      </td>
                    </tr>
                    {cmdAgentId === a.id && a.id && (
                      <tr className="nl-cmd-row">
                        <td colSpan={7}>
                          <CommandSender sshArgs={sshArgs} port={port} jwt={jwt} agentId={a.id} t={t} fmtErr={fmtErr} />
                        </td>
                      </tr>
                    )}
                  </Fragment>
                ))}
              </tbody>
            </table>
          )}
        </>
      )}
    </div>
  );
}

function SummaryCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="nl-card">
      <span className="nl-card__v">{value}</span>
      <span className="nl-card__l">{label}</span>
    </div>
  );
}

function pct(n: number): string {
  if (!Number.isFinite(n)) return "—";
  return `${Math.round(n)}%`;
}

// ── B: generate a config + install command for a new machine ──────

function AddMachineCard({
  sshArgs,
  port,
  jwt,
  defaultServerUrl,
  t,
  fmtErr,
}: {
  sshArgs: SshParams;
  port: number;
  jwt: string;
  defaultServerUrl: string;
  t: (k: string) => string;
  fmtErr: (e: unknown) => string;
}) {
  const [serverUrl, setServerUrl] = useState(defaultServerUrl);
  const [permission, setPermission] = useState("0");
  const [hostname, setHostname] = useState("");
  const [tlsVerify, setTlsVerify] = useState(false);
  const [busy, setBusy] = useState(false);
  const [res, setRes] = useState<cmd.NanoLinkGenerateConfig | null>(null);
  const [err, setErr] = useState("");

  const generate = async () => {
    setBusy(true);
    setErr("");
    setRes(null);
    try {
      const r = await cmd.nanolinkServerGenerateConfig({
        ...sshArgs,
        nlPort: port,
        jwt,
        serverUrl: serverUrl.trim(),
        token: "",
        permission: Number(permission),
        tlsVerify,
        hostname: hostname.trim(),
      });
      setRes(r);
    } catch (e) {
      setErr(fmtErr(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="nl-addform">
      <p className="nl-empty__sub">
        {t("Generates an agent config + one-line install command to run on the new host. Requires a server admin login.")}
      </p>
      <label className="nl-field">
        <span>{t("This server's address (host:gRPC port)")}</span>
        <input className="dlg-input" value={serverUrl} onChange={(e) => setServerUrl(e.currentTarget.value)} spellCheck={false} autoCorrect="off" />
      </label>
      <label className="nl-field">
        <span>{t("Permission level")}</span>
        <Select
          value={permission}
          onChange={setPermission}
          items={[
            { value: "0", label: `0 — ${t("Read only")}` },
            { value: "1", label: `1 — ${t("Basic write")}` },
            { value: "2", label: `2 — ${t("Service control")}` },
            { value: "3", label: `3 — ${t("System admin")}` },
          ]}
        />
      </label>
      <label className="nl-field">
        <span>{t("Report hostname (optional)")}</span>
        <input className="dlg-input" value={hostname} onChange={(e) => setHostname(e.currentTarget.value)} placeholder={t("system hostname")} spellCheck={false} autoCorrect="off" />
      </label>
      <label className="nl-field nl-field--row">
        <input type="checkbox" checked={tlsVerify} onChange={(e) => setTlsVerify(e.currentTarget.checked)} />
        <span>{t("Verify server TLS certificate")}</span>
      </label>
      <div className="nl-install__actions">
        <button type="button" className="btn is-primary is-compact" disabled={busy || !serverUrl.trim()} onClick={() => void generate()}>
          {busy ? t("Generating…") : t("Generate config")}
        </button>
      </div>

      {err && <div className="nl-msg nl-msg--block">{err}</div>}

      {res && (
        <div className="nl-gen">
          {res.generatedToken && (
            <div className="nl-kv">
              <span className="nl-kv__k">{t("Token")}</span>
              <span className="nl-kv__v nl-mono">{res.generatedToken}</span>
              <IconButton variant="mini" title={t("Copy")} onClick={() => void writeClipboardText(res.generatedToken)}>
                <Copy size={12} />
              </IconButton>
            </div>
          )}
          <CopyBlock label={t("Linux / macOS install")} text={res.installCommandUnix} t={t} />
          <CopyBlock label={t("Windows install")} text={res.installCommandWindows} t={t} />
          <details className="nl-details">
            <summary>{t("Agent config (YAML)")}</summary>
            <pre className="nl-output">{res.configYaml}</pre>
          </details>
        </div>
      )}
    </div>
  );
}

function CopyBlock({ label, text, t }: { label: string; text: string; t: (k: string) => string }) {
  return (
    <div className="nl-copyblock">
      <div className="nl-copyblock__head">
        <span className="nl-kv__k">{label}</span>
        <IconButton variant="mini" title={t("Copy")} onClick={() => void writeClipboardText(text)}>
          <Copy size={12} />
        </IconButton>
      </div>
      <pre className="nl-output">{text}</pre>
    </div>
  );
}

// ── C: dispatch a command to one connected agent ──────────────────

function CommandSender({
  sshArgs,
  port,
  jwt,
  agentId,
  t,
  fmtErr,
}: {
  sshArgs: SshParams;
  port: number;
  jwt: string;
  agentId: string;
  t: (k: string) => string;
  fmtErr: (e: unknown) => string;
}) {
  const [cmdType, setCmdType] = useState("PROCESS_LIST");
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState(false);
  const [out, setOut] = useState("");
  const [err, setErr] = useState("");

  const needsTarget = [
    "SERVICE_STATUS",
    "SERVICE_START",
    "SERVICE_STOP",
    "SERVICE_RESTART",
    "SERVICE_LOGS",
  ].includes(cmdType);

  const send = async () => {
    setBusy(true);
    setErr("");
    setOut("");
    try {
      const d = await cmd.nanolinkServerSendCommand({
        ...sshArgs,
        nlPort: port,
        jwt,
        agentId,
        cmdType,
        target: needsTarget ? target.trim() : "",
      });
      let done = "";
      for (let i = 0; i < 12; i += 1) {
        await new Promise((r) => setTimeout(r, 800));
        const cr = await cmd.nanolinkServerCommandResult({
          ...sshArgs,
          nlPort: port,
          jwt,
          agentId,
          commandId: d.commandId,
        });
        if (cr.status === "done") {
          done = cr.json;
          break;
        }
      }
      setOut(done || t("Timed out waiting for the agent's result."));
    } catch (e) {
      setErr(fmtErr(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="nl-cmd">
      <div className="nl-cmd__bar">
        <Select
          value={cmdType}
          onChange={setCmdType}
          compact
          items={[
            { value: "PROCESS_LIST", label: t("List processes") },
            { value: "SERVICE_LIST", label: t("List services") },
            { value: "HEALTH_CHECK", label: t("Health check") },
            { value: "AGENT_GET_VERSION", label: t("Agent version") },
            { value: "SERVICE_STATUS", label: t("Service status") },
            { value: "SERVICE_RESTART", label: t("Restart service") },
            { value: "SERVICE_START", label: t("Start service") },
            { value: "SERVICE_STOP", label: t("Stop service") },
          ]}
        />
        {needsTarget && (
          <input
            className="dlg-input"
            value={target}
            onChange={(e) => setTarget(e.currentTarget.value)}
            placeholder={t("service name")}
            spellCheck={false}
            autoCorrect="off"
          />
        )}
        <button
          type="button"
          className="btn is-primary is-compact"
          disabled={busy || (needsTarget && !target.trim())}
          onClick={() => void send()}
        >
          {busy ? t("Running…") : t("Send")}
        </button>
      </div>
      {err && <div className="nl-msg nl-msg--block">{err}</div>}
      {out && <pre className="nl-output nl-output--tall">{prettyJson(out)}</pre>}
    </div>
  );
}

function prettyJson(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

/** Trim an RFC3339 timestamp to `YYYY-MM-DD HH:MM:SS` (server-local), or
 *  pass it through when it doesn't match. */
function shortStamp(s: string): string {
  if (!s) return "—";
  const m = s.match(/(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2}:\d{2})/);
  return m ? `${m[1]} ${m[2]}` : s;
}
