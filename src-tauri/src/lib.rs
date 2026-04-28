use pier_core::connections::{
    self, ConnectionStore, DbCredentialPatch, NewDbCredential, ResolvedDbCredential,
};
use pier_core::credentials;
use pier_core::markdown;
use pier_core::services::docker;
use pier_core::services::firewall;
use pier_core::services::git::{CommitInfo, GitClient, StashEntry, UnpushedCommit};
use pier_core::services::mysql::{self as mysql_service, MysqlClient, MysqlConfig};
use pier_core::services::nginx;
use pier_core::services::postgres::{PostgresClient, PostgresConfig};
use pier_core::services::redis::{RedisClient, RedisConfig};
use pier_core::services::package_manager;
use pier_core::services::package_mirror;
use pier_core::services::server_monitor;
use pier_core::services::sqlite::SqliteClient;
use pier_core::services::sqlite_remote;
use pier_core::ssh::config::{DbCredential, DbCredentialSource, DbKind};
use pier_core::ssh::db_detect::{self, DbDetectionReport, DetectedDbInstance};
use pier_core::ssh::service_detector;
use pier_core::ssh::{
    AuthMethod, ExecStream, HostKeyVerifier, SftpClient, SshConfig, SshSession, Tunnel,
};
use pier_core::terminal::{Cell, Color, NotifyEvent, NotifyFn, PierTerminal};
use serde::Serialize;
use std::collections::HashMap;
use std::ffi::c_void;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};
use tokio_util::sync::CancellationToken;

mod git_panel;
use git_panel::*;

mod ssh_mux;

mod ssh_cred_cache;
use ssh_cred_cache::{SshCredCache, TargetKey};

mod terminal_smart;
use terminal_smart::{
    completion_library_install_pack, completion_library_install_pack_from_path,
    completion_library_list, completion_library_reload, completion_library_remove_pack,
    terminal_completions, terminal_history_clear, terminal_history_load, terminal_history_push,
    terminal_man_synopsis, terminal_validate_command,
};

struct AppState {
    next_terminal_id: AtomicU64,
    next_tunnel_id: AtomicU64,
    terminals: Mutex<HashMap<String, ManagedTerminal>>,
    tunnels: Mutex<HashMap<String, ManagedTunnel>>,
    log_streams: Mutex<HashMap<String, ExecStream>>,
    /// Cached SSH sessions reused across SFTP panel calls so we don't
    /// re-handshake on every directory listing. Keyed by
    /// `auth_mode:user@host:port` — identity bits are only the SSH
    /// addressing, not the password, so rotating a saved password
    /// invalidates the cache via explicit eviction (not by changing
    /// the key).
    sftp_sessions: Mutex<HashMap<String, Arc<SshSession>>>,
    /// Cached SFTP subsystem handles, one per SSH session key. Each
    /// SFTP panel command used to re-issue `request_subsystem("sftp")`
    /// (two extra round-trips per call); we now open it once per
    /// session and reuse. `SftpClient` is Arc-backed internally so
    /// `clone()` is cheap. Entries are invalidated alongside
    /// `sftp_sessions` via [`evict_ssh_session`] whenever the
    /// underlying SSH connection dies.
    sftp_clients: Mutex<HashMap<String, SftpClient>>,
    /// Resolved remote `$HOME` (or best-candidate starting dir) per
    /// session, so the ~8-RTT probe in [`resolve_remote_home`] only
    /// runs on the first browse for a target. Invalidated together
    /// with the SSH session — a reconnect means we re-probe, since
    /// the server config (mounts, homedir location) may have
    /// actually changed.
    sftp_home_cache: Mutex<HashMap<String, String>>,
    /// Per-target handshake coordination — a singleflight gate plus
    /// a short-lived negative cache. Every caller with a cache miss
    /// acquires the per-key [`HandshakeGuard`] from this map:
    ///
    ///   * one thread wins the gate and runs the actual handshake;
    ///   * waiters on the same key block on the gate, then re-check
    ///     both the session cache AND the negative-failure cache —
    ///     so if the winner's handshake rejected, every other waiter
    ///     returns the same error without running its own attempt.
    ///     Without the negative cache, N waiters on a broken target
    ///     each serially re-tried a full connect, turning one slow
    ///     failure into N × `connect_timeout_secs` of blocked IPC
    ///     worker threads.
    ///
    /// Entries are never removed; memory cost is one guard per
    /// unique target ever seen during this run (negligible).
    session_init_guards: Mutex<HashMap<String, Arc<HandshakeGuard>>>,
    /// Per-target `/proc/net/dev` baselines used by
    /// `server_monitor_probe` to compute network throughput between
    /// successive polls. Keyed the same way as `sftp_sessions` so a
    /// session eviction also lets the network baseline reset
    /// naturally. Only the most recent sample is kept; the
    /// `Option` lets the first probe install a baseline without a
    /// rate, and every subsequent one diff against it.
    monitor_net_baselines: Mutex<HashMap<String, server_monitor::NetSample>>,
    /// Process-level SSH credential cache: maps `(host, port, user)`
    /// to whatever password / passphrase / explicit key path the
    /// terminal-side ssh just successfully used. Right-side panels
    /// (firewall, monitor, SFTP, Docker, DB tunnels) consult this
    /// before falling through to the empty-credential AutoChain.
    ///
    /// In-memory only; cleared on app exit. See
    /// [`ssh_cred_cache::SshCredCache`] for the rationale.
    ssh_cred_cache: SshCredCache,
    /// In-flight cancellation tokens for the Software panel's
    /// install / update / uninstall lifecycles. Keyed by the same
    /// `installId` the frontend already generates per-row, so the
    /// cancel command is a one-line `HashMap::get`. Tokens are
    /// inserted right before the `spawn_blocking` task starts and
    /// removed in the join branch — success, failure, and explicit
    /// cancel all clean up.
    software_cancel: Mutex<HashMap<String, CancellationToken>>,
    /// Active "open with external editor" sessions for the SFTP
    /// editor dialog. Each entry keeps the local temp file path,
    /// the remote target, and a cancellation token that the file
    /// watcher thread polls so `sftp_external_edit_stop` can wind
    /// it down without races. Entries are removed on stop; the
    /// watcher thread also exits when the token is cancelled.
    external_editors: Mutex<HashMap<String, ExternalEditWatcher>>,
}

/// Bookkeeping for one in-flight "edit remote file with the OS
/// default editor" session. The watcher thread polls
/// [`local_path`] for mtime/size changes and uploads the bytes
/// back to [`remote_path`] over the cached SFTP client when the
/// file settles. `cleanup_temp_dir` drives whether stop should
/// also `remove_dir_all` the per-watcher temp directory.
struct ExternalEditWatcher {
    stop_token: CancellationToken,
    local_path: PathBuf,
    cleanup_temp_dir: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            next_terminal_id: AtomicU64::new(1),
            next_tunnel_id: AtomicU64::new(1),
            terminals: Mutex::new(HashMap::new()),
            tunnels: Mutex::new(HashMap::new()),
            log_streams: Mutex::new(HashMap::new()),
            sftp_sessions: Mutex::new(HashMap::new()),
            sftp_clients: Mutex::new(HashMap::new()),
            sftp_home_cache: Mutex::new(HashMap::new()),
            session_init_guards: Mutex::new(HashMap::new()),
            monitor_net_baselines: Mutex::new(HashMap::new()),
            ssh_cred_cache: SshCredCache::default(),
            software_cancel: Mutex::new(HashMap::new()),
            external_editors: Mutex::new(HashMap::new()),
        }
    }
}

/// Singleflight gate + negative cache for handshake attempts against
/// a single target. Callers with a cache miss pull an Arc of this
/// from `AppState.session_init_guards` and interact with it before
/// attempting their own `SshSession::connect_blocking`.
struct HandshakeGuard {
    /// Serialises handshake attempts — winner runs the connect,
    /// losers wait then re-check the cache and negative entry.
    gate: Mutex<()>,
    /// Latest failed handshake for this target, if any: when it
    /// happened, the error string, and a fingerprint of the
    /// credentials that produced the failure. Waiters that hit this
    /// within the short TTL below AND with a matching fingerprint
    /// short-circuit on the same error; mismatched fingerprints
    /// (e.g. the watcher just captured a password from the OpenSSH
    /// prompt, so the credential bag changed since the last attempt)
    /// bypass the negative cache and run a fresh handshake. Older
    /// entries past the TTL are ignored regardless, so a transient
    /// failure (wifi flap, sshd restart) doesn't permanently
    /// blackhole a target.
    last_fail: Mutex<Option<(Instant, String, u64)>>,
}

impl HandshakeGuard {
    fn new() -> Self {
        Self {
            gate: Mutex::new(()),
            last_fail: Mutex::new(None),
        }
    }
}

/// Stable hash of the credential bag we're about to attempt. Used by
/// the handshake negative-cache so a previous failure stops gating
/// the moment any of the inputs changes — most importantly the
/// transition from "no captured password yet" to "user typed
/// password into ssh prompt", which is exactly when the right-side
/// panels need to reconnect even though we just saw `auto:user@host`
/// fail seconds ago.
///
/// Includes `saved_index` so picking a different saved profile also
/// invalidates a cached failure. Cheap (FxHash via DefaultHasher);
/// collisions only cost one unnecessary skip of the negative cache,
/// which is the safe direction.
fn ssh_credential_fingerprint(
    auth_mode: &str,
    password: &str,
    key_path: &str,
    saved_index: Option<usize>,
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    auth_mode.hash(&mut h);
    password.hash(&mut h);
    key_path.hash(&mut h);
    saved_index.hash(&mut h);
    h.finish()
}

/// How long a recent handshake failure suppresses further attempts
/// from waiters on the same target. Short enough that a user who
/// retries a few seconds later actually gets a fresh attempt; long
/// enough that a storm of waiters all piling in on the same stale
/// session / wrong-password / unreachable-host failure doesn't each
/// wait another full `connect_timeout_secs`.
const HANDSHAKE_NEGATIVE_CACHE: Duration = Duration::from_secs(3);

/// Event emitted to the webview whenever a terminal session has new output
/// or exits. The frontend listens for this and requests a fresh snapshot —
/// replaces the old 80ms polling loop.
const TERMINAL_EVENT: &str = "terminal:event";

/// Event emitted when the SSH-child watcher observes a change in the
/// set of `ssh` clients running under a local terminal. Payload carries
/// the innermost live target or `null` to signal "no ssh is currently
/// running in this terminal". The frontend is the authoritative
/// subscriber: it updates `tab.sshHost` / `tab.nestedSshTarget` straight
/// from this event, so the right-side Server Monitor panel follows the
/// terminal instead of the other way around.
const TERMINAL_SSH_STATE_EVENT: &str = "terminal:ssh-state";

/// One-shot "the PTY just printed an OpenSSH server password prompt"
/// signal, emitted from the terminal reader thread when it sees
/// `<user>@<host>'s password:`. The frontend arms a single-line
/// capture so the next Enter-terminated keystroke stream lands in
/// `tab.sshPassword` (and the process-level credential cache) for
/// the right-side russh session.
const TERMINAL_SSH_PASSWORD_PROMPT_EVENT: &str = "terminal:ssh-password-prompt";

