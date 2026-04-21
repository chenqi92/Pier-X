import { HardDrive } from "lucide-react";
import { useState } from "react";
import * as cmd from "../lib/commands";
import { isReadOnlySql, queryResultToTsv } from "../lib/commands";
import type { QueryExecutionResult, SqliteBrowserState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import DbConnRow from "../components/DbConnRow";
import PanelHeader from "../components/PanelHeader";
import PreviewTable from "../components/PreviewTable";
import QueryResultPanel from "../components/QueryResultPanel";
import StatusDot from "../components/StatusDot";

export default function SqlitePanel() {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const [path, setPath] = useState("");
  const [tableName, setTableName] = useState("");
  const [sql, setSql] = useState("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;");
  const [readOnly, setReadOnly] = useState(true);
  const [writeConfirm, setWriteConfirm] = useState("");
  const [state, setState] = useState<SqliteBrowserState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [queryResult, setQueryResult] = useState<QueryExecutionResult | null>(null);
  const [queryBusy, setQueryBusy] = useState(false);
  const [queryError, setQueryError] = useState("");
  const [notice, setNotice] = useState("");

  const canBrowse = path.trim().length > 0;
  const needsWrite = sql.trim() && !isReadOnlySql(sql);
  const canRun = canBrowse && sql.trim() && !queryBusy && (!needsWrite || (!readOnly && writeConfirm.trim().toUpperCase() === "WRITE"));

  async function browse(nextTable = tableName) {
    setBusy(true); setError("");
    try {
      const s = await cmd.sqliteBrowse(path.trim(), nextTable.trim() || null);
      setState(s); setTableName(s.tableName);
    } catch (e) { setState(null); setError(formatError(e)); }
    finally { setBusy(false); }
  }

  async function runQuery() {
    setQueryBusy(true); setQueryError(""); setNotice("");
    try {
      const r = await cmd.sqliteExecute(path.trim(), sql);
      setQueryResult(r); setNotice(t("{elapsed} ms", { elapsed: r.elapsedMs }));
      if (needsWrite) { setReadOnly(true); setWriteConfirm(""); }
      void browse(tableName);
    } catch (e) { setQueryResult(null); setQueryError(formatError(e)); }
    finally { setQueryBusy(false); }
  }

  const trimmedPath = path.trim();
  const connName = trimmedPath || t("SQLite Browser");
  const connSub = trimmedPath ? trimmedPath : t("Not connected");
  const dbFileName = trimmedPath ? (trimmedPath.split(/[/\\]/).pop() || trimmedPath) : "";
  const headerMeta = dbFileName
    ? state
      ? t("{file} · {count} tables", { file: dbFileName, count: state.tables.length })
      : dbFileName
    : t("No database");
  const connTag = (
    <>
      <StatusDot tone={state ? "pos" : "off"} />
      {state ? t("open") : t("offline")}
    </>
  );

  return (
    <>
      <PanelHeader
        icon={HardDrive}
        title={t("SQLite")}
        meta={headerMeta}
      />
      <DbConnRow
        icon={HardDrive}
        tint="var(--panel-2)"
        iconTint="var(--ink-2)"
        name={connName}
        sub={connSub}
        tag={connTag}
      />
      <div className="panel-scroll">
      <section className="panel-section">
        <div className="form-stack">
          <label className="field-stack">
            <span className="field-label">{t("Database file")}</span>
            <div className="branch-row">
              <input className="field-input" onChange={(e) => setPath(e.currentTarget.value)} placeholder={t("/path/to/app.db")} value={path} />
              <button className="mini-button" disabled={!canBrowse || busy} onClick={() => void browse()} type="button">{busy ? t("Browsing...") : t("Browse")}</button>
            </div>
          </label>
          {error && <div className="status-note status-note--error">{error}</div>}
        </div>
      </section>

      {state && (
        <section className="panel-section">
          <div className="panel-section__title"><span>{t("Tables & Columns")}</span></div>
          <div className="form-stack">
            <div className="token-list">{state.tables.map((tbl) => <button key={tbl} className={state.tableName === tbl ? "token-button token-button--selected" : "token-button"} onClick={() => { setTableName(tbl); setSql(`SELECT * FROM "${tbl.replace(/"/g, '""')}" LIMIT 100;`); void browse(tbl); }} type="button">{tbl}</button>)}</div>
            {state.columns.length > 0 && <div className="column-list">{state.columns.map((col) => <div className="column-row" key={col.name}><div className="column-row__head"><strong>{col.name}</strong><span className="connection-pill">{col.colType}</span></div><div className="column-row__meta">{col.notNull ? t("Not null") : t("Nullable")}{col.primaryKey ? ` · ${t("PK")}` : ""}</div></div>)}</div>}
          </div>
        </section>
      )}

      {state && <section className="panel-section"><div className="panel-section__title"><span>{t("Sample Rows")}</span></div><PreviewTable preview={state.preview} emptyLabel={t("Select a table.")} /></section>}

      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Query Editor")}</span></div>
        <div className="form-stack">
          <div className="query-guard-row">
            <span className={readOnly ? "safety-pill safety-pill--locked" : "safety-pill safety-pill--unlocked"}>{readOnly ? t("Read Only") : t("Writes Unlocked")}</span>
            <button className="mini-button" onClick={() => { setReadOnly((p) => !p); setWriteConfirm(""); }} type="button">{readOnly ? t("Unlock Writes") : t("Re-lock Writes")}</button>
          </div>
          <textarea className="field-textarea field-textarea--editor" onChange={(e) => setSql(e.currentTarget.value)} rows={4} value={sql} />
          {needsWrite && !readOnly && <input className="field-input" onChange={(e) => setWriteConfirm(e.currentTarget.value)} placeholder={t("Type WRITE to confirm")} value={writeConfirm} />}
          <div className="button-row">
            <button className="mini-button" disabled={!canRun} onClick={() => void runQuery()} type="button">{queryBusy ? t("Running...") : t("Run Query")}</button>
            {queryResult && <button className="mini-button" onClick={() => { navigator.clipboard.writeText(queryResultToTsv(queryResult)).catch(() => {}); setNotice(t("Copied")); }} type="button">{t("Copy TSV")}</button>}
          </div>
          {notice && <div className="status-note">{notice}</div>}
        </div>
      </section>

      <section className="panel-section"><div className="panel-section__title"><span>{t("Query Results")}</span></div><QueryResultPanel result={queryResult} error={queryError} emptyLabel={t("Run a query.")} /></section>
    </div>
    </>
  );
}
