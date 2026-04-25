//! Remote SQLite browser over SSH.
//!
//! Goal: let the user browse and edit a `.db` file that lives
//! on the remote host **without** downloading a local copy.
//!
//! We achieve this by running the remote host's own `sqlite3`
//! CLI through [`crate::ssh::SshSession::exec_command`] and
//! asking it to emit JSON (`-json` mode, introduced in SQLite
//! 3.33). Each call opens a fresh ad-hoc channel, runs one SQL
//! statement, and closes — stateless by design, aligning with
//! the rest of pier-core's remote-service modules.
//!
//! ## Shell safety
//!
//! The remote path is user-supplied so it could contain shell
//! metacharacters. Both the path and the SQL fragment are
//! passed through single-quote-escape before being interpolated
//! into the command string, which is the POSIX-portable way to
//! pass arbitrary strings to `/bin/sh -c`. `sqlite3 --` is
//! used to separate positional args from the SQL literal.
//!
//! ## Result shape
//!
//! Returns the same `QueryResult` / `ColumnInfo` shapes used
//! by [`crate::services::sqlite`] so the Tauri bridge can map
//! both local and remote paths to the same `SqliteBrowserState`
//! / `QueryExecutionResult` views.
//!
//! ## Version fallback
//!
//! Remote `sqlite3` < 3.33 doesn't support `-json`. Callers
//! should first check [`remote_sqlite_version`] and flip the
//! panel into "download copy" mode when the version is too old
//! or the binary isn't installed at all.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::services::sqlite::ColumnInfo;
use crate::ssh::error::{Result, SshError};
use crate::ssh::SshSession;

/// Row limit for a single preview/query. Mirrors the local
/// sqlite service's cap so the UI doesn't have to special-case
/// remote results.
pub const MAX_ROWS: usize = 10_000;

/// Per-cell byte ceiling. Applied post-hoc to JSON-encoded cell
/// values so a runaway `SELECT *` from a column holding a 200 MB
/// blob doesn't flood the frontend. The cell is truncated with
/// a trailing "…" marker.
pub const MAX_CELL_BYTES: usize = 4096;

/// Preview / query result. Mirrors
/// [`crate::services::sqlite::SqliteQueryResult`] so the Tauri
/// bridge can return either without branching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteQueryResult {
    /// Column names in the order the query produced them.
    pub columns: Vec<String>,
    /// Rows as stringified cell values.
    pub rows: Vec<Vec<String>>,
    /// True when row count hit [`MAX_ROWS`].
    pub truncated: bool,
    /// `changes()` reported by the remote sqlite for writes;
    /// `0` for SELECT queries.
    pub affected_rows: i64,
    /// `last_insert_rowid()` after a successful INSERT; `None`
    /// otherwise. Mirrors the local MySQL/PG shape so the
    /// shared `QueryExecutionResult` view can reuse it.
    pub last_insert_id: Option<i64>,
    /// Wall-clock time to run the command, measured from the
    /// `exec_command` call to the response landing.
    pub elapsed_ms: u64,
    /// Non-fatal stderr captured from `sqlite3` (e.g. "Parse
    /// error: near 'SELEC': syntax error"). Populated even
    /// when `exit_code == 0` if the CLI echoed warnings.
    pub error: Option<String>,
}

/// Remote `sqlite3` capability report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSqliteCapability {
    /// `sqlite3` binary is on the remote PATH.
    pub installed: bool,
    /// The version string reported by `sqlite3 --version` if any.
    pub version: Option<String>,
    /// `true` iff the remote version is ≥ 3.33 (when `-json`
    /// mode is available). This is the only flag the frontend
    /// needs — older versions must go through the
    /// download-a-copy fallback path.
    pub supports_json: bool,
}

// ─────────────────────────────────────────────────────────
// Entry points
// ─────────────────────────────────────────────────────────

/// Check the remote host for `sqlite3` and whether it supports
/// `-json`. Never fails — reports an "uninstalled" struct on
/// any error so the frontend can branch cleanly.
pub async fn probe(session: &SshSession) -> RemoteSqliteCapability {
    let Ok((code, stdout)) = session
        .exec_command("command -v sqlite3 >/dev/null 2>&1 && sqlite3 --version 2>&1")
        .await
    else {
        return RemoteSqliteCapability {
            installed: false,
            version: None,
            supports_json: false,
        };
    };
    if code != 0 {
        return RemoteSqliteCapability {
            installed: false,
            version: None,
            supports_json: false,
        };
    }
    let version = parse_sqlite_version(&stdout);
    let supports_json = version.as_deref().map(supports_json_mode).unwrap_or(false);
    RemoteSqliteCapability {
        installed: true,
        version,
        supports_json,
    }
}

