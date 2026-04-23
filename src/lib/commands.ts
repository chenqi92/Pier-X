// ── Tauri Command Wrappers ───────────────────────────────────────
// Typed wrappers for all invoke() calls to pier-core via Tauri IPC.

import { invoke } from "@tauri-apps/api/core";
import type {
  CoreInfo,
  DbCredential,
  DbCredentialInput,
  DbCredentialPatch,
  DbCredentialResolved,
  DbDetectionReport,
  DetectedServiceView,
  DockerImageView,
  DockerNetworkView,
  DockerOverview,
  DockerVolumeView,
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

/** Dev-only: toggle the Tauri webview DevTools. Returns an error in release. */
export const devToggleDevtools = () => invoke<void>("dev_toggle_devtools");

export const listDirectory = (path?: string) =>
  invoke<FileEntry[]>("list_directory", { path: path ?? null });

export const listDrives = () => invoke<FileEntry[]>("list_drives");

// Local file mutations — mirror the SFTP panel's create/rename/remove
// actions for the sidebar's local directory view. All paths are
// absolute OS paths (the sidebar tracks `currentPath` as an absolute
// string already).
export const localCreateFile = (path: string) =>
  invoke<void>("local_create_file", { path });
export const localCreateDir = (path: string) =>
  invoke<void>("local_create_dir", { path });
export const localRename = (from: string, to: string) =>
  invoke<void>("local_rename", { from, to });
export const localRemove = (path: string, isDir: boolean) =>
  invoke<void>("local_remove", { path, isDir });

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

export type GitCommitOptions = { signoff?: boolean; amend?: boolean };

export const gitCommit = (path: string | null, message: string, options?: GitCommitOptions) =>
  invoke<string>("git_commit", {
    path,
    message,
    signoff: options?.signoff ?? false,
    amend: options?.amend ?? false,
  });

export const gitCommitAndPush = (path: string | null, message: string, options?: GitCommitOptions) =>
  invoke<string>("git_commit_and_push", {
    path,
    message,
    signoff: options?.signoff ?? false,
    amend: options?.amend ?? false,
  });

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
  /** Sidebar group label. Empty / missing → default (ungrouped). */
  group?: string | null;
}) => invoke<void>("ssh_connection_save", {
  name: params.name,
  host: params.host,
  port: params.port,
  user: params.user,
  authMode: params.authKind,
  password: params.password || null,
  keyPath: params.keyPath || null,
  group: params.group && params.group.trim() ? params.group.trim() : null,
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
  /** When `undefined`, the backend preserves the existing group.
   *  Pass `null` or `""` to explicitly ungroup, or a label to reassign. */
  group?: string | null;
}) => invoke<void>("ssh_connection_update", {
  index: params.index,
  name: params.name,
  host: params.host,
  port: params.port,
  user: params.user,
  authMode: params.authKind,
  password: params.password || null,
  keyPath: params.keyPath || null,
  group: params.group === undefined
    ? null
    : params.group && params.group.trim() ? params.group.trim() : "",
});

export const sshConnectionDelete = (index: number) =>
  invoke<void>("ssh_connection_delete", { index });

/**
 * Atomic reorder + group-reassign of the saved-connections list.
 * `order[i]` is the old index of the connection that should land
 * in slot `i`. `groups[i]` is the new group label for that slot;
 * pass `null` (or an empty string) to ungroup.
 */
export const sshConnectionsReorder = (
  order: number[],
  groups: Array<string | null>,
) => invoke<void>("ssh_connections_reorder", { order, groups });

/**
 * Rename every connection whose group matches `from` to `to`.
 * `to === null` or empty strips the group (ungroups). Passing an
 * empty `from` targets connections with no explicit group.
 */
export const sshGroupRename = (from: string, to: string | null) =>
  invoke<void>("ssh_group_rename", { from, to });

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
  savedConnectionIndex?: number | null;
}) =>
  invoke<TunnelInfoView>("ssh_tunnel_open", {
    ...params,
    localPort: params.localPort ?? null,
    savedConnectionIndex: params.savedConnectionIndex ?? null,
  });

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
  savedConnectionIndex?: number | null;
}) => invoke<DockerOverview>("docker_overview", params);