/// Sibling of [`TERMINAL_SSH_PASSWORD_PROMPT_EVENT`] but for OpenSSH
/// key-decryption passphrase prompts (`Enter passphrase for key
/// '<path>':`). Fires a different event because the captured value
/// belongs in `tab.sshKeyPassphrase`, not `tab.sshPassword` —
/// crossing them costs the user a wrong auth attempt and surfaces
/// as a confusing "auth rejected" error on the right side.
#[allow(dead_code)]
const TERMINAL_SSH_PASSPHRASE_PROMPT_EVENT: &str = "terminal:ssh-passphrase-prompt";

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TerminalEventPayload {
    session_id: String,
    /// "data" → snapshot dirty, fetch a new one.
    /// "exit" → child process ended; no more data events will fire.
    kind: &'static str,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TerminalSshStatePayload {
    session_id: String,
    /// `None` when no ssh client is running inside the terminal.
    target: Option<TerminalSshTargetView>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TerminalSshTargetView {
    host: String,
    user: String,
    port: u16,
    /// `-i <path>` if the user passed one. Empty string ≈ not set;
    /// frontend treats empty as "use saved connection's key or
    /// interactive password".
    identity_path: String,
}

/// State carried across the C-FFI notify boundary. The pointer handed to
/// `PierTerminal::new` lives inside this `Box`, which `ManagedTerminal`
/// keeps alive for the session's lifetime — the field declaration order
/// guarantees `terminal` is dropped (and its reader thread joined)
/// before we deallocate the context the reader was using.
struct NotifyContext {
    app: tauri::AppHandle,
    session_id: String,
}

struct ManagedTerminal {
    // Drop order: `terminal` drops first, which signals shutdown and joins
    // the reader thread. Only then is `_notify_ctx` freed — otherwise the
    // reader could fire the notify callback against a dangling pointer.
    terminal: PierTerminal,
    _notify_ctx: Box<NotifyContext>,
}

struct ManagedTunnel {
    tunnel: Tunnel,
    remote_host: String,
    remote_port: u16,
    local_port: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreInfo {
    version: String,
    profile: &'static str,
    ui_target: &'static str,
    home_dir: String,
    workspace_root: String,
    default_shell: String,
    platform: &'static str,
    services: Vec<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileEntry {
    name: String,
    path: String,
    kind: &'static str,
    size: u64,
    size_label: String,
    modified: String,
    modified_ts: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitChangeEntry {
    path: String,
    status: String,
    staged: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitOverview {
    repo_path: String,
    branch_name: String,
    tracking: String,
    ahead: i32,
    behind: i32,
    is_clean: bool,
    staged_count: usize,
    unstaged_count: usize,
    changes: Vec<GitChangeEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitCommitEntry {
    hash: String,
    short_hash: String,
    message: String,
    author: String,
    relative_date: String,
    refs: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitStashEntry {
    index: String,
    message: String,
    relative_date: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DataPreview {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    truncated: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryExecutionResult {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    truncated: bool,
    affected_rows: u64,
    last_insert_id: Option<u64>,
    elapsed_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MysqlColumnView {
    name: String,
    column_type: String,
    nullable: bool,
    key: String,
    default_value: String,
    extra: String,
    comment: String,
}

/// Per-table enrichment surfaced in the MySQL panel's schema tree.
/// Mirrors `pier_core::services::mysql::TableSummary` but with a
/// camelCase serialisation so the frontend doesn't have to remap.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MysqlTableSummary {
    name: String,
    row_count: Option<u64>,
    data_bytes: Option<u64>,
    index_bytes: Option<u64>,
    engine: Option<String>,
    updated_at: Option<String>,
    comment: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MysqlRoutineSummary {
    name: String,
    /// `"PROCEDURE"` or `"FUNCTION"`.
    kind: String,
}

/// One index summary surfaced in the Structure tab — same shape
/// as `pier_core::services::mysql::IndexSummary` but in
/// camelCase for direct JSON consumption.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MysqlIndexView {
    name: String,
    columns: Vec<String>,
    unique: bool,
    kind: String,
}

/// One foreign-key summary surfaced in the Structure tab —
/// same shape as `pier_core::services::mysql::ForeignKey`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MysqlForeignKeyView {
    name: String,
    columns: Vec<String>,
    ref_schema: String,
    ref_table: String,
    ref_columns: Vec<String>,
    on_update: String,
    on_delete: String,
}


#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MysqlBrowserState {
    database_name: String,
    databases: Vec<String>,
    table_name: String,
    /// Bare table names — kept as an array of strings so the
    /// existing selectors (`tables.find(name === …)`) keep
    /// working without a reshape on the frontend.
    tables: Vec<String>,
    /// Per-table enrichment. Same `name`s as `tables`, in the
    /// same order, but with the engine / row-count / size /
    /// last-update extras the panel renders as inline badges
    /// and tooltip metadata.
    table_summaries: Vec<MysqlTableSummary>,
    /// View names defined in the active database. Rendered in a
    /// separate folder under the database in the schema tree.
    views: Vec<String>,
    /// Stored procedures + functions defined in the active
    /// database. The `kind` field discriminates the two.
    routines: Vec<MysqlRoutineSummary>,
    columns: Vec<MysqlColumnView>,
    /// All indexes on the active table. Empty when no table is
    /// selected, or when `information_schema.statistics` isn't
    /// readable for the connected user (failsoft).
    indexes: Vec<MysqlIndexView>,
    /// All outgoing foreign keys on the active table. Empty when
    /// no table is selected.
    foreign_keys: Vec<MysqlForeignKeyView>,
    preview: Option<DataPreview>,
    /// Effective page size used for the preview query — what the
    /// caller asked for, clamped to `[1, 500]`. The panel echoes
    /// this back into the next request.
    page_size: u64,
    /// Effective offset used for the preview query.
    page_offset: u64,
    /// `SELECT COUNT(*)` for the active table, when computable.
    /// `None` on error or when no table is selected — the panel
    /// renders `Page N of ?` in that case.
    total_rows: Option<u64>,
    /// Wall-clock for the preview SELECT only — feeds the grid
    /// toolbar's "{ms} ms" chip. Zero when no preview ran.
    browse_elapsed_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SqliteColumnView {
    name: String,
    col_type: String,
    not_null: bool,
    primary_key: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SqliteIndexView {
    name: String,
    unique: bool,
    /// Origin marker from `PRAGMA index_list`: `c` (CREATE INDEX),
    /// `u` (UNIQUE constraint), or `pk` (PRIMARY KEY).
    origin: String,
    columns: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SqliteTriggerView {
    name: String,
    event: String,
    sql: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SqliteBrowserState {
    path: String,
    table_name: String,
    tables: Vec<String>,
    columns: Vec<SqliteColumnView>,
    preview: Option<DataPreview>,
    /// Indexes attached to the active table — empty when no
    /// table is selected.
    indexes: Vec<SqliteIndexView>,
    /// Triggers attached to the active table — empty when no
    /// table is selected.
    triggers: Vec<SqliteTriggerView>,
    /// On-disk size of the SQLite file in bytes; 0 when stat
    /// fails (the panel treats 0 as "unknown").
    file_size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RedisKeyView {
    key: String,
    kind: String,
    length: u64,
    ttl_seconds: i64,
    encoding: String,
    preview: Vec<String>,
    preview_truncated: bool,
}

/// Enriched key-list row — name plus per-key kind / TTL pulled
/// in the same scan pipeline. Lets the panel render the
/// `STR`/`HASH`/`LIST`/`ZSET`/`STREAM`/`SET` badge + a TTL chip
/// without a follow-up call per key.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RedisKeyEntry {
    key: String,
    /// Lower-case redis-cli type name.
    kind: String,
    /// Seconds until expiry; `-1` for no TTL, `-2` for missing.
    ttl_seconds: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RedisBrowserState {
    pong: String,
    pattern: String,
    limit: usize,
    truncated: bool,
    key_name: String,
    /// Each entry pairs a key with its kind + TTL. Replaces the
    /// previous bare-name `Vec<String>` so the panel can render
    /// per-row badges without a second roundtrip per key.
    keys: Vec<RedisKeyEntry>,
    /// Next-page cursor returned by `SCAN`. `"0"` means the
    /// scan reached the end; anything else is the resume token
    /// the panel should pass on a load-more click.
    next_cursor: String,
    /// Round-trip time of the scan + per-key probe pipeline.
    /// Surfaced as a small chip in the panel header.
    rtt_ms: u64,
    server_version: String,
    used_memory: String,
    details: Option<RedisKeyView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RedisCommandResultView {
    summary: String,
    lines: Vec<String>,
    elapsed_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PostgresColumnView {
    name: String,
    column_type: String,
    nullable: bool,
    key: String,
    default_value: String,
    extra: String,
    comment: String,
}

/// Per-table enrichment for the PG schema tree. Same shape as
/// `MysqlTableSummary` so the frontend can share the badge +
/// tooltip rendering logic. PG leaves `engine` / `updatedAt`
/// `null` because the PostgreSQL catalog doesn't expose those
/// the way MySQL's `information_schema.tables` does.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PostgresTableSummary {
    name: String,
    row_count: Option<u64>,
    data_bytes: Option<u64>,
    index_bytes: Option<u64>,
    engine: Option<String>,
    updated_at: Option<String>,
    comment: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PostgresRoutineSummary {
    name: String,
    /// Upper-cased `routine_type`, e.g. `"FUNCTION"` / `"PROCEDURE"`.
    kind: String,
}

/// One index summary for the PG Structure tab. Same shape as
/// the MySQL view; PG `kind` is the access-method name (`btree`,
/// `hash`, `gin`, …).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PostgresIndexView {
    name: String,
    columns: Vec<String>,
    unique: bool,
    kind: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PostgresForeignKeyView {
    name: String,
    columns: Vec<String>,
    ref_schema: String,
    ref_table: String,
    ref_columns: Vec<String>,
    on_update: String,
    on_delete: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PostgresPoolView {
    /// `pg_stat_activity` rows whose `state = 'active'` for the
    /// current database. Zero when the role can't read the view.
    active: u32,
    /// Total connections to the current database (any state).
    total: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PostgresBrowserState {
    database_name: String,
    databases: Vec<String>,
    schema_name: String,
    /// All user-visible schemas in the active database. The panel
    /// renders these as the left-rail picker; the active schema is
    /// `schema_name`. Excludes `pg_catalog` / `information_schema`
    /// / `pg_toast*`. Empty when no database is selected.
    schemas: Vec<String>,
    table_name: String,
    /// Bare table names in the active schema. Kept as
    /// `Vec<String>` for selectors that look tables up by name.
    tables: Vec<String>,
    /// 1:1 enrichment for `tables` — row count / data size /
    /// index size. `engine` and `updatedAt` are always `null`
    /// for PG.
    table_summaries: Vec<PostgresTableSummary>,
    /// View names defined in the active schema. Rendered in a
    /// separate folder.
    views: Vec<String>,
    /// Stored functions + procedures defined in the active
    /// schema. The `kind` field discriminates the two.
    routines: Vec<PostgresRoutineSummary>,
    columns: Vec<PostgresColumnView>,
    /// All indexes on the active table. Empty when no table is
    /// selected; failsoft to empty when `pg_index` isn't
    /// readable for the connected role.
    indexes: Vec<PostgresIndexView>,
    /// All outgoing foreign keys on the active table.
    foreign_keys: Vec<PostgresForeignKeyView>,
    preview: Option<DataPreview>,
    /// Connection-pool snapshot for the active database. `(0, 0)`
    /// when the role lacks `pg_stat_activity` access — the panel
    /// hides the chip in that case.
    pool: PostgresPoolView,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DockerContainerView {
    id: String,
    image: String,
    names: String,
    status: String,
    state: String,
    created: String,
    ports: String,
    running: bool,
    cpu_perc: String,
    mem_usage: String,
    mem_perc: String,
    /// Raw `docker ps` Labels string: comma-separated `key=value`
    /// pairs. Empty when the container has no labels or the CLI is
    /// old enough not to emit this field. Parsed by the frontend
    /// to extract `com.docker.compose.project` / `.service` etc.
    #[serde(default)]
    labels: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DockerImageView {
    id: String,
    repository: String,
    tag: String,
    size: String,
    created: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DockerVolumeView {
    name: String,
    driver: String,
    mountpoint: String,
    size: String,
    size_bytes: u64,
    links: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DockerNetworkView {
    id: String,
    name: String,
    driver: String,
    scope: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DockerOverview {
    containers: Vec<DockerContainerView>,
    images: Vec<DockerImageView>,
    volumes: Vec<DockerVolumeView>,
    networks: Vec<DockerNetworkView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SftpEntryView {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
    /// POSIX permission bits formatted as the 10-character string
    /// `ls -l` would show (e.g. `-rw-r--r--`, `drwxr-xr-x`). Empty
    /// if the server didn't report them.
    permissions: String,
    /// Last modified time as Unix seconds, or `None` if the server
    /// didn't supply it. The frontend renders this as a relative
    /// "3m", "2d" label.
    modified: Option<u64>,
    /// Owner display string (named user from `/etc/passwd`, falling
    /// back to the numeric uid). Empty when the server didn't
    /// report either.
    owner: String,
    /// Group display string (named group, falling back to the gid).
    /// Empty when neither was reported.
    group: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SftpBrowseState {
    current_path: String,
    entries: Vec<SftpEntryView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerSnapshotView {
    uptime: String,
    load_1: f64,
    load_5: f64,
    load_15: f64,
    mem_total_mb: f64,
    mem_used_mb: f64,
    mem_free_mb: f64,
    swap_total_mb: f64,
    swap_used_mb: f64,
    disk_total: String,
    disk_used: String,
    disk_avail: String,
    disk_use_pct: f64,
    cpu_pct: f64,
    cpu_count: u32,
    proc_count: u32,
    os_label: String,
    /// Bytes-per-second received across non-loopback interfaces.
    /// `-1` until two consecutive probes have run.
    net_rx_bps: f64,
    net_tx_bps: f64,
    top_processes: Vec<ProcessRowView>,
    top_processes_mem: Vec<ProcessRowView>,
    disks: Vec<DiskEntryView>,
    /// Block-device topology from `lsblk -P -b`. Empty when the remote
    /// doesn't have lsblk (BusyBox, macOS) or when the caller asked for
    /// a fast-tier probe that skipped the disk segments.
    block_devices: Vec<BlockDeviceEntryView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiskEntryView {
    filesystem: String,
    fs_type: String,
    total: String,
    used: String,
    avail: String,
    use_pct: f64,
    mountpoint: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BlockDeviceEntryView {
    name: String,
    kname: String,
    pkname: String,
    dev_type: String,
    size_bytes: u64,
    rota: bool,
    tran: String,
    model: String,
    fs_type: String,
    mountpoint: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessRowView {
    pid: String,
    command: String,
    cpu_pct: String,
    mem_pct: String,
    elapsed: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DetectedServiceView {
    name: String,
    version: String,
    status: String,
    port: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LogEventView {
    kind: String, // "stdout", "stderr", "exit"
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TunnelInfoView {
    tunnel_id: String,
    local_host: String,
    local_port: u16,
    remote_host: String,
    remote_port: u16,
    alive: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedSshConnection {
    index: usize,
    name: String,
    host: String,
    port: u16,
    user: String,
    auth_kind: &'static str,
    key_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    group: Option<String>,
    /// DB credentials remembered for this profile. Passwords
    /// are never sent — only a `has_password` flag, resolved
    /// lazily via `db_cred_resolve` at connect time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    databases: Vec<DbCredentialView>,
}

/// Frontend-safe projection of [`DbCredential`]. Passwords are
/// NEVER included — only a `has_password` flag. The typed
/// panel code resolves the actual password via the dedicated
/// `db_cred_resolve` command right before connecting.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DbCredentialView {
    id: String,
    kind: &'static str,
    label: String,
    host: String,
    port: u16,
    user: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    database: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sqlite_path: Option<String>,
    has_password: bool,
    favorite: bool,
    source: DbCredentialSourceView,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DbCredentialSourceView {
    Manual,
    Detected { signature: String },
}

/// Resolved password sidecar for `db_cred_resolve`. The
/// plaintext is local to the Tauri IPC pipe; nothing here
/// should be persisted by the caller.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DbCredentialResolvedView {
    credential: DbCredentialView,
    /// Plaintext password, `None` if passwordless or unresolved.
    password: Option<String>,
}

/// Payload for `db_cred_save` and `db_cred_update`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DbCredentialInput {
    kind: String,
    label: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    user: String,
    #[serde(default)]
    database: Option<String>,
    #[serde(default)]
    sqlite_path: Option<String>,
    #[serde(default)]
    favorite: bool,
    /// Optional signature tying this save to a previous
    /// detection result. Omit for "manual" entries.
    #[serde(default)]
    detection_signature: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DbCredentialPatchInput {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    user: Option<String>,
    /// `Some(Some(""))` clears the field, `Some(Some("x"))`
    /// sets it, absent means "don't touch".
    #[serde(default, deserialize_with = "deserialize_double_option")]
    database: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    sqlite_path: Option<Option<String>>,
    #[serde(default)]
    favorite: Option<bool>,
}

/// Serde helper — distinguish "field absent" from
/// "field present but null" so patches can explicitly clear
/// fields.
fn deserialize_double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer).map(Some)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DetectedDbInstanceView {
    source: String,
    kind: String,
    host: String,
    port: u16,
    label: String,
    image: Option<String>,
    container_id: Option<String>,
    version: Option<String>,
    pid: Option<u32>,
    process_name: Option<String>,
    signature: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DbDetectionReportView {
    instances: Vec<DetectedDbInstanceView>,
    mysql_cli: bool,
    psql_cli: bool,
    redis_cli: bool,
    sqlite_cli: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalSessionInfo {
    session_id: String,
    shell: String,
    cols: u16,
    rows: u16,
}

#[derive(Clone, PartialEq)]
struct SegmentStyle {
    fg: String,
    bg: String,
    bold: bool,
    underline: bool,
    cursor: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalSegment {
    text: String,
    fg: String,
    bg: String,
    bold: bool,
    underline: bool,
    cursor: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalLine {
    segments: Vec<TerminalSegment>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalSnapshot {
    cols: u16,
    rows: u16,
    alive: bool,
    scrollback_len: usize,
    bell_pending: bool,
    lines: Vec<TerminalLine>,
    /// Smart-mode prompt-end position — `[row, col]` of the most
    /// recent OSC 133;B emitted by the shell. `null` when smart mode
    /// is off, the shell hasn't drawn a wrapped prompt yet, or the
    /// user is scrolled into history.
    prompt_end: Option<[u16; 2]>,
    /// Live cursor position. Used by the smart-mode UI to anchor
    /// the Tab popover at the user's cursor in tabs (russh / nested
    /// shells) where OSC 133 isn't emitted, so `prompt_end` would
    /// otherwise leave the popover floating in the viewport.
    cursor_x: u16,
    cursor_y: u16,
    /// `true` when the user is currently inside an editable input
    /// line (between OSC 133;B and OSC 133;C). The frontend mirror
    /// buffer should only accept keystrokes while this is set.
    awaiting_input: bool,
    /// `true` while a TUI is using the alternate screen (vim,
    /// htop, less, tmux). The smart-mode UI hides itself.
    alt_screen: bool,
    /// `true` while a bracketed-paste sequence is in flight.
    /// The smart-mode UI pauses completion / autosuggest.
    bracketed_paste: bool,
}

/// Notify callback invoked by PierTerminal's reader thread. Coalesces
/// "data" events to at most one emission per `TERMINAL_EMIT_MIN_MS`; "exit"
/// events always pass through so the UI learns the child died. Runs on the
/// reader thread — must be cheap and non-blocking (Tauri's `emit` just
/// queues a message).
extern "C" fn tauri_terminal_notify(user_data: *mut c_void, event: u32) {
    if user_data.is_null() {
        return;
    }
    // SAFETY: `user_data` points into a Box<NotifyContext> kept alive by
    // ManagedTerminal for as long as the reader thread runs. We only take
    // a shared reference — never reconstitute or free the Box here.
    let ctx = unsafe { &*(user_data as *const NotifyContext) };

    // Password-prompt signal: a one-shot event so the frontend can
    // arm a "capture the next typed line" window anchored to an
    // actual OpenSSH prompt rather than heuristic keystroke parsing.
    if event == NotifyEvent::SshPasswordPrompt as u32 {
        #[derive(Serialize, Clone)]
        #[serde(rename_all = "camelCase")]
        struct PromptPayload<'a> {
            session_id: &'a str,
        }
        let _ = ctx.app.emit(
            TERMINAL_SSH_PASSWORD_PROMPT_EVENT,
            PromptPayload {
                session_id: &ctx.session_id,
            },
        );
        return;
    }

    // SSH-state transitions use a dedicated event + payload shape so
    // the frontend can update tab state without reparsing keystrokes.
    // Already debounced by the watcher (only fires on change), so no
    // extra throttling is needed here.
    if event == NotifyEvent::SshStateChanged as u32 {
        let target = {
            let state: tauri::State<'_, AppState> = ctx.app.state();
            let sessions = match state.terminals.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            sessions
                .get(&ctx.session_id)
                .and_then(|managed| managed.terminal.current_ssh_target())
        };
        let _ = ctx.app.emit(
            TERMINAL_SSH_STATE_EVENT,
            TerminalSshStatePayload {
                session_id: ctx.session_id.clone(),
                target: target.map(|t| TerminalSshTargetView {
                    host: t.host,
                    user: t.user,
                    port: t.port,
                    identity_path: t.identity_path,
                }),
            },
        );
        return;
    }

    // Every data/exit notification is emitted as it arrives. The
    // frontend's refresh loop is rate-limited by an `inflight` +
    // `dirty` pair, so a storm of notifications coalesces into
    // back-to-back snapshot fetches rather than piling up render
    // work — which is exactly what a previous millisecond-level
    // throttle was trying to achieve. The throttle was dropped
    // because it had no trailing emit: a quick burst of keystrokes
    // followed by a pause left the last characters invisible until
    // the frontend's 1.5s safety timer finally swept, producing
    // the "seconds of lag" users were seeing on casual typing.
    let is_exit = event == NotifyEvent::Exited as u32;
    let _ = ctx.app.emit(
        TERMINAL_EVENT,
        TerminalEventPayload {
            session_id: ctx.session_id.clone(),
            kind: if is_exit { "exit" } else { "data" },
        },
    );
}

/// Allocate a session id + its notify context. The raw pointer into the
/// returned Box is stable (Box is pinned) and must be handed to
/// `PierTerminal::new` as `user_data`; the caller then stores the Box
/// inside `ManagedTerminal` so it outlives the reader thread.
fn allocate_notify_context(
    state: &tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> (String, Box<NotifyContext>) {
    let session_id = format!(
        "term-{}",
        state.next_terminal_id.fetch_add(1, Ordering::Relaxed) + 1
    );
    let ctx = Box::new(NotifyContext {
        app,
        session_id: session_id.clone(),
    });
    (session_id, ctx)
}

fn home_dir() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Expand a user-entered local path into an absolute `PathBuf`.
/// Supports the common `~` / `~/foo` tilde prefix so the SFTP
/// upload / download dialogs accept the same shorthand users would
/// type at a shell.
fn expand_local_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return PathBuf::new();
    }
    if trimmed == "~" {
        return home_dir();
    }
    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        return home_dir().join(rest);
    }
    PathBuf::from(trimmed)
}

fn workspace_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| home_dir());
    // `tauri dev` 把进程 cwd 设为 src-tauri/（tauri.conf.json 所在），
    // basename 透到 UI 即"src-tauri"标签。剥掉这层壳，返回项目根。
    if cwd.file_name().and_then(|s| s.to_str()) == Some("src-tauri") {
        if let Some(parent) = cwd.parent() {
            return parent.to_path_buf();
        }
    }
    cwd
}

fn resolve_existing_path(path: Option<String>) -> PathBuf {
    path.map(PathBuf::from)
        .filter(|candidate| candidate.exists())
        .unwrap_or_else(workspace_root)
}

fn open_git_client(path: Option<String>) -> Result<GitClient, String> {
    let target = resolve_existing_path(path);
    let target_str = target.display().to_string();
    GitClient::open(&target_str).map_err(|error| error.to_string())
}

fn default_shell() -> String {
    #[cfg(windows)]
    {
        return String::from("powershell.exe");
    }

    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| String::from("/bin/zsh"))
    }
}

fn format_size(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let size_f = size as f64;
    if size_f >= GB {
        format!("{:.1} GB", size_f / GB)
    } else if size_f >= MB {
        format!("{:.1} MB", size_f / MB)
    } else if size_f >= KB {
        format!("{:.1} KB", size_f / KB)
    } else {
        format!("{} B", size)
    }
}

fn normalize_ssh_port(port: u16) -> u16 {
    if port == 0 {
        22
    } else {
        port
    }
}

fn normalize_mysql_port(port: u16) -> u16 {
    if port == 0 {
        3306
    } else {
        port
    }
}

fn normalize_redis_port(port: u16) -> u16 {
    if port == 0 {
        6379
    } else {
        port
    }
}

fn normalize_postgres_port(port: u16) -> u16 {
    if port == 0 {
        5432
    } else {
        port
    }
}

fn map_postgres_preview(result: pier_core::services::postgres::QueryResult) -> DataPreview {
    DataPreview {
        columns: result.columns.clone(),
        rows: result
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|cell| cell.unwrap_or_default())
                    .collect()
            })
            .collect(),
        truncated: result.truncated,
    }
}

fn map_postgres_query_result(
    result: pier_core::services::postgres::QueryResult,
) -> QueryExecutionResult {
    QueryExecutionResult {
        columns: result.columns.clone(),
        rows: result
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|cell| cell.unwrap_or_default())
                    .collect()
            })
            .collect(),
        truncated: result.truncated,
        affected_rows: result.affected_rows,
        last_insert_id: result.last_insert_id,
        elapsed_ms: result.elapsed_ms,
    }
}

fn build_ssh_session_from_params(
    host: &str,
    port: u16,
    user: &str,
    auth_mode: &str,
    password: &str,
    key_path: &str,
    key_passphrase: Option<&str>,
) -> Result<SshSession, String> {
    let resolved_host = host.trim();
    let resolved_user = user.trim();
    if resolved_host.is_empty() || resolved_user.is_empty() {
        return Err(String::from("SSH host and user must not be empty."));
    }
    let _ = key_passphrase; // currently only AutoChain consumes it
    let auth = match auth_mode {
        "key" => AuthMethod::PublicKeyFile {
            private_key_path: key_path.to_string(),
            passphrase_credential_id: None,
        },
        "agent" => AuthMethod::Agent,
        // The watcher infers `auto` for any plain `ssh user@host`
        // (no `-i`, no saved profile). We can't tell upfront whether
        // the server wants pubkey, password, or PAM keyboard-
        // interactive — so route through AutoChain, which tries
        // every method we have evidence for on a SINGLE SSH
        // transport (one TCP/kex, N userauth rounds, OpenSSH-style
        // preference order). Threading `key_path` and `password`
        // through means a captured interactive password is no longer
        // dropped silently the way plain `AuthMethod::Auto` did.
        "auto" => AuthMethod::AutoChain {
            explicit_key_path: if key_path.is_empty() {
                None
            } else {
                Some(key_path.to_string())
            },
            password: if password.is_empty() {
                None
            } else {
                Some(password.to_string())
            },
            key_passphrase: key_passphrase
                .filter(|p| !p.is_empty())
                .map(|p| p.to_string()),
        },
        _ => AuthMethod::DirectPassword {
            password: password.to_string(),
        },
    };
    let mut config = SshConfig::new(
        String::new(),
        resolved_host.to_string(),
        resolved_user.to_string(),
    );
    config.port = normalize_ssh_port(port);
    config.auth = auth;
    SshSession::connect_blocking(&config, HostKeyVerifier::default()).map_err(|e| e.to_string())
}

/// Build an SSH session for a panel command, preferring the stored
/// connection record when `saved_index` is set.
///
/// This is the same path `terminal_create_ssh_saved` takes — when a
/// saved connection is in play the stored [`SshConfig`] already carries
/// the right [`AuthMethod`] (KeychainPassword / PublicKeyFile / Agent
/// / DirectPassword), so we don't have to reconstruct it from the
/// param bag. The param fallback remains for ad-hoc connections that
/// were never saved.
fn build_ssh_session_saved_or_params(
    saved_index: Option<usize>,
    host: &str,
    port: u16,
    user: &str,
    auth_mode: &str,
    password: &str,
    key_path: &str,
    key_passphrase: Option<&str>,
) -> Result<SshSession, String> {
    // Prefer the explicit param path whenever the caller hands us a
    // non-empty credential — that's the most recent intent (a
    // captured terminal password, a freshly typed dialog entry, etc.)
    // and bypasses the saved-config's KeychainPassword path which
    // would otherwise blow up with "saved password missing in
    // keychain" the moment the OS credential store is empty or out
    // of sync. Without this short-circuit, panels that route through
    // `build_ssh_session_saved_or_params` (Docker, SFTP, MySQL/PG/
    // Redis tunnels) regress whenever the keychain is missing even
    // though Server Monitor — which uses the params path directly —
    // happily connects.
    let have_param_credential = match auth_mode {
        "password" => !password.is_empty(),
        "key" => !key_path.is_empty(),
        "agent" | "auto" => true,
        _ => false,
    };
    if have_param_credential {
        return build_ssh_session_from_params(
            host,
            port,
            user,
            auth_mode,
            password,
            key_path,
            key_passphrase,
        );
    }

    if let Some(index) = saved_index {
        if let Ok(config) = open_saved_ssh_config(index) {
            return SshSession::connect_blocking(&config, HostKeyVerifier::default())
                .map_err(|e| e.to_string());
        }
    }
    build_ssh_session_from_params(
        host,
        port,
        user,
        auth_mode,
        password,
        key_path,
        key_passphrase,
    )
}

/// Stable key for the SSH session cache. Only the addressing bits,
/// not the secret — rotating a password requires explicit
/// eviction, not a cache miss via key change.
fn sftp_cache_key(host: &str, port: u16, user: &str, auth_mode: &str) -> String {
    format!(
        "{}:{}@{}:{}",
        auth_mode.trim().to_ascii_lowercase(),
        user.trim(),
        host.trim(),
        normalize_ssh_port(port)
    )
}

/// Shared entry point for every panel command that needs an SSH
/// session against a remote host. Returns a cached session when one
/// exists for `(auth_mode, user, host, port)` — which, crucially,
/// includes the handle seeded by `create_ssh_terminal_from_config`
/// whenever the user opens a saved SSH connection tab. This is what
/// wires "all right-panel tools reuse the terminal's SSH channel"
/// into a single place: the Docker, SFTP, monitor, log, and DB
/// panels all route through here and share one russh handshake per
/// target.
///
/// Falls back to `build_ssh_session_saved_or_params` so the path
/// that actually opens a connection honors the saved-config short-
/// circuit (keychain-resolved passwords, key files, agent auth)
/// while still preferring an explicitly-passed credential when the
/// frontend has one in-memory.
fn get_or_open_ssh_session(
    state: &tauri::State<'_, AppState>,
    host: &str,
    port: u16,
    user: &str,
    auth_mode: &str,
    password: &str,
    key_path: &str,
    saved_index: Option<usize>,
) -> Result<Arc<SshSession>, String> {
    // Sweep stale entries opportunistically — keeps the credential
    // cache from accumulating roamed-to hosts forever. Cheap (one
    // mutex + retain over a small map).
    state.ssh_cred_cache.prune_expired();

    // Pull anything we already know about this target from the
    // process-level credential cache. Frontend-supplied scalars
    // win when non-empty (the user is in the middle of changing
    // passwords, etc.); the cache just fills the gaps. This is the
    // path that fixes "system ssh authenticated with password →
    // right-side russh panel can't reach the same host because no
    // password was passed in" — the watcher captured the password
    // into the cache, and AutoChain now finds it here.
    let cred_target = TargetKey::new(host, port, user);
    let (effective_password, effective_key_path, effective_passphrase) = {
        let cached = state.ssh_cred_cache.get(&cred_target);
        let pw = if password.is_empty() {
            cached
                .as_ref()
                .and_then(|c| c.password.clone())
                .unwrap_or_default()
        } else {
            password.to_string()
        };
        let kp = if key_path.is_empty() {
            cached
                .as_ref()
                .and_then(|c| c.key_path.clone())
                .unwrap_or_default()
        } else {
            key_path.to_string()
        };
        let passphrase = cached.and_then(|c| c.key_passphrase);
        (pw, kp, passphrase)
    };
    let password = effective_password.as_str();
    let key_path = effective_key_path.as_str();
    let passphrase = effective_passphrase;

    let key = sftp_cache_key(host, port, user, auth_mode);
    // Fingerprint of the credentials we're about to attempt. Compared
    // against the negative-cache entry so a stale "auth rejected"
    // failure stops gating the moment any input changes — most
    // commonly the watcher just captured an interactive password and
    // the previous `auto + empty` rejection no longer applies.
    let cred_fp = ssh_credential_fingerprint(auth_mode, password, key_path, saved_index);

    // Fast path: cache hit.
    {
        let cache = state
            .sftp_sessions
            .lock()
            .map_err(|_| "ssh session cache poisoned".to_string())?;
        if let Some(existing) = cache.get(&key) {
            return Ok(Arc::clone(existing));
        }
    }

    // Slow path — singleflight with a short-lived negative cache.
    //
    // Grab-or-create the per-key handshake guard, release the map
    // lock immediately (never held across I/O), then:
    //   1. Peek at the negative cache — if we failed recently for
    //      this target, short-circuit with the same error so waiters
    //      don't serially re-attempt a broken connect.
    //   2. Acquire the serialisation gate; only one thread per
    //      target runs the actual handshake at a time.
    //   3. Re-check both the session cache and the negative cache
    //      under the gate: a winner may have just succeeded
    //      (populated the cache) or failed (populated last_fail).
    //
    // We intentionally never hold `sftp_sessions`, `session_init_guards`,
    // or the guard's inner mutexes across `SshSession::connect_blocking`;
    // doing so would serialise unrelated targets through one mutex
    // and promote any slow handshake into a global IPC-thread stall.
    let guard = {
        let mut map = state
            .session_init_guards
            .lock()
            .map_err(|_| "session init map poisoned".to_string())?;
        map.entry(key.clone())
            .or_insert_with(|| Arc::new(HandshakeGuard::new()))
            .clone()
    };

    // Pre-gate negative check — avoids even acquiring the gate if
    // we already know this target is broken with these credentials.
    if let Some(err) = recent_handshake_failure(&guard, cred_fp) {
        return Err(err);
    }

    let _gate = guard
        .gate
        .lock()
        .map_err(|_| "session init gate poisoned".to_string())?;

    // Post-gate re-check: maybe a winner just finished while we
    // were waiting.
    {
        let cache = state
            .sftp_sessions
            .lock()
            .map_err(|_| "ssh session cache poisoned".to_string())?;
        if let Some(existing) = cache.get(&key) {
            return Ok(Arc::clone(existing));
        }
    }
    if let Some(err) = recent_handshake_failure(&guard, cred_fp) {
        return Err(err);
    }

    pier_core::logging::write_event(
        "INFO",
        "ssh.cache",
        &format!("opening fresh SSH session for {}", key),
    );
    let session = match build_ssh_session_saved_or_params(
        saved_index,
        host,
        port,
        user,
        auth_mode,
        password,
        key_path,
        passphrase.as_deref(),
    ) {
        Ok(s) => s,
        Err(e) => {
            // Populate the negative cache so sibling waiters don't
            // each spend another full connect timeout. The
            // fingerprint stamps which credential bag produced the
            // failure — when the bag changes (e.g. password just
            // captured), a fresh handshake gets a fresh shot.
            //
            // Skip when password-mode was requested with no password
            // material at all (param empty + cred-cache empty + no
            // saved profile): the failure is "the caller hadn't
            // supplied credentials yet", not "the credentials are
            // wrong". Stamping it would just delay the legitimate
            // retry the moment the watcher captures the password.
            let attempt_was_credential_starved = auth_mode == "password"
                && password.is_empty()
                && saved_index.is_none();
            if !attempt_was_credential_starved {
                if let Ok(mut slot) = guard.last_fail.lock() {
                    *slot = Some((Instant::now(), e.clone(), cred_fp));
                }
            }
            pier_core::logging::write_event(
                "ERROR",
                "ssh.cache",
                &format!("open failed for {}: {}", key, e),
            );
            return Err(e);
        }
    };
    // Clear any stale failure entry on success.
    if let Ok(mut slot) = guard.last_fail.lock() {
        *slot = None;
    }
    let arc = Arc::new(session);

    state
        .sftp_sessions
        .lock()
        .map_err(|_| "ssh session cache poisoned".to_string())?
        .insert(key, Arc::clone(&arc));
    Ok(arc)
}

/// Peek at the handshake guard's negative-cache slot. Returns the
/// cached error string only when:
///   1. it's still within [`HANDSHAKE_NEGATIVE_CACHE`] (older
///      entries are ignored so a transient network glitch doesn't
///      permanently blackhole a target), AND
///   2. its credential fingerprint matches `current_fp` — i.e. the
///      caller is about to retry with the SAME credentials that
///      already failed. A fingerprint mismatch means something
///      changed (most commonly: the OpenSSH prompt watcher just
///      captured a password that wasn't there last attempt) and
///      the previous rejection no longer applies.
fn recent_handshake_failure(guard: &HandshakeGuard, current_fp: u64) -> Option<String> {
    let slot = guard.last_fail.lock().ok()?;
    let (at, msg, fp) = slot.as_ref()?;
    if *fp != current_fp {
        return None;
    }
    if at.elapsed() <= HANDSHAKE_NEGATIVE_CACHE {
        Some(msg.clone())
    } else {
        None
    }
}

/// Drop the cached session for a target. Called when a panel op
/// fails in a way that suggests the underlying connection has died
/// (server bounced, idle-timed-out keepalive) so the next call
/// opens a fresh one. Paired with `run_with_session_retry` to give
/// panel commands one automatic recovery without surfacing the
/// transient error to the user.
fn evict_ssh_session(
    state: &tauri::State<'_, AppState>,
    host: &str,
    port: u16,
    user: &str,
    auth_mode: &str,
) {
    let key = sftp_cache_key(host, port, user, auth_mode);
    if let Ok(mut cache) = state.sftp_sessions.lock() {
        if cache.remove(&key).is_some() {
            pier_core::logging::write_event(
                "WARN",
                "ssh.cache",
                &format!("evicted cached session {}", key),
            );
        }
    }
    // SSH session death implies SFTP subsystem death — a cached
    // client would just produce a second round-trip failure on the
    // retry path. Same reasoning for the $HOME cache: on reconnect
    // the mount layout may have changed, so re-probe.
    if let Ok(mut cache) = state.sftp_clients.lock() {
        cache.remove(&key);
    }
    if let Ok(mut cache) = state.sftp_home_cache.lock() {
        cache.remove(&key);
    }
}

/// Return the cached SFTP subsystem handle for this target, opening
/// one against `session` if none is cached. Every SFTP command used
/// to call `open_sftp_blocking` itself, paying a `request_subsystem`
/// + `SftpSession::new` round-trip pair on every call; the cache
/// collapses that to once per SSH session.
fn get_or_open_sftp_client(
    state: &tauri::State<'_, AppState>,
    session: &SshSession,
    host: &str,
    port: u16,
    user: &str,
    auth_mode: &str,
) -> Result<SftpClient, String> {
    let key = sftp_cache_key(host, port, user, auth_mode);
    if let Ok(cache) = state.sftp_clients.lock() {
        if let Some(existing) = cache.get(&key) {
            return Ok(existing.clone());
        }
    }
    let client = session.open_sftp_blocking().map_err(|e| e.to_string())?;
    if let Ok(mut cache) = state.sftp_clients.lock() {
        cache.insert(key, client.clone());
    }
    Ok(client)
}

/// Run `op` against the cached session. On a first-attempt failure
/// that looks like a dead session (any `Err(_)`), evict the cache
/// entry and try again with a fresh session. The second failure
/// bubbles up unchanged.
///
/// Covers the common case where russh silently drops a session
/// (server-side idle timeout, network hiccup) and the UI would
/// otherwise show a one-shot error until the next full reconnect.
fn run_with_session_retry<T, F>(
    state: &tauri::State<'_, AppState>,
    host: &str,
    port: u16,
    user: &str,
    auth_mode: &str,
    password: &str,
    key_path: &str,
    saved_index: Option<usize>,
    mut op: F,
) -> Result<T, String>
where
    F: FnMut(&SshSession) -> Result<T, String>,
{
    let mut attempt = 0;
    loop {
        let session = get_or_open_ssh_session(
            state,
            host,
            port,
            user,
            auth_mode,
            password,
            key_path,
            saved_index,
        )?;
        match op(&session) {
            Ok(v) => return Ok(v),
            Err(e) if attempt == 0 => {
                evict_ssh_session(state, host, port, user, auth_mode);
                attempt += 1;
                let _ = e;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Convert raw POSIX permission bits into the 10-character `ls -l`
/// style string. Used to decorate SFTP listings in the inspector.
/// Special bits (setuid / setgid / sticky) are not rendered — the
/// three rwx triplets plus the leading type glyph are enough for
/// the panel's use.
fn format_posix_permissions(bits: u32, is_dir: bool, is_link: bool) -> String {
    let mut out = String::with_capacity(10);
    out.push(if is_link {
        'l'
    } else if is_dir {
        'd'
    } else {
        '-'
    });
    for shift in [6u32, 3, 0] {
        let perm = (bits >> shift) & 0o7;
        out.push(if perm & 0o4 != 0 { 'r' } else { '-' });
        out.push(if perm & 0o2 != 0 { 'w' } else { '-' });
        out.push(if perm & 0o1 != 0 { 'x' } else { '-' });
    }
    out
}

fn build_tunnel_view(tunnel_id: String, tunnel: &ManagedTunnel) -> TunnelInfoView {
    TunnelInfoView {
        tunnel_id,
        local_host: String::from("127.0.0.1"),
        local_port: tunnel.local_port,
        remote_host: tunnel.remote_host.clone(),
        remote_port: tunnel.remote_port,
        alive: tunnel.tunnel.is_alive(),
    }
}

fn choose_active_item(preferred: Option<String>, items: &[String]) -> String {
    let resolved = preferred.unwrap_or_default().trim().to_string();
    if !resolved.is_empty() && items.iter().any(|item| item == &resolved) {
        resolved
    } else {
        items.first().cloned().unwrap_or_default()
    }
}

fn tokenize_command_line(command: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for character in command.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }

        match character {
            '\\' => escaped = true,
            '"' | '\'' => {
                if let Some(active) = quote {
                    if active == character {
                        quote = None;
                    } else {
                        current.push(character);
                    }
                } else {
                    quote = Some(character);
                }
            }
            value if value.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }

    if escaped {
        current.push('\\');
    }
    if quote.is_some() {
        return Err(String::from("unterminated quoted string in command input"));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    if tokens.is_empty() {
        return Err(String::from("command must not be empty"));
    }

    Ok(tokens)
}

fn map_mysql_preview(result: mysql_service::QueryResult) -> DataPreview {
    DataPreview {
        columns: result.columns,
        rows: result
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|cell| cell.unwrap_or_else(|| String::from("NULL")))
                    .collect()
            })
            .collect(),
        truncated: result.truncated,
    }
}

fn map_mysql_query_result(result: mysql_service::QueryResult) -> QueryExecutionResult {
    QueryExecutionResult {
        columns: result.columns,
        rows: result
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|cell| cell.unwrap_or_else(|| String::from("NULL")))
                    .collect()
            })
            .collect(),
        truncated: result.truncated,
        affected_rows: result.affected_rows,
        last_insert_id: result.last_insert_id,
        elapsed_ms: result.elapsed_ms,
    }
}

fn map_sqlite_preview(
    result: pier_core::services::sqlite::SqliteQueryResult,
) -> Option<DataPreview> {
    if result.error.is_some() {
        None
    } else {
        Some(DataPreview {
            columns: result.columns,
            rows: result.rows,
            truncated: false,
        })
    }
}

fn map_sqlite_query_result(
    result: pier_core::services::sqlite::SqliteQueryResult,
) -> Result<QueryExecutionResult, String> {
    if let Some(error) = result.error {
        Err(error)
    } else {
        Ok(QueryExecutionResult {
            columns: result.columns,
            rows: result.rows,
            truncated: false,
            affected_rows: result.affected_rows.max(0) as u64,
            last_insert_id: None,
            elapsed_ms: result.elapsed_ms,
        })
    }
}

fn map_redis_details(details: pier_core::services::redis::KeyDetails) -> RedisKeyView {
    RedisKeyView {
        key: details.key,
        kind: details.kind,
        length: details.length,
        ttl_seconds: details.ttl_seconds,
        encoding: details.encoding,
        preview: details.preview,
        preview_truncated: details.preview_truncated,
    }
}

fn slugify_for_credential(value: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn make_credential_id(host: &str, user: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let host_slug = slugify_for_credential(host);
    let user_slug = slugify_for_credential(user);
    format!("pier-x.ssh.{host_slug}.{user_slug}.{millis}")
}

fn auth_kind(auth: &AuthMethod) -> &'static str {
    match auth {
        AuthMethod::Agent => "agent",
        // AutoChain is just `Auto` plus opportunistic credential
        // reuse — same external surface from the saved-config /
        // cache-key perspective, so it stamps as "auto".
        AuthMethod::Auto | AuthMethod::AutoChain { .. } => "auto",
        AuthMethod::PublicKeyFile { .. } => "key",
        AuthMethod::KeychainPassword { .. } | AuthMethod::DirectPassword { .. } => "password",
    }
}

fn delete_auth_credentials(auth: &AuthMethod) -> Result<(), String> {
    match auth {
        AuthMethod::KeychainPassword { credential_id } => {
            credentials::delete(credential_id).map_err(|error| error.to_string())
        }
        AuthMethod::PublicKeyFile {
            passphrase_credential_id: Some(credential_id),
            ..
        } => credentials::delete(credential_id).map_err(|error| error.to_string()),
        _ => Ok(()),
    }
}

fn auth_credential_id(auth: &AuthMethod) -> Option<&str> {
    match auth {
        AuthMethod::KeychainPassword { credential_id } => Some(credential_id.as_str()),
        AuthMethod::PublicKeyFile {
            passphrase_credential_id: Some(credential_id),
            ..
        } => Some(credential_id.as_str()),
        _ => None,
    }
}

fn map_saved_connection(index: usize, config: &SshConfig) -> SavedSshConnection {
    SavedSshConnection {
        index,
        name: config.name.clone(),
        host: config.host.clone(),
        port: config.port,
        user: config.user.clone(),
        auth_kind: auth_kind(&config.auth),
        key_path: match &config.auth {
            AuthMethod::PublicKeyFile {
                private_key_path, ..
            } => private_key_path.clone(),
            _ => String::new(),
        },
        group: config
            .group
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        databases: config.databases.iter().map(map_db_credential).collect(),
    }
}

fn db_kind_str(k: DbKind) -> &'static str {
    match k {
        DbKind::Mysql => "mysql",
        DbKind::Postgres => "postgres",
        DbKind::Redis => "redis",
        DbKind::Sqlite => "sqlite",
    }
}

fn parse_db_kind(s: &str) -> Result<DbKind, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "mysql" => Ok(DbKind::Mysql),
        "postgres" | "postgresql" => Ok(DbKind::Postgres),
        "redis" => Ok(DbKind::Redis),
        "sqlite" => Ok(DbKind::Sqlite),
        other => Err(format!("unknown db kind: {other}")),
    }
}

fn map_db_credential(c: &DbCredential) -> DbCredentialView {
    DbCredentialView {
        id: c.id.clone(),
        kind: db_kind_str(c.kind),
        label: c.label.clone(),
        host: c.host.clone(),
        port: c.port,
        user: c.user.clone(),
        database: c.database.clone(),
        sqlite_path: c.sqlite_path.clone(),
        // `password_available` consults the process-local plaintext
        // cache as well, so Direct-variant creds whose serde-skipped
        // password was lost through a YAML round-trip still report
        // `hasPassword=true` while the app is running. Matters for
        // the frontend's "Saved password unavailable" fallback.
        has_password: pier_core::connections::password_available(c),
        favorite: c.favorite,
        source: match &c.source {
            DbCredentialSource::Manual => DbCredentialSourceView::Manual,
            DbCredentialSource::Detected { signature } => DbCredentialSourceView::Detected {
                signature: signature.clone(),
            },
        },
    }
}

fn map_detected_db_instance(d: DetectedDbInstance) -> DetectedDbInstanceView {
    let source = match d.source {
        pier_core::ssh::db_detect::DetectionSource::Docker => "docker",
        pier_core::ssh::db_detect::DetectionSource::Systemd => "systemd",
        pier_core::ssh::db_detect::DetectionSource::Direct => "direct",
    };
    let kind = match d.kind {
        pier_core::ssh::db_detect::DetectedDbKind::Mysql => "mysql",
        pier_core::ssh::db_detect::DetectedDbKind::Postgres => "postgres",
        pier_core::ssh::db_detect::DetectedDbKind::Redis => "redis",
    };
    DetectedDbInstanceView {
        source: source.to_string(),
        kind: kind.to_string(),
        host: d.host,
        port: d.port,
        label: d.label,
        image: d.metadata.image,
        container_id: d.metadata.container_id,
        version: d.metadata.version,
        pid: d.metadata.pid,
        process_name: d.metadata.process_name,
        signature: d.signature,
    }
}

fn map_db_detection_report(r: DbDetectionReport) -> DbDetectionReportView {
    DbDetectionReportView {
        instances: r
            .instances
            .into_iter()
            .map(map_detected_db_instance)
            .collect(),
        mysql_cli: r.clis.mysql,
        psql_cli: r.clis.psql,
        redis_cli: r.clis.redis_cli,
        sqlite_cli: r.clis.sqlite3,
    }
}

fn map_resolved_credential(r: ResolvedDbCredential) -> DbCredentialResolvedView {
    let ResolvedDbCredential {
        credential,
        password,
    } = r;
    DbCredentialResolvedView {
        credential: map_db_credential(&credential),
        password,
    }
}

fn build_manual_ssh_config(
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: Option<String>,
    key_path: Option<String>,
) -> Result<SshConfig, String> {
    let resolved_host = host.trim();
    let resolved_user = user.trim();

    if resolved_host.is_empty() || resolved_user.is_empty() {
        return Err(String::from("SSH host and user must not be empty."));
    }

    let mut config = SshConfig::new(
        format!("{resolved_user}@{resolved_host}"),
        resolved_host,
        resolved_user,
    );
    config.port = normalize_ssh_port(port);
    config.auth = match auth_mode.trim() {
        "agent" => AuthMethod::Agent,
        "auto" => AuthMethod::Auto,
        "key" => {
            let resolved_key_path = key_path
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| String::from("SSH key path must not be empty."))?;
            AuthMethod::PublicKeyFile {
                private_key_path: resolved_key_path,
                passphrase_credential_id: None,
            }
        }
        _ => {
            let resolved_password = password
                .filter(|value| !value.is_empty())
                .ok_or_else(|| String::from("SSH password must not be empty."))?;
            AuthMethod::DirectPassword {
                password: resolved_password,
            }
        }
    };

    Ok(config)
}

fn open_saved_ssh_config(index: usize) -> Result<SshConfig, String> {
    let store = ConnectionStore::load_default().map_err(|error| error.to_string())?;
    store
        .connections
        .get(index)
        .cloned()
        .ok_or_else(|| format!("unknown saved SSH connection: {}", index))
}

fn store_terminal_session(
    state: tauri::State<'_, AppState>,
    session_id: String,
    notify_ctx: Box<NotifyContext>,
    terminal: PierTerminal,
    shell: String,
    cols: u16,
    rows: u16,
) -> Result<TerminalSessionInfo, String> {
    let mut sessions = state
        .terminals
        .lock()
        .map_err(|_| String::from("terminal state poisoned"))?;
    sessions.insert(
        session_id.clone(),
        ManagedTerminal {
            terminal,
            _notify_ctx: notify_ctx,
        },
    );

    Ok(TerminalSessionInfo {
        session_id,
        shell,
        cols,
        rows,
    })
}

fn create_ssh_terminal_from_config(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    config: SshConfig,
    cols: u16,
    rows: u16,
) -> Result<TerminalSessionInfo, String> {
    let resolved_cols = cols.max(40);
    let resolved_rows = rows.max(12);
    let shell = format!("ssh:{}@{}:{}", config.user, config.host, config.port);

    // Reuse a cached SSH session when one already exists for this
    // target — opening a new terminal tab against a host that an
    // SFTP / Docker / Software panel already authenticated to should
    // NOT trigger a second russh handshake. Doing so:
    //   * doubles handshake CPU + network round-trips
    //   * races with sshd's MaxStartups limit (default 10:30:100),
    //     which on a busy host throttles new auth attempts and made
    //     the new terminal stall on "Launching shell..."
    //   * pointlessly invalidates the per-target singleflight gate
    //     in `get_or_open_ssh_session`
    //
    // Key format must match `sftp_cache_key`.
    let auth_mode_key = match &config.auth {
        AuthMethod::Agent => "agent",
        AuthMethod::Auto | AuthMethod::AutoChain { .. } => "auto",
        AuthMethod::PublicKeyFile { .. } => "key",
        _ => "password",
    };
    let cache_key = sftp_cache_key(&config.host, config.port, &config.user, auth_mode_key);
    let cached = {
        state
            .sftp_sessions
            .lock()
            .ok()
            .and_then(|cache| cache.get(&cache_key).cloned())
    };
    let session: SshSession = if let Some(arc) = cached {
        // Clone yields a second Arc-handle to the same russh
        // connection; the cache keeps the original alive so closing
        // this terminal tab won't drop the panel-side session.
        SshSession::clone(&*arc)
    } else {
        let fresh = SshSession::connect_blocking(&config, HostKeyVerifier::default())
            .map_err(|error| error.to_string())?;
        if let Ok(mut cache) = state.sftp_sessions.lock() {
            cache.insert(cache_key, Arc::new(fresh.clone()));
        }
        fresh
    };

    let (session_id, mut notify_ctx) = allocate_notify_context(&state, app);
    let user_data = &mut *notify_ctx as *mut NotifyContext as *mut c_void;

    let pty = session
        .open_shell_channel_blocking(resolved_cols, resolved_rows)
        .map_err(|error| error.to_string())?;
    let terminal = PierTerminal::with_pty(
        Box::new(pty),
        resolved_cols,
        resolved_rows,
        tauri_terminal_notify as NotifyFn,
        user_data,
    )
    .map_err(|error| error.to_string())?;

    // Push a one-shot prompt-hook line into the remote shell so it
    // starts emitting OSC 7 (cwd) and OSC 133 (prompt sentinels) on
    // every subsequent prompt. Without this the smart Tab popover
    // can't tell where the user has `cd`-ed and falls back to the
    // SFTP root (usually $HOME) which isn't useful.
    //
    // Mirrors the wezterm / VTE `shell-integration.sh` pattern: a
    // shell-detecting one-liner that wires up bash and zsh and is
    // a silent no-op under fish/dash/sh. Sent with a leading space
    // so `HISTCONTROL=ignorespace` (default in modern distros)
    // skips it from history.
    let _ = terminal.write(&pier_core::terminal::remote_init_payload());

    store_terminal_session(
        state,
        session_id,
        notify_ctx,
        terminal,
        shell,
        resolved_cols,
        resolved_rows,
    )
}

/// Emit a semantic color tag so the frontend can remap to the user's
/// selected theme palette.
///
/// Formats:
/// - `""` → use the theme's default foreground / background (inherit)
/// - `"ansi:N"` → indexed ANSI color (0..=255); 0..=15 are mapped to the
///   theme's 16-color palette, 16..=255 go through the fixed 256-color
///   cube approximation.
/// - `"#rrggbb"` → truecolor, passed through as-is.
fn render_terminal_color(color: Color, _foreground: bool) -> String {
    match color {
        Color::Default => String::new(),
        Color::Indexed(index) => format!("ansi:{index}"),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}

// ANSI palette mapping moved to the frontend (src/panels/TerminalPanel.tsx
// `resolveTerminalColor`) so the user-selected terminal theme can be
// applied to the 16 basic ANSI colors.

fn resolve_segment_style(cell: &Cell, is_cursor: bool) -> SegmentStyle {
    let mut fg = render_terminal_color(cell.fg, true);
    let mut bg = render_terminal_color(cell.bg, false);
    if cell.reverse {
        std::mem::swap(&mut fg, &mut bg);
    }
    SegmentStyle {
        fg,
        bg,
        bold: cell.bold,
        underline: cell.underline,
        cursor: is_cursor,
    }
}

fn build_terminal_lines(
    snapshot: &pier_core::terminal::GridSnapshot,
    alive: bool,
) -> Vec<TerminalLine> {
    let width = snapshot.cols as usize;
    snapshot
        .cells
        .chunks(width)
        .enumerate()
        .map(|(row_index, row)| {
            let mut segments = Vec::new();
            let mut current_style: Option<SegmentStyle> = None;
            let mut current_text = String::new();

            for (col_index, cell) in row.iter().enumerate() {
                let is_cursor = alive
                    && row_index == snapshot.cursor_y as usize
                    && col_index == snapshot.cursor_x as usize;
                let next_style = resolve_segment_style(cell, is_cursor);
                let next_char = if cell.ch == '\0' { ' ' } else { cell.ch };

                if current_style.as_ref() == Some(&next_style) {
                    current_text.push(next_char);
                    continue;
                }

                if let Some(style) = current_style.take() {
                    segments.push(TerminalSegment {
                        text: std::mem::take(&mut current_text),
                        fg: style.fg,
                        bg: style.bg,
                        bold: style.bold,
                        underline: style.underline,
                        cursor: style.cursor,
                    });
                }

                current_text.push(next_char);
                current_style = Some(next_style);
            }

            if let Some(style) = current_style.take() {
                segments.push(TerminalSegment {
                    text: current_text,
                    fg: style.fg,
                    bg: style.bg,
                    bold: style.bold,
                    underline: style.underline,
                    cursor: style.cursor,
                });
            }

            TerminalLine { segments }
        })
        .collect()
}

#[tauri::command]
fn core_info() -> CoreInfo {
    CoreInfo {
        version: pier_core::VERSION.to_string(),
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        ui_target: "tauri",
        home_dir: home_dir().display().to_string(),
        workspace_root: workspace_root().display().to_string(),
        platform: if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            "linux"
        },
        default_shell: default_shell(),
        services: vec!["terminal", "ssh", "git", "mysql", "sqlite", "redis"],
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SshKeyInfo {
    /// Absolute path to the private key file.
    path: String,
    /// First-line comment from the matching `.pub` file
    /// (e.g. "user@host"); empty if no .pub or unreadable.
    comment: String,
    /// Algorithm token from the .pub file (e.g. "ssh-ed25519",
    /// "ssh-rsa", "ecdsa-sha2-nistp256"); empty if unknown.
    kind: String,
    /// Octal mode of the private key file (e.g. "600"); empty when
    /// permissions can't be read (Windows or transient FS errors).
    mode: String,
    /// Whether the matching `<path>.pub` exists on disk.
    has_public: bool,
}

/// Read-only inventory of `~/.ssh/id_*` private keys. Surfaced in
/// Settings → SSH keys. Skips known_hosts, config, agent socket, and
/// `.pub` files themselves — only paired private keys make the cut.
/// Generation / agent-load are deferred (security-sensitive
/// platform-specific work).
#[tauri::command]
fn ssh_keys_list() -> Result<Vec<SshKeyInfo>, String> {
    let ssh_dir = home_dir().join(".ssh");
    if !ssh_dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries = fs::read_dir(&ssh_dir).map_err(|e| format!("read ~/.ssh failed: {}", e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        // Match `id_*` private keys. Skip `.pub`, known_hosts, config,
        // authorized_keys, ssh-agent socket. We intentionally do NOT
        // broaden to "any private-looking file" because users sometimes
        // drop misc files in ~/.ssh — false positives in a settings UI
        // are confusing.
        if !name.starts_with("id_") {
            continue;
        }
        if name.ends_with(".pub") {
            continue;
        }
        let pub_path = path.with_extension("pub");
        let has_public = pub_path.exists();

        let mut kind = String::new();
        let mut comment = String::new();
        if has_public {
            if let Ok(text) = fs::read_to_string(&pub_path) {
                if let Some(first_line) = text.lines().next() {
                    let mut parts = first_line.split_whitespace();
                    if let Some(algo) = parts.next() {
                        kind = algo.to_string();
                    }
                    let _b64 = parts.next();
                    let rest: Vec<&str> = parts.collect();
                    if !rest.is_empty() {
                        comment = rest.join(" ");
                    }
                }
            }
        }

        let mode = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::metadata(&path)
                    .map(|m| format!("{:o}", m.permissions().mode() & 0o777))
                    .unwrap_or_default()
            }
            #[cfg(not(unix))]
            {
                String::new()
            }
        };

        out.push(SshKeyInfo {
            path: path.display().to_string(),
            comment,
            kind,
            mode,
            has_public,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ComponentInfo {
    name: &'static str,
    role: &'static str,
    version: &'static str,
}

/// Static snapshot of major dependencies powering Pier-X. Surfaced
/// in Settings → About → Components. Update when bumping versions
/// in `src-tauri/Cargo.toml`, `pier-core/Cargo.toml`, or
/// `package.json` — there is no auto-derive.
#[tauri::command]
fn core_components_info() -> Vec<ComponentInfo> {
    vec![
        ComponentInfo {
            name: "Tauri",
            role: "App runtime",
            version: "2.x",
        },
        ComponentInfo {
            name: "russh",
            role: "SSH client",
            version: "0.60",
        },
        ComponentInfo {
            name: "git2",
            role: "Git bindings",
            version: "0.19",
        },
        ComponentInfo {
            name: "tokio",
            role: "Async runtime",
            version: "1.x",
        },
        ComponentInfo {
            name: "React",
            role: "UI framework",
            version: "19.x",
        },
        ComponentInfo {
            name: "Vite",
            role: "Frontend build",
            version: "7.x",
        },
        ComponentInfo {
            name: "@xterm/xterm",
            role: "Terminal renderer",
            version: "6.x",
        },
        ComponentInfo {
            name: "CodeMirror",
            role: "SFTP file editor",
            version: "6.x",
        },
    ]
}

#[tauri::command]
fn list_directory(path: Option<String>) -> Result<Vec<FileEntry>, String> {
    let target = resolve_existing_path(path);

    let mut entries: Vec<FileEntry> = fs::read_dir(&target)
        .map_err(|error| format!("Failed to read {}: {}", target.display(), error))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            let kind = if metadata.is_dir() {
                "directory"
            } else {
                "file"
            };
            let file_size = metadata.len();
            let modified_ts = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let modified = if modified_ts > 0 {
                // Format as MM-dd HH:mm
                let secs = modified_ts as i64;
                let days = secs / 86400;
                let time_of_day = secs % 86400;
                let hours = time_of_day / 3600;
                let minutes = (time_of_day % 3600) / 60;
                // Approximate month-day (good enough for display)
                let epoch_days = days + 719468; // days from year 0
                let era = epoch_days / 146097;
                let doe = epoch_days - era * 146097;
                let yoe = (doe - doe / 1461 + doe / 36524 - doe / 146097) / 365;
                let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
                let mp = (5 * doy + 2) / 153;
                let d = doy - (153 * mp + 2) / 5 + 1;
                let m = if mp < 10 { mp + 3 } else { mp - 9 };
                format!("{:02}-{:02} {:02}:{:02}", m, d, hours, minutes)
            } else {
                String::new()
            };
            Some(FileEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                path: path.display().to_string(),
                kind,
                size: file_size,
                size_label: if metadata.is_dir() {
                    String::from("--")
                } else {
                    format_size(file_size)
                },
                modified,
                modified_ts,
            })
        })
        .collect();

    entries.sort_by(|left, right| {
        let left_dir = left.kind == "directory";
        let right_dir = right.kind == "directory";
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });

    Ok(entries)
}

/// Enumerate top-level volumes so the sidebar can render a "This PC"
/// view above drive roots.
///
/// On Windows we call `GetLogicalDrives` (kernel32) — a bitmask of
/// currently-mounted drives — instead of doing `.exists()` probes per
/// letter. The probing approach blocked for seconds on disconnected
/// network drives, stale DVD drives, or non-present floppy (`A:\`),
/// because `.exists()` issues an `open()` the driver handles
/// synchronously. The bitmask call returns instantly and only reports
/// drives the OS actually has mounted.
///
/// On other platforms this yields `/` so the frontend can reuse the
/// same rendering path without special-casing.
#[tauri::command]
fn list_drives() -> Vec<FileEntry> {
    let mut drives: Vec<FileEntry> = Vec::new();
    #[cfg(windows)]
    {
        // kernel32!GetLogicalDrives — bit N set means drive letter
        // (b'A' + N) is mounted. Returns 0 on failure, which we treat
        // as "no drives" rather than an error so the UI still renders.
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLogicalDrives() -> u32;
        }
        let mask = unsafe { GetLogicalDrives() };
        for i in 0u8..26 {
            if mask & (1u32 << i) == 0 {
                continue;
            }
            let letter = b'A' + i;
            let root = format!("{}:\\", letter as char);
            drives.push(FileEntry {
                name: format!("{}:", letter as char),
                path: root,
                kind: "directory",
                size: 0,
                size_label: String::from("--"),
                modified: String::new(),
                modified_ts: 0,
            });
        }
    }
    #[cfg(not(windows))]
    {
        drives.push(FileEntry {
            name: String::from("/"),
            path: String::from("/"),
            kind: "directory",
            size: 0,
            size_label: String::from("--"),
            modified: String::new(),
            modified_ts: 0,
        });
    }
    drives
}

// ── Local file mutation commands ─────────────────────────────────
//
// These mirror the SFTP panel's right-click actions for the local
// sidebar: create / rename / remove / make-dir. Paths travel as
// strings and are passed through `std::fs` directly — callers on the
// frontend side are responsible for displaying errors via the
// localized error bar, same pattern as SFTP.

#[tauri::command]
fn local_create_file(path: String) -> Result<(), String> {
    let p = std::path::PathBuf::from(&path);
    if p.exists() {
        return Err(format!("{} already exists", p.display()));
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&p)
        .map(|_| ())
        .map_err(|e| format!("Failed to create {}: {}", p.display(), e))
}

#[tauri::command]
fn local_create_dir(path: String) -> Result<(), String> {
    let p = std::path::PathBuf::from(&path);
    if p.exists() {
        return Err(format!("{} already exists", p.display()));
    }
    std::fs::create_dir(&p).map_err(|e| format!("Failed to create {}: {}", p.display(), e))
}

#[tauri::command]
fn local_rename(from: String, to: String) -> Result<(), String> {
    let src = std::path::PathBuf::from(&from);
    let dst = std::path::PathBuf::from(&to);
    if !src.exists() {
        return Err(format!("{} does not exist", src.display()));
    }
    if dst.exists() {
        return Err(format!("{} already exists", dst.display()));
    }
    std::fs::rename(&src, &dst).map_err(|e| format!("Failed to rename: {}", e))
}

#[tauri::command]
fn local_remove(path: String, is_dir: bool) -> Result<(), String> {
    let p = std::path::PathBuf::from(&path);
    if is_dir {
        // Recursive — same mental model as SFTP's remote remove, which
        // also deletes directory trees in one call.
        std::fs::remove_dir_all(&p).map_err(|e| format!("Failed to remove {}: {}", p.display(), e))
    } else {
        std::fs::remove_file(&p).map_err(|e| format!("Failed to remove {}: {}", p.display(), e))
    }
}

#[tauri::command]
fn git_overview(path: Option<String>) -> Result<GitOverview, String> {
    let client = open_git_client(path)?;
    let branch = client.branch_info().map_err(|error| error.to_string())?;
    let changes = client.status().map_err(|error| error.to_string())?;

    let staged_count = changes.iter().filter(|change| change.staged).count();
    let unstaged_count = changes.len().saturating_sub(staged_count);
    let change_entries = changes
        .iter()
        .take(18)
        .map(|change| GitChangeEntry {
            path: change.path.clone(),
            status: change.status.code().to_string(),
            staged: change.staged,
        })
        .collect();

    Ok(GitOverview {
        repo_path: client.repo_path().display().to_string(),
        branch_name: branch.name,
        tracking: branch.tracking,
        ahead: branch.ahead,
        behind: branch.behind,
        is_clean: changes.is_empty(),
        staged_count,
        unstaged_count,
        changes: change_entries,
    })
}

#[tauri::command]
fn git_diff(
    path: Option<String>,
    file_path: String,
    staged: bool,
    untracked: bool,
) -> Result<String, String> {
    let client = open_git_client(path)?;
    if untracked {
        client
            .diff_untracked(&file_path)
            .map_err(|error| error.to_string())
    } else {
        client
            .diff(&file_path, staged)
            .map_err(|error| error.to_string())
    }
}

#[tauri::command]
fn git_stage_paths(path: Option<String>, paths: Vec<String>) -> Result<(), String> {
    let client = open_git_client(path)?;
    client.stage(&paths).map_err(|error| error.to_string())
}

#[tauri::command]
fn git_unstage_paths(path: Option<String>, paths: Vec<String>) -> Result<(), String> {
    let client = open_git_client(path)?;
    client.unstage(&paths).map_err(|error| error.to_string())
}

#[tauri::command]
fn git_stage_all(path: Option<String>) -> Result<(), String> {
    let client = open_git_client(path)?;
    client.stage_all().map_err(|error| error.to_string())
}

#[tauri::command]
fn git_unstage_all(path: Option<String>) -> Result<(), String> {
    let client = open_git_client(path)?;
    client.unstage_all().map_err(|error| error.to_string())
}

#[tauri::command]
fn git_discard_paths(path: Option<String>, paths: Vec<String>) -> Result<(), String> {
    let client = open_git_client(path)?;
    client.discard(&paths).map_err(|error| error.to_string())
}

#[tauri::command]
fn git_commit(
    path: Option<String>,
    message: String,
    signoff: Option<bool>,
    amend: Option<bool>,
    sign: Option<bool>,
) -> Result<String, String> {
    let client = open_git_client(path)?;
    client
        .commit_with(
            message.trim(),
            signoff.unwrap_or(false),
            amend.unwrap_or(false),
            sign.unwrap_or(false),
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn git_branch_list(path: Option<String>) -> Result<Vec<String>, String> {
    let client = open_git_client(path)?;
    client.branch_list().map_err(|error| error.to_string())
}

#[tauri::command]
fn git_checkout_branch(path: Option<String>, name: String) -> Result<String, String> {
    let client = open_git_client(path)?;
    client
        .checkout_branch(name.trim())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn git_recent_commits(
    path: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<GitCommitEntry>, String> {
    let client = open_git_client(path)?;
    let resolved_limit = limit.unwrap_or(8).clamp(1, 16);
    let commits = match client.log(resolved_limit) {
        Ok(entries) => entries,
        Err(error) => {
            let message = error.to_string();
            if message.contains("does not have any commits yet") {
                Vec::new()
            } else {
                return Err(message);
            }
        }
    };

    Ok(commits.into_iter().map(map_commit_entry).collect())
}

fn map_commit_entry(entry: CommitInfo) -> GitCommitEntry {
    GitCommitEntry {
        hash: entry.hash,
        short_hash: entry.short_hash,
        message: entry.message,
        author: entry.author,
        relative_date: entry.relative_date,
        refs: entry.refs,
    }
}

#[tauri::command]
fn git_push(path: Option<String>) -> Result<String, String> {
    let client = open_git_client(path)?;
    client.push().map_err(|error| error.to_string())
}

#[tauri::command]
fn git_pull(path: Option<String>) -> Result<String, String> {
    let client = open_git_client(path)?;
    client.pull().map_err(|error| error.to_string())
}

#[tauri::command]
fn git_stash_list(path: Option<String>) -> Result<Vec<GitStashEntry>, String> {
    let client = open_git_client(path)?;
    client
        .stash_list()
        .map(|entries| entries.into_iter().map(map_stash_entry).collect())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn git_stash_push(path: Option<String>, message: String) -> Result<String, String> {
    let client = open_git_client(path)?;
    client
        .stash_push(message.trim())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn git_stash_apply(path: Option<String>, index: String) -> Result<String, String> {
    let client = open_git_client(path)?;
    client
        .stash_apply(index.trim())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn git_stash_pop(path: Option<String>, index: String) -> Result<String, String> {
    let client = open_git_client(path)?;
    client
        .stash_pop(index.trim())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn git_stash_drop(path: Option<String>, index: String) -> Result<String, String> {
    let client = open_git_client(path)?;
    client
        .stash_drop(index.trim())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn git_stash_reword(
    path: Option<String>,
    index: String,
    message: String,
) -> Result<String, String> {
    let client = open_git_client(path)?;
    client
        .stash_reword(index.trim(), message.trim())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn git_unpushed_commits(path: Option<String>) -> Result<Vec<UnpushedCommit>, String> {
    let client = open_git_client(path)?;
    client
        .unpushed_commits()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn ssh_connections_list() -> Result<Vec<SavedSshConnection>, String> {
    let store = ConnectionStore::load_default().map_err(|error| error.to_string())?;
    Ok(store
        .connections
        .iter()
        .enumerate()
        .map(|(index, config)| map_saved_connection(index, config))
        .collect())
}

#[tauri::command]
fn ssh_connection_save(
    name: String,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: Option<String>,
    key_path: Option<String>,
    group: Option<String>,
) -> Result<(), String> {
    let resolved_host = host.trim();
    let resolved_user = user.trim();
    let resolved_name = name.trim();

    if resolved_host.is_empty() || resolved_user.is_empty() {
        return Err(String::from("SSH host and user must not be empty."));
    }

    let mut config = SshConfig::new(
        if resolved_name.is_empty() {
            format!("{resolved_user}@{resolved_host}")
        } else {
            resolved_name.to_string()
        },
        resolved_host,
        resolved_user,
    );
    config.port = normalize_ssh_port(port);
    config.auth = match auth_mode.trim() {
        "agent" => AuthMethod::Agent,
        "key" => {
            let resolved_key_path = key_path
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| String::from("SSH key path must not be empty."))?;
            AuthMethod::PublicKeyFile {
                private_key_path: resolved_key_path,
                passphrase_credential_id: None,
            }
        }
        _ => {
            let resolved_password = password
                .filter(|value| !value.is_empty())
                .ok_or_else(|| String::from("SSH password must not be empty."))?;
            let credential_id = make_credential_id(resolved_host, resolved_user);
            // Probe the keyring with a write+read round-trip. On
            // backends that silently drop writes (Windows under
            // certain group policies, Linux without an unlocked
            // secret-service) we can't trust the credential to be
            // there on the next launch — fall back to storing the
            // password in the SshConfig itself as
            // `DirectPassword`. Less secure (the connections file
            // is plain-text JSON), but at least the saved
            // connection actually works.
            let keychain_ok = credentials::set_and_verify(&credential_id, &resolved_password)
                .map_err(|error| error.to_string())?;
            if keychain_ok {
                AuthMethod::KeychainPassword { credential_id }
            } else {
                AuthMethod::DirectPassword {
                    password: resolved_password,
                }
            }
        }
    };

    config.group = group
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut store = ConnectionStore::load_default().map_err(|error| error.to_string())?;
    store.add(config);
    store.save_default().map_err(|error| error.to_string())
}

#[tauri::command]
fn ssh_connection_delete(index: usize) -> Result<(), String> {
    let mut store = ConnectionStore::load_default().map_err(|error| error.to_string())?;
    let removed = store
        .remove(index)
        .ok_or_else(|| format!("unknown saved SSH connection: {}", index))?;
    store.save_default().map_err(|error| error.to_string())?;
    delete_auth_credentials(&removed.auth)
}

/// Resolve the stored password for a saved SSH connection.
/// Returns an empty string for non-password auth (agent/key) or when the
/// keychain has no entry. Only held in-memory on the frontend for the
/// session's lifetime; never persisted to localStorage.
#[tauri::command]
fn ssh_connection_resolve_password(index: usize) -> Result<String, String> {
    let store = ConnectionStore::load_default().map_err(|error| error.to_string())?;
    let conn = store
        .connections
        .get(index)
        .ok_or_else(|| format!("unknown saved SSH connection: {}", index))?;
    match &conn.auth {
        AuthMethod::KeychainPassword { credential_id } => {
            match credentials::get(credential_id).map_err(|error| error.to_string())? {
                Some(password) => Ok(password),
                None => Ok(String::new()),
            }
        }
        // Saved with the keychain-fallback path: hand the password
        // straight from the SshConfig so the frontend can prime
        // tab.sshPassword and right-side panels can authenticate.
        AuthMethod::DirectPassword { password } => Ok(password.clone()),
        _ => Ok(String::new()),
    }
}

#[tauri::command]
fn ssh_connection_update(
    index: usize,
    name: String,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: Option<String>,
    key_path: Option<String>,
    group: Option<String>,
) -> Result<(), String> {
    let resolved_host = host.trim();
    let resolved_user = user.trim();
    let resolved_name = name.trim();

    if resolved_host.is_empty() || resolved_user.is_empty() {
        return Err(String::from("SSH host and user must not be empty."));
    }

    let mut store = ConnectionStore::load_default().map_err(|error| error.to_string())?;
    let existing = store
        .connections
        .get(index)
        .cloned()
        .ok_or_else(|| format!("unknown saved SSH connection: {}", index))?;
    let old_auth = existing.auth.clone();

    let mut config = SshConfig::new(
        if resolved_name.is_empty() {
            format!("{resolved_user}@{resolved_host}")
        } else {
            resolved_name.to_string()
        },
        resolved_host,
        resolved_user,
    );
    config.port = normalize_ssh_port(port);
    config.auth = match auth_mode.trim() {
        "agent" => AuthMethod::Agent,
        "key" => {
            let resolved_key_path = key_path
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| String::from("SSH key path must not be empty."))?;
            let passphrase_credential_id = match &old_auth {
                AuthMethod::PublicKeyFile {
                    passphrase_credential_id,
                    ..
                } => passphrase_credential_id.clone(),
                _ => None,
            };
            AuthMethod::PublicKeyFile {
                private_key_path: resolved_key_path,
                passphrase_credential_id,
            }
        }
        _ => {
            // Both old AuthMethods that can carry a saved password
            // need to be checked: KeychainPassword (the keychain
            // round-trip succeeded last time) and DirectPassword
            // (the previous save fell back because the keychain
            // round-trip failed). Either one can hand us back an
            // existing credential id to reuse.
            let existing_credential_id = match &old_auth {
                AuthMethod::KeychainPassword { credential_id } => Some(credential_id.clone()),
                _ => None,
            };
            match password
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                Some(resolved_password) => {
                    let credential_id = existing_credential_id
                        .clone()
                        .unwrap_or_else(|| make_credential_id(resolved_host, resolved_user));
                    let keychain_ok =
                        credentials::set_and_verify(&credential_id, &resolved_password)
                            .map_err(|error| error.to_string())?;
                    if keychain_ok {
                        AuthMethod::KeychainPassword { credential_id }
                    } else {
                        // Keyring backend dropped the write; persist
                        // the password directly in the SshConfig so
                        // the saved connection still works on the
                        // next launch (matches the new-save fallback
                        // path in `ssh_connection_save`).
                        AuthMethod::DirectPassword {
                            password: resolved_password,
                        }
                    }
                }
                None => match existing_credential_id {
                    Some(credential_id) => AuthMethod::KeychainPassword { credential_id },
                    None => match &old_auth {
                        // No new password typed and the previous
                        // save was already DirectPassword — keep it
                        // as-is rather than rejecting the update.
                        AuthMethod::DirectPassword { password } => AuthMethod::DirectPassword {
                            password: password.clone(),
                        },
                        _ => return Err(String::from("SSH password must not be empty.")),
                    },
                },
            }
        }
    };

    // Preserve the previous group unless the caller explicitly passed
    // one. Passing `Some("")` / whitespace clears it; passing `None`
    // keeps the existing assignment so non-group-aware callers don't
    // accidentally ungroup rows.
    config.group = match group {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        None => existing.group.clone(),
    };

    let new_auth = config.auth.clone();
    store.connections[index] = config;
    store.save_default().map_err(|error| error.to_string())?;

    let reused_credential = auth_credential_id(&old_auth)
        .zip(auth_credential_id(&new_auth))
        .is_some_and(|(old_id, new_id)| old_id == new_id);

    if !reused_credential {
        delete_auth_credentials(&old_auth)?;
    }

    Ok(())
}

/// Atomic reorder + group-reassign for the saved-connections list.
/// Used by the sidebar drag-drop UI: `order[i]` is the old index of
/// the connection that should land in slot `i`, and `groups[i]` is
/// the new group label for that slot (None / empty → default group).
/// Group display order is derived from first-appearance in the new
/// list, so reordering groups is done by arranging members contiguously.
#[tauri::command]
fn ssh_connections_reorder(order: Vec<usize>, groups: Vec<Option<String>>) -> Result<(), String> {
    let mut store = ConnectionStore::load_default().map_err(|error| error.to_string())?;
    store
        .reorder_with_groups(&order, &groups)
        .map_err(|error| error.to_string())?;
    store.save_default().map_err(|error| error.to_string())
}

/// Rename every connection whose group matches `from` to `to`.
/// `to == None` or an empty / whitespace-only `to` ungroups them
/// (deletes the group label). Passing an empty `from` targets the
/// implicit "default" bucket (connections with no group).
#[tauri::command]
fn ssh_group_rename(from: String, to: Option<String>) -> Result<(), String> {
    let mut store = ConnectionStore::load_default().map_err(|error| error.to_string())?;
    store.rename_group(from.trim(), to.as_deref());
    store.save_default().map_err(|error| error.to_string())
}

#[tauri::command]
fn ssh_tunnel_open(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    remote_host: String,
    remote_port: u16,
    local_port: Option<u16>,
    saved_connection_index: Option<usize>,
) -> Result<TunnelInfoView, String> {
    let resolved_remote_host = if remote_host.trim().is_empty() {
        String::from("127.0.0.1")
    } else {
        remote_host.trim().to_string()
    };
    if remote_port == 0 {
        return Err(String::from("Tunnel remote port must not be empty."));
    }

    // Reuse the cached SSH session (seeded by the terminal) so a DB
    // panel opening its first tunnel doesn't re-handshake.
    let tunnel = run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| {
            session
                .open_local_forward_blocking(
                    local_port.unwrap_or(0),
                    &resolved_remote_host,
                    remote_port,
                )
                .map_err(|error| error.to_string())
        },
    )?;
    let managed_tunnel = ManagedTunnel {
        local_port: tunnel.local_port(),
        remote_host: resolved_remote_host,
        remote_port,
        tunnel,
    };
    let tunnel_id = format!(
        "tunnel-{}",
        state.next_tunnel_id.fetch_add(1, Ordering::Relaxed) + 1
    );
    let view = build_tunnel_view(tunnel_id.clone(), &managed_tunnel);

    state
        .tunnels
        .lock()
        .map_err(|_| String::from("tunnel state poisoned"))?
        .insert(tunnel_id, managed_tunnel);

    Ok(view)
}

#[tauri::command]
fn ssh_tunnel_info(
    state: tauri::State<'_, AppState>,
    tunnel_id: String,
) -> Result<TunnelInfoView, String> {
    let tunnels = state
        .tunnels
        .lock()
        .map_err(|_| String::from("tunnel state poisoned"))?;
    let tunnel = tunnels
        .get(&tunnel_id)
        .ok_or_else(|| format!("unknown tunnel: {}", tunnel_id))?;
    Ok(build_tunnel_view(tunnel_id, tunnel))
}

/// Snapshot of every active local port forward. Ordering is not
/// guaranteed — callers that want a stable display should sort
/// on the frontend (e.g. by local_port). Tunnels whose accept
/// loop has died still appear here so the UI can surface them
/// as "dead" instead of quietly vanishing.
#[tauri::command]
fn ssh_tunnel_list(state: tauri::State<'_, AppState>) -> Result<Vec<TunnelInfoView>, String> {
    let tunnels = state
        .tunnels
        .lock()
        .map_err(|_| String::from("tunnel state poisoned"))?;
    Ok(tunnels
        .iter()
        .map(|(id, t)| build_tunnel_view(id.clone(), t))
        .collect())
}

#[tauri::command]
fn ssh_tunnel_close(state: tauri::State<'_, AppState>, tunnel_id: String) -> Result<(), String> {
    let mut tunnels = state
        .tunnels
        .lock()
        .map_err(|_| String::from("tunnel state poisoned"))?;
    tunnels
        .remove(&tunnel_id)
        .map(|_| ())
        .ok_or_else(|| format!("unknown tunnel: {}", tunnel_id))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct KnownHostsListResult {
    path: Option<String>,
    entries: Vec<pier_core::ssh::KnownHostEntry>,
}

#[tauri::command]
fn ssh_known_hosts_list() -> Result<KnownHostsListResult, String> {
    let path = pier_core::ssh::default_known_hosts_path();
    let entries = match &path {
        Some(p) => pier_core::ssh::list_known_hosts(p).map_err(|e| e.to_string())?,
        None => Vec::new(),
    };
    Ok(KnownHostsListResult {
        path: path.map(|p| p.to_string_lossy().to_string()),
        entries,
    })
}

#[tauri::command]
fn ssh_known_hosts_remove(line: usize) -> Result<(), String> {
    let path = pier_core::ssh::default_known_hosts_path()
        .ok_or_else(|| String::from("home directory is not resolvable"))?;
    pier_core::ssh::remove_known_host_line(&path, line).map_err(|e| e.to_string())
}

/// Background pre-warm for the shared SSH session cache.
///
/// Called by the terminal panel the moment it detects a nested ssh
/// target (user typed `ssh user@host` in a local terminal, or nested
/// ssh inside an existing SSH tab) for which we have enough auth to
/// open our own russh session: a saved-connection index, a pubkey /
/// agent auth, or a password captured from the PTY prompt.
///
/// The real ssh the user launched lives in their local shell and has
/// its own TCP connection we can't reuse. So we open a parallel russh
/// session in the background and seed `sftp_sessions` under the same
/// `(auth_mode, user, host, port)` key the panel commands will look
/// up. By the time the user clicks Docker / SFTP / Monitor / Log /
/// DB panels, the cache is warm and the panel's first call avoids
/// the 1-3s handshake cost it would otherwise pay.
///
/// Fire-and-forget: returns immediately. Errors during the async
/// handshake are logged and dropped — this is pure optimization, a
/// miss just means the panel pays the cost the old way.
#[tauri::command]
fn ssh_session_prewarm(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    if host.trim().is_empty() || user.trim().is_empty() {
        return Ok(());
    }
    // Skip if the cache already has this target — cheap lock, no
    // need to spawn a blocking task just to return early.
    let key = sftp_cache_key(&host, port, &user, &auth_mode);
    let state: tauri::State<'_, AppState> = app.state();
    let already_cached = state
        .sftp_sessions
        .lock()
        .map(|cache| cache.contains_key(&key))
        .unwrap_or(false);
    if already_cached {
        return Ok(());
    }
    drop(state);
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        // Errors are intentional no-ops: prewarm is best-effort, and a
        // failure here just means the next panel call opens its own
        // session the usual way.
        let session = match get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        ) {
            Ok(s) => s,
            Err(_) => return,
        };
        // Also prewarm the SFTP subsystem and the $HOME probe — the
        // SFTP panel's first browse would otherwise still pay both
        // costs (≈ 2 RTT for the subsystem + 1 RTT for the home
        // probe). With them primed, opening the SFTP panel collapses
        // to a single `list_dir` round-trip.
        let _ = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode);
        let _ = resolve_remote_home_cached(&state, &session, &host, port, &user, &auth_mode);
    });
    Ok(())
}

fn map_stash_entry(entry: StashEntry) -> GitStashEntry {
    GitStashEntry {
        index: entry.index,
        message: entry.message,
        relative_date: entry.relative_date,
    }
}

#[tauri::command]
fn mysql_browse(
    host: String,
    port: u16,
    user: String,
    password: String,
    database: Option<String>,
    table: Option<String>,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Result<MysqlBrowserState, String> {
    let resolved_host = host.trim();
    let resolved_user = user.trim();
    if resolved_host.is_empty() || resolved_user.is_empty() {
        return Err(String::from("MySQL host and user must not be empty."));
    }

    let client = MysqlClient::connect_blocking(MysqlConfig {
        host: resolved_host.to_string(),
        port: normalize_mysql_port(port),
        user: resolved_user.to_string(),
        password,
        database: database.clone().filter(|value| !value.trim().is_empty()),
    })
    .map_err(|error| error.to_string())?;

    let databases = client
        .list_databases_blocking()
        .map_err(|error| error.to_string())?;
    let database_name = choose_active_item(database, &databases);
    // Pull the enriched table list once and derive the bare-name
    // `tables` array from it — saves a second SHOW TABLES round
    // trip while still giving the schema tree row counts / engine
    // / sizing in the same call.
    let table_summaries: Vec<MysqlTableSummary> = if database_name.is_empty() {
        Vec::new()
    } else {
        client
            .list_tables_meta_blocking(&database_name)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|s| MysqlTableSummary {
                name: s.name,
                row_count: s.row_count,
                data_bytes: s.data_bytes,
                index_bytes: s.index_bytes,
                engine: s.engine,
                updated_at: s.updated_at,
                comment: s.comment,
            })
            .collect()
    };
    let tables: Vec<String> = table_summaries
        .iter()
        .map(|s| s.name.clone())
        .collect();
    // Views + routines are pulled per-database too. Failures here
    // are non-fatal — a permission-restricted user might be unable
    // to read `information_schema.routines`; we'd rather show a
    // working tables list than block the whole panel.
    let views = if database_name.is_empty() {
        Vec::new()
    } else {
        client.list_views_blocking(&database_name).unwrap_or_default()
    };
    let routines = if database_name.is_empty() {
        Vec::new()
    } else {
        client
            .list_routines_blocking(&database_name)
            .unwrap_or_default()
            .into_iter()
            .map(|r| MysqlRoutineSummary {
                name: r.name,
                kind: r.kind,
            })
            .collect()
    };
    let table_name = choose_active_item(table, &tables);
    let columns = if database_name.is_empty() || table_name.is_empty() {
        Vec::new()
    } else {
        client
            .list_columns_blocking(&database_name, &table_name)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|column| MysqlColumnView {
                name: column.name,
                column_type: column.column_type,
                nullable: column.nullable,
                key: column.key,
                default_value: column.default_value.unwrap_or_default(),
                extra: column.extra,
                comment: column.comment,
            })
            .collect()
    };
    // Paging: default page size matches the original 24-row preview;
    // hard upper bound at 500 keeps a single browse hit reasonable.
    // The frontend `Page N of M` chip relies on `effectivePageSize` /
    // `totalRows` — both come back in the response so the UI doesn't
    // have to remember the request shape.
    let effective_page_size = limit.unwrap_or(24).clamp(1, 500);
    let effective_offset = offset.unwrap_or(0);

    // Time the preview query specifically — the panel surfaces
    // this as a "{ms} ms" chip on the grid toolbar so the user
    // sees how expensive the page they're flipping through is.
    let mut browse_elapsed_ms: u64 = 0;
    let preview = if database_name.is_empty()
        || table_name.is_empty()
        || !mysql_service::is_safe_ident(&database_name)
        || !mysql_service::is_safe_ident(&table_name)
    {
        None
    } else {
        match client.execute_blocking(&format!(
            "SELECT * FROM `{database_name}`.`{table_name}` \
             LIMIT {effective_page_size} OFFSET {effective_offset}"
        )) {
            Ok(r) => {
                browse_elapsed_ms = r.elapsed_ms;
                Some(map_mysql_preview(r))
            }
            Err(_) => None,
        }
    };
    // `COUNT(*)` is best-effort — on very large tables this scan
    // can be slow, but the panel's "Page N of M" chip needs the
    // total. Errors surface as `None` so the UI falls back to a
    // page-N-of-? indicator.
    let total_rows: Option<u64> = if database_name.is_empty()
        || table_name.is_empty()
        || !mysql_service::is_safe_ident(&database_name)
        || !mysql_service::is_safe_ident(&table_name)
    {
        None
    } else {
        client
            .execute_blocking(&format!(
                "SELECT COUNT(*) AS total FROM `{database_name}`.`{table_name}`"
            ))
            .ok()
            .and_then(|r| {
                r.rows
                    .first()
                    .and_then(|row| row.first())
                    .and_then(|v| v.as_ref().and_then(|s| s.parse::<u64>().ok()))
            })
    };
    // Index + FK lookups are scoped to the active table. Failsoft
    // to empty arrays — a permission-restricted user (or an old
    // server without a populated `referential_constraints`) will
    // still see the rest of the panel without an error.
    let (indexes, foreign_keys) = if database_name.is_empty() || table_name.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let ix = client
            .list_indexes_blocking(&database_name, &table_name)
            .unwrap_or_default()
            .into_iter()
            .map(|i| MysqlIndexView {
                name: i.name,
                columns: i.columns,
                unique: i.unique,
                kind: i.kind,
            })
            .collect();
        let fk = client
            .list_foreign_keys_blocking(&database_name, &table_name)
            .unwrap_or_default()
            .into_iter()
            .map(|f| MysqlForeignKeyView {
                name: f.name,
                columns: f.columns,
                ref_schema: f.ref_schema,
                ref_table: f.ref_table,
                ref_columns: f.ref_columns,
                on_update: f.on_update,
                on_delete: f.on_delete,
            })
            .collect();
        (ix, fk)
    };

    Ok(MysqlBrowserState {
        database_name,
        databases,
        table_name,
        tables,
        table_summaries,
        views,
        routines,
        columns,
        indexes,
        foreign_keys,
        preview,
        page_size: effective_page_size,
        page_offset: effective_offset,
        total_rows,
        browse_elapsed_ms,
    })
}

#[tauri::command]
fn sqlite_browse(path: String, table: Option<String>) -> Result<SqliteBrowserState, String> {
    let resolved_path = path.trim();
    if resolved_path.is_empty() {
        return Err(String::from("SQLite database path must not be empty."));
    }

    let client = SqliteClient::open(resolved_path).map_err(|error| error.to_string())?;
    let tables = client.list_tables().map_err(|error| error.to_string())?;
    let table_name = choose_active_item(table, &tables);
    let columns = if table_name.is_empty() {
        Vec::new()
    } else {
        client
            .table_columns(&table_name)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|column| SqliteColumnView {
                name: column.name,
                col_type: column.col_type,
                not_null: column.not_null,
                primary_key: column.primary_key,
            })
            .collect()
    };
    // Indexes / triggers are best-effort: we never want a corrupt
    // sqlite_master row or a permission flip on the temp dir to
    // tank the entire browse — the column grid is the load-bearing
    // surface.
    let indexes = if table_name.is_empty() {
        Vec::new()
    } else {
        client
            .table_indexes(&table_name)
            .unwrap_or_default()
            .into_iter()
            .map(|i| SqliteIndexView {
                name: i.name,
                unique: i.unique,
                origin: i.origin,
                columns: i.columns,
            })
            .collect()
    };
    let triggers = if table_name.is_empty() {
        Vec::new()
    } else {
        client
            .table_triggers(&table_name)
            .unwrap_or_default()
            .into_iter()
            .map(|t| SqliteTriggerView {
                name: t.name,
                event: t.event,
                sql: t.sql,
            })
            .collect()
    };
    let preview = if table_name.is_empty() {
        None
    } else {
        let escaped = table_name.replace('"', "\"\"");
        map_sqlite_preview(client.execute(&format!("SELECT * FROM \"{escaped}\" LIMIT 24;")))
    };
    let file_size = client.file_size();

    Ok(SqliteBrowserState {
        path: resolved_path.to_string(),
        table_name,
        tables,
        columns,
        preview,
        indexes,
        triggers,
        file_size,
    })
}

#[tauri::command]
fn redis_browse(
    host: String,
    port: u16,
    db: i64,
    pattern: Option<String>,
    key: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
    username: Option<String>,
    password: Option<String>,
) -> Result<RedisBrowserState, String> {
    let resolved_host = host.trim();
    if resolved_host.is_empty() {
        return Err(String::from("Redis host must not be empty."));
    }

    let client = RedisClient::connect_blocking(RedisConfig {
        host: resolved_host.to_string(),
        port: normalize_redis_port(port),
        db,
        username: username.filter(|s| !s.is_empty()),
        password: password.filter(|s| !s.is_empty()),
    })
    .map_err(|error| error.to_string())?;
    let pong = client.ping_blocking().map_err(|error| error.to_string())?;
    let pattern = pattern
        .unwrap_or_else(|| String::from("*"))
        .trim()
        .to_string();
    let effective_pattern = if pattern.is_empty() {
        String::from("*")
    } else {
        pattern
    };
    let cursor_in = cursor.unwrap_or_else(|| String::from("0"));
    let effective_limit = limit.unwrap_or(120).clamp(1, 500);
    let page = client
        .scan_keys_paged_blocking(&effective_pattern, &cursor_in, effective_limit)
        .map_err(|error| error.to_string())?;
    let key_names: Vec<String> = page.keys.iter().map(|e| e.key.clone()).collect();
    let key_name = choose_active_item(key, &key_names);
    let details = if key_name.is_empty() {
        None
    } else {
        client
            .inspect_blocking(&key_name)
            .ok()
            .map(map_redis_details)
    };
    let server_info = client.info_blocking("server").unwrap_or_default();
    let memory_info = client.info_blocking("memory").unwrap_or_default();

    let entries: Vec<RedisKeyEntry> = page
        .keys
        .into_iter()
        .map(|e| RedisKeyEntry {
            key: e.key,
            kind: e.kind,
            ttl_seconds: e.ttl_seconds,
        })
        .collect();

    Ok(RedisBrowserState {
        pong,
        pattern: effective_pattern,
        limit: effective_limit,
        // Caller can compare `next_cursor != "0"` to decide
        // whether more keys exist — drives the "Load more" UI.
        truncated: page.next_cursor != "0",
        key_name,
        keys: entries,
        next_cursor: page.next_cursor,
        rtt_ms: page.rtt_ms,
        server_version: server_info
            .get("redis_version")
            .or_else(|| server_info.get("valkey_version"))
            .cloned()
            .unwrap_or_default(),
        used_memory: memory_info
            .get("used_memory_human")
            .cloned()
            .unwrap_or_default(),
        details,
    })
}

#[tauri::command]
fn redis_execute(
    host: String,
    port: u16,
    db: i64,
    command: String,
    username: Option<String>,
    password: Option<String>,
) -> Result<RedisCommandResultView, String> {
    let resolved_host = host.trim();
    if resolved_host.is_empty() {
        return Err(String::from("Redis host must not be empty."));
    }

    let args = tokenize_command_line(command.trim())?;
    let client = RedisClient::connect_blocking(RedisConfig {
        host: resolved_host.to_string(),
        port: normalize_redis_port(port),
        db,
        username: username.filter(|s| !s.is_empty()),
        password: password.filter(|s| !s.is_empty()),
    })
    .map_err(|error| error.to_string())?;
    let result = client
        .execute_command_blocking(&args)
        .map_err(|error| error.to_string())?;

    Ok(RedisCommandResultView {
        summary: result.summary,
        lines: result.lines,
        elapsed_ms: result.elapsed_ms,
    })
}

/// Confirm-guarded RENAME. Backed by `RENAMENX` so the call
/// fails on collision instead of silently overwriting a key.
/// The panel surfaces the false return as a user-visible error
/// ("a key with that name already exists") and lets the user
/// decide whether to retry against an empty target.
#[tauri::command]
fn redis_rename_key(
    host: String,
    port: u16,
    db: i64,
    from: String,
    to: String,
    username: Option<String>,
    password: Option<String>,
) -> Result<bool, String> {
    let resolved_host = host.trim();
    if resolved_host.is_empty() {
        return Err(String::from("Redis host must not be empty."));
    }
    let from_key = from.trim();
    let to_key = to.trim();
    if from_key.is_empty() || to_key.is_empty() {
        return Err(String::from("Both source and destination keys are required."));
    }
    if from_key == to_key {
        return Err(String::from("Source and destination keys must differ."));
    }
    let client = RedisClient::connect_blocking(RedisConfig {
        host: resolved_host.to_string(),
        port: normalize_redis_port(port),
        db,
        username: username.filter(|s| !s.is_empty()),
        password: password.filter(|s| !s.is_empty()),
    })
    .map_err(|error| error.to_string())?;
    client
        .rename_nx_blocking(from_key, to_key)
        .map_err(|error| error.to_string())
}

/// Confirm-guarded DEL. Returns `true` when the key existed
/// (so the panel can show "deleted" vs "did not exist" in the
/// notice). Bulk delete is intentionally not exposed here —
/// the UI walks one key per confirm to keep destructive blast
/// radius small.
#[tauri::command]
fn redis_delete_key(
    host: String,
    port: u16,
    db: i64,
    key: String,
    username: Option<String>,
    password: Option<String>,
) -> Result<bool, String> {
    let resolved_host = host.trim();
    if resolved_host.is_empty() {
        return Err(String::from("Redis host must not be empty."));
    }
    let key_name = key.trim();
    if key_name.is_empty() {
        return Err(String::from("Key name must not be empty."));
    }
    let client = RedisClient::connect_blocking(RedisConfig {
        host: resolved_host.to_string(),
        port: normalize_redis_port(port),
        db,
        username: username.filter(|s| !s.is_empty()),
        password: password.filter(|s| !s.is_empty()),
    })
    .map_err(|error| error.to_string())?;
    client
        .del_blocking(key_name)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn mysql_execute(
    host: String,
    port: u16,
    user: String,
    password: String,
    database: Option<String>,
    sql: String,
) -> Result<QueryExecutionResult, String> {
    let resolved_host = host.trim();
    let resolved_user = user.trim();
    let resolved_sql = sql.trim();
    if resolved_host.is_empty() || resolved_user.is_empty() {
        return Err(String::from("MySQL host and user must not be empty."));
    }
    if resolved_sql.is_empty() {
        return Err(String::from("SQL must not be empty."));
    }

    let client = MysqlClient::connect_blocking(MysqlConfig {
        host: resolved_host.to_string(),
        port: normalize_mysql_port(port),
        user: resolved_user.to_string(),
        password,
        database: database.filter(|value| !value.trim().is_empty()),
    })
    .map_err(|error| error.to_string())?;

    client
        .execute_blocking(resolved_sql)
        .map(map_mysql_query_result)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn sqlite_execute(path: String, sql: String) -> Result<QueryExecutionResult, String> {
    let resolved_path = path.trim();
    let resolved_sql = sql.trim();
    if resolved_path.is_empty() {
        return Err(String::from("SQLite database path must not be empty."));
    }
    if resolved_sql.is_empty() {
        return Err(String::from("SQL must not be empty."));
    }

    let client = SqliteClient::open(resolved_path).map_err(|error| error.to_string())?;
    map_sqlite_query_result(client.execute(resolved_sql))
}

/// Run a multi-statement SQL script against the SQLite file.
/// Each top-level semicolon-separated statement returns its own
/// [`QueryExecutionResult`] with per-statement timing — the panel
/// renders the last result's grid and shows the timings list
/// above. The first failing statement aborts the run and the
/// command resolves to `Err(message)` — matching `sqlite_execute`
/// so the panel's error UI doesn't need a third state.
#[tauri::command]
fn sqlite_execute_script(path: String, sql: String) -> Result<Vec<QueryExecutionResult>, String> {
    let resolved_path = path.trim();
    let resolved_sql = sql.trim();
    if resolved_path.is_empty() {
        return Err(String::from("SQLite database path must not be empty."));
    }
    if resolved_sql.is_empty() {
        return Err(String::from("SQL must not be empty."));
    }
    let client = SqliteClient::open(resolved_path).map_err(|error| error.to_string())?;
    let results = client.execute_script(resolved_sql);
    let mut out = Vec::with_capacity(results.len());
    for r in results {
        match map_sqlite_query_result(r) {
            Ok(view) => out.push(view),
            Err(message) => return Err(message),
        }
    }
    Ok(out)
}

#[tauri::command]
fn terminal_create(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    cols: u16,
    rows: u16,
    shell: Option<String>,
    smart_mode: Option<bool>,
) -> Result<TerminalSessionInfo, String> {
    let resolved_cols = cols.max(40);
    let resolved_rows = rows.max(12);
    let resolved_shell = shell
        .filter(|candidate| !candidate.trim().is_empty())
        .unwrap_or_else(default_shell);

    let (session_id, mut notify_ctx) = allocate_notify_context(&state, app);
    let user_data = &mut *notify_ctx as *mut NotifyContext as *mut c_void;

    // Inject the ssh-mux wrapper into PATH so any `ssh` (or scp /
    // rsync / git) the user runs in this PTY picks up Pier-X's
    // ControlMaster config — first connection authenticates, every
    // subsequent ssh to the same target inside the persist window
    // is a free ride. The wrapper is a tiny POSIX-shell shim that
    // exec's /usr/bin/ssh -F <pier-x-ssh-config> "$@".
    //
    // No-op when ssh_mux::init failed at startup (e.g. unwritable
    // cache dir) — `prepended_path` returns the inherited PATH
    // unchanged in that case, so terminals still come up.
    let inherited_path =
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".to_string());
    let prefixed_path = ssh_mux::prepended_path(&inherited_path);
    let extra_env: &[(&str, &str)] = &[("PATH", prefixed_path.as_str())];

    let terminal = PierTerminal::new_with_smart_env(
        resolved_cols,
        resolved_rows,
        &resolved_shell,
        smart_mode.unwrap_or(false),
        extra_env,
        tauri_terminal_notify as NotifyFn,
        user_data,
    )
    .map_err(|error| error.to_string())?;

    store_terminal_session(
        state,
        session_id,
        notify_ctx,
        terminal,
        resolved_shell,
        resolved_cols,
        resolved_rows,
    )
}

#[tauri::command]
fn terminal_create_ssh(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    cols: u16,
    rows: u16,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: Option<String>,
    key_path: Option<String>,
) -> Result<TerminalSessionInfo, String> {
    let config = build_manual_ssh_config(host, port, user, auth_mode, password, key_path)?;
    create_ssh_terminal_from_config(state, app, config, cols, rows)
}

#[tauri::command]
fn terminal_create_ssh_saved(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    cols: u16,
    rows: u16,
    index: usize,
) -> Result<TerminalSessionInfo, String> {
    let config = open_saved_ssh_config(index)?;
    create_ssh_terminal_from_config(state, app, config, cols, rows)
}

#[tauri::command]
fn terminal_write(
    state: tauri::State<'_, AppState>,
    session_id: String,
    data: String,
) -> Result<usize, String> {
    let sessions = state
        .terminals
        .lock()
        .map_err(|_| String::from("terminal state poisoned"))?;
    let managed = sessions
        .get(&session_id)
        .ok_or_else(|| format!("unknown terminal session: {}", session_id))?;
    managed
        .terminal
        .write(data.as_bytes())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn terminal_resize(
    state: tauri::State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let mut sessions = state
        .terminals
        .lock()
        .map_err(|_| String::from("terminal state poisoned"))?;
    let managed = sessions
        .get_mut(&session_id)
        .ok_or_else(|| format!("unknown terminal session: {}", session_id))?;
    managed
        .terminal
        .resize(cols.max(40), rows.max(12))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn terminal_snapshot(
    state: tauri::State<'_, AppState>,
    session_id: String,
    scrollback_offset: Option<usize>,
) -> Result<TerminalSnapshot, String> {
    let sessions = state
        .terminals
        .lock()
        .map_err(|_| String::from("terminal state poisoned"))?;
    let managed = sessions
        .get(&session_id)
        .ok_or_else(|| format!("unknown terminal session: {}", session_id))?;

    let alive = managed.terminal.is_alive();
    let snapshot = managed
        .terminal
        .snapshot_view(scrollback_offset.unwrap_or(0));

    Ok(TerminalSnapshot {
        cols: snapshot.cols,
        rows: snapshot.rows,
        alive,
        scrollback_len: managed.terminal.scrollback_len(),
        bell_pending: managed.terminal.take_bell_pending(),
        lines: build_terminal_lines(&snapshot, alive),
        prompt_end: snapshot.prompt_end.map(|(r, c)| [r, c]),
        cursor_x: snapshot.cursor_x,
        cursor_y: snapshot.cursor_y,
        awaiting_input: snapshot.awaiting_input,
        alt_screen: snapshot.alt_screen,
        bracketed_paste: snapshot.bracketed_paste,
    })
}

#[tauri::command]
fn terminal_set_scrollback_limit(
    state: tauri::State<'_, AppState>,
    session_id: String,
    limit: usize,
) -> Result<(), String> {
    let sessions = state
        .terminals
        .lock()
        .map_err(|_| String::from("terminal state poisoned"))?;
    let managed = sessions
        .get(&session_id)
        .ok_or_else(|| format!("unknown terminal session: {}", session_id))?;
    managed.terminal.set_scrollback_limit(limit);
    Ok(())
}

/// Return the last-known shell working directory if OSC 7 has
/// fired for this session. Returns `None` (null in JS) when
/// the shell hasn't reported one yet — the SQLite panel then
/// falls back to `~` for its directory scan.
#[tauri::command]
fn terminal_current_cwd(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Option<String>, String> {
    let sessions = state
        .terminals
        .lock()
        .map_err(|_| String::from("terminal state poisoned"))?;
    let managed = sessions
        .get(&session_id)
        .ok_or_else(|| format!("unknown terminal session: {}", session_id))?;
    Ok(managed.terminal.current_cwd())
}

/// SFTP-backed Tab completion for an active SSH terminal tab. Same
/// shape as `terminal_completions` but file rows come from the live
/// SFTP session keyed by `(auth_mode, user, host, port)` — so e.g.
/// `cd /mnt/da` + Tab in a russh tab lists `/mnt/data/`, `/mnt/dev/`
/// etc from the **remote** filesystem, not the local Mac.
///
/// Library + builtin/PATH-binary rows still come from the local pack
/// dir (those are about *what to type*, not *what files exist where*),
/// so the user sees the same docker/git/kubectl subcommands as in a
/// local tab.
#[tauri::command]
fn terminal_completions_remote(
    state: tauri::State<'_, AppState>,
    line: String,
    cursor: usize,
    cwd: Option<String>,
    locale: Option<String>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
) -> Result<Vec<pier_core::terminal::Completion>, String> {
    use pier_core::terminal::{
        complete_with_library_using, DirReadEntry, DirReader, LocalDirReader,
    };
    use std::path::Path;

    let key = sftp_cache_key(&host, port, &user, &auth_mode);
    let session_opt = match state.sftp_sessions.lock() {
        Ok(g) => g.get(&key).cloned(),
        Err(p) => p.into_inner().get(&key).cloned(),
    };
    let sftp_client = if let Some(session) = session_opt {
        get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode).ok()
    } else {
        None
    };

    /// `DirReader` that lists a remote directory through the cached
    /// SFTP client. Failures fold to an empty list — completion
    /// shows fewer rows but never errors out at the keyboard.
    ///
    /// When the completer asks for `.` (no cwd known yet because the
    /// remote shell hasn't reported OSC 7 / our prompt-hook hasn't
    /// installed), we substitute the SFTP server's canonical home
    /// — same behaviour as `pwd` over an interactive SFTP session.
    /// We cache the answer per-completion-call to avoid round-trips
    /// for every relative subdir typed at the same prefix.
    struct SftpDirReader {
        client: SftpClient,
        // OnceCell would be cleaner but we don't have it imported and
        // a Mutex<Option<...>> is fine for one-shot caching at this
        // call rate. RefCell is `!Sync`; DirReader is called only
        // from one thread at a time but the trait isn't restricted.
        home_cache: std::sync::Mutex<Option<String>>,
    }
    impl SftpDirReader {
        fn resolve(&self, dir: &Path) -> String {
            let raw = dir.to_string_lossy();
            if raw != "." {
                return raw.into_owned();
            }
            // Only ever "." reaches here; consult / populate the cache.
            let mut guard = self.home_cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(cached) = guard.as_ref() {
                return cached.clone();
            }
            let resolved = self
                .client
                .canonicalize_blocking(".")
                .unwrap_or_else(|_| ".".to_string());
            *guard = Some(resolved.clone());
            resolved
        }
    }
    impl DirReader for SftpDirReader {
        fn list(&self, dir: &Path) -> Vec<DirReadEntry> {
            let path = self.resolve(dir);
            let entries = match self.client.list_dir_blocking(&path) {
                Ok(e) => e,
                Err(_) => return Vec::new(),
            };
            entries
                .into_iter()
                .map(|e| DirReadEntry {
                    name: e.name,
                    is_dir: e.is_dir,
                })
                .collect()
        }
    }

    let cwd_path = cwd.as_deref().map(Path::new);
    let locale_str = locale.as_deref().unwrap_or("en");
    let lib = terminal_smart::completion_library_snapshot();

    let rows = if let Some(client) = sftp_client {
        let reader = SftpDirReader {
            client,
            home_cache: std::sync::Mutex::new(None),
        };
        complete_with_library_using(&line, cursor, cwd_path, &lib, locale_str, &reader)
    } else {
        // No SFTP cached yet (tab still authenticating, mismatched
        // auth_mode, etc.) — fall back to local readdir. Library +
        // history rows are unaffected.
        complete_with_library_using(
            &line,
            cursor,
            cwd_path,
            &lib,
            locale_str,
            &LocalDirReader,
        )
    };
    Ok(rows)
}

#[tauri::command]
fn terminal_close(state: tauri::State<'_, AppState>, session_id: String) -> Result<(), String> {
    let mut sessions = state
        .terminals
        .lock()
        .map_err(|_| String::from("terminal state poisoned"))?;
    sessions
        .remove(&session_id)
        .map(|_| ())
        .ok_or_else(|| format!("unknown terminal session: {}", session_id))
}

// ── PostgreSQL ──────────────────────────────────────────────────────

#[tauri::command]
fn postgres_browse(
    host: String,
    port: u16,
    user: String,
    password: String,
    database: Option<String>,
    schema: Option<String>,
    table: Option<String>,
) -> Result<PostgresBrowserState, String> {
    let resolved_host = host.trim();
    let resolved_user = user.trim();
    if resolved_host.is_empty() || resolved_user.is_empty() {
        return Err(String::from("PostgreSQL host and user must not be empty."));
    }

    let client = PostgresClient::connect_blocking(PostgresConfig {
        host: resolved_host.to_string(),
        port: normalize_postgres_port(port),
        user: resolved_user.to_string(),
        password,
        database: database.clone().filter(|v| !v.trim().is_empty()),
    })
    .map_err(|e| e.to_string())?;

    let databases = client
        .list_databases_blocking()
        .map_err(|e| e.to_string())?;
    let database_name = choose_active_item(database, &databases);
    // Schema list is best-effort — failure to enumerate (e.g. low-
    // privilege role) shouldn't block the browse, the picker just
    // shows the active schema only.
    let schemas = if database_name.is_empty() {
        Vec::new()
    } else {
        client.list_schemas_blocking().unwrap_or_default()
    };
    let schema_name = schema
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            // Prefer "public" when available, otherwise fall back
            // to the first user-visible schema. Roles without
            // permission to see "public" still get a sensible
            // default instead of a hardcoded miss.
            if schemas.iter().any(|s| s == "public") {
                String::from("public")
            } else {
                schemas
                    .first()
                    .cloned()
                    .unwrap_or_else(|| String::from("public"))
            }
        });
    // Pull the enriched table list once and derive the bare-name
    // `tables` array from it — same pattern as the MySQL panel.
    let table_summaries: Vec<PostgresTableSummary> = if database_name.is_empty() {
        Vec::new()
    } else {
        client
            .list_tables_meta_blocking(&schema_name)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|s| PostgresTableSummary {
                name: s.name,
                row_count: s.row_count,
                data_bytes: s.data_bytes,
                index_bytes: s.index_bytes,
                engine: s.engine,
                updated_at: s.updated_at,
                comment: s.comment,
            })
            .collect()
    };
    let tables: Vec<String> = table_summaries
        .iter()
        .map(|s| s.name.clone())
        .collect();
    // Views + routines failsoft on permission errors — same logic
    // as MySQL.
    let views = if database_name.is_empty() {
        Vec::new()
    } else {
        client.list_views_blocking(&schema_name).unwrap_or_default()
    };
    let routines = if database_name.is_empty() {
        Vec::new()
    } else {
        client
            .list_routines_blocking(&schema_name)
            .unwrap_or_default()
            .into_iter()
            .map(|r| PostgresRoutineSummary {
                name: r.name,
                kind: r.kind,
            })
            .collect()
    };
    let table_name = choose_active_item(table, &tables);
    let columns = if database_name.is_empty() || table_name.is_empty() {
        Vec::new()
    } else {
        client
            .list_columns_blocking(&schema_name, &table_name)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|col| PostgresColumnView {
                name: col.name,
                column_type: col.column_type,
                nullable: col.nullable,
                key: col.key,
                default_value: col.default_value.unwrap_or_default(),
                extra: col.extra,
                comment: col.comment,
            })
            .collect()
    };
    let preview = if database_name.is_empty() || table_name.is_empty() {
        None
    } else {
        let escaped_schema = schema_name.replace('"', "\"\"");
        let escaped_table = table_name.replace('"', "\"\"");
        client
            .execute_blocking(&format!(
                "SELECT * FROM \"{escaped_schema}\".\"{escaped_table}\" LIMIT 24"
            ))
            .ok()
            .map(map_postgres_preview)
    };
    // Index + FK lookups for the active table — failsoft to empty
    // when the catalog tables aren't readable for the role.
    let (indexes, foreign_keys) = if database_name.is_empty() || table_name.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let ix = client
            .list_indexes_blocking(&schema_name, &table_name)
            .unwrap_or_default()
            .into_iter()
            .map(|i| PostgresIndexView {
                name: i.name,
                columns: i.columns,
                unique: i.unique,
                kind: i.kind,
            })
            .collect();
        let fk = client
            .list_foreign_keys_blocking(&schema_name, &table_name)
            .unwrap_or_default()
            .into_iter()
            .map(|f| PostgresForeignKeyView {
                name: f.name,
                columns: f.columns,
                ref_schema: f.ref_schema,
                ref_table: f.ref_table,
                ref_columns: f.ref_columns,
                on_update: f.on_update,
                on_delete: f.on_delete,
            })
            .collect();
        (ix, fk)
    };

    let (active, total) = client.pool_status_blocking().unwrap_or((0, 0));

    Ok(PostgresBrowserState {
        database_name,
        databases,
        schema_name,
        schemas,
        table_name,
        tables,
        table_summaries,
        views,
        routines,
        columns,
        indexes,
        foreign_keys,
        preview,
        pool: PostgresPoolView { active, total },
    })
}

#[tauri::command]
fn postgres_execute(
    host: String,
    port: u16,
    user: String,
    password: String,
    database: Option<String>,
    sql: String,
) -> Result<QueryExecutionResult, String> {
    let client = PostgresClient::connect_blocking(PostgresConfig {
        host: host.trim().to_string(),
        port: normalize_postgres_port(port),
        user: user.trim().to_string(),
        password,
        database: database.filter(|v| !v.trim().is_empty()),
    })
    .map_err(|e| e.to_string())?;

    let result = client.execute_blocking(&sql).map_err(|e| e.to_string())?;
    Ok(map_postgres_query_result(result))
}

/// Dedicated DB connectivity probe. Opens the kind-specific client,
/// runs the cheapest version-inspection query, returns the measured
/// round-trip alongside the server version string. Used by the
/// "Test" button in the New-connection dialog.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DbTestConnectionResult {
    ok: bool,
    elapsed_ms: u64,
    server_version: String,
}

#[tauri::command]
fn db_test_connection(
    kind: String,
    host: String,
    port: u16,
    user: String,
    password: String,
    database: Option<String>,
) -> Result<DbTestConnectionResult, String> {
    let host_trim = host.trim();
    if host_trim.is_empty() {
        return Err(String::from("Host must not be empty."));
    }
    let start = std::time::Instant::now();

    match kind.as_str() {
        "mysql" => {
            let user_trim = user.trim();
            if user_trim.is_empty() {
                return Err(String::from("MySQL user must not be empty."));
            }
            let client = MysqlClient::connect_blocking(MysqlConfig {
                host: host_trim.to_string(),
                port: normalize_mysql_port(port),
                user: user_trim.to_string(),
                password,
                database: database.filter(|v| !v.trim().is_empty()),
            })
            .map_err(|e| e.to_string())?;
            let version = client
                .execute_blocking("SELECT VERSION()")
                .ok()
                .and_then(|r| r.rows.into_iter().next())
                .and_then(|row| row.into_iter().next())
                .flatten()
                .unwrap_or_default();
            Ok(DbTestConnectionResult {
                ok: true,
                elapsed_ms: start.elapsed().as_millis() as u64,
                server_version: version,
            })
        }
        "postgres" => {
            let user_trim = user.trim();
            if user_trim.is_empty() {
                return Err(String::from("PostgreSQL user must not be empty."));
            }
            let client = PostgresClient::connect_blocking(PostgresConfig {
                host: host_trim.to_string(),
                port: normalize_postgres_port(port),
                user: user_trim.to_string(),
                password,
                database: database.filter(|v| !v.trim().is_empty()),
            })
            .map_err(|e| e.to_string())?;
            let version = client
                .execute_blocking("SELECT version()")
                .ok()
                .and_then(|r| r.rows.into_iter().next())
                .and_then(|row| row.into_iter().next())
                .flatten()
                .unwrap_or_default();
            Ok(DbTestConnectionResult {
                ok: true,
                elapsed_ms: start.elapsed().as_millis() as u64,
                server_version: version,
            })
        }
        "redis" => {
            let client = RedisClient::connect_blocking(RedisConfig {
                host: host_trim.to_string(),
                port: normalize_redis_port(port),
                db: 0,
                username: {
                    let u = user.trim();
                    if u.is_empty() { None } else { Some(u.to_string()) }
                },
                password: if password.is_empty() { None } else { Some(password) },
            })
            .map_err(|e| e.to_string())?;
            client.ping_blocking().map_err(|e| e.to_string())?;
            let server_info = client.info_blocking("server").unwrap_or_default();
            let version = server_info
                .get("redis_version")
                .or_else(|| server_info.get("valkey_version"))
                .cloned()
                .unwrap_or_default();
            Ok(DbTestConnectionResult {
                ok: true,
                elapsed_ms: start.elapsed().as_millis() as u64,
                server_version: version,
            })
        }
        _ => Err(format!("Unsupported kind: {kind}")),
    }
}

/// Summary view of a single file inside `~/.ssh/`. Returned by
/// `ssh_key_list` — consumed by the Settings → SSH keys pane.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SshKeyEntry {
    name: String,
    path: String,
    kind: String,
    size: u64,
    modified: Option<u64>,
    comment: String,
    algorithm: String,
}

fn resolve_ssh_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .or_else(|| {
            let drive = std::env::var("HOMEDRIVE").ok()?;
            let path = std::env::var("HOMEPATH").ok()?;
            Some(format!("{drive}{path}"))
        })?;
    Some(PathBuf::from(home).join(".ssh"))
}

fn classify_ssh_file(name: &str, first_line: &str) -> (String, String, String) {
    if name == "config" {
        return ("config".into(), String::new(), String::new());
    }
    if name == "known_hosts" || name.starts_with("known_hosts") {
        return ("known_hosts".into(), String::new(), String::new());
    }
    if name == "authorized_keys" {
        return ("authorized_keys".into(), String::new(), String::new());
    }
    let trimmed = first_line.trim();
    if trimmed.starts_with("-----BEGIN") {
        return ("private".into(), String::new(), String::new());
    }
    if trimmed.starts_with("ssh-rsa ")
        || trimmed.starts_with("ssh-ed25519 ")
        || trimmed.starts_with("ssh-dss ")
        || trimmed.starts_with("ecdsa-sha2-")
        || trimmed.starts_with("sk-ssh-ed25519@openssh.com ")
        || trimmed.starts_with("sk-ecdsa-sha2-nistp256@openssh.com ")
    {
        let mut parts = trimmed.splitn(3, ' ');
        let alg = parts.next().unwrap_or("").to_string();
        let _body = parts.next().unwrap_or("");
        let comment = parts.next().unwrap_or("").trim().to_string();
        return ("public".into(), alg, comment);
    }
    ("other".into(), String::new(), String::new())
}

/// Enumerate the local `~/.ssh/` directory. Only reads the first
/// 1 KB of each file (enough to classify public keys + detect PEM
/// armor) to keep the command cheap on large `known_hosts`.
#[tauri::command]
fn ssh_key_list() -> Result<Vec<SshKeyEntry>, String> {
    let ssh_dir = match resolve_ssh_dir() {
        Some(dir) => dir,
        None => return Ok(Vec::new()),
    };
    if !ssh_dir.is_dir() {
        return Ok(Vec::new());
    }

    let read = match fs::read_dir(&ssh_dir) {
        Ok(rd) => rd,
        Err(err) => return Err(format!("Cannot read {}: {err}", ssh_dir.display())),
    };

    let mut out: Vec<SshKeyEntry> = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = metadata.len();
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        let first_line = fs::read(&path)
            .ok()
            .map(|bytes| {
                let slice = &bytes[..bytes.len().min(1024)];
                let text = String::from_utf8_lossy(slice);
                text.lines().next().unwrap_or("").to_string()
            })
            .unwrap_or_default();

        let (kind, algorithm, comment) = classify_ssh_file(&name, &first_line);
        out.push(SshKeyEntry {
            name,
            path: path.to_string_lossy().into_owned(),
            kind,
            size,
            modified,
            comment,
            algorithm,
        });
    }
    out.sort_by(|a, b| {
        let rank = |k: &str| match k {
            "public" => 0,
            "private" => 1,
            "authorized_keys" => 2,
            "known_hosts" => 3,
            "config" => 4,
            _ => 5,
        };
        rank(&a.kind)
            .cmp(&rank(&b.kind))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

// ── Docker ──────────────────────────────────────────────────────────

#[tauri::command]
fn docker_overview(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    all: bool,
    saved_connection_index: Option<usize>,
) -> Result<DockerOverview, String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;

    // First-open path: containers only. Images / volumes / networks are
    // loaded by their own tab-specific commands when the user opens those
    // Docker tabs, which keeps the initial click to one Docker exec.
    let containers = docker::list_containers_blocking(&session, all)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|c| DockerContainerView {
            running: c.is_running(),
            cpu_perc: String::new(),
            mem_usage: String::new(),
            mem_perc: String::new(),
            id: c.id,
            image: c.image,
            names: c.names,
            status: c.status,
            state: c.state,
            created: c.created,
            ports: c.ports,
            labels: c.labels,
        })
        .collect();

    Ok(DockerOverview {
        containers,
        images: Vec::new(),
        volumes: Vec::new(),
        networks: Vec::new(),
    })
}

#[tauri::command]
fn docker_images(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<Vec<DockerImageView>, String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let images = docker::list_images_blocking(&session)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|i| DockerImageView {
            id: i.id,
            repository: i.repository,
            tag: i.tag,
            size: i.size,
            created: i.created,
        })
        .collect();
    Ok(images)
}

#[tauri::command]
fn docker_volumes(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<Vec<DockerVolumeView>, String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let volumes: Vec<DockerVolumeView> = docker::list_volumes_blocking(&session)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|v| DockerVolumeView {
            name: v.name,
            driver: v.driver,
            mountpoint: v.mountpoint,
            size: String::new(),
            size_bytes: 0,
            links: -1,
        })
        .collect();
    Ok(volumes)
}

#[tauri::command]
fn docker_networks(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<Vec<DockerNetworkView>, String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let networks = docker::list_networks_blocking(&session)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|n| DockerNetworkView {
            id: n.id,
            name: n.name,
            driver: n.driver,
            scope: n.scope,
        })
        .collect();
    Ok(networks)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DockerContainerStatsView {
    /// Container id the sample belongs to. UI merges by id / short id.
    id: String,
    cpu_perc: String,
    mem_usage: String,
    mem_perc: String,
}

#[tauri::command]
fn docker_stats(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<Vec<DockerContainerStatsView>, String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let stats = docker::list_container_stats_blocking(&session).unwrap_or_default();
    Ok(stats
        .into_iter()
        .map(|s| DockerContainerStatsView {
            id: s.id,
            cpu_perc: s.cpu_perc,
            mem_usage: s.mem_usage,
            mem_perc: s.mem_perc,
        })
        .collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DockerVolumeUsageView {
    name: String,
    size: String,
    size_bytes: u64,
    links: i64,
}

#[tauri::command]
fn docker_volume_usage(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<Vec<DockerVolumeUsageView>, String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let usages = docker::list_volume_sizes_blocking(&session).unwrap_or_default();
    Ok(usages
        .into_iter()
        .map(|v| DockerVolumeUsageView {
            size_bytes: docker::parse_size_to_bytes(&v.size),
            name: v.name,
            size: v.size,
            links: v.links,
        })
        .collect())
}

#[tauri::command]
fn docker_container_action(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    container_id: String,
    action: String,
    saved_connection_index: Option<usize>,
) -> Result<String, String> {
    run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| match action.as_str() {
            "start" => docker::start_blocking(session, &container_id)
                .map_err(|e| e.to_string())
                .map(|_| String::from("started")),
            "stop" => docker::stop_blocking(session, &container_id)
                .map_err(|e| e.to_string())
                .map(|_| String::from("stopped")),
            "restart" => docker::restart_blocking(session, &container_id)
                .map_err(|e| e.to_string())
                .map(|_| String::from("restarted")),
            "remove" => docker::remove_blocking(session, &container_id, false)
                .map_err(|e| e.to_string())
                .map(|_| String::from("removed")),
            _ => Err(format!("unknown docker action: {}", action)),
        },
    )
}

// ── SFTP ────────────────────────────────────────────────────────────

/// Resolve a sensible starting directory for the SFTP panel on the
/// remote host, using the already-authenticated session.
///
/// **Important caveat**: `$HOME` as reported by the server is a
/// declaration, not a guarantee. DSM in particular hands every user
/// a `$HOME` of `/var/services/homes/<user>` regardless of whether
/// that path actually exists — the server will happily tell ssh
/// "your home is X" and then print
/// `Could not chdir to home directory X: No such file or directory`
/// to the login shell. A naive `$HOME` probe would hand the SFTP
/// panel that dead path and the first `list_dir` would fail.
///
/// So instead of trusting `$HOME`, we build an ordered list of
/// candidate starting directories, and return the first one that
/// passes a cheap `test -d && test -r` probe. The list is:
///
///   1. The login shell's own `pwd` — what the terminal side lands
///      at after a real login; most robust because if `$HOME` was
///      invalid the shell already fell back to `/`.
///   2. `$HOME` as declared by the environment.
///   3. DSM-specific layout: `/volume<N>/homes/<user>` for N=1..=4,
///      which is where Synology's Home Service actually places per-
///      user directories. Probed only when the username looks safe
///      (ASCII alphanumerics and `._-` only) so we don't inject
///      weirdness into a shell-exec test.
///   4. `/volume1` — the most common top-level shared area on DSM.
///   5. `/` — always listable, last resort.
///
/// Not silver-bullet: if a user has no listable directory anywhere
/// on the host (extremely restrictive ACLs, no Home Service, no
/// share access), we still hand back `/` and the caller will see
/// whatever the server lets them see there. The point of this
/// function is to give a sane default, not to paper over
/// impossible-to-navigate filesystems.
/// Probe the remote for a sensible default starting directory.
///
/// Historically this issued 2–8 separate `exec_command` calls (one
/// each for `pwd`, `$HOME`, and one `test -d` per Synology volume
/// candidate). Every `exec_command` opens a fresh SSH channel, so
/// the cost added up to a very visible hiccup on the first SFTP
/// browse — especially over transoceanic links where each RTT was
/// 150–300 ms.
///
/// The script below walks the same candidate list inside a single
/// remote `sh -lc` invocation, `printf`s the first viable path, and
/// exits. One channel open, one round-trip. The `user` is inlined
/// because it's already validated by [`is_safe_shell_username`]
/// (ASCII alphanumerics plus `.`, `_`, `-`) — none of those
/// characters expand inside double-quoted shell context.
fn resolve_remote_home(session: &SshSession, user: &str) -> Result<String, String> {
    let volume_block = if is_safe_shell_username(user) {
        format!("for n in 1 2 3 4; do pick \"/volume$n/homes/{user}\"; done; ")
    } else {
        String::new()
    };
    // `pick` is a tiny shell function that validates and prints a
    // candidate; the first match exits the whole script via `exit 0`
    // so we stop as soon as we find one. `exit 1` at the end makes
    // the exec return a non-zero status if nothing matched.
    let script = format!(
        "sh -lc 'pick(){{ [ -d \"$1\" ] && [ -r \"$1\" ] && printf %s \"$1\" && exit 0; }}; \
         pick \"$(pwd 2>/dev/null)\"; \
         pick \"${{HOME:-}}\"; \
         {volume_block}\
         pick /volume1; \
         pick /; \
         exit 1'"
    );

    match session.exec_command_blocking(&script) {
        Ok((0, stdout)) => sanitise_absolute_path(&stdout)
            .ok_or_else(|| "home probe returned invalid path".to_string()),
        Ok(_) => Err("no listable directory found among candidates".into()),
        Err(e) => Err(e.to_string()),
    }
}

/// Cached wrapper around [`resolve_remote_home`]. The probe is
/// pure-ish (same host + same login → same answer for the life of
/// the session), so we only run it once per cached SSH session.
/// Invalidated when the SSH session is evicted.
fn resolve_remote_home_cached(
    state: &tauri::State<'_, AppState>,
    session: &SshSession,
    host: &str,
    port: u16,
    user: &str,
    auth_mode: &str,
) -> Result<String, String> {
    let key = sftp_cache_key(host, port, user, auth_mode);
    if let Ok(cache) = state.sftp_home_cache.lock() {
        if let Some(existing) = cache.get(&key) {
            return Ok(existing.clone());
        }
    }
    let home = resolve_remote_home(session, user)?;
    if let Ok(mut cache) = state.sftp_home_cache.lock() {
        cache.insert(key, home.clone());
    }
    Ok(home)
}

/// Cheap check: is `p` already a normalised absolute SFTP path that
/// we don't need to round-trip `canonicalize` for? The common
/// sources of `target_path` after the first browse (breadcrumb
/// click, "Up", cached `$HOME`) all satisfy this.
fn is_clean_absolute_path(p: &str) -> bool {
    if !p.starts_with('/') {
        return false;
    }
    !p.split('/').any(|seg| seg == "..")
}

fn sanitise_absolute_path(raw: &str) -> Option<String> {
    let p = raw.trim();
    if p.starts_with('/') && !p.contains('\0') && p.len() < 4096 {
        Some(p.to_string())
    } else {
        None
    }
}

fn is_safe_shell_username(user: &str) -> bool {
    !user.is_empty()
        && user.len() <= 64
        && user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[tauri::command]
fn sftp_browse(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    path: Option<String>,
    saved_connection_index: Option<usize>,
) -> Result<SftpBrowseState, String> {
    let explicit_path = path.filter(|p| !p.trim().is_empty());

    // Try with the cached session + cached SFTP subsystem first; if
    // anything fails (session stale, server bounced, SFTP channel
    // silently broken), evict and retry once with fresh handles.
    let mut attempt = 0;
    loop {
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;

        let sftp = match get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode) {
            Ok(s) => s,
            Err(e) if attempt == 0 => {
                evict_ssh_session(&state, &host, port, &user, &auth_mode);
                attempt += 1;
                let _ = e;
                continue;
            }
            Err(e) => return Err(e),
        };

        // Resolve the effective target path. An explicit caller-
        // supplied path (breadcrumb click, "Up", path-edit) wins.
        // Otherwise we probe the user's `$HOME` on the remote — on
        // Synology and any other multi-user host, `/` is the wrong
        // starting point (a non-root user typically has no listable
        // top-level entries besides a handful of system dirs, and
        // some DSM builds return permission errors on the first
        // attempt, which used to cascade into an SFTP panel that
        // looked hung). `$HOME` matches what the terminal would be
        // sitting at after a fresh login. If the probe fails, fall
        // back to `/`. The probe is cached per-session so only the
        // first browse pays the cost.
        let target_path = match explicit_path.clone() {
            Some(p) => p,
            None => resolve_remote_home_cached(&state, &session, &host, port, &user, &auth_mode)
                .unwrap_or_else(|_| "/".to_string()),
        };

        // Skip the canonicalize round-trip when the caller already
        // handed us a normalised absolute path — which is the
        // overwhelmingly common case (breadcrumb, cached $HOME,
        // `pwd` output). We only round-trip when the user typed
        // something with `..` segments.
        let canonical = if is_clean_absolute_path(&target_path) {
            target_path.clone()
        } else {
            sftp.canonicalize_blocking(&target_path)
                .unwrap_or_else(|_| target_path.clone())
        };

        let raw_entries = match sftp.list_dir_blocking(&canonical) {
            Ok(v) => v,
            Err(e) if attempt == 0 => {
                // list_dir failing on a cached SFTP client most often
                // means the subsystem went stale (server-side idle
                // timeout, or a dropped SSH connection). Evict both
                // the SFTP client and the SSH session so the retry
                // above re-handshakes from scratch.
                evict_ssh_session(&state, &host, port, &user, &auth_mode);
                attempt += 1;
                let _ = e;
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };

        let entries = raw_entries
            .into_iter()
            .filter(|entry| entry.name != "." && entry.name != "..")
            .map(|entry| SftpEntryView {
                permissions: entry
                    .permissions
                    .map(|p| format_posix_permissions(p, entry.is_dir, entry.is_link))
                    .unwrap_or_default(),
                modified: entry.modified,
                owner: entry.owner.clone().unwrap_or_default(),
                group: entry.group.clone().unwrap_or_default(),
                name: entry.name,
                path: entry.path,
                is_dir: entry.is_dir,
                size: entry.size,
            })
            .collect();

        return Ok(SftpBrowseState {
            current_path: canonical,
            entries,
        });
    }
}

// ── Markdown ────────────────────────────────────────────────────────

#[tauri::command]
fn markdown_render(source: String) -> String {
    markdown::render_html(&source)
}

#[tauri::command]
fn markdown_render_file(path: String) -> Result<String, String> {
    let source = markdown::load_file(std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    Ok(markdown::render_html(&source))
}

// ── Server Monitor ──────────────────────────────────────────────────

// `include_disks=true` runs the full probe (CPU/memory/network + `df`
// + `lsblk`); `include_disks=false` skips the disk segments so the
// fast 5 s tier doesn't burn SSH / remote CPU re-running them every
// poll. The frontend caches the prior full snapshot's disks in between.
#[tauri::command]
fn server_monitor_probe(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    include_disks: bool,
) -> Result<ServerSnapshotView, String> {
    // Reuse the shared SSH session cache so each 5-second poll
    // doesn't re-handshake. When the terminal for this tab is
    // already up its session is in the cache and we hit it; on a
    // local terminal that just typed `ssh user@host`, the first
    // probe primes the cache and every subsequent poll reuses it.
    // On `probe_blocking` failure we evict the cache entry and
    // retry once — covers the case where the cached session
    // silently went stale (server bounced, idle keepalive timeout).
    let baseline_key = sftp_cache_key(&host, port, &user, &auth_mode);
    let mut attempt = 0;
    let snap = loop {
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )
        .map_err(|e| {
            pier_core::logging::write_event(
                "ERROR",
                "monitor.probe",
                &format!("{}@{}:{} session open failed: {}", user, host, port, e),
            );
            e
        })?;
        // Pull the previous net sample (if any), pass it through the
        // probe, then save the updated sample back. Holding the
        // mutex only across the load and store keeps the long
        // network probe out of the lock.
        let mut local_baseline = state
            .monitor_net_baselines
            .lock()
            .ok()
            .and_then(|guard| guard.get(&baseline_key).copied());
        match server_monitor::probe_with_baseline_blocking(
            &session,
            &mut local_baseline,
            include_disks,
        ) {
            Ok(snap) => {
                if let Some(sample) = local_baseline {
                    if let Ok(mut guard) = state.monitor_net_baselines.lock() {
                        guard.insert(baseline_key.clone(), sample);
                    }
                }
                break snap;
            }
            Err(e) if attempt == 0 => {
                pier_core::logging::write_event(
                    "WARN",
                    "monitor.probe",
                    &format!(
                        "{}@{}:{} probe attempt 1 failed, evicting + retrying: {}",
                        user, host, port, e
                    ),
                );
                evict_ssh_session(&state, &host, port, &user, &auth_mode);
                attempt += 1;
                continue;
            }
            Err(e) => {
                pier_core::logging::write_event(
                    "ERROR",
                    "monitor.probe",
                    &format!("{}@{}:{} probe failed after retry: {}", user, host, port, e),
                );
                return Err(e.to_string());
            }
        }
    };

    Ok(ServerSnapshotView {
        uptime: snap.uptime,
        load_1: snap.load_1,
        load_5: snap.load_5,
        load_15: snap.load_15,
        mem_total_mb: snap.mem_total_mb,
        mem_used_mb: snap.mem_used_mb,
        mem_free_mb: snap.mem_free_mb,
        swap_total_mb: snap.swap_total_mb,
        swap_used_mb: snap.swap_used_mb,
        disk_total: snap.disk_total,
        disk_used: snap.disk_used,
        disk_avail: snap.disk_avail,
        disk_use_pct: snap.disk_use_pct,
        cpu_pct: snap.cpu_pct,
        cpu_count: snap.cpu_count,
        proc_count: snap.proc_count,
        os_label: snap.os_label,
        net_rx_bps: snap.net_rx_bps,
        net_tx_bps: snap.net_tx_bps,
        top_processes: snap
            .top_processes
            .into_iter()
            .map(|p| ProcessRowView {
                pid: p.pid,
                command: p.command,
                cpu_pct: p.cpu_pct,
                mem_pct: p.mem_pct,
                elapsed: p.elapsed,
            })
            .collect(),
        top_processes_mem: snap
            .top_processes_mem
            .into_iter()
            .map(|p| ProcessRowView {
                pid: p.pid,
                command: p.command,
                cpu_pct: p.cpu_pct,
                mem_pct: p.mem_pct,
                elapsed: p.elapsed,
            })
            .collect(),
        disks: snap
            .disks
            .into_iter()
            .map(|d| DiskEntryView {
                filesystem: d.filesystem,
                fs_type: d.fs_type,
                total: d.total,
                used: d.used,
                avail: d.avail,
                use_pct: d.use_pct,
                mountpoint: d.mountpoint,
            })
            .collect(),
        block_devices: snap
            .block_devices
            .into_iter()
            .map(|b| BlockDeviceEntryView {
                name: b.name,
                kname: b.kname,
                pkname: b.pkname,
                dev_type: b.dev_type,
                size_bytes: b.size_bytes,
                rota: b.rota,
                tran: b.tran,
                model: b.model,
                fs_type: b.fs_type,
                mountpoint: b.mountpoint,
            })
            .collect(),
    })
}

// ── Firewall ──────────────────────────────────────────────────────

#[tauri::command]
fn firewall_snapshot(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<firewall::FirewallSnapshot, String> {
    // Same SSH session reuse pattern as `server_monitor_probe` —
    // every refresh hits the cached russh handle. One full snapshot
    // is one `exec_command` (the probe script chains via shell), so
    // amortising the handshake matters when the panel polls on a
    // 2-second cadence for the Traffic tab.
    let mut attempt = 0;
    let snap = loop {
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        match firewall::snapshot_blocking(&session) {
            Ok(s) => break s,
            Err(e) if attempt == 0 => {
                evict_ssh_session(&state, &host, port, &user, &auth_mode);
                attempt += 1;
                let _ = e;
                continue;
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    Ok(snap)
}

// ── SSH ControlMaster (terminal-side mux) ────────────────────────

/// Frontend mirror of [`ssh_mux::MuxSettings`]. Kept as a separate
/// type so a future schema change here doesn't ripple through the
/// internal struct's field naming conventions.
#[derive(serde::Serialize, serde::Deserialize)]
struct SshMuxSettingsView {
    enabled: bool,
    persist_seconds: u32,
}

impl From<ssh_mux::MuxSettings> for SshMuxSettingsView {
    fn from(s: ssh_mux::MuxSettings) -> Self {
        Self {
            enabled: s.enabled,
            persist_seconds: s.persist_seconds,
        }
    }
}

#[tauri::command]
fn ssh_mux_get_settings() -> SshMuxSettingsView {
    ssh_mux::settings().into()
}

#[tauri::command]
fn ssh_mux_set_settings(enabled: bool, persist_seconds: u32) -> Result<(), String> {
    // Clamp to a sane band — under 10s the master barely covers a
    // shell open + close cycle (worse UX than no mux), and over 24h
    // is just "leak forever". Frontend slider should use the same
    // bounds so the user never sees a silent clamp.
    let clamped = persist_seconds.clamp(10, 86_400);
    ssh_mux::set_settings(ssh_mux::MuxSettings {
        enabled,
        persist_seconds: clamped,
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn ssh_mux_forget_target(host: String, port: u16, user: String) -> Result<(), String> {
    ssh_mux::forget_target(&host, port, &user).map_err(|e| e.to_string())
}

#[tauri::command]
fn ssh_mux_shutdown_all() -> usize {
    ssh_mux::shutdown_all_masters()
}

// ── SSH credential cache (process-level, in-memory) ──────────────

/// Mirror a password the terminal-side ssh just successfully used
/// into the process-level credential cache, so right-side panels
/// (firewall, monitor, SFTP, Docker, DB) can reach the same target
/// without re-prompting. Empty `password` is a no-op (we never
/// cache the empty string — that's how "no credential captured yet"
/// is represented).
#[tauri::command]
fn ssh_cred_cache_put_password(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    password: String,
) {
    state
        .ssh_cred_cache
        .put_password(TargetKey::new(&host, port, &user), &password);
}

/// Same shape as [`ssh_cred_cache_put_password`] but writes the key
/// passphrase slot — the value the user typed at OpenSSH's
/// `Enter passphrase for key '<path>':` prompt. Kept separate so a
/// passphrase never gets mistakenly attempted as a server password.
#[tauri::command]
fn ssh_cred_cache_put_passphrase(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    passphrase: String,
) {
    state
        .ssh_cred_cache
        .put_passphrase(TargetKey::new(&host, port, &user), &passphrase);
}

/// Drop everything we know about `(host, port, user)`. Wired into
/// the "Forget this connection's credentials" right-click affordance.
/// Also tears down any live ControlMaster master for the same target
/// so subsequent ssh re-authenticates from scratch — this is the
/// user-facing "log out" gesture.
#[tauri::command]
fn ssh_cred_cache_forget(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
) {
    state
        .ssh_cred_cache
        .forget(&TargetKey::new(&host, port, &user));
    let _ = ssh_mux::forget_target(&host, port, &user);
    // Also evict the russh session cache so the next right-side
    // panel call doesn't keep talking to a connection that
    // semantically belongs to the now-forgotten credential.
    evict_ssh_session(&state, &host, port, &user, "auto");
    evict_ssh_session(&state, &host, port, &user, "password");
    evict_ssh_session(&state, &host, port, &user, "key");
    evict_ssh_session(&state, &host, port, &user, "agent");
}

// ── Service Detection ────────────────────────────────────────────

#[tauri::command]
fn detect_services(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<Vec<DetectedServiceView>, String> {
    // Same shared-cache strategy as `server_monitor_probe`: reuse
    // the terminal's russh handle when it's already there, prime
    // the cache otherwise. The detector runs several `which` /
    // `--version` probes serially over one SSH session, so a fresh
    // handshake per call is wasteful on slow links.
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;

    let services = service_detector::detect_all_blocking(&session);
    Ok(services
        .into_iter()
        .map(|s| DetectedServiceView {
            name: s.name,
            version: s.version,
            status: format!("{:?}", s.status),
            port: s.port,
        })
        .collect())
}

// ── DB Instance Detection ───────────────────────────────────────

/// Detect reachable DB instances (MySQL / PostgreSQL / Redis)
/// on the remote host, combining docker + listening-socket
/// probes. Lightweight: runs all probes concurrently over the
/// already-open SSH session cache. See
/// [`pier_core::ssh::db_detect`] for the algorithm.
#[tauri::command]
fn db_detect(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<DbDetectionReportView, String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let report = db_detect::detect_blocking(&session);
    Ok(map_db_detection_report(report))
}

// ── DB Credential CRUD ──────────────────────────────────────────

#[tauri::command]
fn db_cred_save(
    saved_connection_index: usize,
    credential: DbCredentialInput,
    password: Option<String>,
) -> Result<DbCredentialView, String> {
    let kind = parse_db_kind(&credential.kind)?;
    let source = match credential.detection_signature {
        Some(sig) if !sig.is_empty() => DbCredentialSource::Detected { signature: sig },
        _ => DbCredentialSource::Manual,
    };
    let input = NewDbCredential {
        kind,
        label: credential.label,
        host: credential.host,
        port: credential.port,
        user: credential.user,
        database: credential.database,
        sqlite_path: credential.sqlite_path,
        favorite: credential.favorite,
        source,
    };
    let cred = connections::save_db_credential(saved_connection_index, input, password)
        .map_err(|e| e.to_string())?;
    Ok(map_db_credential(&cred))
}

#[tauri::command]
fn db_cred_update(
    saved_connection_index: usize,
    credential_id: String,
    patch: DbCredentialPatchInput,
    new_password: Option<Option<String>>,
) -> Result<DbCredentialView, String> {
    let patch = DbCredentialPatch {
        label: patch.label,
        host: patch.host,
        port: patch.port,
        user: patch.user,
        database: patch.database,
        sqlite_path: patch.sqlite_path,
        favorite: patch.favorite,
    };
    let cred = connections::update_db_credential(
        saved_connection_index,
        &credential_id,
        patch,
        new_password,
    )
    .map_err(|e| e.to_string())?;
    Ok(map_db_credential(&cred))
}

#[tauri::command]
fn db_cred_delete(saved_connection_index: usize, credential_id: String) -> Result<(), String> {
    connections::delete_db_credential(saved_connection_index, &credential_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn db_cred_resolve(
    saved_connection_index: usize,
    credential_id: String,
) -> Result<DbCredentialResolvedView, String> {
    let resolved = connections::resolve_db_credential(saved_connection_index, &credential_id)
        .map_err(|e| e.to_string())?;
    Ok(map_resolved_credential(resolved))
}

#[tauri::command]
fn docker_inspect_db_env(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    container_id: String,
    saved_connection_index: Option<usize>,
) -> Result<DockerDbEnvView, String> {
    let env = run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| {
            docker::inspect_db_env_blocking(session, &container_id).map_err(|e| e.to_string())
        },
    )?;
    Ok(DockerDbEnvView {
        mysql_database: env.mysql_database,
        mysql_user: env.mysql_user,
        postgres_db: env.postgres_db,
        postgres_user: env.postgres_user,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DockerDbEnvView {
    mysql_database: Option<String>,
    mysql_user: Option<String>,
    postgres_db: Option<String>,
    postgres_user: Option<String>,
}

// ── Remote SQLite ───────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteSqliteCapabilityView {
    installed: bool,
    version: Option<String>,
    supports_json: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteSqliteInstallReportView {
    /// One of `installed` / `unsupported-distro` / `sudo-requires-password`
    /// / `package-manager-failed`. Mirrors the kebab-case tag emitted by
    /// `RemoteSqliteInstallStatus`'s serde representation so the frontend
    /// can match on a flat string.
    status: String,
    distro_id: String,
    package_manager: String,
    command: String,
    exit_code: i32,
    output_tail: String,
    installed_version: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteSqliteBrowserState {
    path: String,
    table_name: String,
    tables: Vec<String>,
    columns: Vec<SqliteColumnView>,
    preview: Option<DataPreview>,
    /// Remote indexes / triggers are not introspected yet — the
    /// remote `sqlite3` worker only runs queries, not PRAGMAs that
    /// require multiple round-trips. The fields exist so the panel
    /// can render a single `SqliteBrowserState` shape regardless of
    /// local-vs-remote source.
    indexes: Vec<SqliteIndexView>,
    triggers: Vec<SqliteTriggerView>,
    /// File size from the candidate listing (`sqliteFindInDir`)
    /// when known; 0 when the panel opened the path manually.
    file_size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteSqliteCandidate {
    path: String,
    size_bytes: u64,
    modified: Option<i64>,
}

#[tauri::command]
fn sqlite_remote_capable(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<RemoteSqliteCapabilityView, String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let cap = sqlite_remote::probe_blocking(&session);
    Ok(RemoteSqliteCapabilityView {
        installed: cap.installed,
        version: cap.version,
        supports_json: cap.supports_json,
    })
}

#[tauri::command]
async fn sqlite_install_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<RemoteSqliteInstallReportView, String> {
    // `apt-get update + install` over SSH routinely takes 20–60s. Running
    // that on the IPC worker would starve every other invoke (server
    // monitor probes, terminal CWD polling, panel refreshes) and the
    // whole UI looks frozen. Push the wait onto tokio's blocking pool
    // instead — same pattern as `local_docker_overview`.
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let report = sqlite_remote::install_blocking(&session).map_err(|e| e.to_string())?;
        let status = match report.status {
            sqlite_remote::RemoteSqliteInstallStatus::Installed => "installed",
            sqlite_remote::RemoteSqliteInstallStatus::UnsupportedDistro => "unsupported-distro",
            sqlite_remote::RemoteSqliteInstallStatus::SudoRequiresPassword => {
                "sudo-requires-password"
            }
            sqlite_remote::RemoteSqliteInstallStatus::PackageManagerFailed => {
                "package-manager-failed"
            }
        };
        Ok::<_, String>(RemoteSqliteInstallReportView {
            status: status.to_string(),
            distro_id: report.distro_id,
            package_manager: report.package_manager,
            command: report.command,
            exit_code: report.exit_code,
            output_tail: report.output_tail,
            installed_version: report.installed_version,
        })
    })
    .await
    .map_err(|e| format!("sqlite_install_remote join: {e}"))?
}

// ── Software panel ─────────────────────────────────────────────

const SOFTWARE_INSTALL_EVENT: &str = "software-install";
const SOFTWARE_UNINSTALL_EVENT: &str = "software-uninstall";

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwareDescriptorView {
    id: String,
    display_name: String,
    notes: Option<String>,
    has_service: bool,
    /// Filesystem dirs declared as user data on the descriptor —
    /// surfaced to the uninstall dialog so it can list them in the
    /// "also delete data directories" warning. Empty for stateless
    /// software.
    data_dirs: Vec<String>,
    /// `true` when the daemon supports `systemctl reload` without a
    /// downtime restart. Drives the "Reload (no downtime)" entry in
    /// the row's service menu (currently only nginx).
    supports_reload: bool,
    /// `Some(_)` when the descriptor exposes a v2 vendor-script
    /// install path (e.g. Docker → `https://get.docker.com`). The
    /// frontend renders the install button as a split-button when
    /// this is non-null. `None` = only the default apt / dnf / …
    /// path is offered.
    vendor_script: Option<VendorScriptDescriptorView>,
    /// Major-version variants (e.g. OpenJDK 8/11/17/21). Empty for
    /// single-version software. When non-empty, the panel renders a
    /// variant picker before install and routes the user's choice
    /// back through the install command's `variantKey` field.
    version_variants: Vec<SoftwareVersionVariantView>,
    /// Common config files declared on the descriptor — surfaced in
    /// the row's expanded details pane (filtered through `test -e`
    /// before display so stale entries never reach the UI).
    config_paths: Vec<String>,
    /// Default network ports this software listens on. Surfaced as
    /// "default" alongside an `ss -ltn` probe in the details pane.
    default_ports: Vec<u16>,
    /// App-store category (`database` / `web` / `runtime` / …).
    /// Drives the panel's section grouping. Empty = "其它".
    category: String,
}

/// Static view of one [`package_manager::VersionVariant`] entry.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwareVersionVariantView {
    key: String,
    label: String,
}

/// View of [`package_manager::VendorScriptDescriptor`] — same fields,
/// camelCase'd for the frontend.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct VendorScriptDescriptorView {
    label: String,
    url: String,
    notes: String,
    conflicts_with_apt: bool,
    /// `true` when the descriptor has any `cleanup_scripts` entries.
    /// Drives whether the uninstall dialog renders the "remove
    /// upstream source" checkbox.
    has_cleanup_scripts: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct HostPackageEnvView {
    distro_id: String,
    distro_pretty: String,
    package_manager: Option<String>,
    is_root: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PackageStatusView {
    id: String,
    installed: bool,
    version: Option<String>,
    service_active: Option<bool>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwareProbeView {
    env: HostPackageEnvView,
    statuses: Vec<PackageStatusView>,
}

/// View of [`package_manager::PackageDetail`]. Lazy-loaded — the panel
/// fetches this only when the user expands the row, so the slow
/// candidate-version + ss probes don't block the first paint.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwarePackageDetailView {
    package_id: String,
    installed: bool,
    install_paths: Vec<String>,
    config_paths: Vec<String>,
    default_ports: Vec<u16>,
    listening_ports: Vec<u16>,
    listen_probe_ok: bool,
    service_unit: Option<String>,
    latest_version: Option<String>,
    installed_version: Option<String>,
    variants: Vec<SoftwarePackageVariantStatusView>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwarePackageVariantStatusView {
    key: String,
    label: String,
    installed: bool,
    installed_version: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwareInstallReportView {
    package_id: String,
    /// One of `installed` / `unsupported-distro` / `sudo-requires-password`
    /// / `package-manager-failed` / `vendor-script-download-failed` /
    /// `vendor-script-failed`. Mirrors the kebab-case tag emitted by
    /// `package_manager::InstallStatus`.
    status: String,
    distro_id: String,
    package_manager: String,
    command: String,
    exit_code: i32,
    output_tail: String,
    installed_version: Option<String>,
    service_active: Option<bool>,
    /// `Some(_)` when the install ran via the v2 vendor-script
    /// channel. Carries the label + URL the user picked so the
    /// frontend can render `via {label} ({url})` in the activity log
    /// without re-reading the registry.
    vendor_script: Option<VendorScriptUsedView>,
}

/// View of [`package_manager::VendorScriptUsedView`] — same fields,
/// camelCase'd for the frontend.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct VendorScriptUsedView {
    label: String,
    url: String,
}

/// Streaming event payload — a flat shape so the frontend can listen
/// once and dispatch on `kind`.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwareInstallEvent {
    install_id: String,
    /// `"line"` (during run), `"done"` (final report), or `"failed"`
    /// (the spawn task itself errored).
    kind: String,
    text: Option<String>,
    report: Option<SoftwareInstallReportView>,
    message: Option<String>,
}

/// View of `package_manager::UninstallReport`. Mirrors the install
/// report layout so the frontend's outcome card is shape-compatible.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwareUninstallReportView {
    package_id: String,
    /// `uninstalled` / `unsupported-distro` / `sudo-requires-password`
    /// / `package-manager-failed` / `not-installed`.
    status: String,
    distro_id: String,
    package_manager: String,
    command: String,
    exit_code: i32,
    output_tail: String,
    data_dirs_removed: bool,
}

/// Uninstall-side streaming event. Lives on its own channel
/// (`SOFTWARE_UNINSTALL_EVENT`) so the report shape can differ from
/// the install event without ugly union encoding.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwareUninstallEvent {
    install_id: String,
    /// `"line"` (during run), `"done"` (final report), or `"failed"`
    /// (the spawn task itself errored).
    kind: String,
    text: Option<String>,
    report: Option<SoftwareUninstallReportView>,
    message: Option<String>,
}

fn package_manager_to_string(m: package_manager::PackageManager) -> String {
    m.as_str().to_string()
}

fn install_status_kebab(status: package_manager::InstallStatus) -> &'static str {
    match status {
        package_manager::InstallStatus::Installed => "installed",
        package_manager::InstallStatus::UnsupportedDistro => "unsupported-distro",
        package_manager::InstallStatus::SudoRequiresPassword => "sudo-requires-password",
        package_manager::InstallStatus::PackageManagerFailed => "package-manager-failed",
        package_manager::InstallStatus::Cancelled => "cancelled",
        package_manager::InstallStatus::VendorScriptDownloadFailed => {
            "vendor-script-download-failed"
        }
        package_manager::InstallStatus::VendorScriptFailed => "vendor-script-failed",
    }
}

fn uninstall_status_kebab(status: package_manager::UninstallStatus) -> &'static str {
    match status {
        package_manager::UninstallStatus::Uninstalled => "uninstalled",
        package_manager::UninstallStatus::UnsupportedDistro => "unsupported-distro",
        package_manager::UninstallStatus::SudoRequiresPassword => "sudo-requires-password",
        package_manager::UninstallStatus::PackageManagerFailed => "package-manager-failed",
        package_manager::UninstallStatus::NotInstalled => "not-installed",
        package_manager::UninstallStatus::Cancelled => "cancelled",
    }
}

fn report_to_view(report: package_manager::InstallReport) -> SoftwareInstallReportView {
    SoftwareInstallReportView {
        package_id: report.package_id,
        status: install_status_kebab(report.status).to_string(),
        distro_id: report.distro_id,
        package_manager: report.package_manager,
        command: report.command,
        exit_code: report.exit_code,
        output_tail: report.output_tail,
        installed_version: report.installed_version,
        service_active: report.service_active,
        vendor_script: report.vendor_script.map(|v| VendorScriptUsedView {
            label: v.label,
            url: v.url,
        }),
    }
}

fn uninstall_report_to_view(
    report: package_manager::UninstallReport,
) -> SoftwareUninstallReportView {
    SoftwareUninstallReportView {
        package_id: report.package_id,
        status: uninstall_status_kebab(report.status).to_string(),
        distro_id: report.distro_id,
        package_manager: report.package_manager,
        command: report.command,
        exit_code: report.exit_code,
        output_tail: report.output_tail,
        data_dirs_removed: report.data_dirs_removed,
    }
}

#[tauri::command]
fn software_registry() -> Vec<SoftwareDescriptorView> {
    package_manager::registry()
        .iter()
        .map(|d| SoftwareDescriptorView {
            id: d.id.to_string(),
            display_name: d.display_name.to_string(),
            notes: d.notes.map(str::to_string),
            has_service: !d.service_units.is_empty(),
            data_dirs: d.data_dirs.iter().map(|s| (*s).to_string()).collect(),
            supports_reload: d.supports_reload,
            vendor_script: d.vendor_script.map(|v| VendorScriptDescriptorView {
                label: v.label.to_string(),
                url: v.url.to_string(),
                notes: v.notes.to_string(),
                conflicts_with_apt: v.conflicts_with_apt,
                has_cleanup_scripts: !v.cleanup_scripts.is_empty(),
            }),
            version_variants: d
                .version_variants
                .iter()
                .map(|v| SoftwareVersionVariantView {
                    key: v.key.to_string(),
                    label: v.label.to_string(),
                })
                .collect(),
            config_paths: d.config_paths.iter().map(|s| (*s).to_string()).collect(),
            default_ports: d.default_ports.to_vec(),
            category: d.category.to_string(),
        })
        .collect()
}

#[tauri::command]
async fn software_probe_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<SoftwareProbeView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let env = package_manager::probe_host_env_blocking(&session);
        let statuses = package_manager::probe_all_blocking(&session);
        Ok::<_, String>(SoftwareProbeView {
            env: HostPackageEnvView {
                distro_id: env.distro_id,
                distro_pretty: env.distro_pretty,
                package_manager: env.package_manager.map(package_manager_to_string),
                is_root: env.is_root,
            },
            statuses: statuses
                .into_iter()
                .map(|s| PackageStatusView {
                    id: s.id,
                    installed: s.installed,
                    version: s.version,
                    service_active: s.service_active,
                })
                .collect(),
        })
    })
    .await
    .map_err(|e| format!("software_probe_remote join: {e}"))?
}

/// Run a streaming install or update. The synchronous body emits a
/// `line` event per stdout/stderr line and a final `done` event with
/// the structured report; on join failure we emit `failed`.
///
/// Registers a [`CancellationToken`] keyed by `install_id` in
/// `AppState.software_cancel` so a sibling `software_install_cancel`
/// invocation can flip it. The token is removed in every exit path —
/// success, package-manager failure, runtime cancel, or join error —
/// so a stale entry can't accumulate.
/// `via_vendor_script` is only meaningful when `is_update == false` —
/// the v2 channel is install-only; updates always use the default
/// package-manager path because the official installers (e.g.
/// get.docker.com) are idempotent installers, not upgrade scripts.
async fn software_install_or_update_inner(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    package_id: String,
    install_id: String,
    enable_service: bool,
    version: Option<String>,
    variant_key: Option<String>,
    is_update: bool,
    via_vendor_script: bool,
) -> Result<SoftwareInstallReportView, String> {
    let app_for_failure = app.clone();
    let install_id_for_failure = install_id.clone();
    let token = register_software_cancel(&app, &install_id);
    let token_for_task = token.clone();
    let join = tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let app_for_lines = app.clone();
        let install_id_for_lines = install_id.clone();
        let on_line = move |line: &str| {
            let _ = app_for_lines.emit(
                SOFTWARE_INSTALL_EVENT,
                SoftwareInstallEvent {
                    install_id: install_id_for_lines.clone(),
                    kind: "line".to_string(),
                    text: Some(line.to_string()),
                    report: None,
                    message: None,
                },
            );
        };
        let version_ref = version.as_deref();
        let variant_ref = variant_key.as_deref();
        let report = if is_update {
            package_manager::update_blocking(
                &session,
                &package_id,
                enable_service,
                version_ref,
                variant_ref,
                on_line,
                Some(token_for_task.clone()),
            )
        } else if via_vendor_script {
            package_manager::install_via_script_blocking(
                &session,
                &package_id,
                enable_service,
                variant_ref,
                on_line,
                Some(token_for_task.clone()),
            )
        } else {
            package_manager::install_blocking(
                &session,
                &package_id,
                enable_service,
                version_ref,
                variant_ref,
                on_line,
                Some(token_for_task.clone()),
            )
        }
        .map_err(|e| e.to_string())?;
        let view = report_to_view(report);
        // When pier-core decided the run was cancelled, emit a
        // `cancelled` event instead of `done`. The frontend treats
        // cancelled as terminal (same as done/failed) and clears the
        // row's busy state, but the activity log carries the localized
        // "cancelled" outcome rather than a fake-success report.
        let kind = if view.status == "cancelled" {
            "cancelled"
        } else {
            "done"
        };
        let _ = app.emit(
            SOFTWARE_INSTALL_EVENT,
            SoftwareInstallEvent {
                install_id: install_id.clone(),
                kind: kind.to_string(),
                text: None,
                report: Some(view.clone()),
                message: None,
            },
        );
        Ok::<_, String>(view)
    })
    .await;
    unregister_software_cancel(&app_for_failure, &install_id_for_failure);
    match join {
        Ok(inner) => inner,
        Err(e) => {
            let msg = format!("software install join: {e}");
            let _ = app_for_failure.emit(
                SOFTWARE_INSTALL_EVENT,
                SoftwareInstallEvent {
                    install_id: install_id_for_failure,
                    kind: "failed".to_string(),
                    text: None,
                    report: None,
                    message: Some(msg.clone()),
                },
            );
            Err(msg)
        }
    }
}

/// Register a fresh cancellation token under `install_id` and return a
/// clone for the task to consume. If a task with the same id is
/// already registered (the frontend should never do this, but guard
/// anyway), the prior token is dropped — the prior task keeps its
/// clone and continues unaffected, but a subsequent cancel call can
/// only reach the new task.
fn register_software_cancel(app: &tauri::AppHandle, install_id: &str) -> CancellationToken {
    let state: tauri::State<'_, AppState> = app.state();
    let token = CancellationToken::new();
    if let Ok(mut map) = state.software_cancel.lock() {
        map.insert(install_id.to_string(), token.clone());
    }
    token
}

fn unregister_software_cancel(app: &tauri::AppHandle, install_id: &str) {
    let state: tauri::State<'_, AppState> = app.state();
    if let Ok(mut map) = state.software_cancel.lock() {
        map.remove(install_id);
    };
}

#[tauri::command]
async fn software_install_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    package_id: String,
    install_id: String,
    enable_service: bool,
    version: Option<String>,
    variant_key: Option<String>,
    via_vendor_script: Option<bool>,
) -> Result<SoftwareInstallReportView, String> {
    // `via_vendor_script == Some(true)` routes through the descriptor's
    // vendor_script channel (download + run the official installer)
    // instead of the default package-manager path. The frontend's
    // confirm dialog gates this — there's no UI path that sets the
    // flag without an explicit user opt-in.
    software_install_or_update_inner(
        app,
        host,
        port,
        user,
        auth_mode,
        password,
        key_path,
        saved_connection_index,
        package_id,
        install_id,
        enable_service,
        version,
        variant_key,
        false,
        via_vendor_script.unwrap_or(false),
    )
    .await
}

#[tauri::command]
async fn software_update_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    package_id: String,
    install_id: String,
    enable_service: bool,
    version: Option<String>,
    variant_key: Option<String>,
) -> Result<SoftwareInstallReportView, String> {
    software_install_or_update_inner(
        app,
        host,
        port,
        user,
        auth_mode,
        password,
        key_path,
        saved_connection_index,
        package_id,
        install_id,
        enable_service,
        version,
        variant_key,
        true,
        false,
    )
    .await
}

/// Enumerate package-manager-visible versions for a descriptor on the
/// remote host. Returns an empty Vec on unsupported distro / pacman /
/// queries that produce no rows. The frontend caches the result for
/// 5 minutes per host+package.
#[tauri::command]
async fn software_versions_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    package_id: String,
    variant_key: Option<String>,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        package_manager::available_versions_blocking(
            &session,
            &package_id,
            variant_key.as_deref(),
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("software_versions_remote join: {e}"))?
}

/// Lazy details probe for the row's expand pane. Runs install-path,
/// config-path existence, listening-port, candidate-version, and
/// per-variant probes. Frontend invokes this only when the user clicks
/// the disclosure so the panel's first paint is unaffected.
#[tauri::command]
async fn software_details_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    package_id: String,
) -> Result<SoftwarePackageDetailView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let detail = package_manager::probe_details_blocking(&session, &package_id)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(SoftwarePackageDetailView {
            package_id: detail.package_id,
            installed: detail.installed,
            install_paths: detail.install_paths,
            config_paths: detail.config_paths,
            default_ports: detail.default_ports,
            listening_ports: detail.listening_ports,
            listen_probe_ok: detail.listen_probe_ok,
            service_unit: detail.service_unit,
            latest_version: detail.latest_version,
            installed_version: detail.installed_version,
            variants: detail
                .variants
                .into_iter()
                .map(|v| SoftwarePackageVariantStatusView {
                    key: v.key,
                    label: v.label,
                    installed: v.installed,
                    installed_version: v.installed_version,
                })
                .collect(),
        })
    })
    .await
    .map_err(|e| format!("software_details_remote join: {e}"))?
}

// ── Bundles (v2.6) ─────────────────────────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwareBundleView {
    id: String,
    display_name: String,
    description: String,
    package_ids: Vec<String>,
}

/// Look up co-install suggestions for `id`. Static data — no
/// remote call needed; the panel uses this to render the chip
/// strip after a successful install.
#[tauri::command]
fn software_co_install_suggestions(id: String) -> Vec<String> {
    package_manager::co_install_suggestions(&id)
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Static catalog of curated bundles. Same shape every call; the
/// frontend renders these as one-click cards above the registry list.
#[tauri::command]
fn software_bundles() -> Vec<SoftwareBundleView> {
    package_manager::bundles()
        .iter()
        .map(|b| SoftwareBundleView {
            id: b.id.to_string(),
            display_name: b.display_name.to_string(),
            description: b.description.to_string(),
            package_ids: b.package_ids.iter().map(|s| (*s).to_string()).collect(),
        })
        .collect()
}

// ── Install-command preview (v2.5) ────────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InstallCommandPreviewView {
    package_id: String,
    package_manager: String,
    is_root: bool,
    inner_command: String,
    wrapped_command: String,
}

/// Synthesise the install command without running it. The panel
/// uses this to feed the "复制安装命令" menu entry — users who
/// don't want pier-x running sudo on their behalf can paste the
/// printed command into their own SSH session.
#[tauri::command]
async fn software_install_preview(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    package_id: String,
    version: Option<String>,
    variant_key: Option<String>,
    is_update: bool,
) -> Result<InstallCommandPreviewView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let preview = package_manager::install_command_preview_blocking(
            &session,
            &package_id,
            version.as_deref(),
            variant_key.as_deref(),
            is_update,
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(InstallCommandPreviewView {
            package_id: preview.package_id,
            package_manager: preview.package_manager,
            is_root: preview.is_root,
            inner_command: preview.inner_command,
            wrapped_command: preview.wrapped_command,
        })
    })
    .await
    .map_err(|e| format!("software_install_preview join: {e}"))?
}

// ── PostgreSQL service-level orchestration (v2.8) ──────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PostgresActionReportView {
    status: String,
    command: String,
    exit_code: i32,
    output_tail: String,
}

fn pg_report_to_view(r: package_manager::PostgresActionReport) -> PostgresActionReportView {
    PostgresActionReportView {
        status: r.status,
        command: r.command,
        exit_code: r.exit_code,
        output_tail: r.output_tail,
    }
}

#[tauri::command]
async fn postgres_create_user_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    pg_username: String,
    pg_password: String,
    is_superuser: bool,
) -> Result<PostgresActionReportView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path,
            saved_connection_index,
        )?;
        let report = package_manager::postgres_create_user_blocking(
            &session,
            &pg_username,
            &pg_password,
            is_superuser,
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(pg_report_to_view(report))
    })
    .await
    .map_err(|e| format!("postgres_create_user_remote join: {e}"))?
}

#[tauri::command]
async fn postgres_create_db_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    db_name: String,
    owner: String,
) -> Result<PostgresActionReportView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path,
            saved_connection_index,
        )?;
        let report = package_manager::postgres_create_db_blocking(&session, &db_name, &owner)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(pg_report_to_view(report))
    })
    .await
    .map_err(|e| format!("postgres_create_db_remote join: {e}"))?
}