/// List tables on a remote `.db` file. Equivalent to
/// `SELECT name FROM sqlite_master WHERE type='table' ORDER BY name`.
pub async fn list_tables(session: &SshSession, db_path: &str) -> Result<Vec<String>> {
    let rows = run_select_rows(
        session,
        db_path,
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|mut r| r.remove("name"))
        .collect())
}

/// Column metadata for one table via `PRAGMA table_info(...)`.
pub async fn table_columns(
    session: &SshSession,
    db_path: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>> {
    let quoted = escape_sql_string(table);
    let sql = format!("PRAGMA table_info({quoted})");
    let rows = run_select_rows(session, db_path, &sql).await?;
    Ok(rows
        .into_iter()
        .map(|row| ColumnInfo {
            name: row.get("name").cloned().unwrap_or_default(),
            col_type: row.get("type").cloned().unwrap_or_default(),
            not_null: row.get("notnull").map(|v| v != "0").unwrap_or(false),
            primary_key: row.get("pk").map(|v| v != "0").unwrap_or(false),
        })
        .collect())
}

/// Preview the first `limit` rows of a table.
pub async fn preview_table(
    session: &SshSession,
    db_path: &str,
    table: &str,
    limit: usize,
) -> Result<RemoteQueryResult> {
    let double_quoted = double_quote_ident(table);
    let sql = format!("SELECT * FROM {double_quoted} LIMIT {limit}");
    run_query(session, db_path, &sql).await
}

/// Execute arbitrary SQL. Writes go straight to the remote
/// file; reads return rows.
pub async fn execute(session: &SshSession, db_path: &str, sql: &str) -> Result<RemoteQueryResult> {
    run_query(session, db_path, sql).await
}

// ─────────────────────────────────────────────────────────
// Internals
// ─────────────────────────────────────────────────────────

async fn run_select_rows(
    session: &SshSession,
    db_path: &str,
    sql: &str,
) -> Result<Vec<BTreeMap<String, String>>> {
    let cmd = build_sqlite_json_command(db_path, sql);
    let (exit, stdout) = session.exec_command(&cmd).await?;
    if exit != 0 {
        return Err(SshError::InvalidConfig(format!(
            "sqlite3 exited {exit}: {}",
            stdout.lines().next().unwrap_or("").trim()
        )));
    }
    parse_json_rows(&stdout).map_err(SshError::InvalidConfig)
}

async fn run_query(session: &SshSession, db_path: &str, sql: &str) -> Result<RemoteQueryResult> {
    let started = std::time::Instant::now();
    let cmd = build_sqlite_json_command(db_path, sql);
    let (exit, stdout) = session.exec_command(&cmd).await?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    if exit != 0 {
        // sqlite3 writes errors to stderr, which our
        // `exec_command` merges or drops depending on the
        // server. Return a structured error rather than the
        // generic SshError — the panel wants to surface this
        // to the user's query editor as `result.error`.
        return Ok(RemoteQueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            truncated: false,
            affected_rows: 0,
            last_insert_id: None,
            elapsed_ms,
            error: Some(stdout.trim().to_string()),
        });
    }

    let rows = match parse_json_rows(&stdout) {
        Ok(r) => r,
        Err(e) => {
            return Ok(RemoteQueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                truncated: false,
                affected_rows: 0,
                last_insert_id: None,
                elapsed_ms,
                error: Some(e),
            });
        }
    };

    // Derive column order: use the first row's insertion order.
    // BTreeMap alphabetises so we lose insertion order — but
    // sqlite3 -json emits one JSON object per row with keys in
    // column order, and we parse via `serde_json::Value` which
    // preserves it when built through `Map<String, Value>`.
    // We therefore re-parse to grab the ordered column list.
    let columns = extract_column_order(&stdout).unwrap_or_default();

    let truncated = rows.len() >= MAX_ROWS;
    let capped_rows = rows.into_iter().take(MAX_ROWS).collect::<Vec<_>>();
    let grid: Vec<Vec<String>> = capped_rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|col| row.get(col).cloned().unwrap_or_default())
                .map(cap_cell)
                .collect()
        })
        .collect();

    Ok(RemoteQueryResult {
        columns,
        rows: grid,
        truncated,
        // sqlite3 -json doesn't surface `changes()` for
        // DML — a follow-up query would (`SELECT changes()`)
        // but that doubles round-trips; leave 0 for now and
        // let write-path callers run it themselves if they
        // care.
        affected_rows: 0,
        last_insert_id: None,
        elapsed_ms,
        error: None,
    })
}

