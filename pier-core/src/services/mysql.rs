//! MySQL client backend for the M5d panel.
//!
//! ## Shape
//!
//! Follows the same arc as [`super::redis`]: an owned client
//! handle that wraps a single long-lived
//! [`mysql_async::Pool`], synchronous/async method pairs that
//! route through [`crate::ssh::runtime::shared`], and a
//! typed [`QueryResult`] that the FFI turns into JSON for
//! the UI.
//!
//! ## Connection model
//!
//! A [`MysqlClient`] holds a pool with capacity 1 — we model
//! "one panel = one live connection" rather than contending
//! for multiplexed sessions. A future M5d+ iteration can
//! widen this when we add a concurrent query runner, but for
//! M5d the single-connection model matches how every SQL GUI
//! behaves (one tab, one backend session).
//!
//! ## Result shape
//!
//! Every query returns a [`QueryResult`] with:
//!   * `columns` — ordered column names (empty for
//!     non-SELECT statements).
//!   * `rows` — 2D vec of stringified cells. NULL becomes
//!     the empty string tagged with `null_cells` so the UI
//!     can render `NULL` differently from `""`.
//!   * `affected_rows` — non-zero for DML statements.
//!   * `last_insert_id` — `Some` after an AUTO_INCREMENT insert.
//!   * `elapsed_ms` — wall-clock round trip on the shared
//!     runtime.
//!
//! ## Not yet
//!
//! * Streaming result sets. M5d reads the whole result into
//!   memory; we cap at [`MAX_ROWS`] to keep the UI alive on
//!   a `SELECT *` against a huge table.
//! * Prepared statements / parameter binding. The UI runs
//!   whatever the user types in the SQL editor, as-is.
//! * Schema introspection beyond `SHOW DATABASES` / `SHOW
//!   TABLES` / `SHOW COLUMNS`.

use std::collections::BTreeSet;
use std::time::Instant;

use mysql_async::prelude::*;
use mysql_async::{Column, Pool, Row, Value};
use serde::{Deserialize, Serialize};

/// Hard cap on how many rows a single [`MysqlClient::execute`]
/// call will materialize. `SELECT * FROM huge` gets truncated
/// to this many rows plus a `truncated: true` flag in the
/// result. 10k is enough to scroll through meaningfully in
/// the UI without making `QueryResult` serialize megabytes.
pub const MAX_ROWS: usize = 10_000;

/// Hard cap on the length of any stringified cell value. A
/// multi-MB BLOB in a single row would otherwise make the
/// JSON round-trip balloon the UI heap.
pub const MAX_CELL_BYTES: usize = 4096;

/// Errors surfaced by the MySQL client.
#[derive(Debug, thiserror::Error)]
pub enum MysqlError {
    /// Underlying mysql_async error (connect, query, IO).
    #[error("mysql: {0}")]
    Native(String),

    /// Caller supplied a malformed URL / host / port.
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

impl From<mysql_async::Error> for MysqlError {
    fn from(e: mysql_async::Error) -> Self {
        // mysql_async's Error display includes the server
        // message for common failures, which is what the UI
        // wants to show.
        MysqlError::Native(e.to_string())
    }
}

/// Result alias for mysql ops.
pub type Result<T, E = MysqlError> = std::result::Result<T, E>;

/// Connection config for a MySQL endpoint. Kept as a struct
/// so future auth modes (TLS CA, client cert) can be added
/// without a signature change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MysqlConfig {
    /// Hostname or IP. Usually `"127.0.0.1"` via SSH tunnel.
    pub host: String,
    /// TCP port. The tunnel's local port, not the remote 3306.
    pub port: u16,
    /// MySQL user.
    pub user: String,
    /// Plaintext password. Empty string means "no password".
    pub password: String,
    /// Default database to `USE` on connect, if any.
    pub database: Option<String>,
}

