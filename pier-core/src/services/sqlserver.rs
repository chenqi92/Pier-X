//! Microsoft SQL Server client backend for the unified database panel.
//!
//! ## Shape vs MySQL / PostgreSQL
//!
//! Follows the same arc as [`super::postgres`]: an owned client handle
//! over a single live connection, `async` + `_blocking` method pairs
//! through the shared runtime, and a [`QueryResult`] whose serde shape
//! matches the MySQL / PostgreSQL modules byte-for-byte so the desktop
//! result grid reuses one model.
//!
//! ## Driver
//!
//! [`tiberius`] is the pure-Rust TDS implementation. We build it without
//! its TLS features and connect with encryption **disabled** — the same
//! rationale as the other clients: traffic rides the SSH tunnel to
//! `127.0.0.1:<port>`, so the transport is already encrypted. Servers
//! that *require* TLS for login aren't reachable in this mode yet; that's
//! a follow-up (enable the `rustls` feature + `EncryptionLevel::Required`).
//!
//! ## Not yet
//!
//! * Streaming cursors — same `MAX_ROWS` cap approach as the siblings.
//! * Rich schema introspection (indexes / FKs / structure editing). v1
//!   exposes database + table listing and arbitrary T-SQL execution.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use tiberius::{AuthMethod, Client, Config, EncryptionLevel, Row};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

/// Same cap as the sibling clients — 10k rows per query result.
pub const MAX_ROWS: usize = 10_000;
/// Same cap as the sibling clients — 4 KB per cell display string.
pub const MAX_CELL_BYTES: usize = 4096;

/// Errors surfaced by the SQL Server client.
#[derive(Debug, thiserror::Error)]
pub enum SqlServerError {
    /// Underlying tiberius error.
    #[error("sqlserver: {0}")]
    Native(#[from] tiberius::error::Error),

    /// TCP / socket failure before the TDS handshake.
    #[error("io: {0}")]
    Io(String),

    /// Caller supplied invalid config.
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

/// Result alias for SQL Server ops.
pub type Result<T, E = SqlServerError> = std::result::Result<T, E>;

/// Stringified row — `None` is SQL NULL. Matches the sibling clients.
pub type ResultRow = Vec<Option<String>>;

/// Connection config. Mirrors [`super::postgres::PostgresConfig`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlServerConfig {
    /// Hostname or IP.
    pub host: String,
    /// TCP port (default 1433).
    pub port: u16,
    /// SQL Server login.
    pub user: String,
    /// Plaintext password.
    pub password: String,
    /// Initial database. Empty = the login's default database.
    pub database: Option<String>,
    /// TLS for a direct (non-tunneled) connection. Defaults to
    /// [`TlsMode::Off`](super::db_tls::TlsMode::Off) so existing tunneled
    /// connections keep encryption disabled exactly as before.
    #[serde(default)]
    pub tls_mode: super::db_tls::TlsMode,
    /// TLS server name to validate the certificate against, when it
    /// differs from [`host`](Self::host). Set when connecting through a
    /// loopback indirection (SSH tunnel / egress forwarder): `host` is
    /// then `127.0.0.1` (the TCP target) while this carries the real DB
    /// hostname tiberius validates the cert against, so `verify-full`
    /// works through a tunnel. `None`/empty → validate against `host`.
    /// Ignored when `tls_mode` is `Off`.
    #[serde(default)]
    pub tls_server_name: Option<String>,
}

/// Full query result. Same shape as [`super::postgres::QueryResult`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryResult {
    /// Column names.
    pub columns: Vec<String>,
    /// Rows (capped at [`MAX_ROWS`]).
    pub rows: Vec<ResultRow>,
    /// True if more rows existed than we returned.
    pub truncated: bool,
    /// Number of rows returned (SQL Server gives no affected-row count
    /// through `simple_query`, so this mirrors the row count like PG).
    pub affected_rows: u64,
    /// Not applicable for SQL Server — kept for schema parity.
    pub last_insert_id: Option<u64>,
    /// Wall-clock execution time.
    pub elapsed_ms: u64,
}

/// One schema-qualified table for the panel's sidebar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRef {
    /// Owning schema (e.g. `dbo`).
    pub schema: String,
    /// Table name.
    pub name: String,
}