#[tauri::command]
async fn postgres_open_remote_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<PostgresActionReportView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path,
            saved_connection_index,
        )?;
        let report = package_manager::postgres_open_remote_blocking(&session)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(pg_report_to_view(report))
    })
    .await
    .map_err(|e| format!("postgres_open_remote_remote join: {e}"))?
}

// ── DB metrics (v2.13) ───────────────────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DbMetricsView {
    kind: String,
    connections: Option<u32>,
    memory_mib: Option<u32>,
    extra: Option<String>,
    probe_ok: bool,
}

fn db_metrics_to_view(m: package_manager::DbMetrics) -> DbMetricsView {
    DbMetricsView {
        kind: m.kind,
        connections: m.connections,
        memory_mib: m.memory_mib,
        extra: m.extra,
        probe_ok: m.probe_ok,
    }
}

#[tauri::command]
async fn software_db_metrics(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    package_id: String,
    root_password: Option<String>,
) -> Result<DbMetricsView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path,
            saved_connection_index,
        )?;
        let metrics = match package_id.as_str() {
            "postgres" => package_manager::postgres_metrics_blocking(&session),
            "mariadb" => package_manager::mysql_metrics_blocking(
                &session,
                root_password.as_deref(),
            ),
            "redis" => package_manager::redis_metrics_blocking(&session),
            _ => return Err(format!("no metrics for package {package_id}")),
        }
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(db_metrics_to_view(metrics))
    })
    .await
    .map_err(|e| format!("software_db_metrics join: {e}"))?
}

