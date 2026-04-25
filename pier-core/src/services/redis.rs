//! Redis / Valkey client panel.
//!
//! M5a per-service tool. The UI path is:
//!
//! 1. User clicks a detected Redis service pill in the terminal
//!    view → `ssh::tunnel::open_local_forward` binds a local
//!    port (typically `10000 + 6379 = 16379`).
//! 2. User clicks again → Pier-X opens a new `redis` tab
//!    pointing at `localhost:<tunnel_port>`.
//! 3. The Tauri runtime calls [`RedisClient::connect`] and
//!    surfaces the result back to the shell.
//!
//! ## Connection management
//!
//! Every [`RedisClient`] wraps a single
//! [`redis::aio::ConnectionManager`]. The manager transparently
//! reconnects when the underlying TCP socket dies (e.g. the
//! tunnel flaps), so the shell doesn't need to track connection
//! state — it either gets a result or an error.
//!
//! ## Scanning, not KEYS
//!
//! `KEYS *` on a production Redis with millions of keys can
//! pin the server for seconds. We use `SCAN` with a caller-
//! supplied `count` hint and cap the total keys returned per
//! request to [`DEFAULT_SCAN_LIMIT`]. Pagination is left for
//! M5a+ — the initial UI just shows the first N matches with a
//! "reached scan limit" hint.
//!
//! ## Auth
//!
//! Redis 6+ supports AUTH with an optional ACL username plus a
//! password. Both live on [`RedisConfig`] as `Option<String>` and
//! are threaded into [`redis::ConnectionInfo`] explicitly so values
//! with `@` / `:` / `/` round-trip safely — URL encoding through
//! `to_url()` would mangle those. Empty string is treated the same
//! as `None` (no AUTH sent), so the pre-auth flow still works.
//!
//! ## Not yet
//!
//! * TLS. Same rationale — the SSH tunnel already encrypts.
//! * Pub/sub and streams. These need a long-lived read side
//!   and a dedicated tokio task, which doesn't fit the current
//!   request-reply command shape.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use redis::aio::ConnectionManager;
use redis::{
    AsyncCommands, Client, ConnectionAddr, IntoConnectionInfo, RedisConnectionInfo,
    RedisError as NativeRedisError,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::ssh::runtime;

/// Hard cap for a single [`RedisClient::scan_keys`] call. The
/// UI still lets users specify a smaller limit, but we never
/// return more than this no matter what the caller asks for.
pub const DEFAULT_SCAN_LIMIT: usize = 1000;

/// Errors surfaced by the Redis client. Kept deliberately flat
/// — the UI only ever shows the `Display` impl to the user.
#[derive(Debug, thiserror::Error)]
pub enum RedisError {
    /// Underlying `redis` crate error (connect, command, IO).
    #[error("redis: {0}")]
    Native(#[from] NativeRedisError),

    /// Caller supplied a malformed URL / host / port.
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

/// Result alias for redis ops.
pub type Result<T, E = RedisError> = std::result::Result<T, E>;

/// Connection config for a Redis endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RedisConfig {
    /// Hostname or IP. Usually `"127.0.0.1"` when reached via
    /// an SSH tunnel.
    pub host: String,
    /// TCP port. The tunnel's local port, not the remote 6379.
    pub port: u16,
    /// Logical database index (Redis default is 0). Ignored on
    /// Redis Cluster, which only supports db 0.
    pub db: i64,
    /// Optional ACL username (Redis 6+). `None` or empty sends
    /// no `AUTH username` prefix, falling back to password-only
    /// AUTH against the implicit `default` user.
    #[serde(default)]
    pub username: Option<String>,
    /// Optional AUTH secret. `None` or empty skips AUTH.
    #[serde(default)]
    pub password: Option<String>,
}

impl RedisConfig {
    /// Build a Redis connection URL of the form
    /// `redis://<host>:<port>/<db>`. Credentials are intentionally
    /// omitted — `connect` builds a [`ConnectionInfo`] directly so
    /// special characters in passwords don't need URL encoding.
    /// Kept public because the old M5a callers (tests, tools) still
    /// use it for the no-auth case.
    pub fn to_url(&self) -> String {
        if self.db == 0 {
            format!("redis://{}:{}", self.host, self.port)
        } else {
            format!("redis://{}:{}/{}", self.host, self.port, self.db)
        }
    }

    /// Convert a configured username/password pair into the
    /// `Option<String>` shape [`RedisConnectionInfo`] expects.
    /// Empty strings normalize to `None` so the UI can pass ""
    /// and still produce a password-less connection.
    fn auth_parts(&self) -> (Option<String>, Option<String>) {
        let u = self
            .username
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let p = self
            .password
            .as_ref()
            .map(|s| s.clone())
            .filter(|s| !s.is_empty());
        (u, p)
    }
}

/// High-level key-type tag. Stored as a string so serialized
/// command payloads don't need a versioned discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    /// Unknown or missing key — the server returned "none".
    None,
    /// Plain string (GET/SET).
    String,
    /// List (LPUSH/RPUSH).
    List,
    /// Set (SADD).
    Set,
    /// Sorted set (ZADD).
    ZSet,
    /// Hash (HSET).
    Hash,
    /// Stream (XADD).
    Stream,
}

impl KeyKind {
    /// Parse the string returned by `TYPE` into a tag.
    pub fn parse(s: &str) -> Self {
        match s {
            "string" => KeyKind::String,
            "list" => KeyKind::List,
            "set" => KeyKind::Set,
            "zset" => KeyKind::ZSet,
            "hash" => KeyKind::Hash,
            "stream" => KeyKind::Stream,
            _ => KeyKind::None,
        }
    }

    /// Short lowercase label used in the FFI JSON payload.
    pub fn as_str(self) -> &'static str {
        match self {
            KeyKind::None => "none",
            KeyKind::String => "string",
            KeyKind::List => "list",
            KeyKind::Set => "set",
            KeyKind::ZSet => "zset",
            KeyKind::Hash => "hash",
            KeyKind::Stream => "stream",
        }
    }
}

