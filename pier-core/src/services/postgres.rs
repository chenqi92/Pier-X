//! PostgreSQL client backend for the M7a panel.
//!
//! ## Shape vs MySQL
//!
//! Follows the exact same arc as [`super::mysql`]: an owned
//! client handle holding a single live connection, sync/async
//! method pairs through the shared runtime, and a typed
//! [`QueryResult`] matching the MySQL module's shape byte-for-
//! byte so the QML result grid can reuse the same model.
//!
//! ## Connection model
//!
//! Unlike MySQL where `mysql_async::Pool` manages a pool,
//! `tokio-postgres` gives us a raw `Client` that represents
//! **one TCP connection** and a spawned `Connection` future
//! that drives its I/O. We spawn the Connection onto the
//! shared runtime and keep the Client for queries. When the
//! client is dropped the connection future resolves and the
//! TCP socket closes.
//!
//! ## Result shape
//!
//! Same [`QueryResult`] / [`ResultRow`] / [`ColumnInfo`] types
//! as MySQL — the FFI and QML layers don't need to know which
//! backend produced the grid.
//!
//! ## Not yet
//!
//! * Streaming cursors. PG supports server-side cursors via
//!   `DECLARE CURSOR` / `FETCH`, which would let us stream
//!   huge results without loading them all into memory. M7a
//!   uses the same `MAX_ROWS` cap approach as MySQL.
//! * `\d` style table describe. The PG equivalent is
//!   `information_schema.columns` which we query directly.
//! * LISTEN/NOTIFY. That's a streaming shape and belongs in
//!   ExecStream-land, not here.

use std::collections::BTreeSet;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio_postgres::{Client, NoTls, Row};

/// Same cap as MySQL — 10k rows per query result.
pub const MAX_ROWS: usize = 10_000;
/// Same cap as MySQL — 4 KB per cell display string.
pub const MAX_CELL_BYTES: usize = 4096;

/// Errors surfaced by the PostgreSQL client.
#[derive(Debug, thiserror::Error)]
pub enum PostgresError {
    /// Underlying tokio-postgres error.
    #[error("postgres: {0}")]
    Native(#[from] tokio_postgres::Error),

    /// Caller supplied invalid config.
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

/// Result alias for PG ops.
pub type Result<T, E = PostgresError> = std::result::Result<T, E>;

/// Connection config. Mirrors [`super::mysql::MysqlConfig`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresConfig {
    /// Hostname or IP.
    pub host: String,
    /// TCP port (default 5432).
    pub port: u16,
    /// PostgreSQL user.
    pub user: String,
    /// Plaintext password.
    pub password: String,
    /// Default database. Empty = connect to the user's default.
    pub database: Option<String>,
}

/// Column metadata from `information_schema.columns`.
/// Same field names as [`super::mysql::ColumnInfo`] so the
/// QML side can bind the same roles.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnInfo {
    /// Column name.
    pub name: String,
    /// Data type, e.g. `integer`, `varchar`.
    pub column_type: String,
    /// True if the column accepts NULL.
    pub nullable: bool,
    /// Key marker — PG doesn't have MySQL's `PRI`/`UNI`/`MUL`
    /// in the same way, so this is populated from constraint
    /// info when available, or empty.
    pub key: String,
    /// Column default expression.
    pub default_value: Option<String>,
    /// Extra metadata (e.g. `nextval(...)` for serial cols).
    pub extra: String,
}

/// One row of query results. Same type as MySQL's.
pub type ResultRow = Vec<Option<String>>;

/// Full query result. Same shape as [`super::mysql::QueryResult`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryResult {
    /// Column names.
    pub columns: Vec<String>,
    /// Rows (capped at [`MAX_ROWS`]).
    pub rows: Vec<ResultRow>,
    /// True if more rows existed than we returned.
    pub truncated: bool,
    /// Affected row count for DML.
    pub affected_rows: u64,
    /// Not applicable for PG (no AUTO_INCREMENT) but kept
    /// for schema parity with MySQL's QueryResult.
    pub last_insert_id: Option<u64>,
    /// Wall-clock execution time.
    pub elapsed_ms: u64,
}

