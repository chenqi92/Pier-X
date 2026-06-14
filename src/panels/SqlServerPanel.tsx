import { useRef, useState } from "react";
import { Columns3, Play, RefreshCw, Table2, Unplug } from "lucide-react";
import type { QueryExecutionResult, TabState } from "../lib/types";
import * as cmd from "../lib/commands";
import type { SqlServerColumnView, SqlServerOverview } from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import { DB_KIND_META } from "../lib/rightToolMeta";
import { useTabStore } from "../stores/useTabStore";
import {
  useDbCredentialFlow,
  type DbCredentialFieldAdapter,
} from "../components/db/useDbCredentialFlow";
import { inferEnv } from "../components/db/dbTheme";
import DbConnectSplash from "../components/db/DbConnectSplash";
import type { DbSplashRowData } from "../components/db/DbSplashRow";
import DbAddCredentialDialog, {
  type DbConnectionDraft,
} from "../components/DbAddCredentialDialog";
import PanelHeader from "../components/PanelHeader";
import Select from "../components/Select";
import DbAiGenerate from "../components/db/DbAiGenerate";

// SQL Server client, fully aligned with MySQL/PG: saved credentials in
// the keyring, the shared connect splash, and the SSH tunnel (slot
// "sqlserver"). The connected view is a lean tables + columns + T-SQL
// editor. TLS is off (tunnel-encrypted); see pier-core/services/sqlserver.

type Props = { tab: TabState | null };

const META = DB_KIND_META.sqlserver;

const SQLSERVER_ADAPTER: DbCredentialFieldAdapter = {
  readHost: (t) => t.mssqlHost,
  readPort: (t) => t.mssqlPort,
  readUser: (t) => t.mssqlUser,
  readPassword: (t) => t.mssqlPassword,
  readActiveCredId: (t) => t.mssqlActiveCredentialId,
  readTunnelId: (t) => t.mssqlTunnelId,
  readTunnelPort: (t) => t.mssqlTunnelPort,
  patchFromCred: (cred) => ({
    mssqlActiveCredentialId: cred.id,
    mssqlHost: cred.host,
    mssqlPort: cred.port,
    mssqlUser: cred.user,
    mssqlPassword: "",
    mssqlDatabase: cred.database ?? "",
    mssqlTunnelId: null,
    mssqlTunnelPort: null,
  }),
  patchFromSaved: (cred) => ({
    mssqlActiveCredentialId: cred.id,
    mssqlHost: cred.host,
    mssqlPort: cred.port,
    mssqlUser: cred.user,
    mssqlDatabase: cred.database ?? "",
    mssqlTunnelId: null,
    mssqlTunnelPort: null,
  }),
  patchFromDraft: (draft) => ({
    mssqlActiveCredentialId: null,
    mssqlHost: draft.host,
    mssqlPort: draft.port,
    mssqlUser: draft.user,
    mssqlPassword: draft.password,
    mssqlDatabase: draft.database ?? "",
    mssqlTunnelId: null,
    mssqlTunnelPort: null,
  }),
  patchPassword: (password) => ({ mssqlPassword: password }),
  patchPasswordAfterRotate: (password) => ({ mssqlPassword: password }),
};

export default function SqlServerPanel({ tab }: Props) {
  const { t } = useI18n();
  if (!tab) {
    return (
      <div className="panel-section panel-section--empty">
        <div className="status-note mono">{t("Open an SSH tab to connect to a database.")}</div>
      </div>
    );
  }
  return <SqlServerBody tab={tab} />;
}

