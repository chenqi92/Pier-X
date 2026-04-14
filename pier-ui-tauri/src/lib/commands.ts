// ── Tauri Command Wrappers ───────────────────────────────────────
// Typed wrappers for all invoke() calls to pier-core via Tauri IPC.

import { invoke } from "@tauri-apps/api/core";
import type {
  CoreInfo,
  DockerOverview,
  FileEntry,
  GitCommitEntry,
  GitOverview,
  GitStashEntry,
  MysqlBrowserState,
  PostgresBrowserState,
  QueryExecutionResult,
  RedisBrowserState,
  RedisCommandResult,
  SavedSshConnection,
  ServerSnapshotView,
  SftpBrowseState,
  SqliteBrowserState,
  TerminalSessionInfo,
  TerminalSnapshot,
} from "./types";

// ── Core ────────────────────────────────────────────────────────

export const coreInfo = () => invoke<CoreInfo>("core_info");

export const listDirectory = (path?: string) =>
  invoke<FileEntry[]>("list_directory", { path: path ?? null });

// ── Git ─────────────────────────────────────────────────────────

export const gitOverview = (path?: string) =>
  invoke<GitOverview>("git_overview", { path: path ?? null });

export const gitDiff = (path: string | null, filePath: string, staged: boolean) =>
  invoke<string>("git_diff", { path, filePath, staged });

export const gitStagePaths = (path: string | null, paths: string[]) =>
  invoke<void>("git_stage_paths", { path, paths });

export const gitUnstagePaths = (path: string | null, paths: string[]) =>
  invoke<void>("git_unstage_paths", { path, paths });

export const gitStageAll = (path: string | null) =>
  invoke<void>("git_stage_all", { path });

export const gitUnstageAll = (path: string | null) =>
  invoke<void>("git_unstage_all", { path });

export const gitDiscardPaths = (path: string | null, paths: string[]) =>
  invoke<void>("git_discard_paths", { path, paths });

export const gitCommit = (path: string | null, message: string) =>
  invoke<string>("git_commit", { path, message });

export const gitBranchList = (path: string | null) =>
  invoke<string[]>("git_branch_list", { path });

export const gitCheckoutBranch = (path: string | null, name: string) =>
  invoke<string>("git_checkout_branch", { path, name });

export const gitRecentCommits = (path: string | null, limit?: number) =>
  invoke<GitCommitEntry[]>("git_recent_commits", { path, limit: limit ?? null });

export const gitPush = (path: string | null) =>
  invoke<string>("git_push", { path });

export const gitPull = (path: string | null) =>
  invoke<string>("git_pull", { path });

export const gitStashList = (path: string | null) =>
  invoke<GitStashEntry[]>("git_stash_list", { path });

export const gitStashPush = (path: string | null, message: string) =>
  invoke<string>("git_stash_push", { path, message });

export const gitStashApply = (path: string | null, index: string) =>
  invoke<string>("git_stash_apply", { path, index });

export const gitStashPop = (path: string | null, index: string) =>
  invoke<string>("git_stash_pop", { path, index });

export const gitStashDrop = (path: string | null, index: string) =>
  invoke<string>("git_stash_drop", { path, index });

// ── SSH Connections ─────────────────────────────────────────────

export const sshConnectionsList = () =>
  invoke<SavedSshConnection[]>("ssh_connections_list");

export const sshConnectionSave = (params: {
  name: string;
  host: string;
  port: number;
  user: string;
  authKind: string;
  password: string;
  keyPath: string;
}) => invoke<void>("ssh_connection_save", params);

export const sshConnectionDelete = (index: number) =>
  invoke<void>("ssh_connection_delete", { index });

// ── Terminal ────────────────────────────────────────────────────

export const terminalCreate = (cols: number, rows: number, shell?: string) =>
  invoke<TerminalSessionInfo>("terminal_create", {
    cols,
    rows,
    shell: shell ?? null,
  });

export const terminalCreateSsh = (params: {
  cols: number;
  rows: number;
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
}) => invoke<TerminalSessionInfo>("terminal_create_ssh", params);

export const terminalCreateSshSaved = (
  cols: number,
  rows: number,
  index: number,
) => invoke<TerminalSessionInfo>("terminal_create_ssh_saved", { cols, rows, index });

export const terminalWrite = (sessionId: string, data: string) =>
  invoke<number>("terminal_write", { sessionId, data });

export const terminalResize = (sessionId: string, cols: number, rows: number) =>
  invoke<void>("terminal_resize", { sessionId, cols, rows });

export const terminalSnapshot = (sessionId: string, scrollbackOffset: number) =>
  invoke<TerminalSnapshot>("terminal_snapshot", { sessionId, scrollbackOffset });

