//! Backend SQL read-only gate — defense in depth for the DB panels.
//!
//! The DB panels enforce a read-only lock in the UI via the TypeScript
//! `isReadOnlySql`. This is the backend mirror so the *commands* — not
//! just the renderer — refuse a writing statement when the caller
//! declares read-only intent. It is a deliberate close port of the
//! frontend logic so the two agree.
//!
//! This is NOT a substitute for a least-privilege database account (the
//! only thing that fully stops a writing statement against a fully
//! compromised renderer), but it catches frontend bugs, parser gaps, and
//! injection that can influence the SQL string without arbitrary code
//! execution.

/// Error message returned to the UI when a DB execute command rejects a
/// statement because read-only mode is on and [`is_read_only_sql`] said
/// no. Kept here so the message lives with the policy it enforces.
pub const READ_ONLY_REJECT_MSG: &str =
    "Read-only mode is on: this statement writes or isn't a single read-only query. Unlock writes to run it.";

/// Leading keywords that begin a statement with no row/schema mutation.
/// Mirrors `readOnlySqlKeywords` in `src/lib/commands.ts`.
const READ_ONLY_KEYWORDS: &[&str] = &[
    "SELECT", "SHOW", "DESCRIBE", "DESC", "EXPLAIN", "PRAGMA", "HELP", "USE", "SET", "BEGIN",
    "START", "COMMIT", "ROLLBACK",
];

/// Keywords that mutate rows, schema, privileges, or write files. Used to
/// reject statements that smuggle a write past a benign leading keyword —
/// e.g. `EXPLAIN ANALYZE DELETE …` (PostgreSQL actually executes the
/// analyzed statement) or `SELECT … INTO newtbl`.
const WRITE_KEYWORDS: &[&str] = &[
    "INSERT", "UPDATE", "DELETE", "REPLACE", "MERGE", "UPSERT", "TRUNCATE", "DROP", "CREATE",
    "ALTER", "RENAME", "GRANT", "REVOKE", "CALL", "EXEC", "EXECUTE", "LOAD", "COPY", "IMPORT",
    "INTO", "ATTACH", "DETACH", "VACUUM", "REINDEX", "CLUSTER", "LOCK", "UNLOCK", "HANDLER",
    "INSTALL", "UNINSTALL", "OPTIMIZE", "REPAIR",
];

/// Is `sql` a single read-only statement? Conservative: a write hidden
/// behind a benign leading statement (`SELECT 1; DROP TABLE x`) is
/// multi-statement and rejected; `SET GLOBAL` / `SET PERSIST[_ONLY]`
/// mutate server-wide state and are rejected. Anything not provably
/// read-only returns `false`.
pub fn is_read_only_sql(sql: &str) -> bool {
    // A write hidden after a `;` would otherwise ride in behind a benign
    // leading keyword. Require a single statement first.
    if has_multiple_statements(sql) {
        return false;
    }
    let body = strip_leading_sql_comments(sql);
    let (keyword, rest) = match split_leading_keyword(body) {
        Some(v) => v,
        None => return false,
    };
    if !READ_ONLY_KEYWORDS.contains(&keyword.as_str()) {
        return false;
    }
    match keyword.as_str() {
        // `SET` is allowed for benign session settings (SET NAMES, SET
        // SESSION …) but SET GLOBAL / SET PERSIST[_ONLY] mutate server-wide
        // state, and SET ROLE / SET SESSION AUTHORIZATION change the
        // privilege context — none may pass the read-only gate.
        "SET" => set_is_read_only(rest),
        // EXPLAIN/DESCRIBE/DESC can wrap a writing statement. In particular
        // PostgreSQL's `EXPLAIN ANALYZE <DML>` *executes* the analyzed DML.
        // Reject when the explained tail contains any write keyword.
        "EXPLAIN" | "DESCRIBE" | "DESC" => !statement_contains_keyword(rest, WRITE_KEYWORDS),
        // `SELECT … INTO …` (SQL Server / PostgreSQL / MySQL OUTFILE)
        // creates or writes. A plain SELECT is otherwise read-only
        // (including `… FOR UPDATE` locking reads), so only block INTO.
        "SELECT" => !statement_contains_keyword(rest, &["INTO"]),
        _ => true,
    }
}

/// Split the comment-stripped statement `body` into its uppercased leading
/// alphabetic keyword and the remaining text after it. `None` when the
/// statement doesn't start with a word.
fn split_leading_keyword(body: &str) -> Option<(String, &str)> {
    let kw: String = body.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if kw.is_empty() {
        return None;
    }
    // `kw` is ASCII, so its byte length equals its char count.
    let rest = &body[kw.len()..];
    Some((kw.to_ascii_uppercase(), rest))
}

