import { useEffect, useMemo, useState } from "react";
import * as cmd from "../lib/commands";
import { isReadOnlySql, queryResultToTsv } from "../lib/commands";
import { writeClipboardText } from "../lib/clipboard";
import { RIGHT_TOOL_META } from "../lib/rightToolMeta";
import type {
  QueryExecutionResult,
  SqliteBrowserState,
  TabState,
} from "../lib/types";
import type { RemoteSqliteCandidate } from "../lib/commands";
import { effectiveSshTarget } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import DbConnRow from "../components/DbConnRow";
import DismissibleNote from "../components/DismissibleNote";
import PanelHeader from "../components/PanelHeader";
import PreviewTable from "../components/PreviewTable";
import QueryResultPanel from "../components/QueryResultPanel";
import StatusDot from "../components/StatusDot";

type Props = { tab: TabState | null };

/** Tri-state of the remote sqlite3 probe. */
type RemoteStatus =
  | { kind: "unknown" }
  | { kind: "local-only" }
  | { kind: "installed"; supportsJson: boolean; version: string | null }
  | { kind: "missing" };

export default function SqlitePanel({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);

  const sshTarget = tab ? effectiveSshTarget(tab) : null;
  const hasSsh = sshTarget !== null;

  const [path, setPath] = useState("");
  const [tableName, setTableName] = useState("");
  const [sql, setSql] = useState(
    "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;",
  );
  const [readOnly, setReadOnly] = useState(true);
  const [writeConfirm, setWriteConfirm] = useState("");
  const [state, setState] = useState<SqliteBrowserState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [queryResult, setQueryResult] = useState<QueryExecutionResult | null>(null);
  const [queryBusy, setQueryBusy] = useState(false);
  const [queryError, setQueryError] = useState("");
  const [notice, setNotice] = useState("");

  // Remote discovery
  const [remoteStatus, setRemoteStatus] = useState<RemoteStatus>(
    hasSsh ? { kind: "unknown" } : { kind: "local-only" },
  );
  const [candidates, setCandidates] = useState<RemoteSqliteCandidate[]>([]);
  const [cwdHint, setCwdHint] = useState(""); // directory we scanned
  const [shellCwd, setShellCwd] = useState<string | null>(null);
  // `scanInput` is the controlled value of the scan-directory
  // input. It defaults to "" so the first OSC 7 update seeds it
  // (via the effect below); subsequent `shellCwd` updates don't
  // overwrite what the user has typed. Reset to empty when tab
  // changes to re-enable auto-seeding.
  const [scanInput, setScanInput] = useState("");
  const [scanInputTouched, setScanInputTouched] = useState(false);

  // Poll the terminal session for OSC 7 CWD updates. We fire
  // once on mount to catch an already-reported CWD, then refresh
  // every 15 s — OSC 7 is prompt-driven (shells emit it on every
  // new prompt), so a short interval just adds lock contention
  // for no UX gain. The SqlitePanel only needs it as a scan
  // default; users who care about precision can type their own
  // path.
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

  // Seed the scan-directory input from the shell CWD the FIRST
  // time we learn it, and only if the user hasn't typed anything
  // yet. This avoids clobbering a half-typed path when OSC 7
  // fires mid-edit.
  useEffect(() => {
    if (!scanInputTouched && shellCwd && scanInput !== shellCwd) {
      setScanInput(shellCwd);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shellCwd, scanInputTouched]);

  // When SSH context changes, re-probe sqlite3 capability.
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
  const needsWrite = sql.trim() && !isReadOnlySql(sql);
  const canRun =
    canBrowse &&
    sql.trim() &&
    !queryBusy &&
    (!needsWrite || (!readOnly && writeConfirm.trim().toUpperCase() === "WRITE"));

  const isRemoteMode =
    hasSsh && remoteStatus.kind === "installed" && remoteStatus.supportsJson;

  async function browse(nextTable = tableName) {
    setBusy(true);
    setError("");
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
          dbPath: path.trim(),
          table: nextTable.trim() || null,
        });
        setState(s);
        setTableName(s.tableName);
      } else {
        const s = await cmd.sqliteBrowse(path.trim(), nextTable.trim() || null);
        setState(s);
        setTableName(s.tableName);
      }
    } catch (e) {
      // Keep prior state visible so a transient browse error
      // doesn't blank the panel body.
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
      if (isRemoteMode && sshTarget) {
        const r = await cmd.sqliteExecuteRemote({
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
        setQueryResult(r);
        setNotice(t("{elapsed} ms", { elapsed: r.elapsedMs }));
      } else {
        const r = await cmd.sqliteExecute(path.trim(), sql);
        setQueryResult(r);
        setNotice(t("{elapsed} ms", { elapsed: r.elapsedMs }));
      }
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

  const trimmedPath = path.trim();
  const connName = trimmedPath || t("SQLite Browser");
  const connSub = trimmedPath
    ? hasSsh
      ? `${sshTarget?.user}@${sshTarget?.host}:${trimmedPath}`
      : trimmedPath
    : t("Not connected");
  const dbFileName = trimmedPath
    ? trimmedPath.split(/[/\\]/).pop() || trimmedPath
    : "";
  const headerMeta = dbFileName
    ? state
      ? t("{file} · {count} tables", { file: dbFileName, count: state.tables.length })
      : dbFileName
    : t("No database");
  const connTag = (
    <>
      <StatusDot tone={state ? "pos" : "off"} />
      {state ? t("open") : t("offline")}
    </>
  );

  const remoteBanner = useMemo(() => {
    if (!hasSsh) return null;
    switch (remoteStatus.kind) {
      case "unknown":
        return null;
      case "missing":
        return (
          <div className="status-note status-note--warn">
            {t("Remote sqlite3 not found — install `sqlite3` on the server to read remote .db files directly.")}
          </div>
        );
      case "installed":
        if (!remoteStatus.supportsJson) {
          return (
            <div className="status-note status-note--warn">
              {t("Remote sqlite3 is too old for -json mode. Version {version}. Need ≥ 3.33.", {
                version: remoteStatus.version ?? "?",
              })}
            </div>
          );
        }
        return (
          <div className="status-note">
            {t("Remote SQLite v{version} · reads & writes apply directly on the server", {
              version: remoteStatus.version ?? "?",
            })}
          </div>
        );
      default:
        return null;
    }
  }, [hasSsh, remoteStatus, t]);

  return (
    <>
      <PanelHeader
        icon={RIGHT_TOOL_META.sqlite.icon}
        title={t("SQLite")}
        meta={headerMeta}
      />
      <DbConnRow
        icon={RIGHT_TOOL_META.sqlite.icon}
        tint="var(--panel-2)"
        iconTint="var(--ink-2)"
        name={connName}
        sub={connSub}
        tag={connTag}
      />
      <div className="panel-scroll">
        <section className="panel-section">
          <div className="form-stack">
            {remoteBanner}
            {hasSsh && isRemoteMode && (
              <div className="form-stack">
                <div className="field-grid">
                  <label className="field-stack">
                    <span className="field-label">
                      {t("Scan directory")}
                      {shellCwd && (
                        <span
                          className="panel-section__hint"
                          style={{ marginLeft: "var(--sp-1)" }}
                        >
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
                        className="mini-button"
                        onClick={() => {
                          void scanDir(scanInput.trim() || shellCwd || "~");
                        }}
                        type="button"
                      >
                        {t("Scan")}
                      </button>
                    </div>
                  </label>
                </div>
                {candidates.length > 0 && (
                  <div className="token-list">
                    {candidates.map((c) => (
                      <button
                        key={c.path}
                        className={
                          path === c.path
                            ? "token-button token-button--selected"
                            : "token-button"
                        }
                        onClick={() => {
                          setPath(c.path);
                          setState(null);
                          setTableName("");
                          void browse("");
                        }}
                        title={`${formatBytes(c.sizeBytes)}${cwdHint ? ` · ${cwdHint}` : ""}`}
                        type="button"
                      >
                        {shortPath(c.path)}
                        <span className="db-instance-pill__port">
                          {" "}
                          · {formatBytes(c.sizeBytes)}
                        </span>
                      </button>
                    ))}
                  </div>
                )}
                {candidates.length === 0 && cwdHint && (
                  <div className="status-note mono">
                    {t("No .db / .sqlite / .sqlite3 files under {dir}", { dir: cwdHint })}
                  </div>
                )}
              </div>
            )}
            <label className="field-stack">
              <span className="field-label">
                {hasSsh ? t("Database file (remote path)") : t("Database file")}
              </span>
              <div className="branch-row">
                <input
                  className="field-input"
                  onChange={(e) => setPath(e.currentTarget.value)}
                  placeholder={
                    hasSsh ? t("/srv/app/db.sqlite3") : t("/path/to/app.db")
                  }
                  value={path}
                />
                <button
                  className="mini-button"
                  disabled={!canBrowse || busy}
                  onClick={() => void browse()}
                  type="button"
                >
                  {busy ? t("Browsing...") : t("Browse")}
                </button>
              </div>
            </label>
            {error && (
              <DismissibleNote variant="status" tone="error" onDismiss={() => setError("")}>
                {error}
              </DismissibleNote>
            )}
          </div>
        </section>

        {state && (
          <section className="panel-section">
            <div className="panel-section__title">
              <span>{t("Tables & Columns")}</span>
            </div>
            <div className="form-stack">
              <div className="token-list">
                {state.tables.map((tbl) => (
                  <button
                    key={tbl}
                    className={
                      state.tableName === tbl
                        ? "token-button token-button--selected"
                        : "token-button"
                    }
                    onClick={() => {
                      setTableName(tbl);
                      setSql(`SELECT * FROM "${tbl.replace(/"/g, '""')}" LIMIT 100;`);
                      void browse(tbl);
                    }}
                    type="button"
                  >
                    {tbl}
                  </button>
                ))}
              </div>
              {state.columns.length > 0 && (
                <div className="column-list">
                  {state.columns.map((col) => (
                    <div className="column-row" key={col.name}>
                      <div className="column-row__head">
                        <strong>{col.name}</strong>
                        <span className="connection-pill">{col.colType}</span>
                      </div>
                      <div className="column-row__meta">
                        {col.notNull ? t("Not null") : t("Nullable")}
                        {col.primaryKey ? ` · ${t("PK")}` : ""}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </section>
        )}

        {state && (
          <section className="panel-section">
            <div className="panel-section__title">
              <span>{t("Sample Rows")}</span>
            </div>
            <PreviewTable preview={state.preview} emptyLabel={t("Select a table.")} />
          </section>
        )}

        <section className="panel-section">
          <div className="panel-section__title">
            <span>{t("Query Editor")}</span>
          </div>
          <div className="form-stack">
            <div className="query-guard-row">
              <span
                className={
                  readOnly ? "safety-pill safety-pill--locked" : "safety-pill safety-pill--unlocked"
                }
              >
                {readOnly ? t("Read Only") : t("Writes Unlocked")}
              </span>
              <button
                className="mini-button"
                onClick={() => {
                  setReadOnly((p) => !p);
                  setWriteConfirm("");
                }}
                type="button"
              >
                {readOnly ? t("Unlock Writes") : t("Re-lock Writes")}
              </button>
            </div>
            <textarea
              className="field-textarea field-textarea--editor"
              onChange={(e) => setSql(e.currentTarget.value)}
              rows={4}
              value={sql}
            />
            {needsWrite && !readOnly && (
              <input
                className="field-input"
                onChange={(e) => setWriteConfirm(e.currentTarget.value)}
                placeholder={t("Type WRITE to confirm")}
                value={writeConfirm}
              />
            )}
            <div className="button-row">
              <button
                className="mini-button"
                disabled={!canRun}
                onClick={() => void runQuery()}
                type="button"
              >
                {queryBusy ? t("Running...") : t("Run Query")}
              </button>
              {queryResult && (
                <button
                  className="mini-button"
                  onClick={() => {
                    void writeClipboardText(queryResultToTsv(queryResult));
                    setNotice(t("Copied"));
                  }}
                  type="button"
                >
                  {t("Copy TSV")}
                </button>
              )}
            </div>
            {notice && <div className="status-note">{notice}</div>}
          </div>
        </section>

        <section className="panel-section">
          <div className="panel-section__title">
            <span>{t("Query Results")}</span>
          </div>
          <QueryResultPanel
            result={queryResult}
            error={queryError}
            emptyLabel={t("Run a query.")}
          />
        </section>
      </div>
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