/// One key's summary metadata, returned by
/// [`RedisClient::inspect`]. The `preview` field holds a small
/// slice of the value rendered to strings — never the entire
/// value, no matter how large the key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyDetails {
    /// The key name the user asked about.
    pub key: String,
    /// Key type label ("string", "list", ...).
    pub kind: String,
    /// Length of the top-level value:
    ///   * string: number of bytes
    ///   * list/set/zset/hash/stream: number of elements
    ///   * none: 0
    pub length: u64,
    /// Remaining TTL in seconds. `-1` means no TTL, `-2` means
    /// the key does not exist (same semantics as `TTL` itself).
    pub ttl_seconds: i64,
    /// Object encoding (e.g. `"ziplist"`, `"listpack"`,
    /// `"raw"`). Pulled from `OBJECT ENCODING`, or empty if
    /// the server refused (ACL hiding OBJECT).
    pub encoding: String,
    /// Human-readable value preview, truncated for display.
    /// Shape varies by type — see the module docs on inspect.
    pub preview: Vec<String>,
    /// True when `preview` was truncated vs the real value.
    pub preview_truncated: bool,
}

/// Result of an arbitrary Redis command execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandResult {
    /// Short single-line summary used by the UI header.
    pub summary: String,
    /// Response body rendered into display lines.
    pub lines: Vec<String>,
    /// Wall-clock execution time on the shared runtime.
    pub elapsed_ms: u64,
}

/// How many preview entries / bytes to return per inspect.
const PREVIEW_ITEMS: usize = 32;
/// Byte cap for string-key previews.
const PREVIEW_STRING_BYTES: usize = 1024;

/// Redis client handle. Cheap to clone — the underlying
/// [`ConnectionManager`] is reference-counted through `Arc`.
#[derive(Clone)]
pub struct RedisClient {
    manager: Arc<Mutex<ConnectionManager>>,
}

impl std::fmt::Debug for RedisClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisClient")
            .field("refcount", &Arc::strong_count(&self.manager))
            .finish()
    }
}

