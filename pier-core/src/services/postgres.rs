//! PostgreSQL client backend for the M7a panel.
//!
//! ## Shape vs MySQL
//!
//! Follows the exact same arc as [`super::mysql`]: an owned
//! client handle holding a single live connection, sync/async
//! method pairs through the shared runtime, and a typed
//! [`QueryResult`] matching the MySQL module's shape byte-for-
//! byte so the desktop result grid can reuse the same model.
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
//! as MySQL so higher app layers don't need backend-specific
//! result models.
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

/// One user-defined enum type in the active schema. Used by the
/// data grid to render a `<datalist>` whenever a column's resolved
/// type (`format_type`) matches an enum name — gives the user a
/// dropdown of valid values when editing an enum cell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumType {
    /// Enum name as it appears in `format_type` (e.g. `mood`).
    pub name: String,
    /// Values declared on the enum, in catalog order.
    pub values: Vec<String>,
}

/// Column metadata from `information_schema.columns`.
/// Same field names as [`super::mysql::ColumnInfo`] so shared
/// UI/runtime code can bind the same roles.
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
    /// Column comment via `col_description(attrelid, attnum)`. Empty
    /// when the column has no comment — same shape as MySQL.
    pub comment: String,
}

/// Schema-tree enrichment for one table — mirror of
/// [`super::mysql::TableSummary`] adapted to PostgreSQL semantics.
/// `engine` is always `None` for PG (no per-table storage engine);
/// `updated_at` is also `None` because PG doesn't track per-table
/// last-update at the catalog level the way MySQL does. We keep
/// the same field shape for cross-engine UI sharing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableSummary {
    /// Table name (relname).
    pub name: String,
    /// Row count estimate from `pg_class.reltuples`.
    pub row_count: Option<u64>,
    /// Data segment size in bytes (`pg_relation_size`).
    pub data_bytes: Option<u64>,
    /// Total index size in bytes (`pg_indexes_size`).
    pub index_bytes: Option<u64>,
    /// Always `None` for PostgreSQL — kept for cross-engine shape parity.
    pub engine: Option<String>,
    /// Always `None` for PostgreSQL — PG has no per-table update timestamp.
    pub updated_at: Option<String>,
    /// Table comment via `obj_description(c.oid, 'pg_class')`. Empty
    /// when the table has no comment — same shape as MySQL.
    pub comment: String,
}

/// One row in the routines folder. `kind` is the upper-cased
/// `routine_type` from `information_schema.routines` — usually
/// `"FUNCTION"` or `"PROCEDURE"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutineSummary {
    /// Routine name from `information_schema.routines.routine_name`.
    pub name: String,
    /// Upper-cased `routine_type` — usually `"FUNCTION"` or `"PROCEDURE"`.
    pub kind: String,
}

/// One index defined on a table — same shape as
/// [`super::mysql::IndexSummary`]. PG's `kind` comes from the
/// access-method name (`btree`, `hash`, `gin`, `gist`, …).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexSummary {
    /// Index name (relname of the pg_index row).
    pub name: String,
    /// Indexed columns / expressions in declaration order.
    pub columns: Vec<String>,
    /// True for UNIQUE / PRIMARY KEY indexes.
    pub unique: bool,
    /// Access method name (`btree`, `hash`, `gin`, `gist`, `brin`, …).
    pub kind: String,
}

/// One foreign-key constraint — same shape as
/// [`super::mysql::ForeignKey`]. PG stores referential actions
/// as single chars; we expand to the same `NO ACTION` /
/// `RESTRICT` / `CASCADE` / `SET NULL` / `SET DEFAULT` spelling
/// MySQL uses so the panel can render both engines uniformly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKey {
    /// Constraint name (`conname`).
    pub name: String,
    /// Local columns participating in the FK, in declaration order.
    pub columns: Vec<String>,
    /// Schema of the referenced table.
    pub ref_schema: String,
    /// Referenced table name.
    pub ref_table: String,
    /// Referenced columns, paired by index with `columns`.
    pub ref_columns: Vec<String>,
    /// `ON UPDATE` action expanded to `NO ACTION` / `RESTRICT` /
    /// `CASCADE` / `SET NULL` / `SET DEFAULT`.
    pub on_update: String,
    /// `ON DELETE` action, same spelling as `on_update`.
    pub on_delete: String,
}