/// Decide whether a `SET …` statement is read-only. `rest` is the text
/// after the `SET` keyword. Blocks server-wide (`GLOBAL` / `PERSIST` /
/// `PERSIST_ONLY`) and privilege-context (`ROLE`, `SESSION AUTHORIZATION`)
/// mutations; benign session settings (`SET NAMES`, `SET SESSION x = …`)
/// pass.
fn set_is_read_only(rest: &str) -> bool {
    let lower = rest.to_ascii_lowercase();
    let trimmed = lower.trim_start();
    let bounded_after = |tail: &str| {
        tail.chars()
            .next()
            .map_or(true, |c| !(c.is_ascii_alphanumeric() || c == '_'))
    };
    for kw in ["global", "persist_only", "persist", "role"] {
        if let Some(tail) = trimmed.strip_prefix(kw) {
            if bounded_after(tail) {
                return false;
            }
        }
    }
    if let Some(after_session) = trimmed.strip_prefix("session") {
        if bounded_after(after_session) {
            if let Some(tail) = after_session.trim_start().strip_prefix("authorization") {
                if bounded_after(tail) {
                    return false;
                }
            }
        }
    }
    true
}

/// Scan `sql` for a top-level occurrence of any keyword in `set`,
/// respecting string literals (`'`, `"`, `` ` ``) and comments so a
/// keyword inside a string or quoted identifier is ignored. Word
/// boundaries are honored (`delete_flag` does not match `DELETE`).
fn statement_contains_keyword(sql: &str, set: &[&str]) -> bool {
    let chars: Vec<char> = sql.chars().collect();
    let (mut in_single, mut in_double, mut in_backtick) = (false, false, false);
    let (mut in_line, mut in_block) = (false, false);
    let mut word = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let n = chars.get(i + 1).copied();
        let in_str_or_comment = in_line || in_block || in_single || in_double || in_backtick;
        let is_word_char =
            !in_str_or_comment && (c.is_ascii_alphanumeric() || c == '_' || c == '$');
        if !is_word_char && !word.is_empty() {
            if set.contains(&word.to_ascii_uppercase().as_str()) {
                return true;
            }
            word.clear();
        }
        if in_line {
            if c == '\n' {
                in_line = false;
            }
            i += 1;
            continue;
        }
        if in_block {
            if c == '*' && n == Some('/') {
                in_block = false;
                i += 1;
            }
            i += 1;
            continue;
        }
        if in_single {
            if c == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if c == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        if in_backtick {
            if c == '`' {
                in_backtick = false;
            }
            i += 1;
            continue;
        }
        if c == '-' && n == Some('-') {
            in_line = true;
            i += 2;
            continue;
        }
        if c == '/' && n == Some('*') {
            in_block = true;
            i += 2;
            continue;
        }
        match c {
            '\'' => in_single = true,
            '"' => in_double = true,
            '`' => in_backtick = true,
            _ if c.is_ascii_alphanumeric() || c == '_' || c == '$' => word.push(c),
            _ => {}
        }
        i += 1;
    }
    !word.is_empty() && set.contains(&word.to_ascii_uppercase().as_str())
}

/// Strip leading line/block comments + whitespace; return the remainder
/// from the first real token, or `""` if an unterminated comment
/// swallows the rest. Mirrors `stripLeadingSqlComments` in the frontend.
fn strip_leading_sql_comments(sql: &str) -> &str {
    let mut remaining = sql.trim_start();
    loop {
        if let Some(rest) = remaining.strip_prefix("--") {
            match rest.find('\n') {
                Some(i) => remaining = rest[i + 1..].trim_start(),
                None => return "",
            }
        } else if let Some(rest) = remaining.strip_prefix("/*") {
            match rest.find("*/") {
                Some(i) => remaining = rest[i + 2..].trim_start(),
                None => return "",
            }
        } else {
            return remaining;
        }
    }
}

