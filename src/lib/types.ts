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

export type GitUnpushedCommit = {
  hash: string;
  shortHash: string;
  message: string;
  author: string;
  relativeDate: string;
  isHead: boolean;
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
  /** Common-ancestor lines from the `|||||||` section when the
   *  user has `merge.conflictStyle=diff3`. Empty when the file
   *  was generated with the default two-way style. */
  baseLines: string[];
  /** True when the `|||||||` marker was present — distinguishes
   *  "no base recorded" from "base was empty". Drives the
   *  three-column layout and the "Accept base" action. */
  hasBase: boolean;
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
  /** Optional environment tag — used for visual cues in lists / bus
   *  view to make prod hosts unmistakable from staging or local
   *  ones. Stored verbatim (`prod` / `staging` / `dev` / `local` /
   *  any free-form string); the chip styling matches by name. */
  envTag?: string | null;
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
  /** `COLUMN_COMMENT` from `SHOW FULL COLUMNS`. Empty string when
   *  the column has no comment. */
  comment: string;
};

/** Per-table enrichment surfaced as schema-tree badges + tooltip
 *  metadata. Each entry pairs 1:1 with an item in
 *  `MysqlBrowserState.tables` (same order, same `name`).
 *  `null` fields are forwarded from `information_schema.tables`
 *  when the engine hasn't gathered stats yet — the UI renders
 *  them as `—` rather than `0`. */
export type MysqlTableSummary = {
  name: string;
  rowCount: number | null;
  dataBytes: number | null;
  indexBytes: number | null;
  engine: string | null;
  updatedAt: string | null;
  /** `information_schema.tables.table_comment`. Empty string when
   *  the table has no comment. */
  comment: string;
};

/** Stored procedure / function row in the schema tree.
 *  `kind` is `"PROCEDURE"` or `"FUNCTION"` per `routine_type`. */
export type MysqlRoutineSummary = {
  name: string;
  kind: string;
};

/** Index summary for the Structure tab. `unique` distinguishes
 *  unique / primary indexes from regular ones; `kind` is the
 *  engine-specific access method (BTREE / HASH / FULLTEXT for
 *  MySQL, btree / hash / gin / gist for PG). */
export type DbIndexView = {
  name: string;
  columns: string[];
  unique: boolean;
  kind: string;
};

/** Foreign-key summary for the Structure tab. Composite FKs come
 *  back with `columns` and `refColumns` paired by index. Action
 *  strings are normalised to MySQL spelling for both engines
 *  (`NO ACTION` / `RESTRICT` / `CASCADE` / `SET NULL` /
 *  `SET DEFAULT`). */
export type DbForeignKeyView = {
  name: string;
  columns: string[];
  refSchema: string;
  refTable: string;
  refColumns: string[];
  onUpdate: string;
  onDelete: string;
};


export type MysqlBrowserState = {
  databaseName: string;
  databases: string[];
  tableName: string;
  tables: string[];
  /** Same names as `tables`, in the same order. Carries engine,
   *  row-count estimate, on-disk size, and last-update timestamp
   *  for each table. */
  tableSummaries: MysqlTableSummary[];
  /** View names defined in the active database. Rendered as a
   *  separate folder in the schema tree. */
  views: string[];
  /** Stored procedures + functions in the active database. */
  routines: MysqlRoutineSummary[];
  columns: MysqlColumnView[];
  /** Indexes on the active table; empty when no table selected. */
  indexes: DbIndexView[];
  /** Outgoing foreign keys on the active table. */
  foreignKeys: DbForeignKeyView[];
  preview: DataPreview | null;
  /** Effective page size used by the last browse — clamped to 1..500. */
  pageSize: number;
  /** Effective row offset used by the last browse. */
  pageOffset: number;
  /** SELECT COUNT(*) for the active table; null when COUNT failed
   *  or no table is selected. */
  totalRows: number | null;
  /** Wall-clock ms for the preview SELECT only — drives the
   *  toolbar elapsed-ms chip. Zero when no preview ran. */
  browseElapsedMs: number;
};

// ── SQLite ──────────────────────────────────────────────────────

export type SqliteColumnView = {
  name: string;
  colType: string;
  notNull: boolean;
  primaryKey: boolean;
};

export type SqliteIndexView = {
  name: string;
  unique: boolean;
  /** `c` (CREATE INDEX), `u` (UNIQUE constraint), or `pk` (PRIMARY KEY). */
  origin: string;
  columns: string[];
};