/// One row of query results. Same type as MySQL's.
pub type ResultRow = Vec<Option<String>>;

/// One row of `pg_stat_activity`. We surface the columns most useful
/// for a "what's running right now" panel and skip the rest (xid, etc.)
/// which are mostly internal-debug. Optional fields mirror PG's NULLs:
/// idle backends won't have a `query_duration_ms`, autovacuum workers
/// won't have a `usename`, etc.
#[allow(missing_docs)]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PgActivityRow {
    pub pid: i32,
    pub usename: Option<String>,
    pub datname: Option<String>,
    pub client_addr: Option<String>,
    pub application_name: Option<String>,
    pub state: Option<String>,
    /// Time spent on the current statement, in milliseconds.
    pub query_duration_ms: Option<i64>,
    /// Time since the last `state` transition, in milliseconds. For
    /// idle-in-transaction sessions this is how long the txn has been
    /// holding locks.
    pub state_duration_ms: Option<i64>,
    pub wait_event_type: Option<String>,
    pub wait_event: Option<String>,
    pub query: Option<String>,
}

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
            .query(
                "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
                &[],
            )
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

    /// User-visible schemas in the active database. Filters out
    /// `pg_catalog`, `information_schema`, and the per-database
    /// `pg_toast*` / `pg_temp_*` schemas — the panel never wants
    /// the user to land in those by accident. Returns names in
    /// alphabetical order.
    pub async fn list_schemas(&self) -> Result<Vec<String>> {
        let rows = self
            .client
            .query(
                "SELECT schema_name FROM information_schema.schemata \
                 WHERE schema_name NOT IN ('pg_catalog', 'information_schema') \
                   AND schema_name NOT LIKE 'pg_toast%' \
                   AND schema_name NOT LIKE 'pg_temp_%' \
                 ORDER BY schema_name",
                &[],
            )
            .await?;
        Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
    }

    /// Blocking wrapper for [`Self::list_schemas`].
    pub fn list_schemas_blocking(&self) -> Result<Vec<String>> {
        crate::ssh::runtime::shared().block_on(self.list_schemas())
    }

    /// Snapshot of `pg_stat_activity` filtered to the current
    /// database. Returns `(active, total)` — `active` is rows with
    /// `state = 'active'`, `total` is the row count regardless of
    /// state. Returns `(0, 0)` on permission errors so the panel's
    /// chip silently hides instead of shouting.
    pub async fn pool_status(&self) -> Result<(u32, u32)> {
        // Fall back gracefully: a low-privilege role may not be
        // allowed to read other backends. The query uses
        // `current_database()` so non-superusers see only their
        // own DB's rows.
        let row = match self
            .client
            .query_one(
                "SELECT \
                   COUNT(*)::int4 AS total, \
                   COUNT(*) FILTER (WHERE state = 'active')::int4 AS active \
                 FROM pg_stat_activity \
                 WHERE datname = current_database()",
                &[],
            )
            .await
        {
            Ok(r) => r,
            Err(_) => return Ok((0, 0)),
        };
        let total: i32 = row.get("total");
        let active: i32 = row.get("active");
        Ok((active.max(0) as u32, total.max(0) as u32))
    }

    /// Blocking wrapper for [`Self::pool_status`].
    pub fn pool_status_blocking(&self) -> Result<(u32, u32)> {
        crate::ssh::runtime::shared().block_on(self.pool_status())
    }

    /// Fetch a snapshot of `pg_stat_activity` filtered to non-self,
    /// non-idle rows the caller can see. The connecting role's RBAC
    /// determines what shows up — superusers see everything, low-priv
    /// roles see only their own backends. Sorted longest-running first
    /// so the slow-query view lands on the worst offender immediately.
    pub async fn list_activity(&self) -> Result<Vec<PgActivityRow>> {
        // EXTRACT(EPOCH FROM …) returns f64 seconds; cast to bigint
        // milliseconds in SQL so the driver hands us a plain i64 and
        // we don't carry float-precision noise across the wire.
        let sql = "SELECT \
                pid, \
                usename::text AS usename, \
                datname::text AS datname, \
                COALESCE(host(client_addr), client_hostname)::text AS client_addr, \
                application_name::text AS application_name, \
                state::text AS state, \
                CASE WHEN query_start IS NULL THEN NULL \
                     ELSE (EXTRACT(EPOCH FROM (now() - query_start)) * 1000)::bigint \
                END AS query_duration_ms, \
                CASE WHEN state_change IS NULL THEN NULL \
                     ELSE (EXTRACT(EPOCH FROM (now() - state_change)) * 1000)::bigint \
                END AS state_duration_ms, \
                wait_event_type::text AS wait_event_type, \
                wait_event::text AS wait_event, \
                query::text AS query \
             FROM pg_stat_activity \
             WHERE pid <> pg_backend_pid() \
               AND backend_type = 'client backend' \
             ORDER BY query_start NULLS LAST, pid";
        let pg_rows = self.client.query(sql, &[]).await?;
        let mut out = Vec::with_capacity(pg_rows.len());
        for r in pg_rows {
            out.push(PgActivityRow {
                pid: r.get::<_, i32>("pid"),
                usename: r.try_get::<_, Option<String>>("usename").unwrap_or(None),
                datname: r.try_get::<_, Option<String>>("datname").unwrap_or(None),
                client_addr: r
                    .try_get::<_, Option<String>>("client_addr")
                    .unwrap_or(None),
                application_name: r
                    .try_get::<_, Option<String>>("application_name")
                    .unwrap_or(None),
                state: r.try_get::<_, Option<String>>("state").unwrap_or(None),
                query_duration_ms: r
                    .try_get::<_, Option<i64>>("query_duration_ms")
                    .unwrap_or(None),
                state_duration_ms: r
                    .try_get::<_, Option<i64>>("state_duration_ms")
                    .unwrap_or(None),
                wait_event_type: r
                    .try_get::<_, Option<String>>("wait_event_type")
                    .unwrap_or(None),
                wait_event: r
                    .try_get::<_, Option<String>>("wait_event")
                    .unwrap_or(None),
                query: r.try_get::<_, Option<String>>("query").unwrap_or(None),
            });
        }
        Ok(out)
    }

    /// Blocking wrapper for [`Self::list_activity`].
    pub fn list_activity_blocking(&self) -> Result<Vec<PgActivityRow>> {
        crate::ssh::runtime::shared().block_on(self.list_activity())
    }

    /// `pg_cancel_backend(pid)` — politely asks PG to abort the running
    /// query on `pid` (sends SIGINT). The connection itself stays open;
    /// only the in-flight statement is interrupted. Returns the boolean
    /// result that PG hands back (`true` if the signal was sent).
    pub async fn cancel_query(&self, pid: i32) -> Result<bool> {
        let row = self
            .client
            .query_one("SELECT pg_cancel_backend($1) AS ok", &[&pid])
            .await?;
        Ok(row.try_get::<_, bool>("ok").unwrap_or(false))
    }

    /// Blocking wrapper for [`Self::cancel_query`].
    pub fn cancel_query_blocking(&self, pid: i32) -> Result<bool> {
        crate::ssh::runtime::shared().block_on(self.cancel_query(pid))
    }

    /// `pg_terminate_backend(pid)` — forcefully closes the backend's
    /// connection (sends SIGTERM). Heavier hammer than [`Self::cancel_query`];
    /// callers should prefer cancel first and only escalate if the query
    /// is genuinely stuck.
    pub async fn terminate_backend(&self, pid: i32) -> Result<bool> {
        let row = self
            .client
            .query_one("SELECT pg_terminate_backend($1) AS ok", &[&pid])
            .await?;
        Ok(row.try_get::<_, bool>("ok").unwrap_or(false))
    }

    /// Blocking wrapper for [`Self::terminate_backend`].
    pub fn terminate_backend_blocking(&self, pid: i32) -> Result<bool> {
        crate::ssh::runtime::shared().block_on(self.terminate_backend(pid))
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
        // We pull `col_description` via the catalog OID to reach the
        // attribute comment. `information_schema.columns` doesn't
        // expose comments directly, hence the join through pg_attribute.
        // `format_type(atttypid, atttypmod)` is the catalog's own
        // pretty-printer: it expands `_int4` → `integer[]`, prints
        // `varchar(255)` from atttypmod, distinguishes `numeric(10,2)`
        // from a bare `numeric`, and surfaces user-defined enum / domain
        // names verbatim. We fall back to `c.data_type` if the format
        // call ever returns an empty string (won't normally happen, but
        // a robust fallback keeps the panel useful on weird catalogs).
        let rows = self
            .client
            .query(
                "SELECT c.column_name, \
                        COALESCE(NULLIF(pg_catalog.format_type(a.atttypid, a.atttypmod), ''), c.data_type) AS pretty_type, \
                        c.is_nullable, \
                        c.column_default, '' AS extra, \
                        COALESCE(pg_catalog.col_description(pgc.oid, a.attnum), '') AS comment \
                 FROM information_schema.columns c \
                 JOIN pg_catalog.pg_class pgc \
                   ON pgc.relname = c.table_name \
                 JOIN pg_catalog.pg_namespace pgn \
                   ON pgn.oid = pgc.relnamespace AND pgn.nspname = c.table_schema \
                 JOIN pg_catalog.pg_attribute a \
                   ON a.attrelid = pgc.oid AND a.attname = c.column_name \
                 WHERE c.table_schema = $1 AND c.table_name = $2 \
                 ORDER BY c.ordinal_position",
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
                let comment: String = r.get(5);
                ColumnInfo {
                    name,
                    column_type,
                    nullable: nullable_str.eq_ignore_ascii_case("YES"),
                    key: String::new(), // PG constraint info is more complex
                    default_value,
                    extra,
                    comment,
                }
            })
            .collect())
    }

    /// Blocking wrapper for [`Self::list_columns`].
    pub fn list_columns_blocking(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        crate::ssh::runtime::shared().block_on(self.list_columns(schema, table))
    }

    /// Enriched table list — same set of tables as
    /// [`Self::list_tables`] but joined with `pg_class` for the
    /// row-count estimate (`reltuples`) and `pg_total_relation_size`
    /// / `pg_indexes_size` for on-disk sizing. We expose the same
    /// `TableSummary` shape as MySQL so the panel can render
    /// consistent badges + tooltips across engines.
    ///
    /// `reltuples` is a planner statistic, not exact — that's
    /// already how MySQL's `information_schema.tables.table_rows`
    /// behaves, so the field semantics line up.
    pub async fn list_tables_meta(&self, schema: &str) -> Result<Vec<TableSummary>> {
        let schema = if schema.is_empty() { "public" } else { schema };
        if !super::mysql::is_safe_ident(schema) {
            return Err(PostgresError::InvalidConfig(format!(
                "refusing unsafe schema identifier {schema:?}"
            )));
        }
        let rows = self
            .client
            .query(
                "SELECT c.relname,
                        c.reltuples::bigint,
                        pg_relation_size(c.oid),
                        pg_indexes_size(c.oid),
                        COALESCE(obj_description(c.oid, 'pg_class'), '') AS comment
                 FROM pg_class c
                 JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname = $1 AND c.relkind = 'r'
                 ORDER BY c.relname",
                &[&schema],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let name: String = r.get(0);
                let row_i: Option<i64> = r.try_get(1).ok();
                let data_i: Option<i64> = r.try_get(2).ok();
                let idx_i: Option<i64> = r.try_get(3).ok();
                let comment: String = r.try_get(4).unwrap_or_default();
                TableSummary {
                    name,
                    row_count: row_i.and_then(|n| if n < 0 { None } else { Some(n as u64) }),
                    data_bytes: data_i.and_then(|n| if n < 0 { None } else { Some(n as u64) }),
                    index_bytes: idx_i.and_then(|n| if n < 0 { None } else { Some(n as u64) }),
                    engine: None,
                    updated_at: None,
                    comment,
                }
            })
            .collect())
    }

    /// Blocking wrapper for [`Self::list_tables_meta`].
    pub fn list_tables_meta_blocking(&self, schema: &str) -> Result<Vec<TableSummary>> {
        crate::ssh::runtime::shared().block_on(self.list_tables_meta(schema))
    }

    /// View names in `schema`. Pulled from `information_schema.views`.
    pub async fn list_views(&self, schema: &str) -> Result<Vec<String>> {
        let schema = if schema.is_empty() { "public" } else { schema };
        if !super::mysql::is_safe_ident(schema) {
            return Err(PostgresError::InvalidConfig(format!(
                "refusing unsafe schema identifier {schema:?}"
            )));
        }
        let rows = self
            .client
            .query(
                "SELECT table_name FROM information_schema.views \
                 WHERE table_schema = $1 ORDER BY table_name",
                &[&schema],
            )
            .await?;
        Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
    }

    /// Blocking wrapper for [`Self::list_views`].
    pub fn list_views_blocking(&self, schema: &str) -> Result<Vec<String>> {
        crate::ssh::runtime::shared().block_on(self.list_views(schema))
    }

    /// Stored functions + procedures in `schema`. PG separates
    /// `FUNCTION` and `PROCEDURE` in `routine_type`; both go in
    /// the same folder so the tree behaviour matches MySQL.
    pub async fn list_routines(&self, schema: &str) -> Result<Vec<RoutineSummary>> {
        let schema = if schema.is_empty() { "public" } else { schema };
        if !super::mysql::is_safe_ident(schema) {
            return Err(PostgresError::InvalidConfig(format!(
                "refusing unsafe schema identifier {schema:?}"
            )));
        }
        let rows = self
            .client
            .query(
                "SELECT routine_name, routine_type \
                 FROM information_schema.routines \
                 WHERE routine_schema = $1 \
                 ORDER BY routine_name",
                &[&schema],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| RoutineSummary {
                name: r.get::<_, String>(0),
                kind: r.get::<_, String>(1),
            })
            .collect())
    }

    /// Blocking wrapper for [`Self::list_routines`].
    pub fn list_routines_blocking(&self, schema: &str) -> Result<Vec<RoutineSummary>> {
        crate::ssh::runtime::shared().block_on(self.list_routines(schema))
    }

    /// User-defined enum types in `schema`. Walks `pg_type` joined
    /// with `pg_enum` so each enum gets one row per declared value.
    /// Values are returned in catalog order (`enumsortorder`) which
    /// is also the declaration order — what users expect to see in
    /// a dropdown.
    pub async fn list_enums(&self, schema: &str) -> Result<Vec<EnumType>> {
        let schema = if schema.is_empty() { "public" } else { schema };
        if !super::mysql::is_safe_ident(schema) {
            return Err(PostgresError::InvalidConfig(format!(
                "refusing unsafe schema identifier {schema:?}"
            )));
        }
        let rows = self
            .client
            .query(
                "SELECT t.typname, e.enumlabel \
                 FROM pg_catalog.pg_type t \
                 JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
                 JOIN pg_catalog.pg_enum e ON e.enumtypid = t.oid \
                 WHERE n.nspname = $1 \
                 ORDER BY t.typname, e.enumsortorder",
                &[&schema],
            )
            .await?;

        let mut out: Vec<EnumType> = Vec::new();
        for r in rows.iter() {
            let name: String = r.get(0);
            let label: String = r.get(1);
            match out.last_mut() {
                Some(last) if last.name == name => last.values.push(label),
                _ => out.push(EnumType {
                    name,
                    values: vec![label],
                }),
            }
        }
        Ok(out)
    }

    /// Blocking wrapper for [`Self::list_enums`].
    pub fn list_enums_blocking(&self, schema: &str) -> Result<Vec<EnumType>> {
        crate::ssh::runtime::shared().block_on(self.list_enums(schema))
    }

    /// All indexes on `<schema>.<table>`. Walks `pg_index` joined
    /// with `pg_class` (index + table relations), `pg_namespace`
    /// (schema), `pg_attribute` (column names), and `pg_am` for
    /// the access method.
    pub async fn list_indexes(&self, schema: &str, table: &str) -> Result<Vec<IndexSummary>> {
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
                "SELECT i.relname AS index_name,
                        a.attname AS column_name,
                        ix.indisunique AS is_unique,
                        am.amname AS kind,
                        ord
                   FROM pg_index ix
                   JOIN pg_class i ON i.oid = ix.indexrelid
                   JOIN pg_class t ON t.oid = ix.indrelid
                   JOIN pg_namespace n ON n.oid = t.relnamespace
                   JOIN pg_am am ON am.oid = i.relam,
                        unnest(ix.indkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = u.attnum
                  WHERE n.nspname = $1 AND t.relname = $2
                  ORDER BY i.relname, ord",
                &[&schema, &table],
            )
            .await?;
        let mut out: Vec<IndexSummary> = Vec::new();
        for r in rows {
            let name: String = r.get(0);
            let column: String = r.get(1);
            let unique: bool = r.get(2);
            let kind: String = r.get(3);
            match out.last_mut() {
                Some(last) if last.name == name => {
                    last.columns.push(column);
                }
                _ => {
                    out.push(IndexSummary {
                        name,
                        columns: vec![column],
                        unique,
                        kind,
                    });
                }
            }
        }
        Ok(out)
    }

    /// Blocking wrapper for [`Self::list_indexes`].
    pub fn list_indexes_blocking(&self, schema: &str, table: &str) -> Result<Vec<IndexSummary>> {
        crate::ssh::runtime::shared().block_on(self.list_indexes(schema, table))
    }

    /// All foreign keys outgoing from `<schema>.<table>`. PG
    /// stores FKs in `pg_constraint` with `contype = 'f'`; column
    /// indexes live in `conkey` (local) and `confkey` (foreign).
    pub async fn list_foreign_keys(&self, schema: &str, table: &str) -> Result<Vec<ForeignKey>> {
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
                "SELECT con.conname AS name,
                        att.attname AS column_name,
                        fns.nspname AS ref_schema,
                        fcl.relname AS ref_table,
                        fatt.attname AS ref_column,
                        con.confupdtype AS upd,
                        con.confdeltype AS del,
                        ord
                   FROM pg_constraint con
                   JOIN pg_class cl ON cl.oid = con.conrelid
                   JOIN pg_namespace ns ON ns.oid = cl.relnamespace
                   JOIN pg_class fcl ON fcl.oid = con.confrelid
                   JOIN pg_namespace fns ON fns.oid = fcl.relnamespace,
                        unnest(con.conkey, con.confkey) WITH ORDINALITY AS u(localcol, refcol, ord)
                   JOIN pg_attribute att ON att.attrelid = cl.oid AND att.attnum = u.localcol
                   JOIN pg_attribute fatt ON fatt.attrelid = fcl.oid AND fatt.attnum = u.refcol
                  WHERE ns.nspname = $1 AND cl.relname = $2 AND con.contype = 'f'
                  ORDER BY con.conname, ord",
                &[&schema, &table],
            )
            .await?;
        let mut out: Vec<ForeignKey> = Vec::new();
        for r in rows {
            let name: String = r.get(0);
            let column: String = r.get(1);
            let ref_schema: String = r.get(2);
            let ref_table: String = r.get(3);
            let ref_column: String = r.get(4);
            let upd_byte: i8 = r.try_get(5).unwrap_or(b'a' as i8);
            let del_byte: i8 = r.try_get(6).unwrap_or(b'a' as i8);
            let on_update = map_action(upd_byte as u8 as char);
            let on_delete = map_action(del_byte as u8 as char);
            match out.last_mut() {
                Some(last) if last.name == name => {
                    last.columns.push(column);
                    last.ref_columns.push(ref_column);
                }
                _ => {
                    out.push(ForeignKey {
                        name,
                        columns: vec![column],
                        ref_schema,
                        ref_table,
                        ref_columns: vec![ref_column],
                        on_update,
                        on_delete,
                    });
                }
            }
        }
        Ok(out)
    }

    /// Blocking wrapper for [`Self::list_foreign_keys`].
    pub fn list_foreign_keys_blocking(&self, schema: &str, table: &str) -> Result<Vec<ForeignKey>> {
        crate::ssh::runtime::shared().block_on(self.list_foreign_keys(schema, table))
    }
}

/// Expand the single-char `confupdtype` / `confdeltype` field
/// from `pg_constraint` to the same spelling MySQL emits via
/// `referential_constraints`. Lets the panel render both engines
/// with one set of strings.
fn map_action(c: char) -> String {
    match c {
        'a' => "NO ACTION".to_string(),
        'r' => "RESTRICT".to_string(),
        'c' => "CASCADE".to_string(),
        'n' => "SET NULL".to_string(),
        'd' => "SET DEFAULT".to_string(),
        // Anything else (catalog corruption, future PG version
        // adding a new code) stays empty so the panel renders a
        // dash — better than guessing.
        _ => String::new(),
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
        Type::TEXT | Type::VARCHAR | Type::NAME | Type::BPCHAR => row.get::<_, Option<String>>(i),
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
            comment: String::new(),
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
