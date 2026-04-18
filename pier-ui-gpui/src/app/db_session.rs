//! Per-tab database session state (P0-1 Phase A skeleton).
//!
//! One `DbSessionState` is parked behind each [`crate::app::route::DbKind`]
//! tab. It mirrors the role [`crate::app::ssh_session::SshSessionState`]
//! plays for the SFTP browser:
//!
//! 1. UI calls `begin_*` on the entity to mint a request + bump a nonce
//!    + flip status into `Connecting / Querying`.
//! 2. PierApp dispatches `run_*(request)` on `cx.background_executor()`
//!    so the blocking `pier-core` calls never freeze the UI.
//! 3. The async task hands the `*Result` back to the entity via
//!    `apply_*_result`, which checks the nonce and stores the outcome.
//!
//! Phase A intentionally only wires MySQL and PostgreSQL; Redis (a
//! key-value inspector) and SQLite (subprocess + file picker) get their
//! own variants and code paths in later phases.
//!
//! Step 2 of 6: types + entity skeleton + stub `run_*` returning
//! "not implemented yet" errors. Step 3 fills in the bodies.

#![allow(dead_code)] // Step 5 wires it into the view; until then the
// fields are populated but only consumed by tests.

use std::sync::Arc;

use gpui::SharedString;
use pier_core::db_connections::{DbConnection, DbEngine};
use pier_core::services::mysql::{MysqlClient, QueryResult as MysqlQueryResult};
use pier_core::services::postgres::{PostgresClient, QueryResult as PgQueryResult};

/// High-level connection state, drives the status pill in the UI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DbStatus {
    /// No connection attempt yet (just opened the tab, or `Disconnect`
    /// was clicked).
    #[default]
    Idle,
    /// `connect_blocking` is in flight on the background executor.
    Connecting,
    /// Connected — `client` is `Some` and queries can be issued.
    Connected,
    /// Last connect attempt failed; see `last_error`.
    Failed,
}

/// Engine-tagged client handle. Wrapped in `Arc` so background tasks can
/// `.clone()` cheaply while the UI thread retains a handle for the next
/// query — `MysqlClient` (a `Pool`) and `PostgresClient` (a
/// `tokio_postgres::Client`) both support concurrent `&self` execute,
/// so a shared `Arc` is enough.
#[derive(Clone)]
pub enum DbClient {
    /// MySQL backend.
    Mysql(Arc<MysqlClient>),
    /// PostgreSQL backend.
    Postgres(Arc<PostgresClient>),
}

impl DbClient {
    /// Engine variant carried by this client. Useful when surfaces
    /// (e.g. result table headers) need to know what backend produced
    /// the rows without unpacking the inner client.
    pub fn engine(&self) -> DbEngine {
        match self {
            Self::Mysql(_) => DbEngine::Mysql,
            Self::Postgres(_) => DbEngine::Postgres,
        }
    }
}

/// `QueryResult` is structurally identical between MySQL and PG (see
/// `pier-core/services/{mysql,postgres}.rs`), but the types are distinct
/// so the UI layer wraps them in this enum and the renderer matches on
/// `engine()` to avoid duplicating render code.
#[derive(Clone, Debug)]
pub enum DbQueryResult {
    /// Result from MySQL.
    Mysql(MysqlQueryResult),
    /// Result from PostgreSQL.
    Postgres(PgQueryResult),
}

impl DbQueryResult {
    /// Column names in row-order. Empty for non-SELECT statements.
    pub fn columns(&self) -> &[String] {
        match self {
            Self::Mysql(r) => &r.columns,
            Self::Postgres(r) => &r.columns,
        }
    }

    /// Materialised rows, capped server-side at 10k by pier-core.
    pub fn rows(&self) -> &[Vec<Option<String>>] {
        match self {
            Self::Mysql(r) => &r.rows,
            Self::Postgres(r) => &r.rows,
        }
    }

    /// True if the server had more rows than we returned.
    pub fn truncated(&self) -> bool {
        match self {
            Self::Mysql(r) => r.truncated,
            Self::Postgres(r) => r.truncated,
        }
    }