impl RedisClient {
    /// Open a connection to the configured Redis endpoint and
    /// verify liveness with a `PING`. Returns an error if the
    /// TCP connect, RESP handshake, or PING fails.
    pub async fn connect(config: RedisConfig) -> Result<Self> {
        if config.host.is_empty() {
            return Err(RedisError::InvalidConfig("empty host".into()));
        }
        if config.port == 0 {
            return Err(RedisError::InvalidConfig("port must be > 0".into()));
        }
        let (username, password) = config.auth_parts();
        // redis 1.2 makes `ConnectionInfo` fields crate-private, so we
        // must build it through the public builder chain. Start from a
        // bare `ConnectionAddr`, layer the ACL / password / DB on the
        // returned `ConnectionInfo`. This path (unlike `from_url`)
        // preserves the raw password bytes and doesn't require URL
        // encoding, so `@` / `:` / `/` in secrets work correctly.
        let mut redis_info = RedisConnectionInfo::default().set_db(config.db);
        if let Some(u) = username {
            redis_info = redis_info.set_username(u);
        }
        if let Some(p) = password {
            redis_info = redis_info.set_password(p);
        }
        let conn_info = ConnectionAddr::Tcp(config.host.clone(), config.port)
            .into_connection_info()?
            .set_redis_settings(redis_info);
        let client = Client::open(conn_info)?;
        let mut manager = ConnectionManager::new(client).await?;

        // Sanity ping. ConnectionManager will have handshaked
        // by now, but we still want a round-trip so connect
        // errors surface immediately instead of on first use.
        let reply: String = redis::cmd("PING").query_async(&mut manager).await?;
        if reply != "PONG" {
            return Err(RedisError::InvalidConfig(format!(
                "unexpected PING reply: {reply}"
            )));
        }

        Ok(Self {
            manager: Arc::new(Mutex::new(manager)),
        })
    }

    /// Blocking wrapper for [`Self::connect`].
    pub fn connect_blocking(config: RedisConfig) -> Result<Self> {
        runtime::shared().block_on(Self::connect(config))
    }

    /// `PING` round-trip. Used as a cheap liveness check.
    pub async fn ping(&self) -> Result<String> {
        let mut conn = self.manager.lock().await;
        let reply: String = redis::cmd("PING").query_async(&mut *conn).await?;
        Ok(reply)
    }

    /// Blocking wrapper for [`Self::ping`].
    pub fn ping_blocking(&self) -> Result<String> {
        runtime::shared().block_on(self.ping())
    }