/// PostgreSQL client handle.
pub struct PostgresClient {
    client: Client,
    // The spawned Connection future's JoinHandle. We don't
    // need it for anything except keeping the task alive;
    // dropping the client makes the future resolve and the
    // handle join cleanly on the next runtime poll.
    _conn_handle: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for PostgresClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresClient").finish()
    }
}

impl PostgresClient {
    /// Connect to the configured endpoint. Performs the full
    /// TCP handshake + PG startup + auth synchronously on the
    /// shared runtime and returns an error if any step fails.
    pub async fn connect(config: PostgresConfig) -> Result<Self> {
        if config.host.is_empty() {
            return Err(PostgresError::InvalidConfig("empty host".into()));
        }
        if config.port == 0 {
            return Err(PostgresError::InvalidConfig("port must be > 0".into()));
        }
        if config.user.is_empty() {
            return Err(PostgresError::InvalidConfig("empty user".into()));
        }

        // Build a connection string. tokio-postgres supports
        // both key=value and URL forms; key=value is safer for
        // passwords that contain special chars.
        let mut params = format!(
            "host={} port={} user={}",
            config.host, config.port, config.user
        );
        if !config.password.is_empty() {
            // Escape single quotes in password for the
            // key=value format.
            let escaped = config.password.replace('\'', "\\'");
            params.push_str(&format!(" password='{escaped}'"));
        }
        if let Some(db) = config.database.as_ref().filter(|d| !d.is_empty()) {
            params.push_str(&format!(" dbname={db}"));
        }

        let (client, connection) = tokio_postgres::connect(&params, NoTls).await?;

        // Spawn the connection future onto the shared runtime.
        // Errors from the connection are logged but don't
        // propagate — the Client's next query will surface
        // the break.
        let conn_handle = crate::ssh::runtime::shared().spawn(async move {
            if let Err(e) = connection.await {
                log::warn!("postgres connection error: {e}");
            }
        });

        // Round-trip probe.
        client.simple_query("SELECT 1").await?;

        Ok(Self {
            client,
            _conn_handle: conn_handle,
        })
    }

    /// Blocking wrapper for [`Self::connect`].
    pub fn connect_blocking(config: PostgresConfig) -> Result<Self> {
        crate::ssh::runtime::shared().block_on(Self::connect(config))
    }

