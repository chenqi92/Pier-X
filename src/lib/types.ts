// ── Pier-X Type Definitions ──────────────────────────────────────
// Extracted from App.tsx — mirrors Tauri command return types in lib.rs

export type CoreInfo = {
  version: string;
  profile: string;
  uiTarget: string;
  homeDir: string;
  workspaceRoot: string;
  defaultShell: string;
  platform: "macos" | "windows" | "linux";
  services: string[];
};

export type FileEntry = {
  name: string;
  path: string;
  kind: "directory" | "file";
  size: number;
  sizeLabel: string;
  modified: string;
  modifiedTs: number;
};

// ── Git ─────────────────────────────────────────────────────────

export type GitChangeEntry = {
  path: string;
  status: string;
  staged: boolean;
};

export type GitOverview = {
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

export type GitCommitEntry = {
  hash: string;
  shortHash: string;
  message: string;
  author: string;
  relativeDate: string;
  refs: string;
};

export type GitStashEntry = {
  index: string;
  message: string;
  relativeDate: string;
};

export type GitPanelFile = {
  path: string;
  fileName: string;
  status: string;
  staged: boolean;
  additions: number;
  deletions: number;
};

export type GitPanelState = {
  repoPath: string;
  currentBranch: string;
  trackingBranch: string;
  aheadCount: number;
  behindCount: number;
  stagedFiles: GitPanelFile[];
  unstagedFiles: GitPanelFile[];
  totalChanges: number;
  conflictCount: number;
  workingTreeClean: boolean;
};

export type GitGraphMetadata = {
  branches: string[];
  authors: string[];
  repoFiles: string[];
  gitUserName: string;
};

export type GitGraphSegmentView = {
  xTop: number;
  yTop: number;
  xBottom: number;
  yBottom: number;
  colorIndex: number;
};

export type GitGraphArrowView = {
  x: number;
  y: number;
  colorIndex: number;
  isDown: boolean;
};

export type GitGraphRowView = {
  hash: string;
  shortHash: string;
  message: string;
  author: string;
  dateTimestamp: number;
  refs: string;
  parents: string;
  nodeColumn: number;
  colorIndex: number;
  segments: GitGraphSegmentView[];
  arrows: GitGraphArrowView[];
};

export type GitCommitChangedFileView = {
  additions: number;
  deletions: number;
  path: string;
};

export type GitCommitDetailView = {
  hash: string;
  shortHash: string;
  author: string;
  date: string;
  message: string;
  parentHash: string;
  parentHashes: string[];
  stats: string;
  changedFiles: GitCommitChangedFileView[];
};

export type GitComparisonFileView = {
  path: string;
  name: string;
  dir: string;
};

export type GitTagView = {
  name: string;
  hash: string;
  timestamp: number;
  message: string;
};

export type GitRemoteView = {
  name: string;
  fetchUrl: string;
  pushUrl: string;
};

export type GitConfigEntryView = {
  key: string;
  value: string;
  scope: string;
};

export type GitRebaseItemView = {
  id: string;
  action: string;
  hash: string;
  shortHash: string;
  message: string;
};

export type GitRebasePlanView = {
  inProgress: boolean;
  items: GitRebaseItemView[];
};

export type GitSubmoduleView = {
  path: string;
  commitHash: string;
  shortHash: string;
  status: string;
  statusSymbol: string;
  url: string;
};

export type GitConflictHunkView = {
  oursLines: string[];
  theirsLines: string[];
  resolution: string;
};

export type GitConflictFileView = {
  name: string;
  path: string;
  conflictCount: number;
  conflicts: GitConflictHunkView[];
};

export type GitBlameLineView = {
  lineNumber: number;
  hash: string;
  shortHash: string;
  author: string;
  timestamp: number;
  date: string;
  content: string;
};

export type GitGraphHistoryParams = {
  path?: string | null;
  limit?: number | null;
  skip?: number | null;
  branch?: string | null;
  author?: string | null;
  searchText?: string | null;
  firstParent?: boolean | null;
  noMerges?: boolean | null;
  afterTimestamp?: number | null;
  paths?: string[] | null;
  topoOrder?: boolean | null;
  showLongEdges?: boolean | null;
};

// ── SSH ─────────────────────────────────────────────────────────

export type SavedSshConnection = {
  index: number;
  name: string;
  host: string;
  port: number;
  user: string;
  authKind: "password" | "agent" | "key";
  keyPath: string;
  /** Explicit sidebar group label. Missing / empty means the
   *  connection lives in the implicit "default" bucket. */
  group?: string | null;
  /** Database credentials remembered for this SSH profile.
   *  Passwords are NOT included — only a `hasPassword` flag;
   *  resolve via `dbCredResolve` at connect time. */
  databases?: DbCredential[];
};

// ── DB Credentials (persisted with SSH profile) ────────────────

export type DbKind = "mysql" | "postgres" | "redis" | "sqlite";

export type DbCredentialSource =
  | { kind: "manual" }
  | { kind: "detected"; signature: string };

export type DbCredential = {
  id: string;
  kind: DbKind;
  label: string;
  host: string;
  port: number;
  user: string;
  database: string | null;
  sqlitePath: string | null;
  /** True when a password is stored (in keyring or runtime
   *  Direct fallback). Resolve lazily via `dbCredResolve`. */
  hasPassword: boolean;
  favorite: boolean;
  source: DbCredentialSource;
};

/** Input shape for `db_cred_save` — `password: null` means
 *  "no password"; omit the password field to default to
 *  passwordless for Redis/SQLite. */
export type DbCredentialInput = {
  kind: DbKind;
  label: string;
  host: string;
  port: number;
  user: string;
  database: string | null;
  sqlitePath: string | null;
  favorite: boolean;
  /** Signature of the detection row this was adopted from.
   *  Empty / omitted → `source: manual`. */
  detectionSignature?: string | null;
};

/** Patch for `db_cred_update`. Absent fields are not touched;
 *  a `{database: null}` or `{sqlitePath: null}` explicitly
 *  clears the field. */
export type DbCredentialPatch = {
  label?: string;
  host?: string;
  port?: number;
  user?: string;
  database?: string | null;
  sqlitePath?: string | null;
  favorite?: boolean;
};

/** Response from `db_cred_resolve`. Plaintext password is
 *  scoped to the Tauri IPC pipe — don't persist. */
export type DbCredentialResolved = {
  credential: DbCredential;
  password: string | null;
};

// ── DB Instance Detection (runtime, not persisted) ─────────────

export type DetectionSource = "systemd" | "docker" | "direct";
export type DetectedDbKind = "mysql" | "postgres" | "redis";

export type DetectedDbInstance = {
  source: DetectionSource;
  kind: DetectedDbKind;
  host: string;
  port: number;
  label: string;
  image?: string | null;
  containerId?: string | null;
  version?: string | null;
  pid?: number | null;
  processName?: string | null;
  /** Stable dedupe key; lines up with `detectionSignature`
   *  on saved credentials. */
  signature: string;
};

export type DbDetectionReport = {
  instances: DetectedDbInstance[];
  /** CLI availability on the remote host. */
  mysqlCli: boolean;
  psqlCli: boolean;
  redisCli: boolean;
  sqliteCli: boolean;
};

// ── Data Previews ───────────────────────────────────────────────

export type DataPreview = {
  columns: string[];
  rows: string[][];
  truncated: boolean;
};

export type QueryExecutionResult = {
  columns: string[];
  rows: string[][];
  truncated: boolean;
  affectedRows: number;
  lastInsertId: number | null;
  elapsedMs: number;
};

// ── MySQL ───────────────────────────────────────────────────────

export type MysqlColumnView = {
  name: string;
  columnType: string;
  nullable: boolean;
  key: string;
  defaultValue: string;
  extra: string;
};

export type MysqlBrowserState = {
  databaseName: string;
  databases: string[];
  tableName: string;
  tables: string[];
  columns: MysqlColumnView[];
  preview: DataPreview | null;
};

// ── SQLite ──────────────────────────────────────────────────────

export type SqliteColumnView = {
  name: string;
  colType: string;
  notNull: boolean;
  primaryKey: boolean;
};

export type SqliteBrowserState = {
  path: string;
  tableName: string;
  tables: string[];
  columns: SqliteColumnView[];
  preview: DataPreview | null;
};

// ── Redis ───────────────────────────────────────────────────────

export type RedisKeyView = {
  key: string;
  kind: string;
  length: number;
  ttlSeconds: number;
  encoding: string;
  preview: string[];
  previewTruncated: boolean;
};

export type RedisBrowserState = {
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

export type RedisCommandResult = {
  summary: string;
  lines: string[];
  elapsedMs: number;
};

// ── PostgreSQL ──────────────────────────────────────────────────

export type PostgresColumnView = {
  name: string;
  columnType: string;
  nullable: boolean;
  key: string;
  defaultValue: string;
  extra: string;
};

export type PostgresBrowserState = {
  databaseName: string;
  databases: string[];
  schemaName: string;
  tableName: string;
  tables: string[];
  columns: PostgresColumnView[];
  preview: DataPreview | null;
};

// ── Docker ──────────────────────────────────────────────────────

export type DockerContainerView = {
  id: string;
  image: string;
  names: string;
  status: string;
  state: string;
  created: string;
  ports: string;
  running: boolean;
  /** Pre-formatted CPU percent from `docker stats`, e.g. "1.23%". Empty when unavailable. */
  cpuPerc: string;
  /** Pre-formatted memory usage, e.g. "48.5MiB / 1.94GiB". Empty when unavailable. */
  memUsage: string;
  /** Memory percent of the container limit, e.g. "2.44%". Empty when unavailable. */
  memPerc: string;
};

export type DockerImageView = {
  id: string;
  repository: string;
  tag: string;
  size: string;
  created: string;
};

export type DockerVolumeView = {
  name: string;
  driver: string;
  mountpoint: string;
  /** Pre-formatted volume size from `docker system df -v`, e.g. "4.2GB". Empty when unavailable. */
  size: string;
  /** Raw byte count for sort-by-size. `0` when unknown. */
  sizeBytes: number;
  /** Number of containers referencing this volume. `-1` when unknown. */
  links: number;
};

export type DockerNetworkView = {
  id: string;
  name: string;
  driver: string;
  scope: string;
};

export type DockerOverview = {
  containers: DockerContainerView[];
  images: DockerImageView[];
  volumes: DockerVolumeView[];
  networks: DockerNetworkView[];
};

// ── SFTP ────────────────────────────────────────────────────────

export type SftpEntryView = {
  name: string;
  path: string;
  isDir: boolean;
  size: number;
  permissions: string;
  /** Last-modified timestamp (Unix seconds) if the server reported one. */
  modified: number | null;
};

export type SftpBrowseState = {
  currentPath: string;
  entries: SftpEntryView[];
};

// ── Server Monitor ──────────────────────────────────────────────

export type ServerSnapshotView = {
  uptime: string;
  load1: number;
  load5: number;
  load15: number;
  memTotalMb: number;
  memUsedMb: number;
  memFreeMb: number;
  swapTotalMb: number;
  swapUsedMb: number;
  diskTotal: string;
  diskUsed: string;
  diskAvail: string;
  diskUsePct: number;
  cpuPct: number;
  /** Logical CPU count from `nproc`. 0 when unavailable. */
  cpuCount: number;
  /** Total process count. 0 when unavailable. */
  procCount: number;
  /** OS / kernel label, e.g. `"Ubuntu 24.04.1 · 5.15.0-139-generic"`. */
  osLabel: string;
  /** Bytes-per-second received across non-loopback interfaces. `-1`
   *  on the first probe (no baseline yet) or when `/proc/net/dev`
   *  isn't available. */
  netRxBps: number;
  netTxBps: number;
  topProcesses: ProcessRowView[];
  /** Same shape as `topProcesses` but sorted by memory % rather than
   *  CPU %. Populated independently on the remote rather than
   *  client-side resorted, so genuine memory hogs (Java heaps, DB
   *  caches) that sit near 0% CPU still surface. */
  topProcessesMem: ProcessRowView[];
  /** Per-filesystem breakdown from `df -hPT`, with Docker volumes and
   *  pseudo filesystems (tmpfs / overlay / devtmpfs) filtered out. */
  disks: DiskEntryView[];
};

export type DiskEntryView = {
  filesystem: string;
  fsType: string;
  total: string;
  used: string;
  avail: string;
  usePct: number;
  mountpoint: string;
};

export type ProcessRowView = {
  pid: string;
  command: string;
  cpuPct: string;
  memPct: string;
  elapsed: string;
};

export type DetectedServiceView = {
  name: string;
  version: string;
  status: string;
  port: number;
};

export type LogEventView = {
  kind: "stdout" | "stderr" | "exit" | "error";
  text: string;
};

// ── Log Source (structured selector state) ─────────────────────
//
// The Log panel compiles a LogSource into the shell command that
// `log_stream_start` runs. File and System modes are the default
// paths; Custom is a fallback for paste-a-command use cases.
export type LogSourceMode = "file" | "system" | "custom";

export type LogSource = {
  mode: LogSourceMode;
  /** File mode: absolute remote path of the log file. */
  filePath: string;
  /** File mode: the directory we last listed (so we can repopulate the dropdown). */
  fileDir: string;
  /** System mode: id into LOG_SYSTEM_PRESETS. */
  systemPresetId: string;
  /** System mode: optional argument (unit name, container id, …). */
  systemArg: string;
  /** Custom mode: raw shell command. */
  customCommand: string;
};

export type TunnelInfoView = {
  tunnelId: string;
  localHost: string;
  localPort: number;
  remoteHost: string;
  remotePort: number;
  alive: boolean;
};

// ── Terminal ────────────────────────────────────────────────────

export type TerminalSessionInfo = {
  sessionId: string;
  shell: string;
  cols: number;
  rows: number;
};

export type TerminalSegment = {
  text: string;
  fg: string;
  bg: string;
  bold: boolean;
  underline: boolean;
  cursor: boolean;
};

export type TerminalLine = {
  segments: TerminalSegment[];
};

export type TerminalSnapshot = {
  cols: number;
  rows: number;
  alive: boolean;
  scrollbackLen: number;
  bellPending: boolean;
  lines: TerminalLine[];
};

export type TerminalSize = {
  cols: number;
  rows: number;
};

export type TerminalTarget =
  | { kind: "local" }
  | { kind: "sshSaved"; index: number; label: string }
  | {
      kind: "ssh";
      host: string;
      port: number;
      user: string;
      authMode: "password" | "agent" | "key" | "auto";
      password?: string;
      keyPath?: string;
    };

// ── UI Surface Types ────────────────────────────────────────────

export type DataSurface = "mysql" | "sqlite" | "redis" | "postgres";

export type RightTool =
  | "git"
  | "monitor"
  | "docker"
  | "mysql"
  | "redis"
  | "log"
  | "sftp"
  | "sqlite"
  | "postgres"
  | "markdown";

// ── Tab Model (matches Qt Main.qml tab schema) ─────────────────

/**
 * Overlay SSH addressing inferred from the user typing `ssh user@host`
 * inside an already-SSH tab. Panels that probe a host with a SEPARATE
 * SSH session (Server Monitor, Detected Services) prefer this over
 * the tab's primary `ssh*` fields, so the right sidebar reflects the
 * nested target without disturbing the live PTY / tunnels rooted on
 * the original host. Cleared when the user starts typing a non-`ssh`
 * line on the same prompt is *not* attempted — once set, it stays
 * until explicitly replaced or the tab closes.
 */
export type NestedSshTarget = {
  host: string;
  user: string;
  port: number;
  authMode: "password" | "agent" | "key" | "auto";
  password: string;
  keyPath: string;
  savedConnectionIndex: number | null;
};

export type TabState = {
  id: string;
  title: string;
  tabColor: number; // -1 = none, 0..7 = color index
  backend: "local" | "ssh" | "sftp" | "markdown";
  // SSH credentials
  sshHost: string;
  sshPort: number;
  sshUser: string;
  sshAuthMode: "password" | "agent" | "key" | "auto";
  sshPassword: string;
  sshKeyPath: string;
  /** Index into the saved-connections list. When set, the backend
   * resolves the password from the secure store instead of relying on
   * `sshPassword` being passed from the frontend. */
  sshSavedConnectionIndex: number | null;
  // Terminal session
  terminalSessionId: string | null;
  // Right panel tool preference
  rightTool: RightTool;
  // Service context per tab
  redisHost: string;
  redisPort: number;
  redisDb: number;
  redisTunnelId: string | null;
  redisTunnelPort: number | null;
  mysqlHost: string;
  mysqlPort: number;
  mysqlUser: string;
  mysqlPassword: string;
  mysqlDatabase: string;
  mysqlTunnelId: string | null;
  mysqlTunnelPort: number | null;
  pgHost: string;
  pgPort: number;
  pgUser: string;
  pgPassword: string;
  pgDatabase: string;
  pgTunnelId: string | null;
  pgTunnelPort: number | null;
  /** When set, points at a `SavedSshConnection.databases[]`
   *  entry of the matching kind. Drives the instance picker
   *  pill-bar selection and the auto-browse effect on saved
   *  profile open. `null` = "user is filling in manually". */
  mysqlActiveCredentialId: string | null;
  pgActiveCredentialId: string | null;
  redisActiveCredentialId: string | null;
  sqliteActiveCredentialId: string | null;
  logCommand: string;
  logSource: LogSource;
  markdownPath: string;
  startupCommand: string;
  /** Registry mirror prefix for `docker pull`, e.g.
   *  `"docker.m.daocloud.io"`. Applied only when the image ref does not
   *  already contain a registry domain. Empty → no rewrite. */
  dockerRegistryMirror: string;
  /** Optional `HTTPS_PROXY` value passed as a one-off env var to
   *  `docker pull`. Does not touch the remote daemon config. */
  dockerPullProxy: string;
  /** Set when this tab is a real SSH tab and the user typed
   *  `ssh user@host` inside that session — nested SSH. The right
   *  sidebar reads this in preference to the primary ssh* fields so
   *  it can monitor the nested target while leaving the original
   *  session and any tunnels untouched. `null` on local tabs and on
   *  SSH tabs that have not seen a nested ssh command. */
  nestedSshTarget: NestedSshTarget | null;
};

/**
 * Resolve the SSH addressing the right-side panels should target
 * for this tab. Honors a nested-ssh overlay if one is set, otherwise
 * falls back to the tab's primary ssh* fields. Returns `null` only
 * when the tab has no usable SSH context at all.
 */
export function effectiveSshTarget(tab: TabState): NestedSshTarget | null {
  if (tab.nestedSshTarget) return tab.nestedSshTarget;
  if (!tab.sshHost.trim() || !tab.sshUser.trim()) return null;
  return {
    host: tab.sshHost,
    user: tab.sshUser,
    port: tab.sshPort,
    authMode: tab.sshAuthMode,
    password: tab.sshPassword,
    keyPath: tab.sshKeyPath,
    savedConnectionIndex: tab.sshSavedConnectionIndex,
  };
}

export const DEFAULT_LOG_SOURCE: LogSource = {
  mode: "system",
  filePath: "",
  fileDir: "/var/log",
  systemPresetId: "syslog",
  systemArg: "",
  customCommand: "",
};

// ── Tab color palette (matches Qt TabBar.qml) ──────────────────

export const TAB_COLORS = [
  { name: "Red", value: "#e06c75" },
  { name: "Orange", value: "#d19a66" },
  { name: "Yellow", value: "#e5c07b" },
  { name: "Green", value: "#98c379" },
  { name: "Blue", value: "#61afef" },
  { name: "Purple", value: "#c678dd" },
  { name: "Pink", value: "#e06c95" },
  { name: "Teal", value: "#56b6c2" },
] as const;
