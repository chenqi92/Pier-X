import { Database } from "lucide-react";
import { useEffect, useState } from "react";
import * as cmd from "../lib/commands";
import { isReadOnlySql, queryResultToTsv } from "../lib/commands";
import { closeTunnelSlot, ensureTunnelSlot, syncTunnelState } from "../lib/sshTunnel";
import type { MysqlBrowserState, QueryExecutionResult, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import DbConnRow from "../components/DbConnRow";
import PanelHeader from "../components/PanelHeader";
import PreviewTable from "../components/PreviewTable";
import QueryResultPanel from "../components/QueryResultPanel";
import StatusDot from "../components/StatusDot";
import { useTabStore } from "../stores/useTabStore";

type Props = { tab: TabState };

export default function MySqlPanel({ tab }: Props) {
  const { t } = useI18n();
  const updateTab = useTabStore((s) => s.updateTab);
  const [host, setHost] = useState(tab.mysqlHost);
  const [port, setPort] = useState(String(tab.mysqlPort));
  const [user, setUser] = useState(tab.mysqlUser);
  const [password, setPassword] = useState(tab.mysqlPassword);
  const [dbName, setDbName] = useState(tab.mysqlDatabase);
  const [tableName, setTableName] = useState("");
  const [sql, setSql] = useState("SHOW TABLES;");
  const [readOnly, setReadOnly] = useState(true);
  const [writeConfirm, setWriteConfirm] = useState("");
  const [state, setState] = useState<MysqlBrowserState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [queryResult, setQueryResult] = useState<QueryExecutionResult | null>(null);
  const [queryBusy, setQueryBusy] = useState(false);
  const [queryError, setQueryError] = useState("");
  const [notice, setNotice] = useState("");
  const [tunnelBusy, setTunnelBusy] = useState(false);
  const [tunnelError, setTunnelError] = useState("");
  const [tunnelNotice, setTunnelNotice] = useState("");

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();
  const p = Number.parseInt(port, 10);
  const canBrowse = host.trim() && user.trim() && Number.isFinite(p) && p > 0;
  const needsWrite = sql.trim() && !isReadOnlySql(sql);
  const canRun = canBrowse && sql.trim() && !queryBusy && (!needsWrite || (!readOnly && writeConfirm.trim().toUpperCase() === "WRITE"));

  useEffect(() => {
    setHost((current) => (current === tab.mysqlHost ? current : tab.mysqlHost));
  }, [tab.mysqlHost]);

  useEffect(() => {
    const next = String(tab.mysqlPort);
    setPort((current) => (current === next ? current : next));
  }, [tab.mysqlPort]);

  useEffect(() => {
    setUser((current) => (current === tab.mysqlUser ? current : tab.mysqlUser));
  }, [tab.mysqlUser]);

  useEffect(() => {
    setPassword((current) => (current === tab.mysqlPassword ? current : tab.mysqlPassword));
  }, [tab.mysqlPassword]);

  useEffect(() => {
    setDbName((current) => (current === tab.mysqlDatabase ? current : tab.mysqlDatabase));
  }, [tab.mysqlDatabase]);

  useEffect(() => {
    if (!hasSsh || !tab.mysqlTunnelId) {
      return;
    }
    let cancelled = false;
    void syncTunnelState(tab, "mysql", updateTab).then((info) => {
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
  }, [hasSsh, tab.id, tab.mysqlTunnelId, tab.mysqlTunnelPort, updateTab, t]);

  function persistPort(nextPort: string) {
    const parsed = Number.parseInt(nextPort, 10);
    if (Number.isFinite(parsed) && parsed > 0) {
      updateTab(tab.id, { mysqlPort: parsed });
    }
  }

  async function ensureConnectionTarget(forceTunnel = false) {
    if (!hasSsh) {
      return { host: host.trim(), port: p };
    }

    const info = await ensureTunnelSlot({
      tab,
      slot: "mysql",
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
      setTunnelError(String(e));
    } finally {
      setTunnelBusy(false);
    }
  }

  async function closeTunnel() {
    if (!hasSsh || !tab.mysqlTunnelId) {
      return;
    }
    setTunnelBusy(true);
    setTunnelError("");
    try {
      await closeTunnelSlot(tab, "mysql", updateTab);
      setTunnelNotice(t("Tunnel closed."));
    } catch (e) {
      setTunnelError(String(e));
    } finally {
      setTunnelBusy(false);
    }
  }

  async function invalidateTunnel() {
    if (!hasSsh || !tab.mysqlTunnelId) {
      return;
    }
    await closeTunnelSlot(tab, "mysql", updateTab);
    setTunnelNotice("");
    setTunnelError("");
  }

  async function browse(nextDb = dbName, nextTable = tableName) {
    setBusy(true);
    setError("");
    try {
      const target = await ensureConnectionTarget();
      const s = await cmd.mysqlBrowse({
        host: target.host,
        port: target.port,
        user: user.trim(),
        password,
        database: nextDb.trim() || null,
        table: nextTable.trim() || null,
      });
      setState(s);
      setDbName(s.databaseName);
      setTableName(s.tableName);
      updateTab(tab.id, { mysqlDatabase: s.databaseName });
    } catch (e) {
      setState(null);
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function runQuery() {
    setQueryBusy(true);
    setQueryError("");
    setNotice("");
    try {
      const target = await ensureConnectionTarget();
      const r = await cmd.mysqlExecute({
        host: target.host,
        port: target.port,
        user: user.trim(),
        password,
        database: dbName.trim() || null,
        sql,
      });
      setQueryResult(r);
      setNotice(t("{elapsed} ms", { elapsed: r.elapsedMs }));
      if (needsWrite) {
        setReadOnly(true);
        setWriteConfirm("");
      }
    } catch (e) {
      setQueryResult(null);
      setQueryError(String(e));
    } finally {
      setQueryBusy(false);
    }
  }

  const connName = dbName.trim() || host.trim() || t("MySQL Browser");
  const connSub = host.trim()
    ? `${user || "?"}@${host}:${port}${hasSsh ? " · ssh tunnel" : ""}`
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
        icon={Database}
        title="MYSQL"
        meta={`${dbName.trim() || "mysql"} · ${hasSsh ? `tunnel :${port}` : `${host}:${port}`}`}
      />
      <DbConnRow
        icon={Database}
        tint="var(--warn-dim)"
        iconTint="var(--warn)"
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
                  if (hasSsh && tab.mysqlTunnelId && nextValue !== host) {
                    void invalidateTunnel();
                  }
                  setHost(nextValue);
                  updateTab(tab.id, { mysqlHost: nextValue });
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
                  if (hasSsh && tab.mysqlTunnelId && nextValue !== port) {
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
              <span className="field-label">{t("User")}</span>
              <input
                className="field-input"
                onChange={(event) => {
                  const nextValue = event.currentTarget.value;
                  setUser(nextValue);
                  updateTab(tab.id, { mysqlUser: nextValue });
                }}
                value={user}
              />
            </label>
            <label className="field-stack">
              <span className="field-label">{t("Password")}</span>
              <input
                className="field-input"
                type="password"
                onChange={(event) => {
                  const nextValue = event.currentTarget.value;
                  setPassword(nextValue);
                  updateTab(tab.id, { mysqlPassword: nextValue });
                }}
                value={password}
              />
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
                  <strong>{tab.mysqlTunnelPort ? `127.0.0.1:${tab.mysqlTunnelPort}` : "—"}</strong>
                </div>
              </div>
              <div className="button-row">
                <button className="mini-button" disabled={!canBrowse || !!tab.mysqlTunnelId || tunnelBusy} onClick={() => void openTunnel(false)} type="button">
                  {tunnelBusy ? t("Opening...") : t("Open Tunnel")}
                </button>
                <button className="mini-button" disabled={!tab.mysqlTunnelId || tunnelBusy} onClick={() => void openTunnel(true)} type="button">
                  {t("Refresh Tunnel")}
                </button>
                <button className="mini-button" disabled={!tab.mysqlTunnelId || tunnelBusy} onClick={() => void closeTunnel()} type="button">
                  {t("Close Tunnel")}
                </button>
              </div>
              <div className="inline-note">{t("Queries will connect through the SSH tunnel.")}</div>
              {tunnelNotice && <div className="status-note">{tunnelNotice}</div>}
              {tunnelError && <div className="status-note status-note--error">{tunnelError}</div>}
            </>
          )}
          <div className="button-row">
            <button className="mini-button" disabled={!canBrowse || busy} onClick={() => void browse()} type="button">{busy ? t("Browsing...") : t("Browse")}</button>
          </div>
          {error && <div className="status-note status-note--error">{error}</div>}
        </div>
      </section>

      {state && (
        <section className="panel-section">
          <div className="panel-section__title"><span>{t("Tables & Columns")}</span></div>
          <div className="form-stack">
            <div className="token-list">
              {state.databases.map((db) => (
                <button
                  key={db}
                  className={state.databaseName === db ? "token-button token-button--selected" : "token-button"}
                  onClick={() => {
                    setDbName(db);
                    updateTab(tab.id, { mysqlDatabase: db });
                    void browse(db, "");
                  }}
                  type="button"
                >
                  {db}
                </button>
              ))}
            </div>
            <div className="token-list">
              {state.tables.map((tbl) => (
                <button
                  key={tbl}
                  className={state.tableName === tbl ? "token-button token-button--selected" : "token-button"}
                  onClick={() => {
                    setTableName(tbl);
                    setSql(`SELECT * FROM \`${tbl}\` LIMIT 100;`);
                    void browse(dbName, tbl);
                  }}
                  type="button"
                >
                  {tbl}
                </button>
              ))}
            </div>
            {state.columns.length > 0 && (
              <div className="column-list">
                {state.columns.map((col) => (
                  <div className="column-row" key={col.name}>
                    <div className="column-row__head">
                      <strong>{col.name}</strong>
                      <span className="connection-pill">{col.columnType}</span>
                    </div>
                    <div className="column-row__meta">
                      {col.nullable ? t("Nullable") : t("Not null")}
                      {col.key ? ` · ${col.key}` : ""}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </section>
      )}

      {state && (
        <section className="panel-section">
          <div className="panel-section__title"><span>{t("Sample Rows")}</span></div>
          <PreviewTable preview={state.preview} emptyLabel={t("Select a table.")} />
        </section>
      )}

      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Query Editor")}</span></div>
        <div className="form-stack">
          <div className="query-guard-row">
            <span className={readOnly ? "safety-pill safety-pill--locked" : "safety-pill safety-pill--unlocked"}>{readOnly ? t("Read Only") : t("Writes Unlocked")}</span>
            <button className="mini-button" onClick={() => { setReadOnly((prev) => !prev); setWriteConfirm(""); }} type="button">{readOnly ? t("Unlock Writes") : t("Re-lock Writes")}</button>
          </div>
          <textarea className="field-textarea field-textarea--editor" onChange={(event) => setSql(event.currentTarget.value)} rows={4} value={sql} />
          {needsWrite && !readOnly && <input className="field-input" onChange={(event) => setWriteConfirm(event.currentTarget.value)} placeholder={t("Type WRITE to confirm")} value={writeConfirm} />}
          <div className="button-row">
            <button className="mini-button" disabled={!canRun} onClick={() => void runQuery()} type="button">{queryBusy ? t("Running...") : t("Run Query")}</button>
            {queryResult && <button className="mini-button" onClick={() => { navigator.clipboard.writeText(queryResultToTsv(queryResult)).catch(() => {}); setNotice(t("Copied TSV")); }} type="button">{t("Copy TSV")}</button>}
          </div>
          {notice && <div className="status-note">{notice}</div>}
        </div>
      </section>

      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Query Results")}</span></div>
        <QueryResultPanel result={queryResult} error={queryError} emptyLabel={t("Run a query to see results.")} />
      </section>
    </div>
    </>
  );
}
