//! Socket-CLI PostgreSQL backend — runs the remote host's own `psql`
//! over an SSH exec channel, typically as the `postgres` OS user, so the
//! panel reaches the cluster via Unix-socket **peer** authentication with
//! no password. This is the counterpart to the native [`super::postgres`]
//! backend (`tokio_postgres` over a TCP tunnel + explicit role).
//!
//! ## Peer auth vs. the terminal's user
//!
//! Postgres `peer` ties the DB role to the OS user (role name == OS user
//! name). The superuser role `postgres` maps to the OS user `postgres`,
//! so "browse as superuser" is `sudo -u postgres psql` — not `sudo`
//! (root, whose role usually doesn't exist). The Tauri layer therefore
//! becomes a specific OS user (default `postgres`) rather than root.
//!
//! Output uses `-A -F '\t' -P null='\N' -P footer=off`, giving a
//! tab-separated header line + data rows with NULL printed as `\N` —
//! mirroring the `mysql --batch` shape so parsing stays uniform. Unlike
//! mysql, `psql -A` does **not** escape embedded tabs/newlines; values
//! containing them (rare for schema names / typical cells) can misparse.

use crate::ssh::error::{Result, SshError};
use crate::ssh::SshSession;
use crate::sudo::Elevation;

use super::mysql::is_safe_ident;
use super::postgres::{ColumnInfo, QueryResult, RoutineSummary, TableSummary, MAX_ROWS};

/// Single-quote `s` for `/bin/sh`.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A parsed `psql -A` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliTable {
    /// Column names from the header line.
    pub columns: Vec<String>,
    /// Data rows; `None` is SQL NULL (`\N`).
    pub rows: Vec<Vec<Option<String>>>,
}

/// Parse `psql -A -F '\t' -P null='\N' -P footer=off` output: first line
/// is the header, the rest are tab-separated rows, `\N` is NULL. psql
/// does not escape, so cells are taken verbatim.
pub fn parse_psql(stdout: &str) -> CliTable {
    let body = stdout.strip_suffix('\n').unwrap_or(stdout);
    if body.is_empty() {
        return CliTable {
            columns: Vec::new(),
            rows: Vec::new(),
        };
    }
    let mut lines = body.split('\n');
    let header = lines.next().unwrap_or("");
    let columns: Vec<String> = header.split('\t').map(|s| s.to_string()).collect();
    let rows = lines
        .map(|line| {
            line.split('\t')
                .map(|f| if f == "\\N" { None } else { Some(f.to_string()) })
                .collect()
        })
        .collect();
    CliTable { columns, rows }
}

/// Build the `psql` command. `-X` ignores psqlrc; `-q` quiet; `-A`
/// unaligned; `-F` tab field sep; `-P null` + `-P footer=off` give a
/// clean parseable shape. `database`, when a safe identifier, selects
/// the connection database.
fn psql_command(sql: &str, database: Option<&str>) -> String {
    let mut cmd = String::from("psql -X -q -A -F '\\t' -P null='\\N' -P footer=off");
    if let Some(db) = database {
        if is_safe_ident(db) {
            cmd.push_str(" -d ");
            cmd.push_str(db);
        }
    }
    cmd.push_str(" -c ");
    cmd.push_str(&shell_single_quote(sql));
    cmd
}

fn run_psql(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    sql: &str,
    database: Option<&str>,
) -> Result<CliTable> {
    let cmd = psql_command(sql, database);
    let (code, out) = session
        .exec_as_effective_blocking(&cmd, elevation, secret)
        .map_err(|e| SshError::InvalidConfig(e.to_string()))?;
    if code != 0 {
        let first = out.lines().next().unwrap_or("").trim();
        crate::logging::write_event(
            "WARN",
            "elevation.db",
            &format!("postgres socket-CLI exited {code}: {first}"),
        );
        return Err(SshError::InvalidConfig(format!("psql exited {code}: {first}")));
    }
    let table = parse_psql(&out);
    crate::logging::write_event_verbose(
        "DEBUG",
        "elevation.db",
        &format!(
            "postgres socket-CLI ok: cols={} rows={} sql={}",
            table.columns.len(),
            table.rows.len(),
            sql.chars().take(60).collect::<String>()
        ),
    );
    Ok(table)
}

/// Probe — `SELECT version()`.
pub fn probe(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
) -> Result<String> {
    let t = run_psql(session, elevation, secret, "SELECT version()", None)?;
    Ok(t.rows
        .first()
        .and_then(|r| r.first().cloned().flatten())
        .unwrap_or_default())
}

