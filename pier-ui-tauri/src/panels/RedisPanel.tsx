import { useState } from "react";
import * as cmd from "../lib/commands";
import { quoteCommandArg } from "../lib/commands";
import type { RedisBrowserState, RedisCommandResult, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";

type Props = { tab: TabState };

export default function RedisPanel({ tab }: Props) {
  const { t } = useI18n();
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

  const p = Number.parseInt(port, 10);
  const d = Number.parseInt(db, 10);
  const canBrowse = host.trim() && Number.isFinite(p) && p > 0 && Number.isFinite(d);

  async function browse(nextKey = keyName) {
    setBusy(true); setError("");
    try {
      const s = await cmd.redisBrowse({ host: host.trim(), port: p, db: d, pattern: pattern.trim() || "*", key: nextKey.trim() || null });
      setState(s); setKeyName(s.keyName);
    } catch (e) { setState(null); setError(String(e)); }
    finally { setBusy(false); }
  }

  async function runCommand() {
    setCmdBusy(true); setCmdError("");
    try {
      const r = await cmd.redisExecute({ host: host.trim(), port: p, db: d, command });
      setCmdResult(r);
    } catch (e) { setCmdResult(null); setCmdError(String(e)); }
    finally { setCmdBusy(false); }
  }

  return (
    <div className="panel-scroll">
      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Redis Browser")}</span></div>
        <div className="form-stack">
          <div className="field-grid">
            <label className="field-stack"><span className="field-label">{t("Host")}</span><input className="field-input" onChange={(e) => setHost(e.currentTarget.value)} value={host} /></label>
            <label className="field-stack"><span className="field-label">{t("Port")}</span><input className="field-input field-input--narrow" onChange={(e) => setPort(e.currentTarget.value)} value={port} /></label>
          </div>
          <div className="field-grid">
            <label className="field-stack"><span className="field-label">{t("DB")}</span><input className="field-input" onChange={(e) => setDb(e.currentTarget.value)} value={db} /></label>
            <label className="field-stack"><span className="field-label">{t("Pattern")}</span><input className="field-input" onChange={(e) => setPattern(e.currentTarget.value)} value={pattern} /></label>
          </div>
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
            <div className="token-list">{state.keys.map((k) => <button key={k} className={state.keyName === k ? "token-button token-button--selected" : "token-button"} onClick={() => { setKeyName(k); setCommand(`TYPE ${quoteCommandArg(k)}`); void browse(k); }} type="button">{k}</button>)}</div>
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
            <div className="preview-list">{state.details.preview.map((item, i) => <div className="preview-item" key={i}>{item}</div>)}</div>
          </div>
        </section>
      )}

      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Command Editor")}</span></div>
        <div className="form-stack">
          <textarea className="field-textarea field-textarea--editor" onChange={(e) => setCommand(e.currentTarget.value)} rows={4} value={command} />
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
              <div className="preview-list">{cmdResult.lines.map((line, i) => <div className="preview-item" key={i}>{line}</div>)}</div>
            </div>
          ) : null}
        </section>
      )}
    </div>
  );
}