    /// Affected row count for DML.
    pub fn affected_rows(&self) -> u64 {
        match self {
            Self::Mysql(r) => r.affected_rows,
            Self::Postgres(r) => r.affected_rows,
        }
    }

    /// Wall-clock time the engine spent on this statement.
    pub fn elapsed_ms(&self) -> u64 {
        match self {
            Self::Mysql(r) => r.elapsed_ms,
            Self::Postgres(r) => r.elapsed_ms,
        }
    }
}

/// Per-tab session state held inside an `Entity<DbSessionState>`.
pub struct DbSessionState {
    /// Connection currently selected in the dropdown. `None` until the
    /// user picks one (or the first connection is auto-selected).
    pub connection: Option<DbConnection>,
    /// Live client after a successful `Connect`. `None` while
    /// `Idle / Connecting / Failed`.
    pub client: Option<DbClient>,
    /// Drives the status pill.
    pub status: DbStatus,
    /// Last error message from any failed call (connect / list / query).
    pub last_error: Option<SharedString>,
    /// `SHOW DATABASES` / `pg_database` cache, populated on connect.
    pub databases: Vec<String>,
    /// Database currently picked in the schema sidebar.
    pub selected_database: Option<String>,
    /// Tables in `selected_database`, populated on database select.
    pub tables: Vec<String>,
    /// Last successful query result, displayed in the result table.
    pub last_result: Option<DbQueryResult>,
    /// True while `execute_blocking` is in flight — UI disables the
    /// Run button.
    pub query_in_flight: bool,

    // ─── Stale-result guards (mirror SshSessionState's *_nonce fields).
    pub(crate) connect_nonce: u64,
    pub(crate) list_nonce: u64,
    pub(crate) query_nonce: u64,
}

impl DbSessionState {
    /// Empty session state, ready to accept a connection selection.
    pub fn new() -> Self {
        Self {
            connection: None,
            client: None,
            status: DbStatus::Idle,
            last_error: None,
            databases: Vec::new(),
            selected_database: None,
            tables: Vec::new(),
            last_result: None,
            query_in_flight: false,
            connect_nonce: 0,
            list_nonce: 0,
            query_nonce: 0,
        }
    }

    /// True while any background task this state issued is in flight.
    /// Used by the UI to gate buttons.
    pub fn is_busy(&self) -> bool {
        matches!(self.status, DbStatus::Connecting) || self.query_in_flight
    }

    /// Pick a connection (drops any existing client, status returns to
    /// Idle so the user has to click `Connect`). Returns the new
    /// selection so the caller can immediately schedule a connect.
    pub fn select_connection(&mut self, connection: DbConnection) -> &DbConnection {
        self.connection = Some(connection);
        self.client = None;
        self.status = DbStatus::Idle;
        self.last_error = None;
        self.databases.clear();
        self.selected_database = None;
        self.tables.clear();
        self.last_result = None;
        self.query_in_flight = false;
        self.connection.as_ref().expect("just set")
    }

    /// Bump status into `Connecting` and mint a new connect request.
    /// Returns `None` if no connection is selected.
    pub fn begin_connect(&mut self, password: Option<String>) -> Option<ConnectRequest> {
        let connection = self.connection.clone()?;
        self.connect_nonce = self.connect_nonce.wrapping_add(1);
        self.status = DbStatus::Connecting;
        self.last_error = None;
        Some(ConnectRequest {
            nonce: self.connect_nonce,
            connection,
            password,
        })
    }

    /// Apply the result of a `run_connect` call. Stale nonces are
    /// dropped so an older outcome can't overwrite a newer one.
    pub fn apply_connect_result(&mut self, result: ConnectResult) {
        if result.nonce != self.connect_nonce {
            log::debug!(
                "db_session: dropping stale connect result (got {}, want {})",
                result.nonce,
                self.connect_nonce
            );
            return;
        }
        match result.outcome {
            Ok(client) => {
                self.client = Some(client);
                self.status = DbStatus::Connected;
                self.last_error = None;
            }
            Err(err) => {
                self.client = None;
                self.status = DbStatus::Failed;
                self.last_error = Some(err.into());
            }
        }
    }

