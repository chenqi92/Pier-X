import { Zap } from "lucide-react";
import { useEffect, useState } from "react";
import * as cmd from "../lib/commands";
import { quoteCommandArg } from "../lib/commands";
import { closeTunnelSlot, ensureTunnelSlot, syncTunnelState } from "../lib/sshTunnel";
import type { RedisBrowserState, RedisCommandResult, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import DbConnRow from "../components/DbConnRow";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";
import { useTabStore } from "../stores/useTabStore";

type Props = { tab: TabState };

export default function RedisPanel({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const updateTab = useTabStore((s) => s.updateTab);
  const [host, setHost] = useState(tab.redisHost);
  const [port, setPort] = useState(String(tab.redisPort));
  const [db, setDb] = useState(String(tab.redisDb));
  const [pattern, setPattern] = useState("*");
  const [keyName, setKeyName] = useState("");
  const [command, setCommand] = useState("PING");
  const [state, setState] = useState<RedisBrowserState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [cmdResult, setCmdResult] = useState<RedisCommandResult | null>(null);
  const [cmdBusy, setCmdBusy] = useState(false);
  const [cmdError, setCmdError] = useState("");
  const [tunnelBusy, setTunnelBusy] = useState(false);
  const [tunnelError, setTunnelError] = useState("");
  const [tunnelNotice, setTunnelNotice] = useState("");

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();
  const p = Number.parseInt(port, 10);
  const d = Number.parseInt(db, 10);
  const canBrowse = host.trim() && Number.isFinite(p) && p > 0 && Number.isFinite(d);

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
    if (!hasSsh || !tab.redisTunnelId) {
      return;
    }
    let cancelled = false;
    void syncTunnelState(tab, "redis", updateTab).then((info) => {
      if (cancelled || !info?.alive) {
        return;
      }
      setTunnelNotice(
        t("Tunnel ready on {host}:{port}.", {
          host: info.localHost,
          port: info.localPort,
        }),
      );
    });
    return () => {
      cancelled = true;
    };
  }, [hasSsh, tab.id, tab.redisTunnelId, tab.redisTunnelPort, updateTab, t]);

  function persistPort(nextPort: string) {
    const parsed = Number.parseInt(nextPort, 10);
    if (Number.isFinite(parsed) && parsed > 0) {
      updateTab(tab.id, { redisPort: parsed });
    }
  }

  function persistDb(nextDb: string) {
    const parsed = Number.parseInt(nextDb, 10);
    if (Number.isFinite(parsed)) {
      updateTab(tab.id, { redisDb: parsed });
    }
  }

  async function ensureConnectionTarget(forceTunnel = false) {
    if (!hasSsh) {
      return { host: host.trim(), port: p };
    }

    const info = await ensureTunnelSlot({
      tab,
      slot: "redis",
      remoteHost: host.trim(),
      remotePort: p,
      updateTab,
      force: forceTunnel,
    });
    setTunnelError("");
    setTunnelNotice(
      t("Tunnel ready on {host}:{port}.", {
        host: info.localHost,
        port: info.localPort,
      }),
    );
    return { host: info.localHost, port: info.localPort };
  }

  async function openTunnel(force = false) {
    if (!hasSsh || !canBrowse) {
      return;
    }
    setTunnelBusy(true);
    setTunnelError("");
    try {
      await ensureConnectionTarget(force);
    } catch (e) {
      setTunnelError(formatError(e));
    } finally {
      setTunnelBusy(false);
    }
  }

  async function closeTunnel() {
    if (!hasSsh || !tab.redisTunnelId) {
      return;
    }
    setTunnelBusy(true);
    setTunnelError("");
    try {
      await closeTunnelSlot(tab, "redis", updateTab);
      setTunnelNotice(t("Tunnel closed."));
    } catch (e) {
      setTunnelError(formatError(e));
    } finally {
      setTunnelBusy(false);
    }
  }

  async function invalidateTunnel() {
    if (!hasSsh || !tab.redisTunnelId) {
      return;
    }
    await closeTunnelSlot(tab, "redis", updateTab);
    setTunnelNotice("");
    setTunnelError("");
  }

  async function browse(nextKey = keyName) {
    setBusy(true);
    setError("");
    try {
      const target = await ensureConnectionTarget();
      const s = await cmd.redisBrowse({
        host: target.host,
        port: target.port,
        db: d,
        pattern: pattern.trim() || "*",
        key: nextKey.trim() || null,
      });
      setState(s);
      setKeyName(s.keyName);
    } catch (e) {
      setState(null);
      setError(formatError(e));
    } finally {
      setBusy(false);
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
      });
      setCmdResult(r);
    } catch (e) {
      setCmdResult(null);
      setCmdError(formatError(e));
    } finally {
      setCmdBusy(false);
    }
  }

  const connName = host.trim() || t("Redis Browser");
  const connSub = host.trim()
    ? t("{host}:{port} · db {db}{suffix}", {
        host,
        port,
        db,
        suffix: hasSsh ? ` · ${t("SSH tunnel")}` : "",
      })
    : t("Not connected");
  const connTag = (
    <>
      <StatusDot tone={state ? "pos" : "off"} />
      {state ? `:${port}` : t("offline")}
    </>
  );

  return (
    <>
      <PanelHeader
        icon={Zap}
        title={t("Redis")}
        meta={t("{name} · db {db}", {
          name: tab.title || host || t("Redis"),
          db,
        })}
      />
      <DbConnRow
        icon={Zap}
        tint="var(--neg-dim)"
        iconTint="var(--neg)"
        name={connName}
        sub={connSub}
        tag={connTag}
      />
      <div className="panel-scroll">
      <section className="panel-section">
        <div className="form-stack">
          <div className="field-grid">
            <label className="field-stack">
              <span className="field-label">{t("Host")}</span>
              <input
                className="field-input"
                onChange={(event) => {
                  const nextValue = event.currentTarget.value;
                  if (hasSsh && tab.redisTunnelId && nextValue !== host) {
                    void invalidateTunnel();
                  }
                  setHost(nextValue);
                  updateTab(tab.id, { redisHost: nextValue });
                }}
                value={host}
              />
            </label>
            <label className="field-stack">
              <span className="field-label">{t("Port")}</span>
              <input
                className="field-input field-input--narrow"
                onChange={(event) => {
                  const nextValue = event.currentTarget.value;
                  if (hasSsh && tab.redisTunnelId && nextValue !== port) {
                    void invalidateTunnel();
                  }
                  setPort(nextValue);
                  persistPort(nextValue);
                }}
                value={port}
              />
            </label>
          </div>
          <div className="field-grid">
            <label className="field-stack">
              <span className="field-label">{t("DB")}</span>
              <input
                className="field-input"
                onChange={(event) => {
                  const nextValue = event.currentTarget.value;
                  setDb(nextValue);
                  persistDb(nextValue);
                }}
                value={db}
              />
            </label>
            <label className="field-stack">
              <span className="field-label">{t("Pattern")}</span>
              <input className="field-input" onChange={(event) => setPattern(event.currentTarget.value)} value={pattern} />
            </label>
          </div>
          {hasSsh && (
            <>
              <div className="data-meta-grid">
                <div className="meta-chip">
                  <span>{t("Tunnel remote")}</span>
                  <strong>{host.trim() || "127.0.0.1"}:{Number.isFinite(p) && p > 0 ? p : "?"}</strong>
                </div>
                <div className="meta-chip">
                  <span>{t("Tunnel local")}</span>
                  <strong>{tab.redisTunnelPort ? `127.0.0.1:${tab.redisTunnelPort}` : "—"}</strong>
                </div>
              </div>
              <div className="button-row">
                <button className="mini-button" disabled={!canBrowse || !!tab.redisTunnelId || tunnelBusy} onClick={() => void openTunnel(false)} type="button">
                  {tunnelBusy ? t("Opening...") : t("Open Tunnel")}
                </button>
                <button className="mini-button" disabled={!tab.redisTunnelId || tunnelBusy} onClick={() => void openTunnel(true)} type="button">
                  {t("Refresh Tunnel")}
                </button>
                <button className="mini-button" disabled={!tab.redisTunnelId || tunnelBusy} onClick={() => void closeTunnel()} type="button">
                  {t("Close Tunnel")}
                </button>
              </div>
              <div className="inline-note">{t("Queries will connect through the SSH tunnel.")}</div>
              {tunnelNotice && <div className="status-note">{tunnelNotice}</div>}
              {tunnelError && <div className="status-note status-note--error">{tunnelError}</div>}
            </>
          )}
          <div className="button-row">
            <button className="mini-button" disabled={!canBrowse || busy} onClick={() => void browse()} type="button">{busy ? t("Scanning...") : t("Scan Keys")}</button>
          </div>
          {state && <div className="status-note">{state.pong} · {state.serverVersion || "?"}{state.usedMemory ? ` · ${state.usedMemory}` : ""}</div>}
          {error && <div className="status-note status-note--error">{error}</div>}
        </div>
      </section>

      {state && (
        <section className="panel-section">
          <div className="panel-section__title"><span>{t("Keys")}</span></div>
          <div className="form-stack">
            <div className="token-list">
              {state.keys.map((k) => (
                <button
                  key={k}
                  className={state.keyName === k ? "token-button token-button--selected" : "token-button"}
                  onClick={() => {
                    setKeyName(k);
                    setCommand(`TYPE ${quoteCommandArg(k)}`);
                    void browse(k);
                  }}
                  type="button"
                >
                  {k}
                </button>
              ))}
            </div>
            {state.truncated && <div className="inline-note">{t("Truncated")}</div>}
          </div>
        </section>
      )}

      {state?.details && (
        <section className="panel-section">
          <div className="panel-section__title"><span>{t("Key Preview")}</span></div>
          <div className="form-stack">
            <div className="data-meta-grid">
              <div className="meta-chip"><span>{t("Key")}</span><strong>{state.details.key}</strong></div>
              <div className="meta-chip"><span>{t("Type")}</span><strong>{state.details.kind}</strong></div>
              <div className="meta-chip"><span>{t("Length")}</span><strong>{state.details.length}</strong></div>
              <div className="meta-chip"><span>{t("TTL")}</span><strong>{state.details.ttlSeconds}</strong></div>
            </div>
            <div className="preview-list">{state.details.preview.map((item, index) => <div className="preview-item" key={index}>{item}</div>)}</div>
          </div>
        </section>
      )}

      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Command Editor")}</span></div>
        <div className="form-stack">
          <textarea className="field-textarea field-textarea--editor" onChange={(event) => setCommand(event.currentTarget.value)} rows={4} value={command} />
          <div className="button-row">
            <button className="mini-button" disabled={!canBrowse || cmdBusy} onClick={() => void runCommand()} type="button">{cmdBusy ? t("Running...") : t("Run Command")}</button>
          </div>
        </div>
      </section>

      {(cmdResult || cmdError) && (
        <section className="panel-section">
          <div className="panel-section__title"><span>{t("Command Response")}</span></div>
          {cmdError ? <div className="status-note status-note--error">{cmdError}</div> : cmdResult ? (
            <div className="form-stack">
              <div className="inline-note">{cmdResult.summary} · {cmdResult.elapsedMs} ms</div>
              <div className="preview-list">{cmdResult.lines.map((line, index) => <div className="preview-item" key={index}>{line}</div>)}</div>
            </div>
          ) : null}
        </section>
      )}
    </div>
    </>
  );
}