function SqlServerBody({ tab }: { tab: TabState }) {
  const { t } = useI18n();
  const fmt = (e: unknown) => localizeError(e, t);
  const updateTab = useTabStore((s) => s.updateTab);
  const passwordInputRef = useRef<HTMLInputElement | null>(null);

  const [overview, setOverview] = useState<SqlServerOverview | null>(null);
  const [results, setResults] = useState<QueryExecutionResult | null>(null);
  const [columns, setColumns] = useState<SqlServerColumnView[] | null>(null);
  const [selected, setSelected] = useState("");
  const [sql, setSql] = useState("SELECT @@VERSION;");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  function resetPanel() {
    setOverview(null);
    setResults(null);
    setColumns(null);
    setSelected("");
    setError("");
  }

  async function browse(passwordOverride?: string, draft?: DbConnectionDraft) {
    setBusy(true);
    setError("");
    try {
      const target = await flow.ensureConnectionTarget(false, draft);
      const pw = passwordOverride !== undefined ? passwordOverride : tab.mssqlPassword;
      const user = draft?.user ?? tab.mssqlUser;
      const database = draft ? draft.database ?? "" : tab.mssqlDatabase;
      const ov = await cmd.mssqlOverview({
        host: target.host,
        port: target.port,
        user: user.trim(),
        password: pw,
        database: database.trim() || null,
      });
      setOverview(ov);
      if (ov.currentDatabase && ov.currentDatabase !== tab.mssqlDatabase) {
        updateTab(tab.id, { mssqlDatabase: ov.currentDatabase });
      }
    } catch (e) {
      setError(fmt(e));
      setOverview(null);
    } finally {
      setBusy(false);
    }
  }

  const flow = useDbCredentialFlow({
    tab,
    kind: "sqlserver",
    tunnelSlot: "sqlserver",
    adapter: SQLSERVER_ADAPTER,
    browse: (pwOverride, draft) => browse(pwOverride, draft),
    hasLiveState: overview !== null,
    onReset: resetPanel,
    setError,
    passwordInputRef,
    t,
  });

  async function run(text?: string) {
    const q = (text ?? sql).trim();
    if (!q) return;
    setBusy(true);
    setError("");
    try {
      const target = await flow.ensureConnectionTarget();
      setResults(
        await cmd.mssqlExecute({
          host: target.host,
          port: target.port,
          user: tab.mssqlUser.trim(),
          password: tab.mssqlPassword,
          database: tab.mssqlDatabase.trim() || null,
          sql: q,
        }),
      );
    } catch (e) {
      setError(fmt(e));
    } finally {
      setBusy(false);
    }
  }

  async function openTable(schema: string, name: string) {
    setSelected(`${schema}.${name}`);
    const q = `SELECT TOP 100 * FROM [${schema}].[${name}];`;
    setSql(q);
    setBusy(true);
    setError("");
    try {
      const target = await flow.ensureConnectionTarget();
      const base = {
        host: target.host,
        port: target.port,
        user: tab.mssqlUser.trim(),
        password: tab.mssqlPassword,
        database: tab.mssqlDatabase.trim() || null,
      };
      const [cols, rows] = await Promise.all([
        cmd.mssqlColumns({ ...base, schema, table: name }),
        cmd.mssqlExecute({ ...base, sql: q }),
      ]);
      setColumns(cols);
      setResults(rows);
    } catch (e) {
      setError(fmt(e));
    } finally {
      setBusy(false);
    }
  }

  const dialogs = (
    <DbAddCredentialDialog
      open={flow.addOpen}
      onClose={() => flow.setAddOpen(false)}
      kind="sqlserver"
      savedConnectionIndex={flow.savedIndex}
      adopting={flow.adopting}
      tab={tab}
      onSaved={flow.handleCredentialAdded}
      onConnect={flow.handleCredentialConnected}
    />
  );

  // ── Splash (not connected) ────────────────────────────────────────
  if (!overview) {
    const savedRows: DbSplashRowData[] = flow.savedForKind.map((cred) => ({
      id: cred.id,
      name: cred.label || cred.id,
      env: inferEnv(cred.label),
      engine: META.label,
      addr: `${cred.host}:${cred.port}`,
      via: { kind: flow.hasSsh ? "tunnel" : "direct", label: flow.hasSsh ? t("SSH tunnel") : t("direct") },
      user: cred.user,
      authHint: cred.hasPassword ? t("keyring") : undefined,
      stats: <span className="sep">—</span>,
      lastUsed: null,
      status: "unknown",
      tintVar: META.tintVar,
      connectLabel: t("Connect"),
      onConnect: () => flow.activateCredential(cred.id),
      pending: flow.activating === cred.id,
    }));
    return (
      <>
        {error && <div className="db-panel-banner"><div className="status-note mono status-note--error">{error}</div></div>}
        <DbConnectSplash
          kind="sqlserver"
          probeTarget={flow.probeTarget}
          probeState={flow.probeState}
          detected={[]}
          saved={savedRows}
          onAddManual={() => {
            flow.setAdopting(null);
            flow.setAddOpen(true);
          }}
          footerHint={flow.connectingStep ?? undefined}
        />
        {dialogs}
      </>
    );
  }

  // ── Connected shell ───────────────────────────────────────────────
  const dbOptions = (overview.databases ?? []).map((d) => ({ value: d, label: d }));
  return (
    <div className="dbq-panel">
      <PanelHeader
        icon={META.icon}
        title={META.label}
        meta={`${tab.mssqlUser}@${tab.mssqlHost}`}
        actions={
          <>
            <button type="button" className="btn is-ghost is-compact" title={t("Refresh")} onClick={() => void browse()} disabled={busy}>
              <RefreshCw size={11} />
            </button>
            <button type="button" className="btn is-ghost is-compact" title={t("Disconnect")} onClick={() => void flow.disconnect()}>
              <Unplug size={11} />
            </button>
          </>
        }
      />
      <div className="dbq-body">
        <aside className="dbq-side">
          <div className="dbq-side__db">
            <Select
              value={tab.mssqlDatabase}
              onChange={(v) => {
                updateTab(tab.id, { mssqlDatabase: v });
                setColumns(null);
                setSelected("");
                void browse();
              }}
              items={dbOptions}
              mono
            />
          </div>
          <div className="dbq-side__tables">
            {overview.tables.length === 0 && <div className="empty-note">{t("No tables.")}</div>}
            {overview.tables.map((tbl) => {
              const key = `${tbl.schema}.${tbl.name}`;
              return (
                <button
                  key={key}
                  type="button"
                  className={"dbq-table-row mono" + (key === selected ? " is-active" : "")}
                  title={key}
                  onClick={() => void openTable(tbl.schema, tbl.name)}
                >
                  <Table2 size={11} />
                  <span className="dbq-table-row__name">{tbl.name}</span>
                  <span className="dbq-table-row__schema">{tbl.schema}</span>
                </button>
              );
            })}
          </div>
        </aside>
        <section className="dbq-main">
          <div className="dbq-editor">
            <textarea
              className="dbq-editor__ta mono"
              value={sql}
              spellCheck={false}
              onChange={(e) => setSql(e.target.value)}
              onKeyDown={(e) => {
                if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                  e.preventDefault();
                  void run();
                }
              }}
            />
            <div className="dbq-editor__bar">
              <button type="button" className="btn is-primary is-compact" disabled={busy} onClick={() => void run()}>
                <Play size={11} /> {t("Run")}
              </button>
              <span className="dbq-editor__hint">⌘⏎</span>
              <DbAiGenerate
                onResult={setSql}
                schema={() => ({
                  dialect: "SQL Server (T-SQL)",
                  database: tab.mssqlDatabase || undefined,
                  tables: overview.tables.map((tb) => `${tb.schema}.${tb.name}`),
                  currentTable:
                    selected && columns && columns.length > 0
                      ? {
                          name: selected,
                          columns: columns.map((c) => ({ name: c.name, type: c.columnType })),
                        }
                      : undefined,
                })}
              />
              {error && <span className="status-note mono status-note--error">{error}</span>}
            </div>
          </div>
          {columns && (
            <div className="dbq-cols">
              <div className="dbq-cols__head mono">
                <Columns3 size={11} /> {selected}
              </div>
              <div className="data-table-wrap">
                <table className="data-table">
                  <thead>
                    <tr>
                      <th>{t("Column")}</th>
                      <th>{t("Type")}</th>
                      <th>{t("Null")}</th>
                      <th>{t("Key")}</th>
                      <th>{t("Default")}</th>
                      <th>{t("Extra")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {columns.map((c) => (
                      <tr key={c.name}>
                        <td className="mono">{c.name}</td>
                        <td className="mono">{c.columnType}</td>
                        <td>{c.nullable ? "YES" : "NO"}</td>
                        <td>{c.key}</td>
                        <td className="mono">{c.defaultValue}</td>
                        <td>{c.extra}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}
          <div className="dbq-results">
            {results ? (
              <div className="data-table-wrap ux-selectable">
                <table className="data-table">
                  <thead>
                    <tr>{results.columns.map((c, i) => <th key={i}>{c}</th>)}</tr>
                  </thead>
                  <tbody>
                    {results.rows.map((row, i) => (
                      <tr key={i}>{row.map((cell, j) => <td key={j}>{cell}</td>)}</tr>
                    ))}
                  </tbody>
                </table>
                <div className="dbq-results__foot">
                  {t("{n} rows", { n: String(results.rows.length) })}
                  {results.truncated ? ` · ${t("truncated")}` : ""}
                  {` · ${results.elapsedMs} ms`}
                </div>
              </div>
            ) : (
              <div className="empty-note">{t("Run a query to see results.")}</div>
            )}
          </div>
        </section>
      </div>
      {dialogs}
    </div>
  );
}
