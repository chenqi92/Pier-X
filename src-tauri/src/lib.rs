use pier_core::connections::{
    self, ConnectionStore, DbCredentialPatch, NewDbCredential, ResolvedDbCredential,
};
use pier_core::credentials;
use pier_core::markdown;
use pier_core::services::docker;
use pier_core::services::firewall;
use pier_core::services::git::{CommitInfo, GitClient, StashEntry, UnpushedCommit};
use pier_core::services::mysql::{self as mysql_service, MysqlClient, MysqlConfig};
use pier_core::services::postgres::{PostgresClient, PostgresConfig};
use pier_core::services::redis::{RedisClient, RedisConfig};
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

mod git_panel;
use git_panel::*;

mod ssh_mux;

mod ssh_cred_cache;
use ssh_cred_cache::{SshCredCache, TargetKey};

mod terminal_smart;
use terminal_smart::{
    terminal_completions, terminal_man_synopsis, terminal_validate_command,
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MysqlBrowserState {
    database_name: String,
    databases: Vec<String>,
    table_name: String,
    tables: Vec<String>,
    columns: Vec<MysqlColumnView>,
    preview: Option<DataPreview>,
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
struct SqliteBrowserState {
    path: String,
    table_name: String,
    tables: Vec<String>,
    columns: Vec<SqliteColumnView>,
    preview: Option<DataPreview>,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RedisBrowserState {
    pong: String,
    pattern: String,
    limit: usize,
    truncated: bool,
    key_name: String,
    keys: Vec<String>,
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PostgresBrowserState {
    database_name: String,
    databases: Vec<String>,
    schema_name: String,
    table_name: String,
    tables: Vec<String>,
    columns: Vec<PostgresColumnView>,
    preview: Option<DataPreview>,
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
    std::env::current_dir().unwrap_or_else(|_| home_dir())
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
            if let Ok(mut slot) = guard.last_fail.lock() {
                *slot = Some((Instant::now(), e.clone(), cred_fp));
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
    let session = SshSession::connect_blocking(&config, HostKeyVerifier::default())
        .map_err(|error| error.to_string())?;

    // Seed the SFTP cache with the freshly-authenticated connection.
    // `SshSession` is `Arc`-backed (`#[derive(Clone)]`) so the clone
    // just bumps a refcount; the right-side SFTP panel can then
    // reuse the live channel instead of re-handshaking (and avoid
    // the "InvalidConfig" error when the tab has a key/agent auth
    // or a password that's already been consumed). Key format must
    // match `sftp_cache_key`.
    let auth_mode_key = match &config.auth {
        AuthMethod::Agent => "agent",
        AuthMethod::Auto | AuthMethod::AutoChain { .. } => "auto",
        AuthMethod::PublicKeyFile { .. } => "key",
        _ => "password",
    };
    let cache_key = sftp_cache_key(&config.host, config.port, &config.user, auth_mode_key);
    if let Ok(mut cache) = state.sftp_sessions.lock() {
        cache.insert(cache_key, Arc::new(session.clone()));
    }

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
    let tables = if database_name.is_empty() {
        Vec::new()
    } else {
        client
            .list_tables_blocking(&database_name)
            .map_err(|error| error.to_string())?
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
            })
            .collect()
    };
    let preview = if database_name.is_empty()
        || table_name.is_empty()
        || !mysql_service::is_safe_ident(&database_name)
        || !mysql_service::is_safe_ident(&table_name)
    {
        None
    } else {
        client
            .execute_blocking(&format!(
                "SELECT * FROM `{database_name}`.`{table_name}` LIMIT 24"
            ))
            .ok()
            .map(map_mysql_preview)
    };

    Ok(MysqlBrowserState {
        database_name,
        databases,
        table_name,
        tables,
        columns,
        preview,
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
    let preview = if table_name.is_empty() {
        None
    } else {
        let escaped = table_name.replace('"', "\"\"");
        map_sqlite_preview(client.execute(&format!("SELECT * FROM \"{escaped}\" LIMIT 24;")))
    };

    Ok(SqliteBrowserState {
        path: resolved_path.to_string(),
        table_name,
        tables,
        columns,
        preview,
    })
}

#[tauri::command]
fn redis_browse(
    host: String,
    port: u16,
    db: i64,
    pattern: Option<String>,
    key: Option<String>,
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
    let scan = client
        .scan_keys_blocking(&effective_pattern, 120)
        .map_err(|error| error.to_string())?;
    let key_name = choose_active_item(key, &scan.keys);
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

    Ok(RedisBrowserState {
        pong,
        pattern: effective_pattern,
        limit: scan.limit,
        truncated: scan.truncated,
        key_name,
        keys: scan.keys,
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
    let schema_name = schema
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| String::from("public"));
    let tables = if database_name.is_empty() {
        Vec::new()
    } else {
        client
            .list_tables_blocking(&schema_name)
            .map_err(|e| e.to_string())?
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

    Ok(PostgresBrowserState {
        database_name,
        databases,
        schema_name,
        table_name,
        tables,
        columns,
        preview,
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
        match server_monitor::probe_with_baseline_blocking(&session, &mut local_baseline) {
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
struct RemoteSqliteBrowserState {
    path: String,
    table_name: String,
    tables: Vec<String>,
    columns: Vec<SqliteColumnView>,
    preview: Option<DataPreview>,
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

    Ok(RemoteSqliteBrowserState {
        path: trimmed.to_string(),
        table_name,
        tables,
        columns,
        preview,
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

#[tauri::command]
fn local_system_info() -> Result<ServerSnapshotView, String> {
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
        // shows up alongside the root-fs gauge.
        let df_full = std::process::Command::new("df")
            .args(["-hT"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let mut df_snap = server_monitor::ServerSnapshot::default();
        server_monitor::parse_df(&df_full, &mut df_snap);
        let disk_total = df_snap.disk_total.clone();
        let disk_used = df_snap.disk_used.clone();
        let disk_avail = df_snap.disk_avail.clone();
        let disk_use_pct = if df_snap.disk_use_pct == 0.0 && disk_total.is_empty() {
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
        let df_full = std::process::Command::new("df")
            .args(["-hPT"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let mut df_snap = server_monitor::ServerSnapshot::default();
        server_monitor::parse_df(&df_full, &mut df_snap);
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
            disk_use_pct: if df_snap.disk_use_pct == 0.0 && df_snap.disk_total.is_empty() {
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
            redis_browse,
            redis_execute,
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
            terminal_man_synopsis,
            postgres_browse,
            postgres_execute,
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
            sqlite_browse_remote,
            sqlite_execute_remote,
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