// ── Cross-host clone (v2.12) ─────────────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ClonePlanEntry {
    /// Raw package name from the source host's manager.
    package: String,
    /// Descriptor id when we recognise the package, else `None`.
    /// The frontend hides un-resolvable rows by default since
    /// installing them on the target requires the same manager
    /// and might not exist there.
    descriptor_id: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ClonePlanView {
    /// Resolved manager on the source host (`apt` / `dnf` / …).
    package_manager: String,
    /// All explicitly-installed packages.
    entries: Vec<ClonePlanEntry>,
}

/// List the source host's explicitly-installed packages and
/// resolve each to a registry descriptor where possible. The
/// frontend then renders these as a checklist and feeds the
/// chosen subset into per-host install loops on the target side.
#[tauri::command]
async fn software_clone_plan(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<ClonePlanView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path,
            saved_connection_index,
        )?;
        let env = package_manager::probe_host_env_blocking(&session);
        let Some(manager) = env.package_manager else {
            return Ok::<_, String>(ClonePlanView {
                package_manager: String::new(),
                entries: Vec::new(),
            });
        };
        let names = package_manager::list_user_installed_blocking(&session)
            .map_err(|e| e.to_string())?;
        let entries = names
            .into_iter()
            .map(|p| {
                let descriptor_id =
                    package_manager::resolve_descriptor_for_package(&p, manager)
                        .map(|s| s.to_string());
                ClonePlanEntry {
                    package: p,
                    descriptor_id,
                }
            })
            .collect();
        Ok::<_, String>(ClonePlanView {
            package_manager: manager.as_str().to_string(),
            entries,
        })
    })
    .await
    .map_err(|e| format!("software_clone_plan join: {e}"))?
}

