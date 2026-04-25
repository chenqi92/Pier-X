import { useEffect, useRef, useState } from "react";

import DbAddCredentialDialog from "../components/DbAddCredentialDialog";
import DbPasswordUpdateDialog from "../components/DbPasswordUpdateDialog";
import DbTunnelChip from "../components/DbTunnelChip";
import DismissibleNote from "../components/DismissibleNote";
import DbConnectSplash from "../components/db/DbConnectSplash";
import DbConnectedShell, { type DbConnectedTab } from "../components/db/DbConnectedShell";
import type { DbHeaderInstance } from "../components/db/DbHeaderPicker";
import DbConfigView, { type DbConfigRow } from "../components/db/DbConfigView";
import DbResultGrid from "../components/db/DbResultGrid";
import { type DbSchemaDatabase } from "../components/db/DbSchemaTree";
import DbStructureView from "../components/db/DbStructureView";
import DbSqlEditor from "../components/db/DbSqlEditor";
import type { DbSplashRowData } from "../components/db/DbSplashRow";
import { inferEnv } from "../components/db/dbTheme";
import {
  useDbCredentialFlow,
  type DbCredentialFieldAdapter,
} from "../components/db/useDbCredentialFlow";
import { useDbSqlTabs } from "../components/db/useDbSqlTabs";
import {
  gridColumnsFromPostgres,
  mutationToSql,
  qualifyTable,
  type DbMutation,
} from "../components/db/dbColumnRules";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import { writeClipboardText } from "../lib/clipboard";
import * as cmd from "../lib/commands";
import { isReadOnlySql, queryResultToTsv } from "../lib/commands";
import type {
  PostgresBrowserState,
  QueryExecutionResult,
  TabState,
} from "../lib/types";
import { useTabStore } from "../stores/useTabStore";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";

type Props = { tab: TabState };

// PostgreSQL numeric types — parallels the MySQL regex in MySqlPanel.
const NUMERIC_TYPE_RE = /^(smallint|integer|bigint|numeric|decimal|real|double|money|serial|bigserial)/i;

const POSTGRES_ADAPTER: DbCredentialFieldAdapter = {
  readHost: (t) => t.pgHost,
  readPort: (t) => t.pgPort,
  readUser: (t) => t.pgUser,
  readPassword: (t) => t.pgPassword,
  readActiveCredId: (t) => t.pgActiveCredentialId,
  readTunnelId: (t) => t.pgTunnelId,
  readTunnelPort: (t) => t.pgTunnelPort,
  patchFromCred: (cred) => ({
    pgActiveCredentialId: cred.id,
    pgHost: cred.host,
    pgPort: cred.port,
    pgUser: cred.user,
    pgPassword: "",
    pgDatabase: cred.database ?? "",
    pgTunnelId: null,
    pgTunnelPort: null,
  }),
  patchFromSaved: (cred) => ({
    pgActiveCredentialId: cred.id,
    pgHost: cred.host,
    pgPort: cred.port,
    pgUser: cred.user,
    pgDatabase: cred.database ?? "",
    pgTunnelId: null,
    pgTunnelPort: null,
  }),
  patchPassword: (password) => ({ pgPassword: password }),
  patchPasswordAfterRotate: (password) => ({ pgPassword: password }),
};

export default function PostgresPanel(props: Props) {
  const ready = useDeferredMount();
  const variant = props.tab.pgActiveCredentialId ? "grid" : "splash";
  return (
    <div className="panel-stage">
      {ready ? <PostgresPanelBody {...props} /> : <PanelSkeleton variant={variant} rows={8} />}
    </div>
  );
}