    /// Execute a single SQL statement.
    pub async fn execute(&self, sql: &str) -> Result<QueryResult> {
        let start = Instant::now();

        // PG's simple_query returns SimpleQueryMessage which
        // includes both row data and command-complete tags.
        // For a richer experience we use the extended protocol
        // via `query` which gives us typed Column info.
        let stmt = self.client.prepare(sql).await?;
        let columns: Vec<String> = stmt
            .columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect();

        let pg_rows: Vec<Row> = self.client.query(&stmt, &[]).await?;

        let mut rows: Vec<ResultRow> = Vec::new();
        let mut truncated = false;
        for (i, pg_row) in pg_rows.iter().enumerate() {
            if i >= MAX_ROWS {
                truncated = true;
                break;
            }
            rows.push(row_to_cells(pg_row));
        }

        Ok(QueryResult {
            columns,
            rows,
            truncated,
            affected_rows: pg_rows.len() as u64,
            last_insert_id: None,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Blocking wrapper for [`Self::execute`].
    pub fn execute_blocking(&self, sql: &str) -> Result<QueryResult> {
        crate::ssh::runtime::shared().block_on(self.execute(sql))
    }

    /// List databases, filtering internal ones.
    pub async fn list_databases(&self) -> Result<Vec<String>> {
        let rows = self
            .client
            .query("SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname", &[])
            .await?;
        let hidden: BTreeSet<&str> = ["template0", "template1"].into_iter().collect();
        Ok(rows
            .iter()
            .filter_map(|r| {
                let name: String = r.get(0);
                if hidden.contains(name.as_str()) {
                    None
                } else {
                    Some(name)
                }
            })
            .collect())
    }

    /// Blocking wrapper for [`Self::list_databases`].
    pub fn list_databases_blocking(&self) -> Result<Vec<String>> {
        crate::ssh::runtime::shared().block_on(self.list_databases())
    }

    /// List tables in the given schema (default `public`).
    pub async fn list_tables(&self, schema: &str) -> Result<Vec<String>> {
        let schema = if schema.is_empty() { "public" } else { schema };
        if !super::mysql::is_safe_ident(schema) {
            return Err(PostgresError::InvalidConfig(format!(
                "refusing unsafe schema identifier {schema:?}"
            )));
        }
        let rows = self
            .client
            .query(
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = $1 ORDER BY table_name",
                &[&schema],
            )
            .await?;
        Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
    }

    /// Blocking wrapper for [`Self::list_tables`].
    pub fn list_tables_blocking(&self, schema: &str) -> Result<Vec<String>> {
        crate::ssh::runtime::shared().block_on(self.list_tables(schema))
    }

    /// Column info from `information_schema.columns`.
    pub async fn list_columns(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let schema = if schema.is_empty() { "public" } else { schema };
        if !super::mysql::is_safe_ident(schema) {
            return Err(PostgresError::InvalidConfig(format!(
                "refusing unsafe schema identifier {schema:?}"
            )));
        }
        if !super::mysql::is_safe_ident(table) {
            return Err(PostgresError::InvalidConfig(format!(
                "refusing unsafe table identifier {table:?}"
            )));
        }
        let rows = self
            .client
            .query(
                "SELECT column_name, data_type, is_nullable, \
                        column_default, '' AS extra \
                 FROM information_schema.columns \
                 WHERE table_schema = $1 AND table_name = $2 \
                 ORDER BY ordinal_position",
                &[&schema, &table],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let name: String = r.get(0);
                let column_type: String = r.get(1);
                let nullable_str: String = r.get(2);
                let default_value: Option<String> = r.get(3);
                let extra: String = r.get(4);
                ColumnInfo {
                    name,
                    column_type,
                    nullable: nullable_str.eq_ignore_ascii_case("YES"),
                    key: String::new(), // PG constraint info is more complex
                    default_value,
                    extra,
                }
            })
            .collect())
    }

    /// Blocking wrapper for [`Self::list_columns`].
    pub fn list_columns_blocking(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        crate::ssh::runtime::shared().block_on(self.list_columns(schema, table))
    }
}

/// Convert a tokio-postgres Row into our stringified ResultRow.
fn row_to_cells(row: &Row) -> ResultRow {
    let mut out = Vec::with_capacity(row.len());
    for i in 0..row.len() {
        // tokio-postgres doesn't have a universal "get as
        // string" — we try common types in order of
        // likelihood and fall back to a Debug representation.
        let cell: Option<String> = try_get_string(row, i);
        out.push(cell);
    }
    out
}

/// Try to extract column `i` as a display string. Returns
/// `None` for SQL NULL. Falls back to Debug formatting for
/// types we don't have explicit converters for.
fn try_get_string(row: &Row, i: usize) -> Option<String> {
    // Check for NULL first via the raw bytes.
    use tokio_postgres::types::Type;
    let col_type = row.columns()[i].type_();

    // Try the most common PG types. tokio-postgres panics
    // (not errors) if you call get::<_, WrongType>, so we
    // match on the type OID before attempting the cast.
    match *col_type {
        Type::BOOL => row.get::<_, Option<bool>>(i).map(|v| v.to_string()),
        Type::INT2 => row.get::<_, Option<i16>>(i).map(|v| v.to_string()),
        Type::INT4 => row.get::<_, Option<i32>>(i).map(|v| v.to_string()),
        Type::INT8 => row.get::<_, Option<i64>>(i).map(|v| v.to_string()),
        Type::FLOAT4 => row.get::<_, Option<f32>>(i).map(|v| v.to_string()),
        Type::FLOAT8 => row.get::<_, Option<f64>>(i).map(|v| v.to_string()),
        Type::TEXT | Type::VARCHAR | Type::NAME | Type::BPCHAR => {
            row.get::<_, Option<String>>(i)
        }
        Type::BYTEA => row
            .get::<_, Option<Vec<u8>>>(i)
            .map(|v| format!("\\x{}", hex_prefix(&v))),
        _ => {
            // Fallback: try as String (works for most text-ish
            // types including numeric, uuid, json, etc.). If
            // that fails, return a placeholder.
            match row.try_get::<_, String>(i) {
                Ok(s) => Some(truncate_display(s)),
                Err(_) => Some(format!("<{}>", col_type.name())),
            }
        }
    }
    .map(truncate_display)
}

/// Hex-encode the first 16 bytes of a bytea value.
fn hex_prefix(bytes: &[u8]) -> String {
    let n = bytes.len().min(16);
    let mut out = String::with_capacity(n * 2 + 3);
    for b in &bytes[..n] {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    if bytes.len() > n {
        out.push('…');
    }
    out
}

/// Truncate display string to MAX_CELL_BYTES.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_empty_host() {
        let r = crate::ssh::runtime::shared().block_on(PostgresClient::connect(PostgresConfig {
            host: "".into(),
            port: 5432,
            user: "root".into(),
            password: "".into(),
            database: None,
        }));
        assert!(matches!(r, Err(PostgresError::InvalidConfig(_))));
    }

