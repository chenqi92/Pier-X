import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useRef, useState } from "react";

import ConfirmDialog from "../components/ConfirmDialog";
import DbAddCredentialDialog from "../components/DbAddCredentialDialog";
import DbPasswordUpdateDialog from "../components/DbPasswordUpdateDialog";
import DbTunnelChip from "../components/DbTunnelChip";
import DismissibleNote from "../components/DismissibleNote";
import InlineInstallCta from "../components/InlineInstallCta";
import DbConnectSplash from "../components/db/DbConnectSplash";
import DbConnectedShell, { type DbConnectedTab } from "../components/db/DbConnectedShell";
import DbCreateDbDialog from "../components/db/DbCreateDbDialog";
import type { DbHeaderInstance } from "../components/db/DbHeaderPicker";
import DbConfigView, { type DbConfigRow } from "../components/db/DbConfigView";
import DbResultGrid from "../components/db/DbResultGrid";
import DbRowDetail from "../components/db/DbRowDetail";
import { buildFkEdges } from "../components/db/fkNav";
import { type DbSchemaActions, type DbSchemaDatabase } from "../components/db/DbSchemaTree";
import DbStructureView from "../components/db/DbStructureView";
import {
  exportTablesAsInserts,
  splitSqlStatements,
} from "../components/db/dbImportExport";
import DbSqlEditor from "../components/db/DbSqlEditor";
import ExplainPlanView from "../components/db/ExplainPlanView";
import {
  extractJsonPlanCell,
  parsePostgresPlan,
  type PlanNode,
} from "../lib/explainPlan";
import type { DbSplashRowData } from "../components/db/DbSplashRow";
import { inferEnv } from "../components/db/dbTheme";
import {
  useDbCredentialFlow,
  type DbCredentialFieldAdapter,
} from "../components/db/useDbCredentialFlow";
import { useDbSqlTabs } from "../components/db/useDbSqlTabs";
import PostgresActivityDialog from "../components/db/PostgresActivityDialog";
import {
  ddlToSql,
  gridColumnsFromPostgres,
  mutationToSql,
  qualifyTable,
  type DbMutation,
  type DdlMutation,
} from "../components/db/dbColumnRules";
import { formatSqlText } from "../components/db/sqlFormat";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import { writeClipboardText } from "../lib/clipboard";
import {
  formatBytes as formatDbBytes,
  formatLastSeen,
  getDbConnCache,
  setDbConnCache,
} from "../lib/dbConnCache";
import * as cmd from "../lib/commands";
import { isReadOnlySql, queryResultToCsv, queryResultToTsv } from "../lib/commands";
import type {
  PostgresBrowserState,
  QueryExecutionResult,
  TabState,
} from "../lib/types";
import { useTabStore } from "../stores/useTabStore";
import { softwareKeyForTab, useSoftwareStore } from "../stores/useSoftwareStore";
import { useSoftwareSnapshot } from "../lib/softwareInstall";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";

type Props = { tab: TabState };

// PostgreSQL numeric types — parallels the MySQL regex in MySqlPanel.
const NUMERIC_TYPE_RE = /^(smallint|integer|bigint|numeric|decimal|real|double|money|serial|bigserial)/i;

/** Compact human-readable byte formatter for the schema-tree
 *  tooltip. Inlined per CLAUDE.md "three similar lines beats
 *  premature abstraction" — same shape as the copies in
 *  `MySqlPanel`, `SqlitePanel`, `SftpPanel`. */