export const dockerImages = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
}) => invoke<DockerImageView[]>("docker_images", params);

export const dockerVolumes = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
}) => invoke<DockerVolumeView[]>("docker_volumes", params);

export const dockerNetworks = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
}) => invoke<DockerNetworkView[]>("docker_networks", params);

export const dockerContainerAction = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  containerId: string;
  action: string;
  savedConnectionIndex?: number | null;
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
  savedConnectionIndex?: number | null;
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
  savedConnectionIndex?: number | null;
}) => invoke<ServerSnapshotView>("server_monitor_probe", params);

// ── Service Detection ───────────────────────────────────────────

export const detectServices = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
}) => invoke<DetectedServiceView[]>("detect_services", params);

// ── DB Instance Detection + Credential CRUD ────────────────────

export const dbDetect = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
}) => invoke<DbDetectionReport>("db_detect", params);

export const dbCredSave = (
  savedConnectionIndex: number,
  credential: DbCredentialInput,
  password: string | null,
) =>
  invoke<DbCredential>("db_cred_save", {
    savedConnectionIndex,
    credential,
    password,
  });

export const dbCredUpdate = (
  savedConnectionIndex: number,
  credentialId: string,
  patch: DbCredentialPatch,
  /** `undefined` = don't touch password, `null` = clear to
   *  passwordless, string = set new password. */
  newPassword?: string | null,
) =>
  invoke<DbCredential>("db_cred_update", {
    savedConnectionIndex,
    credentialId,
    patch,
    newPassword: newPassword === undefined ? undefined : newPassword,
  });

export const dbCredDelete = (savedConnectionIndex: number, credentialId: string) =>
  invoke<void>("db_cred_delete", { savedConnectionIndex, credentialId });

export const dbCredResolve = (savedConnectionIndex: number, credentialId: string) =>
  invoke<DbCredentialResolved>("db_cred_resolve", {
    savedConnectionIndex,
    credentialId,
  });

export type DockerDbEnv = {
  mysqlDatabase: string | null;
  mysqlUser: string | null;
  postgresDb: string | null;
  postgresUser: string | null;
};

/** Pull the DB-relevant env vars (`MYSQL_DATABASE`, `POSTGRES_USER`,
 *  …) out of a container's `docker inspect`. Used by the Add
 *  dialog to pre-fill form fields when the user adopts a docker
 *  instance. Missing keys → `null`. */
export const dockerInspectDbEnv = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  containerId: string;
  savedConnectionIndex?: number | null;
}) => invoke<DockerDbEnv>("docker_inspect_db_env", params);

// ── Remote SQLite ───────────────────────────────────────────────

export type RemoteSqliteCapability = {
  installed: boolean;
  version: string | null;
  supportsJson: boolean;
};

export type RemoteSqliteCandidate = {
  path: string;
  sizeBytes: number;
  modified: number | null;
};

type SshParams = {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
};

export const sqliteRemoteCapable = (params: SshParams) =>
  invoke<RemoteSqliteCapability>("sqlite_remote_capable", params);

export const sqliteBrowseRemote = (
  params: SshParams & { dbPath: string; table?: string | null },
) =>
  invoke<SqliteBrowserState>("sqlite_browse_remote", {
    ...params,
    table: params.table ?? null,
  });

export const sqliteExecuteRemote = (params: SshParams & { dbPath: string; sql: string }) =>
  invoke<QueryExecutionResult>("sqlite_execute_remote", params);

export const sqliteFindInDir = (
  params: SshParams & { directory: string; maxDepth?: number | null },
) =>
  invoke<RemoteSqliteCandidate[]>("sqlite_find_in_dir", {
    ...params,
    maxDepth: params.maxDepth ?? null,
  });

/** Last-known shell working directory, if the remote shell has
 *  emitted an OSC 7 sequence (most distros' default bash/zsh
 *  do). Returns null before the first prompt fires. */