export const terminalClose = (sessionId: string) =>
  invoke<void>("terminal_close", { sessionId });

// ── MySQL ───────────────────────────────────────────────────────

export const mysqlBrowse = (params: {
  host: string;
  port: number;
  user: string;
  password: string;
  database?: string | null;
  table?: string | null;
}) => invoke<MysqlBrowserState>("mysql_browse", params);

export const mysqlExecute = (params: {
  host: string;
  port: number;
  user: string;
  password: string;
  database?: string | null;
  sql: string;
}) => invoke<QueryExecutionResult>("mysql_execute", params);

// ── SQLite ──────────────────────────────────────────────────────

export const sqliteBrowse = (path: string, table?: string | null) =>
  invoke<SqliteBrowserState>("sqlite_browse", { path, table: table ?? null });

export const sqliteExecute = (path: string, sql: string) =>
  invoke<QueryExecutionResult>("sqlite_execute", { path, sql });

// ── Redis ───────────────────────────────────────────────────────

export const redisBrowse = (params: {
  host: string;
  port: number;
  db: number;
  pattern: string;
  key?: string | null;
}) => invoke<RedisBrowserState>("redis_browse", params);

export const redisExecute = (params: {
  host: string;
  port: number;
  db: number;
  command: string;
}) => invoke<RedisCommandResult>("redis_execute", params);

// ── PostgreSQL ──────────────────────────────────────────────────

export const postgresBrowse = (params: {
  host: string;
  port: number;
  user: string;
  password: string;
  database?: string | null;
  schema?: string | null;
  table?: string | null;
}) => invoke<PostgresBrowserState>("postgres_browse", params);

export const postgresExecute = (params: {
  host: string;
  port: number;
  user: string;
  password: string;
  database?: string | null;
  sql: string;
}) => invoke<QueryExecutionResult>("postgres_execute", params);

// ── Docker ──────────────────────────────────────────────────────

export const dockerOverview = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  all: boolean;
}) => invoke<DockerOverview>("docker_overview", params);

export const dockerContainerAction = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  containerId: string;
  action: string;
}) => invoke<string>("docker_container_action", params);

// ── SFTP ────────────────────────────────────────────────────────

export const sftpBrowse = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  path?: string | null;
}) => invoke<SftpBrowseState>("sftp_browse", params);

// ── Markdown ────────────────────────────────────────────────────

export const markdownRender = (source: string) =>
  invoke<string>("markdown_render", { source });

export const markdownRenderFile = (path: string) =>
  invoke<string>("markdown_render_file", { path });

// ── Server Monitor ──────────────────────────────────────────────

export const serverMonitorProbe = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
}) => invoke<ServerSnapshotView>("server_monitor_probe", params);

// ── Utility Functions ───────────────────────────────────────────

const readOnlySqlKeywords = new Set([
  "SELECT", "SHOW", "DESCRIBE", "DESC", "EXPLAIN", "PRAGMA", "HELP",
  "USE", "SET", "BEGIN", "START", "COMMIT", "ROLLBACK",
]);

export function leadingSqlKeyword(sql: string): string | null {
  let remaining = sql.trimStart();
  while (remaining.length > 0) {
    if (remaining.startsWith("--")) {
      const newlineIndex = remaining.indexOf("\n");
      if (newlineIndex < 0) return null;
      remaining = remaining.slice(newlineIndex + 1).trimStart();
      continue;
    }
    if (remaining.startsWith("/*")) {
      const commentEnd = remaining.indexOf("*/", 2);
      if (commentEnd < 0) return null;
      remaining = remaining.slice(commentEnd + 2).trimStart();
      continue;
    }
    break;
  }
  const match = /^[A-Za-z]+/.exec(remaining);
  return match ? match[0].toUpperCase() : null;
}

export function isReadOnlySql(sql: string): boolean {
  const keyword = leadingSqlKeyword(sql);
  return keyword !== null && readOnlySqlKeywords.has(keyword);
}

export function queryResultToTsv(result: QueryExecutionResult): string {
  const normalizeCell = (value: string) =>
    value.replace(/[\t\n\r]/g, " ");
  const header = result.columns.map(normalizeCell).join("\t");
  const rows = result.rows.map((row) =>
    row.map(normalizeCell).join("\t"),
  );
  return [header, ...rows].join("\n");
}

export function quoteCommandArg(value: string): string {
  return /[\s"'\\]/.test(value) ? `"${value.replace(/["\\]/g, "\\$&")}"` : value;
}

export const controlKeyMap: Record<string, string> = {
  "@": "\u0000",
  "[": "\u001b",
  "\\": "\u001c",
  "]": "\u001d",
  "^": "\u001e",
  _: "\u001f",
};