function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

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
  const [plan, setPlan] = useState<PlanNode | null>(null);
  const [planMeta, setPlanMeta] = useState<string>("");
  const [openedRow, setOpenedRow] = useState<string[] | null>(null);
  const [planHistory, setPlanHistory] = useState<PlanNode[]>([]);
  const [comparePrev, setComparePrev] = useState(false);
  const PLAN_HISTORY_CAP = 5;
  const [queryBusy, setQueryBusy] = useState(false);
  const [queryError, setQueryError] = useState("");
  const [notice, setNotice] = useState("");

  const [connectedTab, setConnectedTab] = useState<DbConnectedTab>("data");
  const [activityOpen, setActivityOpen] = useState(false);

  const sqlTabs = useDbSqlTabs({
    initialSql: "SELECT version();",
    initialName: t("query"),
    storageKey: "postgres",
  });
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
      // Cache the roundtrip + size on the active credential so the
      // splash can render a "23 ms · 4.2 MB" chip on next visit.
      if (tab.pgActiveCredentialId) {
        const sizeBytes = s.tableSummaries.reduce(
          (acc, ts) => acc + (ts.dataBytes ?? 0) + (ts.indexBytes ?? 0),
          0,
        );
        setDbConnCache("postgres", tab.pgActiveCredentialId, {
          connectMs: s.browseElapsedMs,
          sizeBytes: sizeBytes > 0 ? sizeBytes : undefined,
        });
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

  const swKey = softwareKeyForTab(tab);
  const swSshParams = useMemo(
    () =>
      flow.sshTarget
        ? {
            host: flow.sshTarget.host,
            port: flow.sshTarget.port,
            user: flow.sshTarget.user,
            authMode: flow.sshTarget.authMode,
            password: flow.sshTarget.password,
            keyPath: flow.sshTarget.keyPath,
            savedConnectionIndex: flow.sshTarget.savedConnectionIndex,
          }
        : null,
    [
      flow.sshTarget?.host,
      flow.sshTarget?.port,
      flow.sshTarget?.user,
      flow.sshTarget?.authMode,
      flow.sshTarget?.password,
      flow.sshTarget?.keyPath,
      flow.sshTarget?.savedConnectionIndex,
    ],
  );
  useSoftwareSnapshot(swKey, swSshParams);
  const postgresInstalled = useSoftwareStore((s) =>
    swKey ? s.get(swKey).statuses["postgres"]?.installed : undefined,
  );

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

  /** Save the current query result to a CSV / TSV file. Mirrors
   *  the same flow as MySqlPanel — see that copy for the rationale. */
  async function saveResultAs(format: "csv" | "tsv") {
    if (!queryResult) return;
    try {
      const dialog = await import("@tauri-apps/plugin-dialog");
      const ext = format;
      const base =
        state?.tableName && state.tableName.trim().length > 0
          ? state.tableName.trim()
          : "query";
      const picked = await dialog.save({
        title:
          format === "csv"
            ? t("Save result as CSV")
            : t("Save result as TSV"),
        defaultPath: `${base}.${ext}`,
        filters: [
          {
            name: format === "csv" ? "CSV" : "TSV",
            extensions: [ext],
          },
        ],
      });
      if (typeof picked !== "string") return;
      const blob =
        format === "csv"
          ? queryResultToCsv(queryResult)
          : queryResultToTsv(queryResult);
      await cmd.localWriteTextFile(picked, blob);
      setNotice(
        t("Saved {n} row(s) to {path}", {
          n: queryResult.rows.length,
          path: picked,
        }),
      );
    } catch (e) {
      setQueryError(formatError(e));
    }
  }

  /** Run `EXPLAIN <sql>` without mutating the editor. Skips the
   *  prepend when the user already wrote their own EXPLAIN /
   *  EXPLAIN ANALYZE. */
  async function runExplain() {
    const trimmed = sql.trim();
    if (!trimmed) return;
    const explainSql = /^explain\b/i.test(trimmed) ? trimmed : `EXPLAIN ${trimmed}`;
    setQueryBusy(true);
    setQueryError("");
    setNotice("");
    try {
      const target = await flow.ensureConnectionTarget();
      const r = await cmd.postgresExecute({
        host: target.host,
        port: target.port,
        user: tab.pgUser.trim(),
        password: tab.pgPassword,
        database: tab.pgDatabase.trim() || null,
        sql: explainSql,
      });
      setQueryResult(r);
      setNotice(t("EXPLAIN · {elapsed} ms", { elapsed: r.elapsedMs }));
    } catch (e) {
      setQueryResult(null);
      setQueryError(formatError(e));
    } finally {
      setQueryBusy(false);
    }
  }

  /** Run `EXPLAIN (ANALYZE, FORMAT JSON, BUFFERS) <sql>` and parse
   *  the plan into a tree. ANALYZE actually executes the query, so
   *  this is intentionally distinct from the read-only `runExplain`
   *  — DML statements run through `runPlan` will mutate. We strip
   *  any leading `EXPLAIN [(...)]` the user typed so we always pin
   *  the JSON+ANALYZE+BUFFERS combo. */
  async function runPlan() {
    const trimmed = sql.trim();
    if (!trimmed) return;
    const stripped = trimmed.replace(/^explain(\s*\([^)]*\))?\s+/i, "");
    const planSql = `EXPLAIN (ANALYZE, FORMAT JSON, BUFFERS) ${stripped}`;
    setQueryBusy(true);
    setQueryError("");
    setNotice("");
    try {
      const target = await flow.ensureConnectionTarget();
      const r = await cmd.postgresExecute({
        host: target.host,
        port: target.port,
        user: tab.pgUser.trim(),
        password: tab.pgPassword,
        database: tab.pgDatabase.trim() || null,
        sql: planSql,
      });
      const cell = extractJsonPlanCell(r.rows as unknown[][]);
      if (!cell) {
        setQueryError(t("EXPLAIN returned no plan JSON."));
        return;
      }
      const parsed = parsePostgresPlan(cell);
      if (!parsed) {
        setQueryError(t("Could not parse the plan JSON."));
        return;
      }
      setPlanHistory((prev) => {
        const next = [parsed, ...prev];
        return next.slice(0, PLAN_HISTORY_CAP);
      });
      setPlan(parsed);
      setPlanMeta(
        t("EXPLAIN ANALYZE · {elapsed} ms", { elapsed: r.elapsedMs }),
      );
      setNotice(t("EXPLAIN · {elapsed} ms", { elapsed: r.elapsedMs }));
    } catch (e) {
      setQueryError(formatError(e));
    } finally {
      setQueryBusy(false);
    }
  }

  /** Reformat the active editor's SQL via `sql-formatter` with
   *  the postgresql dialect. Failure leaves the SQL alone. */
  function formatActiveSql() {
    const trimmed = sql.trim();
    if (!trimmed) return;
    try {
      const formatted = formatSqlText(sql, "postgresql");
      if (formatted && formatted !== sql) {
        setSql(formatted);
      }
    } catch (e) {
      setNotice(t("Format failed: {err}", { err: formatError(e) }));
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

  const savedRows: DbSplashRowData[] = flow.savedForKind.map((cred) => {
    const cache = getDbConnCache("postgres", cred.id);
    const statsBits: string[] = [];
    if (cred.database) statsBits.push(cred.database);
    if (cache) {
      statsBits.push(`${cache.connectMs} ms`);
      if (cache.sizeBytes) statsBits.push(formatDbBytes(cache.sizeBytes));
    }
    return {
      id: cred.id,
      name: cred.label || cred.id,
      env: inferEnv(cred.label),
      engine: t("PostgreSQL"),
      addr: `${cred.host}:${cred.port}`,
      via: { kind: viaKind, label: viaLabel },
      user: cred.user,
      authHint: cred.hasPassword ? t("keyring") : undefined,
      stats: statsBits.length > 0
        ? <span>{statsBits.join(" · ")}</span>
        : <span className="sep">—</span>,
      lastUsed: cache ? formatLastSeen(cache.lastConnectedAt) : null,
      status: "unknown",
      tintVar: "var(--svc-postgres)",
      connectLabel: t("Connect"),
      onConnect: () => flow.activateCredential(cred.id),
      pending: flow.activating === cred.id,
    };
  });

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

  // PG tree: backend returns table/view/routine enrichment per
  // the active schema, plus `schemas[]` for the in-tree schema
  // picker. Active db gets the full payload; siblings stay
  // collapsed (we don't enumerate their schemas — would cost a
  // connection per db).
  const databases: DbSchemaDatabase[] = state
    ? state.databases.map((name) => {
        const isCurrent = name === state.databaseName;
        if (!isCurrent) {
          return { name, current: false, tables: [] };
        }
        const summaryByName = new Map(
          state.tableSummaries.map((s) => [s.name, s] as const),
        );
        const tables = state.tables.map((tname) => {
          const meta = summaryByName.get(tname);
          const tooltipParts: string[] = [];
          if (typeof meta?.dataBytes === "number") {
            tooltipParts.push(t("data {n}", { n: formatBytes(meta.dataBytes) }));
          }
          if (typeof meta?.indexBytes === "number") {
            tooltipParts.push(t("idx {n}", { n: formatBytes(meta.indexBytes) }));
          }
          const trimmedComment = meta?.comment?.trim() ?? "";
          if (trimmedComment) {
            tooltipParts.push(t("comment: {c}", { c: trimmedComment }));
          }
          return {
            id: `${name}.${state.schemaName}.${tname}`,
            label: tname,
            count: meta?.rowCount ?? null,
            tooltip: tooltipParts.length > 0 ? tooltipParts.join(" · ") : null,
          };
        });
        const views = state.views.map((vname) => ({
          id: `${name}.${state.schemaName}.${vname}`,
          label: vname,
        }));
        const routines = state.routines.map((r) => ({
          id: `${name}.${state.schemaName}.${r.name}`,
          label: r.name,
          badge:
            r.kind.toUpperCase() === "FUNCTION"
              ? "FN"
              : r.kind.toUpperCase() === "PROCEDURE"
                ? "PR"
                : null,
        }));
        const schemaList =
          state.schemas.length > 0 ? state.schemas : [state.schemaName || "public"];
        return {
          name,
          current: true,
          tables,
          views,
          routines,
          schemas: schemaList,
          activeSchema: state.schemaName,
        };
      })
    : [];

  const pkColumns = state
    ? state.columns.filter((c) => c.key === "PRI" || c.key === "PK").map((c) => c.name)
    : [];
  const numericColumns = state
    ? state.columns.filter((c) => NUMERIC_TYPE_RE.test(c.columnType)).map((c) => c.name)
    : [];
  const gridColumns = state
    ? gridColumnsFromPostgres(state.columns, state.enums)
    : [];

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

  const [committingDdl, setCommittingDdl] = useState(false);
  async function commitStructure(mutations: DdlMutation[]) {
    if (!state || mutations.length === 0) return;
    const tableRef = qualifyTable("postgres", {
      schema: state.schemaName,
      table: state.tableName,
    });
    setCommittingDdl(true);
    setQueryError("");
    setNotice("");
    try {
      const target = await flow.ensureConnectionTarget();
      let written = 0;
      for (const mut of mutations) {
        const sql = ddlToSql({ dialect: "postgres", table: tableRef }, mut);
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
      setNotice(t("Committed {n} structure change(s).", { n: written }));
      await browse();
    } catch (e) {
      setQueryError(formatError(e));
      throw e;
    } finally {
      setCommittingDdl(false);
    }
  }

  // ── Schema-tree right-click actions ──────────────────────────
  const [confirm, setConfirm] = useState<{
    title: string;
    message: string;
    tone?: "destructive";
    onConfirm: () => void | Promise<void>;
  } | null>(null);
  const [createDbOpen, setCreateDbOpen] = useState(false);

  async function pgRunOne(sql: string): Promise<QueryExecutionResult> {
    const target = await flow.ensureConnectionTarget();
    return cmd.postgresExecute({
      host: target.host,
      port: target.port,
      user: tab.pgUser.trim(),
      password: tab.pgPassword,
      database: tab.pgDatabase.trim() || null,
      sql,
    });
  }

  const pgActions = useMemo<DbSchemaActions>(() => {
    const schemaName = state?.schemaName || "public";
    const qSchema = `"${schemaName.replace(/"/g, '""')}"`;
    const qIdent = (s: string) => `"${s.replace(/"/g, '""')}"`;
    return {
      onCopyTableName: (_db, tables) => {
        void writeClipboardText(tables.join("\n"));
      },
      onRefreshDatabase: () => {
        void browse();
      },
      onCreateDatabase: readOnly ? undefined : () => setCreateDbOpen(true),
      onTruncateTables: readOnly
        ? undefined
        : (_db, tables) => {
            setConfirm({
              title: t("Truncate {n} table(s)?", { n: tables.length }),
              message: t(
                "This deletes all rows in:\n  {tables}\n\nStructure (columns, indexes, FKs) is preserved.",
                { tables: tables.join("\n  ") },
              ),
              tone: "destructive",
              onConfirm: async () => {
                setConfirm(null);
                try {
                  const list = tables.map((t) => `${qSchema}.${qIdent(t)}`).join(", ");
                  await pgRunOne(`TRUNCATE TABLE ${list}`);
                  setNotice(t("Truncated {n} table(s).", { n: tables.length }));
                  await browse();
                } catch (e) {
                  setQueryError(formatError(e));
                }
              },
            });
          },
      onDropTables: readOnly
        ? undefined
        : (_db, tables) => {
            setConfirm({
              title: t("Drop {n} table(s)?", { n: tables.length }),
              message: t(
                "This permanently removes:\n  {tables}\n\nAll data and structure are deleted.",
                { tables: tables.join("\n  ") },
              ),
              tone: "destructive",
              onConfirm: async () => {
                setConfirm(null);
                try {
                  for (const tbl of tables) {
                    await pgRunOne(`DROP TABLE ${qSchema}.${qIdent(tbl)}`);
                  }
                  setNotice(t("Dropped {n} table(s).", { n: tables.length }));
                  await browse(undefined, "");
                } catch (e) {
                  setQueryError(formatError(e));
                }
              },
            });
          },
      onDropDatabase: readOnly
        ? undefined
        : (db) => {
            setConfirm({
              title: t("Drop database \"{db}\"?", { db }),
              message: t(
                "This permanently removes the database \"{db}\" and every schema inside it. The connection will close. This cannot be undone.",
                { db },
              ),
              tone: "destructive",
              onConfirm: async () => {
                setConfirm(null);
                try {
                  // PG can't drop the database we're connected to, so
                  // only attempt this when targeting a different DB. The
                  // schema tree's context menu always targets the
                  // current DB today; a future iteration can bounce us
                  // through a maintenance DB. For now, surface the
                  // engine error verbatim.
                  await pgRunOne(`DROP DATABASE ${qIdent(db)}`);
                  setNotice(t("Dropped database \"{db}\".", { db }));
                  await browse();
                } catch (e) {
                  setQueryError(formatError(e));
                }
              },
            });
          },
      onImportSql: readOnly
        ? undefined
        : async () => {
            const picked = await openDialog({
              multiple: false,
              filters: [{ name: "SQL", extensions: ["sql"] }],
            });
            if (typeof picked !== "string") return;
            try {
              const text = await cmd.localReadTextFile(picked);
              const stmts = splitSqlStatements(text);
              if (stmts.length === 0) {
                setNotice(t("No SQL statements found in the file."));
                return;
              }
              for (const sql of stmts) {
                await pgRunOne(sql);
              }
              setNotice(t("Executed {n} statement(s) from {file}.", {
                n: stmts.length,
                file: picked,
              }));
              await browse();
            } catch (e) {
              setQueryError(formatError(e));
            }
          },
      onExportTables: async (_db, tables) => {
        const picked = await saveDialog({
          defaultPath: `${schemaName}-${tables.length === 1 ? tables[0] : "tables"}.sql`,
          filters: [{ name: "SQL", extensions: ["sql"] }],
        });
        if (typeof picked !== "string") return;
        try {
          const result = await exportTablesAsInserts(
            (sql) => pgRunOne(sql),
            "postgres",
            { schema: schemaName },
            tables,
          );
          await cmd.localWriteTextFile(picked, result.sql);
          const totalRows = Object.values(result.perTableRowCounts).reduce(
            (a, b) => a + b,
            0,
          );
          const truncatedNote =
            result.truncatedTables.length > 0
              ? t(" · row cap hit on: {tables}", {
                  tables: result.truncatedTables.join(", "),
                })
              : "";
          setNotice(t("Exported {tables} table(s), {rows} row(s) → {file}{warn}", {
            tables: tables.length,
            rows: totalRows.toLocaleString(),
            file: picked,
            warn: truncatedNote,
          }));
        } catch (e) {
          setQueryError(formatError(e));
        }
      },
      onExportDatabase: (db) => {
        const allTables = state?.tables ?? [];
        if (allTables.length === 0) {
          setNotice(t("No tables to export in {db}.", { db }));
          return;
        }
        void pgActions.onExportTables?.(db, allTables);
      },
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [readOnly, state?.tables, state?.schemaName, tab.pgUser, tab.pgPassword, tab.pgDatabase, t]);

  const headerStats = state
    ? [
        { icon: "database" as const, label: t("{count} dbs", { count: state.databases.length }) },
        { icon: "disk" as const, label: t("{count} tables", { count: state.tables.length }) },
        { icon: "activity" as const, label: state.schemaName || "public" },
        ...(state.pool && state.pool.total > 0
          ? [
              {
                icon: "activity" as const,
                label: t("{active}/{total} conns", {
                  active: state.pool.active,
                  total: state.pool.total,
                }),
              },
            ]
          : []),
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
          onTest={async (pw) => {
            const sshTarget = flow.sshTarget;
            let liveHost = tab.pgHost.trim() || "127.0.0.1";
            let livePort = tab.pgPort;
            let tunnelId: string | null = null;
            let via = "direct";
            try {
              if (sshTarget) {
                const info = await cmd.sshTunnelOpen({
                  host: sshTarget.host,
                  port: sshTarget.port,
                  user: sshTarget.user,
                  authMode: sshTarget.authMode,
                  password: sshTarget.password,
                  keyPath: sshTarget.keyPath,
                  remoteHost: liveHost,
                  remotePort: livePort,
                  localPort: null,
                  savedConnectionIndex: sshTarget.savedConnectionIndex,
                });
                liveHost = info.localHost;
                livePort = info.localPort;
                tunnelId = info.tunnelId;
                via = "ssh-tunnel";
              }
              try {
                await cmd.postgresBrowse({
                  host: liveHost,
                  port: livePort,
                  user: tab.pgUser.trim(),
                  password: pw,
                  database: tab.pgDatabase.trim() || null,
                  schema: null,
                  table: null,
                });
                return { ok: true, via };
              } finally {
                if (tunnelId) {
                  await cmd.sshTunnelClose(tunnelId).catch(() => {});
                }
              }
            } catch (e) {
              return { ok: false, msg: formatError(e) };
            }
          }}
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
          footerHint={
            // Suppress when an error banner is showing — keeps the
            // splash from contradicting itself.
            error || flow.tunnelError
              ? null
              : flow.connectingStep ?? (busy ? t("Connecting...") : null)
          }
          description={
            flow.hasSsh
              ? undefined
              : t("No SSH session on this tab — add a connection manually to connect directly.")
          }
          extraBody={
            flow.hasSsh && postgresInstalled === false ? (
              <InlineInstallCta
                packageId="postgres"
                sshParams={swSshParams}
                swKey={swKey}
                enableService={false}
                hint={t("PostgreSQL client is not installed on this host.")}
                onInstalled={() => void flow.refreshDetection()}
              />
            ) : undefined
          }
        />
        {dialogs}
      </>
    );
  }

  const resultToolbar = (
    <>
      {state.tableName && state.browseElapsedMs > 0 && (
        <span
          className="mono"
          style={{
            fontSize: "var(--size-small)",
            color: "var(--muted)",
            padding: "0 var(--sp-1-5)",
          }}
          title={t("Wall-clock for the preview SELECT")}
        >
          {state.browseElapsedMs} ms
        </span>
      )}
      {queryResult && (
        <>
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
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={() => void saveResultAs("csv")}
            title={t("Save the current result set to a file")}
          >
            {t("Save CSV")}
          </button>
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={() => void saveResultAs("tsv")}
          >
            {t("Save TSV")}
          </button>
        </>
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
      <button
        type="button"
        className="btn is-ghost is-compact"
        onClick={() => setActivityOpen(true)}
        title={t(
          "Show server activity (pg_stat_activity) — slow queries, idle-in-tx, locks",
        )}
      >
        {t("Activity")}
      </button>
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
        favorites={sqlTabs.favorites}
        onAddFavorite={(sql, name) => sqlTabs.addFavorite({ sql, name })}
        onRemoveFavorite={sqlTabs.removeFavorite}
        onPickFavorite={sqlTabs.loadFavorite}
        onExplain={() => void runExplain()}
        onPlan={() => void runPlan()}
        onFormat={formatActiveSql}
      />
      {plan && (
        <>
          {planHistory.length >= 2 && (
            <div className="explain-plan-history mono">
              <span className="explain-plan-history__label">
                {t("History: {n} run(s)", { n: planHistory.length })}
              </span>
              <button
                type="button"
                className={`btn is-compact ${
                  comparePrev ? "is-primary" : "is-ghost"
                }`}
                onClick={() => setComparePrev((v) => !v)}
                title={t("Annotate each node with delta vs previous run")}
              >
                {comparePrev ? t("Hide diff") : t("Diff vs previous run")}
              </button>
            </div>
          )}
          <ExplainPlanView
            plan={plan}
            prevPlan={comparePrev ? planHistory[1] ?? null : null}
            meta={planMeta}
            onClose={() => setPlan(null)}
          />
        </>
      )}
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
        onToggleWritable={() => {
          setReadOnly((prev) => !prev);
          setWriteConfirm("");
        }}
        onOpenRow={(row) => setOpenedRow(row)}
        storageKey={
          state.databaseName && state.schemaName && state.tableName
            ? `pg:${state.databaseName}.${state.schemaName}.${state.tableName}`
            : undefined
        }
      />
      {openedRow && state.preview && (
        <DbRowDetail
          title={state.tableName || t("Row")}
          columns={state.preview.columns.map((name) => ({
            name,
            pk: pkColumns.includes(name),
          }))}
          row={openedRow}
          onClose={() => setOpenedRow(null)}
          foreignKeys={buildFkEdges(
            state.preview.columns,
            openedRow,
            state.foreignKeys,
            "postgres",
            (sql) => {
              setSql(sql);
              setOpenedRow(null);
              void runQuery();
            },
            t,
          )}
        />
      )}
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
        defaultValue: c.defaultValue || undefined,
        extra: c.extra,
        comment: c.comment,
      }))}
      typeAccentVar="var(--svc-postgres)"
      indexes={state.indexes}
      foreignKeys={state.foreignKeys}
      editable={!readOnly && state.tableName !== ""}
      onCommit={commitStructure}
      committing={committingDdl}
      commentEditable
      dialect="postgres"
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
          onSelectSchema: (_db, nextSchema) => {
            // Switching schema clears the active table — the
            // tables under the previous schema don't apply.
            setSchema(nextSchema);
            void browse(undefined, "", undefined, nextSchema);
          },
          actions: pgActions,
        }}
        dataTab={dataTab}
        structureTab={structureTab}
        schemaTab={
          <DbConfigView
            title={t("PostgreSQL settings")}
            note={readOnly ? t("read-only") : t("editable")}
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
                const context = String(row[4] ?? "").toLowerCase();
                // `user`, `superuser`, `sighup` reload at runtime. The
                // panel always issues ALTER SYSTEM + pg_reload_conf,
                // so user-level settings reload too — same path as a
                // proper postgresql.conf edit. `postmaster` and
                // `internal` need a server restart, so we mark those
                // read-only here even though ALTER SYSTEM would accept
                // them — surfacing the restart requirement is more
                // honest than silently writing.
                const dynamic = ["user", "superuser", "sighup"].includes(context);
                return {
                  name: row[0] ?? "",
                  value: unit ? `${setting} ${unit}` : setting,
                  description: row[3] ?? "",
                  source: row[4] ?? "",
                  editable: dynamic && !readOnly,
                  editHint: dynamic
                    ? t("ALTER SYSTEM · pg_reload_conf()")
                    : t("requires restart ({context})", { context: context || "internal" }),
                };
              });
            }}
            onEdit={
              readOnly
                ? undefined
                : async (name, newValue) => {
                    const target = await flow.ensureConnectionTarget();
                    // ALTER SYSTEM SET <name> = <value>; SELECT pg_reload_conf();
                    // The PG identifier rules are stricter than MySQL;
                    // double-quote the name to be safe with reserved
                    // words. Empty newValue → reset.
                    const ident = `"${name.replace(/"/g, '""')}"`;
                    const stmts =
                      newValue === ""
                        ? `ALTER SYSTEM RESET ${ident}; SELECT pg_reload_conf();`
                        : `ALTER SYSTEM SET ${ident} = '${newValue.replace(/'/g, "''")}'; SELECT pg_reload_conf();`;
                    await cmd.postgresExecute({
                      host: target.host,
                      port: target.port,
                      user: tab.pgUser.trim(),
                      password: tab.pgPassword,
                      database: tab.pgDatabase.trim() || null,
                      sql: stmts,
                    });
                  }
            }
          />
        }
      />
      {dialogs}
      <ConfirmDialog
        open={!!confirm}
        title={confirm?.title ?? ""}
        message={confirm?.message ?? ""}
        tone={confirm?.tone}
        onCancel={() => setConfirm(null)}
        onConfirm={() => void confirm?.onConfirm()}
      />
      <DbCreateDbDialog
        open={createDbOpen}
        kind="postgres"
        onCancel={() => setCreateDbOpen(false)}
        onSubmit={async (name, opts) => {
          // PG doesn't allow CREATE DATABASE inside a transaction.
          // postgresExecute starts each call as its own statement so
          // we're fine. OWNER is optional — when omitted, PG defaults
          // to the current role.
          let sql = `CREATE DATABASE "${name.replace(/"/g, '""')}"`;
          if (opts.owner) sql += ` OWNER "${opts.owner.replace(/"/g, '""')}"`;
          await pgRunOne(sql);
          setCreateDbOpen(false);
          setNotice(t("Created database \"{name}\".", { name }));
          await browse();
        }}
      />
      <PostgresActivityDialog
        open={activityOpen}
        onClose={() => setActivityOpen(false)}
        connection={{
          host: tab.pgTunnelPort ? "127.0.0.1" : tab.pgHost,
          port: tab.pgTunnelPort ?? tab.pgPort,
          user: tab.pgUser,
          password: tab.pgPassword,
          database: tab.pgDatabase || null,
        }}
      />
    </>
  );
}
