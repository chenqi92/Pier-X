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
            return Err(SqliteError::InvalidPath(format!("file not found: {path}")));
        }
        // Verify it's a valid sqlite database
        let mut command = Command::new("sqlite3");
        command.arg(path);
        command.arg("SELECT sqlite_version();");
        configure_background_command(&mut command);
        let output = command
            .output()
            .map_err(|e| SqliteError::Command(format!("sqlite3 not found: {e}")))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(SqliteError::Command(format!("not a valid database: {err}")));
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

    /// List indexes attached to a table — driven by `PRAGMA
    /// index_list` plus a follow-up `PRAGMA index_info` per index
    /// to pull ordered column names.
    pub fn table_indexes(&self, table: &str) -> Result<Vec<IndexInfo>, SqliteError> {
        let escaped = table.replace('\'', "''");
        let listing = self.sqlite(&[
            "-header",
            "-separator",
            "\x1f",
            &format!("PRAGMA index_list('{escaped}');"),
        ])?;
        let mut out: Vec<IndexInfo> = Vec::new();
        for line in listing.lines().skip(1) {
            // Fields: seq, name, unique, origin, partial
            let parts: Vec<&str> = line.split('\x1f').collect();
            if parts.len() < 4 {
                continue;
            }
            let name = parts[1].to_string();
            let unique = parts[2] == "1";
            let origin = parts[3].to_string();
            let cols = self
                .sqlite(&[
                    "-header",
                    "-separator",
                    "\x1f",
                    &format!("PRAGMA index_info('{}');", name.replace('\'', "''")),
                ])
                .unwrap_or_default();
            let mut columns: Vec<String> = Vec::new();
            for col_line in cols.lines().skip(1) {
                // Fields: seqno, cid, name
                let cp: Vec<&str> = col_line.split('\x1f').collect();
                if cp.len() >= 3 {
                    columns.push(cp[2].to_string());
                }
            }
            out.push(IndexInfo {
                name,
                unique,
                origin,
                columns,
            });
        }
        Ok(out)
    }

    /// List triggers whose `tbl_name` is the given table — pulled
    /// from `sqlite_master`. The full DDL travels along so the UI
    /// can show it as a hover tooltip without another round-trip.
    pub fn table_triggers(&self, table: &str) -> Result<Vec<TriggerInfo>, SqliteError> {
        let escaped = table.replace('\'', "''");
        let output = self.sqlite(&[
            "-header",
            "-separator",
            "\x1f",
            &format!(
                "SELECT name, IFNULL(sql, '') FROM sqlite_master \
                 WHERE type='trigger' AND tbl_name='{escaped}' ORDER BY name;"
            ),
        ])?;
        let mut out = Vec::new();
        for line in output.lines().skip(1) {
            let parts: Vec<&str> = line.split('\x1f').collect();
            if parts.len() < 2 {
                continue;
            }
            let name = parts[0].to_string();
            let sql = parts[1..].join("\x1f");
            let event = parse_trigger_event(&sql);
            out.push(TriggerInfo { name, event, sql });
        }
        Ok(out)
    }

    /// Size on disk of the open `.db` file, in bytes. Returns 0
    /// when `stat` fails (deleted under us, permission flip, etc.) —
    /// the UI treats 0 as "unknown".
    pub fn file_size(&self) -> u64 {
        std::fs::metadata(&self.db_path)
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// Execute a script that may contain multiple semicolon-separated
    /// statements. Returns one [`SqliteQueryResult`] per statement
    /// with per-statement timing. `BEGIN`/`COMMIT`/`SAVEPOINT` come
    /// back as zero-column results — the caller should treat them
    /// as ack-only entries.
    pub fn execute_script(&self, sql: &str) -> Vec<SqliteQueryResult> {
        let stmts = split_sql_statements(sql);
        if stmts.len() <= 1 {
            return vec![self.execute(stmts.first().map(|s| s.as_str()).unwrap_or(""))];
        }
        stmts.into_iter().map(|s| self.execute(&s)).collect()
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
            .map_err(|e| SqliteError::Command(format!("sqlite3: {e}")))?;

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

/// One row of `PRAGMA index_list` joined with the column list
/// from `PRAGMA index_info`. `origin` is the SQLite-specific
/// origin marker — `c` (CREATE INDEX), `u` (UNIQUE constraint),
/// or `pk` (PRIMARY KEY).
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    pub name: String,
    pub unique: bool,
    pub origin: String,
    pub columns: Vec<String>,
}

/// One row of `sqlite_master` filtered to triggers. `event` is
/// derived from the trigger DDL and follows the
/// `BEFORE INSERT` / `AFTER UPDATE` / `INSTEAD OF DELETE` shape;
/// `sql` carries the full DDL for the hover tooltip.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerInfo {
    pub name: String,
    pub event: String,
    pub sql: String,
}

