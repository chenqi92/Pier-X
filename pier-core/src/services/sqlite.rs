//! SQLite client for local database inspection.
//!
//! Opens a `.db` / `.sqlite` file and provides schema introspection
//! (tables, columns) and arbitrary SQL execution. Unlike MySQL/Redis
//! which connect over TCP, SQLite operates directly on a local file.
//!
//! Uses the `sqlite3` CLI subprocess rather than linking libsqlite —
//! keeps the build dependency-free and matches how the user's own
//! sqlite3 is configured (extensions, etc.).

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::process_util::configure_background_command;

/// Errors surfaced by the SQLite client.
#[allow(missing_docs)]
#[derive(Debug, thiserror::Error)]
pub enum SqliteError {
    #[error("sqlite: {0}")]
    Command(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

/// Query result shape — mirrors the MySQL QueryResult for UI reuse.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub affected_rows: i64,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

/// SQLite client bound to a database file.
pub struct SqliteClient {
    db_path: PathBuf,
}

impl SqliteClient {
    /// Open a SQLite database file.
    pub fn open(path: &str) -> Result<Self, SqliteError> {
        if path.is_empty() {
            return Err(SqliteError::InvalidPath("empty path".into()));
        }
        let p = PathBuf::from(path);
        if !p.exists() {
            return Err(SqliteError::InvalidPath(format!(
                "file not found: {}",
                path
            )));
        }
        // Verify it's a valid sqlite database
        let mut command = Command::new("sqlite3");
        command.arg(path);
        command.arg("SELECT sqlite_version();");
        configure_background_command(&mut command);
        let output = command
            .output()
            .map_err(|e| SqliteError::Command(format!("sqlite3 not found: {}", e)))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(SqliteError::Command(format!(
                "not a valid database: {}",
                err
            )));
        }
        Ok(Self { db_path: p })
    }

    /// List tables in the database.
    pub fn list_tables(&self) -> Result<Vec<String>, SqliteError> {
        let output = self.sqlite(&[".tables"])?;
        Ok(output
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect())
    }

    /// Get column info for a table.
    pub fn table_columns(&self, table: &str) -> Result<Vec<ColumnInfo>, SqliteError> {
        let output = self.sqlite(&[
            "-header",
            "-separator",
            "\x1f",
            &format!("PRAGMA table_info('{}');", table.replace('\'', "''")),
        ])?;
        let mut cols = Vec::new();
        for line in output.lines().skip(1) {
            // Fields: cid, name, type, notnull, dflt_value, pk
            let parts: Vec<&str> = line.split('\x1f').collect();
            if parts.len() >= 3 {
                cols.push(ColumnInfo {
                    name: parts[1].to_string(),
                    col_type: parts[2].to_string(),
                    not_null: parts.get(3).map(|s| *s == "1").unwrap_or(false),
                    primary_key: parts.get(5).map(|s| *s != "0").unwrap_or(false),
                });
            }
        }
        Ok(cols)
    }

    /// Execute an arbitrary SQL statement.
    pub fn execute(&self, sql: &str) -> SqliteQueryResult {
        let start = Instant::now();
        let result = self.sqlite(&["-header", "-separator", "\x1f", sql]);
        let elapsed = start.elapsed().as_millis() as u64;

        match result {
            Ok(output) => {
                let mut lines = output.lines();
                let header = match lines.next() {
                    Some(h) if !h.is_empty() => h,
                    _ => {
                        return SqliteQueryResult {
                            columns: vec![],
                            rows: vec![],
                            affected_rows: 0,
                            elapsed_ms: elapsed,
                            error: None,
                        }
                    }
                };
                let columns: Vec<String> = header.split('\x1f').map(|s| s.to_string()).collect();
                let mut rows = Vec::new();
                for line in lines {
                    if line.is_empty() {
                        continue;
                    }
                    let row: Vec<String> = line.split('\x1f').map(|s| s.to_string()).collect();
                    rows.push(row);
                }
                SqliteQueryResult {
                    columns,
                    rows,
                    affected_rows: 0,
                    elapsed_ms: elapsed,
                    error: None,
                }
            }
            Err(e) => SqliteQueryResult {
                columns: vec![],
                rows: vec![],
                affected_rows: 0,
                elapsed_ms: elapsed,
                error: Some(e.to_string()),
            },
        }
    }

    fn sqlite(&self, args: &[&str]) -> Result<String, SqliteError> {
        let mut cmd = Command::new("sqlite3");
        cmd.arg(self.db_path.to_str().unwrap_or(""));
        cmd.args(args);
        configure_background_command(&mut cmd);
        let output = cmd
            .output()
            .map_err(|e| SqliteError::Command(format!("sqlite3: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SqliteError::Command(stderr.trim().to_string()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// Column metadata from PRAGMA table_info.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub col_type: String,
    pub not_null: bool,
    pub primary_key: bool,
}

unsafe impl Send for SqliteClient {}
unsafe impl Sync for SqliteClient {}
