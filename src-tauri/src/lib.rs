use pier_core::connections::ConnectionStore;
use pier_core::credentials;
use pier_core::markdown;
use pier_core::services::docker;
use pier_core::services::git::{CommitInfo, GitClient, StashEntry};
use pier_core::services::mysql::{self as mysql_service, MysqlClient, MysqlConfig};
use pier_core::services::postgres::{PostgresClient, PostgresConfig};
use pier_core::services::redis::{RedisClient, RedisConfig};
use pier_core::services::server_monitor;
use pier_core::services::sqlite::SqliteClient;
use pier_core::ssh::service_detector;
use pier_core::ssh::{AuthMethod, ExecStream, HostKeyVerifier, SshConfig, SshSession, Tunnel};
use pier_core::terminal::{Cell, Color, NotifyFn, PierTerminal};
use serde::Serialize;
use std::collections::HashMap;
use std::ffi::c_void;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

mod git_panel;
use git_panel::*;

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
        }
    }
}

struct ManagedTerminal {
    terminal: PierTerminal,
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
}

extern "C" fn tauri_terminal_notify(_user_data: *mut c_void, _event: u32) {}

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
    if let Some(rest) = trimmed.strip_prefix("~/").or_else(|| trimmed.strip_prefix("~\\")) {
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
    if port == 0 { 22 } else { port }
}

fn normalize_mysql_port(port: u16) -> u16 {
    if port == 0 { 3306 } else { port }
}

fn normalize_redis_port(port: u16) -> u16 {
    if port == 0 { 6379 } else { port }
}

fn normalize_postgres_port(port: u16) -> u16 {
    if port == 0 { 5432 } else { port }
}

