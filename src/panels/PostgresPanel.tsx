import { Database } from "lucide-react";
import { useEffect, useState } from "react";
import * as cmd from "../lib/commands";
import { isReadOnlySql, queryResultToTsv } from "../lib/commands";
import { closeTunnelSlot, ensureTunnelSlot, syncTunnelState } from "../lib/sshTunnel";
import type { PostgresBrowserState, QueryExecutionResult, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import DbConnRow from "../components/DbConnRow";
import PanelHeader from "../components/PanelHeader";
import PreviewTable from "../components/PreviewTable";
import QueryResultPanel from "../components/QueryResultPanel";
import StatusDot from "../components/StatusDot";
import { useTabStore } from "../stores/useTabStore";

type Props = { tab: TabState };

export default function PostgresPanel({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const updateTab = useTabStore((s) => s.updateTab);
  const [host, setHost] = useState(tab.pgHost);
  const [port, setPort] = useState(String(tab.pgPort));
  const [user, setUser] = useState(tab.pgUser);
  const [password, setPassword] = useState(tab.pgPassword);
  const [dbName, setDbName] = useState(tab.pgDatabase);
  const [schema, setSchema] = useState("public");
  const [tableName, setTableName] = useState("");
  const [sql, setSql] = useState("SELECT version();");
  const [readOnly, setReadOnly] = useState(true);
  const [writeConfirm, setWriteConfirm] = useState("");
  const [state, setState] = useState<PostgresBrowserState | null>(null);
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
    setHost((current) => (current === tab.pgHost ? current : tab.pgHost));
  }, [tab.pgHost]);

  useEffect(() => {
    const next = String(tab.pgPort);
    setPort((current) => (current === next ? current : next));
  }, [tab.pgPort]);

  useEffect(() => {
    setUser((current) => (current === tab.pgUser ? current : tab.pgUser));
  }, [tab.pgUser]);

  useEffect(() => {
    setPassword((current) => (current === tab.pgPassword ? current : tab.pgPassword));
  }, [tab.pgPassword]);

  useEffect(() => {
    setDbName((current) => (current === tab.pgDatabase ? current : tab.pgDatabase));
  }, [tab.pgDatabase]);

  useEffect(() => {
    if (!hasSsh || !tab.pgTunnelId) {
      return;
    }
    let cancelled = false;
    void syncTunnelState(tab, "postgres", updateTab).then((info) => {
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
  }, [hasSsh, tab.id, tab.pgTunnelId, tab.pgTunnelPort, updateTab, t]);

  function persistPort(nextPort: string) {
    const parsed = Number.parseInt(nextPort, 10);
    if (Number.isFinite(parsed) && parsed > 0) {
      updateTab(tab.id, { pgPort: parsed });
    }
  }

  async function ensureConnectionTarget(forceTunnel = false) {
    if (!hasSsh) {
      return { host: host.trim(), port: p };
    }

    const info = await ensureTunnelSlot({
      tab,
      slot: "postgres",
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
    if (!hasSsh || !tab.pgTunnelId) {
      return;
    }
    setTunnelBusy(true);
    setTunnelError("");
    try {
      await closeTunnelSlot(tab, "postgres", updateTab);
      setTunnelNotice(t("Tunnel closed."));
    } catch (e) {
      setTunnelError(formatError(e));
    } finally {
      setTunnelBusy(false);
    }
  }

  async function invalidateTunnel() {
    if (!hasSsh || !tab.pgTunnelId) {
      return;
    }
    await closeTunnelSlot(tab, "postgres", updateTab);
    setTunnelNotice("");
    setTunnelError("");
  }

  async function browse(nextDb = dbName, nextTable = tableName) {
    setBusy(true);
    setError("");
    try {
      const target = await ensureConnectionTarget();
      const s = await cmd.postgresBrowse({
        host: target.host,
        port: target.port,
        user: user.trim(),
        password,
        database: nextDb.trim() || null,
        schema: schema.trim() || null,
        table: nextTable.trim() || null,
      });
      setState(s);
      setDbName(s.databaseName);
      setSchema(s.schemaName);
      setTableName(s.tableName);
      updateTab(tab.id, { pgDatabase: s.databaseName });
    } catch (e) {
      setState(null);
      setError(formatError(e));
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
      const r = await cmd.postgresExecute({
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
      setQueryError(formatError(e));
    } finally {
      setQueryBusy(false);
    }
  }

  const connName = dbName.trim() || host.trim() || t("PostgreSQL Browser");
  const connSub = host.trim()
    ? t("{user}@{host}:{port}{suffix}", {
        user: user || "?",
        host,
        port,
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
        icon={Database}
        title={t("PostgreSQL")}
        meta={
          hasSsh
            ? t("{database} · tunnel :{port}", {
                database: dbName.trim() || t("PostgreSQL"),
                port,
              })
            : t("{database} · {host}:{port}", {
                database: dbName.trim() || t("PostgreSQL"),
                host,
                port,
              })
        }
      />
      <DbConnRow
        icon={Database}
        tint="var(--accent-dim)"
        iconTint="var(--accent)"
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
                  if (hasSsh && tab.pgTunnelId && nextValue !== host) {
                    void invalidateTunnel();
                  }
                  setHost(nextValue);
                  updateTab(tab.id, { pgHost: nextValue });
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
                  if (hasSsh && tab.pgTunnelId && nextValue !== port) {
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
                  updateTab(tab.id, { pgUser: nextValue });
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
                  updateTab(tab.id, { pgPassword: nextValue });
                }}
                value={password}
              />
            </label>
          </div>
          <div className="field-grid">
            <label className="field-stack">
              <span className="field-label">{t("Database")}</span>
              <input
                className="field-input"
                onChange={(event) => {
                  const nextValue = event.currentTarget.value;
                  setDbName(nextValue);
                  updateTab(tab.id, { pgDatabase: nextValue });
                }}
                value={dbName}
              />
            </label>
            <label className="field-stack">
              <span className="field-label">{t("Schema")}</span>
              <input className="field-input" onChange={(event) => setSchema(event.currentTarget.value)} value={schema} />
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
                  <strong>{tab.pgTunnelPort ? `127.0.0.1:${tab.pgTunnelPort}` : "—"}</strong>
                </div>
              </div>
              <div className="button-row">
                <button className="mini-button" disabled={!canBrowse || !!tab.pgTunnelId || tunnelBusy} onClick={() => void openTunnel(false)} type="button">
                  {tunnelBusy ? t("Opening...") : t("Open Tunnel")}
                </button>
                <button className="mini-button" disabled={!tab.pgTunnelId || tunnelBusy} onClick={() => void openTunnel(true)} type="button">
                  {t("Refresh Tunnel")}
                </button>
                <button className="mini-button" disabled={!tab.pgTunnelId || tunnelBusy} onClick={() => void closeTunnel()} type="button">
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
                    updateTab(tab.id, { pgDatabase: db });
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
                    setSql(`SELECT * FROM "${schema}"."${tbl.replace(/"/g, "\"\"")}" LIMIT 100;`);
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