/// User databases (excludes templates) — mirrors
/// [`super::postgres::PostgresClient::list_databases`].
pub fn list_databases(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
) -> Result<Vec<String>> {
    let t = run_psql(
        session,
        elevation,
        secret,
        "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
        None,
    )?;
    const HIDDEN: [&str; 2] = ["template0", "template1"];
    Ok(t.rows
        .into_iter()
        .filter_map(|mut r| r.drain(..).next().flatten())
        .filter(|n| !HIDDEN.contains(&n.as_str()))
        .collect())
}

/// User-visible schemas in `database` — mirrors
/// [`super::postgres::PostgresClient::list_schemas`].
pub fn list_schemas(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    database: Option<&str>,
) -> Result<Vec<String>> {
    let t = run_psql(
        session,
        elevation,
        secret,
        "SELECT schema_name FROM information_schema.schemata \
         WHERE schema_name NOT IN ('pg_catalog','information_schema') \
           AND schema_name NOT LIKE 'pg_toast%' AND schema_name NOT LIKE 'pg_temp_%' \
         ORDER BY schema_name",
        database,
    )?;
    Ok(t.rows
        .into_iter()
        .filter_map(|mut r| r.drain(..).next().flatten())
        .collect())
}

fn col_idx(columns: &[String], name: &str) -> Option<usize> {
    columns.iter().position(|c| c.eq_ignore_ascii_case(name))
}
fn cell<'a>(row: &'a [Option<String>], idx: Option<usize>) -> Option<&'a str> {
    idx.and_then(|i| row.get(i)).and_then(|v| v.as_deref())
}

/// Enriched table list — mirrors
/// [`super::postgres::PostgresClient::list_tables_meta`].
pub fn list_tables_meta(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    database: Option<&str>,
    schema: &str,
) -> Result<Vec<TableSummary>> {
    let schema = if schema.is_empty() { "public" } else { schema };
    if !is_safe_ident(schema) {
        return Err(SshError::InvalidConfig(format!(
            "refusing unsafe schema identifier {schema:?}"
        )));
    }
    let sql = format!(
        "SELECT c.relname AS name, c.reltuples::bigint AS rows, \
                pg_relation_size(c.oid) AS data, pg_indexes_size(c.oid) AS idx, \
                COALESCE(obj_description(c.oid,'pg_class'),'') AS comment \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = '{schema}' AND c.relkind = 'r' ORDER BY c.relname"
    );
    let t = run_psql(session, elevation, secret, &sql, database)?;
    let (i_name, i_rows, i_data, i_idx, i_com) = (
        col_idx(&t.columns, "name"),
        col_idx(&t.columns, "rows"),
        col_idx(&t.columns, "data"),
        col_idx(&t.columns, "idx"),
        col_idx(&t.columns, "comment"),
    );
    Ok(t
        .rows
        .iter()
        .filter_map(|r| {
            let to_u64 = |v: Option<&str>| v.and_then(|s| s.parse::<i64>().ok()).and_then(|n| if n < 0 { None } else { Some(n as u64) });
            Some(TableSummary {
                name: cell(r, i_name)?.to_string(),
                row_count: to_u64(cell(r, i_rows)),
                data_bytes: to_u64(cell(r, i_data)),
                index_bytes: to_u64(cell(r, i_idx)),
                engine: None,
                updated_at: None,
                comment: cell(r, i_com).unwrap_or_default().to_string(),
            })
        })
        .collect())
}

/// View names — mirrors [`super::postgres::PostgresClient::list_views`].
pub fn list_views(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    database: Option<&str>,
    schema: &str,
) -> Result<Vec<String>> {
    let schema = if schema.is_empty() { "public" } else { schema };
    if !is_safe_ident(schema) {
        return Err(SshError::InvalidConfig(format!(
            "refusing unsafe schema identifier {schema:?}"
        )));
    }
    let sql = format!(
        "SELECT table_name FROM information_schema.views \
         WHERE table_schema = '{schema}' ORDER BY table_name"
    );
    let t = run_psql(session, elevation, secret, &sql, database)?;
    Ok(t.rows
        .into_iter()
        .filter_map(|mut r| r.drain(..).next().flatten())
        .collect())
}

