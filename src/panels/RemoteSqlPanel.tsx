import { useRef, useState } from "react";
import { Lock, Play, RefreshCw, Table2, Unlock, Unplug } from "lucide-react";
import type { DbProduct, QueryExecutionResult, TabState } from "../lib/types";
import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import { DB_KIND_META } from "../lib/rightToolMeta";
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
import DbAiGenerate from "../components/db/DbAiGenerate";

// Oracle / Dameng client over the remote host's CLI (sqlplus / disql).
// Aligned with the shared credential flow (saved creds + keyring + connect
// splash) but with NO tunnel — the vendor CLI runs ON the SSH host, so the
// flow passes `tunnelSlot: null` and the panel hands the SSH params plus
// the saved DB host/port straight to the query command. SSH-only.

type RemoteKind = Extract<DbProduct, "oracle" | "dameng">;
type Props = { tab: TabState | null; kind: RemoteKind };

type Dialect = {
  hasService: boolean;
  tablesSql: string;
  preview: (name: string) => string;
};

const DIALECTS: Record<RemoteKind, Dialect> = {
  oracle: {
    hasService: true,
    tablesSql: "SELECT table_name FROM user_tables ORDER BY table_name",
    preview: (n) => `SELECT * FROM "${n}" FETCH FIRST 100 ROWS ONLY`,
  },
  dameng: {
    hasService: false,
    tablesSql: "SELECT table_name FROM user_tables ORDER BY table_name",
    preview: (n) => `SELECT TOP 100 * FROM "${n}"`,
  },
};

function makeAdapter(kind: RemoteKind): DbCredentialFieldAdapter {
  if (kind === "oracle") {
    return {
      readHost: (t) => t.oracleHost,
      readPort: (t) => t.oraclePort,
      readUser: (t) => t.oracleUser,
      readPassword: (t) => t.oraclePassword,
      readActiveCredId: (t) => t.oracleActiveCredentialId,
      readTunnelId: () => null,
      readTunnelPort: () => null,
      patchFromCred: (cred) => ({
        oracleActiveCredentialId: cred.id,
        oracleHost: cred.host,
        oraclePort: cred.port,
        oracleUser: cred.user,
        oraclePassword: "",
        oracleService: cred.database ?? "",
      }),
      patchFromSaved: (cred) => ({
        oracleActiveCredentialId: cred.id,
        oracleHost: cred.host,
        oraclePort: cred.port,
        oracleUser: cred.user,
        oracleService: cred.database ?? "",
      }),
      patchFromDraft: (draft) => ({
        oracleActiveCredentialId: null,
        oracleHost: draft.host,
        oraclePort: draft.port,
        oracleUser: draft.user,
        oraclePassword: draft.password,
        oracleService: draft.database ?? "",
      }),
      patchPassword: (password) => ({ oraclePassword: password }),
      patchPasswordAfterRotate: (password) => ({ oraclePassword: password }),
    };
  }
  return {
    readHost: (t) => t.damengHost,
    readPort: (t) => t.damengPort,
    readUser: (t) => t.damengUser,
    readPassword: (t) => t.damengPassword,
    readActiveCredId: (t) => t.damengActiveCredentialId,
    readTunnelId: () => null,
    readTunnelPort: () => null,
    patchFromCred: (cred) => ({
      damengActiveCredentialId: cred.id,
      damengHost: cred.host,
      damengPort: cred.port,
      damengUser: cred.user,
      damengPassword: "",
    }),
    patchFromSaved: (cred) => ({
      damengActiveCredentialId: cred.id,
      damengHost: cred.host,
      damengPort: cred.port,
      damengUser: cred.user,
    }),
    patchFromDraft: (draft) => ({
      damengActiveCredentialId: null,
      damengHost: draft.host,
      damengPort: draft.port,
      damengUser: draft.user,
      damengPassword: draft.password,
    }),
    patchPassword: (password) => ({ damengPassword: password }),
    patchPasswordAfterRotate: (password) => ({ damengPassword: password }),
  };
}

export default function RemoteSqlPanel({ tab, kind }: Props) {
  const { t } = useI18n();
  if (!tab) {
    return (
      <div className="panel-section panel-section--empty">
        <div className="status-note mono">{t("Open an SSH tab to connect to a database.")}</div>
      </div>
    );
  }
  return <RemoteSqlBody key={kind} tab={tab} kind={kind} />;
}

