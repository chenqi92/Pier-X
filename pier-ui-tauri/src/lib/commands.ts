// ── Tauri Command Wrappers ───────────────────────────────────────
// Typed wrappers for all invoke() calls to pier-core via Tauri IPC.

import { invoke } from "@tauri-apps/api/core";
import type {
  CoreInfo,
  DetectedServiceView,
  DockerOverview,
  GitBlameLineView,
  FileEntry,
  GitCommitDetailView,
  GitCommitEntry,
  GitComparisonFileView,
  GitConfigEntryView,
  GitConflictFileView,
  GitConflictHunkView,
  GitGraphHistoryParams,
  GitGraphMetadata,
  GitGraphRowView,
  GitOverview,
  GitPanelState,
  GitRemoteView,
  GitRebaseItemView,
  GitRebasePlanView,
  GitStashEntry,
  GitSubmoduleView,
  GitTagView,
  MysqlBrowserState,
  PostgresBrowserState,
  QueryExecutionResult,
  RedisBrowserState,
  RedisCommandResult,
  LogEventView,
  SavedSshConnection,
  ServerSnapshotView,
  SftpBrowseState,
  SqliteBrowserState,
  TerminalSessionInfo,
  TerminalSnapshot,
  TunnelInfoView,
} from "./types";

// ── Core ────────────────────────────────────────────────────────

export const coreInfo = () => invoke<CoreInfo>("core_info");

export const listDirectory = (path?: string) =>
  invoke<FileEntry[]>("list_directory", { path: path ?? null });

// ── Git ─────────────────────────────────────────────────────────

export const gitOverview = (path?: string) =>
  invoke<GitOverview>("git_overview", { path: path ?? null });

export const gitPanelState = (path?: string | null) =>
  invoke<GitPanelState>("git_panel_state", { path: path ?? null });

export const gitInitRepo = (path?: string | null) =>
  invoke<string>("git_init_repo", { path: path ?? null });

export const gitDiff = (path: string | null, filePath: string, staged: boolean, untracked?: boolean) =>
  invoke<string>("git_diff", { path, filePath, staged, untracked: !!untracked });

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

export const gitCommitAndPush = (path: string | null, message: string) =>
  invoke<string>("git_commit_and_push", { path, message });

export const gitBranchList = (path: string | null) =>
  invoke<string[]>("git_branch_list", { path });

export const gitCheckoutBranch = (path: string | null, name: string) =>
  invoke<string>("git_checkout_branch", { path, name });

export const gitCheckoutTarget = (path: string | null, target: string, tracking?: string | null) =>
  invoke<string>("git_checkout_target", { path, target, tracking: tracking ?? null });

export const gitCreateBranch = (path: string | null, name: string) =>
  invoke<string>("git_create_branch", { path, name });

export const gitCreateBranchAt = (path: string | null, name: string, startPoint?: string | null) =>
  invoke<string>("git_create_branch_at", { path, name, startPoint: startPoint ?? null });

export const gitDeleteBranch = (path: string | null, name: string) =>
  invoke<string>("git_delete_branch", { path, name });

export const gitRenameBranch = (path: string | null, oldName: string, newName: string) =>
  invoke<string>("git_rename_branch", { path, oldName, newName });

export const gitRenameRemoteBranch = (path: string | null, remoteName: string, oldBranch: string, newName: string) =>
  invoke<string>("git_rename_remote_branch", { path, remoteName, oldBranch, newName });

export const gitDeleteRemoteBranch = (path: string | null, remoteName: string, branchName: string) =>
  invoke<string>("git_delete_remote_branch", { path, remoteName, branchName });

export const gitMergeBranch = (path: string | null, name: string) =>
  invoke<string>("git_merge_branch", { path, name });

export const gitSetBranchTracking = (path: string | null, branchName: string, upstream: string) =>
  invoke<string>("git_set_branch_tracking", { path, branchName, upstream });

export const gitUnsetBranchTracking = (path: string | null, branchName: string) =>
  invoke<string>("git_unset_branch_tracking", { path, branchName });

export const gitRecentCommits = (path: string | null, limit?: number) =>
  invoke<GitCommitEntry[]>("git_recent_commits", { path, limit: limit ?? null });

export const gitGraphMetadata = (path: string | null) =>
  invoke<GitGraphMetadata>("git_graph_metadata", { path });

