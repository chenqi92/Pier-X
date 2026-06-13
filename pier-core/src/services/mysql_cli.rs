//! Socket-CLI MySQL backend — runs the remote host's own `mysql` client
//! over an SSH exec channel, **as the terminal's effective user**, so the
//! panel mirrors what `mysql` would see in the terminal after `su root`
//! / `sudo -i`. On Debian/Ubuntu the default `root@localhost` uses the
//! `auth_socket` plugin, so `sudo mysql` connects with no password.
//!
//! This is the counterpart to the native [`super::mysql`] backend
//! (`mysql_async` over a TCP tunnel + explicit account). It reuses the
//! native backend's result structs ([`TableSummary`], [`ColumnInfo`],
//! [`RoutineSummary`], [`QueryResult`]) so the Tauri layer maps both
//! paths identically.
//!
//! Unlike the native path, identity here is the **OS user**, not a DB
//! account: elevation is applied via [`crate::ssh::SshSession::exec_as_effective`].

use crate::ssh::error::{Result, SshError};
use crate::ssh::SshSession;
use crate::sudo::Elevation;

use super::mysql::{
    is_safe_ident, ColumnInfo, QueryResult, RoutineSummary, TableSummary, MAX_ROWS,
};

/// Single-quote `s` for `/bin/sh`, escaping embedded quotes as `'\''`.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A parsed `mysql --batch` result: header row + data rows. `None` cells
/// are SQL NULL (`\N` in batch output).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliTable {
    /// Column names from the header line.
    pub columns: Vec<String>,
    /// Data rows; each cell is `None` for SQL NULL (`\N`).
    pub rows: Vec<Vec<Option<String>>>,
}

/// Unescape a single `mysql --batch` field. mysql escapes `\t`, `\n`,
/// `\0`, and `\\`; everything else after a backslash is passed through.
fn unescape_field(field: &str) -> String {
    if !field.contains('\\') {
        return field.to_string();
    }
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('0') => out.push('\0'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// One cell: the literal `\N` is SQL NULL; anything else is unescaped.
fn parse_cell(field: &str) -> Option<String> {
    if field == "\\N" {
        None
    } else {
        Some(unescape_field(field))
    }
}

/// Parse `mysql --batch` stdout (tab-separated, first line = headers,
/// real newlines separate rows, embedded specials escaped). Empty
/// stdout (a non-SELECT statement, or no rows + no header) → empty
/// table.
pub fn parse_batch(stdout: &str) -> CliTable {
    let body = stdout.strip_suffix('\n').unwrap_or(stdout);
    if body.is_empty() {
        return CliTable {
            columns: Vec::new(),
            rows: Vec::new(),
        };
    }
    let mut lines = body.split('\n');
    let header = lines.next().unwrap_or("");
    let columns: Vec<String> = header.split('\t').map(unescape_field).collect();
    let rows: Vec<Vec<Option<String>>> = lines
        .map(|line| line.split('\t').map(parse_cell).collect())
        .collect();
    CliTable { columns, rows }
}

/// Build the remote `mysql` command. `--batch` gives machine-parseable
/// tab-separated output; `--protocol=socket` forces the local unix
/// socket so the `auth_socket` plugin authenticates by OS uid.
/// `database`, when a safe identifier, is passed as the default schema.
fn mysql_command(sql: &str, database: Option<&str>) -> String {
    let mut cmd =
        String::from("mysql --batch --protocol=socket --default-character-set=utf8mb4");
    if let Some(db) = database {
        if is_safe_ident(db) {
            cmd.push(' ');
            cmd.push_str(db);
        }
    }
    cmd.push_str(" -e ");
    cmd.push_str(&shell_single_quote(sql));
    cmd
}

/// Run one statement via the remote `mysql` CLI at the given elevation.
fn run_batch(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    sql: &str,
    database: Option<&str>,
) -> Result<CliTable> {
    let cmd = mysql_command(sql, database);
    let (code, out) = session
        .exec_as_effective_blocking(&cmd, elevation, secret)
        .map_err(|e| SshError::InvalidConfig(e.to_string()))?;
    if code != 0 {
        // mysql writes diagnostics (auth failure, syntax error, missing
        // binary) to stderr, which `exec_*` merges into `out`.
        let first = out.lines().next().unwrap_or("").trim();
        crate::logging::write_event(
            "WARN",
            "elevation.db",
            &format!("mysql socket-CLI exited {code}: {first}"),
        );
        return Err(SshError::InvalidConfig(format!("mysql exited {code}: {first}")));
    }
    let table = parse_batch(&out);
    crate::logging::write_event_verbose(
        "DEBUG",
        "elevation.db",
        &format!(
            "mysql socket-CLI ok: cols={} rows={} sql={}",
            table.columns.len(),
            table.rows.len(),
            sql.chars().take(60).collect::<String>()
        ),
    );
    Ok(table)
}

/// Probe whether the elevated `mysql` CLI is usable (binary present +
/// socket auth works). Returns the server version string on success.
pub fn probe(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
) -> Result<String> {
    let table = run_batch(session, elevation, secret, "SELECT VERSION()", None)?;
    Ok(table
        .rows
        .first()
        .and_then(|r| r.first().cloned().flatten())
        .unwrap_or_default())
}

/// `SHOW DATABASES`, minus the internal schemas — mirrors
/// [`super::mysql::MysqlClient::list_databases`].
pub fn list_databases(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
) -> Result<Vec<String>> {
    let table = run_batch(session, elevation, secret, "SHOW DATABASES", None)?;
    const HIDDEN: [&str; 4] = ["information_schema", "performance_schema", "mysql", "sys"];
    Ok(table
        .rows
        .into_iter()
        .filter_map(|mut r| r.drain(..).next().flatten())
        .filter(|n| !HIDDEN.contains(&n.as_str()))
        .collect())
}

/// Column index helper — find a header by name (case-insensitive).
fn col_idx(columns: &[String], name: &str) -> Option<usize> {
    columns.iter().position(|c| c.eq_ignore_ascii_case(name))
}

fn cell<'a>(row: &'a [Option<String>], idx: Option<usize>) -> Option<&'a str> {
    idx.and_then(|i| row.get(i)).and_then(|v| v.as_deref())
}

