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
  FirewallSnapshotView,
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
  GitUnpushedCommit,
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

/** Major dependency snapshot — surfaced in Settings → About →
 *  Components. Backend hardcodes the list; values are static. */
export type ComponentInfo = { name: string; role: string; version: string };
export const coreComponentsInfo = () =>
  invoke<ComponentInfo[]>("core_components_info");

/** One ~/.ssh/id_* private key (paired with its .pub when present).
 *  Surfaced in Settings → SSH keys. Read-only. */
export type SshKeyInfo = {
  path: string;
  comment: string;
  kind: string;
  mode: string;
  hasPublic: boolean;
};
export const sshKeysList = () => invoke<SshKeyInfo[]>("ssh_keys_list");

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

/** Read a UTF-8 text file from the local filesystem. Capped at 64
 *  MiB by the backend; bigger imports go through engine-native dump
 *  tools (mysqldump / pg_dump). */
export const localReadTextFile = (path: string) =>
  invoke<string>("local_read_text_file", { path });

/** Write a UTF-8 text file to the local filesystem. Creates parent
 *  directories as needed. */
export const localWriteTextFile = (path: string, content: string) =>
  invoke<void>("local_write_text_file", { path, content });

// ── Git ─────────────────────────────────────────────────────────

export const gitOverview = (path?: string) =>
  invoke<GitOverview>("git_overview", { path: path ?? null });

export const gitPanelState = (path?: string | null) =>
  invoke<GitPanelState>("git_panel_state", { path: path ?? null });

export const gitInitRepo = (path?: string | null) =>
  invoke<string>("git_init_repo", { path: path ?? null });

/** Mirrors `git config --global` keys consumed by the Settings → Git
 *  page. Backend whitelists exactly these keys — adding a new field
 *  requires updating both ends. */
export type GitGlobalConfig = {
  userName: string;
  userEmail: string;
  /** init.defaultBranch */
  defaultBranch: string;
  /** gpg.format — "openpgp" | "ssh" | "x509" | "" (off). */
  signingMethod: string;
  /** user.signingkey — path or fingerprint depending on method. */
  signingKey: string;
  /** commit.gpgsign */
  signCommits: boolean;
  /** tag.gpgsign */
  signTags: boolean;
};

export const gitGlobalConfigGet = () =>
  invoke<GitGlobalConfig>("git_global_config_get");

export const gitGlobalConfigSet = (config: GitGlobalConfig) =>
  invoke<void>("git_global_config_set", { config });

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

export type GitCommitOptions = { signoff?: boolean; amend?: boolean; sign?: boolean };

export const gitCommit = (path: string | null, message: string, options?: GitCommitOptions) =>
  invoke<string>("git_commit", {
    path,
    message,
    signoff: options?.signoff ?? false,
    amend: options?.amend ?? false,
    sign: options?.sign ?? false,
  });

export const gitCommitAndPush = (path: string | null, message: string, options?: GitCommitOptions) =>
  invoke<string>("git_commit_and_push", {
    path,
    message,
    signoff: options?.signoff ?? false,
    amend: options?.amend ?? false,
    sign: options?.sign ?? false,
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

export const gitStashReword = (path: string | null, index: string, message: string) =>
  invoke<string>("git_stash_reword", { path, index, message });

export const gitUnpushedCommits = (path: string | null) =>
  invoke<GitUnpushedCommit[]>("git_unpushed_commits", { path });

export const gitRewordUnpushedCommit = (path: string | null, hash: string, message: string) =>
  invoke<string>("git_reword_unpushed_commit", { path, hash, message });

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

export const gitRevertCommit = (path: string | null, hash: string, noCommit?: boolean) =>
  invoke<string>("git_revert_commit", { path, hash, noCommit: noCommit ?? false });

export const gitCherryPickCommit = (path: string | null, hash: string, noCommit?: boolean) =>
  invoke<string>("git_cherry_pick_commit", { path, hash, noCommit: noCommit ?? false });

export type GitReflogEntry = {
  hash: string;
  shortHash: string;
  refName: string;
  subject: string;
  relativeDate: string;
};

export const gitReflogList = (path: string | null, limit?: number) =>
  invoke<GitReflogEntry[]>("git_reflog_list", { path, limit: limit ?? 100 });

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

/** One row of the host-health dashboard. Mirrors
 *  `pier_core::services::host_health::HostHealthReport`. The probe
 *  is TCP-only — `status === "online"` means the SSH port accepted
 *  a TCP handshake, NOT that authentication would succeed. */
export type HostHealthReport = {
  savedConnectionIndex: number;
  status: "online" | "offline" | "timeout" | "error";
  latencyMs: number | null;
  errorMessage: string;
  /** Unix epoch seconds when the probe finished. */
  checkedAt: number;
};

/** Probe each saved connection in `indices` in parallel and return
 *  one report per index in input order. `timeoutMs` is clamped to
 *  [200, 30000] inside the backend. The command itself only errors
 *  when the connection store can't be loaded; per-host failures
 *  surface inside the row. */
export const hostHealthProbe = (params: {
  indices: number[];
  timeoutMs: number;
}) =>
  invoke<HostHealthReport[]>("host_health_probe", {
    indices: params.indices,
    timeoutMs: params.timeoutMs,
  });

/** Result of a host-health deep probe over a CACHED SSH session.
 *  All fields are best-effort — a parser miss leaves the field
 *  null rather than raising an error. */
export type HostDeepProbeReport = {
  savedConnectionIndex: number;
  /** "5 days,  3:42" — human-readable uptime portion. */
  uptime: string | null;
  /** "0.12, 0.34, 0.45" — load average triplet. */
  loadAvg: string | null;
  /** "78%" — root filesystem use percentage. */
  diskRootUse: string | null;
  /** "12G" — root filesystem available space. */
  diskRootAvail: string | null;
  /** "Ubuntu 22.04.4 LTS" — `/etc/os-release` PRETTY_NAME. */
  distro: string | null;
  checkedAt: number;
};

/** Run uptime / disk / distro lookup over the cached SSH session
 *  for `savedConnectionIndex`. Returns `null` when there's no
 *  cached session (the user hasn't opened a panel for the host in
 *  this Pier-X session yet). Never authenticates — the deep
 *  probe is meant to ride on existing connections, not start new
 *  ones. */
export const hostHealthDeepProbe = (savedConnectionIndex: number) =>
  invoke<HostDeepProbeReport | null>("host_health_deep_probe", {
    savedConnectionIndex,
  });

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

// ── Process-level SSH credential cache (Stage 3) ──────────────────
//
// Mirror credentials the terminal-side ssh just successfully used
// into a process-wide cache so right-side panels (firewall, monitor,
// SFTP, Docker, DB tunnels) can reach the same target without
// re-prompting. Empty values are no-ops on the backend.

export const sshCredCachePutPassword = (params: {
  host: string;
  port: number;
  user: string;
  password: string;
}) => invoke<void>("ssh_cred_cache_put_password", params);

export const sshCredCachePutPassphrase = (params: {
  host: string;
  port: number;
  user: string;
  passphrase: string;
}) => invoke<void>("ssh_cred_cache_put_passphrase", params);

export const sshCredCacheForget = (params: {
  host: string;
  port: number;
  user: string;
}) => invoke<void>("ssh_cred_cache_forget", params);

// ── SSH ControlMaster (terminal-side mux) ─────────────────────────

export type SshMuxSettings = {
  enabled: boolean;
  persistSeconds: number;
};

export const sshMuxGetSettings = () =>
  invoke<SshMuxSettings>("ssh_mux_get_settings");

export const sshMuxSetSettings = (params: {
  enabled: boolean;
  persistSeconds: number;
}) => invoke<void>("ssh_mux_set_settings", params);

export const sshMuxForgetTarget = (params: {
  host: string;
  port: number;
  user: string;
}) => invoke<void>("ssh_mux_forget_target", params);

export const sshMuxShutdownAll = () =>
  invoke<number>("ssh_mux_shutdown_all");

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

export const sshTunnelList = () =>
  invoke<TunnelInfoView[]>("ssh_tunnel_list");

export const sshTunnelClose = (tunnelId: string) =>
  invoke<void>("ssh_tunnel_close", { tunnelId });

export type KnownHostEntry = {
  line: number;
  host: string;
  keyType: string;
  fingerprint: string;
  hashed: boolean;
};

export type KnownHostsListResult = {
  path: string | null;
  entries: KnownHostEntry[];
};

export const sshKnownHostsList = () =>
  invoke<KnownHostsListResult>("ssh_known_hosts_list");

export const sshKnownHostsRemove = (line: number) =>
  invoke<void>("ssh_known_hosts_remove", { line });

/**
 * Background pre-warm of the shared SSH session cache for a target.
 *
 * When the terminal detects a nested-ssh target (user typed
 * `ssh user@host`) and we have enough credentials to open our own
 * russh session (saved-connection index, key, agent, or a password
 * captured from the PTY prompt), call this once so the first panel
 * click doesn't pay the full SSH handshake latency. Fire-and-forget:
 * the promise resolves as soon as the backend schedules the work,
 * not when the connection is actually established.
 */
export const sshSessionPrewarm = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
}) =>
  invoke<void>("ssh_session_prewarm", {
    ...params,
    savedConnectionIndex: params.savedConnectionIndex ?? null,
  });