/// Column metadata for the structure view. Same field names as
/// [`super::postgres::ColumnInfo`] so the shared grid binds identically.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnInfo {
    /// Column name.
    pub name: String,
    /// Data type, e.g. `int`, `nvarchar(50)`.
    pub column_type: String,
    /// True if the column accepts NULL.
    pub nullable: bool,
    /// `PRI` when part of the primary key, else empty.
    pub key: String,
    /// Column default expression.
    pub default_value: Option<String>,
    /// Identity / computed markers.
    pub extra: String,
}

/// Escape a value for inlining inside a T-SQL single-quoted string literal.
fn esc_lit(s: &str) -> String {
    s.replace('\'', "''")
}

/// SQL Server client handle over one live TDS connection.
pub struct SqlServerClient {
    client: Client<Compat<TcpStream>>,
}

impl std::fmt::Debug for SqlServerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqlServerClient").finish()
    }
}

impl SqlServerClient {
    /// Connect to the configured endpoint. Disables TLS — see the module
    /// docs — and performs the full TDS login synchronously on the shared
    /// runtime, returning an error if any step fails.
    pub async fn connect(config: SqlServerConfig) -> Result<Self> {
        if config.host.is_empty() {
            return Err(SqlServerError::InvalidConfig("empty host".into()));
        }
        if config.port == 0 {
            return Err(SqlServerError::InvalidConfig("port must be > 0".into()));
        }
        if config.user.is_empty() {
            return Err(SqlServerError::InvalidConfig("empty user".into()));
        }

        let mut cfg = Config::new();
        // tiberius uses `cfg.host` as the TLS validation name, but the TCP
        // target is the stream we hand it below — so when connecting through
        // a loopback tunnel we set the host to the real DB name (for
        // verify-full) while still dialing `config.host` (127.0.0.1). `Off`
        // keeps the host as-is (byte-identical legacy path).
        let tls_name = if config.tls_mode.is_off() {
            None
        } else {
            config
                .tls_server_name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };
        cfg.host(tls_name.unwrap_or(config.host.as_str()));
        cfg.port(config.port);
        cfg.authentication(AuthMethod::sql_server(&config.user, &config.password));
        if let Some(db) = config.database.as_ref().filter(|d| !d.is_empty()) {
            cfg.database(db);
        }
        // TLS is opt-in per connection. `Off` keeps encryption disabled
        // (the tunnel encrypts the transport, the historical default);
        // `Require` encrypts but trusts any cert; `VerifyFull` encrypts and
        // verifies against the OS native roots (tiberius drives rustls).
        match config.tls_mode {
            super::db_tls::TlsMode::Off => {
                cfg.encryption(EncryptionLevel::NotSupported);
                cfg.trust_cert();
            }
            super::db_tls::TlsMode::Require => {
                cfg.encryption(EncryptionLevel::Required);
                cfg.trust_cert();
            }
            super::db_tls::TlsMode::VerifyFull => {
                cfg.encryption(EncryptionLevel::Required);
                // No trust_cert() → tiberius verifies the server certificate.
            }
        }

        // Always dial the configured target (loopback when tunneled),
        // independent of the TLS validation name set on `cfg` above —
        // `cfg.get_addr()` would otherwise resolve the TLS name and bypass
        // the tunnel.
        let tcp = TcpStream::connect((config.host.as_str(), config.port))
            .await
            .map_err(|e| SqlServerError::Io(e.to_string()))?;
        tcp.set_nodelay(true).ok();

        let client = Client::connect(cfg, tcp.compat_write()).await?;
        Ok(Self { client })
    }

    /// Blocking wrapper for [`Self::connect`].
    pub fn connect_blocking(config: SqlServerConfig) -> Result<Self> {
        crate::ssh::runtime::shared().block_on(Self::connect(config))
    }

