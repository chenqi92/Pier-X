import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { FolderSearch, HardDrive, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import ConfirmDialog from "../components/ConfirmDialog";
import DismissibleNote from "../components/DismissibleNote";
import InlineInstallCta from "../components/InlineInstallCta";
import DbConnectSplash from "../components/db/DbConnectSplash";
import DbConnectedShell, { type DbConnectedTab } from "../components/db/DbConnectedShell";
import type { DbHeaderInstance } from "../components/db/DbHeaderPicker";
import DbConfigView, { type DbConfigRow } from "../components/db/DbConfigView";
import DbResultGrid from "../components/db/DbResultGrid";
import { type DbSchemaActions, type DbSchemaDatabase } from "../components/db/DbSchemaTree";
import DbStructureView from "../components/db/DbStructureView";
import {
  exportTablesAsInserts,
  splitSqlStatements,
} from "../components/db/dbImportExport";
import DbSqlEditor from "../components/db/DbSqlEditor";
import type { DbSplashRowData } from "../components/db/DbSplashRow";
import { useDbSqlTabs } from "../components/db/useDbSqlTabs";
import {
  ddlToSql,
  gridColumnsFromSqlite,
  mutationToSql,
  qualifyTable,
  type DbMutation,
  type DdlMutation,
} from "../components/db/dbColumnRules";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import { writeClipboardText } from "../lib/clipboard";
import * as cmd from "../lib/commands";
import { isReadOnlySql, queryResultToTsv } from "../lib/commands";
import type { RemoteSqliteCandidate } from "../lib/commands";
import type {
  QueryExecutionResult,
  SqliteBrowserState,
  TabState,
} from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import { softwareKeyForTab } from "../stores/useSoftwareStore";
import { useSoftwareSnapshot } from "../lib/softwareInstall";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";

type Props = { tab: TabState | null };

type RemoteStatus =
  | { kind: "unknown" }
  | { kind: "local-only" }
  | { kind: "installed"; supportsJson: boolean; version: string | null }
  | { kind: "missing" };

const NUMERIC_TYPE_RE = /^(int|integer|bigint|real|double|numeric|decimal|float)/i;

export default function SqlitePanel(props: Props) {
  const ready = useDeferredMount();
  const variant = props.tab?.sqliteActiveCredentialId ? "grid" : "splash";
  return (
    <div className="panel-stage">
      {ready ? <SqlitePanelBody {...props} /> : <PanelSkeleton variant={variant} rows={8} />}
    </div>
  );
}

function SqlitePanelBody({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);

  const sshTarget = tab ? effectiveSshTarget(tab) : null;
  const hasSsh = sshTarget !== null;
  const swKey = tab ? softwareKeyForTab(tab) : null;
  const sshParams = useMemo(
    () =>
      sshTarget
        ? {
            host: sshTarget.host,
            port: sshTarget.port,
            user: sshTarget.user,
            authMode: sshTarget.authMode,
            password: sshTarget.password,
            keyPath: sshTarget.keyPath,
            savedConnectionIndex: sshTarget.savedConnectionIndex,
          }
        : null,
    [
      sshTarget?.host,
      sshTarget?.port,
      sshTarget?.user,
      sshTarget?.authMode,
      sshTarget?.password,
      sshTarget?.keyPath,
      sshTarget?.savedConnectionIndex,
    ],
  );
  useSoftwareSnapshot(swKey, sshParams);

  const [path, setPath] = useState("");
  const [tableName, setTableName] = useState("");
  const sqlTabs = useDbSqlTabs({
    initialSql: "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;",
    initialName: t("query"),
    storageKey: "sqlite",
  });
  const sql = sqlTabs.sql;
  const setSql = sqlTabs.setSql;
  const [readOnly, setReadOnly] = useState(true);
  const [writeConfirm, setWriteConfirm] = useState("");
  const [state, setState] = useState<SqliteBrowserState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [queryResult, setQueryResult] = useState<QueryExecutionResult | null>(null);
  const [queryBusy, setQueryBusy] = useState(false);
  const [queryError, setQueryError] = useState("");
  const [notice, setNotice] = useState("");
  const [committing, setCommitting] = useState(false);
  // Structure-edit commit spinner. Has to be declared up here with the
  // other Hooks — `commitStructure` lives below the `if (!state) return`
  // splash branch, so a `useState` next to it would only register on
  // renders past that gate and trip Rules of Hooks the moment the user
  // opens a database.
  const [committingDdl, setCommittingDdl] = useState(false);

  const [connectedTab, setConnectedTab] = useState<DbConnectedTab>("data");

  const [remoteStatus, setRemoteStatus] = useState<RemoteStatus>(
    hasSsh ? { kind: "unknown" } : { kind: "local-only" },
  );
  const [candidates, setCandidates] = useState<RemoteSqliteCandidate[]>([]);
  const [cwdHint, setCwdHint] = useState("");
  const [shellCwd, setShellCwd] = useState<string | null>(null);
  const [scanInput, setScanInput] = useState("");
  const [scanInputTouched, setScanInputTouched] = useState(false);
  const [manualPath, setManualPath] = useState("");

  const isRemoteMode =
    hasSsh && remoteStatus.kind === "installed" && remoteStatus.supportsJson;

  // Poll for OSC 7 CWD — same cadence + rationale as before.
  useEffect(() => {
    if (!hasSsh || !tab?.terminalSessionId) {
      setShellCwd(null);
      return;
    }
    const sessionId = tab.terminalSessionId;
    let cancelled = false;
    const tick = () => {
      cmd
        .terminalCurrentCwd(sessionId)
        .then((cwd) => {
          if (!cancelled) setShellCwd(cwd);
        })
        .catch(() => {
          /* unknown session — ignore */
        });
    };
    tick();
    const handle = window.setInterval(tick, 15_000);
    return () => {
      cancelled = true;
      window.clearInterval(handle);
    };
  }, [hasSsh, tab?.terminalSessionId]);

  useEffect(() => {
    if (!scanInputTouched && shellCwd && scanInput !== shellCwd) {
      setScanInput(shellCwd);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shellCwd, scanInputTouched]);

  useEffect(() => {
    if (!hasSsh || !sshTarget) {
      setRemoteStatus({ kind: "local-only" });
      return;
    }
    let cancelled = false;
    setRemoteStatus({ kind: "unknown" });
    cmd
      .sqliteRemoteCapable({
        host: sshTarget.host,
        port: sshTarget.port,
        user: sshTarget.user,
        authMode: sshTarget.authMode,
        password: sshTarget.password,
        keyPath: sshTarget.keyPath,
        savedConnectionIndex: sshTarget.savedConnectionIndex,
      })
      .then((cap) => {
        if (cancelled) return;
        if (!cap.installed) {
          setRemoteStatus({ kind: "missing" });
        } else {
          setRemoteStatus({
            kind: "installed",
            supportsJson: cap.supportsJson,
            version: cap.version,
          });
        }
      })
      .catch(() => {
        if (!cancelled) setRemoteStatus({ kind: "missing" });
      });
    return () => {
      cancelled = true;
    };
  }, [hasSsh, sshTarget?.host, sshTarget?.port, sshTarget?.user]);

  const canBrowse = path.trim().length > 0;
  const needsWrite = sql.trim() !== "" && !isReadOnlySql(sql);
  const canRun =
    canBrowse &&
    sql.trim() !== "" &&
    !queryBusy &&
    (!needsWrite || (!readOnly && writeConfirm.trim().toUpperCase() === "WRITE"));

  async function browse(nextTable = tableName, explicitPath?: string) {
    setBusy(true);
    setError("");
    const usePath = (explicitPath ?? path).trim();
    try {
      if (isRemoteMode && sshTarget) {
        const s = await cmd.sqliteBrowseRemote({
          host: sshTarget.host,
          port: sshTarget.port,
          user: sshTarget.user,
          authMode: sshTarget.authMode,
          password: sshTarget.password,
          keyPath: sshTarget.keyPath,
          savedConnectionIndex: sshTarget.savedConnectionIndex,
          dbPath: usePath,
          table: nextTable.trim() || null,
        });
        setState(s);
        setTableName(s.tableName);
      } else {
        const s = await cmd.sqliteBrowse(usePath, nextTable.trim() || null);
        setState(s);
        setTableName(s.tableName);
      }
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  async function runQuery() {
    setQueryBusy(true);
    setQueryError("");
    setNotice("");
    try {
      // Multi-statement detection: only meaningful for the local
      // path (the remote SSH-forwarded sqlite3 worker still runs
      // a single statement at a time). We pick "script mode" when
      // the trimmed input contains a `;` followed by more
      // non-whitespace — a single trailing semicolon stays in
      // single-statement mode.
      const isScript =
        !isRemoteMode &&
        /;[^]*?\S/.test(sql.trim().slice(0, -1));
      let r: QueryExecutionResult;
      if (isRemoteMode && sshTarget) {
        r = await cmd.sqliteExecuteRemote({
          host: sshTarget.host,
          port: sshTarget.port,
          user: sshTarget.user,
          authMode: sshTarget.authMode,
          password: sshTarget.password,
          keyPath: sshTarget.keyPath,
          savedConnectionIndex: sshTarget.savedConnectionIndex,
          dbPath: path.trim(),
          sql,
        });
      } else if (isScript) {
        const all = await cmd.sqliteExecuteScript(path.trim(), sql);
        // Pick the last statement that actually returned rows
        // (most often the user's tail SELECT after a few setup
        // INSERTs). When every statement is a write, fall back
        // to the last result so the timing still surfaces.
        const lastWithRows = [...all].reverse().find((s) => s.rows && s.rows.length > 0);
        r = lastWithRows ?? all[all.length - 1];
        const totalMs = all.reduce((acc, s) => acc + (s.elapsedMs ?? 0), 0);
        setNotice(
          t("{count} statements", { count: all.length }) +
            " · " +
            t("{ms} ms total", { ms: totalMs }),
        );
      } else {
        r = await cmd.sqliteExecute(path.trim(), sql);
      }
      setQueryResult(r);
      if (!isScript) {
        setNotice(t("{elapsed} ms", { elapsed: r.elapsedMs }));
      }
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
      void browse(tableName);
    } catch (e) {
      setQueryResult(null);
      setQueryError(formatError(e));
    } finally {
      setQueryBusy(false);
    }
  }

  async function reprobeSqliteCapability() {
    if (!sshTarget) return;
    try {
      const cap = await cmd.sqliteRemoteCapable({
        host: sshTarget.host,
        port: sshTarget.port,
        user: sshTarget.user,
        authMode: sshTarget.authMode,
        password: sshTarget.password,
        keyPath: sshTarget.keyPath,
        savedConnectionIndex: sshTarget.savedConnectionIndex,
      });
      if (cap.installed) {
        setRemoteStatus({
          kind: "installed",
          supportsJson: cap.supportsJson,
          version: cap.version,
        });
      }
    } catch {
      /* leave remoteStatus alone — InlineInstallCta will show the error */
    }
  }

  async function scanDir(directory: string) {
    if (!sshTarget || !directory.trim()) return;
    setCwdHint(directory);
    try {
      const rows = await cmd.sqliteFindInDir({
        host: sshTarget.host,
        port: sshTarget.port,
        user: sshTarget.user,
        authMode: sshTarget.authMode,
        password: sshTarget.password,
        keyPath: sshTarget.keyPath,
        savedConnectionIndex: sshTarget.savedConnectionIndex,
        directory: directory.trim(),
        maxDepth: 2,
      });
      setCandidates(rows);
    } catch {
      setCandidates([]);
    }
  }

  function disconnect() {
    setState(null);
    setError("");
    setQueryResult(null);
    setQueryError("");
    setNotice("");
  }

  // ── Splash rows (candidates as detected; no saved creds for SQLite yet) ──
  const probeTarget = sshTarget ? `${sshTarget.user}@${sshTarget.host}` : null;
  const probeState =
    remoteStatus.kind === "unknown"
      ? "scanning"
      : remoteStatus.kind === "missing"
        ? "error"
        : "idle";

  const detectedRows: DbSplashRowData[] = candidates.map((c) => ({
    id: c.path,
    name: c.path.split(/[/\\]/).pop() || c.path,
    env: "unknown",
    engine: t("SQLite"),
    addr: c.path,
    via: { kind: "remote", label: cwdHint || t("remote host") },
    stats: <span>{formatBytes(c.sizeBytes)}</span>,
    lastUsed: null,
    status: "up",
    tintVar: "var(--svc-sqlite)",
    connectLabel: t("Open"),
    onConnect: () => {
      setPath(c.path);
      setState(null);
      setTableName("");
      void browse("", c.path);
    },
  }));

  const remoteBannerContent: string | null = useMemo(() => {
    if (!hasSsh) return null;
    switch (remoteStatus.kind) {
      case "missing":
        return t("Remote sqlite3 not found — install `sqlite3` on the server to read remote .db files directly.");
      case "installed":
        if (!remoteStatus.supportsJson) {
          return t("Remote sqlite3 is too old for -json mode. Version {version}. Need ≥ 3.33.", {
            version: remoteStatus.version ?? "?",
          });
        }
        return t("Remote SQLite v{version} · reads & writes apply directly on the server", {
          version: remoteStatus.version ?? "?",
        });
      default:
        return null;
    }
  }, [hasSsh, remoteStatus, t]);

  const showInstallCta = hasSsh && remoteStatus.kind === "missing";

  const extraBody = (
    <div className="form-stack">
      {remoteBannerContent && (
        <div className="status-note mono">{remoteBannerContent}</div>
      )}
      {showInstallCta && (
        <InlineInstallCta
          packageId="sqlite3"
          sshParams={sshParams}
          swKey={swKey}
          enableService={false}
          onInstalled={() => void reprobeSqliteCapability()}
        />
      )}
      {hasSsh && isRemoteMode && (
        <div className="form-stack">
          <label className="field-stack">
            <span className="field-label">
              <FolderSearch size={11} /> {t("Scan remote directory")}
              {shellCwd && (
                <span className="panel-section__hint" style={{ marginLeft: "var(--sp-1)" }}>
                  {t("(shell cwd: {cwd})", { cwd: shortPath(shellCwd) })}
                </span>
              )}
            </span>
            <div className="branch-row">
              <input
                className="field-input mono"
                value={scanInput}
                placeholder={shellCwd ?? "~"}
                onChange={(e) => {
                  setScanInput(e.currentTarget.value);
                  setScanInputTouched(true);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    void scanDir(e.currentTarget.value.trim() || "~");
                  }
                }}
              />
              <button
                type="button"
                className="btn is-ghost is-compact"
                onClick={() => void scanDir(scanInput.trim() || shellCwd || "~")}
              >
                <Search size={10} /> {t("Scan")}
              </button>
            </div>
          </label>
          {candidates.length === 0 && cwdHint && (
            <div className="status-note mono">
              {t("No .db / .sqlite / .sqlite3 files under {dir}", { dir: cwdHint })}
            </div>
          )}
        </div>
      )}
      <label className="field-stack">
        <span className="field-label">
          <HardDrive size={11} />{" "}
          {hasSsh ? t("Database file (remote path)") : t("Database file")}
        </span>
        <div className="branch-row">
          <input
            className="field-input mono"
            onChange={(e) => setManualPath(e.currentTarget.value)}
            placeholder={hasSsh ? "/srv/app/db.sqlite3" : "/path/to/app.db"}
            value={manualPath}
            onKeyDown={(e) => {
              if (e.key === "Enter" && manualPath.trim()) {
                setPath(manualPath.trim());
                void browse("", manualPath.trim());
              }
            }}
          />
          <button
            type="button"
            className="btn is-primary is-compact"
            disabled={!manualPath.trim() || busy}
            onClick={() => {
              setPath(manualPath.trim());
              void browse("", manualPath.trim());
            }}
          >
            {busy ? t("Browsing...") : t("Open")}
          </button>
        </div>
      </label>
      {error && (
        <DismissibleNote variant="status" tone="error" onDismiss={() => setError("")}>
          {error}
        </DismissibleNote>
      )}
    </div>
  );

  if (!state) {
    return (
      <DbConnectSplash
        kind="sqlite"
        probeTarget={probeTarget}
        probeState={probeState}
        onReprobe={undefined}
        detected={detectedRows}
        saved={[]}
        onAddManual={() => {
          /* The manual-path form lives inline in extraBody. */
        }}
        hideAddManual
        description={
          hasSsh
            ? t("Open a database by path, or scan a remote directory for .db / .sqlite files.")
            : t("Open a local SQLite file by path.")
        }
        extraBody={extraBody}
      />
    );
  }

  // ── Connected view ─────────────────────────────────────────
  const currentInstance: DbHeaderInstance = {
    id: "sqlite",
    name: path.split(/[/\\]/).pop() || path || t("SQLite"),
    addr: path,
    via: hasSsh ? t("remote read") : t("local"),
    status: state ? "up" : "unknown",
    sub: <>{path}</>,
  };

  const databases: DbSchemaDatabase[] = [
    {
      name: path.split(/[/\\]/).pop() || t("database"),
      current: true,
      tables: state.tables.map((tname) => ({ id: tname, label: tname })),
    },
  ];

  const pkColumns = state.columns.filter((c) => c.primaryKey).map((c) => c.name);
  const numericColumns = state.columns
    .filter((c) => NUMERIC_TYPE_RE.test(c.colType))
    .map((c) => c.name);
  const gridColumns = gridColumnsFromSqlite(state.columns);

  async function commitMutations(mutations: DbMutation[]) {
    if (!state || mutations.length === 0) return;
    const tableRef = qualifyTable("sqlite", { table: state.tableName });
    setCommitting(true);
    setQueryError("");
    setNotice("");
    try {
      let written = 0;
      for (const mut of mutations) {
        const sql = mutationToSql(
          { dialect: "sqlite", table: tableRef, columns: gridColumns },
          mut,
        );
        if (isRemoteMode && sshTarget) {
          await cmd.sqliteExecuteRemote({
            host: sshTarget.host,
            port: sshTarget.port,
            user: sshTarget.user,
            authMode: sshTarget.authMode,
            password: sshTarget.password,
            keyPath: sshTarget.keyPath,
            savedConnectionIndex: sshTarget.savedConnectionIndex,
            dbPath: path.trim(),
            sql,
          });
        } else {
          await cmd.sqliteExecute(path.trim(), sql);
        }
        written += 1;
      }
      setNotice(t("Committed {n} change(s).", { n: written }));
      void browse(tableName);
    } catch (e) {
      setQueryError(formatError(e));
      throw e;
    } finally {
      setCommitting(false);
    }
  }

  // Structure-edit commit. SQLite needs ≥3.25 for RENAME COLUMN and
  // ≥3.35 for DROP COLUMN; older binaries surface their own parse
  // error verbatim — we don't pre-flight version-check because the
  // capability probe is cached per-session and may go stale.
  // (The `committingDdl` state lives at the top of the component so
  // it is registered on every render, not just when `state !== null`.)
  async function commitStructure(mutations: DdlMutation[]) {
    if (!state || mutations.length === 0) return;
    const tableRef = qualifyTable("sqlite", { table: state.tableName });
    setCommittingDdl(true);
    setQueryError("");
    setNotice("");
    try {
      let written = 0;
      for (const mut of mutations) {
        const sql = ddlToSql({ dialect: "sqlite", table: tableRef }, mut);
        if (isRemoteMode && sshTarget) {
          await cmd.sqliteExecuteRemote({
            host: sshTarget.host,
            port: sshTarget.port,
            user: sshTarget.user,
            authMode: sshTarget.authMode,
            password: sshTarget.password,
            keyPath: sshTarget.keyPath,
            savedConnectionIndex: sshTarget.savedConnectionIndex,
            dbPath: path.trim(),
            sql,
          });
        } else {
          await cmd.sqliteExecute(path.trim(), sql);
        }
        written += 1;
      }
      setNotice(t("Committed {n} structure change(s).", { n: written }));
      void browse(tableName);
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

  async function sqliteRunOne(sql: string): Promise<QueryExecutionResult> {
    const usePath = path.trim();
    if (isRemoteMode && sshTarget) {
      return cmd.sqliteExecuteRemote({
        host: sshTarget.host,
        port: sshTarget.port,
        user: sshTarget.user,
        authMode: sshTarget.authMode,
        password: sshTarget.password,
        keyPath: sshTarget.keyPath,
        savedConnectionIndex: sshTarget.savedConnectionIndex,
        dbPath: usePath,
        sql,
      });
    }
    return cmd.sqliteExecute(usePath, sql);
  }

  const sqliteActions = useMemo<DbSchemaActions>(() => ({
    onCopyTableName: (_db, tables) => {
      void writeClipboardText(tables.join("\n"));
    },
    onRefreshDatabase: () => {
      void browse(tableName);
    },
    // SQLite "databases" are files — `CREATE DATABASE` is meaningless.
    // The user creates a new .db via the connect splash's file picker.
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
                // SQLite has no `TRUNCATE`. The optimised path is
                // `DELETE FROM` (sqlite recognises it without a WHERE
                // and skips the per-row trigger fires when there are
                // none — the so-called "truncate optimisation").
                for (const tbl of tables) {
                  await sqliteRunOne(`DELETE FROM "${tbl.replace(/"/g, '""')}"`);
                }
                setNotice(t("Truncated {n} table(s).", { n: tables.length }));
                void browse(tableName);
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
                  await sqliteRunOne(`DROP TABLE "${tbl.replace(/"/g, '""')}"`);
                }
                setNotice(t("Dropped {n} table(s).", { n: tables.length }));
                void browse("");
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
              await sqliteRunOne(sql);
            }
            setNotice(t("Executed {n} statement(s) from {file}.", {
              n: stmts.length,
              file: picked,
            }));
            void browse(tableName);
          } catch (e) {
            setQueryError(formatError(e));
          }
        },
    onExportTables: async (_db, tables) => {
      const picked = await saveDialog({
        defaultPath: `${tables.length === 1 ? tables[0] : "tables"}.sql`,
        filters: [{ name: "SQL", extensions: ["sql"] }],
      });
      if (typeof picked !== "string") return;
      try {
        const result = await exportTablesAsInserts(
          (sql) => sqliteRunOne(sql),
          "sqlite",
          {},
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
    onExportDatabase: () => {
      const allTables = state?.tables ?? [];
      if (allTables.length === 0) {
        setNotice(t("No tables to export."));
        return;
      }
      void sqliteActions.onExportTables?.("", allTables);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }), [readOnly, state?.tables, tableName, path, isRemoteMode, sshTarget, t]);

  const banner = error ? (
    <DismissibleNote variant="status" tone="error" onDismiss={() => setError("")}>
      {error}
    </DismissibleNote>
  ) : null;

  const resultToolbar = queryResult ? (
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
  ) : null;

  const dataTab = (
    <>
      <DbSqlEditor
        tabName={tableName || t("query")}
        sql={sql}
        onChange={setSql}
        writable={!readOnly}
        onToggleWrite={() => {
          setReadOnly((p) => !p);
          setWriteConfirm("");
        }}
        needsWriteConfirm={Boolean(needsWrite)}
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
      />
      <DbResultGrid
        preview={state.preview}
        pkColumns={pkColumns}
        numericColumns={numericColumns}
        toolbar={resultToolbar}
        emptyLabel={
          state.tableName
            ? t("No rows in this table.")
            : t("Pick a table from the tree to preview rows.")
        }
        columnsMeta={gridColumns}
        writable={!readOnly && state.tableName !== ""}
        onCommit={commitMutations}
        committing={committing}
        onToggleWritable={() => {
          setReadOnly((p) => !p);
          setWriteConfirm("");
        }}
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
        type: c.colType,
        pk: c.primaryKey,
        nullable: !c.notNull,
      }))}
      typeAccentVar="var(--svc-sqlite)"
      indexes={(state.indexes ?? []).map((i) => ({
        name: i.name,
        columns: i.columns,
        unique: i.unique,
        kind: i.origin === "pk" ? "primary key" : i.origin === "u" ? "unique" : "",
      }))}
      triggers={(state.triggers ?? []).map((tr) => ({
        name: tr.name,
        event: tr.event,
        sql: tr.sql,
      }))}
      editable={!readOnly && state.tableName !== ""}
      onCommit={commitStructure}
      committing={committingDdl}
      dialect="sqlite"
    />
  );

  return (
    <>
      {banner && <div className="db-panel-banner db-panel-banner--snug">{banner}</div>}
      <DbConnectedShell
        kind="sqlite"
        current={currentInstance}
        otherInstances={[]}
        onAddConnection={() => disconnect()}
        onDisconnect={() => disconnect()}
        headerStats={[
          { icon: "database" as const, label: t("{count} tables", { count: state.tables.length }) },
          { icon: "disk" as const, label: isRemoteMode ? t("remote") : t("local") },
          ...(state.fileSize > 0
            ? [{ icon: "disk" as const, label: formatBytes(state.fileSize) }]
            : []),
        ]}
        tab={connectedTab}
        onTabChange={setConnectedTab}
        crumb={{
          database: path.split(/[/\\]/).pop() || undefined,
          table: state.tableName || undefined,
          stat: state.preview ? t("{count} rows", { count: state.preview.rows.length }) : null,
        }}
        schema={{
          databases,
          selectedTableId: state.tableName || null,
          onSelectTable: (_db, node) => {
            const tbl = node.label;
            setTableName(tbl);
            sqlTabs.replaceActiveSql(
              `SELECT * FROM "${tbl.replace(/"/g, '""')}" LIMIT 100;`,
              tbl,
            );
            void browse(tbl);
          },
          actions: sqliteActions,
        }}
        dataTab={dataTab}
        structureTab={structureTab}
        schemaTab={
          <DbConfigView
            title={t("SQLite pragmas")}
            note={readOnly ? t("read-only") : t("connection-scoped")}
            load={async () => {
              const PRAGMAS: {
                name: string;
                description: string;
                editable: boolean;
              }[] = [
                // Editable PRAGMAs that take effect on the current
                // connection. Schema/version cookies and application_id
                // are technically writable but rarely intended to be —
                // we leave them read-only here for safety.
                { name: "journal_mode", description: t("Journal mode (delete / wal / memory / …)"), editable: true },
                { name: "synchronous", description: t("Sync level on commit (0 / 1 / 2 / 3)"), editable: true },
                { name: "foreign_keys", description: t("Enforce foreign keys (0 / 1)"), editable: true },
                { name: "page_size", description: t("Page size in bytes"), editable: false },
                { name: "cache_size", description: t("Cache size (pages or KiB)"), editable: true },
                { name: "encoding", description: t("Database text encoding"), editable: false },
                { name: "auto_vacuum", description: t("Auto-vacuum mode (0 / 1 / 2)"), editable: false },
                { name: "user_version", description: t("User-defined schema version"), editable: true },
                { name: "schema_version", description: t("Internal schema cookie"), editable: false },
                { name: "application_id", description: t("Magic application ID"), editable: false },
                { name: "temp_store", description: t("Temp store backing (file / memory)"), editable: true },
                { name: "wal_autocheckpoint", description: t("WAL auto-checkpoint threshold"), editable: true },
              ];
              const usePath = path.trim();
              const runOne = async (sql: string): Promise<QueryExecutionResult> => {
                if (isRemoteMode && sshTarget) {
                  return cmd.sqliteExecuteRemote({
                    host: sshTarget.host,
                    port: sshTarget.port,
                    user: sshTarget.user,
                    authMode: sshTarget.authMode,
                    password: sshTarget.password,
                    keyPath: sshTarget.keyPath,
                    savedConnectionIndex: sshTarget.savedConnectionIndex,
                    dbPath: usePath,
                    sql,
                  });
                }
                return cmd.sqliteExecute(usePath, sql);
              };
              const results = await Promise.all(
                PRAGMAS.map(async (p): Promise<DbConfigRow> => {
                  try {
                    const r = await runOne(`PRAGMA ${p.name};`);
                    return {
                      name: p.name,
                      value: r.rows[0]?.[0] ?? "",
                      description: p.description,
                      source: "PRAGMA",
                      editable: p.editable && !readOnly,
                      editHint: p.editable
                        ? t("connection-scoped")
                        : t("read-only at runtime"),
                    };
                  } catch {
                    return {
                      name: p.name,
                      value: "?",
                      description: p.description,
                      source: "PRAGMA",
                    };
                  }
                }),
              );
              return results;
            }}
            onEdit={
              readOnly
                ? undefined
                : async (name, newValue) => {
                    const usePath = path.trim();
                    // PRAGMA <name> = <value>. Quote unless numeric so
                    // text values (e.g. journal_mode='wal') get the
                    // single quotes the parser expects.
                    const trimmed = newValue.trim();
                    const numeric =
                      trimmed !== "" && Number.isFinite(Number(trimmed));
                    const literal = numeric
                      ? trimmed
                      : `'${newValue.replace(/'/g, "''")}'`;
                    const sql = `PRAGMA ${name} = ${literal};`;
                    if (isRemoteMode && sshTarget) {
                      await cmd.sqliteExecuteRemote({
                        host: sshTarget.host,
                        port: sshTarget.port,
                        user: sshTarget.user,
                        authMode: sshTarget.authMode,
                        password: sshTarget.password,
                        keyPath: sshTarget.keyPath,
                        savedConnectionIndex: sshTarget.savedConnectionIndex,
                        dbPath: usePath,
                        sql,
                      });
                    } else {
                      await cmd.sqliteExecute(usePath, sql);
                    }
                  }
            }
          />
        }
      />
      <ConfirmDialog
        open={!!confirm}
        title={confirm?.title ?? ""}
        message={confirm?.message ?? ""}
        tone={confirm?.tone}
        onCancel={() => setConfirm(null)}
        onConfirm={() => void confirm?.onConfirm()}
      />
    </>
  );
}

function shortPath(p: string): string {
  const parts = p.split("/");
  if (parts.length <= 3) return p;
  return "…/" + parts.slice(-2).join("/");
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