export const gitGraphHistory = (params: GitGraphHistoryParams) =>
  invoke<GitGraphRowView[]>("git_graph_history", { params });

export const gitCommitDetail = (path: string | null, hash: string) =>
  invoke<GitCommitDetailView>("git_commit_detail", { path, hash });

export const gitCommitFileDiff = (path: string | null, hash: string, filePath: string) =>
  invoke<string>("git_commit_file_diff", { path, hash, filePath });

export const gitComparisonFiles = (path: string | null, hash: string) =>
  invoke<GitComparisonFileView[]>("git_comparison_files", { path, hash });

export const gitComparisonDiff = (path: string | null, hash: string, filePath: string) =>
  invoke<string>("git_comparison_diff", { path, hash, filePath });

export const gitBlameFile = (path: string | null, filePath: string) =>
  invoke<GitBlameLineView[]>("git_blame_file", { path, filePath });

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

export const gitTagsList = (path: string | null) =>
  invoke<GitTagView[]>("git_tags_list", { path });

export const gitCreateTag = (path: string | null, name: string, message: string) =>
  invoke<string>("git_create_tag", { path, name, message });

export const gitCreateTagAt = (path: string | null, name: string, target: string | null, message: string) =>
  invoke<string>("git_create_tag_at", { path, name, target, message });

export const gitDeleteTag = (path: string | null, name: string) =>
  invoke<string>("git_delete_tag", { path, name });

export const gitPushTag = (path: string | null, name: string) =>
  invoke<string>("git_push_tag", { path, name });

export const gitPushAllTags = (path: string | null) =>
  invoke<string>("git_push_all_tags", { path });

export const gitRemotesList = (path: string | null) =>
  invoke<GitRemoteView[]>("git_remotes_list", { path });

export const gitAddRemote = (path: string | null, name: string, url: string) =>
  invoke<string>("git_add_remote", { path, name, url });

export const gitSetRemoteUrl = (path: string | null, name: string, url: string) =>
  invoke<string>("git_set_remote_url", { path, name, url });

export const gitRemoveRemote = (path: string | null, name: string) =>
  invoke<string>("git_remove_remote", { path, name });

export const gitFetchRemote = (path: string | null, name?: string | null) =>
  invoke<string>("git_fetch_remote", { path, name: name ?? null });

export const gitConfigList = (path: string | null) =>
  invoke<GitConfigEntryView[]>("git_config_list", { path });

export const gitSetConfigValue = (path: string | null, key: string, value: string, global: boolean) =>
  invoke<string>("git_set_config_value", { path, key, value, global });

export const gitUnsetConfigValue = (path: string | null, key: string, global: boolean) =>
  invoke<string>("git_unset_config_value", { path, key, global });

export const gitResetToCommit = (path: string | null, hash: string, mode: string) =>
  invoke<string>("git_reset_to_commit", { path, hash, mode });

export const gitAmendHeadCommitMessage = (path: string | null, hash: string, message: string) =>
  invoke<string>("git_amend_head_commit_message", { path, hash, message });

export const gitDropCommit = (path: string | null, hash: string, parentHash?: string | null) =>
  invoke<string>("git_drop_commit", { path, hash, parentHash: parentHash ?? null });

export const gitRebasePlan = (path: string | null, count?: number | null) =>
  invoke<GitRebasePlanView>("git_rebase_plan", { path, count: count ?? null });

export const gitExecuteRebase = (path: string | null, items: GitRebaseItemView[], onto?: string | null) =>
  invoke<string>("git_execute_rebase", { path, items, onto: onto ?? null });

export const gitAbortRebase = (path: string | null) =>
  invoke<string>("git_abort_rebase", { path });

export const gitContinueRebase = (path: string | null) =>
  invoke<string>("git_continue_rebase", { path });

export const gitSubmodulesList = (path: string | null) =>
  invoke<GitSubmoduleView[]>("git_submodules_list", { path });

export const gitInitSubmodules = (path: string | null) =>
  invoke<string>("git_init_submodules", { path });

export const gitUpdateSubmodules = (path: string | null, recursive = true) =>
  invoke<string>("git_update_submodules", { path, recursive });

export const gitSyncSubmodules = (path: string | null) =>
  invoke<string>("git_sync_submodules", { path });

