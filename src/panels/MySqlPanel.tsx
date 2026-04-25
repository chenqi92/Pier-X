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
  gridColumnsFromMysql,
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
  MysqlBrowserState,
  QueryExecutionResult,
  TabState,
} from "../lib/types";
import { useTabStore } from "../stores/useTabStore";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";

type Props = { tab: TabState };

/** MySQL column types whose values should render right-aligned. */
const NUMERIC_TYPE_RE = /^(tiny|small|medium|big)?int|^decimal|^numeric|^float|^double|^real/i;

/** Field adapter: maps the hook's generic getters/patches to the flat
 *  `mysql*` slots on `TabState`. */
const MYSQL_ADAPTER: DbCredentialFieldAdapter = {
  readHost: (t) => t.mysqlHost,
  readPort: (t) => t.mysqlPort,
  readUser: (t) => t.mysqlUser,
  readPassword: (t) => t.mysqlPassword,
  readActiveCredId: (t) => t.mysqlActiveCredentialId,
  readTunnelId: (t) => t.mysqlTunnelId,
  readTunnelPort: (t) => t.mysqlTunnelPort,
  patchFromCred: (cred) => ({
    mysqlActiveCredentialId: cred.id,
    mysqlHost: cred.host,
    mysqlPort: cred.port,
    mysqlUser: cred.user,
    mysqlPassword: "",
    mysqlDatabase: cred.database ?? "",
    mysqlTunnelId: null,
    mysqlTunnelPort: null,
  }),
  patchFromSaved: (cred) => ({
    mysqlActiveCredentialId: cred.id,
    mysqlHost: cred.host,
    mysqlPort: cred.port,
    mysqlUser: cred.user,
    mysqlDatabase: cred.database ?? "",
    mysqlTunnelId: null,
    mysqlTunnelPort: null,
  }),
  patchPassword: (password) => ({ mysqlPassword: password }),
  patchPasswordAfterRotate: (password) => ({ mysqlPassword: password }),
};

export default function MySqlPanel(props: Props) {
  const ready = useDeferredMount();
  // Splash skeleton when no credential is bound yet (the body will land
  // on DbConnectSplash); grid skeleton when a credential is already
  // selected (the body will auto-browse straight into DbConnectedShell).
  const variant = props.tab.mysqlActiveCredentialId ? "grid" : "splash";
  return (
    <div className="panel-stage">
      {ready ? <MySqlPanelBody {...props} /> : <PanelSkeleton variant={variant} rows={8} />}
    </div>
  );
}

