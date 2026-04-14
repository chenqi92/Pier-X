import { invoke } from "@tauri-apps/api/core";
import {
  ActivitySquare,
  ChevronUp,
  Command,
  Database,
  FolderTree,
  GitBranch,
  History,
  LayoutDashboard,
  PlugZap,
  Search,
  SquareTerminal,
} from "lucide-react";
import {
  startTransition,
  useDeferredValue,
  useEffect,
  useRef,
  useState,
} from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import "./App.css";

type CoreInfo = {
  version: string;
  profile: string;
  uiTarget: string;
  workspaceRoot: string;
  defaultShell: string;
  services: string[];
};

type FileEntry = {
  name: string;
  path: string;
  kind: "directory" | "file";
  sizeLabel: string;
};

type GitChangeEntry = {
  path: string;
  status: string;
  staged: boolean;
};

type GitOverview = {
  repoPath: string;
  branchName: string;
  tracking: string;
  ahead: number;
  behind: number;
  isClean: boolean;
  stagedCount: number;
  unstagedCount: number;
  changes: GitChangeEntry[];
};

type GitCommitEntry = {
  hash: string;
  shortHash: string;
  message: string;
  author: string;
  relativeDate: string;
  refs: string;
};

type GitStashEntry = {
  index: string;
  message: string;
  relativeDate: string;
};

type SavedSshConnection = {
  index: number;
  name: string;
  host: string;
  port: number;
  user: string;
  authKind: "password" | "agent" | "key";
  keyPath: string;
};

type DataPreview = {
  columns: string[];
  rows: string[][];
  truncated: boolean;
};

type QueryExecutionResult = {
  columns: string[];
  rows: string[][];
  truncated: boolean;
  affectedRows: number;
  lastInsertId: number | null;
  elapsedMs: number;
};

type MysqlColumnView = {
  name: string;
  columnType: string;
  nullable: boolean;
  key: string;
  defaultValue: string;
  extra: string;
};

type MysqlBrowserState = {
  databaseName: string;
  databases: string[];
  tableName: string;
  tables: string[];
  columns: MysqlColumnView[];
  preview: DataPreview | null;
};

type SqliteColumnView = {
  name: string;
  colType: string;
  notNull: boolean;
  primaryKey: boolean;
};

type SqliteBrowserState = {
  path: string;
  tableName: string;
  tables: string[];
  columns: SqliteColumnView[];
  preview: DataPreview | null;
};

type RedisKeyView = {
  key: string;
  kind: string;
  length: number;
  ttlSeconds: number;
  encoding: string;
  preview: string[];
  previewTruncated: boolean;
};

type RedisBrowserState = {
  pong: string;
  pattern: string;
  limit: number;
  truncated: boolean;
  keyName: string;
  keys: string[];
  serverVersion: string;
  usedMemory: string;
  details: RedisKeyView | null;
};

type RedisCommandResult = {
  summary: string;
  lines: string[];
  elapsedMs: number;
};

type DataSurface = "mysql" | "sqlite" | "redis";

type TerminalSessionInfo = {
  sessionId: string;
  shell: string;
  cols: number;
  rows: number;
};

type TerminalSegment = {
  text: string;
  fg: string;
  bg: string;
  bold: boolean;
  underline: boolean;
  cursor: boolean;
};

type TerminalLine = {
  segments: TerminalSegment[];
};

type TerminalSnapshot = {
  cols: number;
  rows: number;
  alive: boolean;
  scrollbackLen: number;
  lines: TerminalLine[];
};

type TerminalSize = {
  cols: number;
  rows: number;
};

type TerminalTarget =
  | {
      kind: "local";
    }
  | {
      kind: "sshSaved";
      index: number;
      label: string;
    }
  | {
      kind: "ssh";
      host: string;
      port: number;
      user: string;
      authMode: "password" | "agent" | "key";
      password?: string;
      keyPath?: string;
    };

const serviceDescriptions: Record<string, string> = {
  terminal: "pier-core 会话层已接入，当前通过轮询快照驱动渲染。",
  ssh: "SSH 终端入口已接入，同一套网格渲染现在可以承载远程 shell。",
  git: "仓库状态已直接复用 pier-core GitClient，同步展示当前分支与变更。",
  mysql: "MySQL 连接和 schema 浏览已接上，下一步补 SQL editor 与结果表。",
  sqlite: "本地 SQLite 文件浏览已接上，当前可读表结构和样本数据。",
  redis: "Redis / Valkey 浏览面板已接上，当前可扫 key 并查看基础预览。",
};

const controlKeyMap: Record<string, string> = {
  "@": "\u0000",
  "[": "\u001b",
  "\\": "\u001c",
  "]": "\u001d",
  "^": "\u001e",
  _: "\u001f",
};

const readOnlySqlKeywords = new Set([
  "SELECT",
  "SHOW",
  "DESCRIBE",
  "DESC",
  "EXPLAIN",
  "PRAGMA",
  "VALUES",
  "WITH",
  "USE",
]);

function leadingSqlKeyword(sql: string) {
  let remaining = sql.trimStart();

  while (remaining) {
    if (remaining.startsWith("--")) {
      const newlineIndex = remaining.indexOf("\n");
      if (newlineIndex === -1) {
        return "";
      }
      remaining = remaining.slice(newlineIndex + 1).trimStart();
      continue;
    }

    if (remaining.startsWith("/*")) {
      const commentEnd = remaining.indexOf("*/", 2);
      if (commentEnd === -1) {
        return "";
      }
      remaining = remaining.slice(commentEnd + 2).trimStart();
      continue;
    }

    break;
  }

  const match = /^[A-Za-z]+/.exec(remaining);
  return match ? match[0].toUpperCase() : "";
}

function isReadOnlySql(sql: string) {
  const keyword = leadingSqlKeyword(sql);
  return keyword !== "" && readOnlySqlKeywords.has(keyword);
}

function queryResultToTsv(result: QueryExecutionResult) {
  const normalizeCell = (value: string) =>
    value.replace(/\t/g, " ").replace(/\r?\n/g, " ");
  const lines: string[] = [];
  if (result.columns.length > 0) {
    lines.push(result.columns.map(normalizeCell).join("\t"));
  }
  for (const row of result.rows) {
    lines.push(row.map(normalizeCell).join("\t"));
  }
  return lines.join("\n");
}