/// `information_schema.tables` enrichment — mirrors
/// [`super::mysql::MysqlClient::list_tables_meta`].
pub fn list_tables_meta(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    database: &str,
) -> Result<Vec<TableSummary>> {
    if !is_safe_ident(database) {
        return Err(SshError::InvalidConfig(format!(
            "refusing unsafe database identifier {database:?}"
        )));
    }
    let sql = format!(
        "SELECT table_name, table_rows, data_length, index_length, engine, \
         CAST(update_time AS CHAR) AS update_time, COALESCE(table_comment,'') AS table_comment \
         FROM information_schema.tables \
         WHERE table_schema = '{database}' AND table_type = 'BASE TABLE' \
         ORDER BY table_name"
    );
    let t = run_batch(session, elevation, secret, &sql, None)?;
    let (i_name, i_rows, i_data, i_idx, i_eng, i_upd, i_com) = (
        col_idx(&t.columns, "table_name"),
        col_idx(&t.columns, "table_rows"),
        col_idx(&t.columns, "data_length"),
        col_idx(&t.columns, "index_length"),
        col_idx(&t.columns, "engine"),
        col_idx(&t.columns, "update_time"),
        col_idx(&t.columns, "table_comment"),
    );
    Ok(t
        .rows
        .iter()
        .filter_map(|r| {
            let name = cell(r, i_name)?.to_string();
            Some(TableSummary {
                name,
                row_count: cell(r, i_rows).and_then(|v| v.parse().ok()),
                data_bytes: cell(r, i_data).and_then(|v| v.parse().ok()),
                index_bytes: cell(r, i_idx).and_then(|v| v.parse().ok()),
                engine: cell(r, i_eng).map(|v| v.to_string()),
                updated_at: cell(r, i_upd).map(|v| v.to_string()),
                comment: cell(r, i_com).unwrap_or_default().to_string(),
            })
        })
        .collect())
}

/// View names — mirrors [`super::mysql::MysqlClient::list_views`].
pub fn list_views(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    database: &str,
) -> Result<Vec<String>> {
    if !is_safe_ident(database) {
        return Err(SshError::InvalidConfig(format!(
            "refusing unsafe database identifier {database:?}"
        )));
    }
    let sql = format!(
        "SELECT table_name FROM information_schema.views \
         WHERE table_schema = '{database}' ORDER BY table_name"
    );
    let t = run_batch(session, elevation, secret, &sql, None)?;
    Ok(t.rows
        .into_iter()
        .filter_map(|mut r| r.drain(..).next().flatten())
        .collect())
}

/// Stored routines — mirrors [`super::mysql::MysqlClient::list_routines`].
pub fn list_routines(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    database: &str,
) -> Result<Vec<RoutineSummary>> {
    if !is_safe_ident(database) {
        return Err(SshError::InvalidConfig(format!(
            "refusing unsafe database identifier {database:?}"
        )));
    }
    let sql = format!(
        "SELECT routine_name, routine_type FROM information_schema.routines \
         WHERE routine_schema = '{database}' ORDER BY routine_name"
    );
    let t = run_batch(session, elevation, secret, &sql, None)?;
    let (i_name, i_kind) = (
        col_idx(&t.columns, "routine_name"),
        col_idx(&t.columns, "routine_type"),
    );
    Ok(t
        .rows
        .iter()
        .filter_map(|r| {
            Some(RoutineSummary {
                name: cell(r, i_name)?.to_string(),
                kind: cell(r, i_kind).unwrap_or_default().to_string(),
            })
        })
        .collect())
}