export const terminalCurrentCwd = (sessionId: string) =>
  invoke<string | null>("terminal_current_cwd", { sessionId });

// ── Docker Extended ─────────────────────────────────────────────

export const dockerInspect = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  containerId: string;
  savedConnectionIndex?: number | null;
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
  savedConnectionIndex?: number | null;
}) => invoke<void>("docker_remove_image", params);

export const dockerRemoveVolume = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  volumeName: string;
  savedConnectionIndex?: number | null;
}) => invoke<void>("docker_remove_volume", params);

export const dockerRemoveNetwork = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  networkName: string;
  savedConnectionIndex?: number | null;
}) => invoke<void>("docker_remove_network", params);

export type DockerRunOptions = {
  image: string;
  name?: string;
  /** `[hostPort, containerPort]` pairs; blank host port lets docker pick one. */
  ports?: [string, string][];
  /** `[key, value]` pairs. */
  env?: [string, string][];
  /** `[hostPath, containerPath]` pairs. */
  volumes?: [string, string][];
  /** `""` (none), `"always"`, `"on-failure"`, `"unless-stopped"`. */
  restart?: string;
  /** Optional trailing command override. */
  command?: string;
};

export const dockerRunContainer = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  options: DockerRunOptions;
  savedConnectionIndex?: number | null;
}) => invoke<string>("docker_run_container", params);

export const dockerPruneVolumes = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
}) => invoke<string>("docker_prune_volumes", params);

export const dockerPruneImages = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
}) => invoke<string>("docker_prune_images", params);

export const dockerVolumeFiles = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  mountpoint: string;
  savedConnectionIndex?: number | null;
}) => invoke<string>("docker_volume_files", params);

export type DockerContainerStatsView = {
  id: string;
  cpuPerc: string;
  memUsage: string;
  memPerc: string;
};

export type DockerVolumeUsageView = {
  name: string;
  size: string;
  sizeBytes: number;
  links: number;
};

/** Slow `docker stats --no-stream` — run it after the base overview
 *  to keep the first paint snappy. */
export const dockerStats = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
}) => invoke<DockerContainerStatsView[]>("docker_stats", params);

/** Slow `docker system df -v` — see `dockerStats` comment. */
export const dockerVolumeUsage = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
}) => invoke<DockerVolumeUsageView[]>("docker_volume_usage", params);

export const dockerPullImage = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  imageRef: string;
  /** Optional env overrides (e.g. `[["HTTPS_PROXY", "http://..."]]`)
   *  applied only to the pull; does not modify the remote daemon. */
  envPrefix?: [string, string][] | null;
  savedConnectionIndex?: number | null;
}) => invoke<string>("docker_pull_image", params);

export const localDockerPullImage = (
  imageRef: string,
  envPrefix?: [string, string][] | null,
) => invoke<string>("local_docker_pull_image", { imageRef, envPrefix: envPrefix ?? null });

export const localDockerRunContainer = (options: DockerRunOptions) =>
  invoke<string>("local_docker_run_container", { options });

export const localDockerPruneVolumes = () =>
  invoke<string>("local_docker_prune_volumes");

export const localDockerPruneImages = () =>
  invoke<string>("local_docker_prune_images");

export const localDockerVolumeFiles = (mountpoint: string) =>
  invoke<string>("local_docker_volume_files", { mountpoint });

// ── SFTP Extended ───────────────────────────────────────────────

export const sftpMkdir = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  path: string;
  savedConnectionIndex?: number | null;
}) => invoke<void>("sftp_mkdir", params);

export const sftpRemove = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  path: string;
  isDir: boolean;
  savedConnectionIndex?: number | null;
}) => invoke<void>("sftp_remove", params);

export const sftpRename = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  from: string;
  to: string;
  savedConnectionIndex?: number | null;
}) => invoke<void>("sftp_rename", params);

export const sftpChmod = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  path: string;
  mode: number;
  savedConnectionIndex?: number | null;
}) => invoke<void>("sftp_chmod", params);