// ── Terminal ────────────────────────────────────────────────────

export const terminalCreate = (
  cols: number,
  rows: number,
  shell?: string,
  smartMode?: boolean,
) =>
  invoke<TerminalSessionInfo>("terminal_create", {
    cols,
    rows,
    shell: shell ?? null,
    smartMode: smartMode ?? false,
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
  /** Row offset for the preview query. Defaults to 0. */
  offset?: number | null;
  /** Page size for the preview query. Backend clamps to [1, 500]; default 24. */
  limit?: number | null;
}) =>
  invoke<MysqlBrowserState>("mysql_browse", {
    ...params,
    offset: params.offset ?? null,
    limit: params.limit ?? null,
  });

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

/** Run a script with multiple `;`-separated statements. Each
 *  statement returns its own [QueryExecutionResult] with
 *  per-statement timing. */
export const sqliteExecuteScript = (path: string, sql: string) =>
  invoke<QueryExecutionResult[]>("sqlite_execute_script", { path, sql });

// ── Redis ───────────────────────────────────────────────────────

export const redisBrowse = (params: {
  host: string;
  port: number;
  db: number;
  pattern: string;
  key?: string | null;
  /** Redis 6+ ACL username. Empty/null = default user. */
  username?: string | null;
  /** AUTH secret. Empty/null = no AUTH. */
  password?: string | null;
  /** SCAN cursor; "0" or null for the first page. */
  cursor?: string | null;
  /** Page size; backend caps at 500. */
  limit?: number | null;
}) =>
  invoke<RedisBrowserState>("redis_browse", {
    ...params,
    username: params.username ?? null,
    password: params.password ?? null,
    cursor: params.cursor ?? null,
    limit: params.limit ?? null,
  });

export const redisExecute = (params: {
  host: string;
  port: number;
  db: number;
  command: string;
  username?: string | null;
  password?: string | null;
}) =>
  invoke<RedisCommandResult>("redis_execute", {
    ...params,
    username: params.username ?? null,
    password: params.password ?? null,
  });

/** Confirm-guarded RENAME via `RENAMENX`. Resolves `false` when
 *  the destination already exists; the caller surfaces that as
 *  an error. */
export const redisRenameKey = (params: {
  host: string;
  port: number;
  db: number;
  from: string;
  to: string;
  username?: string | null;
  password?: string | null;
}) =>
  invoke<boolean>("redis_rename_key", {
    ...params,
    username: params.username ?? null,
    password: params.password ?? null,
  });

/** Confirm-guarded DEL. Resolves `true` when the key existed,
 *  `false` when it didn't (so the panel can distinguish
 *  "deleted" from "no-op"). */
export const redisDeleteKey = (params: {
  host: string;
  port: number;
  db: number;
  key: string;
  username?: string | null;
  password?: string | null;
}) =>
  invoke<boolean>("redis_delete_key", {
    ...params,
    username: params.username ?? null,
    password: params.password ?? null,
  });

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
  /** `true` for the slow tier — collects `df` + `lsblk` alongside
   *  CPU/memory/network. `false` for the fast 5 s tier — skips disk
   *  segments so we don't burn SSH/remote CPU re-reading data that
   *  barely moves. The panel keeps the prior full snapshot's disks
   *  visible in between full polls. */
  includeDisks: boolean;
}) => invoke<ServerSnapshotView>("server_monitor_probe", params);

// ── Firewall ────────────────────────────────────────────────────

export const firewallSnapshot = (params: {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
}) => invoke<FirewallSnapshotView>("firewall_snapshot", params);

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