    /// Mint a list-databases request. Returns `None` when no client.
    pub fn begin_list_databases(&mut self) -> Option<ListRequest> {
        let client = self.client.clone()?;
        self.list_nonce = self.list_nonce.wrapping_add(1);
        Some(ListRequest {
            nonce: self.list_nonce,
            client,
            kind: ListKind::Databases,
        })
    }

    /// Mint a list-tables request for the given database. Returns
    /// `None` when no client.
    pub fn begin_list_tables(&mut self, database: String) -> Option<ListRequest> {
        let client = self.client.clone()?;
        self.list_nonce = self.list_nonce.wrapping_add(1);
        self.selected_database = Some(database.clone());
        self.tables.clear();
        Some(ListRequest {
            nonce: self.list_nonce,
            client,
            kind: ListKind::Tables { database },
        })
    }

    /// Apply a list result; stale nonces dropped.
    pub fn apply_list_result(&mut self, result: ListResult) {
        if result.nonce != self.list_nonce {
            return;
        }
        match result.outcome {
            Ok(ListPayload::Databases(list)) => {
                self.databases = list;
                self.last_error = None;
            }
            Ok(ListPayload::Tables(list)) => {
                self.tables = list;
                self.last_error = None;
            }
            Err(err) => {
                self.last_error = Some(err.into());
            }
        }
    }

    /// Mint a query request. Returns `None` when no client OR when a
    /// query is already in flight (UI is expected to gate the button
    /// but we double-check here).
    pub fn begin_execute(&mut self, sql: String) -> Option<ExecuteRequest> {
        if self.query_in_flight {
            return None;
        }
        let client = self.client.clone()?;
        self.query_nonce = self.query_nonce.wrapping_add(1);
        self.query_in_flight = true;
        self.last_error = None;
        Some(ExecuteRequest {
            nonce: self.query_nonce,
            client,
            sql,
        })
    }

    /// Apply a query result. Stale nonces dropped (and the in-flight
    /// flag is cleared either way so a later request isn't blocked).
    pub fn apply_execute_result(&mut self, result: ExecuteResult) {
        if result.nonce != self.query_nonce {
            // Same nonce => same query; staleness only happens if a
            // newer query was issued. Either way we are no longer
            // waiting on *this* one.
            return;
        }
        self.query_in_flight = false;
        match result.outcome {
            Ok(qr) => {
                self.last_result = Some(qr);
                self.last_error = None;
            }
            Err(err) => {
                self.last_error = Some(err.into());
            }
        }
    }
}

