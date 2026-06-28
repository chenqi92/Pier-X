//! Oracle / Dameng (达梦) clients over the remote host's own CLI.
//!
//! Unlike the wire-protocol clients, Oracle and Dameng have no pure-Rust
//! driver and their native clients are heavy, platform-specific installs.
//! Following the SQLite-remote model, we instead run the vendor CLI that
//! is already on the SSH host — `sqlplus` for Oracle, `disql` for
//! Dameng — over [`crate::ssh::SshSession::exec_command_with_stdin`] and
//! parse its CSV output. The CLI connects to the database from the remote
//! host; the desktop needs nothing installed.
//!
//! Output parsing assumes Oracle SQL*Plus `SET MARKUP CSV ON`. Dameng's
//! `disql` targets SQL*Plus compatibility, so the same script is used —
//! treat Dameng support as best-effort pending validation against a real
//! DM instance.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::ssh::SshSession;

/// Row cap parity with the other clients.
pub const MAX_ROWS: usize = 10_000;

/// Errors surfaced by the remote-CLI clients.
#[derive(Debug, thiserror::Error)]
pub enum RemoteCliError {
    /// SSH transport / exec failure.
    #[error("{0}")]
    Ssh(String),
    /// Caller supplied invalid config.
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

/// Result alias for remote-CLI ops.
pub type Result<T, E = RemoteCliError> = std::result::Result<T, E>;

/// Which vendor CLI to drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CliKind {
    /// Oracle `sqlplus`.
    Oracle,
    /// Dameng `disql`.
    Dameng,
}

/// Connection parameters handed to the remote CLI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliConn {
    /// Database host as reachable FROM the SSH host.
    pub host: String,
    /// Database port (Oracle 1521 / Dameng 5236).
    pub port: u16,
    /// Database user.
    pub user: String,
    /// Database password.
    pub password: String,
    /// Oracle service name / SID. Unused for Dameng.
    pub service: String,
}

/// Stringified query result from the remote CLI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteCliResult {
    /// Column names from the CSV header.
    pub columns: Vec<String>,
    /// Rows, capped at [`MAX_ROWS`].
    pub rows: Vec<Vec<String>>,
    /// True if the row cap was hit.
    pub truncated: bool,
    /// Vendor error text (ORA-…, DM error) when the statement failed.
    pub error: Option<String>,
    /// Wall-clock execution time.
    pub elapsed_ms: u64,
}

/// Run one statement through the remote vendor CLI and parse its CSV.
pub async fn query(
    session: &SshSession,
    kind: CliKind,
    conn: &CliConn,
    sql: &str,
) -> Result<RemoteCliResult> {
    if conn.host.trim().is_empty() {
        return Err(RemoteCliError::InvalidConfig("empty host".into()));
    }
    if conn.user.trim().is_empty() {
        return Err(RemoteCliError::InvalidConfig("empty user".into()));
    }
    let trimmed = sql.trim().trim_end_matches(';');
    if trimmed.is_empty() {
        return Err(RemoteCliError::InvalidConfig("empty SQL".into()));
    }

    let (binary, conn_arg) = match kind {
        CliKind::Oracle => (
            "sqlplus",
            format!(
                "{}/{}@//{}:{}/{}",
                conn.user, conn.password, conn.host, conn.port, conn.service
            ),
        ),
        CliKind::Dameng => (
            "disql",
            format!(
                "{}/{}@{}:{}",
                conn.user, conn.password, conn.host, conn.port
            ),
        ),
    };

    // SQL*Plus-style formatting that emits a single quoted-CSV block.
    let script = format!(
        "SET MARKUP CSV ON\nSET FEEDBACK OFF\nSET PAGESIZE 50000\nSET LINESIZE 32767\nSET TRIMSPOOL ON\nSET HEADING ON\n{trimmed};\nEXIT;\n"
    );

    let flags = match kind {
        CliKind::Oracle => "-S -L",
        // disql reads the connection as its first arg and SQL from stdin.
        CliKind::Dameng => "",
    };
    let command = if flags.is_empty() {
        format!("{binary} {}", shell_single_quote(&conn_arg))
    } else {
        format!("{binary} {flags} {}", shell_single_quote(&conn_arg))
    };

    let start = Instant::now();
    let (exit, stdout) = session
        .exec_command_with_stdin(&command, &script)
        .await
        .map_err(|e| RemoteCliError::Ssh(e.to_string()))?;
    let elapsed_ms = start.elapsed().as_millis() as u64;

    if let Some(err) = detect_error(&stdout, exit) {
        return Ok(RemoteCliResult {
            columns: Vec::new(),
            rows: Vec::new(),
            truncated: false,
            error: Some(err),
            elapsed_ms,
        });
    }

    let mut records = parse_csv(&stdout);
    let columns = if records.is_empty() {
        Vec::new()
    } else {
        records.remove(0)
    };
    let mut truncated = false;
    if records.len() > MAX_ROWS {
        records.truncate(MAX_ROWS);
        truncated = true;
    }

    Ok(RemoteCliResult {
        columns,
        rows: records,
        truncated,
        error: None,
        elapsed_ms,
    })
}

/// Blocking wrapper for [`query`].
pub fn query_blocking(
    session: &SshSession,
    kind: CliKind,
    conn: &CliConn,
    sql: &str,
) -> Result<RemoteCliResult> {
    crate::ssh::runtime::shared().block_on(query(session, kind, conn, sql))
}

/// Detect a vendor error in the CLI output. Returns the offending lines.
fn detect_error(stdout: &str, exit: i32) -> Option<String> {
    let markers = ["ORA-", "SP2-", "PLS-", "DPI-", "TNS-", "DIA-"];
    let hit: Vec<&str> = stdout
        .lines()
        .filter(|l| markers.iter().any(|m| l.trim_start().starts_with(m)))
        .collect();
    if !hit.is_empty() {
        return Some(hit.join("\n"));
    }
    if exit != 0 {
        let text = stdout.trim();
        if text.is_empty() {
            return Some(format!("CLI exited with status {exit}"));
        }
        return Some(text.lines().take(8).collect::<Vec<_>>().join("\n"));
    }
    None
}

/// Wrap a value in single quotes for a POSIX shell, escaping embedded
/// single quotes as `'\''`.
fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Minimal RFC-4180 CSV parser: handles `"`-quoted fields with doubled
/// inner quotes and embedded commas / newlines. Skips blank trailing
/// records that SQL*Plus appends.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut field = String::new();
    let mut record: Vec<String> = Vec::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    let mut started = false;

    while let Some(c) = chars.next() {
        started = true;
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => in_quotes = true,
            ',' => {
                record.push(std::mem::take(&mut field));
            }
            '\r' => {}
            '\n' => {
                record.push(std::mem::take(&mut field));
                push_record(&mut records, std::mem::take(&mut record));
            }
            _ => field.push(c),
        }
    }
    if started && (!field.is_empty() || !record.is_empty()) {
        record.push(field);
        push_record(&mut records, record);
    }
    records
}

fn push_record(records: &mut Vec<Vec<String>>, record: Vec<String>) {
    // Drop the all-empty rows SQL*Plus emits between/after results.
    if record.iter().all(|f| f.is_empty()) {
        return;
    }
    records.push(record);
}