    /// Execute a single T-SQL batch and return the first result set.
    pub async fn execute(&mut self, sql: &str) -> Result<QueryResult> {
        let start = Instant::now();
        let results = self.client.simple_query(sql).await?.into_results().await?;
        let first = results.into_iter().next().unwrap_or_default();

        let columns: Vec<String> = first
            .first()
            .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
            .unwrap_or_default();

        let mut rows: Vec<ResultRow> = Vec::new();
        let mut truncated = false;
        for row in &first {
            if rows.len() >= MAX_ROWS {
                truncated = true;
                break;
            }
            let n = row.columns().len();
            let cells: ResultRow = (0..n).map(|i| cell_to_string(row, i)).collect();
            rows.push(cells);
        }

        let affected_rows = rows.len() as u64;
        Ok(QueryResult {
            columns,
            rows,
            truncated,
            affected_rows,
            last_insert_id: None,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Blocking wrapper for [`Self::execute`].
    pub fn execute_blocking(&mut self, sql: &str) -> Result<QueryResult> {
        crate::ssh::runtime::shared().block_on(self.execute(sql))
    }

    /// User databases (skips the four system databases, id 1-4).
    pub async fn list_databases(&mut self) -> Result<Vec<String>> {
        let rows = self
            .client
            .simple_query("SELECT name FROM sys.databases WHERE database_id > 4 ORDER BY name")
            .await?
            .into_first_result()
            .await?;
        Ok(rows
            .iter()
            .filter_map(|r| r.try_get::<&str, _>(0).ok().flatten().map(str::to_string))
            .collect())
    }

    /// Blocking wrapper for [`Self::list_databases`].
    pub fn list_databases_blocking(&mut self) -> Result<Vec<String>> {
        crate::ssh::runtime::shared().block_on(self.list_databases())
    }

    /// Name of the database the connection is currently using.
    pub async fn current_database(&mut self) -> Result<String> {
        let rows = self
            .client
            .simple_query("SELECT DB_NAME()")
            .await?
            .into_first_result()
            .await?;
        Ok(rows
            .first()
            .and_then(|r| r.try_get::<&str, _>(0).ok().flatten())
            .unwrap_or_default()
            .to_string())
    }

    /// Base tables in the current database, schema-qualified.
    pub async fn list_tables(&mut self) -> Result<Vec<TableRef>> {
        let rows = self
            .client
            .simple_query(
                "SELECT TABLE_SCHEMA, TABLE_NAME FROM INFORMATION_SCHEMA.TABLES \
                 WHERE TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_SCHEMA, TABLE_NAME",
            )
            .await?
            .into_first_result()
            .await?;
        Ok(rows
            .iter()
            .map(|r| TableRef {
                schema: r
                    .try_get::<&str, _>(0)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .to_string(),
                name: r
                    .try_get::<&str, _>(1)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .to_string(),
            })
            .collect())
    }

    /// Blocking wrapper for [`Self::list_tables`].
    pub fn list_tables_blocking(&mut self) -> Result<Vec<TableRef>> {
        crate::ssh::runtime::shared().block_on(self.list_tables())
    }

    /// Blocking wrapper for [`Self::current_database`].
    pub fn current_database_blocking(&mut self) -> Result<String> {
        crate::ssh::runtime::shared().block_on(self.current_database())
    }

    /// User schemas in the active database (dbo-owned; excludes the
    /// fixed role schemas like `db_datareader`).
    pub async fn list_schemas(&mut self) -> Result<Vec<String>> {
        let rows = self
            .client
            .simple_query("SELECT name FROM sys.schemas WHERE principal_id = 1 ORDER BY name")
            .await?
            .into_first_result()
            .await?;
        Ok(rows
            .iter()
            .filter_map(|r| r.try_get::<&str, _>(0).ok().flatten().map(str::to_string))
            .collect())
    }

    /// Blocking wrapper for [`Self::list_schemas`].
    pub fn list_schemas_blocking(&mut self) -> Result<Vec<String>> {
        crate::ssh::runtime::shared().block_on(self.list_schemas())
    }

    /// Column metadata for one table, with primary-key columns marked
    /// `PRI`. Fail-softs the PK lookup to empty so a low-privilege role
    /// still gets the column list.
    pub async fn list_columns(&mut self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let (s, t) = (esc_lit(schema), esc_lit(table));

        // Primary-key columns first (best-effort).
        let pk_sql = format!(
            "SELECT ku.COLUMN_NAME FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc \
             JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE ku \
               ON tc.CONSTRAINT_NAME = ku.CONSTRAINT_NAME \
              AND tc.TABLE_SCHEMA = ku.TABLE_SCHEMA \
             WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY' \
               AND tc.TABLE_SCHEMA = '{s}' AND tc.TABLE_NAME = '{t}'"
        );
        let pk: std::collections::BTreeSet<String> = match self.client.simple_query(pk_sql).await {
            Ok(stream) => stream
                .into_first_result()
                .await
                .unwrap_or_default()
                .iter()
                .filter_map(|r| r.try_get::<&str, _>(0).ok().flatten().map(str::to_string))
                .collect(),
            Err(_) => std::collections::BTreeSet::new(),
        };

        let col_sql = format!(
            "SELECT COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH, IS_NULLABLE, COLUMN_DEFAULT, \
                    COLUMNPROPERTY(OBJECT_ID(QUOTENAME(TABLE_SCHEMA) + '.' + QUOTENAME(TABLE_NAME)), COLUMN_NAME, 'IsIdentity') AS is_identity \
             FROM INFORMATION_SCHEMA.COLUMNS \
             WHERE TABLE_SCHEMA = '{s}' AND TABLE_NAME = '{t}' \
             ORDER BY ORDINAL_POSITION"
        );
        let rows = self
            .client
            .simple_query(col_sql)
            .await?
            .into_first_result()
            .await?;

        Ok(rows
            .iter()
            .map(|r| {
                let name = r
                    .try_get::<&str, _>(0)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .to_string();
                let base_type = r.try_get::<&str, _>(1).ok().flatten().unwrap_or_default();
                let max_len = r.try_get::<i32, _>(2).ok().flatten();
                let column_type = match max_len {
                    Some(-1) => format!("{base_type}(max)"),
                    Some(n) if n > 0 => format!("{base_type}({n})"),
                    _ => base_type.to_string(),
                };
                let nullable = r
                    .try_get::<&str, _>(3)
                    .ok()
                    .flatten()
                    .map(|v| v.eq_ignore_ascii_case("YES"))
                    .unwrap_or(true);
                let default_value = r.try_get::<&str, _>(4).ok().flatten().map(str::to_string);
                let is_identity = r.try_get::<i32, _>(5).ok().flatten().unwrap_or(0) == 1;
                ColumnInfo {
                    key: if pk.contains(&name) {
                        "PRI".to_string()
                    } else {
                        String::new()
                    },
                    name,
                    column_type,
                    nullable,
                    default_value,
                    extra: if is_identity {
                        "IDENTITY".to_string()
                    } else {
                        String::new()
                    },
                }
            })
            .collect())
    }

    /// Blocking wrapper for [`Self::list_columns`].
    pub fn list_columns_blocking(&mut self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        crate::ssh::runtime::shared().block_on(self.list_columns(schema, table))
    }
}

/// Stringify column `i` of a tiberius row. Returns `None` for SQL NULL.
/// Tries the common SQL Server types in order; an unconvertible type
/// yields a placeholder rather than panicking.
fn cell_to_string(row: &Row, i: usize) -> Option<String> {
    if let Ok(v) = row.try_get::<&str, _>(i) {
        return v.map(|s| truncate_display(s.to_string()));
    }
    if let Ok(v) = row.try_get::<i32, _>(i) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<i64, _>(i) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<i16, _>(i) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<u8, _>(i) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<f64, _>(i) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<f32, _>(i) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<bool, _>(i) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<rust_decimal::Decimal, _>(i) {
        return v.map(|x| truncate_display(x.to_string()));
    }
    if let Ok(v) = row.try_get::<chrono::NaiveDateTime, _>(i) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<chrono::NaiveDate, _>(i) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<chrono::NaiveTime, _>(i) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<&[u8], _>(i) {
        return v.map(|b| format!("0x{}", hex_prefix(b)));
    }
    Some("<unsupported>".to_string())
}

/// Hex-encode the first 16 bytes of a binary value.
fn hex_prefix(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let n = bytes.len().min(16);
    let mut out = String::with_capacity(n * 2 + 3);
    for b in &bytes[..n] {
        let _ = write!(out, "{b:02x}");
    }
    if bytes.len() > n {
        out.push('…');
    }
    out
}

/// Truncate a display string to [`MAX_CELL_BYTES`] on a char boundary.
fn truncate_display(s: String) -> String {
    if s.len() <= MAX_CELL_BYTES {
        return s;
    }
    let mut end = MAX_CELL_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push('…');
    out
}