function PreviewTable({
  emptyLabel,
  preview,
}: {
  emptyLabel: string;
  preview: DataPreview | null;
}) {
  if (!preview || preview.columns.length === 0) {
    return <div className="empty-note">{emptyLabel}</div>;
  }

  return (
    <div className="data-table-wrap">
      <table className="data-table">
        <thead>
          <tr>
            {preview.columns.map((column) => (
              <th key={column}>{column}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {preview.rows.map((row, rowIndex) => (
            <tr key={`${rowIndex}-${row.join("|")}`}>
              {row.map((cell, cellIndex) => (
                <td key={`${rowIndex}-${cellIndex}`}>{cell}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {preview.truncated ? (
        <div className="inline-note">Preview truncated to keep the workbench responsive.</div>
      ) : null}
    </div>
  );
}

function quoteCommandArg(value: string) {
  return JSON.stringify(value);
}

function QueryResultPanel({
  emptyLabel,
  error,
  result,
}: {
  emptyLabel: string;
  error: string;
  result: QueryExecutionResult | null;
}) {
  if (error) {
    return <div className="status-note status-note--error">{error}</div>;
  }

  if (!result) {
    return <div className="empty-note">{emptyLabel}</div>;
  }

  return (
    <div className="form-stack">
      <div className="data-meta-grid">
        <div className="meta-chip">
          <span>Rows</span>
          <strong>{result.rows.length}</strong>
        </div>
        <div className="meta-chip">
          <span>Affected</span>
          <strong>{result.affectedRows}</strong>
        </div>
        <div className="meta-chip">
          <span>Elapsed</span>
          <strong>{result.elapsedMs} ms</strong>
        </div>
        <div className="meta-chip">
          <span>Insert Id</span>
          <strong>{result.lastInsertId ?? "--"}</strong>
        </div>
      </div>
      {result.columns.length > 0 ? (
        <PreviewTable
          emptyLabel={emptyLabel}
          preview={{
            columns: result.columns,
            rows: result.rows,
            truncated: result.truncated,
          }}
        />
      ) : (
        <div className="status-note">
          Statement completed without a tabular result set.
          {result.truncated ? " Result was truncated." : ""}
        </div>
      )}
    </div>
  );
}

function RedisCommandPanel({
  emptyLabel,
  error,
  result,
}: {
  emptyLabel: string;
  error: string;
  result: RedisCommandResult | null;
}) {
  if (error) {
    return <div className="status-note status-note--error">{error}</div>;
  }

  if (!result) {
    return <div className="empty-note">{emptyLabel}</div>;
  }

  return (
    <div className="form-stack">
      <div className="data-meta-grid">
        <div className="meta-chip meta-chip--wide">
          <span>Reply</span>
          <strong>{result.summary}</strong>
        </div>
        <div className="meta-chip">
          <span>Elapsed</span>
          <strong>{result.elapsedMs} ms</strong>
        </div>
      </div>
      <div className="preview-list">
        {result.lines.map((line, index) => (
          <div className="preview-item" key={`${index}-${line}`}>
            {line}
          </div>
        ))}
      </div>
    </div>
  );
}

function App() {
  const [coreInfo, setCoreInfo] = useState<CoreInfo | null>(null);
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [browserPath, setBrowserPath] = useState("");
  const [searchText, setSearchText] = useState("");
  const [gitOverview, setGitOverview] = useState<GitOverview | null>(null);
  const [gitError, setGitError] = useState("");
  const [selectedChangeKey, setSelectedChangeKey] = useState("");
  const [gitDiffText, setGitDiffText] = useState("");
  const [gitDiffError, setGitDiffError] = useState("");
  const [gitDiffLoading, setGitDiffLoading] = useState(false);
  const [gitActionBusy, setGitActionBusy] = useState(false);
  const [gitBranches, setGitBranches] = useState<string[]>([]);
  const [recentCommits, setRecentCommits] = useState<GitCommitEntry[]>([]);
  const [gitStashes, setGitStashes] = useState<GitStashEntry[]>([]);
  const [commitMessage, setCommitMessage] = useState("");
  const [stashMessage, setStashMessage] = useState("");
  const [branchSwitchTarget, setBranchSwitchTarget] = useState("");
  const [gitNotice, setGitNotice] = useState("");
  const [gitOperationError, setGitOperationError] = useState("");
  const [terminalTarget, setTerminalTarget] = useState<TerminalTarget>({
    kind: "local",
  });
  const [dataSurface, setDataSurface] = useState<DataSurface>("mysql");
  const [sshAuthMode, setSshAuthMode] = useState<"password" | "agent" | "key">(
    "password",
  );
  const [sshHost, setSshHost] = useState("");
  const [sshPort, setSshPort] = useState("22");
  const [sshUser, setSshUser] = useState("");
  const [sshPassword, setSshPassword] = useState("");
  const [sshKeyPath, setSshKeyPath] = useState("");
  const [sshConnectionName, setSshConnectionName] = useState("");
  const [savedConnections, setSavedConnections] = useState<SavedSshConnection[]>([]);
  const [sshConnectionsError, setSshConnectionsError] = useState("");
  const [sshConnectionsNotice, setSshConnectionsNotice] = useState("");
  const [mysqlHost, setMysqlHost] = useState("127.0.0.1");
  const [mysqlPort, setMysqlPort] = useState("3306");
  const [mysqlUser, setMysqlUser] = useState("root");
  const [mysqlPassword, setMysqlPassword] = useState("");
  const [mysqlDatabaseName, setMysqlDatabaseName] = useState("");
  const [mysqlTableName, setMysqlTableName] = useState("");
  const [mysqlSql, setMysqlSql] = useState("SHOW TABLES;");
  const [mysqlReadOnly, setMysqlReadOnly] = useState(true);
  const [mysqlWriteConfirm, setMysqlWriteConfirm] = useState("");
  const [mysqlState, setMysqlState] = useState<MysqlBrowserState | null>(null);
  const [mysqlBusy, setMysqlBusy] = useState(false);
  const [mysqlError, setMysqlError] = useState("");
  const [mysqlQueryResult, setMysqlQueryResult] = useState<QueryExecutionResult | null>(null);
  const [mysqlQueryBusy, setMysqlQueryBusy] = useState(false);
  const [mysqlQueryError, setMysqlQueryError] = useState("");
  const [mysqlQueryNotice, setMysqlQueryNotice] = useState("");
  const [sqlitePath, setSqlitePath] = useState("");
  const [sqliteTableName, setSqliteTableName] = useState("");
  const [sqliteSql, setSqliteSql] = useState(
    "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name;",
  );
  const [sqliteReadOnly, setSqliteReadOnly] = useState(true);
  const [sqliteWriteConfirm, setSqliteWriteConfirm] = useState("");
  const [sqliteState, setSqliteState] = useState<SqliteBrowserState | null>(null);
  const [sqliteBusy, setSqliteBusy] = useState(false);
  const [sqliteError, setSqliteError] = useState("");
  const [sqliteQueryResult, setSqliteQueryResult] = useState<QueryExecutionResult | null>(null);
  const [sqliteQueryBusy, setSqliteQueryBusy] = useState(false);
  const [sqliteQueryError, setSqliteQueryError] = useState("");
  const [sqliteQueryNotice, setSqliteQueryNotice] = useState("");
  const [redisHost, setRedisHost] = useState("127.0.0.1");
  const [redisPort, setRedisPort] = useState("6379");
  const [redisDb, setRedisDb] = useState("0");
  const [redisPattern, setRedisPattern] = useState("*");
  const [redisKeyName, setRedisKeyName] = useState("");
  const [redisCommand, setRedisCommand] = useState("PING");
  const [redisState, setRedisState] = useState<RedisBrowserState | null>(null);
  const [redisBusy, setRedisBusy] = useState(false);
  const [redisError, setRedisError] = useState("");
  const [redisCommandResult, setRedisCommandResult] = useState<RedisCommandResult | null>(null);
  const [redisCommandBusy, setRedisCommandBusy] = useState(false);
  const [redisCommandError, setRedisCommandError] = useState("");
  const [terminalSession, setTerminalSession] = useState<TerminalSessionInfo | null>(null);
  const [terminalSnapshot, setTerminalSnapshot] = useState<TerminalSnapshot | null>(null);
  const [terminalError, setTerminalError] = useState("");
  const [terminalSize, setTerminalSize] = useState<TerminalSize>({
    cols: 120,
    rows: 26,
  });
  const [scrollbackOffset, setScrollbackOffset] = useState(0);
  const [error, setError] = useState("");
  const terminalViewportRef = useRef<HTMLDivElement | null>(null);
  const terminalMeasureRef = useRef<HTMLSpanElement | null>(null);
  const deferredSearchText = useDeferredValue(searchText);

  function makeChangeKey(path: string, staged: boolean) {
    return `${path}:${staged ? "staged" : "worktree"}`;
  }

  function changeKey(change: GitChangeEntry) {
    return makeChangeKey(change.path, change.staged);
  }

  function hasUntrackedStatus(change: GitChangeEntry) {
    return change.status.includes("?") && !change.staged;
  }

  function getTerminalSelectionText() {
    const viewport = terminalViewportRef.current;
    const selection = window.getSelection();
    if (!viewport || !selection || selection.rangeCount === 0 || selection.isCollapsed) {
      return "";
    }

    const anchorNode = selection.anchorNode;
    const focusNode = selection.focusNode;
    if (
      !anchorNode ||
      !focusNode ||
      !viewport.contains(anchorNode) ||
      !viewport.contains(focusNode)
    ) {
      return "";
    }

    return selection.toString();
  }

  async function copyTerminalSelection(text: string) {
    if (!text) {
      return false;
    }

    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      return false;
    }
  }

  async function pasteIntoTerminal() {
    try {
      const text = await navigator.clipboard.readText();
      if (!text) {
        return;
      }
      await sendTerminalInput(text.replace(/\r?\n/g, "\r"));
    } catch {
      setTerminalError("Clipboard paste is unavailable in this runtime.");
    }
  }

  async function copyQueryResult(
    label: string,
    result: QueryExecutionResult | null,
    setNotice: (value: string) => void,
    setError: (value: string) => void,
  ) {
    if (!result) {
      return;
    }

    try {
      await navigator.clipboard.writeText(queryResultToTsv(result));
      setNotice(`Copied ${label} results as TSV.`);
      setError("");
    } catch {
      setNotice("");
      setError("Clipboard copy is unavailable in this runtime.");
    }
  }

  async function refreshGitOverviewState(preferredKey?: string, preferredPath?: string) {
    if (!browserPath) {
      return null;
    }

    try {
      const nextGitOverview = await invoke<GitOverview>("git_overview", {
        path: browserPath,
      });
      setGitOverview(nextGitOverview);
      setGitError("");

      if (!nextGitOverview.changes.length) {
        setSelectedChangeKey("");
        return nextGitOverview;
      }

      if (
        preferredKey &&
        nextGitOverview.changes.some((change) => changeKey(change) === preferredKey)
      ) {
        setSelectedChangeKey(preferredKey);
        return nextGitOverview;
      }

      if (preferredPath) {
        const matchingPath = nextGitOverview.changes.find(
          (change) => change.path === preferredPath,
        );
        if (matchingPath) {
          setSelectedChangeKey(changeKey(matchingPath));
          return nextGitOverview;
        }
      }

      setSelectedChangeKey(changeKey(nextGitOverview.changes[0]));
      return nextGitOverview;
    } catch (nextError) {
      setGitOverview(null);
      setGitError(String(nextError));
      setSelectedChangeKey("");
      return null;
    }
  }

  async function refreshGitDetailState(preferredBranch?: string) {
    if (!browserPath || !gitOverview) {
      setGitBranches([]);
      setRecentCommits([]);
      setGitStashes([]);
      setBranchSwitchTarget("");
      return;
    }

    const [nextBranches, nextCommits, nextStashes] = await Promise.all([
      invoke<string[]>("git_branch_list", { path: browserPath }),
      invoke<GitCommitEntry[]>("git_recent_commits", { path: browserPath, limit: 8 }),
      invoke<GitStashEntry[]>("git_stash_list", { path: browserPath }),
    ]);

    setGitBranches(nextBranches);
    setRecentCommits(nextCommits);
    setGitStashes(nextStashes);
    setBranchSwitchTarget((previous) => {
      if (preferredBranch && nextBranches.includes(preferredBranch)) {
        return preferredBranch;
      }
      if (previous && nextBranches.includes(previous)) {
        return previous;
      }
      if (gitOverview.branchName && nextBranches.includes(gitOverview.branchName)) {
        return gitOverview.branchName;
      }
      return nextBranches[0] ?? "";
    });
  }

  async function refreshSavedSshConnections() {
    try {
      const nextConnections = await invoke<SavedSshConnection[]>("ssh_connections_list");
      setSavedConnections(nextConnections);
      setSshConnectionsError("");
    } catch (nextError) {
      setSavedConnections([]);
      setSshConnectionsError(String(nextError));
    }
  }

  useEffect(() => {
    let cancelled = false;

    async function bootstrap() {
      try {
        const nextCoreInfo = await invoke<CoreInfo>("core_info");
        if (cancelled) {
          return;
        }
        setCoreInfo(nextCoreInfo);
        setBrowserPath(nextCoreInfo.workspaceRoot);
        void refreshSavedSshConnections();
      } catch (nextError) {
        if (!cancelled) {
          setError(String(nextError));
        }
      }
    }

    void bootstrap();

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!browserPath) {
      return;
    }

    let cancelled = false;

    async function refreshWorkbench() {
      try {
        const [nextEntries, nextGitOverview] = await Promise.all([
          invoke<FileEntry[]>("list_directory", { path: browserPath }),
          invoke<GitOverview>("git_overview", { path: browserPath }),
        ]);
        if (cancelled) {
          return;
        }
        setEntries(nextEntries);
        setGitOverview(nextGitOverview);
        setGitError("");
      } catch (nextError) {
        if (cancelled) {
          return;
        }

        const nextEntries = await invoke<FileEntry[]>("list_directory", {
          path: browserPath,
        }).catch(() => []);
        if (!cancelled) {
          setEntries(nextEntries);
          setGitOverview(null);
          setGitError(String(nextError));
        }
      }
    }

    void refreshWorkbench();

    return () => {
      cancelled = true;
    };
  }, [browserPath]);

  useEffect(() => {
    if (!gitOverview) {
      setGitBranches([]);
      setRecentCommits([]);
      setGitStashes([]);
      setBranchSwitchTarget("");
      return;
    }

    const currentOverview = gitOverview;
    let cancelled = false;

    async function refreshGitDetails() {
      try {
        await refreshGitDetailState(currentOverview.branchName);
      } catch (nextError) {
        if (!cancelled) {
          setGitOperationError(String(nextError));
        }
      }
    }

    void refreshGitDetails();

    return () => {
      cancelled = true;
    };
  }, [browserPath, gitOverview?.repoPath, gitOverview?.branchName]);

  useEffect(() => {
    if (!gitOverview?.changes.length) {
      if (selectedChangeKey) {
        setSelectedChangeKey("");
      }
      return;
    }

    if (!selectedChangeKey) {
      setSelectedChangeKey(changeKey(gitOverview.changes[0]));
      return;
    }

    const hasSelection = gitOverview.changes.some(
      (change) => changeKey(change) === selectedChangeKey,
    );
    if (!hasSelection) {
      setSelectedChangeKey(changeKey(gitOverview.changes[0]));
    }
  }, [gitOverview, selectedChangeKey]);

  useEffect(() => {
    if (!browserPath) {
      return;
    }

    let disposed = false;
    const intervalId = window.setInterval(() => {
      void invoke<GitOverview>("git_overview", { path: browserPath })
        .then((nextGitOverview) => {
          if (!disposed) {
            setGitOverview(nextGitOverview);
            setGitError("");
          }
        })
        .catch((nextError) => {
          if (!disposed) {
            setGitOverview(null);
            setGitError(String(nextError));
          }
        });
    }, 3000);

    return () => {
      disposed = true;
      window.clearInterval(intervalId);
    };
  }, [browserPath]);

  useEffect(() => {
    const viewport = terminalViewportRef.current;
    const measure = terminalMeasureRef.current;
    if (!viewport || !measure) {
      return;
    }

    const recalculate = () => {
      const measureBox = measure.getBoundingClientRect();
      const charWidth = measureBox.width / 10 || 7.8;
      const charHeight = measureBox.height || 19;
      const cols = Math.max(
        48,
        Math.min(220, Math.floor((viewport.clientWidth - 24) / charWidth)),
      );
      const rows = Math.max(
        14,
        Math.min(72, Math.floor((viewport.clientHeight - 20) / charHeight)),
      );
      setTerminalSize((previous) =>
        previous.cols === cols && previous.rows === rows
          ? previous
          : { cols, rows },
      );
    };

    recalculate();
    const observer = new ResizeObserver(recalculate);
    observer.observe(viewport);

    return () => {
      observer.disconnect();
    };
  }, []);

  useEffect(() => {
    if (terminalSession) {
      return;
    }

    let cancelled = false;

    async function createTerminal() {
      try {
        const nextSession =
          terminalTarget.kind === "sshSaved"
            ? await invoke<TerminalSessionInfo>("terminal_create_ssh_saved", {
                cols: terminalSize.cols,
                rows: terminalSize.rows,
                index: terminalTarget.index,
              })
            : terminalTarget.kind === "ssh"
            ? await invoke<TerminalSessionInfo>("terminal_create_ssh", {
                cols: terminalSize.cols,
                rows: terminalSize.rows,
                host: terminalTarget.host,
                port: terminalTarget.port,
                user: terminalTarget.user,
                authMode: terminalTarget.authMode,
                password: terminalTarget.password ?? null,
                keyPath: terminalTarget.keyPath ?? null,
              })
            : await invoke<TerminalSessionInfo>("terminal_create", {
                cols: terminalSize.cols,
                rows: terminalSize.rows,
                shell: coreInfo?.defaultShell ?? null,
              });
        if (!cancelled) {
          setTerminalSession(nextSession);
          setTerminalError("");
        }
      } catch (nextError) {
        if (!cancelled) {
          setTerminalError(String(nextError));
        }
      }
    }

    void createTerminal();

    return () => {
      cancelled = true;
    };
  }, [
    coreInfo?.defaultShell,
    terminalSession,
    terminalSize.cols,
    terminalSize.rows,
    terminalTarget,
  ]);

  useEffect(() => {
    if (!terminalSession) {
      return;
    }

    void invoke("terminal_resize", {
      sessionId: terminalSession.sessionId,
      cols: terminalSize.cols,
      rows: terminalSize.rows,
    }).catch((nextError) => {
      setTerminalError(String(nextError));
    });
  }, [terminalSession, terminalSize.cols, terminalSize.rows]);

  useEffect(() => {
    if (!terminalSession) {
      return;
    }

    let disposed = false;
    let inflight = false;

    const refreshSnapshot = () => {
      if (inflight) {
        return;
      }
      inflight = true;
      void invoke<TerminalSnapshot>("terminal_snapshot", {
        sessionId: terminalSession.sessionId,
        scrollbackOffset,
      })
        .then((nextSnapshot) => {
          if (disposed) {
            return;
          }
          startTransition(() => {
            setTerminalSnapshot(nextSnapshot);
          });
          setTerminalError("");
        })
        .catch((nextError) => {
          if (!disposed) {
            setTerminalError(String(nextError));
          }
        })
        .finally(() => {
          inflight = false;
        });
    };

    refreshSnapshot();
    const intervalId = window.setInterval(refreshSnapshot, 80);

    return () => {
      disposed = true;
      window.clearInterval(intervalId);
    };
  }, [scrollbackOffset, terminalSession]);

  useEffect(() => {
    return () => {
      if (!terminalSession) {
        return;
      }
      void invoke("terminal_close", { sessionId: terminalSession.sessionId });
    };
  }, [terminalSession]);

  const filteredEntries = entries.filter((entry) => {
    if (!deferredSearchText.trim()) {
      return true;
    }
    const query = deferredSearchText.toLowerCase();
    return (
      entry.name.toLowerCase().includes(query) ||
      entry.path.toLowerCase().includes(query)
    );
  });

  const currentPath =
    browserPath || coreInfo?.workspaceRoot || "Loading workspace...";
  const currentPathParts = currentPath.split(/[\\/]/).filter(Boolean);
  const currentDirectoryName =
    currentPathParts[currentPathParts.length - 1] || currentPath;
  const currentParentPath = browserPath
    ? browserPath.replace(/[\\/]+$/, "").replace(/[\\/][^\\/]+$/, "")
    : "";
  const surfaceStatus = terminalSnapshot?.alive
    ? "Live"
    : terminalSession
      ? "Exited"
      : "Booting";
  const rightStatus = gitOverview?.branchName || "No Repo";
  const visibleChangeCount = gitOverview?.changes.length ?? 0;
  const selectedChange =
    gitOverview?.changes.find((change) => changeKey(change) === selectedChangeKey) ?? null;
  const canSwitchBranch =
    !!gitOverview &&
    !!branchSwitchTarget &&
    branchSwitchTarget !== gitOverview.branchName &&
    !gitActionBusy;
  const canCommit =
    !!gitOverview &&
    gitOverview.stagedCount > 0 &&
    !!commitMessage.trim() &&
    !gitActionBusy;
  const canSync = !!gitOverview && !gitActionBusy;
  const canStash = !!gitOverview && !gitOverview.isClean && !gitActionBusy;
  const canDiscardSelected =
    !!selectedChange &&
    !selectedChange.staged &&
    !hasUntrackedStatus(selectedChange) &&
    !gitActionBusy;
  const discardHint = !selectedChange
    ? ""
    : selectedChange.staged
      ? "Discard applies only to worktree changes, not staged entries."
      : hasUntrackedStatus(selectedChange)
        ? "Discard is disabled for untracked files in this build."
        : "";
  const parsedSshPort = Number.parseInt(sshPort, 10);
  const sshNeedsPassword = sshAuthMode === "password";
  const sshNeedsKeyPath = sshAuthMode === "key";
  const canConnectSsh =
    sshHost.trim().length > 0 &&
    sshUser.trim().length > 0 &&
    Number.isFinite(parsedSshPort) &&
    parsedSshPort > 0 &&
    (sshNeedsPassword ? sshPassword.length > 0 : true) &&
    (sshNeedsKeyPath ? sshKeyPath.trim().length > 0 : true);
  const canSaveSshConnection = canConnectSsh;
  const currentTerminalLabel =
    terminalTarget.kind === "sshSaved"
      ? terminalTarget.label
      : terminalTarget.kind === "ssh"
      ? `${terminalTarget.user}@${terminalTarget.host}:${terminalTarget.port} (${terminalTarget.authMode})`
      : coreInfo?.defaultShell ?? "local shell";
  const parsedMysqlPort = Number.parseInt(mysqlPort, 10);
  const canBrowseMysql =
    mysqlHost.trim().length > 0 &&
    mysqlUser.trim().length > 0 &&
    Number.isFinite(parsedMysqlPort) &&
    parsedMysqlPort > 0;
  const parsedRedisPort = Number.parseInt(redisPort, 10);
  const parsedRedisDb = Number.parseInt(redisDb, 10);
  const canBrowseRedis =
    redisHost.trim().length > 0 &&
    Number.isFinite(parsedRedisPort) &&
    parsedRedisPort > 0 &&
    Number.isFinite(parsedRedisDb);
  const canBrowseSqlite = sqlitePath.trim().length > 0;
  const mysqlQueryLooksReadOnly = isReadOnlySql(mysqlSql);
  const sqliteQueryLooksReadOnly = isReadOnlySql(sqliteSql);
  const mysqlNeedsWriteUnlock = mysqlSql.trim().length > 0 && !mysqlQueryLooksReadOnly;
  const sqliteNeedsWriteUnlock = sqliteSql.trim().length > 0 && !sqliteQueryLooksReadOnly;
  const mysqlCanExecuteWrite = !mysqlReadOnly && mysqlWriteConfirm.trim().toUpperCase() === "WRITE";
  const sqliteCanExecuteWrite =
    !sqliteReadOnly && sqliteWriteConfirm.trim().toUpperCase() === "WRITE";
  const canRunMysqlQuery =
    canBrowseMysql &&
    !mysqlQueryBusy &&
    mysqlSql.trim().length > 0 &&
    (!mysqlNeedsWriteUnlock || mysqlCanExecuteWrite);
  const canRunSqliteQuery =
    canBrowseSqlite &&
    !sqliteQueryBusy &&
    sqliteSql.trim().length > 0 &&
    (!sqliteNeedsWriteUnlock || sqliteCanExecuteWrite);

  useEffect(() => {
    setMysqlWriteConfirm("");
    setMysqlQueryNotice("");
  }, [mysqlSql]);

  useEffect(() => {
    setSqliteWriteConfirm("");
    setSqliteQueryNotice("");
  }, [sqliteSql]);

  useEffect(() => {
    if (!browserPath || !selectedChange) {
      setGitDiffText("");
      setGitDiffError("");
      setGitDiffLoading(false);
      return;
    }

    let cancelled = false;
    setGitDiffLoading(true);
    setGitDiffError("");

    void invoke<string>("git_diff", {
      path: browserPath,
      filePath: selectedChange.path,
      staged: selectedChange.staged,
      untracked: hasUntrackedStatus(selectedChange),
    })
      .then((nextDiffText) => {
        if (cancelled) {
          return;
        }
        setGitDiffText(nextDiffText || "No textual diff returned.");
      })
      .catch((nextError) => {
        if (cancelled) {
          return;
        }
        setGitDiffText("");
        setGitDiffError(String(nextError));
      })
      .finally(() => {
        if (!cancelled) {
          setGitDiffLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [browserPath, selectedChange]);

  async function sendTerminalInput(data: string) {
    if (!terminalSession || !data) {
      return;
    }

    try {
      await invoke<number>("terminal_write", {
        sessionId: terminalSession.sessionId,
        data,
      });
      setScrollbackOffset(0);
    } catch (nextError) {
      setTerminalError(String(nextError));
    }
  }

  async function restartTerminal() {
    if (terminalSession) {
      await invoke("terminal_close", { sessionId: terminalSession.sessionId }).catch(
        () => undefined,
      );
    }
    setTerminalSession(null);
    setTerminalSnapshot(null);
    setScrollbackOffset(0);
  }

  async function connectSshTerminal() {
    const port = Number.parseInt(sshPort, 10);
    if (
      !sshHost.trim() ||
      !sshUser.trim() ||
      !Number.isFinite(port) ||
      port <= 0
    ) {
      setTerminalError("SSH host, port, and user are required.");
      return;
    }
    if (sshAuthMode === "password" && !sshPassword) {
      setTerminalError("SSH password is required for password auth.");
      return;
    }
    if (sshAuthMode === "key" && !sshKeyPath.trim()) {
      setTerminalError("SSH key path is required for key-file auth.");
      return;
    }

    setTerminalError("");
    setTerminalTarget({
      kind: "ssh",
      host: sshHost.trim(),
      port,
      user: sshUser.trim(),
      authMode: sshAuthMode,
      password: sshAuthMode === "password" ? sshPassword : undefined,
      keyPath: sshAuthMode === "key" ? sshKeyPath.trim() : undefined,
    });
    await restartTerminal();
  }

  async function saveCurrentSshConnection() {
    const port = Number.parseInt(sshPort, 10);
    if (
      !sshHost.trim() ||
      !sshUser.trim() ||
      !Number.isFinite(port) ||
      port <= 0
    ) {
      setSshConnectionsError("SSH host, port, and user are required before saving.");
      return;
    }
    if (sshAuthMode === "password" && !sshPassword) {
      setSshConnectionsError("SSH password is required before saving password auth.");
      return;
    }
    if (sshAuthMode === "key" && !sshKeyPath.trim()) {
      setSshConnectionsError("SSH key path is required before saving key-file auth.");
      return;
    }

    setSshConnectionsError("");
    try {
      await invoke("ssh_connection_save", {
        name: sshConnectionName.trim(),
        host: sshHost.trim(),
        port,
        user: sshUser.trim(),
        authMode: sshAuthMode,
        password: sshAuthMode === "password" ? sshPassword : null,
        keyPath: sshAuthMode === "key" ? sshKeyPath.trim() : null,
      });
      await refreshSavedSshConnections();
      setSshConnectionsNotice(
        `Saved ${sshConnectionName.trim() || `${sshUser.trim()}@${sshHost.trim()}`}`,
      );
      if (!sshConnectionName.trim()) {
        setSshConnectionName(`${sshUser.trim()}@${sshHost.trim()}`);
      }
    } catch (nextError) {
      setSshConnectionsNotice("");
      setSshConnectionsError(String(nextError));
    }
  }

  async function connectSavedSshConnection(connection: SavedSshConnection) {
    setTerminalError("");
    setSshConnectionsError("");
    setTerminalTarget({
      kind: "sshSaved",
      index: connection.index,
      label: `${connection.name} · ${connection.user}@${connection.host}:${connection.port}`,
    });
    await restartTerminal();
  }

  async function deleteSavedSshConnection(connection: SavedSshConnection) {
    setSshConnectionsError("");
    setSshConnectionsNotice("");
    try {
      await invoke("ssh_connection_delete", { index: connection.index });
      await refreshSavedSshConnections();
      if (terminalTarget.kind === "sshSaved" && terminalTarget.index === connection.index) {
        setTerminalTarget({ kind: "local" });
        await restartTerminal();
      } else if (terminalTarget.kind === "sshSaved" && terminalTarget.index > connection.index) {
        setTerminalTarget({
          ...terminalTarget,
          index: terminalTarget.index - 1,
        });
      }
      setSshConnectionsNotice(`Removed ${connection.name}`);
    } catch (nextError) {
      setSshConnectionsError(String(nextError));
    }
  }

  async function switchToLocalTerminal() {
    setTerminalError("");
    setTerminalTarget({ kind: "local" });
    await restartTerminal();
  }

  async function browseMysql(
    nextDatabaseName = mysqlDatabaseName,
    nextTableName = mysqlTableName,
  ) {
    const port = Number.parseInt(mysqlPort, 10);
    if (
      !mysqlHost.trim() ||
      !mysqlUser.trim() ||
      !Number.isFinite(port) ||
      port <= 0
    ) {
      setMysqlError("MySQL host, port, and user are required.");
      return;
    }

    setMysqlBusy(true);
    setMysqlError("");
    try {
      const nextState = await invoke<MysqlBrowserState>("mysql_browse", {
        host: mysqlHost.trim(),
        port,
        user: mysqlUser.trim(),
        password: mysqlPassword,
        database: nextDatabaseName.trim() || null,
        table: nextTableName.trim() || null,
      });
      setMysqlState(nextState);
      setMysqlDatabaseName(nextState.databaseName);
      setMysqlTableName(nextState.tableName);
      setMysqlQueryError("");
    } catch (nextError) {
      setMysqlState(null);
      setMysqlError(String(nextError));
    } finally {
      setMysqlBusy(false);
    }
  }

  async function browseSqlite(nextTableName = sqliteTableName) {
    if (!sqlitePath.trim()) {
      setSqliteError("SQLite database path is required.");
      return;
    }

    setSqliteBusy(true);
    setSqliteError("");
    try {
      const nextState = await invoke<SqliteBrowserState>("sqlite_browse", {
        path: sqlitePath.trim(),
        table: nextTableName.trim() || null,
      });
      setSqliteState(nextState);
      setSqliteTableName(nextState.tableName);
      setSqliteQueryError("");
    } catch (nextError) {
      setSqliteState(null);
      setSqliteError(String(nextError));
    } finally {
      setSqliteBusy(false);
    }
  }

  async function browseRedis(nextKeyName = redisKeyName) {
    const port = Number.parseInt(redisPort, 10);
    const db = Number.parseInt(redisDb, 10);
    if (!redisHost.trim() || !Number.isFinite(port) || port <= 0 || !Number.isFinite(db)) {
      setRedisError("Redis host, port, and db are required.");
      return;
    }

    setRedisBusy(true);
    setRedisError("");
    try {
      const nextState = await invoke<RedisBrowserState>("redis_browse", {
        host: redisHost.trim(),
        port,
        db,
        pattern: redisPattern.trim() || "*",
        key: nextKeyName.trim() || null,
      });
      setRedisState(nextState);
      setRedisKeyName(nextState.keyName);
      setRedisPattern(nextState.pattern);
      setRedisCommandError("");
    } catch (nextError) {
      setRedisState(null);
      setRedisError(String(nextError));
    } finally {
      setRedisBusy(false);
    }
  }

  async function runRedisCommand() {
    const port = Number.parseInt(redisPort, 10);
    const db = Number.parseInt(redisDb, 10);
    if (!redisHost.trim() || !Number.isFinite(port) || port <= 0 || !Number.isFinite(db)) {
      setRedisCommandError("Redis host, port, and db are required before running a command.");
      return;
    }
    if (!redisCommand.trim()) {
      setRedisCommandError("Enter a Redis command before executing.");
      return;
    }

    setRedisCommandBusy(true);
    setRedisCommandError("");
    try {
      const result = await invoke<RedisCommandResult>("redis_execute", {
        host: redisHost.trim(),
        port,
        db,
        command: redisCommand,
      });
      setRedisCommandResult(result);
      await browseRedis(redisKeyName);
    } catch (nextError) {
      setRedisCommandResult(null);
      setRedisCommandError(String(nextError));
    } finally {
      setRedisCommandBusy(false);
    }
  }

  async function runMysqlQuery() {
    const port = Number.parseInt(mysqlPort, 10);
    if (
      !mysqlHost.trim() ||
      !mysqlUser.trim() ||
      !Number.isFinite(port) ||
      port <= 0
    ) {
      setMysqlQueryError("MySQL host, port, and user are required before running SQL.");
      return;
    }
    if (!mysqlSql.trim()) {
      setMysqlQueryError("Enter a SQL statement before executing.");
      return;
    }
    if (mysqlNeedsWriteUnlock) {
      if (mysqlReadOnly) {
        setMysqlQueryError("Read-only mode blocks mutating MySQL statements. Unlock writes first.");
        return;
      }
      if (!mysqlCanExecuteWrite) {
        setMysqlQueryError("Type WRITE before executing a mutating MySQL statement.");
        return;
      }
    }

    setMysqlQueryBusy(true);
    setMysqlQueryError("");
    setMysqlQueryNotice("");
    try {
      const result = await invoke<QueryExecutionResult>("mysql_execute", {
        host: mysqlHost.trim(),
        port,
        user: mysqlUser.trim(),
        password: mysqlPassword,
        database: mysqlDatabaseName.trim() || null,
        sql: mysqlSql,
      });
      setMysqlQueryResult(result);
      setMysqlQueryNotice(
        mysqlNeedsWriteUnlock
          ? `Write statement executed in ${result.elapsedMs} ms.`
          : `Query completed in ${result.elapsedMs} ms.`,
      );
      await browseMysql(mysqlDatabaseName, mysqlTableName);
    } catch (nextError) {
      setMysqlQueryResult(null);
      setMysqlQueryError(String(nextError));
    } finally {
      if (mysqlNeedsWriteUnlock) {
        setMysqlReadOnly(true);
        setMysqlWriteConfirm("");
      }
      setMysqlQueryBusy(false);
    }
  }

  async function runSqliteQuery() {
    if (!sqlitePath.trim()) {
      setSqliteQueryError("SQLite database path is required before running SQL.");
      return;
    }
    if (!sqliteSql.trim()) {
      setSqliteQueryError("Enter a SQL statement before executing.");
      return;
    }
    if (sqliteNeedsWriteUnlock) {
      if (sqliteReadOnly) {
        setSqliteQueryError("Read-only mode blocks mutating SQLite statements. Unlock writes first.");
        return;
      }
      if (!sqliteCanExecuteWrite) {
        setSqliteQueryError("Type WRITE before executing a mutating SQLite statement.");
        return;
      }
    }

    setSqliteQueryBusy(true);
    setSqliteQueryError("");
    setSqliteQueryNotice("");
    try {
      const result = await invoke<QueryExecutionResult>("sqlite_execute", {
        path: sqlitePath.trim(),
        sql: sqliteSql,
      });
      setSqliteQueryResult(result);
      setSqliteQueryNotice(
        sqliteNeedsWriteUnlock
          ? `Write statement executed in ${result.elapsedMs} ms.`
          : `Query completed in ${result.elapsedMs} ms.`,
      );
      await browseSqlite(sqliteTableName);
    } catch (nextError) {
      setSqliteQueryResult(null);
      setSqliteQueryError(String(nextError));
    } finally {
      if (sqliteNeedsWriteUnlock) {
        setSqliteReadOnly(true);
        setSqliteWriteConfirm("");
      }
      setSqliteQueryBusy(false);
    }
  }

  async function stageSelectedChange() {
    if (!selectedChange || gitActionBusy) {
      return;
    }

    setGitActionBusy(true);
    setGitOperationError("");
    setGitNotice("");
    try {
      if (selectedChange.staged) {
        await invoke("git_unstage_paths", {
          path: browserPath,
          paths: [selectedChange.path],
        });
        await refreshGitOverviewState(
          makeChangeKey(selectedChange.path, false),
          selectedChange.path,
        );
        setGitNotice(`Unstaged ${selectedChange.path}`);
      } else {
        await invoke("git_stage_paths", {
          path: browserPath,
          paths: [selectedChange.path],
        });
        await refreshGitOverviewState(
          makeChangeKey(selectedChange.path, true),
          selectedChange.path,
        );
        setGitNotice(`Staged ${selectedChange.path}`);
      }
    } catch (nextError) {
      setGitOperationError(String(nextError));
    } finally {
      setGitActionBusy(false);
    }
  }

  async function stageAllChanges() {
    if (gitActionBusy || !gitOverview || gitOverview.isClean) {
      return;
    }

    setGitActionBusy(true);
    setGitOperationError("");
    setGitNotice("");
    try {
      await invoke("git_stage_all", { path: browserPath });
      await refreshGitOverviewState(undefined, selectedChange?.path);
      setGitNotice("Staged every visible change.");
    } catch (nextError) {
      setGitOperationError(String(nextError));
    } finally {
      setGitActionBusy(false);
    }
  }

  async function unstageAllChanges() {
    if (gitActionBusy || !gitOverview || gitOverview.stagedCount === 0) {
      return;
    }

    setGitActionBusy(true);
    setGitOperationError("");
    setGitNotice("");
    try {
      await invoke("git_unstage_all", { path: browserPath });
      await refreshGitOverviewState(undefined, selectedChange?.path);
      setGitNotice("Reset the index back to HEAD.");
    } catch (nextError) {
      setGitOperationError(String(nextError));
    } finally {
      setGitActionBusy(false);
    }
  }

  async function discardSelectedChange() {
    if (!selectedChange || !canDiscardSelected) {
      return;
    }

    setGitActionBusy(true);
    setGitOperationError("");
    setGitNotice("");
    try {
      await invoke("git_discard_paths", {
        path: browserPath,
        paths: [selectedChange.path],
      });
      await refreshGitOverviewState(undefined, selectedChange.path);
      await refreshGitDetailState(gitOverview?.branchName);
      setGitNotice(`Discarded worktree edits for ${selectedChange.path}`);
    } catch (nextError) {
      setGitOperationError(String(nextError));
    } finally {
      setGitActionBusy(false);
    }
  }

  async function switchBranch() {
    if (!canSwitchBranch) {
      return;
    }

    setGitActionBusy(true);
    setGitOperationError("");
    setGitNotice("");
    try {
      const output = await invoke<string>("git_checkout_branch", {
        path: browserPath,
        name: branchSwitchTarget,
      });
      const nextOverview = await refreshGitOverviewState();
      await refreshGitDetailState(nextOverview?.branchName ?? branchSwitchTarget);
      setGitNotice(output || `Switched to ${branchSwitchTarget}`);
    } catch (nextError) {
      setGitOperationError(String(nextError));
    } finally {
      setGitActionBusy(false);
    }
  }

  async function commitStagedChanges() {
    if (!canCommit) {
      return;
    }

    const nextMessage = commitMessage.trim();
    setGitActionBusy(true);
    setGitOperationError("");
    setGitNotice("");
    try {
      const output = await invoke<string>("git_commit", {
        path: browserPath,
        message: nextMessage,
      });
      const nextOverview = await refreshGitOverviewState();
      await refreshGitDetailState(nextOverview?.branchName);
      setCommitMessage("");
      setGitNotice(output || "Created a new commit.");
    } catch (nextError) {
      setGitOperationError(String(nextError));
    } finally {
      setGitActionBusy(false);
    }
  }

  async function pushCurrentBranch() {
    if (!canSync) {
      return;
    }

    setGitActionBusy(true);
    setGitOperationError("");
    setGitNotice("");
    try {
      const output = await invoke<string>("git_push", { path: browserPath });
      const nextOverview = await refreshGitOverviewState();
      await refreshGitDetailState(nextOverview?.branchName);
      setGitNotice(output || "Push completed.");
    } catch (nextError) {
      setGitOperationError(String(nextError));
    } finally {
      setGitActionBusy(false);
    }
  }

  async function pullCurrentBranch() {
    if (!canSync) {
      return;
    }

    setGitActionBusy(true);
    setGitOperationError("");
    setGitNotice("");
    try {
      const output = await invoke<string>("git_pull", { path: browserPath });
      const nextOverview = await refreshGitOverviewState();
      await refreshGitDetailState(nextOverview?.branchName);
      setGitNotice(output || "Pull completed.");
    } catch (nextError) {
      setGitOperationError(String(nextError));
    } finally {
      setGitActionBusy(false);
    }
  }

  async function stashCurrentChanges() {
    if (!canStash) {
      return;
    }

    setGitActionBusy(true);
    setGitOperationError("");
    setGitNotice("");
    try {
      const output = await invoke<string>("git_stash_push", {
        path: browserPath,
        message: stashMessage,
      });
      const nextOverview = await refreshGitOverviewState();
      await refreshGitDetailState(nextOverview?.branchName);
      setStashMessage("");
      setGitNotice(output || "Created a stash entry.");
    } catch (nextError) {
      setGitOperationError(String(nextError));
    } finally {
      setGitActionBusy(false);
    }
  }

  async function restoreStash(index: string, pop: boolean) {
    if (!canSync) {
      return;
    }

    setGitActionBusy(true);
    setGitOperationError("");
    setGitNotice("");
    try {
      const command = pop ? "git_stash_pop" : "git_stash_apply";
      const output = await invoke<string>(command, { path: browserPath, index });
      const nextOverview = await refreshGitOverviewState();
      await refreshGitDetailState(nextOverview?.branchName);
      setGitNotice(output || `${pop ? "Popped" : "Applied"} ${index}.`);
    } catch (nextError) {
      setGitOperationError(String(nextError));
    } finally {
      setGitActionBusy(false);
    }
  }

  async function dropStash(index: string) {
    if (!canSync) {
      return;
    }

    setGitActionBusy(true);
    setGitOperationError("");
    setGitNotice("");
    try {
      const output = await invoke<string>("git_stash_drop", { path: browserPath, index });
      const nextOverview = await refreshGitOverviewState();
      await refreshGitDetailState(nextOverview?.branchName);
      setGitNotice(output || `Dropped ${index}.`);
    } catch (nextError) {
      setGitOperationError(String(nextError));
    } finally {
      setGitActionBusy(false);
    }
  }

  function handleTerminalKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    const modifierKey = event.ctrlKey || event.metaKey;
    const selectionText = getTerminalSelectionText();

    if (modifierKey && !event.altKey && event.key.toLowerCase() === "v") {
      event.preventDefault();
      void pasteIntoTerminal();
      return;
    }

    if (
      modifierKey &&
      !event.altKey &&
      event.key.toLowerCase() === "c" &&
      selectionText
    ) {
      event.preventDefault();
      void copyTerminalSelection(selectionText);
      return;
    }

    let payload = "";

    if (event.ctrlKey && !event.altKey && !event.metaKey) {
      if (event.key.length === 1) {
        const upper = event.key.toUpperCase();
        if (upper >= "A" && upper <= "Z") {
          payload = String.fromCharCode(upper.charCodeAt(0) - 64);
        } else if (upper in controlKeyMap) {
          payload = controlKeyMap[upper];
        }
      }
    } else if (event.key === "Enter") {
      payload = "\r";
    } else if (event.key === "Backspace") {
      payload = "\u007f";
    } else if (event.key === "Tab") {
      payload = event.shiftKey ? "\u001b[Z" : "\t";
    } else if (event.key === "Escape") {
      payload = "\u001b";
    } else if (event.key === "ArrowUp") {
      payload = "\u001b[A";
    } else if (event.key === "ArrowDown") {
      payload = "\u001b[B";
    } else if (event.key === "ArrowRight") {
      payload = "\u001b[C";
    } else if (event.key === "ArrowLeft") {
      payload = "\u001b[D";
    } else if (event.key === "Home") {
      payload = "\u001b[H";
    } else if (event.key === "End") {
      payload = "\u001b[F";
    } else if (!event.metaKey && !event.ctrlKey && event.key.length === 1) {
      payload = event.key;
    }

    if (!payload) {
      return;
    }

    event.preventDefault();
    void sendTerminalInput(payload);
  }

  function handleTerminalWheel(event: React.WheelEvent<HTMLDivElement>) {
    if (!terminalSnapshot?.scrollbackLen) {
      return;
    }

    event.preventDefault();
    const step = Math.max(1, Math.round(Math.abs(event.deltaY) / 36));
    setScrollbackOffset((previous) => {
      if (event.deltaY < 0) {
        return Math.min(previous + step, terminalSnapshot.scrollbackLen);
      }
      return Math.max(previous - step, 0);
    });
  }

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="topbar__brand">
          <div className="brand-mark">PX</div>
          <div className="brand-copy">
            <strong>Pier-X</strong>
            <span>Tauri Workbench</span>
          </div>
        </div>
        <div className="command-pill">
          <Command size={14} />
          <span>Shift Shift</span>
          <small>Command Palette</small>
        </div>
        <div className="topbar__meta">
          <span className="meta-pill meta-pill--accent">
            {coreInfo ? `core ${coreInfo.version}` : "pier-core ..."}
          </span>
          <span className="meta-pill">{coreInfo?.profile ?? "bootstrap"}</span>
          <span className="meta-pill">
            {terminalSession?.shell ?? coreInfo?.defaultShell ?? "shell"}
          </span>
        </div>
      </header>

      <Group orientation="vertical" className="workspace">
        <Panel defaultSize={78} minSize={56}>
          <Group orientation="horizontal">
            <Panel defaultSize={18} minSize={14}>
              <aside className="pane pane--left">
                <div className="pane__header">
                  <div>
                    <p className="eyebrow">Explorer</p>
                    <h2>{currentDirectoryName}</h2>
                  </div>
                  <span className="status-pill">{filteredEntries.length} items</span>
                </div>
                <div className="path-pill path-pill--interactive">
                  <div className="path-pill__copy">
                    <FolderTree size={14} />
                    <span>{currentPath}</span>
                  </div>
                  <button
                    className="mini-button"
                    disabled={!currentParentPath || currentParentPath === browserPath}
                    onClick={() => setBrowserPath(currentParentPath)}
                    type="button"
                  >
                    <ChevronUp size={13} />
                    Up
                  </button>
                </div>
                <label className="search-box" aria-label="Search files">
                  <Search size={14} />
                  <input
                    onChange={(event) => setSearchText(event.currentTarget.value)}
                    placeholder="Search files, commands, connections"
                    value={searchText}
                  />
                </label>
                <div className="explorer-head">
                  <span>名称</span>
                  <span>类型</span>
                  <span>大小</span>
                </div>
                <div className="explorer-list">
                  {filteredEntries.map((entry) => (
                    <button
                      key={entry.path}
                      className="file-row"
                      onClick={() => {
                        if (entry.kind === "directory") {
                          setBrowserPath(entry.path);
                        }
                      }}
                      type="button"
                    >
                      <span className="file-row__name">
                        <span
                          className={
                            entry.kind === "directory"
                              ? "file-dot file-dot--directory"
                              : "file-dot"
                          }
                        />
                        {entry.name}
                      </span>
                      <span className="file-row__meta">
                        {entry.kind === "directory" ? "Folder" : "File"}
                      </span>
                      <span className="file-row__meta">{entry.sizeLabel}</span>
                    </button>
                  ))}
                </div>
              </aside>
            </Panel>

            <Separator className="resize-handle resize-handle--col" />

            <Panel defaultSize={58} minSize={38}>
              <main className="pane pane--center">
                <div className="pane__header pane__header--wide">
                  <div>
                    <p className="eyebrow">Workbench</p>
                    <h1>Qt shell out, Tauri shell in.</h1>
                  </div>
                  <div className="pane__actions">
                    <span className="status-pill status-pill--live">{surfaceStatus}</span>
                    <span className="ghost-pill">
                      <ActivitySquare size={14} />
                      {terminalTarget.kind === "ssh" || terminalTarget.kind === "sshSaved"
                        ? "remote shell online path"
                        : "core-backed terminal online"}
                    </span>
                  </div>
                </div>

                <section className="hero-card">
                  <div className="hero-card__copy">
                    <p className="eyebrow eyebrow--accent">Migration Track</p>
                    <h3>终端和 Git 已经直接吃到 pier-core，Tauri 不再只是展示层。</h3>
                    <p>
                      当前工作台已经能在新壳里启动真实 shell、轮询网格快照并展示当前仓库状态。
                      现在数据库连接面板也开始直接吃 `pier-core` service client，下一步再补查询编辑器、
                      结果表和插件边界。
                    </p>
                  </div>
                  <div className="hero-metrics">
                    <div className="metric-card">
                      <span>Workspace</span>
                      <strong>{currentDirectoryName}</strong>
                    </div>
                    <div className="metric-card">
                      <span>Terminal</span>
                      <strong>
                        {terminalTarget.kind === "ssh" || terminalTarget.kind === "sshSaved"
                          ? "SSH Session"
                          : "Local Shell"}
                      </strong>
                    </div>
                    <div className="metric-card">
                      <span>Git</span>
                      <strong>{gitOverview?.branchName ?? "No Repo"}</strong>
                    </div>
                  </div>
                </section>

                <section className="service-section">
                  <div className="section-title">
                    <LayoutDashboard size={15} />
                    <span>Runtime Surface</span>
                  </div>
                  <div className="service-grid">
                    {(coreInfo?.services ?? []).map((service) => (
                      <article key={service} className="service-card">
                        <div className="service-card__title">
                          <SquareTerminal size={16} />
                          <span>{service}</span>
                        </div>
                        <p>{serviceDescriptions[service] ?? "Pending runtime binding"}</p>
                        <span className="service-card__state">
                          {service === "terminal"
                            ? terminalSnapshot?.alive
                              ? "live"
                              : "booting"
                            : service === "ssh"
                              ? terminalTarget.kind === "ssh" || terminalTarget.kind === "sshSaved"
                                ? "live"
                                : "ready"
                            : service === "git"
                              ? gitOverview
                                ? "wired"
                                : "probe"
                              : service === "mysql"
                                ? mysqlState
                                  ? "wired"
                                  : "ready"
                                : service === "sqlite"
                                  ? sqliteState
                                    ? "wired"
                                    : "ready"
                                  : service === "redis"
                                    ? redisState
                                      ? "wired"
                                      : "ready"
                              : "planned"}
                        </span>
                      </article>
                    ))}
                  </div>
                </section>

                <section className="data-section">
                  <div className="diff-section__header">
                    <div className="section-title">
                      <Database size={15} />
                      <span>Data Sources</span>
                    </div>
                    <div className="surface-switcher">
                      {(["mysql", "sqlite", "redis"] as DataSurface[]).map((surface) => (
                        <button
                          key={surface}
                          className={
                            dataSurface === surface
                              ? "surface-button surface-button--selected"
                              : "surface-button"
                          }
                          onClick={() => setDataSurface(surface)}
                          type="button"
                        >
                          {surface === "mysql"
                            ? "MySQL"
                            : surface === "sqlite"
                              ? "SQLite"
                              : "Redis"}
                        </button>
                      ))}
                    </div>
                  </div>

                  {dataSurface === "mysql" ? (
                    <div className="data-grid">
                      <article className="service-card data-card">
                        <div className="stack-card__title">
                          <Database size={15} />
                          <span>MySQL Browser</span>
                        </div>
                        <div className="form-stack">
                          <div className="field-grid">
                            <label className="field-stack">
                              <span className="field-label">Host</span>
                              <input
                                className="field-input"
                                onChange={(event) => setMysqlHost(event.currentTarget.value)}
                                placeholder="127.0.0.1"
                                value={mysqlHost}
                              />
                            </label>
                            <label className="field-stack">
                              <span className="field-label">Port</span>
                              <input
                                className="field-input field-input--narrow"
                                inputMode="numeric"
                                onChange={(event) => setMysqlPort(event.currentTarget.value)}
                                placeholder="3306"
                                value={mysqlPort}
                              />
                            </label>
                          </div>

                          <div className="field-grid field-grid--balanced">
                            <label className="field-stack">
                              <span className="field-label">User</span>
                              <input
                                className="field-input"
                                onChange={(event) => setMysqlUser(event.currentTarget.value)}
                                placeholder="root"
                                value={mysqlUser}
                              />
                            </label>
                            <label className="field-stack">
                              <span className="field-label">Password</span>
                              <input
                                className="field-input"
                                onChange={(event) => setMysqlPassword(event.currentTarget.value)}
                                placeholder="Optional password"
                                type="password"
                                value={mysqlPassword}
                              />
                            </label>
                          </div>

                          <label className="field-stack">
                            <span className="field-label">Preferred database</span>
                            <div className="branch-row">
                              <input
                                className="field-input"
                                onChange={(event) => setMysqlDatabaseName(event.currentTarget.value)}
                                placeholder="Leave empty to use the first visible schema"
                                value={mysqlDatabaseName}
                              />
                              <button
                                className="mini-button"
                                disabled={!canBrowseMysql || mysqlBusy}
                                onClick={() => void browseMysql()}
                                type="button"
                              >
                                {mysqlBusy ? "Browsing..." : "Browse"}
                              </button>
                            </div>
                          </label>

                          {mysqlState ? (
                            <div className="status-note">
                              Connected schema: {mysqlState.databaseName || "none"} · tables{" "}
                              {mysqlState.tables.length}
                            </div>
                          ) : null}
                          {mysqlError ? (
                            <div className="status-note status-note--error">{mysqlError}</div>
                          ) : null}
                        </div>
                      </article>

                      <article className="service-card data-card">
                        <div className="stack-card__title">
                          <LayoutDashboard size={15} />
                          <span>Schema</span>
                        </div>
                        {mysqlState ? (
                          <div className="form-stack">
                            <div className="field-stack">
                              <span className="field-label">Databases</span>
                              <div className="token-list">
                                {mysqlState.databases.map((database) => (
                                  <button
                                    key={database}
                                    className={
                                      mysqlState.databaseName === database
                                        ? "token-button token-button--selected"
                                        : "token-button"
                                    }
                                    onClick={() => {
                                      setMysqlDatabaseName(database);
                                      setMysqlTableName("");
                                      setMysqlSql(`SHOW TABLES FROM \`${database}\`;`);
                                      void browseMysql(database, "");
                                    }}
                                    type="button"
                                  >
                                    {database}
                                  </button>
                                ))}
                              </div>
                            </div>

                            <div className="field-stack">
                              <span className="field-label">Tables</span>
                              <div className="token-list">
                                {mysqlState.tables.map((table) => (
                                  <button
                                    key={table}
                                    className={
                                      mysqlState.tableName === table
                                        ? "token-button token-button--selected"
                                        : "token-button"
                                    }
                                    onClick={() => {
                                      setMysqlTableName(table);
                                      setMysqlSql(
                                        `SELECT * FROM \`${mysqlState.databaseName}\`.\`${table}\` LIMIT 100;`,
                                      );
                                      void browseMysql(mysqlState.databaseName, table);
                                    }}
                                    type="button"
                                  >
                                    {table}
                                  </button>
                                ))}
                              </div>
                            </div>

                            <div className="field-stack">
                              <span className="field-label">Columns</span>
                              {mysqlState.columns.length > 0 ? (
                                <div className="column-list">
                                  {mysqlState.columns.map((column) => (
                                    <div className="column-row" key={column.name}>
                                      <div className="column-row__head">
                                        <strong>{column.name}</strong>
                                        <span className="connection-pill">{column.columnType}</span>
                                      </div>
                                      <div className="column-row__meta">
                                        {column.nullable ? "nullable" : "not null"}
                                        {column.key ? ` • ${column.key}` : ""}
                                        {column.extra ? ` • ${column.extra}` : ""}
                                        {column.defaultValue ? ` • default ${column.defaultValue}` : ""}
                                      </div>
                                    </div>
                                  ))}
                                </div>
                              ) : (
                                <div className="empty-note">
                                  Choose a table to inspect its column layout.
                                </div>
                              )}
                            </div>
                          </div>
                        ) : (
                          <div className="empty-note">
                            Connect to MySQL to browse databases, tables, and columns.
                          </div>
                        )}
                      </article>

                      <article className="service-card data-card data-card--wide">
                        <div className="stack-card__title">
                          <Search size={15} />
                          <span>Sample Rows</span>
                        </div>
                        <PreviewTable
                          emptyLabel="Choose a table to preview up to 24 rows."
                          preview={mysqlState?.preview ?? null}
                        />
                      </article>

                      <article className="service-card data-card data-card--wide">
                        <div className="stack-card__title">
                          <Command size={15} />
                          <span>Query Editor</span>
                        </div>
                        <div className="form-stack">
                          <div className="query-guard-row">
                            <span
                              className={
                                mysqlReadOnly
                                  ? "safety-pill safety-pill--locked"
                                  : "safety-pill safety-pill--unlocked"
                              }
                            >
                              {mysqlReadOnly ? "Read Only" : "Writes Unlocked"}
                            </span>
                            <button
                              className="mini-button"
                              onClick={() => {
                                setMysqlReadOnly((previous) => !previous);
                                setMysqlWriteConfirm("");
                                setMysqlQueryNotice("");
                                setMysqlQueryError("");
                              }}
                              type="button"
                            >
                              {mysqlReadOnly ? "Unlock Writes" : "Re-lock Writes"}
                            </button>
                          </div>
                          <label className="field-stack">
                            <span className="field-label">MySQL SQL</span>
                            <textarea
                              className="field-textarea field-textarea--editor"
                              onChange={(event) => {
                                setMysqlSql(event.currentTarget.value);
                                setMysqlQueryNotice("");
                                setMysqlQueryError("");
                              }}
                              placeholder="SELECT * FROM your_table LIMIT 100;"
                              rows={8}
                              value={mysqlSql}
                            />
                          </label>
                          {mysqlNeedsWriteUnlock ? (
                            <div className="status-note status-note--warning">
                              This statement looks mutating. Keep write mode locked for read-only
                              exploration, or unlock and type `WRITE` to continue.
                            </div>
                          ) : (
                            <div className="inline-note">
                              Read-only statements run immediately and keep the workbench safe by
                              default.
                            </div>
                          )}
                          {mysqlNeedsWriteUnlock && !mysqlReadOnly ? (
                            <label className="field-stack">
                              <span className="field-label">Type WRITE to confirm</span>
                              <input
                                className="field-input"
                                onChange={(event) => setMysqlWriteConfirm(event.currentTarget.value)}
                                placeholder="WRITE"
                                value={mysqlWriteConfirm}
                              />
                            </label>
                          ) : null}
                          <div className="button-row">
                            <button
                              className="mini-button"
                              disabled={!canRunMysqlQuery}
                              onClick={() => void runMysqlQuery()}
                              type="button"
                            >
                              {mysqlQueryBusy ? "Running..." : "Run Query"}
                            </button>
                          </div>
                          {mysqlQueryNotice ? (
                            <div className="status-note">{mysqlQueryNotice}</div>
                          ) : null}
                        </div>
                      </article>

                      <article className="service-card data-card data-card--wide">
                        <div className="stack-card__title stack-card__title--spread">
                          <span className="stack-card__title-inner">
                            <LayoutDashboard size={15} />
                            <span>Query Results</span>
                          </span>
                          <button
                            className="mini-button"
                            disabled={!mysqlQueryResult}
                            onClick={() =>
                              void copyQueryResult(
                                "MySQL",
                                mysqlQueryResult,
                                setMysqlQueryNotice,
                                setMysqlQueryError,
                              )
                            }
                            type="button"
                          >
                            Copy TSV
                          </button>
                        </div>
                        <QueryResultPanel
                          emptyLabel="Run a MySQL statement to inspect its result set."
                          error={mysqlQueryError}
                          result={mysqlQueryResult}
                        />
                      </article>
                    </div>
                  ) : dataSurface === "sqlite" ? (
                    <div className="data-grid">
                      <article className="service-card data-card">
                        <div className="stack-card__title">
                          <Database size={15} />
                          <span>SQLite Browser</span>
                        </div>
                        <div className="form-stack">
                          <label className="field-stack">
                            <span className="field-label">Database file</span>
                            <div className="branch-row">
                              <input
                                className="field-input"
                                onChange={(event) => setSqlitePath(event.currentTarget.value)}
                                placeholder="C:\\data\\app.db"
                                value={sqlitePath}
                              />
                              <button
                                className="mini-button"
                                disabled={!canBrowseSqlite || sqliteBusy}
                                onClick={() => void browseSqlite()}
                                type="button"
                              >
                                {sqliteBusy ? "Browsing..." : "Browse"}
                              </button>
                            </div>
                          </label>

                          {sqliteState ? (
                            <div className="status-note">
                              Opened {sqliteState.path} · tables {sqliteState.tables.length}
                            </div>
                          ) : null}
                          {sqliteError ? (
                            <div className="status-note status-note--error">{sqliteError}</div>
                          ) : null}
                        </div>
                      </article>

                      <article className="service-card data-card">
                        <div className="stack-card__title">
                          <LayoutDashboard size={15} />
                          <span>Tables &amp; Columns</span>
                        </div>
                        {sqliteState ? (
                          <div className="form-stack">
                            <div className="field-stack">
                              <span className="field-label">Tables</span>
                              <div className="token-list">
                                {sqliteState.tables.map((table) => (
                                  <button
                                    key={table}
                                    className={
                                      sqliteState.tableName === table
                                        ? "token-button token-button--selected"
                                        : "token-button"
                                    }
                                    onClick={() => {
                                      setSqliteTableName(table);
                                      setSqliteSql(`SELECT * FROM "${table.replace(/"/g, '""')}" LIMIT 100;`);
                                      void browseSqlite(table);
                                    }}
                                    type="button"
                                  >
                                    {table}
                                  </button>
                                ))}
                              </div>
                            </div>

                            <div className="field-stack">
                              <span className="field-label">Columns</span>
                              {sqliteState.columns.length > 0 ? (
                                <div className="column-list">
                                  {sqliteState.columns.map((column) => (
                                    <div className="column-row" key={column.name}>
                                      <div className="column-row__head">
                                        <strong>{column.name}</strong>
                                        <span className="connection-pill">{column.colType}</span>
                                      </div>
                                      <div className="column-row__meta">
                                        {column.notNull ? "not null" : "nullable"}
                                        {column.primaryKey ? " • primary key" : ""}
                                      </div>
                                    </div>
                                  ))}
                                </div>
                              ) : (
                                <div className="empty-note">
                                  Choose a table to inspect its schema.
                                </div>
                              )}
                            </div>
                          </div>
                        ) : (
                          <div className="empty-note">
                            Open a SQLite file to inspect the schema and sample rows.
                          </div>
                        )}
                      </article>

                      <article className="service-card data-card data-card--wide">
                        <div className="stack-card__title">
                          <Search size={15} />
                          <span>Sample Rows</span>
                        </div>
                        <PreviewTable
                          emptyLabel="Choose a table to preview up to 24 rows."
                          preview={sqliteState?.preview ?? null}
                        />
                      </article>

                      <article className="service-card data-card data-card--wide">
                        <div className="stack-card__title">
                          <Command size={15} />
                          <span>Query Editor</span>
                        </div>
                        <div className="form-stack">
                          <div className="query-guard-row">
                            <span
                              className={
                                sqliteReadOnly
                                  ? "safety-pill safety-pill--locked"
                                  : "safety-pill safety-pill--unlocked"
                              }
                            >
                              {sqliteReadOnly ? "Read Only" : "Writes Unlocked"}
                            </span>
                            <button
                              className="mini-button"
                              onClick={() => {
                                setSqliteReadOnly((previous) => !previous);
                                setSqliteWriteConfirm("");
                                setSqliteQueryNotice("");
                                setSqliteQueryError("");
                              }}
                              type="button"
                            >
                              {sqliteReadOnly ? "Unlock Writes" : "Re-lock Writes"}
                            </button>
                          </div>
                          <label className="field-stack">
                            <span className="field-label">SQLite SQL</span>
                            <textarea
                              className="field-textarea field-textarea--editor"
                              onChange={(event) => {
                                setSqliteSql(event.currentTarget.value);
                                setSqliteQueryNotice("");
                                setSqliteQueryError("");
                              }}
                              placeholder='SELECT * FROM "your_table" LIMIT 100;'
                              rows={8}
                              value={sqliteSql}
                            />
                          </label>
                          {sqliteNeedsWriteUnlock ? (
                            <div className="status-note status-note--warning">
                              This statement looks mutating. Keep write mode locked for read-only
                              exploration, or unlock and type `WRITE` to continue.
                            </div>
                          ) : (
                            <div className="inline-note">
                              Read-only statements run immediately and keep the workbench safe by
                              default.
                            </div>
                          )}
                          {sqliteNeedsWriteUnlock && !sqliteReadOnly ? (
                            <label className="field-stack">
                              <span className="field-label">Type WRITE to confirm</span>
                              <input
                                className="field-input"
                                onChange={(event) => setSqliteWriteConfirm(event.currentTarget.value)}
                                placeholder="WRITE"
                                value={sqliteWriteConfirm}
                              />
                            </label>
                          ) : null}
                          <div className="button-row">
                            <button
                              className="mini-button"
                              disabled={!canRunSqliteQuery}
                              onClick={() => void runSqliteQuery()}
                              type="button"
                            >
                              {sqliteQueryBusy ? "Running..." : "Run Query"}
                            </button>
                          </div>
                          {sqliteQueryNotice ? (
                            <div className="status-note">{sqliteQueryNotice}</div>
                          ) : null}
                        </div>
                      </article>

                      <article className="service-card data-card data-card--wide">
                        <div className="stack-card__title stack-card__title--spread">
                          <span className="stack-card__title-inner">
                            <LayoutDashboard size={15} />
                            <span>Query Results</span>
                          </span>
                          <button
                            className="mini-button"
                            disabled={!sqliteQueryResult}
                            onClick={() =>
                              void copyQueryResult(
                                "SQLite",
                                sqliteQueryResult,
                                setSqliteQueryNotice,
                                setSqliteQueryError,
                              )
                            }
                            type="button"
                          >
                            Copy TSV
                          </button>
                        </div>
                        <QueryResultPanel
                          emptyLabel="Run a SQLite statement to inspect its result set."
                          error={sqliteQueryError}
                          result={sqliteQueryResult}
                        />
                      </article>
                    </div>
                  ) : (
                    <div className="data-grid">
                      <article className="service-card data-card">
                        <div className="stack-card__title">
                          <Database size={15} />
                          <span>Redis Browser</span>
                        </div>
                        <div className="form-stack">
                          <div className="field-grid">
                            <label className="field-stack">
                              <span className="field-label">Host</span>
                              <input
                                className="field-input"
                                onChange={(event) => setRedisHost(event.currentTarget.value)}
                                placeholder="127.0.0.1"
                                value={redisHost}
                              />
                            </label>
                            <label className="field-stack">
                              <span className="field-label">Port</span>
                              <input
                                className="field-input field-input--narrow"
                                inputMode="numeric"
                                onChange={(event) => setRedisPort(event.currentTarget.value)}
                                placeholder="6379"
                                value={redisPort}
                              />
                            </label>
                          </div>

                          <div className="field-grid field-grid--balanced">
                            <label className="field-stack">
                              <span className="field-label">DB</span>
                              <input
                                className="field-input"
                                inputMode="numeric"
                                onChange={(event) => setRedisDb(event.currentTarget.value)}
                                placeholder="0"
                                value={redisDb}
                              />
                            </label>
                            <label className="field-stack">
                              <span className="field-label">Pattern</span>
                              <input
                                className="field-input"
                                onChange={(event) => setRedisPattern(event.currentTarget.value)}
                                placeholder="user:*"
                                value={redisPattern}
                              />
                            </label>
                          </div>

                          <div className="button-row">
                            <button
                              className="mini-button"
                              disabled={!canBrowseRedis || redisBusy}
                              onClick={() => void browseRedis()}
                              type="button"
                            >
                              {redisBusy ? "Browsing..." : "Scan Keys"}
                            </button>
                          </div>

                          {redisState ? (
                            <div className="status-note">
                              {redisState.pong} · {redisState.serverVersion || "version unknown"}
                              {redisState.usedMemory ? ` · ${redisState.usedMemory}` : ""}
                            </div>
                          ) : null}
                          {redisError ? (
                            <div className="status-note status-note--error">{redisError}</div>
                          ) : null}
                        </div>
                      </article>

                      <article className="service-card data-card">
                        <div className="stack-card__title">
                          <LayoutDashboard size={15} />
                          <span>Keys</span>
                        </div>
                        {redisState ? (
                          <div className="form-stack">
                            <div className="inline-note">
                              Pattern `{redisState.pattern}` · showing up to {redisState.limit} keys
                              {redisState.truncated ? " (truncated)" : ""}
                            </div>
                            <div className="token-list">
                              {redisState.keys.map((key) => (
                                <button
                                  key={key}
                                  className={
                                    redisState.keyName === key
                                      ? "token-button token-button--selected"
                                      : "token-button"
                                  }
                                  onClick={() => {
                                    setRedisKeyName(key);
                                    setRedisCommand(`TYPE ${quoteCommandArg(key)}`);
                                    void browseRedis(key);
                                  }}
                                  type="button"
                                >
                                  {key}
                                </button>
                              ))}
                            </div>
                          </div>
                        ) : (
                          <div className="empty-note">
                            Connect to Redis / Valkey to scan keys and inspect previews.
                          </div>
                        )}
                      </article>

                      <article className="service-card data-card data-card--wide">
                        <div className="stack-card__title">
                          <Search size={15} />
                          <span>Key Preview</span>
                        </div>
                        {redisState?.details ? (
                          <div className="form-stack">
                            <div className="data-meta-grid">
                              <div className="meta-chip">
                                <span>Key</span>
                                <strong>{redisState.details.key}</strong>
                              </div>
                              <div className="meta-chip">
                                <span>Type</span>
                                <strong>{redisState.details.kind}</strong>
                              </div>
                              <div className="meta-chip">
                                <span>Length</span>
                                <strong>{redisState.details.length}</strong>
                              </div>
                              <div className="meta-chip">
                                <span>TTL</span>
                                <strong>{redisState.details.ttlSeconds}</strong>
                              </div>
                            </div>
                            <div className="inline-note">
                              Encoding: {redisState.details.encoding || "unknown"}
                            </div>
                            <div className="preview-list">
                              {redisState.details.preview.map((item, index) => (
                                <div className="preview-item" key={`${redisState.details?.key}-${index}`}>
                                  {item}
                                </div>
                              ))}
                            </div>
                            {redisState.details.previewTruncated ? (
                              <div className="inline-note">
                                Preview truncated to avoid loading large Redis values into the UI.
                              </div>
                            ) : null}
                          </div>
                        ) : (
                          <div className="empty-note">
                            Pick a key to inspect its type, TTL, encoding, and preview payload.
                          </div>
                        )}
                      </article>

                      <article className="service-card data-card data-card--wide">
                        <div className="stack-card__title">
                          <Command size={15} />
                          <span>Command Editor</span>
                        </div>
                        <div className="form-stack">
                          <label className="field-stack">
                            <span className="field-label">Redis command</span>
                            <textarea
                              className="field-textarea field-textarea--editor field-textarea--compact"
                              onChange={(event) => setRedisCommand(event.currentTarget.value)}
                              placeholder='GET "user:42"'
                              rows={5}
                              value={redisCommand}
                            />
                          </label>
                          <div className="button-row">
                            <button
                              className="mini-button"
                              disabled={!canBrowseRedis || redisCommandBusy}
                              onClick={() => void runRedisCommand()}
                              type="button"
                            >
                              {redisCommandBusy ? "Running..." : "Run Command"}
                            </button>
                          </div>
                        </div>
                      </article>

                      <article className="service-card data-card data-card--wide">
                        <div className="stack-card__title">
                          <LayoutDashboard size={15} />
                          <span>Command Response</span>
                        </div>
                        <RedisCommandPanel
                          emptyLabel="Run a Redis command to inspect the raw reply."
                          error={redisCommandError}
                          result={redisCommandResult}
                        />
                      </article>
                    </div>
                  )}
                </section>

                <section className="diff-section">
                  <div className="diff-section__header">
                    <div className="section-title">
                      <GitBranch size={15} />
                      <span>Diff Preview</span>
                    </div>
                    <div className="diff-actions">
                      <button
                        className="mini-button"
                        disabled={!selectedChange || gitActionBusy}
                        onClick={() => void stageSelectedChange()}
                        type="button"
                      >
                        {selectedChange?.staged ? "Unstage Selected" : "Stage Selected"}
                      </button>
                      <button
                        className="mini-button"
                        disabled={!gitOverview || gitOverview.isClean || gitActionBusy}
                        onClick={() => void stageAllChanges()}
                        type="button"
                      >
                        Stage All
                      </button>
                      <button
                        className="mini-button"
                        disabled={!gitOverview || gitOverview.stagedCount === 0 || gitActionBusy}
                        onClick={() => void unstageAllChanges()}
                        type="button"
                      >
                        Unstage All
                      </button>
                      <button
                        className="mini-button"
                        disabled={!canDiscardSelected}
                        onClick={() => void discardSelectedChange()}
                        type="button"
                      >
                        Discard Selected
                      </button>
                    </div>
                  </div>

                  {selectedChange ? (
                    <>
                      <div className="diff-meta">
                        <span className="diff-path">{selectedChange.path}</span>
                        <span
                          className={
                            selectedChange.staged
                              ? "git-badge git-badge--staged"
                              : "git-badge"
                          }
                        >
                          {selectedChange.status}
                        </span>
                      </div>

                      {discardHint ? <div className="inline-note">{discardHint}</div> : null}

                      {gitDiffLoading ? (
                        <div className="diff-placeholder">Loading diff…</div>
                      ) : gitDiffError ? (
                        <div className="diff-placeholder diff-placeholder--error">
                          {gitDiffError}
                        </div>
                      ) : (
                        <pre className="diff-viewer">{gitDiffText || "No diff output."}</pre>
                      )}
                    </>
                  ) : (
                    <div className="diff-placeholder">
                      Pick a changed file in the inspector to preview its patch.
                    </div>
                  )}
                </section>
              </main>
            </Panel>

            <Separator className="resize-handle resize-handle--col" />

            <Panel defaultSize={24} minSize={18}>
              <aside className="pane pane--right">
                <div className="pane__header">
                  <div>
                    <p className="eyebrow">Inspector</p>
                    <h2>{rightStatus}</h2>
                  </div>
                  <span className="status-pill status-pill--live">
                    {gitOverview ? "Repo Ready" : "Scanning"}
                  </span>
                </div>

                <section className="stack-card">
                  <div className="stack-card__title">
                    <SquareTerminal size={15} />
                    <span>Terminal Target</span>
                  </div>
                  <div className="form-stack">
                    <div className="button-row">
                      <button
                        className="mini-button"
                        disabled={terminalTarget.kind === "local" && !terminalSession}
                        onClick={() => void switchToLocalTerminal()}
                        type="button"
                      >
                        Local Shell
                      </button>
                      <button
                        className="mini-button"
                        disabled={!canConnectSsh}
                        onClick={() => void connectSshTerminal()}
                        type="button"
                      >
                        Connect SSH
                      </button>
                    </div>

                    <div className="field-grid">
                      <label className="field-stack">
                        <span className="field-label">Host</span>
                        <input
                          className="field-input"
                          onChange={(event) => setSshHost(event.currentTarget.value)}
                          placeholder="server.example.com"
                          value={sshHost}
                        />
                      </label>
                      <label className="field-stack">
                        <span className="field-label">Port</span>
                        <input
                          className="field-input field-input--narrow"
                          inputMode="numeric"
                          onChange={(event) => setSshPort(event.currentTarget.value)}
                          placeholder="22"
                          value={sshPort}
                        />
                      </label>
                    </div>

                    <label className="field-stack">
                      <span className="field-label">Auth mode</span>
                      <select
                        className="field-input field-select"
                        onChange={(event) =>
                          setSshAuthMode(
                            event.currentTarget.value as "password" | "agent" | "key",
                          )
                        }
                        value={sshAuthMode}
                      >
                        <option value="password">Password</option>
                        <option value="agent">SSH Agent</option>
                        <option value="key">Key File</option>
                      </select>
                    </label>

                    <div className="field-grid">
                      <label className="field-stack">
                        <span className="field-label">User</span>
                        <input
                          className="field-input"
                          onChange={(event) => setSshUser(event.currentTarget.value)}
                          placeholder="root"
                          value={sshUser}
                        />
                      </label>
                      {sshNeedsPassword ? (
                        <label className="field-stack">
                          <span className="field-label">Password</span>
                          <input
                            className="field-input"
                            onChange={(event) => setSshPassword(event.currentTarget.value)}
                            placeholder="Password"
                            type="password"
                            value={sshPassword}
                          />
                        </label>
                      ) : sshNeedsKeyPath ? (
                        <label className="field-stack">
                          <span className="field-label">Key file</span>
                          <input
                            className="field-input"
                            onChange={(event) => setSshKeyPath(event.currentTarget.value)}
                            placeholder="C:\\Users\\me\\.ssh\\id_ed25519"
                            value={sshKeyPath}
                          />
                        </label>
                      ) : (
                        <div className="status-note">
                          Agent auth uses the system SSH agent without passing a secret through the
                          UI.
                        </div>
                      )}
                    </div>

                    <div className="status-note">
                      Active terminal target: {currentTerminalLabel}
                    </div>
                    {sshNeedsKeyPath ? (
                      <div className="inline-note">
                        Key-file auth currently assumes the key is unencrypted or the passphrase
                        is already handled outside this UI.
                      </div>
                    ) : null}

                    <label className="field-stack">
                      <span className="field-label">Saved connection label</span>
                      <div className="branch-row">
                        <input
                          className="field-input"
                          onChange={(event) => setSshConnectionName(event.currentTarget.value)}
                          placeholder="prod-api / staging-bastion"
                          value={sshConnectionName}
                        />
                        <button
                          className="mini-button"
                          disabled={!canSaveSshConnection}
                          onClick={() => void saveCurrentSshConnection()}
                          type="button"
                        >
                          Save
                        </button>
                      </div>
                    </label>

                    {sshConnectionsNotice ? (
                      <div className="status-note">{sshConnectionsNotice}</div>
                    ) : null}
                    {sshConnectionsError ? (
                      <div className="status-note status-note--error">{sshConnectionsError}</div>
                    ) : null}

                    {savedConnections.length > 0 ? (
                      <div className="connection-list">
                        {savedConnections.map((connection) => (
                          <div className="connection-row" key={`${connection.index}-${connection.name}`}>
                            <div className="connection-row__head">
                              <strong>{connection.name}</strong>
                              <span className="connection-pill">{connection.authKind}</span>
                            </div>
                            <div className="connection-row__meta">
                              {connection.user}@{connection.host}:{connection.port}
                            </div>
                            {connection.authKind === "key" && connection.keyPath ? (
                              <div className="inline-note">{connection.keyPath}</div>
                            ) : null}
                            <div className="connection-row__actions">
                              <button
                                className="mini-button"
                                onClick={() => void connectSavedSshConnection(connection)}
                                type="button"
                              >
                                Connect
                              </button>
                              <button
                                className="mini-button"
                                onClick={() => void deleteSavedSshConnection(connection)}
                                type="button"
                              >
                                Delete
                              </button>
                            </div>
                          </div>
                        ))}
                      </div>
                    ) : (
                      <div className="empty-note">
                        Saved SSH connections will appear here once you store a reusable target.
                      </div>
                    )}
                  </div>
                </section>

                <section className="stack-card">
                  <div className="stack-card__title">
                    <GitBranch size={15} />
                    <span>Repository</span>
                  </div>
                  {gitOverview ? (
                    <>
                      <ul className="stack-list">
                        <li>
                          <span>Branch</span>
                          <strong>{gitOverview.branchName}</strong>
                        </li>
                        <li>
                          <span>Tracking</span>
                          <strong>{gitOverview.tracking || "Detached / local only"}</strong>
                        </li>
                        <li>
                          <span>Ahead / Behind</span>
                          <strong>
                            {gitOverview.ahead} / {gitOverview.behind}
                          </strong>
                        </li>
                        <li>
                          <span>Staged / Unstaged</span>
                          <strong>
                            {gitOverview.stagedCount} / {gitOverview.unstagedCount}
                          </strong>
                        </li>
                      </ul>
                      <p className="stack-card__path">{gitOverview.repoPath}</p>
                    </>
                  ) : (
                    <div className="empty-note">
                      {gitError || "Current path is not a Git repository."}
                    </div>
                  )}
                </section>

                <section className="stack-card">
                  <div className="stack-card__title">
                    <PlugZap size={15} />
                    <span>Repository Actions</span>
                  </div>
                  {gitOverview ? (
                    <div className="form-stack">
                      <label className="field-stack">
                        <span className="field-label">Switch branch</span>
                        <div className="branch-row">
                          <select
                            className="field-input field-select"
                            disabled={!gitBranches.length || gitActionBusy}
                            onChange={(event) =>
                              setBranchSwitchTarget(event.currentTarget.value)
                            }
                            value={branchSwitchTarget}
                          >
                            {gitBranches.map((branch) => (
                              <option key={branch} value={branch}>
                                {branch}
                              </option>
                            ))}
                          </select>
                          <button
                            className="mini-button"
                            disabled={!canSwitchBranch}
                            onClick={() => void switchBranch()}
                            type="button"
                          >
                            Switch
                          </button>
                        </div>
                      </label>

                      <label className="field-stack">
                        <span className="field-label">Commit staged changes</span>
                        <textarea
                          className="field-textarea"
                          disabled={gitActionBusy}
                          onChange={(event) => setCommitMessage(event.currentTarget.value)}
                          placeholder={
                            gitOverview.stagedCount > 0
                              ? "Summarize the staged change in one line."
                              : "Stage files to enable commit."
                          }
                          rows={3}
                          value={commitMessage}
                        />
                      </label>

                      <div className="commit-toolbar">
                        <span className="inline-note">
                          {gitOverview.stagedCount} staged / {gitOverview.unstagedCount} unstaged
                        </span>
                        <button
                          className="mini-button"
                          disabled={!canCommit}
                          onClick={() => void commitStagedChanges()}
                          type="button"
                        >
                          Commit Staged
                        </button>
                      </div>

                      {gitNotice ? <div className="status-note">{gitNotice}</div> : null}
                      {gitOperationError ? (
                        <div className="status-note status-note--error">{gitOperationError}</div>
                      ) : null}
                    </div>
                  ) : (
                    <div className="empty-note">
                      Open a repository to enable branch and commit actions.
                    </div>
                  )}
                </section>

                <section className="stack-card">
                  <div className="stack-card__title">
                    <PlugZap size={15} />
                    <span>Sync &amp; Stash</span>
                  </div>
                  {gitOverview ? (
                    <div className="form-stack">
                      <div className="button-row">
                        <button
                          className="mini-button"
                          disabled={!canSync}
                          onClick={() => void pullCurrentBranch()}
                          type="button"
                        >
                          Pull
                        </button>
                        <button
                          className="mini-button"
                          disabled={!canSync}
                          onClick={() => void pushCurrentBranch()}
                          type="button"
                        >
                          Push
                        </button>
                      </div>

                      <label className="field-stack">
                        <span className="field-label">Stash current worktree</span>
                        <div className="branch-row">
                          <input
                            className="field-input"
                            disabled={gitActionBusy}
                            onChange={(event) => setStashMessage(event.currentTarget.value)}
                            placeholder="Optional stash label"
                            value={stashMessage}
                          />
                          <button
                            className="mini-button"
                            disabled={!canStash}
                            onClick={() => void stashCurrentChanges()}
                            type="button"
                          >
                            Stash
                          </button>
                        </div>
                      </label>

                      {gitStashes.length > 0 ? (
                        <div className="stash-list">
                          {gitStashes.map((stash) => (
                            <div className="stash-row" key={stash.index}>
                              <div className="stash-row__head">
                                <span className="commit-hash">{stash.index}</span>
                                <span className="inline-note">{stash.relativeDate}</span>
                              </div>
                              <div className="stash-row__message">
                                {stash.message || "WIP stash"}
                              </div>
                              <div className="stash-row__actions">
                                <button
                                  className="mini-button"
                                  disabled={!canSync}
                                  onClick={() => void restoreStash(stash.index, false)}
                                  type="button"
                                >
                                  Apply
                                </button>
                                <button
                                  className="mini-button"
                                  disabled={!canSync}
                                  onClick={() => void restoreStash(stash.index, true)}
                                  type="button"
                                >
                                  Pop
                                </button>
                                <button
                                  className="mini-button"
                                  disabled={!canSync}
                                  onClick={() => void dropStash(stash.index)}
                                  type="button"
                                >
                                  Drop
                                </button>
                              </div>
                            </div>
                          ))}
                        </div>
                      ) : (
                        <div className="empty-note">
                          No stash entries yet. Use this area to park local work before switching.
                        </div>
                      )}
                    </div>
                  ) : (
                    <div className="empty-note">
                      Open a repository to enable remote sync and stash operations.
                    </div>
                  )}
                </section>

                <section className="stack-card">
                  <div className="stack-card__title">
                    <Database size={15} />
                    <span>Working Tree</span>
                  </div>
                  {gitOverview && visibleChangeCount > 0 ? (
                    <div className="git-change-list">
                      {gitOverview.changes.map((change) => (
                        <button
                          key={`${change.path}:${change.staged}`}
                          className={
                            changeKey(change) === selectedChangeKey
                              ? "git-change-button git-change-button--selected"
                              : "git-change-button"
                          }
                          onClick={() => setSelectedChangeKey(changeKey(change))}
                          type="button"
                        >
                          <span
                            className={
                              change.staged ? "git-badge git-badge--staged" : "git-badge"
                            }
                          >
                            {change.status}
                          </span>
                          <span className="git-change-row__path">{change.path}</span>
                        </button>
                      ))}
                    </div>
                  ) : (
                    <div className="empty-note">
                      {gitOverview?.isClean
                        ? "Workspace clean."
                        : "No tracked changes surfaced yet."}
                    </div>
                  )}
                </section>

                <section className="stack-card">
                  <div className="stack-card__title">
                    <History size={15} />
                    <span>Recent Commits</span>
                  </div>
                  {gitOverview ? (
                    recentCommits.length > 0 ? (
                      <div className="history-list">
                        {recentCommits.map((commit) => (
                          <div className="history-row" key={commit.hash}>
                            <div className="history-row__head">
                              <span className="commit-hash">{commit.shortHash}</span>
                              <span className="inline-note">{commit.relativeDate}</span>
                            </div>
                            <div className="history-row__message">{commit.message}</div>
                            <div className="history-row__meta">
                              {commit.author}
                              {commit.refs ? ` • ${commit.refs}` : ""}
                            </div>
                          </div>
                        ))}
                      </div>
                    ) : (
                      <div className="empty-note">
                        This repository does not have a visible commit history yet.
                      </div>
                    )
                  ) : (
                    <div className="empty-note">
                      Open a repository to inspect branch history.
                    </div>
                  )}
                </section>

                {error ? <div className="error-card">{error}</div> : null}
              </aside>
            </Panel>
          </Group>
        </Panel>

        <Separator className="resize-handle resize-handle--row" />

        <Panel defaultSize={22} minSize={16}>
          <section className="terminal-panel">
            <div className="terminal-panel__header">
              <div className="terminal-panel__title">
                <SquareTerminal size={15} />
                <span>Integrated Terminal</span>
              </div>
              <div className="terminal-panel__meta">
                <span className="meta-pill meta-pill--success">
                  {terminalSession?.shell ?? currentTerminalLabel}
                </span>
                <span className="meta-pill">
                  {terminalSnapshot
                    ? `${terminalSnapshot.cols} × ${terminalSnapshot.rows}`
                    : `${terminalSize.cols} × ${terminalSize.rows}`}
                </span>
                {scrollbackOffset > 0 ? (
                  <button
                    className="mini-button"
                    onClick={() => setScrollbackOffset(0)}
                    type="button"
                  >
                    Follow Live
                  </button>
                ) : null}
                <button className="mini-button" onClick={() => void restartTerminal()} type="button">
                  Restart
                </button>
              </div>
            </div>

            <div
              className="terminal-viewport"
              onKeyDown={handleTerminalKeyDown}
              onMouseDown={(event) => event.currentTarget.focus()}
              onWheel={handleTerminalWheel}
              ref={terminalViewportRef}
              tabIndex={0}
            >
              <span aria-hidden className="terminal-measure" ref={terminalMeasureRef}>
                MMMMMMMMMM
              </span>

              {terminalError ? (
                <div className="terminal-placeholder terminal-placeholder--error">
                  {terminalError}
                </div>
              ) : terminalSnapshot ? (
                <div className="terminal-screen">
                  {terminalSnapshot.lines.map((line, index) => (
                    <div className="terminal-row" key={`line-${index}`}>
                      {line.segments.map((segment, segmentIndex) => (
                        <span
                          className={
                            segment.cursor
                              ? "terminal-segment terminal-segment--cursor"
                              : "terminal-segment"
                          }
                          key={`segment-${index}-${segmentIndex}`}
                          style={{
                            backgroundColor: segment.cursor ? undefined : segment.bg,
                            color: segment.cursor ? undefined : segment.fg,
                            fontWeight: segment.bold ? 510 : 400,
                            textDecoration: segment.underline ? "underline" : "none",
                          }}
                        >
                          {segment.text}
                        </span>
                      ))}
                    </div>
                  ))}
                </div>
              ) : (
                <div className="terminal-placeholder">Launching shell...</div>
              )}
            </div>
          </section>
        </Panel>
      </Group>
    </div>
  );
}

export default App;