    /// `SCAN` through the keyspace collecting up to `limit`
    /// keys that match `pattern`. `limit` is clamped to
    /// [`DEFAULT_SCAN_LIMIT`].
    ///
    /// The pattern follows Redis glob syntax (`*`, `?`, `[]`).
    /// Pass `"*"` to enumerate everything under the cap.
    pub async fn scan_keys(&self, pattern: &str, limit: usize) -> Result<ScanResult> {
        let effective_limit = limit.clamp(1, DEFAULT_SCAN_LIMIT);
        let mut conn = self.manager.lock().await;

        let mut cursor: u64 = 0;
        let mut keys: Vec<String> = Vec::new();
        let mut truncated = false;

        loop {
            // SCAN <cursor> MATCH <pattern> COUNT 512
            let (next_cursor, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(512)
                .query_async(&mut *conn)
                .await?;

            for k in batch {
                if keys.len() >= effective_limit {
                    truncated = true;
                    break;
                }
                keys.push(k);
            }

            if truncated || next_cursor == 0 {
                if next_cursor != 0 {
                    truncated = true;
                }
                break;
            }
            cursor = next_cursor;
        }

        keys.sort_unstable();
        Ok(ScanResult {
            keys,
            truncated,
            limit: effective_limit,
        })
    }

    /// Blocking wrapper for [`Self::scan_keys`].
    pub fn scan_keys_blocking(&self, pattern: &str, limit: usize) -> Result<ScanResult> {
        runtime::shared().block_on(self.scan_keys(pattern, limit))
    }

    /// Cursor-driven scan that returns one page of keys with
    /// per-key TYPE + TTL metadata. Drives the panel's
    /// load-more flow plus the type-badge / TTL-chip enrichments.
    ///
    /// `cursor` is `"0"` to start a fresh scan, otherwise the
    /// `next_cursor` from the previous page. The method walks
    /// SCAN until it has either filled `limit` keys or the
    /// cursor returns `"0"` (end of keyspace) — whichever comes
    /// first. After that it issues one TYPE + one TTL command
    /// per collected key, pipelined together so the round-trip
    /// cost is amortised across the whole batch.
    pub async fn scan_keys_paged(
        &self,
        pattern: &str,
        cursor: &str,
        limit: usize,
    ) -> Result<PagedScanResult> {
        let effective_limit = limit.clamp(1, DEFAULT_SCAN_LIMIT);
        let mut conn = self.manager.lock().await;

        let mut current: u64 = cursor.parse().unwrap_or(0);
        let mut collected: Vec<String> = Vec::with_capacity(effective_limit.min(256));
        let started = Instant::now();

        loop {
            let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(current)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(512)
                .query_async(&mut *conn)
                .await?;
            for k in batch {
                if collected.len() >= effective_limit {
                    break;
                }
                collected.push(k);
            }
            current = next;
            if current == 0 || collected.len() >= effective_limit {
                break;
            }
        }

        // Pipeline TYPE + PTTL for every collected key so the
        // round trip is one batch instead of `2 * len(keys)`.
        // PTTL is in milliseconds — convert to seconds for the
        // public type so the UI doesn't have to.
        let mut entries: Vec<KeyEntry> = Vec::with_capacity(collected.len());
        if !collected.is_empty() {
            let mut pipe = redis::pipe();
            for k in &collected {
                pipe.cmd("TYPE").arg(k);
                pipe.cmd("PTTL").arg(k);
            }
            let raw: Vec<redis::Value> = pipe.query_async(&mut *conn).await?;
            for (i, k) in collected.iter().enumerate() {
                let kind = match raw.get(i * 2) {
                    Some(redis::Value::SimpleString(s)) => s.clone(),
                    Some(redis::Value::BulkString(b)) => {
                        String::from_utf8_lossy(b).into_owned()
                    }
                    Some(redis::Value::Okay) => "ok".to_string(),
                    _ => String::new(),
                };
                let pttl: i64 = match raw.get(i * 2 + 1) {
                    Some(redis::Value::Int(n)) => *n,
                    _ => -2,
                };
                let ttl_seconds = match pttl {
                    -1 | -2 => pttl,
                    ms if ms > 0 => ms / 1000,
                    _ => -2,
                };
                entries.push(KeyEntry {
                    key: k.clone(),
                    kind: kind.to_lowercase(),
                    ttl_seconds,
                });
            }
        }

        Ok(PagedScanResult {
            next_cursor: current.to_string(),
            keys: entries,
            rtt_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// Blocking wrapper for [`Self::scan_keys_paged`].
    pub fn scan_keys_paged_blocking(
        &self,
        pattern: &str,
        cursor: &str,
        limit: usize,
    ) -> Result<PagedScanResult> {
        runtime::shared().block_on(self.scan_keys_paged(pattern, cursor, limit))
    }

    /// Fetch metadata + a bounded preview for a single key.
    ///
    /// Never reads more than [`PREVIEW_ITEMS`] collection
    /// entries or [`PREVIEW_STRING_BYTES`] string bytes, so it
    /// is safe to call on multi-GB keys.
    pub async fn inspect(&self, key: &str) -> Result<KeyDetails> {
        let mut conn = self.manager.lock().await;

        let type_reply: String = redis::cmd("TYPE").arg(key).query_async(&mut *conn).await?;
        let kind = KeyKind::parse(&type_reply);

        let ttl: i64 = redis::cmd("TTL").arg(key).query_async(&mut *conn).await?;

        // OBJECT ENCODING may be blocked by ACL; treat failure
        // as "unknown encoding" rather than bubbling up.
        let encoding: String = redis::cmd("OBJECT")
            .arg("ENCODING")
            .arg(key)
            .query_async(&mut *conn)
            .await
            .unwrap_or_default();

        let mut preview: Vec<String> = Vec::new();
        let mut preview_truncated = false;
        let length: u64 = match kind {
            KeyKind::None => 0,
            KeyKind::String => {
                let len: i64 = redis::cmd("STRLEN")
                    .arg(key)
                    .query_async(&mut *conn)
                    .await
                    .unwrap_or(0);
                let value: String = conn.get(key).await.unwrap_or_default();
                preview_truncated = value.len() > PREVIEW_STRING_BYTES;
                let slice = if preview_truncated {
                    safe_byte_prefix(&value, PREVIEW_STRING_BYTES)
                } else {
                    value.as_str()
                };
                preview.push(slice.to_string());
                len.max(0) as u64
            }
            KeyKind::List => {
                let len: i64 = redis::cmd("LLEN")
                    .arg(key)
                    .query_async(&mut *conn)
                    .await
                    .unwrap_or(0);
                let items: Vec<String> = redis::cmd("LRANGE")
                    .arg(key)
                    .arg(0)
                    .arg(PREVIEW_ITEMS as i64 - 1)
                    .query_async(&mut *conn)
                    .await
                    .unwrap_or_default();
                preview_truncated = (len as usize) > items.len();
                preview = items;
                len.max(0) as u64
            }
            KeyKind::Set => {
                let len: i64 = redis::cmd("SCARD")
                    .arg(key)
                    .query_async(&mut *conn)
                    .await
                    .unwrap_or(0);
                // SSCAN one page so we don't pay for the whole
                // set on large keys.
                let (_next, items): (u64, Vec<String>) = redis::cmd("SSCAN")
                    .arg(key)
                    .arg(0)
                    .arg("COUNT")
                    .arg(PREVIEW_ITEMS as i64)
                    .query_async(&mut *conn)
                    .await
                    .unwrap_or((0, Vec::new()));
                preview_truncated = (len as usize) > items.len();
                preview = items;
                len.max(0) as u64
            }
            KeyKind::ZSet => {
                let len: i64 = redis::cmd("ZCARD")
                    .arg(key)
                    .query_async(&mut *conn)
                    .await
                    .unwrap_or(0);
                let items: Vec<(String, f64)> = redis::cmd("ZRANGE")
                    .arg(key)
                    .arg(0)
                    .arg(PREVIEW_ITEMS as i64 - 1)
                    .arg("WITHSCORES")
                    .query_async(&mut *conn)
                    .await
                    .unwrap_or_default();
                preview_truncated = (len as usize) > items.len();
                preview = items
                    .into_iter()
                    .map(|(m, s)| format!("{s}  {m}"))
                    .collect();
                len.max(0) as u64
            }
            KeyKind::Hash => {
                let len: i64 = redis::cmd("HLEN")
                    .arg(key)
                    .query_async(&mut *conn)
                    .await
                    .unwrap_or(0);
                let (_next, entries): (u64, Vec<String>) = redis::cmd("HSCAN")
                    .arg(key)
                    .arg(0)
                    .arg("COUNT")
                    .arg(PREVIEW_ITEMS as i64)
                    .query_async(&mut *conn)
                    .await
                    .unwrap_or((0, Vec::new()));
                // HSCAN returns [field, value, field, value, ...]
                preview = entries
                    .chunks(2)
                    .map(|pair| match pair {
                        [f, v] => format!("{f} = {v}"),
                        _ => pair.join(""),
                    })
                    .collect();
                preview_truncated = (len as usize) > preview.len();
                len.max(0) as u64
            }
            KeyKind::Stream => {
                let len: i64 = redis::cmd("XLEN")
                    .arg(key)
                    .query_async(&mut *conn)
                    .await
                    .unwrap_or(0);
                // XRANGE returns structured entries; stringify
                // the id for preview only, ignore fields.
                let ids: Vec<Vec<redis::Value>> = redis::cmd("XRANGE")
                    .arg(key)
                    .arg("-")
                    .arg("+")
                    .arg("COUNT")
                    .arg(PREVIEW_ITEMS as i64)
                    .query_async(&mut *conn)
                    .await
                    .unwrap_or_default();
                preview_truncated = (len as usize) > ids.len();
                for entry in ids {
                    if let Some(redis::Value::BulkString(raw)) = entry.first() {
                        preview.push(String::from_utf8_lossy(raw).into_owned());
                    }
                }
                len.max(0) as u64
            }
        };

        Ok(KeyDetails {
            key: key.to_string(),
            kind: kind.as_str().to_string(),
            length,
            ttl_seconds: ttl,
            encoding,
            preview,
            preview_truncated,
        })
    }

    /// Blocking wrapper for [`Self::inspect`].
    pub fn inspect_blocking(&self, key: &str) -> Result<KeyDetails> {
        runtime::shared().block_on(self.inspect(key))
    }

    /// Run `INFO <section>` and return the server's `k: v`
    /// section body parsed into an ordered map. Pass `"server"`
    /// for version info, `"memory"` for memory, or empty for
    /// all sections (returns them all concatenated).
    pub async fn info(&self, section: &str) -> Result<BTreeMap<String, String>> {
        let mut conn = self.manager.lock().await;
        let raw: String = if section.is_empty() {
            redis::cmd("INFO").query_async(&mut *conn).await?
        } else {
            redis::cmd("INFO")
                .arg(section)
                .query_async(&mut *conn)
                .await?
        };
        Ok(parse_info(&raw))
    }

    /// Blocking wrapper for [`Self::info`].
    pub fn info_blocking(&self, section: &str) -> Result<BTreeMap<String, String>> {
        runtime::shared().block_on(self.info(section))
    }

    /// Execute an arbitrary Redis command supplied as argv
    /// tokens. The first element is the command name, the rest
    /// are passed through as bulk-string arguments.
    pub async fn execute_command(&self, args: &[String]) -> Result<CommandResult> {
        if args.is_empty() {
            return Err(RedisError::InvalidConfig("empty command".into()));
        }

        let start = Instant::now();
        let mut conn = self.manager.lock().await;
        let mut command = redis::cmd(&args[0]);
        for arg in &args[1..] {
            command.arg(arg);
        }
        let value: redis::Value = command.query_async(&mut *conn).await?;
        Ok(CommandResult {
            summary: summarize_value(&value),
            lines: render_value_lines(&value),
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Blocking wrapper for [`Self::execute_command`].
    pub fn execute_command_blocking(&self, args: &[String]) -> Result<CommandResult> {
        runtime::shared().block_on(self.execute_command(args))
    }

    /// Atomically rename `from` to `to`. Wraps `RENAMENX` so the
    /// command fails if `to` already exists — keeps rename safe
    /// against accidental clobber. The caller can fall back to
    /// the unsafe `RENAME` via `execute_command` when explicit
    /// overwrite is intended.
    pub async fn rename_nx(&self, from: &str, to: &str) -> Result<bool> {
        if from.is_empty() || to.is_empty() {
            return Err(RedisError::InvalidConfig(
                "RENAME arguments must not be empty".into(),
            ));
        }
        let mut conn = self.manager.lock().await;
        let renamed: i64 = redis::cmd("RENAMENX")
            .arg(from)
            .arg(to)
            .query_async(&mut *conn)
            .await?;
        Ok(renamed == 1)
    }

    /// Blocking wrapper for [`Self::rename_nx`].
    pub fn rename_nx_blocking(&self, from: &str, to: &str) -> Result<bool> {
        runtime::shared().block_on(self.rename_nx(from, to))
    }

    /// Delete a single key. Returns true when the key existed
    /// (the server returned `1`), false otherwise. Bulk delete
    /// stays out of scope here — the panel calls this once per
    /// confirmed selection.
    pub async fn del(&self, key: &str) -> Result<bool> {
        if key.is_empty() {
            return Err(RedisError::InvalidConfig("DEL key must not be empty".into()));
        }
        let mut conn = self.manager.lock().await;
        let removed: i64 = redis::cmd("DEL").arg(key).query_async(&mut *conn).await?;
        Ok(removed >= 1)
    }

    /// Blocking wrapper for [`Self::del`].
    pub fn del_blocking(&self, key: &str) -> Result<bool> {
        runtime::shared().block_on(self.del(key))
    }
}

fn summarize_value(value: &redis::Value) -> String {
    truncate_display(format!("{value:?}"), 120)
}

fn render_value_lines(value: &redis::Value) -> Vec<String> {
    let text = format!("{value:#?}");
    let mut lines: Vec<String> = text.lines().map(|line| line.to_string()).collect();
    if lines.is_empty() {
        lines.push(String::from("(empty reply)"));
    }
    lines
}

fn truncate_display(text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }
    let mut end = 0usize;
    for (count, (index, _)) in text.char_indices().enumerate() {
        if count == max_chars {
            break;
        }
        end = index;
    }
    let mut truncated = text[..=end].to_string();
    truncated.push('…');
    truncated
}

/// Result of a [`RedisClient::scan_keys`] call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanResult {
    /// Matched keys in lexicographic order.
    pub keys: Vec<String>,
    /// True if more keys existed than we returned (either the
    /// caller's limit or [`DEFAULT_SCAN_LIMIT`] was hit).
    pub truncated: bool,
    /// Effective limit that was applied.
    pub limit: usize,
}

/// Per-key metadata returned alongside the scan cursor — kind
/// + TTL pulled in a pipeline so the panel can show type
/// badges and remaining-life chips without a second round trip
/// per key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyEntry {
    pub key: String,
    /// Lower-case `redis-cli` type name: `string` / `hash` /
    /// `list` / `set` / `zset` / `stream` / `none` (key was
    /// deleted between SCAN and TYPE).
    pub kind: String,
    /// Seconds until expiry. `-1` for no TTL set, `-2` for the
    /// key not existing anymore (race window).
    pub ttl_seconds: i64,
}

/// Result of a single page of [`RedisClient::scan_keys_paged`]
/// — drives the panel's load-more pagination + key-list
/// enrichment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PagedScanResult {
    /// Next-page cursor. `"0"` when the scan has reached the
    /// end of the keyspace; pass back to continue otherwise.
    pub next_cursor: String,
    /// Up to one batch's worth of keys with metadata. Length is
    /// roughly `COUNT` (see [`PAGED_SCAN_COUNT`]) — the actual
    /// number Redis returned for this iteration of SCAN.
    pub keys: Vec<KeyEntry>,
    /// Round-trip time for this scan (the SCAN call + the
    /// pipelined TYPE/TTL probes). Surfaced in the panel header
    /// as a small "Nms" chip so the user can spot a slow link.
    pub rtt_ms: u64,
}

/// Parse a Redis `INFO` payload. Lines starting with `#` are
/// section headers and are dropped; blank lines and non-kv
/// lines are skipped. Values keep their trailing `\r` stripped.
fn parse_info(raw: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            out.insert(k.to_string(), v.to_string());
        }
    }
    out
}

/// Truncate `s` to at most `n` bytes without splitting a UTF-8
/// codepoint. Falls back to `&s[..n]` only when the prefix is
/// all ASCII.
fn safe_byte_prefix(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_url_omits_default_db() {
        let cfg = RedisConfig {
            host: "127.0.0.1".into(),
            port: 16379,
            db: 0,
            ..RedisConfig::default()
        };
        assert_eq!(cfg.to_url(), "redis://127.0.0.1:16379");
    }

    #[test]
    fn config_url_includes_nonzero_db() {
        let cfg = RedisConfig {
            host: "127.0.0.1".into(),
            port: 16379,
            db: 3,
            ..RedisConfig::default()
        };
        assert_eq!(cfg.to_url(), "redis://127.0.0.1:16379/3");
    }

    #[test]
    fn auth_parts_normalizes_empty_to_none() {
        let cfg = RedisConfig {
            host: "127.0.0.1".into(),
            port: 6379,
            db: 0,
            username: Some(String::new()),
            password: Some("  ".into()),
        };
        let (u, p) = cfg.auth_parts();
        assert_eq!(u, None);
        // password isn't trimmed — spaces can be legal — but empty is.
        assert_eq!(p.as_deref(), Some("  "));

        let cfg = RedisConfig {
            host: "127.0.0.1".into(),
            port: 6379,
            db: 0,
            username: None,
            password: Some(String::new()),
        };
        assert_eq!(cfg.auth_parts(), (None, None));
    }

    #[test]
    fn auth_parts_keeps_ascii_and_nonascii_bytes() {
        let cfg = RedisConfig {
            host: "127.0.0.1".into(),
            port: 6379,
            db: 0,
            username: Some("acl-user".into()),
            password: Some("p@ss:word/slash#hash".into()),
        };
        let (u, p) = cfg.auth_parts();
        assert_eq!(u.as_deref(), Some("acl-user"));
        assert_eq!(p.as_deref(), Some("p@ss:word/slash#hash"));
    }

    #[test]
    fn key_kind_parses_redis_type_strings() {
        assert_eq!(KeyKind::parse("string"), KeyKind::String);
        assert_eq!(KeyKind::parse("list"), KeyKind::List);
        assert_eq!(KeyKind::parse("set"), KeyKind::Set);
        assert_eq!(KeyKind::parse("zset"), KeyKind::ZSet);
        assert_eq!(KeyKind::parse("hash"), KeyKind::Hash);
        assert_eq!(KeyKind::parse("stream"), KeyKind::Stream);
        assert_eq!(KeyKind::parse("none"), KeyKind::None);
        assert_eq!(KeyKind::parse("anything_else"), KeyKind::None);
    }

    #[test]
    fn key_kind_round_trips_through_str() {
        for kind in [
            KeyKind::None,
            KeyKind::String,
            KeyKind::List,
            KeyKind::Set,
            KeyKind::ZSet,
            KeyKind::Hash,
            KeyKind::Stream,
        ] {
            assert_eq!(KeyKind::parse(kind.as_str()), kind);
        }
    }

    #[test]
    fn parse_info_drops_section_headers_and_blanks() {
        let raw = "# Server\r\nredis_version:7.2.4\r\nredis_mode:standalone\r\n\r\n# Clients\r\nconnected_clients:12\r\n";
        let parsed = parse_info(raw);
        assert_eq!(
            parsed.get("redis_version").map(|s| s.as_str()),
            Some("7.2.4")
        );
        assert_eq!(
            parsed.get("redis_mode").map(|s| s.as_str()),
            Some("standalone")
        );
        assert_eq!(
            parsed.get("connected_clients").map(|s| s.as_str()),
            Some("12")
        );
        assert!(!parsed.contains_key("# Server"));
    }

    #[test]
    fn parse_info_tolerates_missing_colon() {
        let raw = "ok:yes\ngarbage_no_colon\nalso_ok:1";
        let parsed = parse_info(raw);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.get("ok").map(|s| s.as_str()), Some("yes"));
    }