    #[test]
    fn config_rejects_zero_port() {
        let r = crate::ssh::runtime::shared().block_on(PostgresClient::connect(PostgresConfig {
            host: "127.0.0.1".into(),
            port: 0,
            user: "root".into(),
            password: "".into(),
            database: None,
        }));
        assert!(matches!(r, Err(PostgresError::InvalidConfig(_))));
    }

    #[test]
    fn config_rejects_empty_user() {
        let r = crate::ssh::runtime::shared().block_on(PostgresClient::connect(PostgresConfig {
            host: "127.0.0.1".into(),
            port: 5432,
            user: "".into(),
            password: "".into(),
            database: None,
        }));
        assert!(matches!(r, Err(PostgresError::InvalidConfig(_))));
    }

    #[test]
    fn query_result_round_trips_through_json() {
        let r = QueryResult {
            columns: vec!["id".into(), "name".into()],
            rows: vec![
                vec![Some("1".into()), Some("alice".into())],
                vec![Some("2".into()), None],
            ],
            truncated: false,
            affected_rows: 0,
            last_insert_id: None,
            elapsed_ms: 5,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: QueryResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.columns, r.columns);
        assert_eq!(back.rows.len(), 2);
        assert_eq!(back.rows[1][1], None);
    }

    #[test]
    fn column_info_round_trips() {
        let c = ColumnInfo {
            name: "id".into(),
            column_type: "integer".into(),
            nullable: false,
            key: String::new(),
            default_value: Some("nextval('id_seq')".into()),
            extra: String::new(),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: ColumnInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn hex_prefix_short() {
        assert_eq!(hex_prefix(&[0xde, 0xad]), "dead");
        let long: Vec<u8> = (0u8..32).collect();
        assert!(hex_prefix(&long).ends_with('…'));
    }

    #[test]
    fn truncate_display_passthrough_short() {
        assert_eq!(truncate_display("hi".into()), "hi");
    }

    #[test]
    fn truncate_display_cuts_long() {
        let long = "a".repeat(MAX_CELL_BYTES + 100);
        let t = truncate_display(long);
        assert!(t.len() <= MAX_CELL_BYTES + 4); // +4 for the …
        assert!(t.ends_with('…'));
    }
}