export type SshParams = {
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

/** Outcome class returned by `sqliteInstallRemote`. Keep in sync with
 *  `RemoteSqliteInstallStatus` in `pier-core/src/services/sqlite_remote.rs`. */
export type RemoteSqliteInstallStatus =
  | "installed"
  | "unsupported-distro"
  | "sudo-requires-password"
  | "package-manager-failed";

export type RemoteSqliteInstallReport = {
  status: RemoteSqliteInstallStatus;
  distroId: string;
  packageManager: string;
  command: string;
  exitCode: number;
  outputTail: string;
  installedVersion: string | null;
};

export const sqliteInstallRemote = (params: SshParams) =>
  invoke<RemoteSqliteInstallReport>("sqlite_install_remote", params);

// ── Software panel ─────────────────────────────────────────────

/** Static info about a v2 vendor-supplied installer (e.g. Docker's
 *  `https://get.docker.com`). The URL set is sealed in the backend
 *  registry — the frontend never passes a URL into the install
 *  command. */
export type VendorScriptDescriptor = {
  /** Short label rendered in the install dropdown. */
  label: string;
  /** Fully-qualified `https://` URL. Read-only — surfaced to the user
   *  in the confirmation dialog so they can verify it before opting in. */
  url: string;
  /** Risk-disclosure text shown in the confirmation dialog. */
  notes: string;
  /** When `true`, installing via this script will conflict with the
   *  distro-package version of the same software (e.g. `docker.io`
   *  vs. upstream `docker-ce`). The dialog warns the user to uninstall
   *  the distro package first. */
  conflictsWithApt: boolean;
  /** `true` when the descriptor declares cleanup snippets. The
   *  uninstall dialog uses this to decide whether to show the
   *  "remove upstream source" checkbox. */
  hasCleanupScripts: boolean;
};

/** Major-version variant on a multi-version descriptor (e.g. OpenJDK
 *  8/11/17/21 on Java). Empty list = single-version software. */
export type SoftwareVersionVariant = {
  /** Stable id passed back to the install command as `variantKey`. */
  key: string;
  /** Human label rendered in the variant dropdown. */
  label: string;
};

/** One row in the software registry. Order is the rendering order. */
export type SoftwareDescriptor = {
  id: string;
  displayName: string;
  notes: string | null;
  hasService: boolean;
  /** Filesystem dirs declared as user data on the descriptor. The
   *  uninstall dialog renders them inside the "also delete data
   *  directories" warning. Empty for stateless software (jq, curl, …). */
  dataDirs: string[];
  /** `true` when the daemon supports `systemctl reload` without a
   *  downtime restart (currently only nginx). Drives whether the
   *  service menu shows a "Reload (no downtime)" entry. */
  supportsReload: boolean;
  /** Non-null when the descriptor exposes a v2 vendor-script install
   *  path. The panel renders the install button as a split-button
   *  (default = apt path, dropdown = "通过 {label}") in this case. */
  vendorScript: VendorScriptDescriptor | null;
  /** Major-version variants. Non-empty list switches the install
   *  flow to "pick a variant first, then install" (e.g. Java →
   *  OpenJDK 8/11/17/21). Empty for single-version software. */
  versionVariants: SoftwareVersionVariant[];
  /** Common config files declared on the descriptor. The details
   *  pane filters these through `test -e` before display. */
  configPaths: string[];
  /** Default ports the software listens on with stock config. */
  defaultPorts: number[];
  /** App-store category id (`database` / `web` / `runtime` / …).
   *  Empty string = "其它" / Other. The panel maps these onto
   *  localized section titles. */
  category: string;
};

export type HostPackageEnv = {
  distroId: string;
  distroPretty: string;
  /** `null` when the host's distro isn't in the supported list — the
   *  install column should be disabled with a helpful message. */
  packageManager: string | null;
  isRoot: boolean;
};

export type SoftwarePackageStatus = {
  id: string;
  installed: boolean;
  version: string | null;
  /** `null` when the descriptor has no service to manage. */
  serviceActive: boolean | null;
};

export type SoftwareProbeResult = {
  env: HostPackageEnv;
  statuses: SoftwarePackageStatus[];
};

/** Outcome class returned by `softwareInstallRemote` /
 *  `softwareUpdateRemote`. Superset of `RemoteSqliteInstallStatus` —
 *  v2 adds Cancelled (cancel button) and the two vendor-script-only
 *  outcomes (vendor channel). */
export type SoftwareInstallStatus =
  | "installed"
  | "unsupported-distro"
  | "sudo-requires-password"
  | "package-manager-failed"
  /** The user clicked Cancel mid-run; the report's `exitCode` is the
   *  pier-core sentinel `-2`. Remote process state is unspecified —
   *  see PRODUCT-SPEC §5.11 v2. */
  | "cancelled"
  | "vendor-script-download-failed"
  | "vendor-script-failed";

/** Echo-back of the vendor script that produced an install — present
 *  on `SoftwareInstallReport` only when the install ran via the v2
 *  channel. The frontend appends `via {label} ({url})` to the
 *  activity log when this is non-null. */
export type VendorScriptUsed = {
  label: string;
  url: string;
};

export type SoftwareInstallReport = {
  packageId: string;
  status: SoftwareInstallStatus;
  distroId: string;
  packageManager: string;
  command: string;
  exitCode: number;
  outputTail: string;
  installedVersion: string | null;
  serviceActive: boolean | null;
  /** Non-null iff the install ran via the v2 vendor-script channel. */
  vendorScript: VendorScriptUsed | null;
  /** Stale / unreachable third-party repos (Docker focal pulled, PPA
   *  dormant, internal mirror moved, …) detected in the install
   *  output. The backend now decouples `apt-get update` from
   *  `apt-get install` (and passes `--setopt=skip_if_unavailable=True`
   *  to dnf/yum) so a single broken repo no longer fails the whole
   *  install — but the user should still be told *which* repo to
   *  clean up. Each entry is `"<manager>: <url-or-id>"`.
   *  Omitted on the wire when empty (legacy clients keep parsing). */
  repoWarnings?: string[];
};

/** Streaming event payload for `software-install`. The frontend filters
 *  by `installId` so concurrent installs across hosts don't interleave.
 *
 *  `cancelled` may carry a final report (when pier-core observed the
 *  cancel inside `exec_command_streaming`) or no report (when
 *  `software_install_cancel` fanned out the signal before the task
 *  finished unwinding). The frontend treats both as terminal. */
export type SoftwareInstallEvent =
  | { installId: string; kind: "line"; text: string }
  | { installId: string; kind: "done"; report: SoftwareInstallReport }
  | { installId: string; kind: "failed"; message: string }
  | { installId: string; kind: "cancelled"; report?: SoftwareInstallReport };

export const softwareRegistry = () => invoke<SoftwareDescriptor[]>("software_registry");

export const softwareProbeRemote = (params: SshParams) =>
  invoke<SoftwareProbeResult>("software_probe_remote", params);

export const softwareInstallRemote = (
  params: SshParams & {
    packageId: string;
    installId: string;
    enableService: boolean;
    /** Pin to a specific package-manager version. `undefined` =
     *  install whatever the manager picks (the registry's default). */
    version?: string | null;
    /** Pick a major-version variant for descriptors that declare any
     *  (e.g. `"openjdk-21"` for Java). `undefined` / `null` = the
     *  descriptor's default install_packages. */
    variantKey?: string | null;
    /** v2: when `true`, route through the descriptor's
     *  `vendorScript` channel (curl + run the official installer)
     *  instead of the default apt / dnf / … path. The panel only
     *  sets this after the user clicks the dropdown's "通过 {label}"
     *  entry AND confirms in the risk dialog. Omit / `false` =
     *  default package-manager path. */
    viaVendorScript?: boolean;
  },
) =>
  invoke<SoftwareInstallReport>("software_install_remote", {
    ...params,
    version: params.version ?? null,
    variantKey: params.variantKey ?? null,
  });

export const softwareUpdateRemote = (
  params: SshParams & {
    packageId: string;
    installId: string;
    enableService: boolean;
    /** Pin to a specific package-manager version. `undefined` =
     *  upgrade to whatever the manager has as latest. */
    version?: string | null;
    /** See `softwareInstallRemote.variantKey`. */
    variantKey?: string | null;
  },
) =>
  invoke<SoftwareInstallReport>("software_update_remote", {
    ...params,
    version: params.version ?? null,
    variantKey: params.variantKey ?? null,
  });

/** Enumerate package-manager-visible versions for `packageId` on the
 *  remote host, freshest first. Empty array on unsupported distros and
 *  on pacman (Arch repos don't carry historical versions). The
 *  software panel caches the result for 5 min per host+package.
 *
 *  Pass `variantKey` to query the variant's package list (e.g. asking
 *  for OpenJDK 21's apt versions instead of the descriptor's default). */
export const softwareVersionsRemote = (
  params: SshParams & { packageId: string; variantKey?: string | null },
) =>
  invoke<string[]>("software_versions_remote", {
    ...params,
    variantKey: params.variantKey ?? null,
  });

/** Per-row "expand" details. Loaded lazily — the panel calls this only
 *  when the user clicks the disclosure on a row, so the slow candidate-
 *  version + ss probes never block the panel's first paint. */
export type SoftwarePackageDetail = {
  packageId: string;
  installed: boolean;
  installPaths: string[];
  configPaths: string[];
  defaultPorts: number[];
  listeningPorts: number[];
  /** `false` when the `ss -ltn` probe failed (host has no `ss`, etc.).
   *  In that case `listeningPorts` is unreliable and the UI hides the
   *  "live ports" line. */
  listenProbeOk: boolean;
  serviceUnit: string | null;
  /** Candidate version from the package manager's metadata cache
   *  (apt-cache policy / dnf info / …). `null` on unsupported distro
   *  or when the query produced nothing parseable. */
  latestVersion: string | null;
  installedVersion: string | null;
  variants: SoftwarePackageVariantStatus[];
};

export type SoftwarePackageVariantStatus = {
  key: string;
  label: string;
  installed: boolean;
  installedVersion: string | null;
};

export const softwareDetailsRemote = (
  params: SshParams & { packageId: string },
) => invoke<SoftwarePackageDetail>("software_details_remote", params);

/** Synthesise the install command without running it. The row's
 *  "复制安装命令" menu entry uses this so users who'd rather paste
 *  into their own SSH session can audit + run the command manually. */
export type InstallCommandPreview = {
  packageId: string;
  packageManager: string;
  isRoot: boolean;
  /** Just the package-manager command, no sudo wrapper. */
  innerCommand: string;
  /** Full `sudo -n sh -c '...' 2>&1` form pier-core would have run. */
  wrappedCommand: string;
};

export const softwareInstallPreview = (
  params: SshParams & {
    packageId: string;
    version?: string | null;
    variantKey?: string | null;
    isUpdate?: boolean;
  },
) =>
  invoke<InstallCommandPreview>("software_install_preview", {
    ...params,
    version: params.version ?? null,
    variantKey: params.variantKey ?? null,
    isUpdate: params.isUpdate ?? false,
  });

/** Curated software bundle (e.g. "DevOps 基础"). The panel renders
 *  these as one-click cards; clicking opens a confirm dialog and
 *  installs the listed packages sequentially. */
export type SoftwareBundle = {
  id: string;
  displayName: string;
  description: string;
  packageIds: string[];
};

export const softwareBundles = () =>
  invoke<SoftwareBundle[]>("software_bundles");

/** Look up curated "X is commonly installed alongside Y" suggestions
 *  for `id`. Empty list = no curated recommendations. */
export const softwareCoInstallSuggestions = (id: string) =>
  invoke<string[]>("software_co_install_suggestions", { id });

/** Topologically sort `ids` so co-install anchors come before their
 *  companions. Pure-CPU lookup — no SSH. Used by `runBundle` to
 *  reorder a manually-curated bundle into "install docker before
 *  compose" order regardless of how the user wrote the JSON. */
export const softwareBundleInstallOrder = (ids: string[]) =>
  invoke<string[]>("software_bundle_install_order", { ids });

// ── Post-install webhooks (v2.14) ───────────────────────────────

export type WebhookEventKindLabel = "install" | "update" | "uninstall";

export type WebhookEntry = {
  url: string;
  label: string;
  /** Subset of `["install","update","uninstall"]`. Empty = fire on all. */
  events: WebhookEventKindLabel[];
  disabled: boolean;
  /** Optional body template — when non-empty the rendered string
   *  is sent verbatim as the request body. Placeholders use
   *  `{{name}}` syntax: `event`, `status`, `packageId`, `host`,
   *  `packageManager`, `version`, `firedAt`, `text`. Empty falls
   *  back to the default Slack-shaped payload. */
  bodyTemplate?: string;
  /** Retry attempts after the first failure (0–5, capped backend-
   *  side). 0 = one shot. Failures that exhaust retries land in
   *  the persistent failure log. */
  maxRetries?: number;
  /** Base seconds for exponential backoff between retries. 0 =
   *  use the backend default (5s, doubling each attempt). */
  retryBackoffSecs?: number;
};

export type WebhookConfig = {
  entries: WebhookEntry[];
};

export type WebhookFireReport = {
  url: string;
  /** 0 when the request never completed (DNS / TLS / connect fail). */
  statusCode: number;
  latencyMs: number;
  /** Empty on success; failure message otherwise. */
  error: string;
  /** Total attempts that ran (1 = first attempt only, 2+ = retries). */
  attempts: number;
};

/** One row of the persistent webhook failure log. Returned newest-
 *  first by `softwareWebhooksFailuresList`. */
export type WebhookFailureRecord = {
  id: string;
  url: string;
  label: string;
  statusCode: number;
  error: string;
  attempts: number;
  /** Body that was sent on the last attempt — replayable verbatim. */
  body: string;
  event: string;
  packageId: string;
  host: string;
  failedAt: number;
};

export const softwareWebhooksLoad = () =>
  invoke<WebhookConfig>("software_webhooks_load");

export const softwareWebhooksSave = (config: WebhookConfig) =>
  invoke<void>("software_webhooks_save", { config });

export const softwareWebhooksTestFire = (params: {
  url: string;
  bodyTemplate?: string;
}) =>
  invoke<WebhookFireReport>("software_webhooks_test_fire", {
    url: params.url,
    bodyTemplate: params.bodyTemplate ?? null,
  });

/** Render a body template against a synthetic install payload —
 *  used by the settings dialog's preview pane so users can verify
 *  their template's wire shape without firing an actual HTTP
 *  request. Pure-CPU on the backend. */
export const softwareWebhooksPreviewBody = (bodyTemplate: string) =>
  invoke<string>("software_webhooks_preview_body", { bodyTemplate });

export const softwareWebhooksPath = () =>
  invoke<string | null>("software_webhooks_path");

/** Read the persistent failure log. Newest entries first. */
export const softwareWebhooksFailuresList = () =>
  invoke<WebhookFailureRecord[]>("software_webhooks_failures_list");

/** Drop one record by id. Returns `true` when the record existed.
 *  Idempotent — a missing id is reported as `false`, never errors. */
export const softwareWebhooksFailuresDismiss = (id: string) =>
  invoke<boolean>("software_webhooks_failures_dismiss", { id });

/** Wipe the entire log file. */
export const softwareWebhooksFailuresClear = () =>
  invoke<void>("software_webhooks_failures_clear");

/** Single-shot replay of a failed fire. The body is sent verbatim,
 *  no retry loop — the user already saw the original chain fail
 *  and will dismiss the record manually if this attempt
 *  succeeds. */
export const softwareWebhooksReplay = (params: {
  url: string;
  body: string;
}) =>
  invoke<WebhookFireReport>("software_webhooks_replay", {
    url: params.url,
    body: params.body,
  });

/** One row of the batch-replay result. `id` echoes the original
 *  failure record id so the UI can mark only the rows whose
 *  replay succeeded — those are auto-dismissed on the backend
 *  side too. */
export type WebhookBatchReplayRow = {
  id: string;
  url: string;
  statusCode: number;
  latencyMs: number;
  error: string;
};

/** Replay the N most-recent failures sequentially. `limit` is
 *  clamped to [1, 50] backend-side. Returns one row per replay
 *  attempt, in newest-first order. Successful replays are
 *  auto-dismissed from the persistent log. */
export const softwareWebhooksReplayBatch = (limit: number) =>
  invoke<WebhookBatchReplayRow[]>("software_webhooks_replay_batch", {
    limit,
  });

/** One row in a system-package search result. */
export type SoftwareSearchHit = {
  name: string;
  summary: string;
};

/** Search the host's package manager catalog. Returns up to
 *  `limit` hits parsed from `apt-cache search` / `dnf search` /
 *  `pacman -Ss` / `apk search -d` / `zypper search`. */
export const softwareSearchRemote = (
  params: SshParams & { query: string; limit?: number },
) => invoke<SoftwareSearchHit[]>("software_search_remote", params);

/** Install a package by name (no descriptor lookup). Streams to
 *  the SOFTWARE_INSTALL_EVENT channel so the existing event
 *  listener wiring works unchanged. */
export const softwareInstallArbitrary = (
  params: SshParams & { packageName: string; installId: string },
) => invoke<SoftwareInstallReport>("software_install_arbitrary", params);

// ── Mirror switching (v2.3) ─────────────────────────────────────

/** Stable id of one curated mirror — must match the backend enum. */
export type MirrorId = "aliyun" | "tsinghua" | "ustc" | "huawei" | "tencent";

export type MirrorChoice = {
  id: MirrorId;
  label: string;
  /** Hostname used to rewrite Debian/Ubuntu apt sources. */
  aptHost: string;
  /** Hostname used to rewrite RHEL-family dnf repo files. */
  dnfHost: string;
  /** Hostname used to rewrite Alpine `/etc/apk/repositories`. */
  apkHost: string | null;
  /** Full URL prefix for pacman `Server = ...` lines. */
  pacmanUrl: string | null;
  /** Hostname used to rewrite openSUSE `/etc/zypp/repos.d/*.repo`. */
  zypperHost: string | null;
};

export type MirrorState = {
  /** Which manager the state applies to (`apt` / `dnf` / empty
   *  when the host has no detected manager). */
  packageManager: string;
  /** Curated id when the detected hostname matches one of the
   *  catalog entries. `null` for "unknown / official upstream". */
  currentId: MirrorId | null;
  /** First hostname found in the sources file. */
  currentHost: string | null;
  /** `true` when a `.pier-bak` companion exists on the host. The
   *  panel uses this to gate the "restore" button. */
  hasBackup: boolean;
};

export type MirrorActionStatus =
  | "ok"
  | "sudo-requires-password"
  | "failed"
  | "unsupported-manager";

export type MirrorActionReport = {
  status: MirrorActionStatus;
  packageManager: string;
  command: string;
  exitCode: number;
  outputTail: string;
  /** Re-detected mirror state after the action — the panel uses
   *  this to update the badge without a second round-trip. */
  stateAfter: MirrorState;
};

export const softwareMirrorCatalog = () =>
  invoke<MirrorChoice[]>("software_mirror_catalog");

/** Filesystem path of the user-extras JSON. `null` when src-tauri
 *  hasn't initialised one (e.g. the OS rejected app_config_dir
 *  resolution). The UI surfaces this in the registry's footer so
 *  users know where to drop custom entries. */
export const softwareUserExtrasPath = () =>
  invoke<string | null>("software_user_extras_path");

/** Read the raw user-extras JSON contents. Empty string when the
 *  file doesn't exist yet. */
export const softwareUserExtrasRead = () =>
  invoke<string>("software_user_extras_read");

/** Write the user-extras file. Validates that the input parses
 *  as JSON before overwriting; an empty string deletes the file.
 *  **Caller surfaces the restart notice** — the running process
 *  keeps the catalog it built at startup. */
export const softwareUserExtrasWrite = (content: string) =>
  invoke<void>("software_user_extras_write", { content });

/** Persistent software-panel preferences. Stored in the app config
 *  dir as `software-prefs.json`. */
export type SoftwarePreferences = {
  preferredMirrorId: MirrorId | null;
};

export const softwarePreferencesGet = () =>
  invoke<SoftwarePreferences>("software_preferences_get");

export const softwarePreferencesSetMirror = (mirrorId: MirrorId | null) =>
  invoke<SoftwarePreferences>("software_preferences_set_mirror", { mirrorId });

/** One row in the software-action history JSONL journal. Written
 *  by helpers around install / uninstall / mirror-set actions; read
 *  by the panel's history dialog. */
export type SoftwareHistoryEntry = {
  ts: number;
  action: string;
  target: string;
  host: string;
  outcome: string;
  note: string;
  /** Saved-connection index when the original logger had one in
   *  scope. The history dialog uses this to re-resolve credentials
   *  for "undo" — when null the undo button stays disabled. */
  savedConnectionIndex: number | null;
};

export const softwareHistoryLog = (params: {
  action: string;
  target: string;
  host: string;
  outcome: string;
  note?: string;
  savedConnectionIndex?: number | null;
}) =>
  invoke<void>("software_history_log", {
    action: params.action,
    target: params.target,
    host: params.host,
    outcome: params.outcome,
    note: params.note ?? "",
    savedConnectionIndex: params.savedConnectionIndex ?? null,
  });

export const softwareHistoryList = (params: {
  sinceTs?: number;
  limit?: number;
} = {}) => invoke<SoftwareHistoryEntry[]>("software_history_list", params);

export const softwareHistoryClear = () =>
  invoke<void>("software_history_clear");

/** Outcome of a PostgreSQL helper action (create user / create
 *  db / open remote). Mirrors the install-report shape so the
 *  panel can reuse the same outcome formatter. */
export type PostgresActionReport = {
  status: "ok" | "sudo-requires-password" | "failed";
  command: string;
  exitCode: number;
  outputTail: string;
};

export const postgresCreateUserRemote = (
  params: SshParams & {
    pgUsername: string;
    pgPassword: string;
    isSuperuser: boolean;
  },
) =>
  invoke<PostgresActionReport>("postgres_create_user_remote", params);

export const postgresCreateDbRemote = (
  params: SshParams & { dbName: string; owner: string },
) => invoke<PostgresActionReport>("postgres_create_db_remote", params);

export const postgresOpenRemote = (params: SshParams) =>
  invoke<PostgresActionReport>("postgres_open_remote_remote", params);

// ── MySQL / MariaDB helpers (v2.9) ──────────────────────────────

export const mysqlCreateUserRemote = (
  params: SshParams & {
    dbUsername: string;
    dbPassword: string;
    dbName: string;
    rootPassword?: string | null;
  },
) =>
  invoke<PostgresActionReport>("mysql_create_user_remote", {
    ...params,
    rootPassword: params.rootPassword ?? null,
  });

export const mysqlCreateDbRemote = (
  params: SshParams & { dbName: string; rootPassword?: string | null },
) =>
  invoke<PostgresActionReport>("mysql_create_db_remote", {
    ...params,
    rootPassword: params.rootPassword ?? null,
  });

export const mysqlOpenRemote = (params: SshParams) =>
  invoke<PostgresActionReport>("mysql_open_remote_remote", params);

// ── Redis helpers (v2.9) ────────────────────────────────────────

export const redisSetPasswordRemote = (
  params: SshParams & { redisPassword: string },
) => invoke<PostgresActionReport>("redis_set_password_remote", params);

export const redisOpenRemote = (params: SshParams) =>
  invoke<PostgresActionReport>("redis_open_remote_remote", params);

// ── Docker Compose templates (v2.11) ────────────────────────────

export type ComposeTemplate = {
  id: string;
  displayName: string;
  description: string;
  yaml: string;
  publishedPorts: number[];
};

export const softwareComposeTemplates = () =>
  invoke<ComposeTemplate[]>("software_compose_templates");

export const softwareComposeApply = (
  params: SshParams & { templateId: string },
) => invoke<PostgresActionReport>("software_compose_apply", params);

export const softwareComposeDown = (
  params: SshParams & { templateId: string },
) => invoke<PostgresActionReport>("software_compose_down", params);

/** Result of converting a Compose template to a multi-document
 *  Kubernetes manifest. The conversion runs entirely client-side
 *  (no SSH) — covers Deployments, Services and PersistentVolume
 *  Claims for the templates we ship. Anything Compose-specific
 *  that doesn't translate (bind mounts, depends_on, healthchecks)
 *  is flagged in `warnings` and inline `# NOTE:` comments. */
export type ComposeK8sExport = {
  /** The original Compose YAML, echoed for the dialog's source pane. */
  composeYaml: string;
  /** Multi-document YAML with `---` separators — paste into
   *  `kubectl apply -f -`. */
  k8sYaml: string;
  deploymentCount: number;
  serviceCount: number;
  pvcCount: number;
  /** 0 or 1 — the converter emits at most one combined Ingress
   *  per template. */
  ingressCount: number;
  /** ConfigMap resources emitted — both from lifted bind mounts
   *  (when `liftBindMounts` was on) AND from top-level Compose
   *  `configs:` declarations. */
  configmapCount: number;
  /** Secret resources emitted from top-level Compose `secrets:`
   *  declarations. */
  secretCount: number;
  /** NetworkPolicy resources emitted from top-level Compose
   *  `networks:` declarations. */
  networkpolicyCount: number;
  /** Items the converter flagged for manual attention. Each entry
   *  is a short human sentence (e.g. "bind mount `./www:/foo`
   *  was dropped"). */
  warnings: string[];
};

/** Convert one Compose template into a Kubernetes manifest. The
 *  command requires no SSH context — pass an optional `namespace`
 *  to target a specific cluster namespace, or omit for the
 *  cluster default.
 *
 *  Ingress: when `ingressHost` is non-empty, an Ingress resource
 *  is emitted that routes `host:` traffic to each HTTP-ish service
 *  via path prefixes. Non-HTTP services (Postgres / Redis) are
 *  skipped automatically. `ingressClass` and `ingressTlsSecret`
 *  are optional refinements. */
export const softwareComposeExportK8s = (params: {
  templateId: string;
  namespace?: string;
  ingressHost?: string;
  ingressClass?: string;
  ingressTlsSecret?: string;
  /** When true, Compose bind mounts (`./local:/in/container`) are
   *  lifted into placeholder `ConfigMap` resources instead of
   *  being dropped with a `# NOTE:` warning. The user is expected
   *  to populate the ConfigMap data with real file content via
   *  `kubectl create configmap … --from-file=` before applying. */
  liftBindMounts?: boolean;
}) =>
  invoke<ComposeK8sExport>("software_compose_export_k8s", {
    templateId: params.templateId,
    namespace: params.namespace ?? null,
    ingressHost: params.ingressHost ?? null,
    ingressClass: params.ingressClass ?? null,
    ingressTlsSecret: params.ingressTlsSecret ?? null,
    liftBindMounts: params.liftBindMounts ?? null,
  });

// ── Cross-host clone (v2.12) ────────────────────────────────────

export type ClonePlanEntry = {
  package: string;
  descriptorId: string | null;
};

export type ClonePlan = {
  packageManager: string;
  entries: ClonePlanEntry[];
};

export const softwareClonePlan = (params: SshParams) =>
  invoke<ClonePlan>("software_clone_plan", params);

// ── DB metrics polling (v2.13) ──────────────────────────────────

export type DbMetrics = {
  kind: string;
  connections: number | null;
  memoryMib: number | null;
  extra: string | null;
  probeOk: boolean;
};

export const softwareDbMetrics = (
  params: SshParams & { packageId: string; rootPassword?: string | null },
) =>
  invoke<DbMetrics>("software_db_metrics", {
    ...params,
    rootPassword: params.rootPassword ?? null,
  });

export const softwareMirrorGet = (params: SshParams) =>
  invoke<MirrorState>("software_mirror_get", params);

export const softwareMirrorSet = (
  params: SshParams & { mirrorId: MirrorId },
) => invoke<MirrorActionReport>("software_mirror_set", params);

export const softwareMirrorRestore = (params: SshParams) =>
  invoke<MirrorActionReport>("software_mirror_restore", params);

/** Per-mirror probe result. `latencyMs = null` = the host couldn't
 *  reach this mirror (DNS fail / timeout / non-2xx HTTP HEAD). */
export type MirrorLatency = {
  mirrorId: MirrorId;
  host: string;
  latencyMs: number | null;
};

export const softwareMirrorBenchmark = (params: SshParams) =>
  invoke<MirrorLatency[]>("software_mirror_benchmark", params);

/** Client-side TCP probe — runs from the local Pier-X process,
 *  not over SSH. Useful when the remote host is unreachable and
 *  the user wants a fallback recommendation based on their own
 *  network's view of the mirror set. */
export const softwareMirrorBenchmarkClient = () =>
  invoke<MirrorLatency[]>("software_mirror_benchmark_client");

/** Subscribe to streaming install/update output. Returns the unlisten
 *  fn — call it on unmount. The handler is invoked with the typed
 *  payload pre-filtered to a single `installId` (so callers don't have
 *  to write the same `if (e.installId === ...)` check everywhere). */
export async function subscribeSoftwareInstall(
  installId: string,
  onEvent: (evt: SoftwareInstallEvent) => void,
): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  const unlisten = await listen<SoftwareInstallEvent>("software-install", (e) => {
    if (e.payload.installId === installId) onEvent(e.payload);
  });
  return unlisten;
}