/// True if `sql` has more than one top-level statement — a `;` followed
/// by further non-comment, non-whitespace content — respecting string
/// literals (`'`, `"`, `` ` ``) and comments. A lone trailing `;` is not
/// multi-statement. Mirrors `hasMultipleStatements` in the frontend.
fn has_multiple_statements(sql: &str) -> bool {
    let chars: Vec<char> = sql.chars().collect();
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut saw_separator = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let n = chars.get(i + 1).copied();
        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if c == '*' && n == Some('/') {
                in_block_comment = false;
                i += 1;
            }
            i += 1;
            continue;
        }
        if in_single {
            if c == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if c == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        if in_backtick {
            if c == '`' {
                in_backtick = false;
            }
            i += 1;
            continue;
        }
        if c == '-' && n == Some('-') {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if c == '/' && n == Some('*') {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if c == '\'' {
            in_single = true;
            i += 1;
            continue;
        }
        if c == '"' {
            in_double = true;
            i += 1;
            continue;
        }
        if c == '`' {
            in_backtick = true;
            i += 1;
            continue;
        }
        if c == ';' {
            saw_separator = true;
            i += 1;
            continue;
        }
        // First meaningful char after a top-level `;` ⇒ second statement.
        if saw_separator && !c.is_whitespace() {
            return true;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::is_read_only_sql;

    #[test]
    fn reads_are_allowed() {
        assert!(is_read_only_sql("SELECT * FROM t"));
        assert!(is_read_only_sql("  select 1"));
        assert!(is_read_only_sql("SHOW TABLES"));
        assert!(is_read_only_sql("EXPLAIN SELECT 1"));
        assert!(is_read_only_sql("PRAGMA table_info(t)"));
        assert!(is_read_only_sql("-- a comment\nSELECT 1"));
        assert!(is_read_only_sql("/* c */ SELECT 1"));
        assert!(is_read_only_sql("SELECT 1;")); // lone trailing ; is fine
        assert!(is_read_only_sql("SET NAMES utf8mb4"));
        assert!(is_read_only_sql("SET SESSION sql_mode = ''"));
    }

    #[test]
    fn writes_are_rejected() {
        assert!(!is_read_only_sql("UPDATE t SET x = 1"));
        assert!(!is_read_only_sql("DELETE FROM t"));
        assert!(!is_read_only_sql("INSERT INTO t VALUES (1)"));
        assert!(!is_read_only_sql("DROP TABLE t"));
        assert!(!is_read_only_sql("TRUNCATE t"));
        assert!(!is_read_only_sql("")); // empty → not provably read-only
        assert!(!is_read_only_sql("/* unterminated"));
    }

    #[test]
    fn hidden_write_behind_select_is_rejected() {
        assert!(!is_read_only_sql("SELECT 1; DROP TABLE x"));
        assert!(!is_read_only_sql("SELECT 1; DELETE FROM t"));
        // a `;` inside a string literal does not count as a separator
        assert!(is_read_only_sql("SELECT ';' AS sep"));
    }

    #[test]
    fn set_global_is_rejected() {
        assert!(!is_read_only_sql("SET GLOBAL max_connections = 1000"));
        assert!(!is_read_only_sql("SET PERSIST max_connections = 1000"));
        assert!(!is_read_only_sql("SET PERSIST_ONLY max_connections = 1000"));
        assert!(!is_read_only_sql("set   global x = 1"));
    }

    #[test]
    fn explain_analyze_dml_is_rejected() {
        // PostgreSQL's EXPLAIN ANALYZE actually runs the analyzed statement.
        assert!(!is_read_only_sql("EXPLAIN ANALYZE DELETE FROM t"));
        assert!(!is_read_only_sql("EXPLAIN ANALYZE UPDATE t SET x = 1"));
        assert!(!is_read_only_sql("EXPLAIN ANALYZE INSERT INTO t VALUES (1)"));
        assert!(!is_read_only_sql("explain (analyze, verbose) delete from t"));
        // Plain EXPLAIN of a read stays allowed.
        assert!(is_read_only_sql("EXPLAIN SELECT 1"));
        assert!(is_read_only_sql("EXPLAIN ANALYZE SELECT * FROM t"));
        assert!(is_read_only_sql("DESCRIBE mytable"));
    }

    #[test]
    fn select_into_is_rejected() {
        // SELECT … INTO creates/populates a table (SQL Server / PostgreSQL)
        // or writes a file (MySQL SELECT … INTO OUTFILE).
        assert!(!is_read_only_sql("SELECT * INTO newtbl FROM t"));
        assert!(!is_read_only_sql("select a, b into dumpfile '/tmp/x' from t"));
        // A column literally named with a write word is not a write.
        assert!(is_read_only_sql("SELECT delete_flag, created_at FROM t"));
        assert!(is_read_only_sql("SELECT * FROM t WHERE note = 'delete me'"));
        // Locking reads remain allowed.
        assert!(is_read_only_sql("SELECT * FROM t FOR UPDATE"));
    }

    #[test]
    fn set_role_and_authorization_are_rejected() {
        assert!(!is_read_only_sql("SET ROLE postgres"));
        assert!(!is_read_only_sql("set role admin"));
        assert!(!is_read_only_sql("SET SESSION AUTHORIZATION postgres"));
        // A setting whose name merely starts with "role"/"session" is fine.
        assert!(is_read_only_sql("SET role_check = 1"));
        assert!(is_read_only_sql("SET SESSION sql_mode = ''"));
    }
}
