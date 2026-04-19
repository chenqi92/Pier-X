//! Database backend e2e integration tests (P3-12 Phase A).
//!
//! Gated behind `--features integration-tests` so the default
//! `cargo test -p pier-core` doesn't need MySQL / PostgreSQL on the
//! machine. CI sets the env vars below and starts the matching
//! services via the `services:` block in `.github/workflows/ci.yml`.
//!
//! Local invocation (after `docker run` of mysql:8 / postgres:16 —
//! see `docs/INTEGRATION_TESTS.md` for one-liner commands):
//!
//! ```sh
//! cargo test -p pier-core --features integration-tests \
//!   --test integration_db -- --test-threads=1
//! ```
//!
//! `--test-threads=1` matters: a single MySQL / Postgres instance
//! shared across parallel tests can deadlock on connection caps or
//! step on each other's session state.

#![cfg(feature = "integration-tests")]

use pier_core::services::mysql::{MysqlClient, MysqlConfig};
use pier_core::services::postgres::{PostgresClient, PostgresConfig};

// ─── MySQL fixtures ───────────────────────────────────────────────────

fn mysql_config_from_env() -> MysqlConfig {
    MysqlConfig {
        host: std::env::var("PIER_TEST_MYSQL_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        port: std::env::var("PIER_TEST_MYSQL_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3306),
        user: std::env::var("PIER_TEST_MYSQL_USER").unwrap_or_else(|_| "root".into()),
        password: std::env::var("PIER_TEST_MYSQL_PASSWORD")
            .unwrap_or_else(|_| "pier-x-test".into()),
        database: std::env::var("PIER_TEST_MYSQL_DB").ok(),
    }
}

#[test]
fn mysql_connect_then_list_databases() {
    let client = MysqlClient::connect_blocking(mysql_config_from_env())
        .expect("connect to MySQL — is the test server running?");
    let dbs = client
        .list_databases_blocking()
        .expect("list MySQL databases");
    // Server-supplied default databases — present on every fresh
    // MySQL 5.7+ install regardless of which DB the test creates.
    assert!(
        dbs.iter().any(|d| d == "mysql"),
        "expected `mysql` schema in default databases list, got: {dbs:?}"
    );
    assert!(
        dbs.iter().any(|d| d == "information_schema"),
        "expected `information_schema` in default databases list, got: {dbs:?}"
    );
}

#[test]
fn mysql_execute_simple_select() {
    let client = MysqlClient::connect_blocking(mysql_config_from_env()).expect("connect to MySQL");
    let result = client
        .execute_blocking("SELECT 1 AS n")
        .expect("execute SELECT 1");
    assert_eq!(result.columns, vec!["n"]);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0][0].as_deref(),
        Some("1"),
        "expected single row [\"1\"], got: {:?}",
        result.rows[0]
    );
    assert!(!result.truncated, "trivial SELECT should not be truncated");
}

#[test]
fn mysql_invalid_password_surfaces_auth_error() {
    let mut bad = mysql_config_from_env();
    bad.password = "definitely-not-the-password".into();
    let err =
        MysqlClient::connect_blocking(bad).expect_err("bad password must fail authentication");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("access")
            || msg.contains("auth")
            || msg.contains("password")
            || msg.contains("denied"),
        "expected auth-related error, got: {msg}"
    );
}

// ─── PostgreSQL fixtures ──────────────────────────────────────────────

fn postgres_config_from_env() -> PostgresConfig {
    PostgresConfig {
        host: std::env::var("PIER_TEST_PG_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        port: std::env::var("PIER_TEST_PG_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5432),
        user: std::env::var("PIER_TEST_PG_USER").unwrap_or_else(|_| "postgres".into()),
        password: std::env::var("PIER_TEST_PG_PASSWORD").unwrap_or_else(|_| "pier-x-test".into()),
        database: std::env::var("PIER_TEST_PG_DB").ok(),
    }
}

#[test]
fn postgres_connect_then_list_databases() {
    let client = PostgresClient::connect_blocking(postgres_config_from_env())
        .expect("connect to PostgreSQL — is the test server running?");
    let dbs = client.list_databases_blocking().expect("list PG databases");
    // `postgres` is the default maintenance DB on every install. It's
    // the safest single name to assert on without snowflaking on
    // template0/template1 visibility rules across PG versions.
    assert!(
        dbs.iter().any(|d| d == "postgres"),
        "expected `postgres` in default databases list, got: {dbs:?}"
    );
}

#[test]
fn postgres_execute_simple_select() {
    let client = PostgresClient::connect_blocking(postgres_config_from_env())
        .expect("connect to PostgreSQL");
    let result = client
        .execute_blocking("SELECT 1 AS n")
        .expect("execute SELECT 1");
    assert_eq!(result.columns, vec!["n"]);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0][0].as_deref(),
        Some("1"),
        "expected single row [\"1\"], got: {:?}",
        result.rows[0]
    );
    assert!(!result.truncated, "trivial SELECT should not be truncated");
}

#[test]
fn postgres_invalid_password_surfaces_auth_error() {
    let mut bad = postgres_config_from_env();
    bad.password = "definitely-not-the-password".into();
    let err =
        PostgresClient::connect_blocking(bad).expect_err("bad password must fail authentication");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("password") || msg.contains("auth") || msg.contains("authentication"),
        "expected auth-related error, got: {msg}"
    );
}
