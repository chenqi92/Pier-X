import { useState } from "react";
import { Play, Plug, RefreshCw, Table2, Unplug } from "lucide-react";
import type { DbProduct, QueryExecutionResult, TabState } from "../lib/types";
import { effectiveSshTarget, isSshTargetReady } from "../lib/types";
import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import { DB_KIND_META } from "../lib/rightToolMeta";
import PanelHeader from "../components/PanelHeader";

// Oracle / Dameng client over the remote host's CLI (sqlplus / disql).
// No tunnel and no local driver: the vendor CLI runs ON the SSH host and
// connects to the DB from there, so this tool is SSH-only. Reuses the
// `.dbq-*` query-panel chrome. Dameng support is best-effort pending
// validation against a real DM instance (disql CSV parsing).

type Props = { tab: TabState | null; kind: Extract<DbProduct, "oracle" | "dameng"> };

type Form = {
  dbHost: string;
  dbPort: string;
  dbUser: string;
  dbPassword: string;
  dbService: string;
};

type Dialect = {
  hasService: boolean;
  defaultPort: number;
  tablesSql: string;
  preview: (name: string) => string;
};

const DIALECTS: Record<"oracle" | "dameng", Dialect> = {
  oracle: {
    hasService: true,
    defaultPort: 1521,
    tablesSql: "SELECT table_name FROM user_tables ORDER BY table_name",
    preview: (n) => `SELECT * FROM "${n}" FETCH FIRST 100 ROWS ONLY`,
  },
  dameng: {
    hasService: false,
    defaultPort: 5236,
    tablesSql: "SELECT table_name FROM user_tables ORDER BY table_name",
    preview: (n) => `SELECT TOP 100 * FROM "${n}"`,
  },
};

function storageKey(kind: string, host: string) {
  return `pier-x:${kind}:${host || "local"}`;
}