// ── Docker Compose templates (v2.11) ─────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ComposeTemplateView {
    id: String,
    display_name: String,
    description: String,
    yaml: String,
    published_ports: Vec<u16>,
}

#[tauri::command]
fn software_compose_templates() -> Vec<ComposeTemplateView> {
    package_manager::compose_templates()
        .iter()
        .map(|t| ComposeTemplateView {
            id: t.id.to_string(),
            display_name: t.display_name.to_string(),
            description: t.description.to_string(),
            yaml: t.yaml.to_string(),
            published_ports: t.published_ports.to_vec(),
        })
        .collect()
}

#[tauri::command]
async fn software_compose_apply(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    template_id: String,
) -> Result<PostgresActionReportView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path,
            saved_connection_index,
        )?;
        let report = package_manager::compose_apply_blocking(&session, &template_id)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(pg_report_to_view(report))
    })
    .await
    .map_err(|e| format!("software_compose_apply join: {e}"))?
}

#[tauri::command]
async fn software_compose_down(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    template_id: String,
) -> Result<PostgresActionReportView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path,
            saved_connection_index,
        )?;
        let report = package_manager::compose_down_blocking(&session, &template_id)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(pg_report_to_view(report))
    })
    .await
    .map_err(|e| format!("software_compose_down join: {e}"))?
}

