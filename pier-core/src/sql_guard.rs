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
    let keyword = match leading_sql_keyword(sql) {
        Some(k) => k,
        None => return false,
    };
    if !READ_ONLY_KEYWORDS.contains(&keyword.as_str()) {
        return false;
    }
    // `SET` is allowed for benign session settings (SET NAMES, SET
    // SESSION …) but SET GLOBAL / SET PERSIST[_ONLY] mutate server-wide
    // state and must not pass the read-only gate.
    if keyword == "SET" {
        let lower = strip_leading_sql_comments(sql).to_ascii_lowercase();
        if let Some(after_set) = lower.strip_prefix("set") {
            let trimmed = after_set.trim_start();
            // Require whitespace between `set` and the next token.
            if trimmed.len() < after_set.len() {
                for kw in ["global", "persist_only", "persist"] {
                    if let Some(tail) = trimmed.strip_prefix(kw) {
                        let bounded = tail
                            .chars()
                            .next()
                            .map_or(true, |c| !(c.is_ascii_alphanumeric() || c == '_'));
                        if bounded {
                            return false;
                        }
                    }
                }
            }
        }
    }
    true
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

/// Uppercased leading alphabetic keyword after stripping comments, or
/// `None` when the statement doesn't start with a word.
fn leading_sql_keyword(sql: &str) -> Option<String> {
    let remaining = strip_leading_sql_comments(sql);
    let kw: String = remaining
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    if kw.is_empty() {
        None
    } else {
        Some(kw.to_ascii_uppercase())
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
}