export default function RemoteSqlPanel({ tab, kind }: Props) {
  const { t } = useI18n();
  const fmt = (e: unknown) => localizeError(e, t);
  const meta = DB_KIND_META[kind];
  const dialect = DIALECTS[kind];
  const sshTarget = tab ? effectiveSshTarget(tab) : null;
  const sshReady = isSshTargetReady(sshTarget);
  const sshHost = sshTarget?.host ?? "";

  const [form, setForm] = useState<Form>(() => {
    const def: Form = {
      dbHost: "127.0.0.1",
      dbPort: String(dialect.defaultPort),
      dbUser: kind === "oracle" ? "system" : "SYSDBA",
      dbPassword: "",
      dbService: kind === "oracle" ? "XEPDB1" : "",
    };
    try {
      const raw = localStorage.getItem(storageKey(kind, sshHost));
      if (raw) return { ...def, ...JSON.parse(raw), dbPassword: "" };
    } catch {
      /* ignore malformed cache */
    }
    return def;
  });

  const [connected, setConnected] = useState(false);
  const [tables, setTables] = useState<string[]>([]);
  const [results, setResults] = useState<QueryExecutionResult | null>(null);
  const [selected, setSelected] = useState<string>("");
  const [sql, setSql] = useState(kind === "oracle" ? "SELECT * FROM v$version" : "SELECT * FROM v$version");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const setField = (k: keyof Form, v: string) =>
    setForm((f) => ({ ...f, [k]: v }));

  const persist = (f: Form) => {
    try {
      localStorage.setItem(
        storageKey(kind, sshHost),
        JSON.stringify({
          dbHost: f.dbHost,
          dbPort: f.dbPort,
          dbUser: f.dbUser,
          dbService: f.dbService,
        }),
      );
    } catch {
      /* best-effort */
    }
  };

  async function exec(sqlText: string): Promise<QueryExecutionResult> {
    if (!sshTarget) throw new Error(t("This tab has no SSH context."));
    const base = {
      host: sshTarget.host,
      port: sshTarget.port,
      user: sshTarget.user,
      authMode: sshTarget.authMode,
      password: sshTarget.password,
      keyPath: sshTarget.keyPath,
      savedConnectionIndex: sshTarget.savedConnectionIndex,
      dbHost: form.dbHost.trim(),
      dbPort: Number.parseInt(form.dbPort, 10) || dialect.defaultPort,
      dbUser: form.dbUser.trim(),
      dbPassword: form.dbPassword,
      sql: sqlText,
    };
    return kind === "oracle"
      ? cmd.oracleQuery({ ...base, dbService: form.dbService.trim() })
      : cmd.damengQuery(base);
  }

  async function connect() {
    setBusy(true);
    setError("");
    try {
      const r = await exec(dialect.tablesSql);
      setTables(r.rows.map((row) => row[0]).filter(Boolean));
      setConnected(true);
      persist(form);
    } catch (e) {
      setError(fmt(e));
      setConnected(false);
    } finally {
      setBusy(false);
    }
  }

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

  function disconnect() {
    setConnected(false);
    setTables([]);
    setResults(null);
    setSelected("");
    setError("");
  }

  // SQL*Plus / disql run on the SSH host, so this tool is remote-only.
  if (!sshTarget || !sshReady) {
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

  // ── Connect form ──────────────────────────────────────────────────
  if (!connected) {
    return (
      <div className="dbq-connect">
        <div className="dbq-connect__card">
          <div className="dbq-connect__title mono">
            <meta.icon size={14} /> {meta.label}
          </div>
          <div className="dbq-connect__sub">{t(meta.splashSubtitle)}</div>
          <div className="dbq-form">
            <label className="field">
              <span className="field-label">{t("Host")}</span>
              <input
                className="field-input is-mono"
                value={form.dbHost}
                onChange={(e) => setField("dbHost", e.target.value)}
                placeholder="127.0.0.1"
              />
            </label>
            <label className="field dbq-form__port">
              <span className="field-label">{t("Port")}</span>
              <input
                className="field-input is-mono"
                value={form.dbPort}
                onChange={(e) => setField("dbPort", e.target.value)}
                placeholder={String(dialect.defaultPort)}
              />
            </label>
            {dialect.hasService && (
              <label className="field">
                <span className="field-label">{t("Service / SID")}</span>
                <input
                  className="field-input is-mono"
                  value={form.dbService}
                  onChange={(e) => setField("dbService", e.target.value)}
                  placeholder="XEPDB1"
                />
              </label>
            )}
            <label className="field">
              <span className="field-label">{t("User")}</span>
              <input
                className="field-input is-mono"
                value={form.dbUser}
                onChange={(e) => setField("dbUser", e.target.value)}
              />
            </label>
            <label className="field">
              <span className="field-label">{t("Password")}</span>
              <input
                className="field-input is-mono"
                type="password"
                value={form.dbPassword}
                onChange={(e) => setField("dbPassword", e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void connect();
                }}
              />
            </label>
          </div>
          <button
            type="button"
            className="btn is-primary"
            disabled={busy || !form.dbHost.trim() || !form.dbUser.trim()}
            onClick={() => void connect()}
          >
            <Plug size={13} /> {busy ? t("Connecting…") : t("Connect")}
          </button>
          {error && <div className="status-note mono status-note--error">{error}</div>}
          <div className="dbq-connect__hint">
            {t("Runs {bin} on the SSH host — it must be installed there and able to reach the database.", {
              bin: kind === "oracle" ? "sqlplus" : "disql",
            })}
          </div>
        </div>
      </div>
    );
  }

  // ── Connected shell ───────────────────────────────────────────────
  return (
    <div className="dbq-panel">
      <PanelHeader
        icon={meta.icon}
        title={meta.label}
        meta={`${form.dbUser}@${form.dbHost}`}
        actions={
          <>
            <button
              type="button"
              className="btn is-ghost is-compact"
              title={t("Refresh")}
              onClick={() => void connect()}
              disabled={busy}
            >
              <RefreshCw size={11} />
            </button>
            <button
              type="button"
              className="btn is-ghost is-compact"
              title={t("Disconnect")}
              onClick={disconnect}
            >
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
              <button
                type="button"
                className="btn is-primary is-compact"
                disabled={busy}
                onClick={() => void run()}
              >
                <Play size={11} /> {t("Run")}
              </button>
              <span className="dbq-editor__hint">⌘⏎</span>
              {error && <span className="status-note mono status-note--error">{error}</span>}
            </div>
          </div>
          <div className="dbq-results">
            {results ? (
              <div className="data-table-wrap ux-selectable">
                <table className="data-table">
                  <thead>
                    <tr>
                      {results.columns.map((c, i) => (
                        <th key={i}>{c}</th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {results.rows.map((row, i) => (
                      <tr key={i}>
                        {row.map((cell, j) => (
                          <td key={j}>{cell}</td>
                        ))}
                      </tr>
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
    </div>
  );
}