function RemoteSqlBody({ tab, kind }: { tab: TabState; kind: RemoteKind }) {
  const { t } = useI18n();
  const fmt = (e: unknown) => localizeError(e, t);
  const meta = DB_KIND_META[kind];
  const dialect = DIALECTS[kind];
  const adapter = useRef(makeAdapter(kind)).current;
  const passwordInputRef = useRef<HTMLInputElement | null>(null);

  const [connected, setConnected] = useState(false);
  const [tables, setTables] = useState<string[]>([]);
  const [results, setResults] = useState<QueryExecutionResult | null>(null);
  const [selected, setSelected] = useState("");
  const [sql, setSql] = useState("SELECT * FROM v$version");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  // Default read-only write-lock, matching the other DB panels: the backend
  // rejects non-read statements unless the lock is released.
  const [readOnly, setReadOnly] = useState(true);

  function resetPanel() {
    setConnected(false);
    setTables([]);
    setResults(null);
    setSelected("");
    setError("");
  }

  function dbAddr() {
    return { host: adapter.readHost(tab), port: adapter.readPort(tab) };
  }

  async function exec(sqlText: string, passwordOverride?: string, draft?: DbConnectionDraft) {
    const ssh = flow.sshTarget;
    if (!ssh) throw new Error(t("This tab has no SSH context."));
    const target = await flow.ensureConnectionTarget(false, draft);
    const pw = passwordOverride !== undefined ? passwordOverride : adapter.readPassword(tab);
    const user = draft?.user ?? adapter.readUser(tab);
    const sshBase = {
      host: ssh.host,
      port: ssh.port,
      user: ssh.user,
      authMode: ssh.authMode,
      password: ssh.password,
      keyPath: ssh.keyPath,
      savedConnectionIndex: ssh.savedConnectionIndex,
      dbHost: target.host,
      dbPort: target.port,
      dbUser: user.trim(),
      dbPassword: pw,
      sql: sqlText,
      readOnly,
    };
    if (kind === "oracle") {
      const service = draft ? draft.database ?? "" : tab.oracleService;
      return cmd.oracleQuery({ ...sshBase, dbService: service.trim() });
    }
    return cmd.damengQuery(sshBase);
  }

  // Connecting = running the table list against the (just-activated) cred.
  async function browse(passwordOverride?: string, draft?: DbConnectionDraft) {
    setBusy(true);
    setError("");
    try {
      const r = await exec(dialect.tablesSql, passwordOverride, draft);
      setTables(r.rows.map((row) => row[0]).filter(Boolean));
      setConnected(true);
    } catch (e) {
      setError(fmt(e));
      setConnected(false);
    } finally {
      setBusy(false);
    }
  }

  const flow = useDbCredentialFlow({
    tab,
    kind,
    tunnelSlot: null,
    adapter,
    browse: (pwOverride, draft) => browse(pwOverride, draft),
    hasLiveState: connected,
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
      setResults(await exec(q));
    } catch (e) {
      setError(fmt(e));
    } finally {
      setBusy(false);
    }
  }

  function openTable(name: string) {
    setSelected(name);
    const q = dialect.preview(name);
    setSql(q);
    void run(q);
  }

  const dialogs = (
    <DbAddCredentialDialog
      open={flow.addOpen}
      onClose={() => flow.setAddOpen(false)}
      kind={kind}
      savedConnectionIndex={flow.savedIndex}
      adopting={flow.adopting}
      tab={tab}
      onSaved={flow.handleCredentialAdded}
      onConnect={flow.handleCredentialConnected}
    />
  );

  if (!flow.hasSsh) {
    return (
      <div className="panel-section panel-section--empty">
        <div className="panel-section__title mono">
          <meta.icon size={12} /> {meta.label}
        </div>
        <div className="status-note mono">
          {t("Open an SSH tab — {label} runs via the remote host's CLI.", { label: meta.label })}
        </div>
      </div>
    );
  }

  if (!connected) {
    const savedRows: DbSplashRowData[] = flow.savedForKind.map((cred) => ({
      id: cred.id,
      name: cred.label || cred.id,
      env: inferEnv(cred.label),
      engine: meta.label,
      addr: `${cred.host}:${cred.port}`,
      via: { kind: "remote", label: kind === "oracle" ? "sqlplus" : "disql" },
      user: cred.user,
      authHint: cred.hasPassword ? t("keyring") : undefined,
      stats: cred.database ? <span>{cred.database}</span> : <span className="sep">—</span>,
      lastUsed: null,
      status: "unknown",
      tintVar: meta.tintVar,
      connectLabel: t("Connect"),
      onConnect: () => flow.activateCredential(cred.id),
      pending: flow.activating === cred.id,
    }));
    return (
      <>
        {error && <div className="db-panel-banner"><div className="status-note mono status-note--error">{error}</div></div>}
        <DbConnectSplash
          kind={kind}
          probeTarget={flow.probeTarget}
          probeState="idle"
          detected={[]}
          saved={savedRows}
          onAddManual={() => {
            flow.setAdopting(null);
            flow.setAddOpen(true);
          }}
          description={t("Runs {bin} on the SSH host — it must be installed there and able to reach the database.", {
            bin: kind === "oracle" ? "sqlplus" : "disql",
          })}
          footerHint={flow.connectingStep ?? undefined}
        />
        {dialogs}
      </>
    );
  }

  const addr = dbAddr();
  return (
    <div className="dbq-panel">
      <PanelHeader
        icon={meta.icon}
        title={meta.label}
        meta={`${adapter.readUser(tab)}@${addr.host}`}
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
          <div className="dbq-side__tables">
            {tables.length === 0 && <div className="empty-note">{t("No tables.")}</div>}
            {tables.map((name) => (
              <button
                key={name}
                type="button"
                className={"dbq-table-row mono" + (name === selected ? " is-active" : "")}
                title={name}
                onClick={() => openTable(name)}
              >
                <Table2 size={11} />
                <span className="dbq-table-row__name">{name}</span>
              </button>
            ))}
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
              <button
                type="button"
                className={"btn is-ghost is-compact" + (readOnly ? "" : " is-active")}
                title={readOnly ? t("Read-only — click to allow writes") : t("Writes enabled — click to lock")}
                onClick={() => setReadOnly((v) => !v)}
              >
                {readOnly ? <Lock size={11} /> : <Unlock size={11} />}
                {readOnly ? t("read-only") : t("editable")}
              </button>
              <span className="dbq-editor__hint">⌘⏎</span>
              <DbAiGenerate
                onResult={setSql}
                schema={() => ({
                  dialect: kind === "oracle" ? "Oracle SQL" : "Dameng SQL (DM8)",
                  tables,
                })}
              />
              {error && <span className="status-note mono status-note--error">{error}</span>}
            </div>
          </div>
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