function MySqlPanelBody({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const updateTab = useTabStore((s) => s.updateTab);

  // ── Panel-local state (connection + editor + grid) ─────────
  const [state, setState] = useState<MysqlBrowserState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [readOnly, setReadOnly] = useState(true);
  const [writeConfirm, setWriteConfirm] = useState("");
  const [queryResult, setQueryResult] = useState<QueryExecutionResult | null>(null);
  const [queryBusy, setQueryBusy] = useState(false);
  const [queryError, setQueryError] = useState("");
  const [notice, setNotice] = useState("");

  const [connectedTab, setConnectedTab] = useState<DbConnectedTab>("data");

  // SQL editor tabs + run history. History persists per-engine
  // via localStorage so a panel reload (or switching tabs and
  // back) preserves the last 200 queries.
  const sqlTabs = useDbSqlTabs({
    initialSql: "SHOW TABLES;",
    initialName: t("query"),
    storageKey: "mysql",
  });
  const sql = sqlTabs.sql;
  const setSql = sqlTabs.setSql;

  const passwordInputRef = useRef<HTMLInputElement | null>(null);

  /** Clear panel-local state on credential switch / disconnect so a fresh
   *  cred doesn't inherit the previous panel's preview / query state. */
  function resetPanel() {
    setState(null);
    setError("");
    setQueryResult(null);
    setQueryError("");
    setNotice("");
    setReadOnly(true);
    setWriteConfirm("");
  }

  // Server-side paging — kept local; switching tables resets offset to 0.
  const [pageSize, setPageSize] = useState(24);
  const [pageOffset, setPageOffset] = useState(0);

  async function browse(
    passwordOverride?: string,
    nextTable?: string,
    nextOffset?: number,
    nextSize?: number,
  ) {
    setBusy(true);
    setError("");
    try {
      const target = await flow.ensureConnectionTarget();
      const pw = passwordOverride !== undefined ? passwordOverride : tab.mysqlPassword;
      const tableTarget = (nextTable ?? state?.tableName ?? "").trim() || null;
      // Switching the active table resets paging — the previous
      // table's offset doesn't apply.
      const tableChanged = tableTarget !== (state?.tableName ?? "");
      const effectiveOffset = nextOffset ?? (tableChanged ? 0 : pageOffset);
      const effectiveSize = nextSize ?? pageSize;
      const s = await cmd.mysqlBrowse({
        host: target.host,
        port: target.port,
        user: tab.mysqlUser.trim(),
        password: pw,
        database: tab.mysqlDatabase.trim() || null,
        table: tableTarget,
        offset: effectiveOffset,
        limit: effectiveSize,
      });
      setState(s);
      setPageSize(s.pageSize);
      setPageOffset(s.pageOffset);
      if (s.databaseName !== tab.mysqlDatabase) {
        updateTab(tab.id, { mysqlDatabase: s.databaseName });
      }
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  const flow = useDbCredentialFlow({
    tab,
    kind: "mysql",
    tunnelSlot: "mysql",
    adapter: MYSQL_ADAPTER,
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
      const r = await cmd.mysqlExecute({
        host: target.host,
        port: target.port,
        user: tab.mysqlUser.trim(),
        password: tab.mysqlPassword,
        database: tab.mysqlDatabase.trim() || null,
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

  // ── Derived ────────────────────────────────────────────────
  const needsWrite = sql.trim() !== "" && !isReadOnlySql(sql);
  const hostReady = tab.mysqlHost.trim() !== "" && tab.mysqlUser.trim() !== "" && tab.mysqlPort > 0;
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
    engine: t("MySQL"),
    addr: `${cred.host}:${cred.port}`,
    via: { kind: viaKind, label: viaLabel },
    user: cred.user,
    authHint: cred.hasPassword ? t("keyring") : undefined,
    stats: cred.database ? <span>{cred.database}</span> : <span className="sep">—</span>,
    lastUsed: null,
    status: "unknown",
    tintVar: "var(--svc-mysql)",
    connectLabel: t("Connect"),
    onConnect: () => flow.activateCredential(cred.id),
  }));

  const detectedRows: DbSplashRowData[] = flow.detectedForKind.map((det) => ({
    id: det.signature,
    name: det.label,
    env: inferEnv(det.label),
    engine: det.version ? `MySQL ${det.version}` : t("MySQL"),
    addr: `${det.host}:${det.port}`,
    via: {
      kind: det.source === "docker" ? "local" : "remote",
      label: det.source === "docker" ? det.image || t("docker container") : det.processName || t("systemd unit"),
    },
    stats: <span className="sep">—</span>,
    lastUsed: null,
    status: "up",
    tintVar: "var(--svc-mysql)",
    connectLabel: t("Adopt & connect"),
    onConnect: () => {
      flow.setAdopting(det);
      flow.setAddOpen(true);
    },
  }));

  // ── Connected-state derived data ───────────────────────────
  const currentCred = tab.mysqlActiveCredentialId
    ? flow.savedForKind.find((c) => c.id === tab.mysqlActiveCredentialId)
    : undefined;

  const currentInstance: DbHeaderInstance = {
    id: currentCred?.id ?? "adhoc",
    name: currentCred?.label || tab.mysqlDatabase || tab.mysqlHost || t("MySQL"),
    addr: `${tab.mysqlHost}:${tab.mysqlPort}`,
    via: flow.hasSsh ? t("SSH tunnel") : t("direct"),
    status: state ? "up" : "unknown",
    sub: <>{`${tab.mysqlHost}:${tab.mysqlPort}`}</>,
  };

  const otherInstances: DbHeaderInstance[] = flow.savedForKind
    .filter((c) => c.id !== tab.mysqlActiveCredentialId)
    .map((c) => ({
      id: c.id,
      name: c.label || c.id,
      addr: `${c.host}:${c.port}`,
      via: c.database ?? "",
      status: "unknown",
    }));

  const databases: DbSchemaDatabase[] = state
    ? state.databases.map((name) => ({
        name,
        current: name === state.databaseName,
        tables:
          name === state.databaseName
            ? state.tables.map((tname) => ({ id: `${name}.${tname}`, label: tname }))
            : [],
      }))
    : [];

  const pkColumns = state ? state.columns.filter((c) => c.key === "PRI").map((c) => c.name) : [];
  const numericColumns = state
    ? state.columns.filter((c) => NUMERIC_TYPE_RE.test(c.columnType)).map((c) => c.name)
    : [];
  const gridColumns = state ? gridColumnsFromMysql(state.columns) : [];

  // Inline-edit commit path. The grid emits abstract mutations; this
  // function turns them into one MySQL UPDATE/INSERT/DELETE per
  // mutation and ships them through `mysqlExecute` sequentially. On
  // partial failure we stop, surface the error, and leave the dirty
  // state intact so the user can retry.
  const [committing, setCommitting] = useState(false);
  async function commitMutations(mutations: DbMutation[]) {
    if (!state || mutations.length === 0) return;
    const tableRef = qualifyTable("mysql", {
      database: state.databaseName,
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
          { dialect: "mysql", table: tableRef, columns: gridColumns },
          mut,
        );
        await cmd.mysqlExecute({
          host: target.host,
          port: target.port,
          user: tab.mysqlUser.trim(),
          password: tab.mysqlPassword,
          database: tab.mysqlDatabase.trim() || null,
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
      ]
    : [];

  // Reset the auto-browse password focus target when the panel remounts.
  useEffect(() => {
    if (error && !state) setTimeout(() => passwordInputRef.current?.focus(), 0);
  }, [error, state]);

  // ── Banner + dialogs ───────────────────────────────────────
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
        kind="mysql"
        savedConnectionIndex={flow.savedIndex}
        adopting={flow.adopting}
        tab={tab}
        onSaved={flow.handleCredentialAdded}
      />
      {tab.mysqlActiveCredentialId && flow.savedIndex !== null && (
        <DbPasswordUpdateDialog
          open={flow.pwUpdateOpen}
          onClose={() => flow.setPwUpdateOpen(false)}
          savedConnectionIndex={flow.savedIndex}
          credentialId={tab.mysqlActiveCredentialId}
          credentialLabel={tab.mysqlDatabase.trim() || tab.mysqlHost.trim() || t("MySQL")}
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
          kind="mysql"
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

  // Pager — derived from the live state. Rendered inline next to
  // the toolbar so the user always has the current page info in
  // view, plus a row-count summary in the crumb stat.
  const totalRows = state.totalRows ?? null;
  const totalPages =
    totalRows !== null && pageSize > 0
      ? Math.max(1, Math.ceil(totalRows / pageSize))
      : null;
  const currentPage = pageSize > 0 ? Math.floor(pageOffset / pageSize) + 1 : 1;
  const canPrev = pageOffset > 0 && !busy;
  const canNext =
    !busy &&
    state.tableName !== "" &&
    (totalRows === null
      ? // Without a row count, only allow Next when the page came
        // back full — otherwise we know we're on the last page.
        (state.preview?.rows.length ?? 0) >= pageSize
      : pageOffset + pageSize < totalRows);

  const pagerToolbar =
    state.tableName !== "" ? (
      <>
        <button
          type="button"
          className="btn is-ghost is-compact"
          disabled={!canPrev}
          onClick={() =>
            void browse(undefined, undefined, Math.max(0, pageOffset - pageSize))
          }
          title={t("Previous page")}
        >
          ←
        </button>
        <span className="mono" style={{ fontSize: "var(--size-small)", color: "var(--muted)" }}>
          {totalPages !== null
            ? t("Page {n} of {total}", { n: currentPage, total: totalPages })
            : t("Page {n} of ?", { n: currentPage })}
        </span>
        <button
          type="button"
          className="btn is-ghost is-compact"
          disabled={!canNext}
          onClick={() => void browse(undefined, undefined, pageOffset + pageSize)}
          title={t("Next page")}
        >
          →
        </button>
        <select
          className="mono"
          style={{ fontSize: "var(--size-small)" }}
          value={pageSize}
          onChange={(e) => {
            const next = Number.parseInt(e.currentTarget.value, 10);
            if (Number.isFinite(next) && next > 0) {
              void browse(undefined, undefined, 0, next);
            }
          }}
          title={t("Rows per page")}
        >
          {[24, 50, 100, 200, 500].map((n) => (
            <option key={n} value={n}>
              {n}/{t("page")}
            </option>
          ))}
        </select>
      </>
    ) : null;

  const resultToolbar = (
    <>
      {pagerToolbar}
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
          localPort={tab.mysqlTunnelPort}
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
        pk: c.key === "PRI",
        nullable: c.nullable,
        keyHint: c.key && c.key !== "PRI" ? c.key : undefined,
      }))}
      typeAccentVar="var(--svc-mysql)"
      footnote={t("Indexes and foreign keys will appear here once the backend exposes them — see docs/BACKEND-GAPS.md.")}
    />
  );

  return (
    <>
      {banner && <div className="db-panel-banner db-panel-banner--snug">{banner}</div>}
      <DbConnectedShell
        kind="mysql"
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
          table: state.tableName || undefined,
          stat: state.preview
            ? totalRows !== null
              ? t("{shown} of {total} rows", {
                  shown: state.preview.rows.length,
                  total: totalRows,
                })
              : t("{count} rows", { count: state.preview.rows.length })
            : null,
        }}
        schema={{
          databases,
          selectedTableId: state.tableName ? `${state.databaseName}.${state.tableName}` : null,
          onSelectTable: (_db, node) => {
            const tbl = node.label;
            sqlTabs.replaceActiveSql(`SELECT * FROM \`${tbl}\` LIMIT 100;`, tbl);
            void browse(undefined, tbl);
          },
          onSelectDatabase: (name) => {
            updateTab(tab.id, { mysqlDatabase: name });
            void browse(undefined, "");
          },
        }}
        dataTab={dataTab}
        structureTab={structureTab}
        schemaTab={
          <DbConfigView
            title={t("MySQL variables")}
            note={t("read-only")}
            load={async () => {
              const target = await flow.ensureConnectionTarget();
              const r = await cmd.mysqlExecute({
                host: target.host,
                port: target.port,
                user: tab.mysqlUser.trim(),
                password: tab.mysqlPassword,
                database: tab.mysqlDatabase.trim() || null,
                sql: "SHOW VARIABLES",
              });
              return r.rows.map(
                (row): DbConfigRow => ({
                  name: row[0] ?? "",
                  value: row[1] ?? "",
                }),
              );
            }}
          />
        }
      />
      {dialogs}
    </>
  );
}