// ── Software panel — uninstall (v1.1) ──────────────────────────

export type SoftwareUninstallStatus =
  | "uninstalled"
  | "unsupported-distro"
  | "sudo-requires-password"
  | "package-manager-failed"
  | "not-installed"
  /** Same semantics as the install side's `cancelled` — see
   *  {@link SoftwareInstallStatus}. */
  | "cancelled";

export type SoftwareUninstallReport = {
  packageId: string;
  status: SoftwareUninstallStatus;
  distroId: string;
  packageManager: string;
  command: string;
  exitCode: number;
  outputTail: string;
  /** True iff the descriptor has data_dirs, the user opted in, and
   *  the package manager succeeded — i.e. `rm -rf` actually ran. */
  dataDirsRemoved: boolean;
};

/** Streaming event payload for `software-uninstall`. Same `installId`
 *  filtering pattern as the install side; the report shape differs
 *  (no version / serviceActive on the uninstall side, dataDirsRemoved
 *  instead). `cancelled` is dispatched in the same two flavours as
 *  {@link SoftwareInstallEvent}. */
export type SoftwareUninstallEvent =
  | { installId: string; kind: "line"; text: string }
  | { installId: string; kind: "done"; report: SoftwareUninstallReport }
  | { installId: string; kind: "failed"; message: string }
  | { installId: string; kind: "cancelled"; report?: SoftwareUninstallReport };