impl MysqlConfig {
    /// Build a `mysql://...` URL from the config. Kept private
    /// to the module because URL-escaping the password is
    /// fiddly — callers should pass a `MysqlConfig` and let
    /// us build the URL.
    fn to_opts(&self) -> Result<mysql_async::Opts> {
        if self.host.is_empty() {
            return Err(MysqlError::InvalidConfig("empty host".into()));
        }
        if self.port == 0 {
            return Err(MysqlError::InvalidConfig("port must be > 0".into()));
        }
        if self.user.is_empty() {
            return Err(MysqlError::InvalidConfig("empty user".into()));
        }
        let mut builder = mysql_async::OptsBuilder::default()
            .ip_or_hostname(self.host.clone())
            .tcp_port(self.port)
            .user(Some(self.user.clone()))
            .pass(if self.password.is_empty() {
                None
            } else {
                Some(self.password.clone())
            });
        if let Some(db) = self.database.as_ref().filter(|d| !d.is_empty()) {
            builder = builder.db_name(Some(db.clone()));
        }
        Ok(builder.into())
    }
}

/// One row of query results. Uses [`Option<String>`] per
/// cell so NULLs are preserved losslessly across the JSON
/// round-trip: `None` → `null`, `Some(s)` → `"s"`.
pub type ResultRow = Vec<Option<String>>;

/// One row in the schema-tree's tables list. Carries the
/// `information_schema.tables` enrichment the panel surfaces as
/// inline badges + tooltip metadata: row count, on-disk size,
/// engine, last-update timestamp.
///
/// `row_count` is `Option<u64>` because InnoDB reports it as a
/// statistical estimate — for some tables it can be unavailable
/// (`NULL` in the catalog) until the engine has gathered stats.
/// We forward the `Option` so the UI can render `—` instead of
/// `0` when the count is genuinely unknown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSummary {
    /// Table name from `information_schema.tables`.
    pub name: String,
    /// Row count estimate; `None` when the catalog reports `NULL`.
    pub row_count: Option<u64>,
    /// On-disk data segment size in bytes.
    pub data_bytes: Option<u64>,
    /// On-disk index segment size in bytes.
    pub index_bytes: Option<u64>,
    /// Storage engine (`InnoDB`, `MyISAM`, `Aria`, …).
    pub engine: Option<String>,
    /// Last-update timestamp as the server formats it; `None` for
    /// engines that don't track it (most InnoDB tables).
    pub updated_at: Option<String>,
    /// `information_schema.tables.table_comment`. Empty string for
    /// tables without a comment — we forward verbatim so the UI can
    /// render a tooltip line only when it has content.
    pub comment: String,
}

/// One row in the schema-tree's routines folder — covers both
/// stored procedures and stored functions. `kind` is `"PROCEDURE"`
/// or `"FUNCTION"` per `information_schema.routines.routine_type`,
/// so the panel can group them under the same folder with
/// per-row icons.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutineSummary {
    /// Routine name from `information_schema.routines.routine_name`.
    pub name: String,
    /// `"PROCEDURE"` or `"FUNCTION"` from `routine_type`.
    pub kind: String,
}

/// One column from `SHOW COLUMNS`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnInfo {
    /// Column name.
    pub name: String,
    /// Raw MySQL type string, e.g. `varchar(255)`.
    pub column_type: String,
    /// True when the column accepts NULL.
    pub nullable: bool,
    /// Key marker from MySQL (`PRI`, `UNI`, `MUL`, or empty).
    pub key: String,
    /// Default value as displayed by the server.
    pub default_value: Option<String>,
    /// Extra metadata, e.g. `auto_increment`.
    pub extra: String,
    /// `COLUMN_COMMENT` from `SHOW FULL COLUMNS`. Empty string when
    /// the column has no comment.
    pub comment: String,
}

/// One index defined on a table — name, ordered column list, and
/// uniqueness / kind metadata. The Structure tab renders this in
/// a small "Indexes" section under the column grid. Single-column
/// and multi-column indexes share the shape; `columns` preserves
/// ordinal position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexSummary {
    /// Index name from `information_schema.statistics.index_name`.
    pub name: String,
    /// Indexed columns in ordinal-position order.
    pub columns: Vec<String>,
    /// True for UNIQUE / PRIMARY KEY indexes.
    pub unique: bool,
    /// `BTREE` / `HASH` / `FULLTEXT` / `SPATIAL` from
    /// `information_schema.statistics.index_type`. Most users
    /// only care that it's BTREE, but exposing the raw value
    /// keeps the UI engine-agnostic.
    pub kind: String,
}