/// Stored routines — mirrors [`super::postgres::PostgresClient::list_routines`].
pub fn list_routines(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    database: Option<&str>,
    schema: &str,
) -> Result<Vec<RoutineSummary>> {
    let schema = if schema.is_empty() { "public" } else { schema };
    if !is_safe_ident(schema) {
        return Err(SshError::InvalidConfig(format!(
            "refusing unsafe schema identifier {schema:?}"
        )));
    }
    let sql = format!(
        "SELECT routine_name, routine_type FROM information_schema.routines \
         WHERE routine_schema = '{schema}' ORDER BY routine_name"
    );
    let t = run_psql(session, elevation, secret, &sql, database)?;
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

/// Column info — mirrors [`super::postgres::PostgresClient::list_columns`].
pub fn list_columns(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    database: Option<&str>,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>> {
    let schema = if schema.is_empty() { "public" } else { schema };
    if !is_safe_ident(schema) || !is_safe_ident(table) {
        return Err(SshError::InvalidConfig(
            "refusing unsafe identifier".to_string(),
        ));
    }
    let sql = format!(
        "SELECT column_name, \
                COALESCE(NULLIF(pg_catalog.format_type(a.atttypid, a.atttypmod),''), c.data_type) AS pretty_type, \
                c.is_nullable, c.column_default, '' AS extra, \
                COALESCE(pg_catalog.col_description(pgc.oid, a.attnum),'') AS comment \
         FROM information_schema.columns c \
         JOIN pg_catalog.pg_class pgc ON pgc.relname = c.table_name \
         JOIN pg_catalog.pg_namespace pgn ON pgn.oid = pgc.relnamespace AND pgn.nspname = c.table_schema \
         JOIN pg_catalog.pg_attribute a ON a.attrelid = pgc.oid AND a.attname = c.column_name \
         WHERE c.table_schema = '{schema}' AND c.table_name = '{table}' \
         ORDER BY c.ordinal_position"
    );
    let t = run_psql(session, elevation, secret, &sql, database)?;
    let (i_name, i_type, i_null, i_def, i_extra, i_com) = (
        col_idx(&t.columns, "column_name"),
        col_idx(&t.columns, "pretty_type"),
        col_idx(&t.columns, "is_nullable"),
        col_idx(&t.columns, "column_default"),
        col_idx(&t.columns, "extra"),
        col_idx(&t.columns, "comment"),
    );
    Ok(t
        .rows
        .iter()
        .filter_map(|r| {
            Some(ColumnInfo {
                name: cell(r, i_name)?.to_string(),
                column_type: cell(r, i_type).unwrap_or_default().to_string(),
                nullable: cell(r, i_null).unwrap_or("").eq_ignore_ascii_case("YES"),
                key: String::new(),
                default_value: cell(r, i_def).map(|v| v.to_string()),
                extra: cell(r, i_extra).unwrap_or_default().to_string(),
                comment: cell(r, i_com).unwrap_or_default().to_string(),
            })
        })
        .collect())
}

/// Execute a statement — mirrors [`super::postgres::PostgresClient::execute`]
/// (minus affected_rows, which `-P footer=off` suppresses; returns 0).
pub fn execute(
    session: &SshSession,
    elevation: &Elevation,
    secret: Option<&str>,
    sql: &str,
    database: Option<&str>,
) -> Result<QueryResult> {
    let started = std::time::Instant::now();
    let mut t = run_psql(session, elevation, secret, sql, database)?;
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
    fn parses_header_rows_and_null() {
        let out = "id\tname\n1\tAlice\n2\t\\N\n";
        let t = parse_psql(out);
        assert_eq!(t.columns, vec!["id", "name"]);
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[1][1], None);
        assert_eq!(t.rows[0][1], Some("Alice".to_string()));
    }

    #[test]
    fn empty_and_header_only() {
        assert!(parse_psql("").rows.is_empty());
        let t = parse_psql("a\tb\n");
        assert_eq!(t.columns, vec!["a", "b"]);
        assert!(t.rows.is_empty());
    }

    #[test]
    fn command_has_clean_flags_and_db() {
        let cmd = psql_command("SELECT 1", Some("shop"));
        assert!(cmd.contains("-A -F '\\t'"));
        assert!(cmd.contains("null='\\N'"));
        assert!(cmd.contains("footer=off"));
        assert!(cmd.contains(" -d shop "));
        assert!(cmd.contains("-c "));
    }

    #[test]
    fn command_drops_unsafe_db() {
        let cmd = psql_command("SELECT 1", Some("a;b"));
        assert!(!cmd.contains("-d a;b"));
    }
}