/** Options carried from the uninstall dialog into the backend. */
export type UninstallOptions = {
  purgeConfig: boolean;
  autoremove: boolean;
  removeDataDirs: boolean;
  /** Run the descriptor's `vendor_script.cleanup_scripts` snippet
   *  after the uninstall succeeds — drops upstream-source files
   *  (e.g. `/etc/apt/sources.list.d/pgdg.list`). No-op when the
   *  descriptor has no cleanup snippet for the host's manager. */
  removeUpstreamSource: boolean;
};

export const softwareUninstallRemote = (
  params: SshParams & {
    packageId: string;
    installId: string;
    options: UninstallOptions;
  },
) => invoke<SoftwareUninstallReport>("software_uninstall_remote", params);

/** Trigger cancellation for an in-flight install / update / uninstall.
 *  The backend keys on the same `installId` the row generated when it
 *  started the activity. Resolves OK even when nothing is running for
 *  that id — the spawn task may have completed in the gap between the
 *  user clicking Cancel and the IPC roundtrip. */
export const softwareInstallCancel = (installId: string) =>
  invoke<void>("software_install_cancel", { installId });

/** Subscribe to streaming uninstall output. Mirrors
 *  {@link subscribeSoftwareInstall} but on the `software-uninstall`
 *  channel. */
