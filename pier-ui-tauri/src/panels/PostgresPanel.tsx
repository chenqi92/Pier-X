import { useState } from "react";
import * as cmd from "../lib/commands";
import { isReadOnlySql, queryResultToTsv } from "../lib/commands";
import type { PostgresBrowserState, QueryExecutionResult, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import PreviewTable from "../components/PreviewTable";
import QueryResultPanel from "../components/QueryResultPanel";

type Props = { tab: TabState };

export default function PostgresPanel({ tab }: Props) {
  const { t } = useI18n();
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

  const p = Number.parseInt(port, 10);
  const canBrowse = host.trim() && user.trim() && Number.isFinite(p) && p > 0;
  const needsWrite = sql.trim() && !isReadOnlySql(sql);
  const canRun = canBrowse && sql.trim() && !queryBusy && (!needsWrite || (!readOnly && writeConfirm.trim().toUpperCase() === "WRITE"));

  async function browse(nextDb = dbName, nextTable = tableName) {
    setBusy(true); setError("");
    try {
      const s = await cmd.postgresBrowse({ host: host.trim(), port: p, user: user.trim(), password, database: nextDb.trim() || null, schema: schema.trim() || null, table: nextTable.trim() || null });
      setState(s); setDbName(s.databaseName); setSchema(s.schemaName); setTableName(s.tableName);
    } catch (e) { setState(null); setError(String(e)); }
    finally { setBusy(false); }
  }

  async function runQuery() {
    setQueryBusy(true); setQueryError(""); setNotice("");
    try {
      const r = await cmd.postgresExecute({ host: host.trim(), port: p, user: user.trim(), password, database: dbName.trim() || null, sql });
      setQueryResult(r); setNotice(`${r.elapsedMs} ms`);
      if (needsWrite) { setReadOnly(true); setWriteConfirm(""); }
    } catch (e) { setQueryResult(null); setQueryError(String(e)); }
    finally { setQueryBusy(false); }
  }

  return (
    <div className="panel-scroll">
      <section className="panel-section">
        <div className="panel-section__title"><span>PostgreSQL Browser</span></div>
        <div className="form-stack">
          <div className="field-grid">
            <label className="field-stack"><span className="field-label">{t("Host")}</span><input className="field-input" onChange={(e) => setHost(e.currentTarget.value)} value={host} /></label>
            <label className="field-stack"><span className="field-label">{t("Port")}</span><input className="field-input field-input--narrow" onChange={(e) => setPort(e.currentTarget.value)} value={port} /></label>
          </div>
          <div className="field-grid">
            <label className="field-stack"><span className="field-label">{t("User")}</span><input className="field-input" onChange={(e) => setUser(e.currentTarget.value)} value={user} /></label>
            <label className="field-stack"><span className="field-label">{t("Password")}</span><input className="field-input" type="password" onChange={(e) => setPassword(e.currentTarget.value)} value={password} /></label>
          </div>
          <div className="field-grid">
            <label className="field-stack"><span className="field-label">Database</span><input className="field-input" onChange={(e) => setDbName(e.currentTarget.value)} value={dbName} /></label>
            <label className="field-stack"><span className="field-label">Schema</span><input className="field-input" onChange={(e) => setSchema(e.currentTarget.value)} value={schema} /></label>
          </div>
          <div className="button-row">
            <button className="mini-button" disabled={!canBrowse || busy} onClick={() => void browse()} type="button">{busy ? "Browsing..." : t("Browse")}</button>
          </div>
          {error && <div className="status-note status-note--error">{error}</div>}
        </div>
      </section>

      {state && (
        <section className="panel-section">
          <div className="panel-section__title"><span>{t("Tables & Columns")}</span></div>
          <div className="form-stack">
            <div className="token-list">{state.databases.map((db) => <button key={db} className={state.databaseName === db ? "token-button token-button--selected" : "token-button"} onClick={() => { setDbName(db); void browse(db, ""); }} type="button">{db}</button>)}</div>
            <div className="token-list">{state.tables.map((tbl) => <button key={tbl} className={state.tableName === tbl ? "token-button token-button--selected" : "token-button"} onClick={() => { setTableName(tbl); setSql(`SELECT * FROM "${schema}"."${tbl.replace(/"/g, '""')}" LIMIT 100;`); void browse(dbName, tbl); }} type="button">{tbl}</button>)}</div>
            {state.columns.length > 0 && <div className="column-list">{state.columns.map((col) => <div className="column-row" key={col.name}><div className="column-row__head"><strong>{col.name}</strong><span className="connection-pill">{col.columnType}</span></div><div className="column-row__meta">{col.nullable ? "nullable" : "not null"}{col.key ? ` · ${col.key}` : ""}</div></div>)}</div>}
          </div>
        </section>
      )}

      {state && <section className="panel-section"><div className="panel-section__title"><span>{t("Sample Rows")}</span></div><PreviewTable preview={state.preview} emptyLabel="Select a table." /></section>}

      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Query Editor")}</span></div>
        <div className="form-stack">
          <div className="query-guard-row">
            <span className={readOnly ? "safety-pill safety-pill--locked" : "safety-pill safety-pill--unlocked"}>{readOnly ? t("Read Only") : t("Writes Unlocked")}</span>
            <button className="mini-button" onClick={() => { setReadOnly((p) => !p); setWriteConfirm(""); }} type="button">{readOnly ? t("Unlock Writes") : t("Re-lock Writes")}</button>
          </div>
          <textarea className="field-textarea field-textarea--editor" onChange={(e) => setSql(e.currentTarget.value)} rows={4} value={sql} />
          {needsWrite && !readOnly && <input className="field-input" onChange={(e) => setWriteConfirm(e.currentTarget.value)} placeholder="Type WRITE to confirm" value={writeConfirm} />}
          <div className="button-row">
            <button className="mini-button" disabled={!canRun} onClick={() => void runQuery()} type="button">{queryBusy ? t("Running...") : t("Run Query")}</button>
            {queryResult && <button className="mini-button" onClick={() => { navigator.clipboard.writeText(queryResultToTsv(queryResult)).catch(() => {}); setNotice("Copied TSV"); }} type="button">{t("Copy TSV")}</button>}
          </div>
          {notice && <div className="status-note">{notice}</div>}
        </div>
      </section>

      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Query Results")}</span></div>
        <QueryResultPanel result={queryResult} error={queryError} emptyLabel="Run a query to see results." />
      </section>
    </div>
  );
}