    #[test]
    fn safe_byte_prefix_keeps_codepoint_boundary() {
        // "é" is 2 bytes in UTF-8.
        let s = "abcé";
        // Requesting 4 bytes would cut the é in half; the
        // helper should fall back to 3 to keep ASCII only.
        assert_eq!(safe_byte_prefix(s, 4), "abc");
        assert_eq!(safe_byte_prefix(s, 5), "abcé");
        assert_eq!(safe_byte_prefix(s, 3), "abc");
    }

    #[test]
    fn safe_byte_prefix_passthrough_when_shorter() {
        assert_eq!(safe_byte_prefix("short", 100), "short");
    }

    #[test]
    fn scan_result_serializes() {
        let r = ScanResult {
            keys: vec!["a".into(), "b".into()],
            truncated: true,
            limit: 10,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"truncated\":true"));
        assert!(json.contains("\"limit\":10"));
    }

    #[test]
    fn key_details_serializes_empty_preview() {
        let d = KeyDetails {
            key: "foo".into(),
            kind: "none".into(),
            length: 0,
            ttl_seconds: -2,
            encoding: String::new(),
            preview: vec![],
            preview_truncated: false,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"ttl_seconds\":-2"));
        assert!(json.contains("\"preview\":[]"));
    }

    #[test]
    fn scan_limit_is_capped_at_default() {
        // Pure unit — connect is integration-bound, but the
        // clamp logic is pure arithmetic.
        assert_eq!(DEFAULT_SCAN_LIMIT.min(100_000_000), DEFAULT_SCAN_LIMIT);
    }
}
