import { useMemo, useState } from "react";
import { Columns3, Play, Plug, RefreshCw, Table2, Unplug } from "lucide-react";
import type { QueryExecutionResult, TabState } from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import * as cmd from "../lib/commands";
import type { SqlServerColumnView, SqlServerOverview } from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import { DB_KIND_META } from "../lib/rightToolMeta";
import { ensureTunnelSlot, closeTunnelSlot } from "../lib/sshTunnel";
import { useTabStore } from "../stores/useTabStore";
import Select from "../components/Select";
import PanelHeader from "../components/PanelHeader";

// SQL Server client. Connects through the tab's SSH tunnel (slot
// "sqlserver", like MySQL / PostgreSQL): the form host/port is the
// address as seen FROM the SSH host (default 127.0.0.1:1433); a local
// forward is opened and the TDS connection rides 127.0.0.1:<localPort>.
// TLS is off (tunnel-encrypted). Saved-credential persistence and
// structure/grid editing are tracked follow-ups.

type Props = { tab: TabState | null };

type Form = {
  host: string;
  port: string;
  user: string;
  password: string;
  database: string;
};

const META = DB_KIND_META.sqlserver;

function storageKey(host: string) {
  return `pier-x:mssql:${host || "local"}`;
}

export default function SqlServerPanel({ tab }: Props) {
  const { t } = useI18n();
  const fmt = (e: unknown) => localizeError(e, t);
  const sshHost = tab ? effectiveSshTarget(tab)?.host ?? "" : "";

  const [form, setForm] = useState<Form>(() => {
    const def: Form = {
      host: "127.0.0.1",
      port: "1433",
      user: "sa",
      password: "",
      database: "",
    };
    try {
      const raw = localStorage.getItem(storageKey(sshHost));
      if (raw) return { ...def, ...JSON.parse(raw), password: "" };
    } catch {
      /* ignore malformed cache */
    }
    return def;
  });

  const [overview, setOverview] = useState<SqlServerOverview | null>(null);
  const [results, setResults] = useState<QueryExecutionResult | null>(null);
  const [columns, setColumns] = useState<SqlServerColumnView[] | null>(null);
  const [selected, setSelected] = useState<string>("");
  const [sql, setSql] = useState("SELECT @@VERSION;");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const persist = (f: Form) => {
    try {
      localStorage.setItem(
        storageKey(sshHost),
        JSON.stringify({ host: f.host, port: f.port, user: f.user, database: f.database }),
      );
    } catch {
      /* best-effort */
    }
  };

  const setField = (k: keyof Form, v: string) =>
    setForm((f) => ({ ...f, [k]: v }));

  // Resolve the actual TDS endpoint: through the SSH tunnel when the tab
  // has an SSH context, else direct to the entered address.
  async function resolveTarget(): Promise<{ host: string; port: number }> {
    const remotePort = Number.parseInt(form.port, 10) || 1433;
    const remoteHost = form.host.trim() || "127.0.0.1";
    if (tab && effectiveSshTarget(tab)) {
      const info = await ensureTunnelSlot({
        tab,
        slot: "sqlserver",
        remoteHost,
        remotePort,
        updateTab: useTabStore.getState().updateTab,
      });
      return { host: "127.0.0.1", port: info.localPort };
    }
    return { host: remoteHost, port: remotePort };
  }

  const auth = () => ({ user: form.user.trim(), password: form.password });

  async function connect(database?: string) {
    setBusy(true);
    setError("");
    try {
      const tgt = await resolveTarget();
      const ov = await cmd.mssqlOverview({
        ...tgt,
        ...auth(),
        database: database ?? (form.database.trim() || null),
      });
      setOverview(ov);
      setForm((f) => {
        const nf = { ...f, database: ov.currentDatabase };
        persist(nf);
        return nf;
      });
    } catch (e) {
      setError(fmt(e));
      setOverview(null);
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
      const tgt = await resolveTarget();
      setResults(
        await cmd.mssqlExecute({ ...tgt, ...auth(), database: form.database.trim() || null, sql: q }),
      );
    } catch (e) {
      setError(fmt(e));
    } finally {
      setBusy(false);
    }
  }

  async function openTable(schema: string, name: string) {
    const key = `${schema}.${name}`;
    setSelected(key);
    const q = `SELECT TOP 100 * FROM [${schema}].[${name}];`;
    setSql(q);
    setBusy(true);
    setError("");
    try {
      const tgt = await resolveTarget();
      const db = form.database.trim() || null;
      const [cols, rows] = await Promise.all([
        cmd.mssqlColumns({ ...tgt, ...auth(), database: db, schema, table: name }),
        cmd.mssqlExecute({ ...tgt, ...auth(), database: db, sql: q }),
      ]);
      setColumns(cols);
      setResults(rows);
    } catch (e) {
      setError(fmt(e));
    } finally {
      setBusy(false);
    }
  }

  async function disconnect() {
    if (tab) {
      await closeTunnelSlot(tab, "sqlserver", useTabStore.getState().updateTab).catch(() => {});
    }
    setOverview(null);
    setResults(null);
    setColumns(null);
    setSelected("");
    setError("");
  }

  const dbOptions = useMemo(
    () => (overview?.databases ?? []).map((d) => ({ value: d, label: d })),
    [overview],
  );

  // ── Connect form ──────────────────────────────────────────────────
  if (!overview) {
    return (
      <div className="dbq-connect">
        <div className="dbq-connect__card">
          <div className="dbq-connect__title mono">
            <META.icon size={14} /> {META.label}
          </div>
          <div className="dbq-connect__sub">{t(META.splashSubtitle)}</div>
          <div className="dbq-form">
            <label className="field">
              <span className="field-label">{t("Host")}</span>
              <input
                className="field-input is-mono"
                value={form.host}
                onChange={(e) => setField("host", e.target.value)}
                placeholder="127.0.0.1"
              />
            </label>
            <label className="field dbq-form__port">
              <span className="field-label">{t("Port")}</span>
              <input
                className="field-input is-mono"
                value={form.port}
                onChange={(e) => setField("port", e.target.value)}
                placeholder="1433"
              />
            </label>
            <label className="field">
              <span className="field-label">{t("User")}</span>
              <input
                className="field-input is-mono"
                value={form.user}
                onChange={(e) => setField("user", e.target.value)}
                placeholder="sa"
              />
            </label>
            <label className="field">
              <span className="field-label">{t("Password")}</span>
              <input
                className="field-input is-mono"
                type="password"
                value={form.password}
                onChange={(e) => setField("password", e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void connect();
                }}
              />
            </label>
            <label className="field">
              <span className="field-label">{t("Database")}</span>
              <input
                className="field-input is-mono"
                value={form.database}
                onChange={(e) => setField("database", e.target.value)}
                placeholder={t("(default)")}
              />
            </label>
          </div>
          <button
            type="button"
            className="btn is-primary"
            disabled={busy || !form.host.trim() || !form.user.trim()}
            onClick={() => void connect()}
          >
            <Plug size={13} /> {busy ? t("Connecting…") : t("Connect")}
          </button>
          {error && <div className="status-note mono status-note--error">{error}</div>}
          <div className="dbq-connect__hint">
            {tab && effectiveSshTarget(tab)
              ? t("Connects via the SSH tunnel — host/port are as seen from the SSH host.")
              : t("Connects directly to the address above (TLS off).")}
          </div>
        </div>
      </div>
    );
  }

  // ── Connected shell ───────────────────────────────────────────────
  return (
    <div className="dbq-panel">
      <PanelHeader
        icon={META.icon}
        title={META.label}
        meta={`${form.user}@${form.host}`}
        actions={
          <>
            <button
              type="button"
              className="btn is-ghost is-compact"
              title={t("Refresh")}
              onClick={() => void connect(form.database)}
              disabled={busy}
            >
              <RefreshCw size={11} />
            </button>
            <button
              type="button"
              className="btn is-ghost is-compact"
              title={t("Disconnect")}
              onClick={() => void disconnect()}
            >
              <Unplug size={11} />
            </button>
          </>
        }
      />
      <div className="dbq-body">
        <aside className="dbq-side">
          <div className="dbq-side__db">
            <Select
              value={form.database}
              onChange={(v) => {
                setField("database", v);
                setColumns(null);
                setSelected("");
                void connect(v);
              }}
              items={dbOptions}
              mono
            />
          </div>
          <div className="dbq-side__tables">
            {overview.tables.length === 0 && (
              <div className="empty-note">{t("No tables.")}</div>
            )}
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