fn map_postgres_preview(
    result: pier_core::services::postgres::QueryResult,
) -> DataPreview {
    DataPreview {
        columns: result.columns.clone(),
        rows: result
            .rows
            .into_iter()
            .map(|row| row.into_iter().map(|cell| cell.unwrap_or_default()).collect())
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
            .map(|row| row.into_iter().map(|cell| cell.unwrap_or_default()).collect())
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
) -> Result<SshSession, String> {
    let resolved_host = host.trim();
    let resolved_user = user.trim();
    if resolved_host.is_empty() || resolved_user.is_empty() {
        return Err(String::from("SSH host and user must not be empty."));
    }
    let auth = match auth_mode {
        "key" => AuthMethod::PublicKeyFile {
            private_key_path: key_path.to_string(),
            passphrase_credential_id: None,
        },
        "agent" => AuthMethod::Agent,
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
    SshSession::connect_blocking(&config, HostKeyVerifier::default())
        .map_err(|e| e.to_string())
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
) -> Result<SshSession, String> {
    if let Some(index) = saved_index {
        if let Ok(config) = open_saved_ssh_config(index) {
            return SshSession::connect_blocking(&config, HostKeyVerifier::default())
                .map_err(|e| e.to_string());
        }
    }
    build_ssh_session_from_params(host, port, user, auth_mode, password, key_path)
}

/// Resolve the effective password for a panel call. If `password` is
/// empty (the frontend hasn't primed it yet) and `saved_index`
/// points at a saved connection with a keychain credential, fetch
/// the password directly from the OS keychain. Used by SFTP
/// commands so a saved password connection works immediately on the
/// first browse, without waiting for the frontend's async prime.
fn resolve_password_for_auth(
    auth_mode: &str,
    password: &str,
    saved_index: Option<usize>,
) -> String {
    if auth_mode != "password" || !password.is_empty() {
        return password.to_string();
    }
    let Some(index) = saved_index else {
        return password.to_string();
    };
    let Ok(store) = ConnectionStore::load_default() else {
        return password.to_string();
    };
    let Some(conn) = store.connections.get(index) else {
        return password.to_string();
    };
    let AuthMethod::KeychainPassword { credential_id } = &conn.auth else {
        return password.to_string();
    };
    match credentials::get(credential_id) {
        Ok(Some(resolved)) => resolved,
        _ => password.to_string(),
    }
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

/// Get a cached SSH session for SFTP work, opening one (and caching
/// it) if none exists. Falls back to resolving the password from the
/// keychain when `saved_index` is set and no password was passed in.
/// On any failure — cached session is dead, password can't be
/// resolved, handshake fails — returns a descriptive error.
fn get_or_open_sftp_ssh_session(
    state: &tauri::State<'_, AppState>,
    host: &str,
    port: u16,
    user: &str,
    auth_mode: &str,
    password: &str,
    key_path: &str,
    saved_index: Option<usize>,
) -> Result<Arc<SshSession>, String> {
    let key = sftp_cache_key(host, port, user, auth_mode);

    {
        let cache = state
            .sftp_sessions
            .lock()
            .map_err(|_| "ssh session cache poisoned".to_string())?;
        if let Some(existing) = cache.get(&key) {
            return Ok(Arc::clone(existing));
        }
    }

    let effective_password = resolve_password_for_auth(auth_mode, password, saved_index);
    let session = build_ssh_session_from_params(
        host,
        port,
        user,
        auth_mode,
        &effective_password,
        key_path,
    )?;
    let arc = Arc::new(session);

    state
        .sftp_sessions
        .lock()
        .map_err(|_| "ssh session cache poisoned".to_string())?
        .insert(key, Arc::clone(&arc));
    Ok(arc)
}

/// Drop the cached session for a tab's fingerprint. Called when an
/// SFTP operation fails in a way that suggests the underlying SSH
/// connection has died, so the next call opens a fresh one.
fn evict_sftp_session(state: &tauri::State<'_, AppState>, host: &str, port: u16, user: &str, auth_mode: &str) {
    let key = sftp_cache_key(host, port, user, auth_mode);
    if let Ok(mut cache) = state.sftp_sessions.lock() {
        cache.remove(&key);
    }
}

/// Convert raw POSIX permission bits into the 10-character `ls -l`
/// style string. Used to decorate SFTP listings in the inspector.
/// Special bits (setuid / setgid / sticky) are not rendered — the
/// three rwx triplets plus the leading type glyph are enough for
/// the panel's use.
fn format_posix_permissions(bits: u32, is_dir: bool, is_link: bool) -> String {
    let mut out = String::with_capacity(10);
    out.push(if is_link { 'l' } else if is_dir { 'd' } else { '-' });
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
    let resolved = preferred
        .unwrap_or_default()
        .trim()
        .to_string();
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

fn map_sqlite_preview(result: pier_core::services::sqlite::SqliteQueryResult) -> Option<DataPreview> {
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
    terminal: PierTerminal,
    shell: String,
    cols: u16,
    rows: u16,
) -> Result<TerminalSessionInfo, String> {
    let session_id = format!(
        "term-{}",
        state.next_terminal_id.fetch_add(1, Ordering::Relaxed) + 1
    );
    let mut sessions = state
        .terminals
        .lock()
        .map_err(|_| String::from("terminal state poisoned"))?;
    sessions.insert(session_id.clone(), ManagedTerminal { terminal });

    Ok(TerminalSessionInfo {
        session_id,
        shell,
        cols,
        rows,
    })
}

fn create_ssh_terminal_from_config(
    state: tauri::State<'_, AppState>,
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
        AuthMethod::PublicKeyFile { .. } => "key",
        _ => "password",
    };
    let cache_key = sftp_cache_key(&config.host, config.port, &config.user, auth_mode_key);
    if let Ok(mut cache) = state.sftp_sessions.lock() {
        cache.insert(cache_key, Arc::new(session.clone()));
    }

    let pty = session
        .open_shell_channel_blocking(resolved_cols, resolved_rows)
        .map_err(|error| error.to_string())?;
    let terminal = PierTerminal::with_pty(
        Box::new(pty),
        resolved_cols,
        resolved_rows,
        tauri_terminal_notify as NotifyFn,
        std::ptr::null_mut(),
    )
    .map_err(|error| error.to_string())?;

    store_terminal_session(state, terminal, shell, resolved_cols, resolved_rows)
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

fn build_terminal_lines(snapshot: &pier_core::terminal::GridSnapshot, alive: bool) -> Vec<TerminalLine> {
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
        profile: if cfg!(debug_assertions) { "debug" } else { "release" },
        ui_target: "tauri",
        home_dir: home_dir().display().to_string(),
        workspace_root: workspace_root().display().to_string(),
        platform: if cfg!(target_os = "macos") { "macos" } else if cfg!(target_os = "windows") { "windows" } else { "linux" },
        default_shell: default_shell(),
        services: vec!["terminal", "ssh", "git", "mysql", "sqlite", "redis"],
    }
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
            let kind = if metadata.is_dir() { "directory" } else { "file" };
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
                let yoe = (doe - doe/1461 + doe/36524 - doe/146097) / 365;
                let doy = doe - (365*yoe + yoe/4 - yoe/100);
                let mp = (5*doy + 2) / 153;
                let d = doy - (153*mp + 2)/5 + 1;
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
        client.diff_untracked(&file_path).map_err(|error| error.to_string())
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
) -> Result<String, String> {
    let client = open_git_client(path)?;
    client
        .commit_with(message.trim(), signoff.unwrap_or(false), amend.unwrap_or(false))
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
fn git_recent_commits(path: Option<String>, limit: Option<usize>) -> Result<Vec<GitCommitEntry>, String> {
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
            credentials::set(&credential_id, &resolved_password).map_err(|error| error.to_string())?;
            AuthMethod::KeychainPassword { credential_id }
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
            let existing_credential_id = match &old_auth {
                AuthMethod::KeychainPassword { credential_id } => Some(credential_id.clone()),
                _ => None,
            };
            match password.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()) {
                Some(resolved_password) => {
                    let credential_id = existing_credential_id
                        .unwrap_or_else(|| make_credential_id(resolved_host, resolved_user));
                    credentials::set(&credential_id, &resolved_password)
                        .map_err(|error| error.to_string())?;
                    AuthMethod::KeychainPassword { credential_id }
                }
                None => match existing_credential_id {
                    Some(credential_id) => AuthMethod::KeychainPassword { credential_id },
                    None => return Err(String::from("SSH password must not be empty.")),
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
            if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
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
fn ssh_connections_reorder(
    order: Vec<usize>,
    groups: Vec<Option<String>>,
) -> Result<(), String> {
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
) -> Result<TunnelInfoView, String> {
    let resolved_remote_host = if remote_host.trim().is_empty() {
        String::from("127.0.0.1")
    } else {
        remote_host.trim().to_string()
    };
    if remote_port == 0 {
        return Err(String::from("Tunnel remote port must not be empty."));
    }

    let session = build_ssh_session_from_params(
        &host, port, &user, &auth_mode, &password, &key_path,
    )?;
    let tunnel = session
        .open_local_forward_blocking(local_port.unwrap_or(0), &resolved_remote_host, remote_port)
        .map_err(|error| error.to_string())?;
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

#[tauri::command]
fn ssh_tunnel_close(
    state: tauri::State<'_, AppState>,
    tunnel_id: String,
) -> Result<(), String> {
    let mut tunnels = state
        .tunnels
        .lock()
        .map_err(|_| String::from("tunnel state poisoned"))?;
    tunnels
        .remove(&tunnel_id)
        .map(|_| ())
        .ok_or_else(|| format!("unknown tunnel: {}", tunnel_id))
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
) -> Result<RedisBrowserState, String> {
    let resolved_host = host.trim();
    if resolved_host.is_empty() {
        return Err(String::from("Redis host must not be empty."));
    }

    let client = RedisClient::connect_blocking(RedisConfig {
        host: resolved_host.to_string(),
        port: normalize_redis_port(port),
        db,
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
        client.inspect_blocking(&key_name).ok().map(map_redis_details)
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
    cols: u16,
    rows: u16,
    shell: Option<String>,
) -> Result<TerminalSessionInfo, String> {
    let resolved_cols = cols.max(40);
    let resolved_rows = rows.max(12);
    let resolved_shell = shell
        .filter(|candidate| !candidate.trim().is_empty())
        .unwrap_or_else(default_shell);
    let terminal = PierTerminal::new(
        resolved_cols,
        resolved_rows,
        &resolved_shell,
        tauri_terminal_notify as NotifyFn,
        std::ptr::null_mut(),
    )
    .map_err(|error| error.to_string())?;

    store_terminal_session(state, terminal, resolved_shell, resolved_cols, resolved_rows)
}

#[tauri::command]
fn terminal_create_ssh(
    state: tauri::State<'_, AppState>,
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
    create_ssh_terminal_from_config(state, config, cols, rows)
}

#[tauri::command]
fn terminal_create_ssh_saved(
    state: tauri::State<'_, AppState>,
    cols: u16,
    rows: u16,
    index: usize,
) -> Result<TerminalSessionInfo, String> {
    let config = open_saved_ssh_config(index)?;
    create_ssh_terminal_from_config(state, config, cols, rows)
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

    let result = client
        .execute_blocking(&sql)
        .map_err(|e| e.to_string())?;
    Ok(map_postgres_query_result(result))
}

// ── Docker ──────────────────────────────────────────────────────────

/// Grab an SSH session for Docker work, preferring the saved-config
/// path when `saved_index` is set. Reuses the shared session cache so
/// switching between the Docker / SFTP / monitor panels does not
/// re-handshake on every call.
fn get_or_open_docker_ssh_session(
    state: &tauri::State<'_, AppState>,
    saved_index: Option<usize>,
    host: &str,
    port: u16,
    user: &str,
    auth_mode: &str,
    password: &str,
    key_path: &str,
) -> Result<Arc<SshSession>, String> {
    let key = sftp_cache_key(host, port, user, auth_mode);
    {
        let cache = state
            .sftp_sessions
            .lock()
            .map_err(|_| "ssh session cache poisoned".to_string())?;
        if let Some(existing) = cache.get(&key) {
            return Ok(Arc::clone(existing));
        }
    }
    let session = build_ssh_session_saved_or_params(
        saved_index, host, port, user, auth_mode, password, key_path,
    )?;
    let arc = Arc::new(session);
    state
        .sftp_sessions
        .lock()
        .map_err(|_| "ssh session cache poisoned".to_string())?
        .insert(key, Arc::clone(&arc));
    Ok(arc)
}

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
    let session = get_or_open_docker_ssh_session(
        &state, saved_connection_index, &host, port, &user, &auth_mode, &password, &key_path,
    )?;

    // Fan out the four cheap listings over the single cached SSH session —
    // russh multiplexes channels so they all run concurrently. The slow
    // `docker stats` and `docker system df -v` calls are split out into
    // `docker_stats` and `docker_volume_usage` so the panel can render
    // base data in one RTT and enrich in the background.
    // The four base listings each fire one `docker ... ls` exec — cheap
    // (~100ms each over SSH). Sequential is fine now that the slow calls
    // (`docker stats`, `docker system df -v`) are split out into
    // `docker_stats` / `docker_volume_usage` which the UI fires in the
    // background without blocking the first paint.
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
        })
        .collect();

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

    Ok(DockerOverview {
        containers,
        images,
        volumes,
        networks,
    })
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
    let session = get_or_open_docker_ssh_session(
        &state, saved_connection_index, &host, port, &user, &auth_mode, &password, &key_path,
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
    let session = get_or_open_docker_ssh_session(
        &state, saved_connection_index, &host, port, &user, &auth_mode, &password, &key_path,
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
    let session = build_ssh_session_saved_or_params(
        saved_connection_index, &host, port, &user, &auth_mode, &password, &key_path,
    )?;

    match action.as_str() {
        "start" => docker::start_blocking(&session, &container_id)
            .map_err(|e| e.to_string())
            .map(|_| String::from("started")),
        "stop" => docker::stop_blocking(&session, &container_id)
            .map_err(|e| e.to_string())
            .map(|_| String::from("stopped")),
        "restart" => docker::restart_blocking(&session, &container_id)
            .map_err(|e| e.to_string())
            .map(|_| String::from("restarted")),
        "remove" => docker::remove_blocking(&session, &container_id, false)
            .map_err(|e| e.to_string())
            .map(|_| String::from("removed")),
        _ => Err(format!("unknown docker action: {}", action)),
    }
}

// ── SFTP ────────────────────────────────────────────────────────────

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
    let target_path = path
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| String::from("/"));

    // Try with the cached session first; if opening an SFTP channel
    // or listing fails (session is stale / server bounced), evict
    // and retry once with a freshly-opened session.
    let mut attempt = 0;
    loop {
        let session = get_or_open_sftp_ssh_session(
            &state, &host, port, &user, &auth_mode, &password, &key_path, saved_connection_index,
        )?;

        let sftp = match session.open_sftp_blocking() {
            Ok(s) => s,
            Err(e) if attempt == 0 => {
                evict_sftp_session(&state, &host, port, &user, &auth_mode);
                attempt += 1;
                let _ = e;
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };

        let canonical = sftp
            .canonicalize_blocking(&target_path)
            .unwrap_or_else(|_| target_path.clone());

        let raw_entries = match sftp.list_dir_blocking(&canonical) {
            Ok(v) => v,
            Err(e) if attempt == 0 => {
                evict_sftp_session(&state, &host, port, &user, &auth_mode);
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
    let source = markdown::load_file(std::path::Path::new(&path))
        .map_err(|e| e.to_string())?;
    Ok(markdown::render_html(&source))
}

// ── Server Monitor ──────────────────────────────────────────────────

#[tauri::command]
fn server_monitor_probe(
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<ServerSnapshotView, String> {
    let effective_password = resolve_password_for_auth(&auth_mode, &password, saved_connection_index);
    let session = build_ssh_session_from_params(
        &host, port, &user, &auth_mode, &effective_password, &key_path,
    )?;

    let snap = server_monitor::probe_blocking(&session)
        .map_err(|e| e.to_string())?;

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
    })
}

// ── Service Detection ────────────────────────────────────────────

#[tauri::command]
fn detect_services(
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<Vec<DetectedServiceView>, String> {
    let effective_password = resolve_password_for_auth(&auth_mode, &password, saved_connection_index);
    let session = build_ssh_session_from_params(
        &host, port, &user, &auth_mode, &effective_password, &key_path,
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

// ── Docker Extended ─────────────────────────────────────────────

#[tauri::command]
fn docker_inspect(
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    container_id: String,
    saved_connection_index: Option<usize>,
) -> Result<String, String> {
    let session = build_ssh_session_saved_or_params(
        saved_connection_index, &host, port, &user, &auth_mode, &password, &key_path,
    )?;
    docker::inspect_container_blocking(&session, &container_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_remove_image(
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
    let session = build_ssh_session_saved_or_params(
        saved_connection_index, &host, port, &user, &auth_mode, &password, &key_path,
    )?;
    docker::remove_image_blocking(&session, &image_id, force)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_remove_volume(
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    volume_name: String,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    let session = build_ssh_session_saved_or_params(
        saved_connection_index, &host, port, &user, &auth_mode, &password, &key_path,
    )?;
    docker::remove_volume_blocking(&session, &volume_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_remove_network(
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    network_name: String,
    saved_connection_index: Option<usize>,
) -> Result<(), String> {
    let session = build_ssh_session_saved_or_params(
        saved_connection_index, &host, port, &user, &auth_mode, &password, &key_path,
    )?;
    docker::remove_network_blocking(&session, &network_name)
        .map_err(|e| e.to_string())
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
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    options: DockerRunOptionsView,
    saved_connection_index: Option<usize>,
) -> Result<String, String> {
    let session = build_ssh_session_saved_or_params(
        saved_connection_index, &host, port, &user, &auth_mode, &password, &key_path,
    )?;
    docker::run_container_blocking(&session, &options.into()).map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_prune_volumes(
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    saved_connection_index: Option<usize>,
) -> Result<String, String> {
    let session = build_ssh_session_saved_or_params(
        saved_connection_index, &host, port, &user, &auth_mode, &password, &key_path,
    )?;
    docker::prune_volumes_blocking(&session).map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_pull_image(
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
    let session = build_ssh_session_saved_or_params(
        saved_connection_index, &host, port, &user, &auth_mode, &password, &key_path,
    )?;
    let env = env_prefix.unwrap_or_default();
    let env_refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    docker::pull_image_blocking(&session, &image_ref, &env_refs).map_err(|e| e.to_string())
}

#[tauri::command]
fn local_docker_pull_image(
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
    host: String,
    port: u16,
    user: String,
    auth_mode: String,
    password: String,
    key_path: String,
    mountpoint: String,
    saved_connection_index: Option<usize>,
) -> Result<String, String> {
    let session = build_ssh_session_saved_or_params(
        saved_connection_index, &host, port, &user, &auth_mode, &password, &key_path,
    )?;
    docker::list_volume_files_blocking(&session, &mountpoint).map_err(|e| e.to_string())
}

#[tauri::command]
fn local_docker_run_container(options: DockerRunOptionsView) -> Result<String, String> {
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
        if g.is_empty() { continue; }
        args.push("-p".into());
        args.push(if h.is_empty() { g.into() } else { format!("{h}:{g}") });
    }
    for (k, v) in &opts.env {
        if k.trim().is_empty() { continue; }
        args.push("-e".into());
        args.push(format!("{}={}", k.trim(), v));
    }
    for (h, g) in &opts.volumes {
        let h = h.trim();
        let g = g.trim();
        if h.is_empty() || g.is_empty() { continue; }
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
fn local_docker_prune_volumes() -> Result<String, String> {
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
fn local_docker_volume_files(mountpoint: String) -> Result<String, String> {
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
    let session = get_or_open_sftp_ssh_session(
        &state, &host, port, &user, &auth_mode, &password, &key_path, saved_connection_index,
    )?;
    let sftp = session.open_sftp_blocking().map_err(|e| e.to_string())?;
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
    let session = get_or_open_sftp_ssh_session(
        &state, &host, port, &user, &auth_mode, &password, &key_path, saved_connection_index,
    )?;
    let sftp = session.open_sftp_blocking().map_err(|e| e.to_string())?;
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
    let session = get_or_open_sftp_ssh_session(
        &state, &host, port, &user, &auth_mode, &password, &key_path, saved_connection_index,
    )?;
    let sftp = session.open_sftp_blocking().map_err(|e| e.to_string())?;
    sftp.rename_blocking(&from, &to).map_err(|e| e.to_string())
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
    let session = get_or_open_sftp_ssh_session(
        &state, &host, port, &user, &auth_mode, &password, &key_path, saved_connection_index,
    )?;
    let sftp = session.open_sftp_blocking().map_err(|e| e.to_string())?;
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
    let session = get_or_open_sftp_ssh_session(
        &state, &host, port, &user, &auth_mode, &password, &key_path, saved_connection_index,
    )?;
    let sftp = session.open_sftp_blocking().map_err(|e| e.to_string())?;
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
    let session = get_or_open_sftp_ssh_session(
        &state, &host, port, &user, &auth_mode, &password, &key_path, saved_connection_index,
    )?;
    let sftp = session.open_sftp_blocking().map_err(|e| e.to_string())?;
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
    let session = get_or_open_sftp_ssh_session(
        &state, &host, port, &user, &auth_mode, &password, &key_path, saved_connection_index,
    )?;
    let sftp = session.open_sftp_blocking().map_err(|e| e.to_string())?;
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
    let effective_password = resolve_password_for_auth(&auth_mode, &password, saved_connection_index);
    let session = build_ssh_session_from_params(
        &host, port, &user, &auth_mode, &effective_password, &key_path,
    )?;
    let stream = session
        .spawn_exec_stream_blocking(&command)
        .map_err(|e| e.to_string())?;

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
fn log_stream_stop(
    state: tauri::State<'_, AppState>,
    stream_id: String,
) -> Result<(), String> {
    let mut streams = state
        .log_streams
        .lock()
        .map_err(|_| "log state poisoned".to_string())?;
    streams.remove(&stream_id);
    Ok(())
}

// ── Local System ────────────────────────────────────────────────

#[tauri::command]
fn local_docker_overview(all: bool) -> Result<DockerOverview, String> {
    // CPU/MEM snapshot — best-effort. Tab-separated so we avoid a JSON
    // dependency on this branch; keyed on short id because `docker stats`
    // prints 12-char ids by default.
    let stats_by_id: std::collections::HashMap<String, (String, String, String)> =
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
                        Some((
                            id,
                            (
                                p.get(2).unwrap_or(&"").to_string(),
                                p.get(3).unwrap_or(&"").to_string(),
                                p.get(4).unwrap_or(&"").to_string(),
                            ),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();

    let fmt = "{{.ID}}\t{{.Image}}\t{{.Names}}\t{{.Status}}\t{{.State}}\t{{.CreatedAt}}\t{{.Ports}}";
    let mut cmd = std::process::Command::new("docker");
    cmd.args(["ps", "--format", fmt]);
    if all { cmd.arg("-a"); }
    let output = cmd
        .output()
        .map_err(|e| format!("docker ps failed: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let containers: Vec<DockerContainerView> = stdout.lines().filter(|l| !l.is_empty()).map(|line| {
        let parts: Vec<&str> = line.split('\t').collect();
        let state = parts.get(4).unwrap_or(&"").to_string();
        let id = parts.first().unwrap_or(&"").to_string();
        let short_id: String = id.chars().take(12).collect();
        let stat = stats_by_id.get(&id).or_else(|| stats_by_id.get(&short_id));
        DockerContainerView {
            cpu_perc: stat.map(|s| s.0.clone()).unwrap_or_default(),
            mem_usage: stat.map(|s| s.1.clone()).unwrap_or_default(),
            mem_perc: stat.map(|s| s.2.clone()).unwrap_or_default(),
            id,
            image: parts.get(1).unwrap_or(&"").to_string(),
            names: parts.get(2).unwrap_or(&"").to_string(),
            status: parts.get(3).unwrap_or(&"").to_string(),
            running: state == "running",
            state,
            created: parts.get(5).unwrap_or(&"").to_string(),
            ports: parts.get(6).unwrap_or(&"").to_string(),
        }
    }).collect();

    let img_output = std::process::Command::new("docker")
        .args(["images", "--format", "{{.ID}}\t{{.Repository}}\t{{.Tag}}\t{{.Size}}\t{{.CreatedAt}}"])
        .output().ok();
    let images: Vec<DockerImageView> = img_output.map(|o| {
        String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.is_empty()).map(|line| {
            let p: Vec<&str> = line.split('\t').collect();
            DockerImageView { id: p.first().unwrap_or(&"").to_string(), repository: p.get(1).unwrap_or(&"").to_string(), tag: p.get(2).unwrap_or(&"").to_string(), size: p.get(3).unwrap_or(&"").to_string(), created: p.get(4).unwrap_or(&"").to_string() }
        }).collect()
    }).unwrap_or_default();

    // Per-volume size via `docker system df -v`. Reuse the pier-core
    // parser so SSH and local paths agree on malformed output.
    let sizes_by_name: std::collections::HashMap<String, docker::VolumeDiskUsage> =
        std::process::Command::new("docker")
            .args(["system", "df", "-v", "--format", "{{json .}}"])
            .output()
            .ok()
            .map(|o| {
                docker::parse_volume_df(&String::from_utf8_lossy(&o.stdout))
                    .into_iter()
                    .map(|v| (v.name.clone(), v))
                    .collect()
            })
            .unwrap_or_default();

    let mut volumes: Vec<DockerVolumeView> = std::process::Command::new("docker")
        .args(["volume", "ls", "--format", "{{.Name}}\t{{.Driver}}\t{{.Mountpoint}}"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.is_empty())
                .map(|line| {
                    let p: Vec<&str> = line.split('\t').collect();
                    let name = p.first().unwrap_or(&"").to_string();
                    let (size, size_bytes, links) = sizes_by_name
                        .get(&name)
                        .map(|df| {
                            (
                                df.size.clone(),
                                docker::parse_size_to_bytes(&df.size),
                                df.links,
                            )
                        })
                        .unwrap_or_else(|| (String::new(), 0, -1));
                    DockerVolumeView {
                        name,
                        driver: p.get(1).unwrap_or(&"").to_string(),
                        mountpoint: p.get(2).unwrap_or(&"").to_string(),
                        size,
                        size_bytes,
                        links,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    volumes.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then(a.name.cmp(&b.name)));

    let networks: Vec<DockerNetworkView> = std::process::Command::new("docker")
        .args(["network", "ls", "--format", "{{.ID}}\t{{.Name}}\t{{.Driver}}\t{{.Scope}}"])
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
        .unwrap_or_default();

    Ok(DockerOverview { containers, images, volumes, networks })
}

#[tauri::command]
fn local_docker_action(container_id: String, action: String) -> Result<String, String> {
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
        let uptime = std::process::Command::new("uptime").output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let vm_stat = std::process::Command::new("vm_stat").output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let sysctl = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().unwrap_or(0.0))
            .unwrap_or(0.0);
        let mem_total_mb = sysctl / (1024.0 * 1024.0);
        // Parse free pages from vm_stat
        let free_pages: f64 = vm_stat.lines()
            .find(|l| l.starts_with("Pages free"))
            .and_then(|l| l.split_whitespace().last())
            .and_then(|v| v.trim_end_matches('.').parse::<f64>().ok())
            .unwrap_or(0.0);
        let page_size = 16384.0_f64; // Apple Silicon default
        let mem_free_mb = free_pages * page_size / (1024.0 * 1024.0);
        let mem_used_mb = mem_total_mb - mem_free_mb;
        // Disk
        let df = std::process::Command::new("df").args(["-h", "/"]).output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let df_parts: Vec<&str> = df.lines().nth(1).unwrap_or("").split_whitespace().collect();
        let disk_total = df_parts.get(1).unwrap_or(&"").to_string();
        let disk_used = df_parts.get(2).unwrap_or(&"").to_string();
        let disk_avail = df_parts.get(3).unwrap_or(&"").to_string();
        let disk_use_pct = df_parts.get(4).unwrap_or(&"0%").trim_end_matches('%').parse::<f64>().unwrap_or(-1.0);
        // Load
        let load_parts: Vec<f64> = uptime.rsplit("load averages:").next()
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
            mem_total_mb, mem_used_mb, mem_free_mb,
            swap_total_mb: 0.0, swap_used_mb: 0.0,
            disk_total, disk_used, disk_avail, disk_use_pct,
            cpu_pct: -1.0,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Linux fallback
        let uptime = fs::read_to_string("/proc/uptime").unwrap_or_default();
        let loadavg = fs::read_to_string("/proc/loadavg").unwrap_or_default();
        let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        fn parse_meminfo(info: &str, key: &str) -> f64 {
            info.lines().find(|l| l.starts_with(key))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0) / 1024.0
        }
        let mem_total_mb = parse_meminfo(&meminfo, "MemTotal");
        let mem_free_mb = parse_meminfo(&meminfo, "MemAvailable").max(parse_meminfo(&meminfo, "MemFree"));
        let swap_total_mb = parse_meminfo(&meminfo, "SwapTotal");
        let swap_free = parse_meminfo(&meminfo, "SwapFree");
        let loads: Vec<f64> = loadavg.split_whitespace().take(3).filter_map(|s| s.parse().ok()).collect();
        let df = std::process::Command::new("df").args(["-h", "/"]).output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        let df_parts: Vec<&str> = df.lines().nth(1).unwrap_or("").split_whitespace().collect();
        Ok(ServerSnapshotView {
            uptime: format!("{:.0}s", uptime.split_whitespace().next().unwrap_or("0").parse::<f64>().unwrap_or(0.0)),
            load_1: *loads.first().unwrap_or(&-1.0),
            load_5: *loads.get(1).unwrap_or(&-1.0),
            load_15: *loads.get(2).unwrap_or(&-1.0),
            mem_total_mb, mem_used_mb: mem_total_mb - mem_free_mb, mem_free_mb,
            swap_total_mb, swap_used_mb: swap_total_mb - swap_free,
            disk_total: df_parts.get(1).unwrap_or(&"").to_string(),
            disk_used: df_parts.get(2).unwrap_or(&"").to_string(),
            disk_avail: df_parts.get(3).unwrap_or(&"").to_string(),
            disk_use_pct: df_parts.get(4).unwrap_or(&"0%").trim_end_matches('%').parse::<f64>().unwrap_or(-1.0),
            cpu_pct: -1.0,
        })
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
        .setup(|_app| {
            // On Windows we draw our own caption controls (minimize /
            // maximize / close) in the titlebar — disable the OS chrome
            // so they don't double up. macOS keeps decorations on to
            // preserve the native traffic lights that titleBarStyle
            // "Overlay" renders on the left; Linux too until we add
            // proper CSD styling.
            #[cfg(target_os = "windows")]
            {
                use tauri::Manager;
                if let Some(window) = _app.get_webview_window("main") {
                    let _ = window.set_decorations(false);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            dev_toggle_devtools,
            core_info,
            list_directory,
            git_overview,
            git_panel_state,
            git_init_repo,
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
            git_drop_commit,
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
            ssh_tunnel_close,
            terminal_create,
            terminal_create_ssh,
            terminal_create_ssh_saved,
            terminal_write,
            terminal_resize,
            terminal_snapshot,
            terminal_set_scrollback_limit,
            terminal_close,
            postgres_browse,
            postgres_execute,
            docker_overview,
            docker_container_action,
            sftp_browse,
            markdown_render,
            markdown_render_file,
            server_monitor_probe,
            detect_services,
            docker_inspect,
            docker_remove_image,
            docker_remove_volume,
            docker_remove_network,
            docker_run_container,
            docker_prune_volumes,
            docker_volume_files,
            docker_stats,
            docker_volume_usage,
            docker_pull_image,
            sftp_mkdir,
            sftp_remove,
            sftp_rename,
            sftp_download,
            sftp_upload,
            sftp_upload_tree,
            sftp_download_tree,
            log_stream_start,
            log_stream_drain,
            log_stream_stop,
            local_docker_overview,
            local_docker_action,
            local_docker_run_container,
            local_docker_prune_volumes,
            local_docker_volume_files,
            local_docker_pull_image,
            local_system_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