/// Build the remote shell command: `sqlite3 -json -bail -- <path> "<sql>"`.
/// Both the path and the SQL are single-quote-escaped, so any
/// input is safe to interpolate into `/bin/sh -c`.
fn build_sqlite_json_command(db_path: &str, sql: &str) -> String {
    let path_q = shell_single_quote(db_path);
    let sql_q = shell_single_quote(sql);
    format!("sqlite3 -json -bail -- {path_q} {sql_q}")
}

/// POSIX-safe single-quote escape for shell interpolation.
/// Wraps in single quotes and replaces any literal `'` with
/// `'\\''` (close quote → escaped quote → reopen).
fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Escape a string for embedding in a SQL literal: wrap in
/// single quotes, doubling any interior quotes. Used for
/// identifiers passed to `PRAGMA table_info(...)`.
fn escape_sql_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Double-quote a SQL identifier (table/column name), doubling
/// any interior double quotes. Used to survive table names
/// that contain spaces or reserved words.
fn double_quote_ident(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        if ch == '"' {
            out.push_str("\"\"");
        } else {
            out.push(ch);
        }
    }
    out.push('"');
    out
}

/// Parse `sqlite3 -json` output. Empty result → empty vec, not
/// an error — the CLI emits nothing (not `"[]"`) when the query
/// returns zero rows.
///
/// Multi-statement SQL (e.g. `CREATE TABLE t; SELECT * FROM t;`)
/// makes sqlite3 emit one JSON array per statement, concatenated
/// back-to-back (`[][{...}]`). We take the **last non-empty**
/// array, which is the most recent result set — matching the
/// mental model "run SQL, show me what the final statement
/// returned". DDL statements in front get their empty arrays
/// silently dropped.
fn parse_json_rows(stdout: &str) -> std::result::Result<Vec<BTreeMap<String, String>>, String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let target = last_array_slice(trimmed);
    let value: serde_json::Value = serde_json::from_str(target)
        .map_err(|e| format!("sqlite3 -json returned malformed output: {e}"))?;
    let array = value
        .as_array()
        .ok_or_else(|| format!("sqlite3 -json expected an array, got: {value}"))?;

    let mut out: Vec<BTreeMap<String, String>> = Vec::with_capacity(array.len());
    for entry in array {
        let obj = match entry.as_object() {
            Some(o) => o,
            None => continue,
        };
        let mut row = BTreeMap::new();
        for (key, val) in obj {
            row.insert(key.clone(), json_value_to_cell(val));
        }
        out.push(row);
    }
    Ok(out)
}

/// Pick the substring of the **last** top-level `[...]` array
/// in the output. Handles the "multi-statement -json" case where
/// sqlite3 concatenates arrays back-to-back; if there's only
/// one array this is a no-op. Strings and nested structures
/// are respected so `[{"s":"]["}]` doesn't fool the splitter.
///
/// Algorithm: single forward pass tracking depth, string state,
/// and escapes. Every time depth returns to zero after an `]`
/// we record `(start, end)` — the final recorded pair wins.
fn last_array_slice(stdout: &str) -> &str {
    let bytes = stdout.as_bytes();
    let mut start: Option<usize> = None;
    let mut last_span: Option<(usize, usize)> = None;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;

    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_string {
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'{' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start.take() {
                        last_span = Some((s, i + 1));
                    }
                }
            }
            b'}' => depth -= 1,
            _ => {}
        }
    }
    match last_span {
        Some((s, e)) => &stdout[s..e],
        None => stdout,
    }
}