// ── MySQL / Redis service-level orchestration (v2.9) ─────────────

#[tauri::command]
async fn mysql_create_user_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    db_username: String,
    db_password: String,
    db_name: String,
    root_password: Option<String>,
) -> Result<PostgresActionReportView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path,
            saved_connection_index,
        )?;
        let report = package_manager::mysql_create_user_blocking(
            &session,
            &db_username,
            &db_password,
            &db_name,
            root_password.as_deref(),
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(pg_report_to_view(report))
    })
    .await
    .map_err(|e| format!("mysql_create_user_remote join: {e}"))?
}

#[tauri::command]
async fn mysql_create_db_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    db_name: String,
    root_password: Option<String>,
) -> Result<PostgresActionReportView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path,
            saved_connection_index,
        )?;
        let report = package_manager::mysql_create_db_blocking(
            &session,
            &db_name,
            root_password.as_deref(),
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(pg_report_to_view(report))
    })
    .await
    .map_err(|e| format!("mysql_create_db_remote join: {e}"))?
}

#[tauri::command]
async fn mysql_open_remote_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<PostgresActionReportView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path,
            saved_connection_index,
        )?;
        let report = package_manager::mysql_open_remote_blocking(&session)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(pg_report_to_view(report))
    })
    .await
    .map_err(|e| format!("mysql_open_remote_remote join: {e}"))?
}

#[tauri::command]
async fn redis_set_password_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    redis_password: String,
) -> Result<PostgresActionReportView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path,
            saved_connection_index,
        )?;
        let report = package_manager::redis_set_password_blocking(&session, &redis_password)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(pg_report_to_view(report))
    })
    .await
    .map_err(|e| format!("redis_set_password_remote join: {e}"))?
}

#[tauri::command]
async fn redis_open_remote_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<PostgresActionReportView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path,
            saved_connection_index,
        )?;
        let report = package_manager::redis_open_remote_blocking(&session)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(pg_report_to_view(report))
    })
    .await
    .map_err(|e| format!("redis_open_remote_remote join: {e}"))?
}

// ── System search + ad-hoc install (v2.7) ──────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SearchHitView {
    name: String,
    summary: String,
}

/// Search the host's system package catalog (apt-cache search /
/// dnf search / …). Frontend calls this with the same string the
/// user types in the panel's search box once the registry has no
/// hits.
#[tauri::command]
async fn software_search_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<SearchHitView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let limit = limit.unwrap_or(20).min(100);
        let hits = package_manager::search_remote_blocking(&session, &query, limit)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(
            hits.into_iter()
                .map(|h| SearchHitView {
                    name: h.name,
                    summary: h.summary,
                })
                .collect(),
        )
    })
    .await
    .map_err(|e| format!("software_search_remote join: {e}"))?
}

/// Install a package not in the registry — used by the search
/// section's "Install" button. Streams output via the same
/// SOFTWARE_INSTALL_EVENT channel as the regular install path so
/// the activity log + cancel button reuse existing infrastructure.
#[tauri::command]
async fn software_install_arbitrary(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    package_name: String,
    install_id: String,
) -> Result<SoftwareInstallReportView, String> {
    let app_for_failure = app.clone();
    let install_id_for_failure = install_id.clone();
    let token = register_software_cancel(&app, &install_id);
    let token_for_task = token.clone();
    let join = tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let app_for_lines = app.clone();
        let install_id_for_lines = install_id.clone();
        let on_line = move |line: &str| {
            let _ = app_for_lines.emit(
                SOFTWARE_INSTALL_EVENT,
                SoftwareInstallEvent {
                    install_id: install_id_for_lines.clone(),
                    kind: "line".to_string(),
                    text: Some(line.to_string()),
                    report: None,
                    message: None,
                },
            );
        };
        let report = package_manager::install_arbitrary_blocking(
            &session,
            &package_name,
            on_line,
            Some(token_for_task.clone()),
        )
        .map_err(|e| e.to_string())?;
        let view = report_to_view(report);
        let kind = if view.status == "cancelled" { "cancelled" } else { "done" };
        let _ = app.emit(
            SOFTWARE_INSTALL_EVENT,
            SoftwareInstallEvent {
                install_id: install_id.clone(),
                kind: kind.to_string(),
                text: None,
                report: Some(view.clone()),
                message: None,
            },
        );
        Ok::<_, String>(view)
    })
    .await;
    unregister_software_cancel(&app_for_failure, &install_id_for_failure);
    match join {
        Ok(inner) => inner,
        Err(e) => {
            let msg = format!("software_install_arbitrary join: {e}");
            let _ = app_for_failure.emit(
                SOFTWARE_INSTALL_EVENT,
                SoftwareInstallEvent {
                    install_id: install_id_for_failure,
                    kind: "failed".to_string(),
                    text: None,
                    report: None,
                    message: Some(msg.clone()),
                },
            );
            Err(msg)
        }
    }
}

// ── Mirror switching (v2.3) ────────────────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MirrorChoiceView {
    id: String,
    label: String,
    apt_host: String,
    dnf_host: String,
    apk_host: Option<String>,
    pacman_url: Option<String>,
    zypper_host: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MirrorStateView {
    package_manager: String,
    current_id: Option<String>,
    current_host: Option<String>,
    has_backup: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MirrorActionReportView {
    /// `ok` / `sudo-requires-password` / `failed` / `unsupported-manager`.
    status: String,
    package_manager: String,
    command: String,
    exit_code: i32,
    output_tail: String,
    state_after: MirrorStateView,
}

fn mirror_action_status_kebab(status: package_mirror::MirrorActionStatus) -> &'static str {
    match status {
        package_mirror::MirrorActionStatus::Ok => "ok",
        package_mirror::MirrorActionStatus::SudoRequiresPassword => "sudo-requires-password",
        package_mirror::MirrorActionStatus::Failed => "failed",
        package_mirror::MirrorActionStatus::UnsupportedManager => "unsupported-manager",
    }
}

fn mirror_state_to_view(s: package_mirror::MirrorState) -> MirrorStateView {
    MirrorStateView {
        package_manager: s.package_manager,
        current_id: s.current_id,
        current_host: s.current_host,
        has_backup: s.has_backup,
    }
}

fn mirror_action_to_view(r: package_mirror::MirrorActionReport) -> MirrorActionReportView {
    MirrorActionReportView {
        status: mirror_action_status_kebab(r.status).to_string(),
        package_manager: r.package_manager,
        command: r.command,
        exit_code: r.exit_code,
        output_tail: r.output_tail,
        state_after: mirror_state_to_view(r.state_after),
    }
}

/// Filesystem path of the user-extras JSON file. The UI surfaces
/// this so the user knows where to drop custom entries; src-tauri
/// guarantees the parent dir exists at startup but doesn't create
/// the file itself (it's optional).
#[tauri::command]
fn software_user_extras_path() -> Option<String> {
    package_manager::user_extras_path().map(|p| p.display().to_string())
}

/// Read the user-extras JSON file into a string for the editor.
/// Returns an empty string when the file doesn't exist (so the
/// editor opens with a blank canvas instead of an error). Any
/// other read error surfaces verbatim.
#[tauri::command]
fn software_user_extras_read() -> Result<String, String> {
    let Some(path) = package_manager::user_extras_path() else {
        return Err("user_extras_path not initialised".to_string());
    };
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("read {}: {e}", path.display())),
    }
}

/// Validate-and-write the user-extras file. The frontend pre-
/// validates (it parses the textarea before letting the user save),
/// but we re-parse here so a hand-edited tab-key submission can't
/// land an invalid JSON blob on disk. Empty input clears the file.
///
/// **Caller MUST surface the "restart Pier-X to apply" notice** —
/// the registry's OnceLock memo means the running process keeps
/// the catalog it built at startup.
#[tauri::command]
fn software_user_extras_write(content: String) -> Result<(), String> {
    let Some(path) = package_manager::user_extras_path() else {
        return Err("user_extras_path not initialised".to_string());
    };
    let trimmed = content.trim();
    if trimmed.is_empty() {
        // Clearing the file is a valid action — drop it so the
        // next startup just uses the built-in catalog.
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("remove {}: {e}", path.display())),
        };
    }
    // Round-trip parse to surface JSON / schema errors before we
    // overwrite the user's previous file.
    let _: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| format!("invalid JSON: {e}"))?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, content).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

// ── Operation history (v2.8) ──────────────────────────────────────
//
// Append-only JSONL journal of significant software-panel actions
// (install, uninstall, mirror-set, mirror-restore, bundle-install).
// One file per Pier-X profile, lives next to software-prefs.json.
// Append-only so concurrent writes from multiple installs in flight
// never trample each other.

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwareHistoryEntry {
    /// Unix epoch seconds (UTC).
    ts: i64,
    /// `"install"` / `"uninstall"` / `"mirror-set"` / `"mirror-restore"` /
    /// `"bundle-install"` / `"install-arbitrary"`.
    action: String,
    /// User-readable target (`"nginx"` / `"aliyun"` / etc.).
    target: String,
    /// Resolved host the action ran against (`user@host:port`).
    host: String,
    /// `"ok"` / `"failed"` / `"cancelled"` / status string from the
    /// underlying report.
    outcome: String,
    /// Free-form note (e.g. localized error message). Empty for
    /// successful runs unless the caller adds one.
    note: String,
    /// Optional saved-connection index — populated by callers that
    /// have one in scope so the rollback flow can reach the host
    /// without re-prompting for credentials. `None` = the row's
    /// "undo" button stays disabled.
    #[serde(default)]
    saved_connection_index: Option<usize>,
}

fn software_history_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("software-history.jsonl"))
}

#[tauri::command]
fn software_history_log(
    app: tauri::AppHandle,
    action: String,
    target: String,
    host: String,
    outcome: String,
    note: String,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    use std::io::Write;
    let Some(path) = software_history_path(&app) else {
        return Err("history path unavailable".to_string());
    };
    let entry = SoftwareHistoryEntry {
        ts: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        action,
        target,
        host,
        outcome,
        note,
        saved_connection_index,
    };
    let line = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    writeln!(f, "{line}").map_err(|e| e.to_string())?;
    Ok(())
}

/// List the most-recent N history entries (newest first). Empty
/// when the file doesn't exist or is unreadable. `since_ts` filters
/// entries whose `ts` is older than that epoch second — pass 0 to
/// disable filtering.
#[tauri::command]
fn software_history_list(
    app: tauri::AppHandle,
    since_ts: Option<i64>,
    limit: Option<usize>,
) -> Vec<SoftwareHistoryEntry> {
    let Some(path) = software_history_path(&app) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let cutoff = since_ts.unwrap_or(0);
    let cap = limit.unwrap_or(200).min(2000);
    let mut all: Vec<SoftwareHistoryEntry> = text
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            serde_json::from_str::<SoftwareHistoryEntry>(trimmed).ok()
        })
        .filter(|e| e.ts >= cutoff)
        .collect();
    all.sort_by(|a, b| b.ts.cmp(&a.ts));
    all.truncate(cap);
    all
}

#[tauri::command]
fn software_history_clear(app: tauri::AppHandle) -> Result<(), String> {
    let Some(path) = software_history_path(&app) else {
        return Err("history path unavailable".to_string());
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("remove {}: {e}", path.display())),
    }
}

/// Software-panel preferences persisted in the app config dir.
/// One file, one key for now (`preferredMirrorId`); easy to grow.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwarePreferences {
    /// Last-picked mirror id. The MirrorDialog suggests this when
    /// the host has no detected mirror so users don't re-pick the
    /// same one every time they SSH to a new box.
    preferred_mirror_id: Option<String>,
}

fn software_preferences_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("software-prefs.json"))
}

fn load_software_preferences(app: &tauri::AppHandle) -> SoftwarePreferences {
    let Some(path) = software_preferences_path(app) else {
        return Default::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Default::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn save_software_preferences(app: &tauri::AppHandle, prefs: &SoftwarePreferences) {
    let Some(path) = software_preferences_path(app) else {
        return;
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(prefs) {
        let _ = std::fs::write(path, bytes);
    }
}

#[tauri::command]
fn software_preferences_get(app: tauri::AppHandle) -> SoftwarePreferences {
    load_software_preferences(&app)
}

#[tauri::command]
fn software_preferences_set_mirror(
    app: tauri::AppHandle,
    mirror_id: Option<String>,
) -> SoftwarePreferences {
    let mut prefs = load_software_preferences(&app);
    prefs.preferred_mirror_id = mirror_id;
    save_software_preferences(&app, &prefs);
    prefs
}

/// Static mirror catalog. Doesn't touch the host — same shape every
/// call, used by the frontend dialog to render the radio list.
#[tauri::command]
fn software_mirror_catalog() -> Vec<MirrorChoiceView> {
    package_mirror::supported_mirrors()
        .iter()
        .map(|m| MirrorChoiceView {
            id: m.id.as_str().to_string(),
            label: m.label.to_string(),
            apt_host: m.apt_host.to_string(),
            dnf_host: m.dnf_host.to_string(),
            apk_host: m.apk_host.map(str::to_string),
            pacman_url: m.pacman_url.map(str::to_string),
            zypper_host: m.zypper_host.map(str::to_string),
        })
        .collect()
}

/// Detect the current mirror state for the host's package manager.
#[tauri::command]
async fn software_mirror_get(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<MirrorStateView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let env = package_manager::probe_host_env_blocking(&session);
        let manager = match env.package_manager {
            Some(m) => m,
            None => {
                // No detected manager — return an empty state that
                // disables the dialog on the frontend.
                return Ok::<_, String>(MirrorStateView {
                    package_manager: String::new(),
                    current_id: None,
                    current_host: None,
                    has_backup: false,
                });
            }
        };
        let s = package_mirror::detect_mirror_blocking(&session, manager);
        Ok::<_, String>(mirror_state_to_view(s))
    })
    .await
    .map_err(|e| format!("software_mirror_get join: {e}"))?
}

/// Switch the host's apt / dnf sources to `mirror_id`. Backs up
/// the originals to `.pier-bak` on first invocation.
#[tauri::command]
async fn software_mirror_set(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    mirror_id: String,
) -> Result<MirrorActionReportView, String> {
    let app_for_prefs = app.clone();
    let mirror_id_for_prefs = mirror_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let env = package_manager::probe_host_env_blocking(&session);
        let manager = env
            .package_manager
            .ok_or_else(|| "host has no detected package manager".to_string())?;
        let id = package_mirror::MirrorId::from_str(&mirror_id)
            .ok_or_else(|| format!("unknown mirror id: {mirror_id}"))?;
        let report = package_mirror::set_mirror_blocking(&session, manager, id)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(mirror_action_to_view(report))
    })
    .await
    .map_err(|e| format!("software_mirror_set join: {e}"))?;
    // Remember the choice on success so the next host's dialog can
    // suggest the same mirror without the user re-picking.
    if let Ok(report) = &result {
        if report.status == "ok" {
            let mut prefs = load_software_preferences(&app_for_prefs);
            prefs.preferred_mirror_id = Some(mirror_id_for_prefs);
            save_software_preferences(&app_for_prefs, &prefs);
        }
    }
    result
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MirrorLatencyView {
    mirror_id: String,
    host: String,
    latency_ms: Option<u32>,
}

/// Client-side TCP probe of every mirror's hostname:443 — runs from
/// **this machine**, not over SSH. Useful when:
///  * the SSH host is offline / unreachable from the user's network
///    (so we can still suggest a mirror to pre-pick)
///  * the user wants to spot-check that the mirrors themselves are
///    healthy, independent of the remote box's network
///
/// Implementation: `std::net::TcpStream::connect_timeout` with a 4s
/// budget per host. Doesn't speak TLS — pure connect-time. Returns
/// the same view shape as the SSH-side benchmark so the dialog can
/// merge results.
#[tauri::command]
async fn software_mirror_benchmark_client() -> Result<Vec<MirrorLatencyView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        use std::time::{Duration, Instant};
        let mirrors = package_mirror::supported_mirrors();
        let mut threads = Vec::with_capacity(mirrors.len());
        for m in mirrors {
            // Probe each mirror's apt_host (the most universal one;
            // every mirror in our catalog declares this field as
            // non-Option). 443 = HTTPS — the same port apt/dnf
            // would speak to fetch metadata.
            let id = m.id.as_str().to_string();
            let host = m.apt_host.to_string();
            threads.push(std::thread::spawn(move || {
                let started = Instant::now();
                let addr_iter = match (host.as_str(), 443u16).to_socket_addrs() {
                    Ok(it) => it,
                    Err(_) => {
                        return MirrorLatencyView {
                            mirror_id: id,
                            host,
                            latency_ms: None,
                        };
                    }
                };
                // Try the first resolved address only — pier-x just
                // needs a reachability gauge, not a full ranking.
                let mut latency_ms: Option<u32> = None;
                for addr in addr_iter {
                    if let Ok(_stream) =
                        std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(4))
                    {
                        let elapsed = started.elapsed();
                        latency_ms = Some(elapsed.as_millis() as u32);
                        break;
                    }
                }
                MirrorLatencyView {
                    mirror_id: id,
                    host,
                    latency_ms,
                }
            }));
        }
        Ok::<_, String>(
            threads
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|_| MirrorLatencyView {
                    mirror_id: String::new(),
                    host: String::new(),
                    latency_ms: None,
                }))
                .collect(),
        )
    })
    .await
    .map_err(|e| format!("software_mirror_benchmark_client join: {e}"))?
}

/// Probe each mirror's reachability + latency over the SSH session.
/// Slow mirrors / unreachable mirrors come back with `latency_ms =
/// null`; the dialog sorts ascending and labels the fastest one as
/// "推荐".
#[tauri::command]
async fn software_mirror_benchmark(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<Vec<MirrorLatencyView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let env = package_manager::probe_host_env_blocking(&session);
        let manager = env
            .package_manager
            .ok_or_else(|| "host has no detected package manager".to_string())?;
        let results = package_mirror::benchmark_mirrors_blocking(&session, manager)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(
            results
                .into_iter()
                .map(|r| MirrorLatencyView {
                    mirror_id: r.mirror_id,
                    host: r.host,
                    latency_ms: r.latency_ms,
                })
                .collect(),
        )
    })
    .await
    .map_err(|e| format!("software_mirror_benchmark join: {e}"))?
}

/// Restore the original sources from `.pier-bak`. No-op when no
/// backup exists (the report still resolves with `ok`).
#[tauri::command]
async fn software_mirror_restore(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<MirrorActionReportView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let env = package_manager::probe_host_env_blocking(&session);
        let manager = env
            .package_manager
            .ok_or_else(|| "host has no detected package manager".to_string())?;
        let report = package_mirror::restore_mirror_blocking(&session, manager)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(mirror_action_to_view(report))
    })
    .await
    .map_err(|e| format!("software_mirror_restore join: {e}"))?
}

/// Run a streaming uninstall. Mirrors `software_install_or_update_inner`
/// in shape but emits on `SOFTWARE_UNINSTALL_EVENT` so the report
/// payload can carry uninstall-specific fields without a discriminant
/// union on the install channel.
#[tauri::command]
async fn software_uninstall_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    package_id: String,
    install_id: String,
    options: package_manager::UninstallOptions,
) -> Result<SoftwareUninstallReportView, String> {
    let app_for_failure = app.clone();
    let install_id_for_failure = install_id.clone();
    let token = register_software_cancel(&app, &install_id);
    let token_for_task = token.clone();
    let join = tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let app_for_lines = app.clone();
        let install_id_for_lines = install_id.clone();
        let on_line = move |line: &str| {
            let _ = app_for_lines.emit(
                SOFTWARE_UNINSTALL_EVENT,
                SoftwareUninstallEvent {
                    install_id: install_id_for_lines.clone(),
                    kind: "line".to_string(),
                    text: Some(line.to_string()),
                    report: None,
                    message: None,
                },
            );
        };
        let report = package_manager::uninstall_blocking(
            &session,
            &package_id,
            &options,
            on_line,
            Some(token_for_task.clone()),
        )
        .map_err(|e| e.to_string())?;
        let view = uninstall_report_to_view(report);
        let kind = if view.status == "cancelled" {
            "cancelled"
        } else {
            "done"
        };
        let _ = app.emit(
            SOFTWARE_UNINSTALL_EVENT,
            SoftwareUninstallEvent {
                install_id: install_id.clone(),
                kind: kind.to_string(),
                text: None,
                report: Some(view.clone()),
                message: None,
            },
        );
        Ok::<_, String>(view)
    })
    .await;
    unregister_software_cancel(&app_for_failure, &install_id_for_failure);
    match join {
        Ok(inner) => inner,
        Err(e) => {
            let msg = format!("software uninstall join: {e}");
            let _ = app_for_failure.emit(
                SOFTWARE_UNINSTALL_EVENT,
                SoftwareUninstallEvent {
                    install_id: install_id_for_failure,
                    kind: "failed".to_string(),
                    text: None,
                    report: None,
                    message: Some(msg.clone()),
                },
            );
            Err(msg)
        }
    }
}

/// View of `package_manager::ServiceActionReport`. Mirrors the install
/// report shape so the panel can reuse a single outcome card / toast
/// formatter, with extra `action` + `unit` + `serviceActiveAfter`
/// fields the UI needs for the per-row dot.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwareServiceActionReportView {
    package_id: String,
    /// `ok` / `sudo-requires-password` / `failed`.
    status: String,
    /// `start` / `stop` / `restart` / `reload`.
    action: String,
    unit: String,
    command: String,
    exit_code: i32,
    output_tail: String,
    service_active_after: bool,
}

/// Streaming event payload for the service-action channel. We keep
/// `kind: "line" | "done" | "failed"` shape compatible with the
/// install channel — the only difference is the `report` payload's
/// shape, which the frontend disambiguates by knowing which channel
/// it subscribed to.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SoftwareServiceActionEvent {
    install_id: String,
    kind: String,
    text: Option<String>,
    report: Option<SoftwareServiceActionReportView>,
    message: Option<String>,
}

const SOFTWARE_SERVICE_EVENT: &str = "software-service-action";

fn service_action_status_kebab(status: package_manager::ServiceActionStatus) -> &'static str {
    match status {
        package_manager::ServiceActionStatus::Ok => "ok",
        package_manager::ServiceActionStatus::SudoRequiresPassword => "sudo-requires-password",
        package_manager::ServiceActionStatus::Failed => "failed",
    }
}

fn service_action_report_to_view(
    report: package_manager::ServiceActionReport,
) -> SoftwareServiceActionReportView {
    SoftwareServiceActionReportView {
        package_id: report.package_id,
        status: service_action_status_kebab(report.status).to_string(),
        action: report.action,
        unit: report.unit,
        command: report.command,
        exit_code: report.exit_code,
        output_tail: report.output_tail,
        service_active_after: report.service_active_after,
    }
}

fn parse_service_action(s: &str) -> Result<package_manager::ServiceAction, String> {
    match s {
        "start" => Ok(package_manager::ServiceAction::Start),
        "stop" => Ok(package_manager::ServiceAction::Stop),
        "restart" => Ok(package_manager::ServiceAction::Restart),
        "reload" => Ok(package_manager::ServiceAction::Reload),
        other => Err(format!("unknown service action: {other}")),
    }
}

/// Drive a `systemctl <verb>` against one descriptor's service. Mirrors
/// the install command's lifecycle: emits `line` events for streaming
/// stdout/stderr, a final `done` event with the structured report, or
/// a `failed` event on join error. The frontend filters by
/// `installId` so concurrent rows on different hosts don't interleave.
#[tauri::command]
async fn software_service_action_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    package_id: String,
    install_id: String,
    action: String,
) -> Result<SoftwareServiceActionReportView, String> {
    let action = parse_service_action(&action)?;
    let app_for_failure = app.clone();
    let install_id_for_failure = install_id.clone();
    let join = tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let descriptor = package_manager::descriptor(&package_id)
            .ok_or_else(|| format!("unknown package id: {package_id}"))?;
        let app_for_lines = app.clone();
        let install_id_for_lines = install_id.clone();
        let on_line = move |line: &str| {
            let _ = app_for_lines.emit(
                SOFTWARE_SERVICE_EVENT,
                SoftwareServiceActionEvent {
                    install_id: install_id_for_lines.clone(),
                    kind: "line".to_string(),
                    text: Some(line.to_string()),
                    report: None,
                    message: None,
                },
            );
        };
        let report =
            package_manager::service_action_blocking(&session, descriptor, action, on_line)
                .map_err(|e| e.to_string())?;
        let view = service_action_report_to_view(report);
        let _ = app.emit(
            SOFTWARE_SERVICE_EVENT,
            SoftwareServiceActionEvent {
                install_id: install_id.clone(),
                kind: "done".to_string(),
                text: None,
                report: Some(view.clone()),
                message: None,
            },
        );
        Ok::<_, String>(view)
    })
    .await;
    match join {
        Ok(inner) => inner,
        Err(e) => {
            let msg = format!("software service action join: {e}");
            let _ = app_for_failure.emit(
                SOFTWARE_SERVICE_EVENT,
                SoftwareServiceActionEvent {
                    install_id: install_id_for_failure,
                    kind: "failed".to_string(),
                    text: None,
                    report: None,
                    message: Some(msg.clone()),
                },
            );
            Err(msg)
        }
    }
}