export type SqliteTriggerView = {
  name: string;
  /** "BEFORE INSERT" / "AFTER UPDATE" / "INSTEAD OF DELETE" / etc. */
  event: string;
  sql: string;
};

export type SqliteBrowserState = {
  path: string;
  tableName: string;
  tables: string[];
  columns: SqliteColumnView[];
  preview: DataPreview | null;
  indexes: SqliteIndexView[];
  triggers: SqliteTriggerView[];
  /** On-disk file size in bytes; 0 means stat failed. */
  fileSize: number;
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

/** Per-row enrichment for the Redis key list. The kind / TTL
 *  come back in the same scan pipeline so the UI can render
 *  badges + chips without a second roundtrip per key. */
export type RedisKeyEntry = {
  key: string;
  /** Lower-case redis-cli type name: `string` / `hash` / `list`
   *  / `set` / `zset` / `stream` / `none`. */
  kind: string;
  /** Seconds until expiry. `-1` for no TTL set, `-2` for the
   *  key not existing anymore (race window between SCAN and
   *  the TYPE/PTTL probe). */
  ttlSeconds: number;
};

export type RedisBrowserState = {
  pong: string;
  pattern: string;
  limit: number;
  truncated: boolean;
  keyName: string;
  /** Now an array of enriched entries instead of bare strings. */
  keys: RedisKeyEntry[];
  /** SCAN cursor for the next page. `"0"` means the scan
   *  reached end-of-keyspace; otherwise pass back to load more. */
  nextCursor: string;
  /** Round-trip time of the SCAN + per-key probe pipeline. */
  rttMs: number;
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
  /** Column comment via `col_description`. Empty string when the
   *  column has no comment. */
  comment: string;
};

export type PostgresPoolView = {
  /** `pg_stat_activity` rows with `state = 'active'` for the
   *  current database. 0 when the role can't read the view. */
  active: number;
  /** Total connections to the current database. */
  total: number;
};

/** Per-table enrichment surfaced in the schema tree. Same shape
 *  as `MysqlTableSummary`; PG always reports `engine: null` and
 *  `updatedAt: null` because the catalog doesn't track those. */
export type PostgresTableSummary = {
  name: string;
  rowCount: number | null;
  dataBytes: number | null;
  indexBytes: number | null;
  engine: string | null;
  updatedAt: string | null;
  /** Table comment via `obj_description`. Empty string when the
   *  table has no comment. */
  comment: string;
};

/** Stored function / procedure row in the schema tree.
 *  `kind` is upper-cased, e.g. `"FUNCTION"` / `"PROCEDURE"`. */
export type PostgresRoutineSummary = {
  name: string;
  kind: string;
};

export type PostgresBrowserState = {
  databaseName: string;
  databases: string[];
  schemaName: string;
  /** All user-visible schemas in the active database. The panel
   *  renders this as a picker the user can switch between
   *  without changing the SQL connection. Excludes `pg_catalog`,
   *  `information_schema`, and `pg_toast*`. */
  schemas: string[];
  tableName: string;
  tables: string[];
  /** Same names as `tables`, in the same order. Carries row
   *  count, data / index bytes — engine / updatedAt are always
   *  null for PG. */
  tableSummaries: PostgresTableSummary[];
  /** View names defined in the active schema. Rendered as a
   *  separate folder in the schema tree. */
  views: string[];
  /** Stored functions + procedures in the active schema. */
  routines: PostgresRoutineSummary[];
  columns: PostgresColumnView[];
  /** Indexes on the active table; empty when no table selected. */
  indexes: DbIndexView[];
  /** Outgoing foreign keys on the active table. */
  foreignKeys: DbForeignKeyView[];
  preview: DataPreview | null;
  pool: PostgresPoolView;
  /** User-defined enum types in the active schema. Drives the
   *  result grid's `<datalist>` autocomplete when editing a
   *  column whose pretty type matches one of these names. */
  enums: PostgresEnumView[];
};

export type PostgresEnumView = {
  name: string;
  values: string[];
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
  /** Raw comma-separated `key=value` label list from `docker ps`.
   *  Empty when the container has no labels. Parsed by the
   *  Projects tab to group by `com.docker.compose.project`. */
  labels?: string;
};

/**
 * Parse the comma-separated `key=value` label string that `docker
 * ps --format '{{.Labels}}'` emits into a map. Returns an empty
 * map for empty input. Docker escapes `,` inside values as `\,`,
 * but the 4 compose labels we actually read never contain commas,
 * so we keep the parser simple: a bare `split(",")`.
 */
export function parseDockerLabels(raw: string | undefined | null): Record<string, string> {
  const out: Record<string, string> = {};
  if (!raw) return out;
  for (const segment of raw.split(",")) {
    const eq = segment.indexOf("=");
    if (eq <= 0) continue;
    const key = segment.slice(0, eq).trim();
    const value = segment.slice(eq + 1).trim();
    if (key) out[key] = value;
  }
  return out;
}

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
  /** Owner display string — named user, falling back to numeric uid.
   *  Empty when the SFTP server omitted owner info. */
  owner: string;
  /** Group display string — named group, falling back to numeric gid. */
  group: string;
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
   *  pseudo filesystems (tmpfs / overlay / devtmpfs) filtered out.
   *  Empty on a fast-tier (no-disk) probe — the panel keeps the prior
   *  list visible until the next slow tick. */
  disks: DiskEntryView[];
  /** Block-device topology from `lsblk -P -b -o NAME,KNAME,PKNAME,
   *  TYPE,SIZE,ROTA,TRAN,MODEL,FSTYPE,MOUNTPOINT`. Includes physical
   *  disks (even unmounted) plus part/crypt/lvm/raid descendants so
   *  the UI can render the storage tree. Empty on macOS (no `lsblk`),
   *  on BusyBox without util-linux, and on fast-tier probes. */
  blockDevices: BlockDeviceEntryView[];
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

export type BlockDeviceEntryView = {
  /** Device basename, e.g. `nvme0n1`, `sda1`, `dm-0`. */
  name: string;
  /** Stable kernel name. Used as the tree node identifier. */
  kname: string;
  /** Parent kernel name. `""` for top-level physical disks. */
  pkname: string;
  /** `disk` / `part` / `lvm` / `crypt` / `raid1` / `loop` etc. */
  devType: string;
  /** Size in bytes (lsblk's `-b` output). 0 when missing. */
  sizeBytes: number;
  /** Rotational media — true for spinning HDDs. */
  rota: boolean;
  /** Transport bus, e.g. `sata`, `nvme`, `virtio`, `usb`. Empty for
   *  device-mapper layers (lvm/crypt) which don't have a bus. */
  tran: string;
  /** Vendor / model name string. */
  model: string;
  /** Filesystem type if this node directly holds one. */
  fsType: string;
  /** Mount point of this node only — empty if it's a parent of a
   *  mounted child rather than the mount itself. */
  mountpoint: string;
};

export type ProcessRowView = {
  pid: string;
  command: string;
  cpuPct: string;
  memPct: string;
  elapsed: string;
  /** Full argv joined by spaces. Empty when the source `ps` didn't
   *  carry it (current SSH path) or sysinfo couldn't read
   *  `/proc/<pid>/cmdline`. UI shows this as a hover tooltip. */
  cmdLine: string;
};

export type DetectedServiceView = {
  name: string;
  version: string;
  status: string;
  port: number;
};

// ── Firewall ────────────────────────────────────────────────────

export type FirewallBackend = "firewalld" | "ufw" | "nftables" | "iptables" | "none";

export type FirewallListeningPort = {
  proto: string;
  localAddr: string;
  localPort: number;
  state: string;
  process: string;
  pid: number | null;
};

export type FirewallInterfaceCounter = {
  iface: string;
  rxBytes: number;
  txBytes: number;
};

export type FirewallSnapshotView = {
  backend: FirewallBackend;
  backendActive: boolean;
  /** True when the SSH user is uid 0. The panel uses this to decide
   *  whether write actions should send `iptables …` or `sudo iptables …`
   *  to the terminal. */
  root: boolean;
  user: string;
  uname: string;
  listening: FirewallListeningPort[];
  interfaces: FirewallInterfaceCounter[];
  /** Server-side ms-since-epoch at probe time. Two snapshots → byte
   *  rate by `(b1 - b0) / ((t1 - t0) / 1000)`. */
  capturedAtMs: number;
  rulesV4: string;
  rulesV6: string;
  natV4: string;
  /** Built-in chain → policy. Only filter-table chains. */
  defaultPolicies: Record<string, string>;
  backendStatus: string;
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
  /** Smart-mode prompt-end position — `[row, col]` of the latest
   * OSC 133;B emitted by the shell. `null` when smart mode is off,
   * the shell hasn't drawn a wrapped prompt yet, or the user is
   * scrolled into history. The smart-mode UI overlays autosuggest
   * and syntax-highlight from this cell onward. */
  promptEnd: [number, number] | null;
  /** Live cursor position. The smart-mode UI uses this to anchor
   * the Tab popover at the cursor cell when `promptEnd` is null
   * (russh sessions / nested shells without OSC 133), so the popover
   * doesn't end up floating in the middle of the viewport. */
  cursorX: number;
  cursorY: number;
  /** `true` when the user is currently inside an editable input
   * line (between OSC 133;B and OSC 133;C). The mirror lineBuffer
   * accepts keystrokes only while this is set. */
  awaitingInput: boolean;
  /** `true` while a TUI is using the alternate screen (vim, htop,
   * less, tmux). The smart-mode UI must hide itself entirely. */
  altScreen: boolean;
  /** `true` while a bracketed-paste sequence is in flight. */
  bracketedPaste: boolean;
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
  | "markdown"
  | "firewall"
  | "webserver"
  | "software";

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
  backend: "local" | "ssh" | "sftp" | "markdown" | "hosts-health";
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
  /** Redis 6+ ACL username. Empty string = default user (no
   *  `AUTH username` prefix). Held in tab state only; the
   *  canonical copy lives on the saved `DbCredential`. */
  redisUser: string;
  /** Redis AUTH secret. Held in memory only — the persisted copy
   *  lives in the OS keyring via `dbCredResolve`. */
  redisPassword: string;
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

// Right-side panels (Firewall, ServerMonitor, Docker, SFTP, …) probe
// over SSH the moment they see a non-null target. The PTY watcher
// populates `sshHost`/`sshUser`/`sshPort` as soon as it spots the
// `ssh user@host` invocation — well before the user has typed the
// password — so a panel that probes on `effectiveSshTarget !== null`
// alone fires `firewall_snapshot` (etc.) with `password=""` and
// surfaces a misleading "agent + publickey rejected" error.
//
// `isSshTargetReady` answers "does the backend have enough credentials
// material to even attempt the handshake?":
//   * a saved profile means the backend resolves credentials itself
//     (keyring → ssh_cred_cache fallback);
//   * password mode requires a captured/typed password to be present;
//   * key/auto modes legitimately probe with empty `password` because
//     the backend tries agent + key file regardless.
export function isSshTargetReady(target: NestedSshTarget | null): target is NestedSshTarget {
  if (!target) return false;
  if (target.savedConnectionIndex != null) return true;
  if (target.authMode === "password") return target.password.length > 0;
  return true;
}

/** Tools that require SSH addressing on the active tab to be functional.
 *  When a tab has no SSH context (plain local terminal) the strip dims
 *  these buttons; if a persisted `rightTool` lands on one of them, the
 *  shell downgrades it to `"monitor"` (which is local-capable). Kept
 *  here, not in `rightToolMeta.ts`, so types.ts stays the canonical
 *  reference for tab-state semantics. */
export const REMOTE_ONLY_TOOLS: ReadonlySet<RightTool> = new Set<RightTool>([
  "firewall",
  "sftp",
  "log",
  "docker",
  "mysql",
  "postgres",
  "redis",
  "sqlite",
  "webserver",
  "software",
]);

/** Returns `true` when the given tool is reachable for the given tab.
 *  Local tabs (no SSH context) cannot reach SSH-only tools. `null` tab
 *  means no active session — only purely local tools (markdown / git)
 *  and the universally-capable monitor are reachable. */
export function isToolReachable(tool: RightTool, tab: TabState | null): boolean {
  if (!REMOTE_ONLY_TOOLS.has(tool)) return true;
  if (!tab) return false;
  return effectiveSshTarget(tab) !== null;
}

/** Resolve the right tool to actually display for a tab, downgrading
 *  to `"monitor"` when the persisted choice can't be reached. Used by
 *  `useTabStore.scrubRuntimeFields` (restart path) and by App.tsx
 *  (tab-switch path) so the right panel never lands on a splash that
 *  the user can't act on from the current context. */
export function resolveReachableTool(tool: RightTool, tab: TabState | null): RightTool {
  return isToolReachable(tool, tab) ? tool : "monitor";
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