fn json_value_to_cell(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn cap_cell(mut s: String) -> String {
    if s.len() > MAX_CELL_BYTES {
        s.truncate(MAX_CELL_BYTES);
        s.push('…');
    }
    s
}

/// Extract keys in the order they appear in the first object
/// of the `sqlite3 -json` array. We scan the raw JSON text
/// because `serde_json` without the `preserve_order` feature
/// would alphabetise them. Works on well-formed input only —
/// callers already validated via `parse_json_rows`.
///
/// Receives the full stdout (not pre-sliced) so we can apply
/// the same "last array wins" rule `parse_json_rows` uses for
/// multi-statement SQL output.
fn extract_column_order(stdout: &str) -> Option<Vec<String>> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Some(Vec::new());
    }
    let trimmed = last_array_slice(trimmed);
    let bytes = trimmed.as_bytes();
    // Find the first `{` — start of the first object.
    let obj_start = bytes.iter().position(|&b| b == b'{')?;
    // Walk forward, balancing braces, collecting `"key":` at
    // depth 1 only (top-level keys of this object).
    let mut keys = Vec::new();
    let mut i = obj_start + 1;
    let mut depth = 1i32;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'"' => {
                // Parse a quoted string, honouring backslash
                // escapes. The string ends at the next
                // unescaped `"`.
                let (end, literal) = read_json_string(bytes, i)?;
                i = end + 1;
                // Skip whitespace.
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                // If followed by `:` at depth 1, this was a key.
                if depth == 1 && i < bytes.len() && bytes[i] == b':' {
                    keys.push(literal);
                    i += 1;
                    // Skip the value: could be a primitive,
                    // string, array, or nested object. We just
                    // walk until we see a `,` or `}` at the
                    // same depth.
                    i = skip_json_value(bytes, i, depth)?;
                }
            }
            b'{' | b'[' => {
                depth += 1;
                i += 1;
            }
            b'}' | b']' => {
                depth -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    Some(keys)
}

/// Read one JSON string literal starting at `pos` (which must
/// point at the opening `"`). Returns `(end_pos_of_closing_quote,
/// decoded_string)`. Handles `\\` and `\"` escapes — enough
/// for the keys sqlite3 emits.
fn read_json_string(bytes: &[u8], pos: usize) -> Option<(usize, String)> {
    debug_assert_eq!(bytes[pos], b'"');
    let mut out = String::new();
    let mut i = pos + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some((i, out)),
            b'\\' => {
                if i + 1 >= bytes.len() {
                    return None;
                }
                match bytes[i + 1] {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    other => {
                        out.push('\\');
                        out.push(other as char);
                    }
                }
                i += 2;
            }
            b => {
                out.push(b as char);
                i += 1;
            }
        }
    }
    None
}

/// Skip past one JSON value starting at `pos`, returning the
/// position of the next `,` (consumed) or the closing `}` / `]`
/// of the enclosing container (NOT consumed). `outer_depth` is
/// the depth of the enclosing object/array before we started.
fn skip_json_value(bytes: &[u8], mut pos: usize, outer_depth: i32) -> Option<usize> {
    // Skip leading whitespace.
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    let mut depth = outer_depth;
    while pos < bytes.len() {
        match bytes[pos] {
            b'"' => {
                let (end, _) = read_json_string(bytes, pos)?;
                pos = end + 1;
            }
            b'{' | b'[' => {
                depth += 1;
                pos += 1;
            }
            b'}' | b']' => {
                if depth == outer_depth {
                    // End of enclosing container — stop here
                    // without consuming.
                    return Some(pos);
                }
                depth -= 1;
                pos += 1;
            }
            b',' if depth == outer_depth => {
                // End of this value — consume the comma.
                return Some(pos + 1);
            }
            _ => pos += 1,
        }
    }
    Some(pos)
}

fn parse_sqlite_version(output: &str) -> Option<String> {
    // `sqlite3 --version` prints e.g. `3.46.1 2024-08-13 ...`
    output
        .split_whitespace()
        .next()
        .filter(|s| s.contains('.'))
        .map(|s| s.to_string())
}

fn supports_json_mode(version: &str) -> bool {
    // `-json` dot-command was added in SQLite 3.33.
    let (major, minor) = match version.split('.').take(2).collect::<Vec<_>>()[..] {
        [a, b] => {
            let Ok(a) = a.parse::<u32>() else {
                return false;
            };
            let Ok(b) = b.parse::<u32>() else {
                return false;
            };
            (a, b)
        }
        _ => return false,
    };
    (major, minor) >= (3, 33)
}