impl Default for DbSessionState {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Background-task request / result envelopes ───────────────────────

/// Request payload for [`run_connect`]. Owned data only — must be
/// `Send + 'static` so it crosses the executor boundary.
pub struct ConnectRequest {
    /// Stale-result guard.
    pub nonce: u64,
    /// Connection metadata (name / host / port / user / db).
    pub connection: DbConnection,
    /// Password retrieved from the OS keychain (or `None` for no-auth
    /// connections such as a local Postgres trust setup).
    pub password: Option<String>,
}

/// Result of [`run_connect`].
pub struct ConnectResult {
    /// Echoed nonce — `apply_connect_result` checks staleness.
    pub nonce: u64,
    /// `Ok(client)` or a stringified error.
    pub outcome: Result<DbClient, String>,
}

/// What kind of list to fetch.
pub enum ListKind {
    /// `SHOW DATABASES` / `pg_database`.
    Databases,
    /// `SHOW TABLES` / `pg_tables` for the given database / schema.
    Tables {
        /// Database name (MySQL) or schema (PostgreSQL).
        database: String,
    },
}

/// Request payload for [`run_list`].
pub struct ListRequest {
    /// Stale-result guard.
    pub nonce: u64,
    /// Live client clone.
    pub client: DbClient,
    /// Which list to fetch.
    pub kind: ListKind,
}

/// Decoded list payload.
pub enum ListPayload {
    /// Database names.
    Databases(Vec<String>),
    /// Table names within a database.
    Tables(Vec<String>),
}

/// Result of [`run_list`].
pub struct ListResult {
    /// Echoed nonce.
    pub nonce: u64,
    /// `Ok(payload)` or a stringified error.
    pub outcome: Result<ListPayload, String>,
}

/// Request payload for [`run_execute`].
pub struct ExecuteRequest {
    /// Stale-result guard.
    pub nonce: u64,
    /// Live client clone.
    pub client: DbClient,
    /// Statement to execute.
    pub sql: String,
}

/// Result of [`run_execute`].
pub struct ExecuteResult {
    /// Echoed nonce.
    pub nonce: u64,
    /// `Ok(result)` or a stringified error.
    pub outcome: Result<DbQueryResult, String>,
}

// ─── Stub workers (filled in by step 3) ───────────────────────────────

/// Open a fresh client + verify authentication. Step 2 stub; step 3
/// wires `MysqlClient::connect_blocking` / `PostgresClient::connect_blocking`.
pub fn run_connect(request: ConnectRequest) -> ConnectResult {
    ConnectResult {
        nonce: request.nonce,
        outcome: Err("run_connect: not implemented yet (Phase A step 3)".into()),
    }
}

/// Fetch the requested list. Step 2 stub.
pub fn run_list(request: ListRequest) -> ListResult {
    ListResult {
        nonce: request.nonce,
        outcome: Err("run_list: not implemented yet (Phase A step 3)".into()),
    }
}

/// Execute a single SQL statement. Step 2 stub.
pub fn run_execute(request: ExecuteRequest) -> ExecuteResult {
    ExecuteResult {
        nonce: request.nonce,
        outcome: Err("run_execute: not implemented yet (Phase A step 3)".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_connection() -> DbConnection {
        DbConnection {
            name: "test".into(),
            engine: DbEngine::Mysql,
            host: "127.0.0.1".into(),
            port: 3306,
            user: "root".into(),
            database: None,
            credential_id: None,
        }
    }

    #[test]
    fn select_connection_resets_dependent_state() {
        let mut state = DbSessionState::new();
        state.databases = vec!["old".into()];
        state.selected_database = Some("old".into());
        state.last_error = Some("previous fail".into());
        state.status = DbStatus::Failed;

        state.select_connection(fixture_connection());

        assert_eq!(state.status, DbStatus::Idle);
        assert!(state.databases.is_empty());
        assert!(state.selected_database.is_none());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn begin_connect_bumps_nonce_and_status() {
        let mut state = DbSessionState::new();
        state.select_connection(fixture_connection());
        let n0 = state.connect_nonce;
        let req = state.begin_connect(Some("pw".into())).unwrap();
        assert_eq!(req.nonce, n0 + 1);
        assert_eq!(state.status, DbStatus::Connecting);
    }

    #[test]
    fn stale_connect_result_is_dropped() {
        let mut state = DbSessionState::new();
        state.select_connection(fixture_connection());
        let _ = state.begin_connect(None); // nonce = 1
        let _ = state.begin_connect(None); // nonce = 2

        // Result for nonce=1 arrives late and must be ignored.
        state.apply_connect_result(ConnectResult {
            nonce: 1,
            outcome: Err("stale".into()),
        });
        assert_eq!(state.status, DbStatus::Connecting); // unchanged
        assert!(state.last_error.is_none());
    }

    #[test]
    fn begin_execute_returns_none_without_client() {
        let mut state = DbSessionState::new();
        assert!(state.begin_execute("SELECT 1".into()).is_none());
    }

    #[test]
    fn apply_execute_result_clears_in_flight_flag() {
        let mut state = DbSessionState::new();
        // Simulate "we issued a query and it's in flight" without
        // booting a real client (covered by integration tests in
        // step 3, when run_execute is wired to the real backend).
        state.query_in_flight = true;
        state.query_nonce = 7;
        state.apply_execute_result(ExecuteResult {
            nonce: 7,
            outcome: Err("forced".into()),
        });
        assert!(!state.query_in_flight);
        assert!(state.last_error.is_some());
    }

    #[test]
    fn stale_execute_result_is_dropped() {
        let mut state = DbSessionState::new();
        state.query_in_flight = true;
        state.query_nonce = 9;
        // Late result for an old nonce — must not touch state.
        state.apply_execute_result(ExecuteResult {
            nonce: 8,
            outcome: Err("stale".into()),
        });
        assert!(state.query_in_flight); // still waiting on nonce=9
        assert!(state.last_error.is_none());
    }
}