export const sftpCreateFile = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  path: string;
  savedConnectionIndex?: number | null;
}) => invoke<void>("sftp_create_file", params);

/** Payload returned by {@link sftpReadText} — raw content plus
 *  metadata the editor dialog renders in its status bar. `lossy`
 *  is true when the remote file contained invalid UTF-8 that had
 *  to be replaced with U+FFFD; the UI warns the user before save. */
export type SftpTextFile = {
  path: string;
  content: string;
  size: number;
  permissions: number | null;
  modified: number | null;
  lossy: boolean;
};

export const sftpReadText = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  path: string;
  /** Upper bound checked before streaming. Backend caps this at 5 MB. */
  maxBytes?: number | null;
  savedConnectionIndex?: number | null;
}) => invoke<SftpTextFile>("sftp_read_text", params);

export const sftpWriteText = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  path: string;
  content: string;
  savedConnectionIndex?: number | null;
}) => invoke<void>("sftp_write_text", params);

export const sftpDownload = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  remotePath: string;
  localPath: string;
  savedConnectionIndex?: number | null;
  /** Opaque id matching the `sftp:progress` events back to a frontend
   *  transfer queue entry. Omit to skip events and take the
   *  whole-file fast path on the backend. */
  transferId?: string | null;
}) => invoke<void>("sftp_download", params);

export const sftpUpload = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  localPath: string;
  remotePath: string;
  savedConnectionIndex?: number | null;
  /** Opaque id for matching `sftp:progress` events — see
   *  {@link sftpDownload}. */
  transferId?: string | null;
}) => invoke<void>("sftp_upload", params);

/** Payload shape of the `sftp:progress` event emitted by the
 *  upload/download commands. */
export type SftpProgressEvent = {
  id: string;
  bytes: number;
  total: number;
  done: boolean;
  error: string | null;
};

/** Event name emitted by the Rust side. Re-export so panels can
 *  subscribe without hard-coding the literal. */
export const SFTP_PROGRESS_EVENT = "sftp:progress";

/** Recursively upload a local directory to a remote path. Aggregate
 *  byte progress is emitted under the same `sftp:progress` channel. */
export const sftpUploadTree = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  localPath: string;
  remotePath: string;
  savedConnectionIndex?: number | null;
  transferId?: string | null;
}) => invoke<void>("sftp_upload_tree", params);

/** Recursively download a remote directory to a local path. */
export const sftpDownloadTree = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  remotePath: string;
  localPath: string;
  savedConnectionIndex?: number | null;
  transferId?: string | null;
}) => invoke<void>("sftp_download_tree", params);

// ── Log Stream ──────────────────────────────────────────────────

export const logStreamStart = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  command: string;
  savedConnectionIndex?: number | null;
}) => invoke<string>("log_stream_start", params);

export const logStreamDrain = (streamId: string) =>
  invoke<LogEventView[]>("log_stream_drain", { streamId });

export const logStreamStop = (streamId: string) =>
  invoke<void>("log_stream_stop", { streamId });

// ── Local System ────────────────────────────────────────────────

export const localDockerOverview = (all: boolean) =>
  invoke<DockerOverview>("local_docker_overview", { all });

export const localDockerImages = () =>
  invoke<DockerImageView[]>("local_docker_images");

export const localDockerVolumes = () =>
  invoke<DockerVolumeView[]>("local_docker_volumes");

export const localDockerNetworks = () =>
  invoke<DockerNetworkView[]>("local_docker_networks");

/** Slow `docker stats --no-stream` against the local daemon — split off
 *  from the overview so the panel's first paint doesn't wait ~2s for the
 *  CLI's sampling window. See {@link dockerStats} for the SSH counterpart. */
export const localDockerStats = () =>
  invoke<DockerContainerStatsView[]>("local_docker_stats");

/** Slow `docker system df -v` against the local daemon — split off from
 *  the overview for the same reason as {@link localDockerStats}. */
export const localDockerVolumeUsage = () =>
  invoke<DockerVolumeUsageView[]>("local_docker_volume_usage");

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