export const gitConflictsList = (path: string | null) =>
  invoke<GitConflictFileView[]>("git_conflicts_list", { path });

export const gitConflictAcceptAll = (path: string | null, filePath: string, resolution: string) =>
  invoke<string>("git_conflict_accept_all", { path, filePath, resolution });

export const gitConflictMarkResolved = (path: string | null, filePath: string, hunks: GitConflictHunkView[]) =>
  invoke<string>("git_conflict_mark_resolved", { params: { path, filePath, hunks } });

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
}) => invoke<void>("ssh_connection_save", {
  name: params.name,
  host: params.host,
  port: params.port,
  user: params.user,
  authMode: params.authKind,
  password: params.password || null,
  keyPath: params.keyPath || null,
});

export const sshConnectionUpdate = (params: {
  index: number;
  name: string;
  host: string;
  port: number;
  user: string;
  authKind: string;
  password: string;
  keyPath: string;
}) => invoke<void>("ssh_connection_update", {
  index: params.index,
  name: params.name,
  host: params.host,
  port: params.port,
  user: params.user,
  authMode: params.authKind,
  password: params.password || null,
  keyPath: params.keyPath || null,
});

export const sshConnectionDelete = (index: number) =>
  invoke<void>("ssh_connection_delete", { index });

/**
 * Resolve the stored password for a saved SSH connection from the OS
 * keychain. Returns an empty string for non-password auth. Use this to
 * prime in-memory state on the frontend so probe/detect/docker/db
 * commands that require an explicit password parameter can work for
 * saved connections, without persisting the secret.
 */
export const sshConnectionResolvePassword = (index: number) =>
  invoke<string>("ssh_connection_resolve_password", { index });

export const sshTunnelOpen = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  remoteHost: string;
  remotePort: number;
  localPort?: number | null;
}) => invoke<TunnelInfoView>("ssh_tunnel_open", { ...params, localPort: params.localPort ?? null });

export const sshTunnelInfo = (tunnelId: string) =>
  invoke<TunnelInfoView>("ssh_tunnel_info", { tunnelId });

export const sshTunnelClose = (tunnelId: string) =>
  invoke<void>("ssh_tunnel_close", { tunnelId });

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

export const terminalSetScrollbackLimit = (sessionId: string, limit: number) =>
  invoke<void>("terminal_set_scrollback_limit", { sessionId, limit });

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

// ── Service Detection ───────────────────────────────────────────

export const detectServices = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
}) => invoke<DetectedServiceView[]>("detect_services", params);

// ── Docker Extended ─────────────────────────────────────────────

export const dockerInspect = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  containerId: string;
}) => invoke<string>("docker_inspect", params);

export const dockerRemoveImage = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  imageId: string;
  force: boolean;
}) => invoke<void>("docker_remove_image", params);

export const dockerRemoveVolume = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  volumeName: string;
}) => invoke<void>("docker_remove_volume", params);

export const dockerRemoveNetwork = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  networkName: string;
}) => invoke<void>("docker_remove_network", params);

// ── SFTP Extended ───────────────────────────────────────────────

export const sftpMkdir = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  path: string;
}) => invoke<void>("sftp_mkdir", params);

export const sftpRemove = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  path: string;
  isDir: boolean;
}) => invoke<void>("sftp_remove", params);

export const sftpRename = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  from: string;
  to: string;
}) => invoke<void>("sftp_rename", params);

export const sftpDownload = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  remotePath: string;
  localPath: string;
}) => invoke<void>("sftp_download", params);

export const sftpUpload = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  localPath: string;
  remotePath: string;
}) => invoke<void>("sftp_upload", params);

// ── Log Stream ──────────────────────────────────────────────────

export const logStreamStart = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  command: string;
}) => invoke<string>("log_stream_start", params);

export const logStreamDrain = (streamId: string) =>
  invoke<LogEventView[]>("log_stream_drain", { streamId });

export const logStreamStop = (streamId: string) =>
  invoke<void>("log_stream_stop", { streamId });

// ── Local System ────────────────────────────────────────────────

export const localDockerOverview = (all: boolean) =>
  invoke<DockerOverview>("local_docker_overview", { all });

export const localDockerAction = (containerId: string, action: string) =>
  invoke<string>("local_docker_action", { containerId, action });

export const localSystemInfo = () =>
  invoke<ServerSnapshotView>("local_system_info");

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