/// One foreign-key constraint on a table. Carries everything the
/// UI needs to show a "X (col) → Y (col)" arrow + on-update /
/// on-delete cascade rules. Multi-column FKs preserve order in
/// `columns` / `ref_columns` (same length, paired by index).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKey {
    /// Constraint name from the catalog (e.g. `fk_orders_user`).
    pub name: String,
    /// Local columns participating in the FK, in declaration order.
    pub columns: Vec<String>,
    /// Schema of the referenced table — usually the same DB.
    pub ref_schema: String,
    /// Referenced table name.
    pub ref_table: String,
    /// Referenced columns, paired by index with `columns`.
    pub ref_columns: Vec<String>,
    /// Action token from `referential_constraints.update_rule` —
    /// `NO ACTION` / `RESTRICT` / `CASCADE` / `SET NULL` /
    /// `SET DEFAULT`.
    pub on_update: String,
    /// Action token from `referential_constraints.delete_rule`.
    pub on_delete: String,
}

/// Full result of a single executed statement.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryResult {
    /// Column names, in the order they came back. Empty for
    /// non-SELECT statements.
    pub columns: Vec<String>,
    /// Materialized rows. Capped at [`MAX_ROWS`] — see
    /// `truncated`.
    pub rows: Vec<ResultRow>,
    /// True if the server had more rows than we returned.
    pub truncated: bool,
    /// Affected row count from DML. Zero for SELECTs.
    pub affected_rows: u64,
    /// Last AUTO_INCREMENT id, if any.
    pub last_insert_id: Option<u64>,
    /// Wall-clock execution time on the shared runtime.
    pub elapsed_ms: u64,
}

/// MySQL client handle. Clone is cheap (the underlying pool
/// is Arc-wrapped inside mysql_async).
#[derive(Clone)]
pub struct MysqlClient {
    pool: Pool,
}

impl std::fmt::Debug for MysqlClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MysqlClient").finish()
    }
}

impl MysqlClient {
    /// Open a connection to the configured endpoint and
    /// verify liveness with `SELECT 1`. Returns an error if
    /// the TCP connect, handshake, auth, or probe fails.
    pub async fn connect(config: MysqlConfig) -> Result<Self> {
        let opts = config.to_opts()?;
        let pool = Pool::new(opts);

        // Round-trip ping. mysql_async establishes the real
        // connection lazily, so we force one now so connect
        // errors surface before the first user query.
        let mut conn = pool.get_conn().await?;
        let _: Option<u8> = "SELECT 1".first::<u8, _>(&mut conn).await?;
        drop(conn);

        Ok(Self { pool })
    }

    /// Blocking wrapper for [`Self::connect`].
    pub fn connect_blocking(config: MysqlConfig) -> Result<Self> {
        crate::ssh::runtime::shared().block_on(Self::connect(config))
    }