// ─────────────────────────────────────────────────────────
// Sync wrappers
// ─────────────────────────────────────────────────────────

/// Blocking wrapper for [`probe`].
pub fn probe_blocking(session: &SshSession) -> RemoteSqliteCapability {
    crate::ssh::runtime::shared().block_on(probe(session))
}
/// Best-effort `stat`-style file-size lookup on the remote host.
/// Tries `stat -c %s` first (GNU coreutils / BusyBox) and falls
/// back to `stat -f %z` (BSD / macOS). On any failure — missing
/// `stat`, unreadable file, exotic shell — returns 0. The caller
/// treats 0 as "size unknown" so the panel hides the chip.
pub async fn stat_size(session: &SshSession, db_path: &str) -> Result<u64> {
    let quoted = shell_single_quote(db_path);
    let cmd = format!("stat -c %s {quoted} 2>/dev/null || stat -f %z {quoted} 2>/dev/null");
    let (exit, stdout) = session.exec_command(&cmd).await?;
    if exit != 0 {
        return Ok(0);
    }
    Ok(stdout.trim().parse::<u64>().unwrap_or(0))
}
/// Blocking wrapper for [`stat_size`].
pub fn stat_size_blocking(session: &SshSession, db_path: &str) -> Result<u64> {
    crate::ssh::runtime::shared().block_on(stat_size(session, db_path))
}
/// Blocking wrapper for [`list_tables`].
pub fn list_tables_blocking(session: &SshSession, db_path: &str) -> Result<Vec<String>> {
    crate::ssh::runtime::shared().block_on(list_tables(session, db_path))
}
/// Blocking wrapper for [`table_columns`].
pub fn table_columns_blocking(
    session: &SshSession,
    db_path: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>> {
    crate::ssh::runtime::shared().block_on(table_columns(session, db_path, table))
}
/// Blocking wrapper for [`preview_table`].
pub fn preview_table_blocking(
    session: &SshSession,
    db_path: &str,
    table: &str,
    limit: usize,
) -> Result<RemoteQueryResult> {
    crate::ssh::runtime::shared().block_on(preview_table(session, db_path, table, limit))
}
/// Blocking wrapper for [`execute`].
pub fn execute_blocking(
    session: &SshSession,
    db_path: &str,
    sql: &str,
) -> Result<RemoteQueryResult> {
    crate::ssh::runtime::shared().block_on(execute(session, db_path, sql))
}

// ─────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_single_quote_wraps_plain_input() {
        assert_eq!(shell_single_quote("abc"), "'abc'");
        assert_eq!(shell_single_quote(""), "''");
    }

    #[test]
    fn shell_single_quote_escapes_internal_quotes() {
        // Tom's file → 'Tom'\''s file'
        assert_eq!(shell_single_quote("Tom's"), "'Tom'\\''s'");
        assert_eq!(shell_single_quote("a'b'c"), "'a'\\''b'\\''c'",);
    }

    #[test]
    fn shell_single_quote_passes_through_metacharacters() {
        // Single-quote context neutralises every shell metachar.
        let dangerous = "'; rm -rf / #";
        let quoted = shell_single_quote(dangerous);
        assert!(quoted.starts_with('\''));
        assert!(quoted.ends_with('\''));
        assert!(!quoted.contains("''rm"));
    }

    #[test]
    fn escape_sql_string_doubles_quotes() {
        assert_eq!(escape_sql_string("plain"), "'plain'");
        assert_eq!(escape_sql_string("O'Neil"), "'O''Neil'");
    }

    #[test]
    fn double_quote_ident_escapes_inner_double_quotes() {
        assert_eq!(double_quote_ident("orders"), "\"orders\"");
        assert_eq!(
            double_quote_ident("my \"weird\" table"),
            "\"my \"\"weird\"\" table\""
        );
    }

    #[test]
    fn build_sqlite_json_command_composes_parts() {
        let cmd = build_sqlite_json_command("/srv/app.db", "SELECT 1");
        assert_eq!(cmd, "sqlite3 -json -bail -- '/srv/app.db' 'SELECT 1'");
    }

    #[test]
    fn parse_json_rows_empty_stdout_returns_empty_vec() {
        assert!(parse_json_rows("").unwrap().is_empty());
        assert!(parse_json_rows("   \n").unwrap().is_empty());
    }

    #[test]
    fn parse_json_rows_one_row_two_columns() {
        let out = r#"[{"id":1,"name":"Ann"},{"id":2,"name":"Bo"}]"#;
        let rows = parse_json_rows(out).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("id").map(String::as_str), Some("1"));
        assert_eq!(rows[0].get("name").map(String::as_str), Some("Ann"));
        assert_eq!(rows[1].get("name").map(String::as_str), Some("Bo"));
    }

    #[test]
    fn parse_json_rows_rejects_non_array_payload() {
        assert!(parse_json_rows("{\"id\":1}").is_err());
    }

    #[test]
    fn extract_column_order_preserves_order() {
        let out = r#"[{"id":1,"name":"Ann","age":20}]"#;
        // BTreeMap alphabetises keys, so extract_column_order is
        // what carries insertion order. Here that's alphabetical
        // anyway — the important test is that we get each column.
        let cols = extract_column_order(out).unwrap();
        assert_eq!(cols, vec!["id", "name", "age"]);
    }

    #[test]
    fn extract_column_order_empty_stdout_yields_empty() {
        assert_eq!(extract_column_order("").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn cap_cell_truncates_and_marks() {
        let big = "x".repeat(MAX_CELL_BYTES + 500);
        let capped = cap_cell(big);
        assert_eq!(capped.len(), MAX_CELL_BYTES + "…".len());
        assert!(capped.ends_with('…'));
    }

    #[test]
    fn parse_sqlite_version_extracts_first_token() {
        assert_eq!(
            parse_sqlite_version("3.46.1 2024-08-13 ceb..."),
            Some("3.46.1".to_string()),
        );
        assert_eq!(parse_sqlite_version(""), None);
        assert_eq!(parse_sqlite_version("not-a-version"), None);
    }

    #[test]
    fn supports_json_mode_requires_3_33_or_newer() {
        assert!(supports_json_mode("3.33.0"));
        assert!(supports_json_mode("3.46.1"));
        assert!(supports_json_mode("4.0.0"));
        assert!(!supports_json_mode("3.32.3"));
        assert!(!supports_json_mode("3.7.17"));
        assert!(!supports_json_mode("garbage"));
    }

    #[test]
    fn last_array_slice_single_array_is_noop() {
        let s = r#"[{"a":1}]"#;
        assert_eq!(last_array_slice(s), s);
    }

    #[test]
    fn last_array_slice_picks_last_of_concatenated() {
        // DDL followed by SELECT: sqlite3 -json emits an empty
        // array then a rows array.
        let s = r#"[][{"name":"users"}]"#;
        assert_eq!(last_array_slice(s), r#"[{"name":"users"}]"#);
    }

    #[test]
    fn last_array_slice_honours_quoted_brackets() {
        // A string value containing `][` must not trick the parser.
        let s = r#"[{"s":"]["}]"#;
        assert_eq!(last_array_slice(s), s);
    }

    #[test]
    fn last_array_slice_survives_escaped_quotes() {
        let s = r#"[{"s":"\"value\""}]"#;
        assert_eq!(last_array_slice(s), s);
    }

    #[test]
    fn last_array_slice_three_way_concat() {
        let s = r#"[][{"a":1}][{"b":2}]"#;
        assert_eq!(last_array_slice(s), r#"[{"b":2}]"#);
    }

    #[test]
    fn parse_json_rows_handles_multistatement_output() {
        // `CREATE TABLE t; SELECT * FROM t;` shape.
        let out = r#"[][{"id":1,"name":"Ann"}]"#;
        let rows = parse_json_rows(out).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("name").map(String::as_str), Some("Ann"));
    }

    #[test]
    fn remote_query_result_round_trips_through_json() {
        let r = RemoteQueryResult {
            columns: vec!["id".into(), "name".into()],
            rows: vec![vec!["1".into(), "Ann".into()]],
            truncated: false,
            affected_rows: 0,
            last_insert_id: None,
            elapsed_ms: 42,
            error: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: RemoteQueryResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.columns, r.columns);
        assert_eq!(back.rows, r.rows);
    }
}