/// Pull the human-readable event ("BEFORE INSERT" etc.) from a
/// `CREATE TRIGGER` statement. We scan token-by-token so leading
/// `IF NOT EXISTS` and the trigger name don't get mistaken for
/// the timing keyword.
fn parse_trigger_event(sql: &str) -> String {
    let upper = sql.to_uppercase();
    let timing = if upper.contains(" INSTEAD OF ") {
        "INSTEAD OF"
    } else if upper.contains(" BEFORE ") {
        "BEFORE"
    } else if upper.contains(" AFTER ") {
        "AFTER"
    } else {
        ""
    };
    let action = if upper.contains(" INSERT ") {
        "INSERT"
    } else if upper.contains(" UPDATE ") {
        "UPDATE"
    } else if upper.contains(" DELETE ") {
        "DELETE"
    } else {
        ""
    };
    match (timing.is_empty(), action.is_empty()) {
        (false, false) => format!("{timing} {action}"),
        (false, true) => timing.to_string(),
        (true, false) => action.to_string(),
        _ => String::new(),
    }
}

/// Split a SQL script on top-level semicolons. Quotes (`'`, `"`,
/// `` ` ``), line comments (`--`), and block comments (`/* */`)
/// are tracked so semicolons inside them don't terminate a
/// statement. Empty statements are dropped. This is a deliberate
/// subset of SQLite's lexer — good enough for the panel's "run
/// this script" affordance.
fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut in_str: Option<u8> = None;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    while i < bytes.len() {
        let c = bytes[i];
        let next = bytes.get(i + 1).copied();
        if in_line_comment {
            current.push(c as char);
            if c == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            current.push(c as char);
            if c == b'*' && next == Some(b'/') {
                current.push('/');
                i += 2;
                in_block_comment = false;
                continue;
            }
            i += 1;
            continue;
        }
        if let Some(q) = in_str {
            current.push(c as char);
            if c == q {
                // SQL doubles the quote to escape; if next == q, keep going.
                if next == Some(q) {
                    current.push(q as char);
                    i += 2;
                    continue;
                }
                in_str = None;
            }
            i += 1;
            continue;
        }
        if c == b'-' && next == Some(b'-') {
            in_line_comment = true;
            current.push_str("--");
            i += 2;
            continue;
        }
        if c == b'/' && next == Some(b'*') {
            in_block_comment = true;
            current.push_str("/*");
            i += 2;
            continue;
        }
        if c == b'\'' || c == b'"' || c == b'`' {
            in_str = Some(c);
            current.push(c as char);
            i += 1;
            continue;
        }
        if c == b';' {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                out.push(trimmed);
            }
            current.clear();
            i += 1;
            continue;
        }
        current.push(c as char);
        i += 1;
    }
    let tail = current.trim().to_string();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

unsafe impl Send for SqliteClient {}
unsafe impl Sync for SqliteClient {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_quoted_semicolons() {
        let stmts = split_sql_statements("SELECT ';' FROM t; INSERT INTO u VALUES (';');");
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0], "SELECT ';' FROM t");
        assert_eq!(stmts[1], "INSERT INTO u VALUES (';')");
    }

    #[test]
    fn split_handles_block_and_line_comments() {
        let stmts = split_sql_statements(
            "/* leading; */ SELECT 1;\n-- trailing; comment\nSELECT 2;",
        );
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].ends_with("SELECT 1"));
        assert!(stmts[1].ends_with("SELECT 2"));
    }

    #[test]
    fn split_handles_escaped_quotes() {
        let stmts = split_sql_statements("SELECT 'it''s ok'; SELECT 2;");
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0], "SELECT 'it''s ok'");
    }

    #[test]
    fn parse_trigger_event_extracts_timing_and_action() {
        let s = "CREATE TRIGGER IF NOT EXISTS t1 AFTER INSERT ON foo BEGIN SELECT 1; END";
        assert_eq!(parse_trigger_event(s), "AFTER INSERT");
    }

    #[test]
    fn parse_trigger_event_handles_instead_of() {
        let s = "CREATE TRIGGER t2 INSTEAD OF DELETE ON v BEGIN SELECT 1; END";
        assert_eq!(parse_trigger_event(s), "INSTEAD OF DELETE");
    }
}