export async function subscribeSoftwareUninstall(
  installId: string,
  onEvent: (evt: SoftwareUninstallEvent) => void,
): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  const unlisten = await listen<SoftwareUninstallEvent>(
    "software-uninstall",
    (e) => {
      if (e.payload.installId === installId) onEvent(e.payload);
    },
  );
  return unlisten;
}

// ── Software panel — service control (v2) ──────────────────────

/** Verbs surfaced in each row's service menu. Map 1:1 onto
 *  `systemctl <verb> <unit>`. */
export type SoftwareServiceAction = "start" | "stop" | "restart" | "reload";

/** Outcome class for a service action — mirrors the install outcome
 *  shape so the panel can reuse a single formatter. `failed` covers
 *  both "systemctl exited non-zero" and "systemctl exited 0 but the
 *  re-probe disagrees with the requested verb". */
export type SoftwareServiceActionStatus =
  | "ok"
  | "sudo-requires-password"
  | "failed";

export type SoftwareServiceActionReport = {
  packageId: string;
  status: SoftwareServiceActionStatus;
  /** Verb that was attempted — echoes the request. */
  action: SoftwareServiceAction;
  /** Service unit name that was driven (e.g. `redis-server` on apt,
   *  `redis` on dnf). Empty when the descriptor has no service unit. */
  unit: string;
  command: string;
  exitCode: number;
  outputTail: string;
  /** Post-action `systemctl is-active` ground truth. The panel uses
   *  this to flip the row's service-active dot without a full re-probe. */
  serviceActiveAfter: boolean;
};