    /// Run a single SQL statement and return the first result
    /// set (plus row counts). Multi-statement strings are not
    /// supported — mysql_async's `query_iter` only returns
    /// the first result set through its sync API shape.
    pub async fn execute(&self, sql: &str) -> Result<QueryResult> {
        let start = Instant::now();
        let mut conn = self.pool.get_conn().await?;

        // `query_iter` gives us a handle to the first
        // ResultSet without committing us to a specific row
        // type. If the statement produced a result set, we
        // iterate it with the cap applied; otherwise we read
        // the affected_rows / last_insert_id fields.
        let mut result = conn.query_iter(sql).await?;

        // Try to collect the first (possibly empty) result
        // set. `collect::<Row>` hits None for non-SELECT
        // statements; we then read counts from the result
        // handle and return.
        let mut columns_out: Vec<String> = Vec::new();
        let mut rows_out: Vec<ResultRow> = Vec::new();
        let mut truncated = false;

        // Fetch column schema before we start pulling rows.
        if let Some(cols) = result.columns().as_ref().map(|arc| arc.clone()) {
            columns_out = cols
                .iter()
                .map(|c: &Column| c.name_str().into_owned())
                .collect();
        }

        // `for_each` is the idiomatic mysql_async row loop —
        // it streams rows out of the tokio task without
        // materializing the full set inside the driver.
        let mut count: usize = 0;
        result
            .for_each(|row: Row| {
                if count >= MAX_ROWS {
                    truncated = true;
                    return;
                }
                rows_out.push(row_to_cells(&row));
                count += 1;
            })
            .await?;

        let affected_rows = result.affected_rows();
        let last_insert_id = result.last_insert_id();
        drop(result);
        drop(conn);

        Ok(QueryResult {
            columns: columns_out,
            rows: rows_out,
            truncated,
            affected_rows,
            last_insert_id,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Blocking wrapper for [`Self::execute`].
    pub fn execute_blocking(&self, sql: &str) -> Result<QueryResult> {
        crate::ssh::runtime::shared().block_on(self.execute(sql))
    }

    /// `SHOW DATABASES`, filtered to strip internal schemas
    /// the user almost never wants to browse
    /// (`information_schema`, `performance_schema`, `mysql`,
    /// `sys`). The filtered names are still queryable by
    /// typing them into the SQL editor, they just don't
    /// clutter the schema picker.
    pub async fn list_databases(&self) -> Result<Vec<String>> {
        let mut conn = self.pool.get_conn().await?;
        let rows: Vec<String> = "SHOW DATABASES".fetch(&mut conn).await?;
        drop(conn);

        let hidden: BTreeSet<&str> = ["information_schema", "performance_schema", "mysql", "sys"]
            .into_iter()
            .collect();
        Ok(rows
            .into_iter()
            .filter(|n| !hidden.contains(n.as_str()))
            .collect())
    }

    /// Blocking wrapper for [`Self::list_databases`].
    pub fn list_databases_blocking(&self) -> Result<Vec<String>> {
        crate::ssh::runtime::shared().block_on(self.list_databases())
    }

    /// `SHOW TABLES FROM <db>`. Returns tables in the server's
    /// order (usually alphabetical).
    pub async fn list_tables(&self, database: &str) -> Result<Vec<String>> {
        if !is_safe_ident(database) {
            return Err(MysqlError::InvalidConfig(format!(
                "refusing unsafe database identifier {database:?}"
            )));
        }
        let mut conn = self.pool.get_conn().await?;
        // SHOW TABLES FROM `foo` can't be parameterized —
        // backtick-escape the identifier after the safety
        // check above.
        let sql = format!("SHOW TABLES FROM `{database}`");
        let rows: Vec<String> = sql.fetch(&mut conn).await?;
        drop(conn);
        Ok(rows)
    }

    /// Blocking wrapper for [`Self::list_tables`].
    pub fn list_tables_blocking(&self, database: &str) -> Result<Vec<String>> {
        crate::ssh::runtime::shared().block_on(self.list_tables(database))
    }

    /// Enriched table list — same set of tables as
    /// [`Self::list_tables`] but joined with
    /// `information_schema.tables` so the UI gets per-table
    /// engine, row-count estimate, on-disk size, and
    /// last-update timestamp. The same safety check on the
    /// database identifier applies.
    ///
    /// We query `information_schema.tables` directly (not
    /// `SHOW TABLE STATUS`) so the parameter binding is
    /// compatible with mysql_async's `params!` macro — no
    /// identifier interpolation needed beyond the schema
    /// itself, which is already guarded.
    pub async fn list_tables_meta(&self, database: &str) -> Result<Vec<TableSummary>> {
        if !is_safe_ident(database) {
            return Err(MysqlError::InvalidConfig(format!(
                "refusing unsafe database identifier {database:?}"
            )));
        }
        let mut conn = self.pool.get_conn().await?;
        // `table_type = 'BASE TABLE'` excludes views (which we
        // surface in their own folder) but includes regular
        // tables in either InnoDB or MyISAM. `update_time` is
        // returned as a string to dodge mysql_async's narrower
        // DATETIME types — we just forward what the server says.
        let sql = "
            SELECT table_name, table_rows, data_length, index_length,
                   engine, CAST(update_time AS CHAR) AS update_time,
                   COALESCE(table_comment, '')
              FROM information_schema.tables
             WHERE table_schema = :schema
               AND table_type = 'BASE TABLE'
             ORDER BY table_name
        ";
        type TableRow = (
            String,
            Option<u64>,
            Option<u64>,
            Option<u64>,
            Option<String>,
            Option<String>,
            String,
        );
        let rows: Vec<TableRow> = sql
            .with(mysql_async::params! { "schema" => database })
            .fetch(&mut conn)
            .await?;
        drop(conn);
        Ok(rows
            .into_iter()
            .map(
                |(name, row_count, data_bytes, index_bytes, engine, updated_at, comment)| {
                    TableSummary {
                        name,
                        row_count,
                        data_bytes,
                        index_bytes,
                        engine,
                        updated_at,
                        comment,
                    }
                },
            )
            .collect())
    }

    /// Blocking wrapper for [`Self::list_tables_meta`].
    pub fn list_tables_meta_blocking(&self, database: &str) -> Result<Vec<TableSummary>> {
        crate::ssh::runtime::shared().block_on(self.list_tables_meta(database))
    }

    /// View names defined in `database`. Mirrors `list_tables`
    /// but filters `table_type = 'VIEW'` so the panel can put
    /// them under their own collapsible folder. Returned in
    /// alphabetical order.
    pub async fn list_views(&self, database: &str) -> Result<Vec<String>> {
        if !is_safe_ident(database) {
            return Err(MysqlError::InvalidConfig(format!(
                "refusing unsafe database identifier {database:?}"
            )));
        }
        let mut conn = self.pool.get_conn().await?;
        let sql = "
            SELECT table_name
              FROM information_schema.views
             WHERE table_schema = :schema
             ORDER BY table_name
        ";
        let rows: Vec<String> = sql
            .with(mysql_async::params! { "schema" => database })
            .fetch(&mut conn)
            .await?;
        drop(conn);
        Ok(rows)
    }

    /// Blocking wrapper for [`Self::list_views`].
    pub fn list_views_blocking(&self, database: &str) -> Result<Vec<String>> {
        crate::ssh::runtime::shared().block_on(self.list_views(database))
    }

    /// Stored procedures + stored functions defined in
    /// `database`. The `kind` field discriminates the two so
    /// the panel can render a per-row icon ("ƒ" for FUNCTION,
    /// "λ" for PROCEDURE — the choice is the panel's). Sorted
    /// by name across both kinds for stable list ordering.
    pub async fn list_routines(&self, database: &str) -> Result<Vec<RoutineSummary>> {
        if !is_safe_ident(database) {
            return Err(MysqlError::InvalidConfig(format!(
                "refusing unsafe database identifier {database:?}"
            )));
        }
        let mut conn = self.pool.get_conn().await?;
        let sql = "
            SELECT routine_name, routine_type
              FROM information_schema.routines
             WHERE routine_schema = :schema
             ORDER BY routine_name
        ";
        let rows: Vec<(String, String)> = sql
            .with(mysql_async::params! { "schema" => database })
            .fetch(&mut conn)
            .await?;
        drop(conn);
        Ok(rows
            .into_iter()
            .map(|(name, kind)| RoutineSummary { name, kind })
            .collect())
    }

    /// Blocking wrapper for [`Self::list_routines`].
    pub fn list_routines_blocking(&self, database: &str) -> Result<Vec<RoutineSummary>> {
        crate::ssh::runtime::shared().block_on(self.list_routines(database))
    }

    /// `SHOW COLUMNS FROM <db>.<table>`.
    pub async fn list_columns(&self, database: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        if !is_safe_ident(database) {
            return Err(MysqlError::InvalidConfig(format!(
                "refusing unsafe database identifier {database:?}"
            )));
        }
        if !is_safe_ident(table) {
            return Err(MysqlError::InvalidConfig(format!(
                "refusing unsafe table identifier {table:?}"
            )));
        }
        let mut conn = self.pool.get_conn().await?;
        // `SHOW FULL COLUMNS` adds Collation / Privileges / Comment
        // to the standard SHOW COLUMNS output. We only need Comment
        // here but the column order is fixed by the server, so we
        // bind all 9 and drop the two we don't surface.
        let sql = format!("SHOW FULL COLUMNS FROM `{database}`.`{table}`");
        type FullColumnRow = (
            String,         // Field
            String,         // Type
            Option<String>, // Collation
            String,         // Null
            String,         // Key
            Option<String>, // Default
            String,         // Extra
            String,         // Privileges
            String,         // Comment
        );
        let rows: Vec<FullColumnRow> = sql.fetch(&mut conn).await?;
        drop(conn);

        Ok(rows
            .into_iter()
            .map(
                |(name, column_type, _collation, null_flag, key, default_value, extra, _priv, comment)| {
                    ColumnInfo {
                        name,
                        column_type,
                        nullable: null_flag.eq_ignore_ascii_case("YES"),
                        key,
                        default_value,
                        extra,
                        comment,
                    }
                },
            )
            .collect())
    }

    /// Blocking wrapper for [`Self::list_columns`].
    pub fn list_columns_blocking(&self, database: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        crate::ssh::runtime::shared().block_on(self.list_columns(database, table))
    }

    /// All indexes defined on `<database>.<table>`. Pulls from
    /// `information_schema.statistics`, which lists one row per
    /// (index, column) pair — we group by `index_name` so the
    /// caller gets one [`IndexSummary`] per index with its
    /// columns ordered by `seq_in_index`.
    pub async fn list_indexes(&self, database: &str, table: &str) -> Result<Vec<IndexSummary>> {
        if !is_safe_ident(database) {
            return Err(MysqlError::InvalidConfig(format!(
                "refusing unsafe database identifier {database:?}"
            )));
        }
        if !is_safe_ident(table) {
            return Err(MysqlError::InvalidConfig(format!(
                "refusing unsafe table identifier {table:?}"
            )));
        }
        let mut conn = self.pool.get_conn().await?;
        // `non_unique` is `0` for unique indexes (including the
        // primary key) and `1` otherwise — we invert. Index_type
        // and the per-column `seq_in_index` come back per row;
        // the panel groups them client-side.
        let sql = "
            SELECT index_name, column_name, seq_in_index, non_unique, index_type
              FROM information_schema.statistics
             WHERE table_schema = :schema AND table_name = :table
             ORDER BY index_name, seq_in_index
        ";
        let rows: Vec<(String, String, u32, u32, String)> = sql
            .with(mysql_async::params! {
                "schema" => database,
                "table" => table,
            })
            .fetch(&mut conn)
            .await?;
        drop(conn);

        // Group rows by index_name preserving order. We rely on
        // the SQL ORDER BY having put related rows together so a
        // simple "current name" walk is enough — no HashMap needed.
        let mut out: Vec<IndexSummary> = Vec::new();
        for (name, column, _seq, non_unique, kind) in rows {
            match out.last_mut() {
                Some(last) if last.name == name => {
                    last.columns.push(column);
                }
                _ => {
                    out.push(IndexSummary {
                        name,
                        columns: vec![column],
                        unique: non_unique == 0,
                        kind,
                    });
                }
            }
        }
        Ok(out)
    }

    /// Blocking wrapper for [`Self::list_indexes`].
    pub fn list_indexes_blocking(&self, database: &str, table: &str) -> Result<Vec<IndexSummary>> {
        crate::ssh::runtime::shared().block_on(self.list_indexes(database, table))
    }

    /// All foreign keys outgoing from `<database>.<table>`. Joins
    /// `key_column_usage` (column-level FK rows) with
    /// `referential_constraints` (one-per-constraint update /
    /// delete rules). Composite FKs come back as one
    /// [`ForeignKey`] with multiple paired entries in `columns` /
    /// `ref_columns` — same logic as the index grouping.
    pub async fn list_foreign_keys(&self, database: &str, table: &str) -> Result<Vec<ForeignKey>> {
        if !is_safe_ident(database) {
            return Err(MysqlError::InvalidConfig(format!(
                "refusing unsafe database identifier {database:?}"
            )));
        }
        if !is_safe_ident(table) {
            return Err(MysqlError::InvalidConfig(format!(
                "refusing unsafe table identifier {table:?}"
            )));
        }
        let mut conn = self.pool.get_conn().await?;
        let sql = "
            SELECT k.constraint_name,
                   k.column_name,
                   k.referenced_table_schema,
                   k.referenced_table_name,
                   k.referenced_column_name,
                   r.update_rule,
                   r.delete_rule,
                   k.ordinal_position
              FROM information_schema.key_column_usage AS k
              JOIN information_schema.referential_constraints AS r
                ON r.constraint_schema = k.constraint_schema
               AND r.constraint_name   = k.constraint_name
             WHERE k.table_schema = :schema
               AND k.table_name   = :table
               AND k.referenced_table_name IS NOT NULL
             ORDER BY k.constraint_name, k.ordinal_position
        ";
        type FkRow = (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            String,
            u32,
        );
        let rows: Vec<FkRow> = sql
            .with(mysql_async::params! {
                "schema" => database,
                "table" => table,
            })
            .fetch(&mut conn)
            .await?;
        drop(conn);

        let mut out: Vec<ForeignKey> = Vec::new();
        for (name, col, ref_schema, ref_table, ref_col, on_update, on_delete, _ord) in rows {
            let ref_schema = ref_schema.unwrap_or_default();
            let ref_table = ref_table.unwrap_or_default();
            let ref_col = ref_col.unwrap_or_default();
            match out.last_mut() {
                Some(last) if last.name == name => {
                    last.columns.push(col);
                    last.ref_columns.push(ref_col);
                }
                _ => {
                    out.push(ForeignKey {
                        name,
                        columns: vec![col],
                        ref_schema,
                        ref_table,
                        ref_columns: vec![ref_col],
                        on_update,
                        on_delete,
                    });
                }
            }
        }
        Ok(out)
    }

    /// Blocking wrapper for [`Self::list_foreign_keys`].
    pub fn list_foreign_keys_blocking(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        crate::ssh::runtime::shared().block_on(self.list_foreign_keys(database, table))
    }

    /// Tear down the pool. Called when the UI panel closes.
    /// Returning Ok means the pool was already disconnected
    /// cleanly; Err usually means the connection had dropped
    /// before we got here, which is still fine from the UI's
    /// point of view.
    pub async fn disconnect(self) -> Result<()> {
        self.pool.disconnect().await?;
        Ok(())
    }
}

/// Convert a single row from mysql_async into our
/// [`ResultRow`] representation. Each cell becomes either
/// `None` (NULL) or `Some(display)`.
fn row_to_cells(row: &Row) -> ResultRow {
    let mut out: ResultRow = Vec::with_capacity(row.len());
    for i in 0..row.len() {
        let value: Option<Value> = row.as_ref(i).cloned();
        out.push(match value {
            None | Some(Value::NULL) => None,
            Some(v) => Some(value_to_display(&v)),
        });
    }
    out
}

/// Render a MySQL `Value` to a display string, truncated at
/// [`MAX_CELL_BYTES`].
fn value_to_display(v: &Value) -> String {
    let text = match v {
        Value::NULL => return String::new(),
        Value::Bytes(bytes) => match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => format!("0x{}", hex_short(bytes)),
        },
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Double(d) => d.to_string(),
        Value::Date(y, mo, d, h, mi, s, us) => {
            format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{us:06}")
        }
        Value::Time(neg, d, h, mi, s, us) => {
            let sign = if *neg { "-" } else { "" };
            format!("{sign}{d}:{h:02}:{mi:02}:{s:02}.{us:06}")
        }
    };
    truncate_utf8(text, MAX_CELL_BYTES)
}

/// Format the first 16 bytes of a BLOB as hex, for the
/// `0x...` preview used on binary columns.
fn hex_short(bytes: &[u8]) -> String {
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

/// Truncate `s` to at most `n` bytes on a UTF-8 boundary.
/// When we have to cut, append `"…"` so the UI can tell at a
/// glance that the cell was shortened.
fn truncate_utf8(s: String, n: usize) -> String {
    if s.len() <= n {
        return s;
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push('…');
    out
}

/// Same allowlist as [`super::docker::is_safe_id`] but a
/// little stricter: identifiers passed to `SHOW TABLES FROM`
/// are backtick-quoted, but a backtick in the name itself
/// would break out of the quoting. MySQL identifiers may be
/// up to 64 chars (`NAME_LEN`) and (per spec) can contain
/// basic ASCII letters, digits, `_`, and `$`. We reject
/// everything else to keep the quoting trivially safe.
pub fn is_safe_ident(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_to_opts_accepts_localhost_defaults() {
        let cfg = MysqlConfig {
            host: "127.0.0.1".into(),
            port: 13306,
            user: "root".into(),
            password: "hunter2".into(),
            database: Some("testdb".into()),
        };
        let opts = cfg.to_opts().expect("opts");
        assert_eq!(opts.ip_or_hostname(), "127.0.0.1");
        assert_eq!(opts.tcp_port(), 13306);
        assert_eq!(opts.user(), Some("root"));
    }

    #[test]
    fn config_to_opts_rejects_empty_host() {
        let cfg = MysqlConfig {
            host: "".into(),
            port: 13306,
            user: "root".into(),
            password: "".into(),
            database: None,
        };
        assert!(matches!(cfg.to_opts(), Err(MysqlError::InvalidConfig(_))));
    }

    #[test]
    fn config_to_opts_rejects_zero_port() {
        let cfg = MysqlConfig {
            host: "127.0.0.1".into(),
            port: 0,
            user: "root".into(),
            password: "".into(),
            database: None,
        };
        assert!(matches!(cfg.to_opts(), Err(MysqlError::InvalidConfig(_))));
    }

    #[test]
    fn config_to_opts_rejects_empty_user() {
        let cfg = MysqlConfig {
            host: "127.0.0.1".into(),
            port: 13306,
            user: "".into(),
            password: "".into(),
            database: None,
        };
        assert!(matches!(cfg.to_opts(), Err(MysqlError::InvalidConfig(_))));
    }

    #[test]
    fn is_safe_ident_accepts_canonical_forms() {
        assert!(is_safe_ident("testdb"));
        assert!(is_safe_ident("my_db_2"));
        assert!(is_safe_ident("TableName"));
        assert!(is_safe_ident("Z9"));
        assert!(is_safe_ident("$sys"));
    }

    #[test]
    fn is_safe_ident_rejects_metacharacters() {
        for evil in [
            "",
            "a b",
            "a;DROP TABLE x",
            "a`b",
            "a\"b",
            "a'b",
            "a\\b",
            "a-b",
            "a.b",
            "a/b",
            "a\nb",
        ] {
            assert!(!is_safe_ident(evil), "{evil:?} must be rejected");
        }
    }

    #[test]
    fn is_safe_ident_rejects_overlong() {
        let too_long = "a".repeat(65);
        assert!(!is_safe_ident(&too_long));
        let max = "a".repeat(64);
        assert!(is_safe_ident(&max));
    }

    #[test]
    fn truncate_utf8_respects_codepoint_boundary() {
        let s = "abcé".to_string();
        assert_eq!(truncate_utf8(s.clone(), 100), "abcé");
        let cut = truncate_utf8(s.clone(), 4);
        assert_eq!(cut, "abc…"); // é is 2 bytes, so we cut to 3
        let cut2 = truncate_utf8("hello world".to_string(), 5);
        assert_eq!(cut2, "hello…");
    }

    #[test]
    fn hex_short_renders_byte_prefix() {
        let bytes = [0xde, 0xad, 0xbe, 0xef];
        assert_eq!(hex_short(&bytes), "deadbeef");
        let long: Vec<u8> = (0u8..32).collect();
        let h = hex_short(&long);
        assert!(h.starts_with("000102"));
        assert!(h.ends_with('…'));
    }

    #[test]
    fn value_to_display_int_and_bytes() {
        assert_eq!(value_to_display(&Value::Int(42)), "42");
        assert_eq!(value_to_display(&Value::UInt(42)), "42");
        assert_eq!(
            value_to_display(&Value::Bytes("hello".as_bytes().to_vec())),
            "hello"
        );
        // Invalid UTF-8 falls back to hex preview.
        let bad = Value::Bytes(vec![0xff, 0xfe, 0xfd]);
        assert!(value_to_display(&bad).starts_with("0x"));
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
            elapsed_ms: 12,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: QueryResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.columns, r.columns);
        assert_eq!(back.rows.len(), 2);
        assert_eq!(back.rows[1][1], None);
    }
}