/// One-shot fetch of the most recent `lines` rows of `journalctl -u
/// <unit>` output. Returns `Ok(vec![])` when the descriptor doesn't
/// have a service unit on this host's distro family — the panel
/// gates the menu on `has_service` so this path is defensive.
#[tauri::command]
async fn software_service_logs_remote(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    package_id: String,
    lines: usize,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let descriptor = package_manager::descriptor(&package_id)
            .ok_or_else(|| format!("unknown package id: {package_id}"))?;
        package_manager::journalctl_tail_blocking(&session, descriptor, lines)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("software_service_logs_remote join: {e}"))?
}

/// Trigger the registered cancellation token for `install_id`. Resolves
/// to `Ok(())` even when no task is running for that id — the user may
/// have clicked Cancel right after the install completed; surfacing
/// "no such task" as an error would be a UX regression. We do still
/// emit a `cancelled` event in that case to give the frontend a single
/// terminal signal it can hang state-reset off.
///
/// We can't tell from the install_id alone whether the task is an
/// install or an uninstall, so the cancel event is fanned out on both
/// channels — the frontend's per-id subscription filters it back down.
#[tauri::command]
async fn software_install_cancel(
    app: tauri::AppHandle,
    install_id: String,
) -> Result<(), String> {
    let token = {
        let state: tauri::State<'_, AppState> = app.state();
        let map = state
            .software_cancel
            .lock()
            .map_err(|e| format!("software_cancel lock: {e}"))?;
        map.get(&install_id).cloned()
    };
    if let Some(tok) = token {
        tok.cancel();
    }
    // Fan the cancelled signal out on both channels — the install
    // channel covers install + update activity, the uninstall channel
    // covers uninstall. The frontend filters by install_id, so
    // whichever subscription matches will pick it up.
    let _ = app.emit(
        SOFTWARE_INSTALL_EVENT,
        SoftwareInstallEvent {
            install_id: install_id.clone(),
            kind: "cancelled".to_string(),
            text: None,
            report: None,
            message: None,
        },
    );
    let _ = app.emit(
        SOFTWARE_UNINSTALL_EVENT,
        SoftwareUninstallEvent {
            install_id: install_id.clone(),
            kind: "cancelled".to_string(),
            text: None,
            report: None,
            message: None,
        },
    );
    // Drop the entry now so a second cancel for the same id is a no-op
    // even if the task hasn't unwound yet.
    unregister_software_cancel(&app, &install_id);
    Ok(())
}

// ── Nginx panel ────────────────────────────────────────────────

#[tauri::command]
async fn nginx_layout(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<nginx::NginxLayout, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        nginx::list_layout_blocking(&session).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("nginx_layout join: {e}"))?
}

#[tauri::command]
async fn nginx_read_file(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    path: String,
) -> Result<NginxReadFileView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        let content = nginx::read_file_blocking(&session, &path)
            .map_err(|e| e.to_string())?;
        let parse = nginx::parse(&content);
        Ok::<_, String>(NginxReadFileView {
            path,
            content,
            parse,
        })
    })
    .await
    .map_err(|e| format!("nginx_read_file join: {e}"))?
}

#[tauri::command]
async fn nginx_save_file(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    path: String,
    content: String,
) -> Result<nginx::NginxSaveResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        nginx::save_file_validate_reload_blocking(&session, &path, &content)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("nginx_save_file join: {e}"))?
}

#[tauri::command]
async fn nginx_validate(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<nginx::NginxValidateResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        nginx::validate_blocking(&session).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("nginx_validate join: {e}"))?
}

#[tauri::command]
async fn nginx_reload(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<nginx::NginxValidateResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        nginx::reload_blocking(&session).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("nginx_reload join: {e}"))?
}

#[tauri::command]
async fn nginx_create_file(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    path: String,
    content: String,
) -> Result<nginx::NginxValidateResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        nginx::create_file_blocking(&session, &path, &content)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("nginx_create_file join: {e}"))?
}

#[tauri::command]
async fn nginx_toggle_site(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    site_name: String,
    enable: bool,
) -> Result<nginx::NginxValidateResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<'_, AppState> = app.state();
        let session = get_or_open_ssh_session(
            &state,
            &host,
            port,
            &user,
            &auth_mode,
            &password,
            &key_path,
            saved_connection_index,
        )?;
        nginx::toggle_site_blocking(&session, &site_name, enable)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("nginx_toggle_site join: {e}"))?
}

/// Bundles the read content with its parsed AST so the frontend gets
/// both in one round-trip. Surfacing the parse from the backend (as
/// opposed to re-parsing in TS) keeps the AST shape canonical: any
/// future renderer change lives in one place.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct NginxReadFileView {
    path: String,
    content: String,
    parse: nginx::NginxParseResult,
}

#[tauri::command]
fn sqlite_browse_remote(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    db_path: String,
    table: Option<String>,
) -> Result<RemoteSqliteBrowserState, String> {
    let trimmed = db_path.trim();
    if trimmed.is_empty() {
        return Err(String::from("remote SQLite path must not be empty"));
    }
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let tables =
        sqlite_remote::list_tables_blocking(&session, trimmed).map_err(|e| e.to_string())?;
    let table_name = choose_active_item(table, &tables);
    let columns = if table_name.is_empty() {
        Vec::new()
    } else {
        sqlite_remote::table_columns_blocking(&session, trimmed, &table_name)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|c| SqliteColumnView {
                name: c.name,
                col_type: c.col_type,
                not_null: c.not_null,
                primary_key: c.primary_key,
            })
            .collect()
    };
    let preview = if table_name.is_empty() {
        None
    } else {
        let result = sqlite_remote::preview_table_blocking(&session, trimmed, &table_name, 24)
            .map_err(|e| e.to_string())?;
        Some(DataPreview {
            columns: result.columns,
            rows: result.rows,
            truncated: result.truncated,
        })
    };

    // Best-effort remote stat — `ls -l` over SSH is one trip per
    // open. If it fails (no `ls`, exotic shell, etc.) the panel
    // shows the file size as "unknown" rather than blocking.
    let file_size = sqlite_remote::stat_size_blocking(&session, trimmed).unwrap_or(0);

    Ok(RemoteSqliteBrowserState {
        path: trimmed.to_string(),
        table_name,
        tables,
        columns,
        preview,
        indexes: Vec::new(),
        triggers: Vec::new(),
        file_size,
    })
}

#[tauri::command]
fn sqlite_execute_remote(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    db_path: String,
    sql: String,
) -> Result<QueryExecutionResult, String> {
    let trimmed_path = db_path.trim();
    let trimmed_sql = sql.trim();
    if trimmed_path.is_empty() {
        return Err(String::from("remote SQLite path must not be empty"));
    }
    if trimmed_sql.is_empty() {
        return Err(String::from("SQL must not be empty"));
    }
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let result = sqlite_remote::execute_blocking(&session, trimmed_path, trimmed_sql)
        .map_err(|e| e.to_string())?;
    // Mirror the local sqlite_execute shape: a syntax error
    // inside the CLI becomes an Err rather than a result with
    // .error. That way the panel's `queryError` path fires.
    if let Some(err) = result.error {
        return Err(err);
    }
    // `RemoteQueryResult.affected_rows / last_insert_id` are
    // i64 but the view is u64 — cast with saturation.
    Ok(QueryExecutionResult {
        columns: result.columns,
        rows: result.rows,
        truncated: result.truncated,
        affected_rows: result.affected_rows.max(0) as u64,
        last_insert_id: result.last_insert_id.and_then(|v| u64::try_from(v).ok()),
        elapsed_ms: result.elapsed_ms,
    })
}

#[tauri::command]
fn sqlite_find_in_dir(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
    directory: String,
    max_depth: Option<u32>,
) -> Result<Vec<RemoteSqliteCandidate>, String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let dir = directory.trim();
    if dir.is_empty() {
        return Err(String::from("directory must not be empty"));
    }
    let depth = max_depth.unwrap_or(2).min(4);
    let escaped_dir = shell_quote_dir(dir);
    // GNU `find -printf` is a non-POSIX extension that BSD / busybox
    // `find` (macOS, Alpine, routers, ...) do not support. To stay
    // portable we list paths with a plain `find` and then shell out
    // to `wc -c` / `stat -c|-f` per file — 20-ish sqlite files max,
    // so the extra process spawns cost <100 ms total.
    let cmd = format!(
        "find {escaped_dir} -maxdepth {depth} -type f \\( -name '*.db' -o -name '*.sqlite' -o -name '*.sqlite3' \\) 2>/dev/null | head -n 50 | while IFS= read -r p; do \
sz=$(wc -c < \"$p\" 2>/dev/null | tr -d ' '); \
m=$(stat -c '%Y' \"$p\" 2>/dev/null || stat -f '%m' \"$p\" 2>/dev/null); \
printf '%s\\t%s\\t%s\\n' \"$p\" \"${{sz:-0}}\" \"${{m:-0}}\"; \
done"
    );
    let rt = pier_core::ssh::runtime::shared();
    let (exit, stdout) = rt
        .block_on(session.exec_command(&cmd))
        .map_err(|e| e.to_string())?;
    if exit != 0 && stdout.trim().is_empty() {
        // non-zero with no output usually means `find` hit a
        // permission issue; return empty list rather than error.
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for line in stdout.lines() {
        let mut parts = line.splitn(3, '\t');
        let Some(path) = parts.next() else { continue };
        if path.is_empty() {
            continue;
        }
        let size_bytes = parts
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let modified = parts
            .next()
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|&v| v > 0);
        out.push(RemoteSqliteCandidate {
            path: path.to_string(),
            size_bytes,
            modified,
        });
    }
    Ok(out)
}

/// Quote a directory argument for a POSIX shell while preserving
/// leading-tilde semantics (`~`, `~/foo`, `~user`). Tilde
/// expansion is a shell feature that only triggers for unquoted
/// leading `~`, so we leave that segment bare and single-quote
/// the remainder.
fn shell_quote_dir(dir: &str) -> String {
    if dir.starts_with('~') {
        return match dir.split_once('/') {
            // `~/foo bar` → `~/'foo bar'` (tilde segment unquoted,
            // rest single-quoted — shell concatenates both into
            // one word before tilde-expansion).
            Some((head, rest)) => format!("{}/{}", head, shell_single_quote(rest)),
            // `~` alone or `~user` with no trailing path.
            None => dir.to_string(),
        };
    }
    shell_single_quote(dir)
}

/// POSIX shell single-quote escape.
fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

// ── Docker Extended ─────────────────────────────────────────────

#[tauri::command]
fn docker_inspect(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    container_id: String,
    saved_connection_index: Option<usize>,
) -> Result<String, String> {
    run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| {
            docker::inspect_container_blocking(session, &container_id).map_err(|e| e.to_string())
        },
    )
}

#[tauri::command]
fn docker_remove_image(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    image_id: String,
    force: bool,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| {
            docker::remove_image_blocking(session, &image_id, force).map_err(|e| e.to_string())
        },
    )
}

#[tauri::command]
fn docker_remove_volume(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    volume_name: String,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| docker::remove_volume_blocking(session, &volume_name).map_err(|e| e.to_string()),
    )
}

#[tauri::command]
fn docker_remove_network(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    network_name: String,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| {
            docker::remove_network_blocking(session, &network_name).map_err(|e| e.to_string())
        },
    )
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DockerRunOptionsView {
    image: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    ports: Vec<(String, String)>,
    #[serde(default)]
    env: Vec<(String, String)>,
    #[serde(default)]
    volumes: Vec<(String, String)>,
    #[serde(default)]
    restart: String,
    #[serde(default)]
    command: String,
}

impl From<DockerRunOptionsView> for docker::RunContainerOptions {
    fn from(v: DockerRunOptionsView) -> Self {
        docker::RunContainerOptions {
            image: v.image,
            name: v.name,
            ports: v.ports,
            env: v.env,
            volumes: v.volumes,
            restart: v.restart,
            command: v.command,
        }
    }
}

#[tauri::command]
fn docker_run_container(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    options: DockerRunOptionsView,
    saved_connection_index: Option<usize>,
) -> Result<String, String> {
    let opts: docker::RunContainerOptions = options.into();
    run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| docker::run_container_blocking(session, &opts).map_err(|e| e.to_string()),
    )
}

#[tauri::command]
fn docker_prune_volumes(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<String, String> {
    run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| docker::prune_volumes_blocking(session).map_err(|e| e.to_string()),
    )
}

#[tauri::command]
fn docker_prune_images(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<String, String> {
    run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| docker::prune_images_blocking(session).map_err(|e| e.to_string()),
    )
}

#[tauri::command]
fn docker_pull_image(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    image_ref: String,
    // `env_prefix`: optional env overrides (e.g. HTTPS_PROXY) applied only
    // to this `docker pull`; does not modify the remote daemon config.
    env_prefix: Option<Vec<(String, String)>>,
    saved_connection_index: Option<usize>,
) -> Result<String, String> {
    let env = env_prefix.unwrap_or_default();
    let env_refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| {
            docker::pull_image_blocking(session, &image_ref, &env_refs).map_err(|e| e.to_string())
        },
    )
}

#[tauri::command]
async fn local_docker_pull_image(
    image_ref: String,
    env_prefix: Option<Vec<(String, String)>>,
) -> Result<String, String> {
    if image_ref.trim().is_empty() {
        return Err("docker pull: image reference is required".into());
    }
    let mut cmd = std::process::Command::new("docker");
    for (k, v) in env_prefix.unwrap_or_default() {
        cmd.env(k, v);
    }
    let output = cmd
        .args(["pull", image_ref.trim()])
        .output()
        .map_err(|e| format!("docker pull failed: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
fn docker_volume_files(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    mountpoint: String,
    saved_connection_index: Option<usize>,
) -> Result<String, String> {
    run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| {
            docker::list_volume_files_blocking(session, &mountpoint).map_err(|e| e.to_string())
        },
    )
}

#[tauri::command]
async fn local_docker_run_container(options: DockerRunOptionsView) -> Result<String, String> {
    let opts: docker::RunContainerOptions = options.into();
    if opts.image.trim().is_empty() {
        return Err("docker run: image is required".into());
    }
    let mut args: Vec<String> = vec!["run".into(), "-d".into()];
    if !opts.name.trim().is_empty() {
        args.push("--name".into());
        args.push(opts.name.trim().into());
    }
    if !opts.restart.trim().is_empty() {
        args.push("--restart".into());
        args.push(opts.restart.trim().into());
    }
    for (h, g) in &opts.ports {
        let h = h.trim();
        let g = g.trim();
        if g.is_empty() {
            continue;
        }
        args.push("-p".into());
        args.push(if h.is_empty() {
            g.into()
        } else {
            format!("{h}:{g}")
        });
    }
    for (k, v) in &opts.env {
        if k.trim().is_empty() {
            continue;
        }
        args.push("-e".into());
        args.push(format!("{}={}", k.trim(), v));
    }
    for (h, g) in &opts.volumes {
        let h = h.trim();
        let g = g.trim();
        if h.is_empty() || g.is_empty() {
            continue;
        }
        args.push("-v".into());
        args.push(format!("{h}:{g}"));
    }
    args.push(opts.image.trim().into());
    if !opts.command.trim().is_empty() {
        // Local std::process::Command does not go through a shell, so we
        // split on whitespace; users wanting shell features can use SSH.
        for tok in opts.command.split_whitespace() {
            args.push(tok.into());
        }
    }
    let output = std::process::Command::new("docker")
        .args(&args)
        .output()
        .map_err(|e| format!("docker run failed: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[tauri::command]
async fn local_docker_prune_volumes() -> Result<String, String> {
    let output = std::process::Command::new("docker")
        .args(["volume", "prune", "-f"])
        .output()
        .map_err(|e| format!("docker volume prune failed: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
async fn local_docker_prune_images() -> Result<String, String> {
    let output = std::process::Command::new("docker")
        .args(["image", "prune", "-a", "-f"])
        .output()
        .map_err(|e| format!("docker image prune failed: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
async fn local_docker_volume_files(mountpoint: String) -> Result<String, String> {
    let output = std::process::Command::new("ls")
        .args(["-la", "--color=never", &mountpoint])
        .output()
        .map_err(|e| format!("ls failed: {e}"))?;
    // `ls` prints to stderr on permission errors; bundle both so the user
    // sees why a listing is empty.
    let mut out = String::from_utf8_lossy(&output.stdout).to_string();
    let err = String::from_utf8_lossy(&output.stderr);
    if !err.trim().is_empty() {
        out.push_str(&err);
    }
    Ok(out.lines().take(200).collect::<Vec<_>>().join("\n"))
}

// ── SFTP Extended ───────────────────────────────────────────────

#[tauri::command]
fn sftp_mkdir(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    path: String,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let sftp = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode)?;
    sftp.create_dir_blocking(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn sftp_remove(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    path: String,
    is_dir: bool,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let sftp = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode)?;
    if is_dir {
        sftp.remove_dir_blocking(&path).map_err(|e| e.to_string())
    } else {
        sftp.remove_file_blocking(&path).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn sftp_rename(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    from: String,
    to: String,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let sftp = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode)?;
    sftp.rename_blocking(&from, &to).map_err(|e| e.to_string())
}

/// Change POSIX permissions on a remote file or directory.
#[tauri::command]
fn sftp_chmod(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    path: String,
    mode: u32,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let sftp = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode)?;
    sftp.set_permissions_blocking(&path, mode)
        .map_err(|e| e.to_string())
}

/// Create an empty remote file (touch semantic — truncates if exists).
#[tauri::command]
fn sftp_create_file(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    path: String,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let sftp = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode)?;
    sftp.create_file_blocking(&path).map_err(|e| e.to_string())
}

/// Metadata + UTF-8 content returned by [`sftp_read_text`]. The
/// frontend editor dialog uses every field: `permissions` seeds the
/// chmod dialog, `size` + `modified` show in the status bar, and
/// `lossy` drives the "non-UTF-8 content" warning banner.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SftpTextFile {
    path: String,
    content: String,
    size: u64,
    permissions: Option<u32>,
    modified: Option<u64>,
    /// True when raw bytes contained invalid UTF-8 sequences that we
    /// had to replace with U+FFFD. Saving will persist the replaced
    /// bytes, so the UI warns the user before letting them overwrite.
    lossy: bool,
    /// Owner display string (named user, falling back to uid).
    /// Empty when the server omitted owner info.
    owner: String,
    /// Group display string (named group, falling back to gid).
    group: String,
    /// Detected line ending convention. One of:
    /// * `"lf"` — Unix-style `\n`
    /// * `"crlf"` — Windows-style `\r\n`
    /// * `"cr"` — classic-Mac `\r` only
    /// * `"mixed"` — multiple kinds present
    /// * `"none"` — no line endings (single-line or empty file)
    eol: String,
    /// Detected encoding label. Currently one of `"utf-8"`,
    /// `"utf-8-bom"`, `"utf-16-le"`, `"utf-16-be"`, or
    /// `"binary"` when the file appears to be non-text. The
    /// content field is always UTF-8 — this is purely a footer
    /// readout for the editor dialog.
    encoding: String,
}

/// Hard ceiling for `sftp_read_text`. Keeping the editor confined to
/// config-sized files avoids loading a multi-GB log into memory when
/// the user mis-clicks — large files should go through download.
const SFTP_TEXT_READ_MAX: u64 = 5 * 1024 * 1024;

/// Read a remote file as UTF-8 text for the editor dialog. Rejects
/// anything larger than `max_bytes` (capped by [`SFTP_TEXT_READ_MAX`])
/// before pulling bytes across the wire.
#[tauri::command]
fn sftp_read_text(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    path: String,
    max_bytes: Option<u64>,
    saved_connection_index: Option<usize>,
) -> Result<SftpTextFile, String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let sftp = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode)?;
    let meta = sftp.stat_blocking(&path).map_err(|e| e.to_string())?;
    let limit = max_bytes
        .unwrap_or(SFTP_TEXT_READ_MAX)
        .min(SFTP_TEXT_READ_MAX);
    if meta.size > limit {
        return Err(format!(
            "File is {} bytes; editor limit is {} bytes",
            meta.size, limit
        ));
    }
    let bytes = sftp.read_file_blocking(&path).map_err(|e| e.to_string())?;
    let encoding = detect_text_encoding(&bytes);
    // Strip the BOM before lossy-decoding so it doesn't show up
    // as a U+FEFF sentinel in the editor. The `encoding` label
    // stays "utf-8-bom" so the footer can preserve it on save.
    let decode_slice: &[u8] = if encoding == "utf-8-bom" && bytes.len() >= 3 {
        &bytes[3..]
    } else {
        &bytes
    };
    let raw_len = decode_slice.len();
    let text = String::from_utf8_lossy(decode_slice).into_owned();
    let lossy = text.as_bytes().len() != raw_len || text.contains('\u{FFFD}');
    let eol = detect_eol(&text);
    Ok(SftpTextFile {
        path,
        content: text,
        size: meta.size,
        permissions: meta.permissions,
        modified: meta.modified,
        lossy,
        owner: meta.owner.clone().unwrap_or_default(),
        group: meta.group.clone().unwrap_or_default(),
        eol,
        encoding,
    })
}

/// Best-effort encoding sniffer for SFTP-backed text files.
/// We only need to distinguish a small handful of cases for the
/// editor footer:
/// * UTF-8 with BOM (`EF BB BF`) — preserved on save.
/// * UTF-16 LE / BE (`FF FE` / `FE FF`) — read for display, but
///   we don't yet round-trip them on save (the user is warned by
///   the existing `lossy` flag).
/// * Plain UTF-8 — the common case.
/// * Binary — anything with NUL bytes in the first 1 KiB.
///
/// This is not a full chardet — it's a pragmatic three-byte BOM
/// check plus a NUL scan. Files without a BOM that are actually
/// in a legacy single-byte encoding (Latin-1, Shift-JIS, ...)
/// fall through as "utf-8" and the `lossy` flag flips on if the
/// bytes don't decode.
fn detect_text_encoding(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from("utf-8-bom");
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return String::from("utf-16-le");
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return String::from("utf-16-be");
    }
    let scan_len = bytes.len().min(1024);
    if bytes[..scan_len].contains(&0u8) {
        return String::from("binary");
    }
    String::from("utf-8")
}

/// Classify a string's line endings. Walks once and counts
/// `\r\n`, lone `\n`, and lone `\r`. The dominant kind wins;
/// ties produce `mixed`.
fn detect_eol(text: &str) -> String {
    let mut crlf = 0usize;
    let mut lf = 0usize;
    let mut cr = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                if bytes.get(i + 1) == Some(&b'\n') {
                    crlf += 1;
                    i += 2;
                    continue;
                }
                cr += 1;
            }
            b'\n' => {
                lf += 1;
            }
            _ => {}
        }
        i += 1;
    }
    let total = crlf + lf + cr;
    if total == 0 {
        return String::from("none");
    }
    let max = crlf.max(lf).max(cr);
    let kinds_at_max = [crlf, lf, cr].iter().filter(|&&n| n == max).count();
    if kinds_at_max > 1 || (max < total) {
        return String::from("mixed");
    }
    if max == crlf {
        String::from("crlf")
    } else if max == lf {
        String::from("lf")
    } else {
        String::from("cr")
    }
}

/// Write UTF-8 text back to a remote file, overwriting. The editor
/// dialog calls this when the user saves.
#[tauri::command]
fn sftp_write_text(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    path: String,
    content: String,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let sftp = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode)?;
    sftp.write_file_blocking(&path, content.as_bytes())
        .map_err(|e| e.to_string())
}

/// Progress update emitted to the frontend for in-flight transfers.
/// Throttled to one event per ~64 KiB chunk by the chunked
/// upload/download loops — the frontend's React batching handles the
/// rest so the transfer queue re-renders at a comfortable rate.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SftpProgressEvent {
    /// Frontend-assigned transfer id so listeners can match events
    /// to the queue entry they created when calling the command.
    id: String,
    bytes: u64,
    total: u64,
    /// True on the final emit (either after the last chunk finishes
    /// or after a failure). Lets the UI stop animating.
    done: bool,
    /// Populated with the error message when the transfer failed.
    error: Option<String>,
}

/// Event name the frontend subscribes to. Kept as a constant so the
/// TypeScript side can import the same string without guessing.
const SFTP_PROGRESS_EVENT: &str = "sftp:progress";

/// Emit a progress event — best-effort. If the frontend window is
/// gone, `emit` errors; we swallow because a transfer shouldn't fail
/// because the panel unmounted.
fn emit_sftp_progress(app: &tauri::AppHandle, evt: SftpProgressEvent) {
    use tauri::Emitter;
    let _ = app.emit(SFTP_PROGRESS_EVENT, evt);
}

#[tauri::command]
fn sftp_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    remote_path: String,
    local_path: String,
    saved_connection_index: Option<usize>,
    transfer_id: Option<String>,
) -> Result<(), String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let sftp = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode)?;
    let resolved_local = expand_local_path(&local_path);
    let id = transfer_id.clone().unwrap_or_default();

    // Fast path: no transfer id means the caller didn't subscribe to
    // progress, so skip the extra metadata/chunk dance and use the
    // whole-file download. Same behaviour as before the progress
    // plumbing landed.
    if transfer_id.is_none() {
        return sftp
            .download_to_blocking(&remote_path, &resolved_local)
            .map_err(|e| e.to_string());
    }

    let app_for_cb = app.clone();
    let id_for_cb = id.clone();
    let result = sftp.download_to_with_progress_blocking(
        &remote_path,
        &resolved_local,
        move |bytes, total| {
            emit_sftp_progress(
                &app_for_cb,
                SftpProgressEvent {
                    id: id_for_cb.clone(),
                    bytes,
                    total,
                    done: false,
                    error: None,
                },
            );
        },
    );

    match result {
        Ok(bytes) => {
            emit_sftp_progress(
                &app,
                SftpProgressEvent {
                    id,
                    bytes,
                    total: bytes,
                    done: true,
                    error: None,
                },
            );
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            emit_sftp_progress(
                &app,
                SftpProgressEvent {
                    id,
                    bytes: 0,
                    total: 0,
                    done: true,
                    error: Some(msg.clone()),
                },
            );
            Err(msg)
        }
    }
}

#[tauri::command]
fn sftp_upload(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    local_path: String,
    remote_path: String,
    saved_connection_index: Option<usize>,
    transfer_id: Option<String>,
) -> Result<(), String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let sftp = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode)?;
    let resolved_local = expand_local_path(&local_path);
    let id = transfer_id.clone().unwrap_or_default();

    if transfer_id.is_none() {
        return sftp
            .upload_from_blocking(&resolved_local, &remote_path)
            .map_err(|e| e.to_string());
    }

    let app_for_cb = app.clone();
    let id_for_cb = id.clone();
    let result = sftp.upload_from_with_progress_blocking(
        &resolved_local,
        &remote_path,
        move |bytes, total| {
            emit_sftp_progress(
                &app_for_cb,
                SftpProgressEvent {
                    id: id_for_cb.clone(),
                    bytes,
                    total,
                    done: false,
                    error: None,
                },
            );
        },
    );

    match result {
        Ok(bytes) => {
            emit_sftp_progress(
                &app,
                SftpProgressEvent {
                    id,
                    bytes,
                    total: bytes,
                    done: true,
                    error: None,
                },
            );
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            emit_sftp_progress(
                &app,
                SftpProgressEvent {
                    id,
                    bytes: 0,
                    total: 0,
                    done: true,
                    error: Some(msg.clone()),
                },
            );
            Err(msg)
        }
    }
}

/// Upload a local directory recursively into `remote_path`. Emits
/// aggregate progress via `sftp:progress` (bytes summed across the
/// whole tree). See [`sftp_upload`] for the event schema — the shape
/// is identical.
#[tauri::command]
fn sftp_upload_tree(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    local_path: String,
    remote_path: String,
    saved_connection_index: Option<usize>,
    transfer_id: Option<String>,
) -> Result<(), String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let sftp = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode)?;
    let resolved_local = expand_local_path(&local_path);
    let id = transfer_id.clone().unwrap_or_default();

    let app_for_cb = app.clone();
    let id_for_cb = id.clone();
    let should_emit = !transfer_id.as_deref().unwrap_or("").is_empty();
    let result = sftp.upload_tree_blocking(&resolved_local, &remote_path, move |bytes, total| {
        if should_emit {
            emit_sftp_progress(
                &app_for_cb,
                SftpProgressEvent {
                    id: id_for_cb.clone(),
                    bytes,
                    total,
                    done: false,
                    error: None,
                },
            );
        }
    });

    match result {
        Ok(bytes) => {
            if should_emit {
                emit_sftp_progress(
                    &app,
                    SftpProgressEvent {
                        id,
                        bytes,
                        total: bytes,
                        done: true,
                        error: None,
                    },
                );
            }
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            if should_emit {
                emit_sftp_progress(
                    &app,
                    SftpProgressEvent {
                        id,
                        bytes: 0,
                        total: 0,
                        done: true,
                        error: Some(msg.clone()),
                    },
                );
            }
            Err(msg)
        }
    }
}

/// Download a remote directory recursively to `local_path`. Mirror
/// image of [`sftp_upload_tree`].
#[tauri::command]
fn sftp_download_tree(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    remote_path: String,
    local_path: String,
    saved_connection_index: Option<usize>,
    transfer_id: Option<String>,
) -> Result<(), String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let sftp = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode)?;
    let resolved_local = expand_local_path(&local_path);
    let id = transfer_id.clone().unwrap_or_default();

    let app_for_cb = app.clone();
    let id_for_cb = id.clone();
    let should_emit = !transfer_id.as_deref().unwrap_or("").is_empty();
    let result = sftp.download_tree_blocking(&remote_path, &resolved_local, move |bytes, total| {
        if should_emit {
            emit_sftp_progress(
                &app_for_cb,
                SftpProgressEvent {
                    id: id_for_cb.clone(),
                    bytes,
                    total,
                    done: false,
                    error: None,
                },
            );
        }
    });

    match result {
        Ok(bytes) => {
            if should_emit {
                emit_sftp_progress(
                    &app,
                    SftpProgressEvent {
                        id,
                        bytes,
                        total: bytes,
                        done: true,
                        error: None,
                    },
                );
            }
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            if should_emit {
                emit_sftp_progress(
                    &app,
                    SftpProgressEvent {
                        id,
                        bytes: 0,
                        total: 0,
                        done: true,
                        error: Some(msg.clone()),
                    },
                );
            }
            Err(msg)
        }
    }
}

// ── SFTP external editor ────────────────────────────────────────

/// Hard ceiling on what we'll mirror through the external-editor
/// flow. We still fetch the bytes once and then poll the local
/// copy, so picking a multi-GB log here would burn temp disk for
/// no real win — desktop editors choke long before that anyway.
const SFTP_EXTERNAL_EDIT_MAX: u64 = 256 * 1024 * 1024;

/// Polling cadence for the watcher thread. 250ms is fine-grained
/// enough that `Save` in any editor reflects within a heartbeat,
/// but coarse enough that an idle watcher costs nothing measurable.
const EXTERNAL_EDIT_POLL: Duration = Duration::from_millis(250);

/// Wait this long after a detected mtime/size change before pushing
/// bytes back. Editors that write the file in multiple syscalls
/// (truncate → write → fsync) would otherwise trigger one upload
/// per intermediate state; debouncing collapses those into one.
const EXTERNAL_EDIT_DEBOUNCE: Duration = Duration::from_millis(600);

/// Event name the frontend subscribes to for upload status updates
/// from active external-editor watchers.
const SFTP_EXTERNAL_EDIT_EVENT: &str = "sftp:external-edit";

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SftpExternalEditEvent {
    /// Watcher id assigned by [`sftp_open_external`]. Frontend
    /// dialogs filter events by their own id.
    watcher_id: String,
    /// One of: `"uploading"`, `"uploaded"`, `"error"`, `"stopped"`.
    /// Lifecycle order is `uploading` → `uploaded` (or `error`)
    /// per detected change, then `stopped` once on shutdown.
    kind: String,
    /// Bytes written on a successful upload.
    bytes: Option<u64>,
    /// Wall-clock timestamp (seconds since epoch) of the upload —
    /// drives the dialog's "last synced HH:MM:SS" footer.
    modified: Option<u64>,
    /// Populated for the `"error"` kind.
    error: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SftpExternalEditOpen {
    watcher_id: String,
    local_path: String,
}

fn emit_external_edit_event(app: &tauri::AppHandle, evt: SftpExternalEditEvent) {
    use tauri::Emitter;
    let _ = app.emit(SFTP_EXTERNAL_EDIT_EVENT, evt);
}

/// Strip path separators from a remote basename so the temp path
/// is always one component deep. Empty / dotty inputs collapse
/// to a generic placeholder.
fn sanitize_temp_basename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | '\0') { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim_matches('.').trim();
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Hand off `path` to the user's OS default opener. Spawning
/// (rather than waiting) lets the editor outlive this command —
/// the watcher thread is the long-lived bit.
fn open_with_default_app(path: &std::path::Path) -> std::io::Result<()> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        // `start` is a `cmd` builtin, not an exe. The empty `""`
        // is the window title slot — without it `start` interprets
        // the first quoted arg as the title and silently no-ops.
        Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(path).spawn()?;
    }
    Ok(())
}

/// Watch `local_path` for changes and push them back over SFTP
/// until [`stop_token`] fires. Runs on a dedicated OS thread —
/// not a tokio task — because the SFTP client's blocking helpers
/// internally `block_on` the shared runtime, which would deadlock
/// if we re-entered from within that runtime.
#[allow(clippy::too_many_arguments)]
fn external_edit_watch_loop(
    app: tauri::AppHandle,
    watcher_id: String,
    local_path: PathBuf,
    remote_path: String,
    sftp: SftpClient,
    stop_token: CancellationToken,
    mut last_mtime: Option<SystemTime>,
    mut last_size: u64,
) {
    let mut pending_change_since: Option<Instant> = None;

    while !stop_token.is_cancelled() {
        std::thread::sleep(EXTERNAL_EDIT_POLL);

        let meta = match std::fs::metadata(&local_path) {
            Ok(m) => m,
            // The user may have deleted the temp file from outside
            // the editor; skip this tick and try again next round.
            Err(_) => continue,
        };
        let cur_mtime = meta.modified().ok();
        let cur_size = meta.len();

        if cur_mtime != last_mtime || cur_size != last_size {
            // Reset the debounce window — wait until the file
            // settles before uploading.
            pending_change_since = Some(Instant::now());
            last_mtime = cur_mtime;
            last_size = cur_size;
            continue;
        }

        let Some(changed_at) = pending_change_since else {
            continue;
        };
        if changed_at.elapsed() < EXTERNAL_EDIT_DEBOUNCE {
            continue;
        }
        pending_change_since = None;

        emit_external_edit_event(
            &app,
            SftpExternalEditEvent {
                watcher_id: watcher_id.clone(),
                kind: "uploading".into(),
                bytes: None,
                modified: None,
                error: None,
            },
        );

        let bytes_res = std::fs::read(&local_path);
        match bytes_res {
            Err(e) => emit_external_edit_event(
                &app,
                SftpExternalEditEvent {
                    watcher_id: watcher_id.clone(),
                    kind: "error".into(),
                    bytes: None,
                    modified: None,
                    error: Some(format!("read local: {e}")),
                },
            ),
            Ok(bytes) => {
                let upload_res = sftp.write_file_blocking(&remote_path, &bytes);
                match upload_res {
                    Err(e) => emit_external_edit_event(
                        &app,
                        SftpExternalEditEvent {
                            watcher_id: watcher_id.clone(),
                            kind: "error".into(),
                            bytes: None,
                            modified: None,
                            error: Some(format!("upload: {e}")),
                        },
                    ),
                    Ok(_) => emit_external_edit_event(
                        &app,
                        SftpExternalEditEvent {
                            watcher_id: watcher_id.clone(),
                            kind: "uploaded".into(),
                            bytes: Some(bytes.len() as u64),
                            modified: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .ok()
                                .map(|d| d.as_secs()),
                            error: None,
                        },
                    ),
                }
            }
        }
    }

    emit_external_edit_event(
        &app,
        SftpExternalEditEvent {
            watcher_id,
            kind: "stopped".into(),
            bytes: None,
            modified: None,
            error: None,
        },
    );
}