/** Streaming event payload for `software-service-action`. Same
 *  filter-by-installId pattern as the install / uninstall channels;
 *  the `done` payload's `report` shape is service-action-specific. */
export type SoftwareServiceActionEvent =
  | { installId: string; kind: "line"; text: string }
  | { installId: string; kind: "done"; report: SoftwareServiceActionReport }
  | { installId: string; kind: "failed"; message: string };

export const softwareServiceActionRemote = (
  params: SshParams & {
    packageId: string;
    installId: string;
    action: SoftwareServiceAction;
  },
) =>
  invoke<SoftwareServiceActionReport>("software_service_action_remote", params);

/** One-shot fetch of the most recent N lines from `journalctl -u <unit>`.
 *  Backing the "View logs" dialog — true tailing is intentionally out
 *  of scope (the existing Log panel handles streaming). */
export const softwareServiceLogsRemote = (
  params: SshParams & { packageId: string; lines: number },
) => invoke<string[]>("software_service_logs_remote", params);

/** Subscribe to streaming service-action output. Returns the unlisten
 *  fn — call it on unmount. Mirrors {@link subscribeSoftwareInstall}. */
export async function subscribeSoftwareServiceAction(
  installId: string,
  onEvent: (evt: SoftwareServiceActionEvent) => void,
): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  const unlisten = await listen<SoftwareServiceActionEvent>(
    "software-service-action",
    (e) => {
      if (e.payload.installId === installId) onEvent(e.payload);
    },
  );
  return unlisten;
}

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

// ── Nginx panel ────────────────────────────────────────────────

/** Nginx config-file kind. The frontend uses this to decide which icon
 *  + label to render and whether the row gets the sites-enabled toggle. */
export type NginxFileKind =
  | { kind: "main" }
  | { kind: "conf-d" }
  | { kind: "site-available"; enabled: boolean }
  | { kind: "site-enabled-orphan"; linkTarget: string };

export type NginxFile = {
  path: string;
  name: string;
  kind: NginxFileKind;
  sizeBytes: number;
  /** Last-modified epoch seconds; 0 when stat failed. */
  mtimeSecs: number;
};

export type NginxLayout = {
  installed: boolean;
  /** `nginx -v` output, e.g. `"nginx version: nginx/1.24.0"`. Empty
   *  when nginx isn't on PATH. */
  version: string;
  /** Compile-time `--with-*` modules from `nginx -V`. */
  builtinModules: string[];
  files: NginxFile[];
  isRoot: boolean;
};

/** A directive in the parsed AST. The shape mirrors
 *  `pier_core::services::nginx::NginxDirective`. */
export type NginxDirective = {
  name: string;
  /** Positional args; quoted args keep their surrounding `"..."` /
   *  `'...'` so round-trip preserves the original style. */
  args: string[];
  leadingComments: string[];
  leadingBlanks: number;
  inlineComment: string | null;
  /** Block body when this directive opens `{ ... }`; mutually
   *  exclusive with `opaqueBody`. */
  block: NginxNode[] | null;
  /** Raw body for `*_by_lua_block` / `*_by_njs_block` — preserved
   *  verbatim so embedded Lua/JS isn't reinterpreted. */
  opaqueBody: string | null;
};

/** A node in the AST: either a directive (with optional block) or a
 *  standalone comment. The two share a string `kind` discriminant
 *  matching the backend's `#[serde(tag = "kind")]`. */
export type NginxNode =
  | ({ kind: "directive" } & NginxDirective)
  | { kind: "comment"; text: string; leadingBlanks: number };

export type NginxParseResult = {
  nodes: NginxNode[];
  /** Recoverable parse warnings. Empty on a clean parse. */
  errors: string[];
};

export type NginxReadFileResult = {
  path: string;
  content: string;
  parse: NginxParseResult;
};

export type NginxValidateResult = {
  ok: boolean;
  exitCode: number;
  output: string;
};

export type NginxSaveResult = {
  validate: NginxValidateResult;
  reloaded: boolean;
  reloadOutput: string;
  /** Whether the original file is in a clean state — true on success
   *  AND on a validation-fail-then-restore path. False only when the
   *  restore step itself failed. */
  restored: boolean;
  restoreError: string | null;
  backupPath: string;
};

