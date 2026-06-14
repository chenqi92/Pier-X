import { useRef, useState } from "react";
import { Activity, Play, RefreshCw, Unplug } from "lucide-react";
import type { QueryExecutionResult, TabState } from "../lib/types";
import * as cmd from "../lib/commands";
import type { InfluxOverview } from "../lib/commands";
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

// InfluxDB client (InfluxQL over HTTP), aligned with the shared credential
// flow: saved credentials in the keyring + connect splash + SSH tunnel
// (slot "influx"). The keyring secret is the 2.x token when no user is
// set, else the 1.x password — decided per query.

type Props = { tab: TabState | null };

const META = DB_KIND_META.influx;

const INFLUX_ADAPTER: DbCredentialFieldAdapter = {
  readHost: (t) => t.influxHost,
  readPort: (t) => t.influxPort,
  readUser: (t) => t.influxUser,
  readPassword: (t) => t.influxPassword,
  readActiveCredId: (t) => t.influxActiveCredentialId,
  readTunnelId: (t) => t.influxTunnelId,
  readTunnelPort: (t) => t.influxTunnelPort,
  patchFromCred: (cred) => ({
    influxActiveCredentialId: cred.id,
    influxHost: cred.host,
    influxPort: cred.port,
    influxUser: cred.user,
    influxPassword: "",
    influxDatabase: cred.database ?? "",
    influxTunnelId: null,
    influxTunnelPort: null,
  }),
  patchFromSaved: (cred) => ({
    influxActiveCredentialId: cred.id,
    influxHost: cred.host,
    influxPort: cred.port,
    influxUser: cred.user,
    influxDatabase: cred.database ?? "",
    influxTunnelId: null,
    influxTunnelPort: null,
  }),
  patchFromDraft: (draft) => ({
    influxActiveCredentialId: null,
    influxHost: draft.host,
    influxPort: draft.port,
    influxUser: draft.user,
    influxPassword: draft.password,
    influxDatabase: draft.database ?? "",
    influxTunnelId: null,
    influxTunnelPort: null,
  }),
  patchPassword: (password) => ({ influxPassword: password }),
  patchPasswordAfterRotate: (password) => ({ influxPassword: password }),
};

export default function InfluxPanel({ tab }: Props) {
  const { t } = useI18n();
  if (!tab) {
    return (
      <div className="panel-section panel-section--empty">
        <div className="status-note mono">{t("Open an SSH tab to connect to a database.")}</div>
      </div>
    );
  }
  return <InfluxBody tab={tab} />;
}

function InfluxBody({ tab }: { tab: TabState }) {
  const { t } = useI18n();
  const fmt = (e: unknown) => localizeError(e, t);
  const updateTab = useTabStore((s) => s.updateTab);
  const passwordInputRef = useRef<HTMLInputElement | null>(null);

  const [overview, setOverview] = useState<InfluxOverview | null>(null);
  const [results, setResults] = useState<QueryExecutionResult | null>(null);
  const [selected, setSelected] = useState("");
  const [sql, setSql] = useState("SHOW MEASUREMENTS");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  function resetPanel() {
    setOverview(null);
    setResults(null);
    setSelected("");
    setError("");
  }

  // The keyring secret is a token when no user is set (2.x), else a
  // 1.x password.
  function authFor(secret: string, user: string) {
    return user.trim()
      ? { user: user.trim(), password: secret, token: "" }
      : { user: "", password: "", token: secret };
  }

  async function browse(passwordOverride?: string, draft?: DbConnectionDraft) {
    setBusy(true);
    setError("");
    try {
      const target = await flow.ensureConnectionTarget(false, draft);
      const secret = passwordOverride !== undefined ? passwordOverride : tab.influxPassword;
      const user = draft?.user ?? tab.influxUser;
      const database = draft ? draft.database ?? "" : tab.influxDatabase;
      const ov = await cmd.influxOverview({
        host: target.host,
        port: target.port,
        database: database.trim() || null,
        ...authFor(secret, user),
      });
      setOverview(ov);
      if (ov.currentDatabase && ov.currentDatabase !== tab.influxDatabase) {
        updateTab(tab.id, { influxDatabase: ov.currentDatabase });
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
    kind: "influx",
    tunnelSlot: "influx",
    adapter: INFLUX_ADAPTER,
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
        await cmd.influxQuery({
          host: target.host,
          port: target.port,
          database: tab.influxDatabase.trim() || null,
          ...authFor(tab.influxPassword, tab.influxUser),
          query: q,
        }),
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

  const dialogs = (
    <DbAddCredentialDialog
      open={flow.addOpen}
      onClose={() => flow.setAddOpen(false)}
      kind="influx"
      savedConnectionIndex={flow.savedIndex}
      adopting={flow.adopting}
      tab={tab}
      onSaved={flow.handleCredentialAdded}
      onConnect={flow.handleCredentialConnected}
    />
  );

  if (!overview) {
    const savedRows: DbSplashRowData[] = flow.savedForKind.map((cred) => ({
      id: cred.id,
      name: cred.label || cred.id,
      env: inferEnv(cred.label),
      engine: META.label,
      addr: `${cred.host}:${cred.port}`,
      via: { kind: flow.hasSsh ? "tunnel" : "direct", label: flow.hasSsh ? t("SSH tunnel") : t("direct") },
      user: cred.user || "—",
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
          kind="influx"
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

  const dbOptions = (overview.databases ?? []).map((d) => ({ value: d, label: d }));
  return (
    <div className="dbq-panel">
      <PanelHeader
        icon={META.icon}
        title={META.label}
        meta={tab.influxHost}
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
              value={tab.influxDatabase}
              onChange={(v) => {
                updateTab(tab.id, { influxDatabase: v });
                setSelected("");
                void browse();
              }}
              items={dbOptions}
              mono
            />
          </div>
          <div className="dbq-side__tables">
            {overview.measurements.length === 0 && <div className="empty-note">{t("No measurements.")}</div>}
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
              <button type="button" className="btn is-primary is-compact" disabled={busy} onClick={() => void run()}>
                <Play size={11} /> {t("Run")}
              </button>
              <span className="dbq-editor__hint">⌘⏎</span>
              <DbAiGenerate
                onResult={setSql}
                schema={() => ({
                  dialect: "InfluxQL (InfluxDB)",
                  database: tab.influxDatabase || undefined,
                  tables: overview.measurements,
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