/// Mirror a remote SFTP file to a local temp path, hand it off to
/// the user's OS default editor, and start a watcher thread that
/// auto-uploads any saves back. Returns a watcher id the frontend
/// passes to [`sftp_external_edit_stop`] when the dialog closes.
#[tauri::command]
fn sftp_open_external(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    path: String,
    saved_connection_index: Option<usize>,
) -> Result<SftpExternalEditOpen, String> {
    let session = get_or_open_ssh_session(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
    )?;
    let sftp = get_or_open_sftp_client(&state, &session, &host, port, &user, &auth_mode)?;

    let meta = sftp.stat_blocking(&path).map_err(|e| e.to_string())?;
    if meta.size > SFTP_EXTERNAL_EDIT_MAX {
        return Err(format!(
            "File is {} MB; external-editor limit is {} MB",
            meta.size / (1024 * 1024),
            SFTP_EXTERNAL_EDIT_MAX / (1024 * 1024),
        ));
    }

    let watcher_id = format!(
        "ext-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let temp_root = std::env::temp_dir()
        .join("pierx-sftp-edit")
        .join(&watcher_id);
    std::fs::create_dir_all(&temp_root)
        .map_err(|e| format!("create temp dir: {e}"))?;

    let basename = path.rsplit('/').next().unwrap_or("file");
    let local_path = temp_root.join(sanitize_temp_basename(basename));

    sftp.download_to_blocking(&path, &local_path)
        .map_err(|e| format!("download: {e}"))?;

    let post_download_meta = std::fs::metadata(&local_path)
        .map_err(|e| format!("stat temp file: {e}"))?;
    let last_mtime = post_download_meta.modified().ok();
    let last_size = post_download_meta.len();

    open_with_default_app(&local_path)
        .map_err(|e| format!("opener failed: {e}"))?;

    let stop_token = CancellationToken::new();
    {
        let mut map = state
            .external_editors
            .lock()
            .map_err(|_| "external editors state poisoned".to_string())?;
        map.insert(
            watcher_id.clone(),
            ExternalEditWatcher {
                stop_token: stop_token.clone(),
                local_path: local_path.clone(),
                cleanup_temp_dir: true,
            },
        );
    }

    let app_for_thread = app.clone();
    let watcher_id_for_thread = watcher_id.clone();
    let local_for_thread = local_path.clone();
    let remote_for_thread = path.clone();
    let sftp_for_thread = sftp.clone();
    let stop_for_thread = stop_token.clone();
    std::thread::Builder::new()
        .name(format!("sftp-extedit-{watcher_id}"))
        .spawn(move || {
            external_edit_watch_loop(
                app_for_thread,
                watcher_id_for_thread,
                local_for_thread,
                remote_for_thread,
                sftp_for_thread,
                stop_for_thread,
                last_mtime,
                last_size,
            );
        })
        .map_err(|e| format!("spawn watcher: {e}"))?;

    Ok(SftpExternalEditOpen {
        watcher_id,
        local_path: local_path.to_string_lossy().into_owned(),
    })
}

/// Tear down an external-editor session: cancel the watcher
/// thread (which exits on its next poll) and best-effort delete
/// the per-watcher temp dir. Idempotent — calling with a stale
/// id is a no-op.
#[tauri::command]
fn sftp_external_edit_stop(
    state: tauri::State<'_, AppState>,
    watcher_id: String,
) -> Result<(), String> {
    let handle = {
        let mut map = state
            .external_editors
            .lock()
            .map_err(|_| "external editors state poisoned".to_string())?;
        map.remove(&watcher_id)
    };
    let Some(h) = handle else { return Ok(()); };
    h.stop_token.cancel();
    if h.cleanup_temp_dir {
        if let Some(dir) = h.local_path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    Ok(())
}

// ── Log Stream ──────────────────────────────────────────────────

#[tauri::command]
fn log_stream_start(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    command: String,
    saved_connection_index: Option<usize>,
) -> Result<String, String> {
    // Reuse the terminal's SSH session (or any previously-cached panel
    // session) so a new log tail doesn't re-handshake. `ExecStream`
    // opens its own russh channel on the existing session — cheap
    // compared to a full connect.
    let stream = run_with_session_retry(
        &state,
        &host,
        port,
        &user,
        &auth_mode,
        &password,
        &key_path,
        saved_connection_index,
        |session| {
            session
                .spawn_exec_stream_blocking(&command)
                .map_err(|e| e.to_string())
        },
    )?;

    let id = format!(
        "log-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    state
        .log_streams
        .lock()
        .map_err(|_| "log state poisoned".to_string())?
        .insert(id.clone(), stream);

    Ok(id)
}

#[tauri::command]
fn log_stream_drain(
    state: tauri::State<'_, AppState>,
    stream_id: String,
) -> Result<Vec<LogEventView>, String> {
    let streams = state
        .log_streams
        .lock()
        .map_err(|_| "log state poisoned".to_string())?;

    let stream = streams
        .get(&stream_id)
        .ok_or_else(|| format!("unknown log stream: {}", stream_id))?;

    let events = stream.drain();
    Ok(events
        .into_iter()
        .map(|e| match e {
            pier_core::ssh::ExecEvent::Stdout(text) => LogEventView {
                kind: "stdout".into(),
                text,
            },
            pier_core::ssh::ExecEvent::Stderr(text) => LogEventView {
                kind: "stderr".into(),
                text,
            },
            pier_core::ssh::ExecEvent::Exit(code) => LogEventView {
                kind: "exit".into(),
                text: format!("{}", code),
            },
            pier_core::ssh::ExecEvent::Error(msg) => LogEventView {
                kind: "error".into(),
                text: msg,
            },
        })
        .collect())
}

#[tauri::command]
fn log_stream_stop(state: tauri::State<'_, AppState>, stream_id: String) -> Result<(), String> {
    let mut streams = state
        .log_streams
        .lock()
        .map_err(|_| "log state poisoned".to_string())?;
    streams.remove(&stream_id);
    Ok(())
}

// ── Local System ────────────────────────────────────────────────

#[tauri::command]
async fn local_docker_overview(all: bool) -> Result<DockerOverview, String> {
    // First-open path: one local Docker command only. The images,
    // volumes, and networks tabs load their own listings on demand.
    let containers = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<DockerContainerView>, String> {
        let fmt = "{{.ID}}\t{{.Image}}\t{{.Names}}\t{{.Status}}\t{{.State}}\t{{.CreatedAt}}\t{{.Ports}}\t{{.Labels}}";
        let mut cmd = std::process::Command::new("docker");
        cmd.args(["ps", "--format", fmt]);
        if all {
            cmd.arg("-a");
        }
        let output = cmd
            .output()
            .map_err(|e| format!("docker ps failed: {}", e))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(|line| {
                let parts: Vec<&str> = line.split('\t').collect();
                let state = parts.get(4).unwrap_or(&"").to_string();
                DockerContainerView {
                    cpu_perc: String::new(),
                    mem_usage: String::new(),
                    mem_perc: String::new(),
                    id: parts.first().unwrap_or(&"").to_string(),
                    image: parts.get(1).unwrap_or(&"").to_string(),
                    names: parts.get(2).unwrap_or(&"").to_string(),
                    status: parts.get(3).unwrap_or(&"").to_string(),
                    running: state == "running",
                    state,
                    created: parts.get(5).unwrap_or(&"").to_string(),
                    ports: parts.get(6).unwrap_or(&"").to_string(),
                    labels: parts.get(7).unwrap_or(&"").to_string(),
                }
            })
            .collect())
    })
    .await
    .map_err(|e| format!("docker ps join: {}", e))??;

    Ok(DockerOverview {
        containers,
        images: Vec::new(),
        volumes: Vec::new(),
        networks: Vec::new(),
    })
}

#[tauri::command]
async fn local_docker_images() -> Result<Vec<DockerImageView>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        std::process::Command::new("docker")
            .args([
                "images",
                "--format",
                "{{.ID}}\t{{.Repository}}\t{{.Tag}}\t{{.Size}}\t{{.CreatedAt}}",
            ])
            .output()
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|line| {
                        let p: Vec<&str> = line.split('\t').collect();
                        DockerImageView {
                            id: p.first().unwrap_or(&"").to_string(),
                            repository: p.get(1).unwrap_or(&"").to_string(),
                            tag: p.get(2).unwrap_or(&"").to_string(),
                            size: p.get(3).unwrap_or(&"").to_string(),
                            created: p.get(4).unwrap_or(&"").to_string(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .map_err(|e| format!("docker images join: {}", e))
}

#[tauri::command]
async fn local_docker_volumes() -> Result<Vec<DockerVolumeView>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        // Size / links are populated asynchronously by
        // `local_docker_volume_usage` so we skip `docker system df -v`
        // on this path. Client-side sort handles ordering.
        std::process::Command::new("docker")
            .args([
                "volume",
                "ls",
                "--format",
                "{{.Name}}\t{{.Driver}}\t{{.Mountpoint}}",
            ])
            .output()
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|line| {
                        let p: Vec<&str> = line.split('\t').collect();
                        DockerVolumeView {
                            name: p.first().unwrap_or(&"").to_string(),
                            driver: p.get(1).unwrap_or(&"").to_string(),
                            mountpoint: p.get(2).unwrap_or(&"").to_string(),
                            size: String::new(),
                            size_bytes: 0,
                            links: -1,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .map_err(|e| format!("docker volume ls join: {}", e))
}

#[tauri::command]
async fn local_docker_networks() -> Result<Vec<DockerNetworkView>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        std::process::Command::new("docker")
            .args([
                "network",
                "ls",
                "--format",
                "{{.ID}}\t{{.Name}}\t{{.Driver}}\t{{.Scope}}",
            ])
            .output()
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|line| {
                        let p: Vec<&str> = line.split('\t').collect();
                        DockerNetworkView {
                            id: p.first().unwrap_or(&"").to_string(),
                            name: p.get(1).unwrap_or(&"").to_string(),
                            driver: p.get(2).unwrap_or(&"").to_string(),
                            scope: p.get(3).unwrap_or(&"").to_string(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .map_err(|e| format!("docker network ls join: {}", e))
}

/// Local-docker counterpart of [`docker_stats`]. Runs
/// `docker stats --no-stream` against the host daemon and returns one row
/// per container. Offloaded to the blocking pool because the CLI always
/// waits for its sampling window before exiting.
#[tauri::command]
async fn local_docker_stats() -> Result<Vec<DockerContainerStatsView>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        std::process::Command::new("docker")
            .args([
                "stats",
                "--no-stream",
                "--format",
                "{{.ID}}\t{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.MemPerc}}",
            ])
            .output()
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| !l.is_empty())
                    .filter_map(|line| {
                        let p: Vec<&str> = line.split('\t').collect();
                        let id = p.first()?.to_string();
                        Some(DockerContainerStatsView {
                            id,
                            cpu_perc: p.get(2).unwrap_or(&"").to_string(),
                            mem_usage: p.get(3).unwrap_or(&"").to_string(),
                            mem_perc: p.get(4).unwrap_or(&"").to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .map_err(|e| format!("docker stats join: {}", e))
}

/// Local-docker counterpart of [`docker_volume_usage`]. Parses
/// `docker system df -v` through the shared pier-core parser so SSH and
/// local paths agree on malformed output.
#[tauri::command]
async fn local_docker_volume_usage() -> Result<Vec<DockerVolumeUsageView>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        std::process::Command::new("docker")
            .args(["system", "df", "-v", "--format", "{{json .}}"])
            .output()
            .ok()
            .map(|o| {
                docker::parse_volume_df(&String::from_utf8_lossy(&o.stdout))
                    .into_iter()
                    .map(|v| DockerVolumeUsageView {
                        size_bytes: docker::parse_size_to_bytes(&v.size),
                        name: v.name,
                        size: v.size,
                        links: v.links,
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .map_err(|e| format!("docker system df join: {}", e))
}

#[tauri::command]
async fn local_docker_action(container_id: String, action: String) -> Result<String, String> {
    let output = std::process::Command::new("docker")
        .args([&action, &container_id])
        .output()
        .map_err(|e| format!("docker {} failed: {}", action, e))?;
    if output.status.success() {
        Ok(action.clone())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

// `include_disks` mirrors `server_monitor_probe`: `false` skips the
// disk parts so the fast-tier poll doesn't `df` (and `lsblk` on Linux)
// every 5 s. The previous full snapshot's disks/blockDevices are
// retained on the frontend in between full polls.
#[tauri::command]
fn local_system_info(include_disks: bool) -> Result<ServerSnapshotView, String> {
    #[cfg(target_os = "macos")]
    {
        let uptime = std::process::Command::new("uptime")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let vm_stat = std::process::Command::new("vm_stat")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let sysctl = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<f64>()
                    .unwrap_or(0.0)
            })
            .unwrap_or(0.0);
        let mem_total_mb = sysctl / (1024.0 * 1024.0);
        // Parse free pages from vm_stat
        let free_pages: f64 = vm_stat
            .lines()
            .find(|l| l.starts_with("Pages free"))
            .and_then(|l| l.split_whitespace().last())
            .and_then(|v| v.trim_end_matches('.').parse::<f64>().ok())
            .unwrap_or(0.0);
        let page_size = 16384.0_f64; // Apple Silicon default
        let mem_free_mb = free_pages * page_size / (1024.0 * 1024.0);
        let mem_used_mb = mem_total_mb - mem_free_mb;
        // Disk — parse the full `df -hT` so the per-mount breakdown
        // shows up alongside the root-fs gauge. Skipped on fast-tier
        // polls; the frontend keeps the prior full snapshot's disks.
        let mut df_snap = server_monitor::ServerSnapshot::default();
        if include_disks {
            let df_full = std::process::Command::new("df")
                .args(["-hT"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            server_monitor::parse_df(&df_full, &mut df_snap);
        }
        let disk_total = df_snap.disk_total.clone();
        let disk_used = df_snap.disk_used.clone();
        let disk_avail = df_snap.disk_avail.clone();
        let disk_use_pct = if !include_disks
            || (df_snap.disk_use_pct == 0.0 && disk_total.is_empty())
        {
            -1.0
        } else {
            df_snap.disk_use_pct
        };
        // Load
        let load_parts: Vec<f64> = uptime
            .rsplit("load averages:")
            .next()
            .or_else(|| uptime.rsplit("load average:").next())
            .unwrap_or("")
            .split(|c: char| c == ',' || c == ' ')
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .collect();
        Ok(ServerSnapshotView {
            uptime,
            load_1: *load_parts.first().unwrap_or(&-1.0),
            load_5: *load_parts.get(1).unwrap_or(&-1.0),
            load_15: *load_parts.get(2).unwrap_or(&-1.0),
            mem_total_mb,
            mem_used_mb,
            mem_free_mb,
            swap_total_mb: 0.0,
            swap_used_mb: 0.0,
            disk_total,
            disk_used,
            disk_avail,
            disk_use_pct,
            cpu_pct: -1.0,
            cpu_count: 0,
            proc_count: 0,
            os_label: String::new(),
            net_rx_bps: -1.0,
            net_tx_bps: -1.0,
            top_processes: Vec::new(),
            top_processes_mem: Vec::new(),
            disks: df_snap
                .disks
                .into_iter()
                .map(|d| DiskEntryView {
                    filesystem: d.filesystem,
                    fs_type: d.fs_type,
                    total: d.total,
                    used: d.used,
                    avail: d.avail,
                    use_pct: d.use_pct,
                    mountpoint: d.mountpoint,
                })
                .collect(),
            // macOS local probe: lsblk doesn't exist; the BLOCK DEVICES
            // section stays empty and the UI hides itself accordingly.
            block_devices: Vec::new(),
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Linux fallback
        let uptime = fs::read_to_string("/proc/uptime").unwrap_or_default();
        let loadavg = fs::read_to_string("/proc/loadavg").unwrap_or_default();
        let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        fn parse_meminfo(info: &str, key: &str) -> f64 {
            info.lines()
                .find(|l| l.starts_with(key))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
                / 1024.0
        }
        let mem_total_mb = parse_meminfo(&meminfo, "MemTotal");
        let mem_free_mb =
            parse_meminfo(&meminfo, "MemAvailable").max(parse_meminfo(&meminfo, "MemFree"));
        let swap_total_mb = parse_meminfo(&meminfo, "SwapTotal");
        let swap_free = parse_meminfo(&meminfo, "SwapFree");
        let loads: Vec<f64> = loadavg
            .split_whitespace()
            .take(3)
            .filter_map(|s| s.parse().ok())
            .collect();
        let mut df_snap = server_monitor::ServerSnapshot::default();
        let mut block_devices: Vec<server_monitor::BlockDeviceEntry> = Vec::new();
        if include_disks {
            let df_full = std::process::Command::new("df")
                .args(["-hPT"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            server_monitor::parse_df(&df_full, &mut df_snap);
            // Local Linux probe: pull the same lsblk topology the SSH
            // path does so the BLOCK DEVICES UI works the same way for
            // a "Local Monitor" tab on a Linux desktop.
            let lsblk_out = std::process::Command::new("lsblk")
                .args([
                    "-P", "-b", "-o",
                    "NAME,KNAME,PKNAME,TYPE,SIZE,ROTA,TRAN,MODEL,FSTYPE,MOUNTPOINT",
                ])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            block_devices = server_monitor::parse_lsblk(&lsblk_out);
        }
        Ok(ServerSnapshotView {
            uptime: format!(
                "{:.0}s",
                uptime
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse::<f64>()
                    .unwrap_or(0.0)
            ),
            load_1: *loads.first().unwrap_or(&-1.0),
            load_5: *loads.get(1).unwrap_or(&-1.0),
            load_15: *loads.get(2).unwrap_or(&-1.0),
            mem_total_mb,
            mem_used_mb: mem_total_mb - mem_free_mb,
            mem_free_mb,
            swap_total_mb,
            swap_used_mb: swap_total_mb - swap_free,
            disk_total: df_snap.disk_total.clone(),
            disk_used: df_snap.disk_used.clone(),
            disk_avail: df_snap.disk_avail.clone(),
            disk_use_pct: if !include_disks
                || (df_snap.disk_use_pct == 0.0 && df_snap.disk_total.is_empty())
            {
                -1.0
            } else {
                df_snap.disk_use_pct
            },
            cpu_pct: -1.0,
            cpu_count: 0,
            proc_count: 0,
            os_label: String::new(),
            net_rx_bps: -1.0,
            net_tx_bps: -1.0,
            top_processes: Vec::new(),
            top_processes_mem: Vec::new(),
            disks: df_snap
                .disks
                .into_iter()
                .map(|d| DiskEntryView {
                    filesystem: d.filesystem,
                    fs_type: d.fs_type,
                    total: d.total,
                    used: d.used,
                    avail: d.avail,
                    use_pct: d.use_pct,
                    mountpoint: d.mountpoint,
                })
                .collect(),
            block_devices: block_devices
                .into_iter()
                .map(|b| BlockDeviceEntryView {
                    name: b.name,
                    kname: b.kname,
                    pkname: b.pkname,
                    dev_type: b.dev_type,
                    size_bytes: b.size_bytes,
                    rota: b.rota,
                    tran: b.tran,
                    model: b.model,
                    fs_type: b.fs_type,
                    mountpoint: b.mountpoint,
                })
                .collect(),
        })
    }
}

/// Append a single line to the shared file logger. Called from the
/// frontend's console-capture wrapper so browser-side diagnostics land
/// in the same file Rust-side ones do. Level/source are free-form
/// strings — we validate neither because the whole point is a dump of
/// whatever the UI was trying to say.
#[tauri::command]
fn log_write(level: String, source: String, message: String) {
    pier_core::logging::write_event(&level, &source, &message);
}

/// Toggle the "verbose diagnostics" gate. Off by default. When on,
/// diagnostic records that contain remote-machine output (hostnames,
/// `ps` command names, probe stdout excerpts) are written to the log
/// alongside the normal breadcrumb records. Intended to be wired to
/// a Settings toggle so a user can opt in when they're about to file
/// a bug, then turn it back off.
#[tauri::command]
fn log_set_verbose(enabled: bool) {
    pier_core::logging::set_verbose_diagnostics(enabled);
}

/// Read the current state of the verbose-diagnostics gate — lets a
/// Settings UI render the toggle in its actual position after restart.
#[tauri::command]
fn log_get_verbose() -> bool {
    pier_core::logging::verbose_diagnostics_enabled()
}

/// Resolve the absolute path of the active log file so the frontend
/// can surface it in menus / error dialogs ("send us this file").
/// Returns an empty string when the logger has not been initialised —
/// shouldn't happen in practice, but fail soft rather than panic.
#[tauri::command]
fn log_file_path() -> String {
    pier_core::logging::log_file_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Slurp the (truncated-per-run) log into a string so the UI can
/// render it inside a "view log" panel without spawning an external
/// editor. Caps at 2 MiB — the file is newly created on every run so
/// exceeding that bound means the user is in the middle of something
/// noisy and the tail is what they want anyway.
#[tauri::command]
fn log_read_tail(max_bytes: Option<u64>) -> Result<String, String> {
    let Some(path) = pier_core::logging::log_file_path() else {
        return Ok(String::new());
    };
    let cap = max_bytes.unwrap_or(2 * 1024 * 1024);
    match std::fs::metadata(&path) {
        Ok(meta) => {
            let size = meta.len();
            if size <= cap {
                std::fs::read_to_string(&path).map_err(|e| e.to_string())
            } else {
                use std::io::{Read, Seek, SeekFrom};
                let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                file.seek(SeekFrom::End(-(cap as i64)))
                    .map_err(|e| e.to_string())?;
                let mut buf = String::new();
                file.read_to_string(&mut buf).map_err(|e| e.to_string())?;
                // Drop any partial first line so the tail always starts
                // on a timestamp boundary.
                if let Some(idx) = buf.find('\n') {
                    Ok(buf[idx + 1..].to_string())
                } else {
                    Ok(buf)
                }
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Truncate the log file to 0 bytes. Harmless if the file has
/// already been deleted or was never created. The logger keeps its
/// write handle open — after the truncate, subsequent writes
/// resume at the (now zero) end-of-file, which may leave a few
/// stale bytes if a write was in flight during the call. That's a
/// one-time cosmetic blip, not a corruption risk.
#[tauri::command]
fn log_clear() -> Result<(), String> {
    let Some(path) = pier_core::logging::log_file_path() else {
        return Ok(());
    };
    match std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
    {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Toggle the Tauri webview DevTools. Compiled only in debug builds —
/// the release build ships without the `devtools` feature, so calling
/// this from a production frontend is a no-op that returns an error.
#[cfg(debug_assertions)]
#[tauri::command]
fn dev_toggle_devtools(window: tauri::WebviewWindow) {
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
        window.open_devtools();
    }
}

#[cfg(not(debug_assertions))]
#[tauri::command]
fn dev_toggle_devtools() -> Result<(), String> {
    Err("devtools disabled in release build".into())
}

/// Read a UTF-8 text file from the local filesystem. Used by the DB
/// panels' "Import SQL" right-click action to load an `.sql` file
/// the user picked via the OS file dialog. Capped at 64 MiB so a
/// pathological pick can't OOM the renderer; bigger imports go
/// through `mysqldump`/`pg_dump` (still TODO).
#[tauri::command]
fn local_read_text_file(path: String) -> Result<String, String> {
    let p = std::path::Path::new(&path);
    let meta = std::fs::metadata(p).map_err(|e| e.to_string())?;
    const CAP: u64 = 64 * 1024 * 1024;
    if meta.len() > CAP {
        return Err(format!(
            "file is {} bytes; refusing to read more than {} bytes",
            meta.len(),
            CAP
        ));
    }
    std::fs::read_to_string(p).map_err(|e| e.to_string())
}

/// Write a UTF-8 text file to the local filesystem. Used by the DB
/// panels' "Export SQL" right-click action to save the generated
/// dump to a path the user picked via the OS save dialog.
#[tauri::command]
fn local_write_text_file(path: String, content: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if let Some(parent) = p.parent() {
        // The save-dialog usually creates the directory itself, but
        // be tolerant in case the user typed a fresh path.
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(p, content).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            // Install the shared file logger before we do anything else —
            // the rest of this hook (and every subsequent command) can
            // then emit events that survive a crash. Truncates the file
            // on every run so it never grows unbounded; see
            // `pier_core::logging::init`.
            let log_dir = app
                .path()
                .app_log_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("pier-x").join("logs"));
            match pier_core::logging::init_under(&log_dir, "pier-x.log") {
                Ok(p) => {
                    pier_core::logging::write_event(
                        "INFO",
                        "startup",
                        &format!(
                            "Pier-X {} starting; log file: {}",
                            pier_core::VERSION,
                            p.display(),
                        ),
                    );
                }
                Err(e) => {
                    eprintln!("pier-x: log init failed at {}: {}", log_dir.display(), e);
                }
            }

            // Resolve the user-pack directory for the smart-mode
            // command library and seed the live library cell.
            // Bundled packs are read from the binary; user packs
            // live alongside everything else in app_data_dir.
            // Failure to resolve the dir means user packs are
            // disabled this session — the bundled set still works.
            let pack_dir = app
                .path()
                .app_data_dir()
                .ok()
                .map(|d| d.join("completions").join("packs"));
            if let Some(dir) = &pack_dir {
                if let Err(e) = std::fs::create_dir_all(dir) {
                    pier_core::logging::write_event(
                        "WARN",
                        "completions.library",
                        &format!("could not create pack dir {}: {}", dir.display(), e),
                    );
                }
            }
            terminal_smart::init_user_pack_dir(pack_dir);

            // Tell pier-core where the user-extras catalog lives so
            // the merged registry picks up custom entries on the
            // panel's first registry() call. The file is optional —
            // pier-core silently keeps the built-in catalog when it
            // doesn't exist.
            if let Ok(config_dir) = app.path().app_config_dir() {
                let extras = config_dir.join("software-extras.json");
                if let Err(e) = std::fs::create_dir_all(&config_dir) {
                    pier_core::logging::write_event(
                        "WARN",
                        "software.extras",
                        &format!("could not create config dir {}: {}", config_dir.display(), e),
                    );
                }
                let _ = package_manager::set_user_extras_path(extras);
            }

            // Initialise the ssh-mux wrapper + auto-generated config.
            // Failure to set this up is non-fatal — the worst case is
            // we don't get ControlMaster multiplexing for terminal-side
            // ssh, same behaviour as before this module existed.
            let cache_dir = app
                .path()
                .app_cache_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("com.pier-x"));
            match ssh_mux::init(&cache_dir) {
                Ok(()) => {
                    pier_core::logging::write_event(
                        "INFO",
                        "ssh.mux",
                        &format!(
                            "ssh ControlMaster mux ready; wrapper={:?} settings={:?}",
                            ssh_mux::wrapper_dir(),
                            ssh_mux::settings(),
                        ),
                    );
                    // Tell the pier-core SSH watcher where to look
                    // for the wrapper's per-shell hint files. Without
                    // this the watcher's `scan_with_mux_fallback`
                    // path is a no-op, which means the right-side
                    // panels go silent any time OpenSSH's
                    // `ControlMaster=auto` mode daemonises the master
                    // out of our PTY's ancestor tree (the steady
                    // state for any user who has the mux feature on).
                    if let Some(socket_dir) = ssh_mux::socket_dir() {
                        pier_core::terminal::ssh_watcher::set_mux_hint_dir(
                            socket_dir.to_path_buf(),
                        );
                    }
                }
                Err(e) => {
                    pier_core::logging::write_event(
                        "WARN",
                        "ssh.mux",
                        &format!("ssh-mux init failed at {}: {}", cache_dir.display(), e),
                    );
                }
            }

            // On Windows we draw our own caption controls (minimize /
            // maximize / close) in the titlebar — disable the OS chrome
            // so they don't double up. macOS keeps decorations on to
            // preserve the native traffic lights that titleBarStyle
            // "Overlay" renders on the left; Linux too until we add
            // proper CSD styling.
            #[cfg(target_os = "windows")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_decorations(false);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            dev_toggle_devtools,
            core_info,
            core_components_info,
            ssh_keys_list,
            list_directory,
            list_drives,
            local_create_file,
            local_create_dir,
            local_rename,
            local_remove,
            git_overview,
            git_panel_state,
            git_init_repo,
            git_global_config_get,
            git_global_config_set,
            git_diff,
            git_stage_paths,
            git_unstage_paths,
            git_stage_all,
            git_unstage_all,
            git_discard_paths,
            git_commit,
            git_commit_and_push,
            git_branch_list,
            git_checkout_branch,
            git_checkout_target,
            git_create_branch,
            git_create_branch_at,
            git_delete_branch,
            git_rename_branch,
            git_rename_remote_branch,
            git_delete_remote_branch,
            git_merge_branch,
            git_set_branch_tracking,
            git_unset_branch_tracking,
            git_recent_commits,
            git_graph_metadata,
            git_graph_history,
            git_commit_detail,
            git_commit_file_diff,
            git_comparison_files,
            git_comparison_diff,
            git_blame_file,
            git_push,
            git_pull,
            git_stash_list,
            git_stash_push,
            git_stash_apply,
            git_stash_pop,
            git_stash_drop,
            git_stash_reword,
            git_unpushed_commits,
            git_tags_list,
            git_create_tag,
            git_create_tag_at,
            git_delete_tag,
            git_push_tag,
            git_push_all_tags,
            git_remotes_list,
            git_add_remote,
            git_set_remote_url,
            git_remove_remote,
            git_fetch_remote,
            git_config_list,
            git_set_config_value,
            git_unset_config_value,
            git_reset_to_commit,
            git_amend_head_commit_message,
            git_reword_unpushed_commit,
            git_drop_commit,
            git_revert_commit,
            git_cherry_pick_commit,
            git_reflog_list,
            git_rebase_plan,
            git_execute_rebase,
            git_abort_rebase,
            git_continue_rebase,
            git_submodules_list,
            git_init_submodules,
            git_update_submodules,
            git_sync_submodules,
            git_conflicts_list,
            git_conflict_accept_all,
            git_conflict_mark_resolved,
            mysql_browse,
            mysql_execute,
            sqlite_browse,
            sqlite_execute,
            sqlite_execute_script,
            redis_browse,
            redis_execute,
            redis_rename_key,
            redis_delete_key,
            ssh_connections_list,
            ssh_connection_save,
            ssh_connection_delete,
            ssh_connection_resolve_password,
            ssh_connection_update,
            ssh_connections_reorder,
            ssh_group_rename,
            ssh_tunnel_open,
            ssh_tunnel_info,
            ssh_tunnel_list,
            ssh_tunnel_close,
            ssh_known_hosts_list,
            ssh_known_hosts_remove,
            ssh_session_prewarm,
            terminal_create,
            terminal_create_ssh,
            terminal_create_ssh_saved,
            terminal_write,
            terminal_resize,
            terminal_snapshot,
            terminal_set_scrollback_limit,
            terminal_current_cwd,
            terminal_close,
            terminal_validate_command,
            terminal_completions,
            terminal_completions_remote,
            terminal_man_synopsis,
            terminal_history_load,
            terminal_history_push,
            terminal_history_clear,
            completion_library_list,
            completion_library_reload,
            completion_library_install_pack,
            completion_library_install_pack_from_path,
            completion_library_remove_pack,
            postgres_browse,
            postgres_execute,
            db_test_connection,
            ssh_key_list,
            docker_overview,
            docker_images,
            docker_volumes,
            docker_networks,
            docker_container_action,
            sftp_browse,
            markdown_render,
            markdown_render_file,
            server_monitor_probe,
            firewall_snapshot,
            detect_services,
            db_detect,
            db_cred_save,
            db_cred_update,
            db_cred_delete,
            db_cred_resolve,
            docker_inspect_db_env,
            sqlite_remote_capable,
            sqlite_install_remote,
            sqlite_browse_remote,
            sqlite_execute_remote,
            software_registry,
            software_probe_remote,
            software_install_remote,
            software_update_remote,
            software_uninstall_remote,
            software_versions_remote,
            software_details_remote,
            software_install_preview,
            software_search_remote,
            software_install_arbitrary,
            software_bundles,
            software_co_install_suggestions,
            software_mirror_catalog,
            software_mirror_get,
            software_mirror_set,
            software_mirror_restore,
            software_mirror_benchmark,
            software_mirror_benchmark_client,
            software_user_extras_path,
            software_user_extras_read,
            software_user_extras_write,
            software_preferences_get,
            software_preferences_set_mirror,
            software_history_log,
            software_history_list,
            software_history_clear,
            postgres_create_user_remote,
            postgres_create_db_remote,
            postgres_open_remote_remote,
            mysql_create_user_remote,
            mysql_create_db_remote,
            mysql_open_remote_remote,
            redis_set_password_remote,
            redis_open_remote_remote,
            software_compose_templates,
            software_compose_apply,
            software_compose_down,
            software_clone_plan,
            software_db_metrics,
            software_service_action_remote,
            software_service_logs_remote,
            software_install_cancel,
            nginx_layout,
            nginx_read_file,
            nginx_save_file,
            nginx_validate,
            nginx_reload,
            nginx_toggle_site,
            nginx_create_file,
            sqlite_find_in_dir,
            docker_inspect,
            docker_remove_image,
            docker_remove_volume,
            docker_remove_network,
            docker_run_container,
            docker_prune_volumes,
            docker_prune_images,
            docker_volume_files,
            docker_stats,
            docker_volume_usage,
            docker_pull_image,
            sftp_mkdir,
            sftp_remove,
            sftp_rename,
            sftp_chmod,
            sftp_create_file,
            sftp_read_text,
            sftp_write_text,
            sftp_download,
            sftp_upload,
            sftp_upload_tree,
            sftp_download_tree,
            sftp_open_external,
            sftp_external_edit_stop,
            log_stream_start,
            log_stream_drain,
            log_stream_stop,
            local_docker_overview,
            local_docker_images,
            local_docker_volumes,
            local_docker_networks,
            local_docker_stats,
            local_docker_volume_usage,
            local_docker_action,
            local_docker_run_container,
            local_docker_prune_volumes,
            local_docker_prune_images,
            local_docker_volume_files,
            local_docker_pull_image,
            local_system_info,
            log_write,
            log_file_path,
            log_read_tail,
            log_clear,
            log_set_verbose,
            log_get_verbose,
            local_read_text_file,
            local_write_text_file,
            ssh_mux_get_settings,
            ssh_mux_set_settings,
            ssh_mux_forget_target,
            ssh_mux_shutdown_all,
            ssh_cred_cache_put_password,
            ssh_cred_cache_put_passphrase,
            ssh_cred_cache_forget,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // On a real app exit (window closed, Cmd-Q, signal),
            // walk the ssh-mux socket dir and `ssh -O exit` every
            // master we left running. Without this they keep the
            // remote sshd connection open after Pier-X has gone
            // away — confusing for the user, weird in `ps`, and
            // semantically wrong (the GUI is the lifecycle anchor
            // the user expects).
            //
            // RunEvent::Exit fires AFTER all windows are gone but
            // BEFORE the process actually returns from .run(), so
            // we still have a working tokio context for the
            // Command::status() calls.
            if let tauri::RunEvent::Exit = event {
                let closed = ssh_mux::shutdown_all_masters();
                pier_core::logging::write_event(
                    "INFO",
                    "ssh.mux",
                    &format!("app exit: closed {} ssh master(s)", closed),
                );
                // Wipe the in-memory credential cache too. Belt and
                // braces: the process is exiting so the heap is
                // about to die anyway, but `clear()` zeroes
                // pointers explicitly which makes the intent
                // auditable and protects against any post-exit
                // dump path that might otherwise capture them.
                if let Some(state) = app.try_state::<AppState>() {
                    state.ssh_cred_cache.clear();
                }
            }
        });
}