export const nginxLayout = (params: SshParams) =>
  invoke<NginxLayout>("nginx_layout", params);

export const nginxReadFile = (params: SshParams & { path: string }) =>
  invoke<NginxReadFileResult>("nginx_read_file", params);

export const nginxSaveFile = (
  params: SshParams & { path: string; content: string },
) => invoke<NginxSaveResult>("nginx_save_file", params);

export const nginxValidate = (params: SshParams) =>
  invoke<NginxValidateResult>("nginx_validate", params);

export const nginxReload = (params: SshParams) =>
  invoke<NginxValidateResult>("nginx_reload", params);

export const nginxToggleSite = (
  params: SshParams & { siteName: string; enable: boolean },
) => invoke<NginxValidateResult>("nginx_toggle_site", params);

export const nginxCreateFile = (
  params: SshParams & { path: string; content: string },
) => invoke<NginxValidateResult>("nginx_create_file", params);

// ── Web server detection (multi-product) ─────────────────────────

export type WebServerKind = "nginx" | "apache" | "caddy";
export type WebServerRunState = "active" | "inactive" | "unknown";

export type WebServerInfo = {
  kind: WebServerKind;
  binary: string;
  version: string;
  configRoot: string;
  modulesSummary: string;
  running: WebServerRunState;
};

export type WebServerDetection = {
  detected: WebServerInfo[];
};

export type WebServerActionResult = {
  ok: boolean;
  exitCode: number;
  output: string;
};

export const webServerDetect = (params: SshParams) =>
  invoke<WebServerDetection>("web_server_detect", params);

export const webServerValidate = (
  params: SshParams & { kind: WebServerKind },
) => invoke<WebServerActionResult>("web_server_validate", params);

export const webServerReload = (
  params: SshParams & { kind: WebServerKind },
) => invoke<WebServerActionResult>("web_server_reload", params);

export type WebServerFileKind =
  | { kind: "main" }
  | { kind: "conf-d" }
  | { kind: "site-available"; enabled: boolean }
  | { kind: "other" };

export type WebServerFile = {
  path: string;
  label: string;
  kind: WebServerFileKind;
  sizeBytes: number;
};

export type WebServerLayout = {
  kind: WebServerKind;
  binary: string;
  version: string;
  configRoot: string;
  installed: boolean;
  isRoot: boolean;
  files: WebServerFile[];
};

export type WebServerSaveResult = {
  validate: WebServerActionResult;
  reloaded: boolean;
  reloadOutput: string;
  restored: boolean;
  restoreError: string | null;
  backupPath: string;
};

export const webServerLayout = (
  params: SshParams & { kind: WebServerKind },
) => invoke<WebServerLayout>("web_server_layout", params);

export const webServerReadFile = (
  params: SshParams & { kind: WebServerKind; path: string },
) => invoke<string>("web_server_read_file", params);

export const webServerSaveFile = (
  params: SshParams & { kind: WebServerKind; path: string; content: string },
) => invoke<WebServerSaveResult>("web_server_save_file", params);

export const webServerToggleSite = (
  params: SshParams & {
    kind: WebServerKind;
    siteName: string;
    enable: boolean;
  },
) => invoke<WebServerActionResult>("web_server_toggle_site", params);

export type CreateSiteResult = {
  path: string;
  enabled: boolean;
  enableOutput: string;
};

export const webServerCreateSite = (
  params: SshParams & {
    kind: WebServerKind;
    leafName: string;
    content: string;
    enableAfter: boolean;
  },
) => invoke<CreateSiteResult>("web_server_create_site", params);

// ── Caddy AST (parser + renderer) ────────────────────────────────

export type CaddyNode =
  | {
      kind: "directive";
      name: string;
      args: string[];
      leadingComments: string[];
      leadingBlanks: number;
      inlineComment: string | null;
      block: CaddyNode[] | null;
    }
  | {
      kind: "comment";
      text: string;
      leadingBlanks: number;
    };

export type CaddyParseResult = {
  nodes: CaddyNode[];
  errors: string[];
};

export const caddyParse = (content: string) =>
  invoke<CaddyParseResult>("caddy_parse", { content });

export const caddyRender = (nodes: CaddyNode[]) =>
  invoke<string>("caddy_render", { nodes });

// ── Apache AST (parser + renderer) ───────────────────────────────

export type ApacheNode =
  | {
      kind: "directive";
      name: string;
      args: string[];
      leadingComments: string[];
      leadingBlanks: number;
      inlineComment: string | null;
      section: ApacheNode[] | null;
    }
  | {
      kind: "comment";
      text: string;
      leadingBlanks: number;
    };

export type ApacheParseResult = {
  nodes: ApacheNode[];
  errors: string[];
};

export const apacheParse = (content: string) =>
  invoke<ApacheParseResult>("apache_parse", { content });

export const apacheRender = (nodes: ApacheNode[]) =>
  invoke<string>("apache_render", { nodes });

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
  /** Owner display string — named user, falling back to numeric uid. */
  owner: string;
  /** Group display string — named group, falling back to numeric gid. */
  group: string;
  /** `lf` / `crlf` / `cr` / `mixed` / `none`. */
  eol: string;
  /** `utf-8`, `utf-8-bom`, `utf-16-le`, `utf-16-be`, or `binary`. */
  encoding: string;
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

/** Result of {@link sftpOpenExternal} — backend has already
 *  downloaded the remote file to `localPath` and handed it off to
 *  the OS default editor. Hold onto `watcherId` so the dialog can
 *  call {@link sftpExternalEditStop} on close. */
export type SftpExternalEditOpen = {
  watcherId: string;
  localPath: string;
};

/** Mirror a remote SFTP file to a local temp path, open it in the
 *  user's OS default editor, and start a watcher that auto-uploads
 *  any saves. Used by the editor dialog's too-large-to-inline-edit
 *  branch and as an explicit "Open externally" action. */
export const sftpOpenExternal = (params: {
  host: string; port: number; user: string; authMode: string; password: string; keyPath: string;
  path: string;
  savedConnectionIndex?: number | null;
}) => invoke<SftpExternalEditOpen>("sftp_open_external", params);

/** Tear down the watcher started by {@link sftpOpenExternal}.
 *  Idempotent — safe to call from cleanup paths even if the
 *  watcher was never registered. */
export const sftpExternalEditStop = (watcherId: string) =>
  invoke<void>("sftp_external_edit_stop", { watcherId });

/** Payload of the `sftp:external-edit` event emitted by the
 *  watcher thread on each upload attempt. `kind` advances:
 *  `uploading` → `uploaded` (or `error`) per detected change,
 *  then a single `stopped` on shutdown. */
export type SftpExternalEditEvent = {
  watcherId: string;
  kind: "uploading" | "uploaded" | "error" | "stopped";
  bytes: number | null;
  modified: number | null;
  error: string | null;
};

/** Event channel for {@link SftpExternalEditEvent}. */
export const SFTP_EXTERNAL_EDIT_EVENT = "sftp:external-edit";

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

export const localSystemInfo = (includeDisks: boolean) =>
  invoke<ServerSnapshotView>("local_system_info", { includeDisks });

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