function PostgresPanelBody({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const updateTab = useTabStore((s) => s.updateTab);

  // PostgreSQL tracks its own `schema` — the current active schema on
  // the server. Local-only (mirrors the returned `state.schemaName`).
  const [schema, setSchema] = useState("public");

  const [state, setState] = useState<PostgresBrowserState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [readOnly, setReadOnly] = useState(true);
  const [writeConfirm, setWriteConfirm] = useState("");
  const [queryResult, setQueryResult] = useState<QueryExecutionResult | null>(null);
  const [queryBusy, setQueryBusy] = useState(false);
  const [queryError, setQueryError] = useState("");
  const [notice, setNotice] = useState("");

  const [connectedTab, setConnectedTab] = useState<DbConnectedTab>("data");

  const sqlTabs = useDbSqlTabs({ initialSql: "SELECT version();", initialName: t("query") });
  const sql = sqlTabs.sql;
  const setSql = sqlTabs.setSql;

  const passwordInputRef = useRef<HTMLInputElement | null>(null);

  function resetPanel() {
    setState(null);
    setError("");
    setQueryResult(null);
    setQueryError("");
    setNotice("");
    setReadOnly(true);
    setWriteConfirm("");
  }

  async function browse(passwordOverride?: string, nextTable?: string, nextDb?: string, nextSchema?: string) {
    setBusy(true);
    setError("");
    try {
      const target = await flow.ensureConnectionTarget();
      const pw = passwordOverride !== undefined ? passwordOverride : tab.pgPassword;
      const s = await cmd.postgresBrowse({
        host: target.host,
        port: target.port,
        user: tab.pgUser.trim(),
        password: pw,
        database: (nextDb ?? tab.pgDatabase).trim() || null,
        schema: (nextSchema ?? schema).trim() || null,
        table: (nextTable ?? state?.tableName ?? "").trim() || null,
      });
      setState(s);
      setSchema(s.schemaName);
      if (s.databaseName !== tab.pgDatabase) {
        updateTab(tab.id, { pgDatabase: s.databaseName });
      }
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  const flow = useDbCredentialFlow({
    tab,
    kind: "postgres",
    tunnelSlot: "postgres",
    adapter: POSTGRES_ADAPTER,
    browse: (pwOverride) => browse(pwOverride),
    hasLiveState: state !== null,
    onReset: resetPanel,
    setError,
    passwordInputRef,
    t,
  });

  async function runQuery() {
    setQueryBusy(true);
    setQueryError("");
    setNotice("");
    const needsWrite = sql.trim() !== "" && !isReadOnlySql(sql);
    try {
      const target = await flow.ensureConnectionTarget();
      const r = await cmd.postgresExecute({
        host: target.host,
        port: target.port,
        user: tab.pgUser.trim(),
        password: tab.pgPassword,
        database: tab.pgDatabase.trim() || null,
        sql,
      });
      setQueryResult(r);
      setNotice(t("{elapsed} ms", { elapsed: r.elapsedMs }));
      sqlTabs.pushHistory({
        sql,
        at: t("just now"),
        rows: r.rows?.length ?? null,
        ms: r.elapsedMs,
        write: needsWrite,
      });
      sqlTabs.markActiveSaved();
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

  const needsWrite = sql.trim() !== "" && !isReadOnlySql(sql);
  const hostReady = tab.pgHost.trim() !== "" && tab.pgUser.trim() !== "" && tab.pgPort > 0;
  const canRun =
    hostReady &&
    sql.trim() !== "" &&
    !queryBusy &&
    (!needsWrite || (!readOnly && writeConfirm.trim().toUpperCase() === "WRITE"));

  // ── Splash rows ────────────────────────────────────────────
  const viaLabel = flow.sshTarget ? `${flow.sshTarget.user}@${flow.sshTarget.host}` : t("direct · localhost");
  const viaKind: DbSplashRowData["via"]["kind"] = flow.hasSsh ? "tunnel" : "direct";

  const savedRows: DbSplashRowData[] = flow.savedForKind.map((cred) => ({
    id: cred.id,
    name: cred.label || cred.id,
    env: inferEnv(cred.label),
    engine: t("PostgreSQL"),
    addr: `${cred.host}:${cred.port}`,
    via: { kind: viaKind, label: viaLabel },
    user: cred.user,
    authHint: cred.hasPassword ? t("keyring") : undefined,
    stats: cred.database ? <span>{cred.database}</span> : <span className="sep">—</span>,
    lastUsed: null,
    status: "unknown",
    tintVar: "var(--svc-postgres)",
    connectLabel: t("Connect"),
    onConnect: () => flow.activateCredential(cred.id),
  }));

  const detectedRows: DbSplashRowData[] = flow.detectedForKind.map((det) => ({
    id: det.signature,
    name: det.label,
    env: inferEnv(det.label),
    engine: det.version ? `PostgreSQL ${det.version}` : t("PostgreSQL"),
    addr: `${det.host}:${det.port}`,
    via: {
      kind: det.source === "docker" ? "local" : "remote",
      label: det.source === "docker" ? det.image || t("docker container") : det.processName || t("systemd unit"),
    },
    stats: <span className="sep">—</span>,
    lastUsed: null,
    status: "up",
    tintVar: "var(--svc-postgres)",
    connectLabel: t("Adopt & connect"),
    onConnect: () => {
      flow.setAdopting(det);
      flow.setAddOpen(true);
    },
  }));

  // ── Connected-state derived ───────────────────────────────
  const currentCred = tab.pgActiveCredentialId
    ? flow.savedForKind.find((c) => c.id === tab.pgActiveCredentialId)
    : undefined;

  const currentInstance: DbHeaderInstance = {
    id: currentCred?.id ?? "adhoc",
    name: currentCred?.label || tab.pgDatabase || tab.pgHost || t("PostgreSQL"),
    addr: `${tab.pgHost}:${tab.pgPort}`,
    via: flow.hasSsh ? t("SSH tunnel") : t("direct"),
    status: state ? "up" : "unknown",
    sub: <>{`${tab.pgHost}:${tab.pgPort}`}</>,
  };

  const otherInstances: DbHeaderInstance[] = flow.savedForKind
    .filter((c) => c.id !== tab.pgActiveCredentialId)
    .map((c) => ({
      id: c.id,
      name: c.label || c.id,
      addr: `${c.host}:${c.port}`,
      via: c.database ?? "",
      status: "unknown",
    }));

  // PG tree: `schemas[]` isn't enumerated by the backend yet — we show
  // only the current (db, schema) and collapse other dbs to stubs.
  const databases: DbSchemaDatabase[] = state
    ? state.databases.map((name) => ({
        name,
        current: name === state.databaseName,
        tables:
          name === state.databaseName
            ? state.tables.map((tname) => ({
                id: `${name}.${state.schemaName}.${tname}`,
                label: tname,
              }))
            : [],
      }))
    : [];

  const pkColumns = state
    ? state.columns.filter((c) => c.key === "PRI" || c.key === "PK").map((c) => c.name)
    : [];
  const numericColumns = state
    ? state.columns.filter((c) => NUMERIC_TYPE_RE.test(c.columnType)).map((c) => c.name)
    : [];
  const gridColumns = state ? gridColumnsFromPostgres(state.columns) : [];

  const [committing, setCommitting] = useState(false);
  async function commitMutations(mutations: DbMutation[]) {
    if (!state || mutations.length === 0) return;
    const tableRef = qualifyTable("postgres", {
      schema: state.schemaName,
      table: state.tableName,
    });
    setCommitting(true);
    setQueryError("");
    setNotice("");
    try {
      const target = await flow.ensureConnectionTarget();
      let written = 0;
      for (const mut of mutations) {
        const sql = mutationToSql(
          { dialect: "postgres", table: tableRef, columns: gridColumns },
          mut,
        );
        await cmd.postgresExecute({
          host: target.host,
          port: target.port,
          user: tab.pgUser.trim(),
          password: tab.pgPassword,
          database: tab.pgDatabase.trim() || null,
          sql,
        });
        written += 1;
      }
      setNotice(t("Committed {n} change(s).", { n: written }));
      await browse();
    } catch (e) {
      setQueryError(formatError(e));
      throw e;
    } finally {
      setCommitting(false);
    }
  }

  const headerStats = state
    ? [
        { icon: "database" as const, label: t("{count} dbs", { count: state.databases.length }) },
        { icon: "disk" as const, label: t("{count} tables", { count: state.tables.length }) },
        { icon: "activity" as const, label: state.schemaName || "public" },
      ]
    : [];

  useEffect(() => {
    if (error && !state) setTimeout(() => passwordInputRef.current?.focus(), 0);
  }, [error, state]);

  const banner = error ? (
    <DismissibleNote variant="status" tone="error" onDismiss={() => setError("")}>
      <div>{error}</div>
      {flow.canUpdatePassword(error) && (
        <div className="button-row" style={{ marginTop: 6 }}>
          <button className="mini-button" onClick={() => flow.setPwUpdateOpen(true)} type="button">
            {t("Update password")}
          </button>
        </div>
      )}
    </DismissibleNote>
  ) : flow.tunnelError ? (
    <DismissibleNote variant="status" tone="error" onDismiss={() => flow.setTunnelError("")}>
      {flow.tunnelError}
    </DismissibleNote>
  ) : null;

  const dialogs = (
    <>
      <DbAddCredentialDialog
        open={flow.addOpen}
        onClose={() => flow.setAddOpen(false)}
        kind="postgres"
        savedConnectionIndex={flow.savedIndex}
        adopting={flow.adopting}
        tab={tab}
        onSaved={flow.handleCredentialAdded}
      />
      {tab.pgActiveCredentialId && flow.savedIndex !== null && (
        <DbPasswordUpdateDialog
          open={flow.pwUpdateOpen}
          onClose={() => flow.setPwUpdateOpen(false)}
          savedConnectionIndex={flow.savedIndex}
          credentialId={tab.pgActiveCredentialId}
          credentialLabel={tab.pgDatabase.trim() || tab.pgHost.trim() || t("PostgreSQL")}
          onUpdated={() => void flow.handlePasswordUpdated()}
        />
      )}
    </>
  );

  if (!state) {
    return (
      <>
        {banner && <div className="db-panel-banner">{banner}</div>}
        <DbConnectSplash
          kind="postgres"
          probeTarget={flow.probeTarget}
          probeState={flow.probeState}
          onReprobe={flow.sshTarget ? () => void flow.refreshDetection() : undefined}
          detected={detectedRows}
          saved={savedRows}
          onAddManual={() => {
            flow.setAdopting(null);
            flow.setAddOpen(true);
          }}
          footerHint={busy ? t("Connecting...") : null}
          description={
            flow.hasSsh
              ? undefined
              : t("No SSH session on this tab — add a connection manually to connect directly.")
          }
        />
        {dialogs}
      </>
    );
  }

  const resultToolbar = (
    <>
      {queryResult && (
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={() => {
            void writeClipboardText(queryResultToTsv(queryResult));
            setNotice(t("Copied TSV"));
          }}
        >
          {t("Copy TSV")}
        </button>
      )}
      {flow.hasSsh && (
        <DbTunnelChip
          localPort={tab.pgTunnelPort}
          busy={flow.tunnelBusy}
          hasError={!!flow.tunnelError}
          onRebuild={() => void flow.rebuildTunnel()}
          onClose={() => void flow.closeTunnel()}
        />
      )}
    </>
  );

  const dataTab = (
    <>
      <DbSqlEditor
        tabName={state.tableName || t("query")}
        sql={sql}
        onChange={setSql}
        writable={!readOnly}
        onToggleWrite={() => {
          setReadOnly((prev) => !prev);
          setWriteConfirm("");
        }}
        needsWriteConfirm={needsWrite}
        writeConfirm={writeConfirm}
        onWriteConfirmChange={setWriteConfirm}
        onRun={() => void runQuery()}
        canRun={canRun}
        running={queryBusy}
        tabs={sqlTabs.tabs}
        activeTabId={sqlTabs.activeTabId}
        onActiveTabChange={sqlTabs.setActiveTabId}
        onAddTab={() => sqlTabs.addTab()}
        onCloseTab={sqlTabs.closeTab}
        history={sqlTabs.history}
        onPickHistory={sqlTabs.loadHistory}
      />
      <DbResultGrid
        preview={state.preview}
        pkColumns={pkColumns}
        numericColumns={numericColumns}
        toolbar={resultToolbar}
        emptyLabel={
          state.tableName ? t("No rows in this table.") : t("Pick a table from the tree to preview rows.")
        }
        columnsMeta={gridColumns}
        writable={!readOnly && state.tableName !== ""}
        onCommit={commitMutations}
        committing={committing}
      />
      {queryError && (
        <div className="db-panel-banner">
          <DismissibleNote variant="status" tone="error" onDismiss={() => setQueryError("")}>
            {queryError}
          </DismissibleNote>
        </div>
      )}
      {notice && !queryError && <div className="db-panel-notice">{notice}</div>}
    </>
  );

  const structureTab = (
    <DbStructureView
      columns={state.columns.map((c) => ({
        name: c.name,
        type: c.columnType,
        pk: c.key === "PRI" || c.key === "PK",
        nullable: c.nullable,
        keyHint: c.key && !(c.key === "PRI" || c.key === "PK") ? c.key : undefined,
      }))}
      typeAccentVar="var(--svc-postgres)"
      indexes={state.indexes}
      foreignKeys={state.foreignKeys}
    />
  );

  return (
    <>
      {banner && <div className="db-panel-banner db-panel-banner--snug">{banner}</div>}
      <DbConnectedShell
        kind="postgres"
        current={currentInstance}
        otherInstances={otherInstances}
        onSwitchInstance={flow.activateCredential}
        onAddConnection={() => {
          flow.setAdopting(null);
          flow.setAddOpen(true);
        }}
        onDisconnect={() => void flow.disconnect()}
        headerStats={headerStats}
        tab={connectedTab}
        onTabChange={setConnectedTab}
        crumb={{
          database: state.databaseName || undefined,
          schema: state.schemaName || undefined,
          table: state.tableName || undefined,
          stat: state.preview ? t("{count} rows", { count: state.preview.rows.length }) : null,
        }}
        schema={{
          databases,
          selectedTableId: state.tableName
            ? `${state.databaseName}.${state.schemaName}.${state.tableName}`
            : null,
          onSelectTable: (_db, node) => {
            const tbl = node.label;
            sqlTabs.replaceActiveSql(`SELECT * FROM "${state.schemaName}"."${tbl}" LIMIT 100;`, tbl);
            void browse(undefined, tbl);
          },
          onSelectDatabase: (name) => {
            updateTab(tab.id, { pgDatabase: name });
            void browse(undefined, "", name);
          },
        }}
        dataTab={dataTab}
        structureTab={structureTab}
        schemaTab={
          <DbConfigView
            title={t("PostgreSQL settings")}
            note={t("read-only")}
            load={async () => {
              const target = await flow.ensureConnectionTarget();
              const r = await cmd.postgresExecute({
                host: target.host,
                port: target.port,
                user: tab.pgUser.trim(),
                password: tab.pgPassword,
                database: tab.pgDatabase.trim() || null,
                sql: "SELECT name, setting, unit, short_desc, context FROM pg_settings ORDER BY name",
              });
              return r.rows.map((row): DbConfigRow => {
                const setting = row[1] ?? "";
                const unit = row[2] ?? "";
                return {
                  name: row[0] ?? "",
                  value: unit ? `${setting} ${unit}` : setting,
                  description: row[3] ?? "",
                  source: row[4] ?? "",
                };
              });
            }}
          />
        }
      />
      {dialogs}
    </>
  );
}
