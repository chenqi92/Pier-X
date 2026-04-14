// ── Pier-X Type Definitions ──────────────────────────────────────
// Extracted from App.tsx — mirrors Tauri command return types in lib.rs

export type CoreInfo = {
  version: string;
  profile: string;
  uiTarget: string;
  workspaceRoot: string;
  defaultShell: string;
  services: string[];
};

export type FileEntry = {
  name: string;
  path: string;
  kind: "directory" | "file";
  sizeLabel: string;
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

// ── SSH ─────────────────────────────────────────────────────────

export type SavedSshConnection = {
  index: number;
  name: string;
  host: string;
  port: number;
  user: string;
  authKind: "password" | "agent" | "key";
  keyPath: string;
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
      authMode: "password" | "agent" | "key";
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

export type TabState = {
  id: string;
  title: string;
  tabColor: number; // -1 = none, 0..7 = color index
  backend: "local" | "ssh" | "sftp" | "markdown";
  // SSH credentials
  sshHost: string;
  sshPort: number;
  sshUser: string;
  sshAuthMode: "password" | "agent" | "key";
  sshPassword: string;
  sshKeyPath: string;
  // Terminal session
  terminalSessionId: string | null;
  // Right panel tool preference
  rightTool: RightTool;
  // Service context per tab
  redisHost: string;
  redisPort: number;
  redisDb: number;
  mysqlHost: string;
  mysqlPort: number;
  mysqlUser: string;
  mysqlPassword: string;
  mysqlDatabase: string;
  pgHost: string;
  pgPort: number;
  pgUser: string;
  pgPassword: string;
  pgDatabase: string;
  logCommand: string;
  markdownPath: string;
  startupCommand: string;
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
