import { FolderSearch, HardDrive, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

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
import { useDbSqlTabs } from "../components/db/useDbSqlTabs";
import {
  gridColumnsFromSqlite,
  mutationToSql,
  qualifyTable,
  type DbMutation,
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

  const [path, setPath] = useState("");
  const [tableName, setTableName] = useState("");
  const sqlTabs = useDbSqlTabs({
    initialSql: "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;",
    initialName: t("query"),
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
      } else {
        r = await cmd.sqliteExecute(path.trim(), sql);
      }
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
      void browse(tableName);
    } catch (e) {
      setQueryResult(null);
      setQueryError(formatError(e));
    } finally {
      setQueryBusy(false);
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

  const extraBody = (
    <div className="form-stack">
      {remoteBannerContent && (
        <div className="status-note mono">{remoteBannerContent}</div>
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
          { icon: "database", label: t("{count} tables", { count: state.tables.length }) },
          { icon: "disk", label: isRemoteMode ? t("remote") : t("local") },
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
        }}
        dataTab={dataTab}
        structureTab={structureTab}
        schemaTab={
          <DbConfigView
            title={t("SQLite pragmas")}
            note={t("read-only")}
            load={async () => {
              const PRAGMAS: { name: string; description: string }[] = [
                { name: "journal_mode", description: t("Journal mode (delete / wal / memory / …)") },
                { name: "synchronous", description: t("Sync level on commit (0 / 1 / 2 / 3)") },
                { name: "foreign_keys", description: t("Enforce foreign keys (0 / 1)") },
                { name: "page_size", description: t("Page size in bytes") },
                { name: "cache_size", description: t("Cache size (pages or KiB)") },
                { name: "encoding", description: t("Database text encoding") },
                { name: "auto_vacuum", description: t("Auto-vacuum mode (0 / 1 / 2)") },
                { name: "user_version", description: t("User-defined schema version") },
                { name: "schema_version", description: t("Internal schema cookie") },
                { name: "application_id", description: t("Magic application ID") },
                { name: "temp_store", description: t("Temp store backing (file / memory)") },
                { name: "wal_autocheckpoint", description: t("WAL auto-checkpoint threshold") },
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
          />
        }
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
