import { useMemo, useState } from "react";
import { Play, Plug, Activity, RefreshCw, Unplug } from "lucide-react";
import type { QueryExecutionResult, TabState } from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import * as cmd from "../lib/commands";
import type { InfluxOverview } from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import { DB_KIND_META } from "../lib/rightToolMeta";
import { ensureTunnelSlot, closeTunnelSlot } from "../lib/sshTunnel";
import { useTabStore } from "../stores/useTabStore";
import Select from "../components/Select";
import PanelHeader from "../components/PanelHeader";

// InfluxDB client. Time-series store reached over its HTTP API
// (InfluxQL on `/query`), so the "tables" are measurements and queries
// are InfluxQL. Connects through the tab's SSH tunnel (slot "influx")
// exactly like the SQL clients — the form address is as seen from the
// SSH host (default 127.0.0.1:8086). Auth via a 2.x token or 1.x
// user/password, both optional. Reuses the `.dbq-*` query-panel chrome.

type Props = { tab: TabState | null };

type Form = {
  host: string;
  port: string;
  database: string;
  user: string;
  password: string;
  token: string;
};

const META = DB_KIND_META.influx;

function storageKey(host: string) {
  return `pier-x:influx:${host || "local"}`;
}

export default function InfluxPanel({ tab }: Props) {
  const { t } = useI18n();
  const fmt = (e: unknown) => localizeError(e, t);
  const sshHost = tab ? effectiveSshTarget(tab)?.host ?? "" : "";

  const [form, setForm] = useState<Form>(() => {
    const def: Form = {
      host: "127.0.0.1",
      port: "8086",
      database: "",
      user: "",
      password: "",
      token: "",
    };
    try {
      const raw = localStorage.getItem(storageKey(sshHost));
      if (raw) return { ...def, ...JSON.parse(raw), password: "", token: "" };
    } catch {
      /* ignore malformed cache */
    }
    return def;
  });

  const [overview, setOverview] = useState<InfluxOverview | null>(null);
  const [results, setResults] = useState<QueryExecutionResult | null>(null);
  const [selected, setSelected] = useState<string>("");
  const [sql, setSql] = useState("SHOW MEASUREMENTS");
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

  async function resolveTarget(): Promise<{ host: string; port: number }> {
    const remotePort = Number.parseInt(form.port, 10) || 8086;
    const remoteHost = form.host.trim() || "127.0.0.1";
    if (tab && effectiveSshTarget(tab)) {
      const info = await ensureTunnelSlot({
        tab,
        slot: "influx",
        remoteHost,
        remotePort,
        updateTab: useTabStore.getState().updateTab,
      });
      return { host: "127.0.0.1", port: info.localPort };
    }
    return { host: remoteHost, port: remotePort };
  }

  const auth = () => ({
    user: form.user.trim(),
    password: form.password,
    token: form.token.trim(),
  });

  async function connect(database?: string) {
    setBusy(true);
    setError("");
    try {
      const tgt = await resolveTarget();
      const ov = await cmd.influxOverview({
        ...tgt,
        ...auth(),
        database: database ?? (form.database.trim() || null),
      });
      setOverview(ov);
      setForm((f) => {
        const nf = { ...f, database: ov.currentDatabase || f.database };
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
        await cmd.influxQuery({ ...tgt, ...auth(), database: form.database.trim() || null, query: q }),
      );
    } catch (e) {
      setError(fmt(e));
    } finally {
      setBusy(false);
    }
  }

  function openMeasurement(name: string) {
    setSelected(name);
    const q = `SELECT * FROM "${name}" LIMIT 100`;
    setSql(q);
    void run(q);
  }

  async function disconnect() {
    if (tab) {
      await closeTunnelSlot(tab, "influx", useTabStore.getState().updateTab).catch(() => {});
    }
    setOverview(null);
    setResults(null);
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
                placeholder="8086"
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
            <label className="field">
              <span className="field-label">{t("Token")}</span>
              <input
                className="field-input is-mono"
                type="password"
                value={form.token}
                onChange={(e) => setField("token", e.target.value)}
                placeholder={t("(2.x API token, optional)")}
              />
            </label>
            <label className="field">
              <span className="field-label">{t("User")}</span>
              <input
                className="field-input is-mono"
                value={form.user}
                onChange={(e) => setField("user", e.target.value)}
                placeholder={t("(1.x, optional)")}
              />
            </label>
            <label className="field dbq-form__port">
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
          </div>
          <button
            type="button"
            className="btn is-primary"
            disabled={busy || !form.host.trim()}
            onClick={() => void connect()}
          >
            <Plug size={13} /> {busy ? t("Connecting…") : t("Connect")}
          </button>
          {error && <div className="status-note mono status-note--error">{error}</div>}
          <div className="dbq-connect__hint">
            {tab && effectiveSshTarget(tab)
              ? t("Connects via the SSH tunnel — host/port are as seen from the SSH host.")
              : t("Connects directly to the address above.")}
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
        meta={form.host}
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
                setSelected("");
                void connect(v);
              }}
              items={dbOptions}
              mono
            />
          </div>
          <div className="dbq-side__tables">
            {overview.measurements.length === 0 && (
              <div className="empty-note">{t("No measurements.")}</div>
            )}
            {overview.measurements.map((m) => (
              <button
                key={m}
                type="button"
                className={"dbq-table-row mono" + (m === selected ? " is-active" : "")}
                title={m}
                onClick={() => openMeasurement(m)}
              >
                <Activity size={11} />
                <span className="dbq-table-row__name">{m}</span>
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