/// `SHOW FULL COLUMNS` — mirrors [`super::mysql::MysqlClient::list_columns`].
pub fn list_columns(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    database: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>> {
    if !is_safe_ident(database) || !is_safe_ident(table) {
        return Err(SshError::InvalidConfig(
            "refusing unsafe identifier".to_string(),
        ));
    }
    let sql = format!("SHOW FULL COLUMNS FROM `{database}`.`{table}`");
    let t = run_batch(session, elevation, secret, &sql, None)?;
    let (i_field, i_type, i_null, i_key, i_def, i_extra, i_com) = (
        col_idx(&t.columns, "Field"),
        col_idx(&t.columns, "Type"),
        col_idx(&t.columns, "Null"),
        col_idx(&t.columns, "Key"),
        col_idx(&t.columns, "Default"),
        col_idx(&t.columns, "Extra"),
        col_idx(&t.columns, "Comment"),
    );
    Ok(t
        .rows
        .iter()
        .filter_map(|r| {
            Some(ColumnInfo {
                name: cell(r, i_field)?.to_string(),
                column_type: cell(r, i_type).unwrap_or_default().to_string(),
                nullable: cell(r, i_null).unwrap_or("").eq_ignore_ascii_case("YES"),
                key: cell(r, i_key).unwrap_or_default().to_string(),
                default_value: cell(r, i_def).map(|v| v.to_string()),
                extra: cell(r, i_extra).unwrap_or_default().to_string(),
                comment: cell(r, i_com).unwrap_or_default().to_string(),
            })
        })
        .collect())
}

/// Execute an arbitrary statement, returning columns + capped rows —
/// mirrors [`super::mysql::MysqlClient::execute`] (minus
/// affected_rows / last_insert_id, which `--batch` doesn't surface;
/// those come back 0). `database` sets the default schema.
pub fn execute(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    sql: &str,
    database: Option<&str>,
) -> Result<QueryResult> {
    let started = std::time::Instant::now();
    let mut t = run_batch(session, elevation, secret, sql, database)?;
    let truncated = t.rows.len() > MAX_ROWS;
    if truncated {
        t.rows.truncate(MAX_ROWS);
    }
    Ok(QueryResult {
        columns: t.columns,
        rows: t.rows,
        truncated,
        affected_rows: 0,
        last_insert_id: None,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_and_rows_with_null() {
        let out = "id\tname\temail\n1\tAlice\talice@x.io\n2\tBob\t\\N\n";
        let t = parse_batch(out);
        assert_eq!(t.columns, vec!["id", "name", "email"]);
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[0][1], Some("Alice".to_string()));
        assert_eq!(t.rows[1][2], None); // \N → NULL
    }

    #[test]
    fn unescapes_embedded_specials() {
        // A value containing a tab and a newline, batch-escaped.
        let out = "v\nline1\\nline2\\tcol\n";
        let t = parse_batch(out);
        assert_eq!(t.rows[0][0], Some("line1\nline2\tcol".to_string()));
    }

    #[test]
    fn empty_output_is_empty_table() {
        assert_eq!(parse_batch(""), CliTable { columns: vec![], rows: vec![] });
        assert_eq!(parse_batch("\n"), CliTable { columns: vec![], rows: vec![] });
    }

    #[test]
    fn header_only_has_no_rows() {
        let t = parse_batch("a\tb\n");
        assert_eq!(t.columns, vec!["a", "b"]);
        assert!(t.rows.is_empty());
    }

    #[test]
    fn null_literal_vs_text_n() {
        // `\N` (whole field) is NULL; a real "N" or "\\N" text is not.
        let t = parse_batch("c\n\\N\nN\n");
        assert_eq!(t.rows[0][0], None);
        assert_eq!(t.rows[1][0], Some("N".to_string()));
    }

    #[test]
    fn command_forces_socket_and_quotes_sql() {
        let cmd = mysql_command("SELECT 'a''b'", Some("shopdb"));
        assert!(cmd.contains("--protocol=socket"));
        assert!(cmd.contains(" shopdb "));
        assert!(cmd.contains("-e "));
    }

    #[test]
    fn command_drops_unsafe_database() {
        let cmd = mysql_command("SELECT 1", Some("bad; DROP"));
        // Unsafe identifier is not interpolated as a schema arg.
        assert!(!cmd.contains("bad; DROP "));
    }
}
