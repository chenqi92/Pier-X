// Database panel — shared by MySQL / PostgreSQL / Redis / SQLite tools.
//
// Two access paths, picked with the engine selector at the top:
//
//   * SQLite (no network): discover `.db` / `.sqlite` files under the working
//     dir, open one via pier_core::services::sqlite::SqliteClient, then list its
//     tables and — for the selected table — its columns. All sqlite3 subprocess
//     work runs on the background executor; render only paints cached state.
//   * MySQL / Postgres / Redis (remote): pick a saved SSH connection from
//     data::connections_raw(), open an SshSession with data::connect_blocking on
//     the background executor, and drive a read-only drill-down over it:
//       database list → table list → columns + a `SELECT * … LIMIT 200` preview.
//     Redis instead lists keys (a bounded `SCAN` loop) and, per key, its
//     `TYPE` / `TTL` / value preview. Every remote command is issued through
//     `ssh exec` against the host's own clients (`mysql` / `psql` / `redis-cli`)
//     and is read-only — no writes or DDL are ever sent, honouring the
//     read-only default.
//
// The query console (SQLite + MySQL + Postgres) only accepts read statements
// (SELECT / WITH / PRAGMA / EXPLAIN / SHOW / DESCRIBE). Results land in a shared
// grid; clicking a row expands its columns inline as a key/value list.
//
// Connection / query failures surface as a single `t.neg` line under the header.

use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, ClipboardItem, Context, FocusHandle, FontWeight, KeyDownEvent,
    MouseButton, MouseDownEvent, SharedString, Window,
};
use gpui_component::{h_flex, v_flex};

use pier_core::services::sqlite::{SqliteClient, SqliteQueryResult};
use pier_core::ssh::{SshConfig, SshSession};

use crate::data;
use crate::i18n;
use crate::theme::Theme;
use crate::ui;

/// Hard cap on rows materialised from one query or preview. Applied while
/// parsing the command output (not just at render) so an unbounded `SELECT *`
/// can't pull a whole table into memory.
const MAX_GRID_ROWS: usize = 500;

/// How many rows the result grid paints. Kept at or below [`MAX_GRID_ROWS`] so
/// the DOM stays light even when more rows are held in memory.
const MAX_RENDER_ROWS: usize = 200;

/// Caps for the Redis key drill-down: at most this many keys are collected, and
/// at most this many SCAN round-trips are issued, so a large keyspace can't
/// balloon the panel or spin forever.
const MAX_REDIS_KEYS: usize = 500;
const MAX_SCAN_ITERS: usize = 32;

/// How many past queries the HISTORY rail keeps (newest first). Bounds both the
/// in-memory list and the persisted file.
const MAX_HISTORY: usize = 200;

/// Which backend the panel is currently driving. All four DB tools share this
/// one View, so the engine is chosen here rather than inferred from the tool.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Engine {
    Sqlite,
    Mysql,
    Postgres,
    Redis,
}

impl Engine {
    fn label(self) -> &'static str {
        match self {
            Engine::Sqlite => "SQLite",
            Engine::Mysql => "MySQL",
            Engine::Postgres => "Postgres",
            Engine::Redis => "Redis",
        }
    }

    /// True for the engines reached over SSH (everything but SQLite).
    fn remote(self) -> bool {
        !matches!(self, Engine::Sqlite)
    }

    /// A single read-only command listing the engine's databases, run over the
    /// SSH session. Relies on the remote host's local auth (peer / ~/.my.cnf);
    /// stderr is merged (`2>&1`) so the caller can surface "Access denied" and
    /// similar failures instead of showing a blank list.
    fn list_command(self) -> &'static str {
        match self {
            Engine::Mysql => "mysql -N -B -e 'SHOW DATABASES' 2>&1",
            Engine::Postgres => {
                "psql -At -c 'SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname' 2>&1"
            }
            Engine::Redis => "redis-cli INFO keyspace 2>&1",
            Engine::Sqlite => "",
        }
    }

    /// Parse the listing command's stdout into database names.
    fn parse_list(self, out: &str) -> Vec<String> {
        match self {
            // Lines look like `db0:keys=1,expires=0,avg_ttl=0`.
            Engine::Redis => out
                .lines()
                .filter_map(|l| {
                    let l = l.trim();
                    let rest = l.strip_prefix("db")?;
                    let idx = rest.find(':')?;
                    Some(format!("db{}", &rest[..idx]))
                })
                .collect(),
            _ => out
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }

    /// Field separator the engine's batch output puts between columns. MySQL's
    /// `-B` mode is tab-delimited (and escapes literal tabs in data); psql is
    /// driven with `-F <US>` so an unescaped tab in a value can't split a row.
    fn sep(self) -> char {
        match self {
            Engine::Postgres => '\u{1f}',
            _ => '\t',
        }
    }

    /// Quote `name` as a table identifier for this engine's SQL dialect.
    fn ident(self, name: &str) -> String {
        match self {
            Engine::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
            _ => format!("`{}`", name.replace('`', "``")),
        }
    }

    /// Read-only command listing the tables in `db` (MySQL / Postgres only).
    fn tables_command(self, db: &str) -> String {
        match self {
            Engine::Mysql => format!("mysql -N -B -e {} {} 2>&1", shq("SHOW TABLES"), shq(db)),
            Engine::Postgres => {
                let sql =
                    "SELECT tablename FROM pg_tables WHERE schemaname='public' ORDER BY tablename";
                format!("psql -w -At -v ON_ERROR_STOP=1 -d {} -c {} 2>&1", shq(db), shq(sql))
            }
            _ => String::new(),
        }
    }

    /// Read-only command describing `table`'s columns.
    fn columns_command(self, db: &str, table: &str) -> String {
        match self {
            Engine::Mysql => {
                let sql = format!("DESCRIBE {}", self.ident(table));
                format!("mysql -N -B -e {} {} 2>&1", shq(&sql), shq(db))
            }
            Engine::Postgres => {
                let sql = format!(
                    "SELECT column_name, data_type, is_nullable FROM information_schema.columns \
                     WHERE table_schema='public' AND table_name={} ORDER BY ordinal_position",
                    sql_lit(table),
                );
                format!(
                    "psql -w -At -F {} -v ON_ERROR_STOP=1 -d {} -c {} 2>&1",
                    shq("\u{1f}"),
                    shq(db),
                    shq(&sql),
                )
            }
            _ => String::new(),
        }
    }

    /// Parse `columns_command` output into normalised columns.
    fn parse_columns(self, out: &str) -> Vec<Col> {
        let sep = self.sep();
        out.lines()
            .filter(|l| !l.is_empty())
            .filter_map(|l| {
                let p: Vec<&str> = l.split(sep).collect();
                let name = p.first().copied().unwrap_or("").to_string();
                if name.is_empty() {
                    return None;
                }
                let ty = p.get(1).copied().unwrap_or("").to_string();
                let key = match self {
                    // MySQL DESCRIBE: Field, Type, Null, Key, …
                    Engine::Mysql => {
                        let null = p.get(2).copied().unwrap_or("");
                        let key = p.get(3).copied().unwrap_or("");
                        if key == "PRI" {
                            "PK"
                        } else if null == "NO" {
                            "NN"
                        } else {
                            ""
                        }
                    }
                    // information_schema.columns: column_name, data_type, is_nullable
                    _ => {
                        if p.get(2).copied().unwrap_or("") == "NO" {
                            "NN"
                        } else {
                            ""
                        }
                    }
                };
                Some(Col {
                    name,
                    ty,
                    key: key.to_string(),
                })
            })
            .collect()
    }

    /// A read-only SQL command (preview / query console) against `db`. Keeps the
    /// header row so the grid has column names; psql's footer is suppressed.
    fn sql_command(self, db: &str, sql: &str) -> String {
        match self {
            Engine::Mysql => format!("mysql -B -e {} {} 2>&1", shq(sql), shq(db)),
            Engine::Postgres => format!(
                "psql -w -A -F {} -P footer=off -v ON_ERROR_STOP=1 -d {} -c {} 2>&1",
                shq("\u{1f}"),
                shq(db),
                shq(sql),
            ),
            _ => String::new(),
        }
    }

    /// A read-only command running `sql` and printing rows with NO header row,
    /// using the engine's column separator. Used by the activity / foreign-key
    /// metadata queries that parse their rows positionally.
    fn headerless_command(self, db: &str, sql: &str) -> String {
        // Activity / kill are server-wide and can run before a database is
        // selected, so the db arg is omitted when empty (the engine connects to
        // its default), rather than passing an invalid empty name.
        match self {
            Engine::Mysql if db.is_empty() => format!("mysql -N -B -e {} 2>&1", shq(sql)),
            Engine::Mysql => format!("mysql -N -B -e {} {} 2>&1", shq(sql), shq(db)),
            Engine::Postgres if db.is_empty() => format!(
                "psql -w -At -F {} -v ON_ERROR_STOP=1 -c {} 2>&1",
                shq("\u{1f}"),
                shq(sql),
            ),
            Engine::Postgres => format!(
                "psql -w -At -F {} -v ON_ERROR_STOP=1 -d {} -c {} 2>&1",
                shq("\u{1f}"),
                shq(db),
                shq(sql),
            ),
            _ => String::new(),
        }
    }

    /// Wrap `sql` in EXPLAIN (or EXPLAIN ANALYZE) and emit a command whose stdout
    /// is the engine's plan *tree text*, with real newlines preserved.
    fn explain_command(self, db: &str, sql: &str, analyze: bool) -> String {
        match self {
            // `--raw` stops `-B` escaping the embedded newlines in the single
            // TREE cell; FORMAT=TREE needs MySQL 8.0.16+ (ANALYZE 8.0.18+) — an
            // older server reports its own error, surfaced verbatim.
            Engine::Mysql => {
                let q = if analyze {
                    format!("EXPLAIN ANALYZE {sql}")
                } else {
                    format!("EXPLAIN FORMAT=TREE {sql}")
                };
                format!("mysql -N -B --raw -e {} {} 2>&1", shq(&q), shq(db))
            }
            // Postgres' default text format is already an indented tree; `-At`
            // prints one plan line per row.
            Engine::Postgres => {
                let q = if analyze {
                    format!("EXPLAIN (ANALYZE, BUFFERS) {sql}")
                } else {
                    format!("EXPLAIN {sql}")
                };
                format!("psql -w -At -v ON_ERROR_STOP=1 -d {} -c {} 2>&1", shq(db), shq(&q))
            }
            _ => String::new(),
        }
    }
}

/// One column of the selected table, normalised across engines.
struct Col {
    name: String,
    ty: String,
    /// Short key marker (`PK` / `NN`), empty when neither applies.
    key: String,
}

/// A generic result grid (columns + rows) rendered by `result_table`, shared by
/// the SQLite query, the remote query console, and the remote table preview.
struct Grid {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    elapsed_ms: u64,
    error: Option<String>,
    /// True when rows were capped at [`MAX_GRID_ROWS`] during parsing, i.e. the
    /// command produced more rows than are held here.
    truncated: bool,
}

impl From<SqliteQueryResult> for Grid {
    fn from(r: SqliteQueryResult) -> Self {
        // Cap rows here (not just at render) so an unbounded `SELECT *` can't
        // pull a whole table into memory, matching the remote `parse_grid` path.
        let mut rows = r.rows;
        let truncated = rows.len() > MAX_GRID_ROWS;
        rows.truncate(MAX_GRID_ROWS);
        Grid {
            columns: r.columns,
            rows,
            elapsed_ms: r.elapsed_ms,
            error: r.error,
            truncated,
        }
    }
}

/// One executed query for the HISTORY rail. Pure frontend — `rows` /
/// `elapsed_ms` are taken from the result that ran; `write` marks DML/DDL
/// (those show no row count, since no portable affected-row count exists).
struct HistEntry {
    sql: String,
    rows: usize,
    elapsed_ms: u64,
    write: bool,
}

/// The `TYPE` / `TTL` / value preview for one selected Redis key.
struct RedisDetail {
    key: String,
    /// Redis type word (`string` / `list` / `set` / `zset` / `hash` / …).
    ty: String,
    /// Human TTL: "no expiry", "missing", or "<n>s".
    ttl: String,
    /// Value preview, one element per line (capped by the read command).
    value: String,
}

/// Which remote drill-down a clickable list row triggers.
#[derive(Clone, Copy)]
enum RowAct {
    Db,
    Table,
    Key,
}

/// A captured EXPLAIN / EXPLAIN ANALYZE plan, rendered as an indented mono tree
/// above the result grid. The text is the engine's own tree output (MySQL
/// `FORMAT=TREE`, Postgres' text plan, SQLite `EXPLAIN QUERY PLAN`).
struct PlanView {
    title: SharedString,
    /// Raw plan text, one tree line per `\n`.
    text: String,
    elapsed_ms: u64,
}

/// One server session row for the activity (processlist) view.
struct ProcRow {
    id: String,
    user: String,
    host: String,
    db: String,
    /// MySQL `Command` / Postgres `state` — the session's current activity.
    command: String,
    time_s: i64,
    /// MySQL `State` / Postgres `wait_event`.
    state: String,
    /// The running statement, if any.
    info: String,
}

/// Which sessions the activity view shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ActFilter {
    All,
    Active,
    Long,
}

/// A foreign key on the selected table, used to build clickable row-detail edges
/// that drill into the referenced table. Multi-column keys AND their components.
struct Fk {
    columns: Vec<String>,
    ref_schema: Option<String>,
    ref_table: String,
    ref_columns: Vec<String>,
}

pub struct DbPanel {
    theme: Theme,
    engine: Engine,

    // SQLite (local) state.
    db_files: Vec<String>,
    open_db: Option<String>,
    tables: Vec<String>,
    selected_table: Option<usize>,
    columns: Vec<Col>,

    // Remote (MySQL / Postgres / Redis) state.
    conns: Vec<SshConfig>,
    selected_conn: Option<usize>,
    /// Live session for the selected host, cached so drill-down reuses it.
    session: Option<SshSession>,
    databases: Vec<String>,
    selected_db: Option<usize>,
    // MySQL / Postgres table drill-down.
    r_tables: Vec<String>,
    r_selected_table: Option<usize>,
    r_columns: Vec<Col>,
    // Redis key drill-down.
    redis_keys: Vec<String>,
    /// True when the key list was capped before the SCAN cursor reached 0.
    redis_keys_truncated: bool,
    selected_key: Option<usize>,
    redis_detail: Option<RedisDetail>,

    // Query console + shared result grid.
    query: String,
    query_focus: FocusHandle,
    /// Write mode. Writes / DDL are rejected unless this is toggled on AND the
    /// user retypes "WRITE" in `write_confirm`. Reset to false after every
    /// successful write and on engine switch / reconnect — the read-only
    /// default (PRODUCT-SPEC §5.5) is never relaxed.
    write_unlocked: bool,
    write_confirm: String,
    write_confirm_focus: FocusHandle,
    /// Recently run queries, newest first (capped at [`MAX_HISTORY`]), persisted
    /// best-effort across restarts.
    history: Vec<HistEntry>,
    result: Option<Grid>,
    /// Index of the result row expanded inline as a key/value list.
    expanded_row: Option<usize>,
    /// Captured EXPLAIN plan, shown above the result grid until dismissed.
    plan: Option<PlanView>,
    /// Foreign keys of the selected table (remote or SQLite), driving row-detail
    /// drill-down edges. Empty when none apply.
    fks: Vec<Fk>,

    // Server activity (processlist). Server-wide, independent of selected_db.
    /// `Some` while the activity view is open, hiding the table drill-down.
    activity: Option<Vec<ProcRow>>,
    activity_filter: ActFilter,
    /// Session id armed for a connection-kill confirm — mirrors the web's
    /// confirm on KILL CONNECTION / pg_terminate_backend (session management is
    /// outside the read-only data guard, same as the Tauri panel).
    kill_confirm: Option<String>,

    // Shared.
    busy: bool,
    error: Option<String>,
    /// Monotonic action counter. Each background action bumps it and captures
    /// the value; a callback whose captured value no longer matches has been
    /// superseded by a newer action and drops its result instead of writing back.
    epoch: u64,
}

impl DbPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
            engine: Engine::Sqlite,
            db_files: discover_db_files(),
            open_db: None,
            tables: Vec::new(),
            selected_table: None,
            columns: Vec::new(),
            conns: data::connections_raw(),
            selected_conn: None,
            session: None,
            databases: Vec::new(),
            selected_db: None,
            r_tables: Vec::new(),
            r_selected_table: None,
            r_columns: Vec::new(),
            redis_keys: Vec::new(),
            redis_keys_truncated: false,
            selected_key: None,
            redis_detail: None,
            query: String::new(),
            query_focus: cx.focus_handle(),
            write_unlocked: false,
            write_confirm: String::new(),
            write_confirm_focus: cx.focus_handle(),
            history: load_history(),
            result: None,
            expanded_row: None,
            plan: None,
            fks: Vec::new(),
            activity: None,
            activity_filter: ActFilter::All,
            kill_confirm: None,
            busy: false,
            error: None,
            epoch: 0,
        }
    }

    fn on_query_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "enter" => {
                self.run_query(cx);
                return;
            }
            "backspace" => {
                if self.query.pop().is_some() {
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        let m = &ks.modifiers;
        if m.control || m.alt || m.platform {
            return;
        }
        if let Some(kc) = &ks.key_char {
            if !kc.is_empty() && !kc.chars().any(|c| c.is_control()) {
                self.query.push_str(kc);
                cx.notify();
            }
        }
    }

    /// Keystrokes for the inline "type WRITE" confirmation box. Mirrors
    /// [`Self::on_query_key`]'s accumulation; Enter runs the query.
    fn on_confirm_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "enter" => {
                self.run_query(cx);
                return;
            }
            "backspace" => {
                if self.write_confirm.pop().is_some() {
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        let m = &ks.modifiers;
        if m.control || m.alt || m.platform {
            return;
        }
        if let Some(kc) = &ks.key_char {
            if !kc.is_empty() && !kc.chars().any(|c| c.is_control()) {
                self.write_confirm.push_str(kc);
                cx.notify();
            }
        }
    }

    /// Run the query box against the open SQLite file or the selected remote
    /// database. Read-only statements run directly; writes / DDL are gated by
    /// the unlock toggle, a single-statement rule, and a retyped "WRITE".
    fn run_query(&mut self, cx: &mut Context<Self>) {
        let sql = self.query.trim().to_string();
        if sql.is_empty() {
            return;
        }
        // A fresh result supersedes any plan shown above the grid.
        self.plan = None;
        // Writes are anything the read classifier rejects. The read-only
        // default is never relaxed (PRODUCT-SPEC §5.5): a write needs the
        // toggle on, a single statement, and a "WRITE" confirmation, and the
        // panel re-locks after each successful write (see run_*_query).
        if !is_readonly_sql(&sql) {
            if !self.write_unlocked {
                self.error = Some(i18n::t("db.read_only_locked").to_string());
                cx.notify();
                return;
            }
            if !is_single_statement(&sql) {
                self.error = Some(i18n::t("db.one_statement").to_string());
                cx.notify();
                return;
            }
            if !self.write_confirm.trim().eq_ignore_ascii_case("WRITE") {
                self.error = Some(i18n::t("db.type_write_confirm").to_string());
                cx.notify();
                return;
            }
        }
        match self.engine {
            Engine::Sqlite => self.run_sqlite_query(sql, cx),
            Engine::Mysql | Engine::Postgres => self.run_remote_query(sql, cx),
            Engine::Redis => {}
        }
    }

    /// Record a finished query at the head of the history rail and persist.
    fn push_history(&mut self, sql: String, rows: usize, elapsed_ms: u64, write: bool) {
        self.history.insert(
            0,
            HistEntry {
                sql,
                rows,
                elapsed_ms,
                write,
            },
        );
        self.history.truncate(MAX_HISTORY);
        save_history(&self.history);
    }

    /// Execute `sql` against the open SQLite file on the background executor.
    fn run_sqlite_query(&mut self, sql: String, cx: &mut Context<Self>) {
        let Some(path) = self.open_db.clone() else {
            return;
        };
        let write = !is_readonly_sql(&sql);
        self.busy = true;
        self.error = None;
        self.expanded_row = None;
        self.epoch += 1;
        let gen = self.epoch;
        cx.notify();
        let sql_exec = sql.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    match SqliteClient::open(&path) {
                        Ok(c) => Ok(c.execute(&sql_exec)),
                        Err(e) => Err(e.to_string()),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != gen {
                    return;
                }
                this.busy = false;
                match res {
                    Ok(r) => {
                        if let Some(err) = &r.error {
                            this.error = Some(err.clone());
                            this.result = Some(Grid::from(r));
                        } else {
                            let grid = Grid::from(r);
                            this.push_history(sql, grid.rows.len(), grid.elapsed_ms, write);
                            if write {
                                this.write_unlocked = false;
                                this.write_confirm.clear();
                            }
                            this.result = Some(grid);
                        }
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Execute `sql` against the selected remote database over SSH exec.
    fn run_remote_query(&mut self, sql: String, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(db) = self
            .selected_db
            .and_then(|d| self.databases.get(d))
            .cloned()
        else {
            return;
        };
        let engine = self.engine;
        let write = !is_readonly_sql(&sql);
        self.busy = true;
        self.error = None;
        self.expanded_row = None;
        self.epoch += 1;
        let gen = self.epoch;
        cx.notify();
        let sql_exec = sql.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let cmd = engine.sql_command(&db, &sql_exec);
                    let start = Instant::now();
                    let (code, out) =
                        session.exec_command_blocking(&cmd).map_err(|e| e.to_string())?;
                    let elapsed = start.elapsed().as_millis() as u64;
                    if code != 0 {
                        return Err(err_text(out, &i18n::t("db.err_query_failed")));
                    }
                    // A write has no portable result grid (`psql` prints a
                    // "INSERT 0 1" status tag, `mysql` nothing); skip parsing
                    // so the UI shows a clean "OK · {ms} ms".
                    if write {
                        Ok::<Grid, String>(Grid {
                            columns: Vec::new(),
                            rows: Vec::new(),
                            elapsed_ms: elapsed,
                            error: None,
                            truncated: false,
                        })
                    } else {
                        Ok::<Grid, String>(parse_grid(&out, engine.sep(), elapsed))
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != gen {
                    return;
                }
                this.busy = false;
                match res {
                    Ok(grid) => {
                        this.push_history(sql, grid.rows.len(), grid.elapsed_ms, write);
                        if write {
                            this.write_unlocked = false;
                            this.write_confirm.clear();
                        }
                        this.result = Some(grid);
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Run EXPLAIN / EXPLAIN ANALYZE on the current query and show the engine's
    /// plan tree above the result grid, without disturbing `self.result`.
    fn run_explain(&mut self, analyze: bool, cx: &mut Context<Self>) {
        let raw = self.query.trim().to_string();
        if raw.is_empty() {
            return;
        }
        let sql = strip_explain_prefix(&raw).to_string();
        if !is_single_statement(&sql) {
            self.error = Some(i18n::t("db.one_statement").to_string());
            cx.notify();
            return;
        }
        // EXPLAIN never executes; EXPLAIN ANALYZE *does*, so a write underneath
        // it must clear the same read-only guard a normal run would, and re-lock
        // after — the read-only default (PRODUCT-SPEC §5.5) is never relaxed.
        let relock = analyze && !is_readonly_sql(&sql);
        if relock {
            if !self.write_unlocked {
                self.error = Some(i18n::t("db.read_only_locked").to_string());
                cx.notify();
                return;
            }
            if !self.write_confirm.trim().eq_ignore_ascii_case("WRITE") {
                self.error = Some(i18n::t("db.type_write_confirm").to_string());
                cx.notify();
                return;
            }
        }
        match self.engine {
            // SQLite uses EXPLAIN QUERY PLAN, which never executes the statement.
            Engine::Sqlite => self.run_sqlite_explain(sql, cx),
            Engine::Mysql | Engine::Postgres => self.run_remote_explain(sql, analyze, relock, cx),
            Engine::Redis => {}
        }
    }

    fn run_remote_explain(&mut self, sql: String, analyze: bool, relock: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(db) = self
            .selected_db
            .and_then(|d| self.databases.get(d))
            .cloned()
        else {
            return;
        };
        let engine = self.engine;
        let title = if analyze {
            i18n::t("db.explain_analyze")
        } else {
            i18n::t("db.explain")
        };
        self.busy = true;
        self.error = None;
        self.epoch += 1;
        let gen = self.epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let cmd = engine.explain_command(&db, &sql, analyze);
                    let start = Instant::now();
                    let (code, out) =
                        session.exec_command_blocking(&cmd).map_err(|e| e.to_string())?;
                    let elapsed = start.elapsed().as_millis() as u64;
                    if code != 0 {
                        return Err(err_text(out, &i18n::t("db.err_query_failed")));
                    }
                    Ok::<(String, u64), String>((out.trim_end().to_string(), elapsed))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != gen {
                    return;
                }
                this.busy = false;
                match res {
                    Ok((text, elapsed_ms)) => {
                        // Re-lock after an EXPLAIN ANALYZE that executed a write.
                        if relock {
                            this.write_unlocked = false;
                            this.write_confirm.clear();
                        }
                        this.plan = Some(PlanView {
                            title,
                            text,
                            elapsed_ms,
                        });
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run_sqlite_explain(&mut self, sql: String, cx: &mut Context<Self>) {
        let Some(path) = self.open_db.clone() else {
            return;
        };
        let title = i18n::t("db.explain");
        self.busy = true;
        self.error = None;
        self.epoch += 1;
        let gen = self.epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    match SqliteClient::open(&path) {
                        Ok(c) => Ok(c.execute(&format!("EXPLAIN QUERY PLAN {sql}"))),
                        Err(e) => Err(e.to_string()),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != gen {
                    return;
                }
                this.busy = false;
                match res {
                    Ok(r) => {
                        if let Some(err) = r.error {
                            this.error = Some(err);
                        } else {
                            this.plan = Some(PlanView {
                                title,
                                text: sqlite_plan_tree(&r.columns, &r.rows),
                                elapsed_ms: r.elapsed_ms,
                            });
                        }
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Open (or refresh) the server activity view — `SHOW FULL PROCESSLIST`
    /// (MySQL) / `pg_stat_activity` (Postgres). Server-wide; needs only a live
    /// session, not a selected database.
    fn load_activity(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let engine = self.engine;
        let db = self
            .selected_db
            .and_then(|d| self.databases.get(d))
            .cloned()
            .unwrap_or_default();
        self.kill_confirm = None;
        self.busy = true;
        self.error = None;
        self.epoch += 1;
        let gen = self.epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let sql = match engine {
                        Engine::Mysql => "SHOW FULL PROCESSLIST".to_string(),
                        Engine::Postgres => PG_ACTIVITY_SQL.to_string(),
                        _ => return Ok::<Vec<ProcRow>, String>(Vec::new()),
                    };
                    let cmd = engine.headerless_command(&db, &sql);
                    let (code, out) =
                        session.exec_command_blocking(&cmd).map_err(|e| e.to_string())?;
                    if code != 0 {
                        return Err(err_text(out, &i18n::t("db.err_query_failed")));
                    }
                    Ok(parse_proc_rows(engine, &out))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != gen {
                    return;
                }
                this.busy = false;
                match res {
                    Ok(rows) => this.activity = Some(rows),
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Cancel a running query (`terminate = false`) or drop a whole session
    /// (`terminate = true`). Session management lives outside the read-only data
    /// guard, matching the Tauri panel; the caller gates `terminate` behind a
    /// confirm. `id` is validated as an integer before it reaches the command.
    fn kill_proc(&mut self, id: String, terminate: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Ok(num) = id.trim().parse::<u64>() else {
            return;
        };
        let engine = self.engine;
        let db = self
            .selected_db
            .and_then(|d| self.databases.get(d))
            .cloned()
            .unwrap_or_default();
        self.kill_confirm = None;
        self.busy = true;
        self.error = None;
        self.epoch += 1;
        let gen = self.epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let sql = match (engine, terminate) {
                        (Engine::Mysql, false) => format!("KILL QUERY {num}"),
                        (Engine::Mysql, true) => format!("KILL {num}"),
                        (Engine::Postgres, false) => format!("SELECT pg_cancel_backend({num})"),
                        (Engine::Postgres, true) => format!("SELECT pg_terminate_backend({num})"),
                        _ => return Ok::<(), String>(()),
                    };
                    let cmd = engine.headerless_command(&db, &sql);
                    let (code, out) =
                        session.exec_command_blocking(&cmd).map_err(|e| e.to_string())?;
                    if code != 0 {
                        return Err(err_text(out, &i18n::t("db.err_query_failed")));
                    }
                    Ok(())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != gen {
                    return;
                }
                this.busy = false;
                match res {
                    // Reload so the killed session drops out of the list.
                    Ok(()) => this.load_activity(cx),
                    Err(e) => {
                        this.error = Some(e);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// Run a foreign-key drill-down: load `sql` into the console and execute it.
    fn navigate_fk(&mut self, sql: String, cx: &mut Context<Self>) {
        self.query = sql;
        self.run_query(cx);
    }

    // ── Actions (all blocking work happens on the background executor) ──

    /// Re-scan the working dir for SQLite files and reload saved connections.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.db_files = discover_db_files();
        self.conns = data::connections_raw();
        cx.notify();
    }

    /// Clear every remote drill-down field — used on engine switch and before a
    /// fresh connect so stale tables / keys never bleed across hosts. Also
    /// re-locks writes: both `engine_chip` (engine switch) and `connect_remote`
    /// route through here, so a fresh engine / host always starts read-only.
    fn reset_remote(&mut self) {
        self.session = None;
        self.selected_conn = None;
        self.databases.clear();
        self.selected_db = None;
        self.r_tables.clear();
        self.r_selected_table = None;
        self.r_columns.clear();
        self.redis_keys.clear();
        self.redis_keys_truncated = false;
        self.selected_key = None;
        self.redis_detail = None;
        self.write_unlocked = false;
        self.write_confirm.clear();
        self.activity = None;
        self.kill_confirm = None;
        self.plan = None;
        self.fks.clear();
    }

    /// Open a SQLite file and list its tables.
    fn open_sqlite(&mut self, path: String, cx: &mut Context<Self>) {
        self.open_db = Some(path.clone());
        self.tables.clear();
        self.selected_table = None;
        self.columns.clear();
        self.result = None;
        self.expanded_row = None;
        self.plan = None;
        self.fks.clear();
        self.error = None;
        // Switching files re-locks writes, same as reset_remote on engine
        // switch / reconnect: the read-only default (PRODUCT-SPEC §5.5) is never
        // relaxed, so an unlock armed against the previous file must not carry
        // over to this one.
        self.write_unlocked = false;
        self.write_confirm.clear();
        self.busy = true;
        self.epoch += 1;
        let gen = self.epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    match SqliteClient::open(&path) {
                        Ok(c) => c.list_tables().map_err(|e| e.to_string()),
                        Err(e) => Err(e.to_string()),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != gen {
                    return;
                }
                this.busy = false;
                match res {
                    Ok(tables) => this.tables = tables,
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Load column metadata for the table at `idx` in the open SQLite file.
    fn open_table(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(path) = self.open_db.clone() else {
            return;
        };
        let Some(table) = self.tables.get(idx).cloned() else {
            return;
        };
        self.selected_table = Some(idx);
        self.columns.clear();
        self.fks.clear();
        self.plan = None;
        self.error = None;
        self.busy = true;
        self.epoch += 1;
        let gen = self.epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    match SqliteClient::open(&path) {
                        Ok(c) => {
                            let cols: Vec<Col> = c
                                .table_columns(&table)
                                .map_err(|e| e.to_string())?
                                .into_iter()
                                .map(|c| Col {
                                    name: c.name,
                                    ty: c.col_type,
                                    key: if c.primary_key {
                                        "PK".into()
                                    } else if c.not_null {
                                        "NN".into()
                                    } else {
                                        String::new()
                                    },
                                })
                                .collect();
                            let fk_q = format!(
                                "PRAGMA foreign_key_list(\"{}\")",
                                table.replace('"', "\"\"")
                            );
                            let fks = parse_sqlite_fks(&c.execute(&fk_q));
                            Ok::<(Vec<Col>, Vec<Fk>), String>((cols, fks))
                        }
                        Err(e) => Err(e.to_string()),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != gen {
                    return;
                }
                this.busy = false;
                match res {
                    Ok((cols, fks)) => {
                        this.columns = cols;
                        this.fks = fks;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Open an SSH session to the saved connection at `idx`, cache it, and run
    /// the current engine's read-only database-listing command over it.
    fn connect_remote(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(cfg) = self.conns.get(idx).cloned() else {
            return;
        };
        let engine = self.engine;
        self.reset_remote();
        self.selected_conn = Some(idx);
        self.result = None;
        self.expanded_row = None;
        self.error = None;
        self.busy = true;
        self.epoch += 1;
        let gen = self.epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let session = data::connect_blocking(&cfg)?;
                    let (code, out) = session
                        .exec_command_blocking(engine.list_command())
                        .map_err(|e| e.to_string())?;
                    if code != 0 {
                        return Err(err_text(out, &i18n::t("db.err_list_databases")));
                    }
                    let dbs = engine.parse_list(&out);
                    Ok::<(SshSession, Vec<String>), String>((session, dbs))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != gen {
                    return;
                }
                this.busy = false;
                match res {
                    Ok((session, dbs)) => {
                        this.session = Some(session);
                        this.databases = dbs;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Select the database at `idx`: list its tables (MySQL / Postgres) or scan
    /// its keys (Redis) over the cached session.
    fn select_db(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(db) = self.databases.get(idx).cloned() else {
            return;
        };
        let engine = self.engine;
        self.selected_db = Some(idx);
        self.r_tables.clear();
        self.r_selected_table = None;
        self.r_columns.clear();
        self.fks.clear();
        self.plan = None;
        self.redis_keys.clear();
        self.redis_keys_truncated = false;
        self.selected_key = None;
        self.redis_detail = None;
        self.result = None;
        self.expanded_row = None;
        self.error = None;
        self.busy = true;
        self.epoch += 1;
        let gen = self.epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    if matches!(engine, Engine::Redis) {
                        let n = redis_db_index(&db);
                        let mut cursor = "0".to_string();
                        let mut keys: Vec<String> = Vec::new();
                        let mut iters = 0usize;
                        // SCAN is cursor-paginated; loop until the cursor wraps
                        // back to 0, bounded by MAX_REDIS_KEYS / MAX_SCAN_ITERS
                        // so a large keyspace can't spin or balloon the panel.
                        let more = loop {
                            let cmd = format!("redis-cli -n {n} SCAN {cursor} COUNT 200 2>&1");
                            let (code, out) =
                                session.exec_command_blocking(&cmd).map_err(|e| e.to_string())?;
                            if code != 0 {
                                return Err(err_text(out, &i18n::t("db.err_scan")));
                            }
                            // First line is the next cursor; the rest are keys.
                            let mut lines = out.lines();
                            cursor = lines.next().unwrap_or("0").trim().to_string();
                            for l in lines {
                                let l = l.trim();
                                if !l.is_empty() {
                                    keys.push(l.to_string());
                                }
                            }
                            iters += 1;
                            if cursor == "0" {
                                break false;
                            }
                            if keys.len() >= MAX_REDIS_KEYS || iters >= MAX_SCAN_ITERS {
                                break true;
                            }
                        };
                        let truncated = more || keys.len() > MAX_REDIS_KEYS;
                        keys.truncate(MAX_REDIS_KEYS);
                        Ok::<(Vec<String>, bool), String>((keys, truncated))
                    } else {
                        let cmd = engine.tables_command(&db);
                        let (code, out) =
                            session.exec_command_blocking(&cmd).map_err(|e| e.to_string())?;
                        if code != 0 {
                            return Err(err_text(out, &i18n::t("db.err_list_tables")));
                        }
                        let tables = out
                            .lines()
                            .map(|l| l.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        Ok::<(Vec<String>, bool), String>((tables, false))
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != gen {
                    return;
                }
                this.busy = false;
                match res {
                    Ok((list, truncated)) => {
                        if matches!(engine, Engine::Redis) {
                            this.redis_keys = list;
                            this.redis_keys_truncated = truncated;
                        } else {
                            this.r_tables = list;
                        }
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Load columns for the remote table at `idx` and run its preview SELECT.
    fn open_remote_table(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(db) = self
            .selected_db
            .and_then(|d| self.databases.get(d))
            .cloned()
        else {
            return;
        };
        let Some(table) = self.r_tables.get(idx).cloned() else {
            return;
        };
        let engine = self.engine;
        self.r_selected_table = Some(idx);
        self.r_columns.clear();
        self.fks.clear();
        self.plan = None;
        self.result = None;
        self.expanded_row = None;
        self.error = None;
        self.busy = true;
        self.epoch += 1;
        let gen = self.epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let cols_cmd = engine.columns_command(&db, &table);
                    let (cc, cout) = session
                        .exec_command_blocking(&cols_cmd)
                        .map_err(|e| e.to_string())?;
                    if cc != 0 {
                        return Err(err_text(cout, &i18n::t("db.err_list_columns")));
                    }
                    let columns = engine.parse_columns(&cout);

                    let preview_sql = format!("SELECT * FROM {} LIMIT 200", engine.ident(&table));
                    let prev_cmd = engine.sql_command(&db, &preview_sql);
                    let start = Instant::now();
                    let (pc, pout) = session
                        .exec_command_blocking(&prev_cmd)
                        .map_err(|e| e.to_string())?;
                    let elapsed = start.elapsed().as_millis() as u64;
                    if pc != 0 {
                        return Err(err_text(pout, &i18n::t("db.err_preview")));
                    }
                    let grid = parse_grid(&pout, engine.sep(), elapsed);
                    // Foreign keys are best-effort: a metadata failure must not
                    // sink the table preview, so any error yields no edges.
                    let fk_cmd = engine.headerless_command(&db, &fk_sql(engine, &table));
                    let fks = match session.exec_command_blocking(&fk_cmd) {
                        Ok((0, fout)) => parse_fks(engine, &fout),
                        _ => Vec::new(),
                    };
                    Ok::<(Vec<Col>, Grid, Vec<Fk>), String>((columns, grid, fks))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != gen {
                    return;
                }
                this.busy = false;
                match res {
                    Ok((cols, grid, fks)) => {
                        this.r_columns = cols;
                        this.result = Some(grid);
                        this.fks = fks;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Select the Redis key at `idx` and fetch its type, TTL, and value preview.
    fn select_key(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(db) = self
            .selected_db
            .and_then(|d| self.databases.get(d))
            .cloned()
        else {
            return;
        };
        let Some(key) = self.redis_keys.get(idx).cloned() else {
            return;
        };
        self.selected_key = Some(idx);
        self.redis_detail = None;
        self.error = None;
        self.busy = true;
        self.epoch += 1;
        let gen = self.epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let n = redis_db_index(&db);
                    let kq = shq(&key);
                    // TYPE + TTL in one round-trip; value command depends on type.
                    let head = format!(
                        "redis-cli -n {n} TYPE {kq} 2>&1; redis-cli -n {n} TTL {kq} 2>&1"
                    );
                    let (_c, hout) = session
                        .exec_command_blocking(&head)
                        .map_err(|e| e.to_string())?;
                    let mut hl = hout.lines();
                    let ty = hl.next().unwrap_or("").trim().to_string();
                    let ttl_raw = hl.next().unwrap_or("").trim().to_string();
                    let val_cmd = redis_value_command(&n, &ty, &key);
                    let value = if val_cmd.is_empty() {
                        String::new()
                    } else {
                        session
                            .exec_command_blocking(&val_cmd)
                            .map_err(|e| e.to_string())?
                            .1
                            .trim_end()
                            .to_string()
                    };
                    Ok::<RedisDetail, String>(RedisDetail {
                        key,
                        ty,
                        ttl: human_ttl(&ttl_raw),
                        value,
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != gen {
                    return;
                }
                this.busy = false;
                match res {
                    Ok(detail) => this.redis_detail = Some(detail),
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ── Chrome ───────────────────────────────────────────────────

    fn engine_chip(&self, cx: &mut Context<Self>, e: Engine) -> impl IntoElement {
        let t = &self.theme;
        let active = self.engine == e;
        h_flex()
            .id(SharedString::from(format!("eng-{}", e.label())))
            .items_center()
            .px(t.sp2)
            .py(px(3.0))
            .rounded(t.radius_sm)
            .text_size(t.fs_ui)
            .text_color(if active { t.ink } else { t.muted })
            .when(active, |d| d.bg(t.accent_dim))
            .when(!active, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    if this.engine != e {
                        this.engine = e;
                        this.reset_remote();
                        this.result = None;
                        this.expanded_row = None;
                        this.error = None;
                        this.epoch += 1;
                        cx.notify();
                    }
                }),
            )
            .child(e.label())
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .w_full()
            .px(t.sp3)
            .py(t.sp2)
            .border_b_1()
            .border_color(t.line)
            .child(self.engine_chip(cx, Engine::Sqlite))
            .child(self.engine_chip(cx, Engine::Mysql))
            .child(self.engine_chip(cx, Engine::Postgres))
            .child(self.engine_chip(cx, Engine::Redis))
            .child(div().flex_1())
            .child(
                div()
                    .id("db-reload")
                    .px(t.sp3)
                    .py(px(3.0))
                    .rounded(t.radius_sm)
                    .bg(t.panel_2)
                    .text_size(t.fs_ui)
                    .text_color(t.ink_2)
                    .hover(|s| s.bg(t.elev))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| this.reload(cx)),
                    )
                    .child(i18n::t("db.reload")),
            )
    }

    // ── Bodies ───────────────────────────────────────────────────

    fn sqlite_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let mut col = v_flex().pb(t.sp3);

        col = col.child(ui::section_label(
            t,
            i18n::tf("db.databases_count", &[&self.db_files.len().to_string()]),
        ));
        if self.db_files.is_empty() {
            col = col.child(hint(t, i18n::t("db.no_db_files")));
        } else {
            for (i, path) in self.db_files.iter().enumerate() {
                let selected = self.open_db.as_deref() == Some(path.as_str());
                let p = path.clone();
                col = col.child(
                    h_flex()
                        .id(SharedString::from(format!("dbf-{i}")))
                        .items_center()
                        .gap(t.sp2)
                        .h(px(26.0))
                        .px(t.sp3)
                        .when(selected, |d| d.bg(t.accent_dim))
                        .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                                this.open_sqlite(p.clone(), cx)
                            }),
                        )
                        .child(ui::icon("database", px(14.0), if selected { t.accent } else { t.muted }))
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .text_color(if selected { t.ink } else { t.ink_2 })
                                .child(file_name(path)),
                        ),
                );
            }
        }

        if self.open_db.is_some() {
            col = col.child(ui::section_label(
                t,
                i18n::tf("db.tables_count", &[&self.tables.len().to_string()]),
            ));
            if self.tables.is_empty() && !self.busy {
                col = col.child(hint(t, i18n::t("db.no_tables")));
            }
            for (i, table) in self.tables.iter().enumerate() {
                let selected = self.selected_table == Some(i);
                col = col.child(
                    h_flex()
                        .id(SharedString::from(format!("tbl-{i}")))
                        .items_center()
                        .gap(t.sp2)
                        .h(px(26.0))
                        .px(t.sp3)
                        .when(selected, |d| d.bg(t.accent_dim))
                        .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                                this.open_table(i, cx)
                            }),
                        )
                        .child(ui::icon("layers", px(14.0), if selected { t.accent } else { t.muted }))
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .font_family(t.mono.clone())
                                .text_size(t.fs_sm)
                                .text_color(if selected { t.ink } else { t.ink_2 })
                                .child(table.clone()),
                        ),
                );
            }
        }

        if self.selected_table.is_some() {
            col = col.child(ui::section_label(
                t,
                i18n::tf("db.columns_count", &[&self.columns.len().to_string()]),
            ));
            for c in &self.columns {
                col = col.child(column_row(t, c));
            }
        }

        if self.open_db.is_some() {
            col = col
                .child(ui::section_label(t, i18n::t("db.query_label")))
                .child(self.query_console(cx))
                .child(self.write_bar(cx))
                .child(self.plan_view(cx))
                .child(self.result_table(cx))
                .child(self.history_section(cx));
        }

        col.into_any_element()
    }

    fn remote_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let mut col = v_flex().pb(t.sp3);

        col = col.child(ui::section_label(
            t,
            i18n::tf("db.connections_count", &[&self.conns.len().to_string()]),
        ));
        if self.conns.is_empty() {
            col = col.child(hint(t, i18n::t("side.no_saved_connections")));
        } else {
            for (i, c) in self.conns.iter().enumerate() {
                let selected = self.selected_conn == Some(i);
                let addr = format!("{}@{}:{}", c.user, c.host, c.port);
                let name = c.name.clone();
                col = col.child(
                    h_flex()
                        .id(SharedString::from(format!("conn-{i}")))
                        .items_center()
                        .gap(t.sp2)
                        .h(px(42.0))
                        .px(t.sp3)
                        .when(selected, |d| d.bg(t.accent_dim))
                        .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                                this.connect_remote(i, cx)
                            }),
                        )
                        .child(ui::status_dot(if selected { t.accent } else { t.muted }))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .child(
                                    div()
                                        .overflow_hidden()
                                        .text_color(if selected { t.ink } else { t.ink_2 })
                                        .child(name),
                                )
                                .child(
                                    div()
                                        .overflow_hidden()
                                        .font_family(t.mono.clone())
                                        .text_size(t.fs_sm)
                                        .text_color(t.muted)
                                        .child(addr),
                                ),
                        ),
                );
            }
        }

        // Server activity (processlist) — available once a MySQL / Postgres
        // session is live; when open it replaces the table drill-down.
        if self.session.is_some() && matches!(self.engine, Engine::Mysql | Engine::Postgres) {
            col = col.child(self.activity_button(cx));
        }
        if let Some(rows) = &self.activity {
            col = col.child(self.activity_view(cx, rows));
            return col.into_any_element();
        }

        if self.session.is_some() {
            col = col.child(ui::section_label(
                t,
                i18n::tf("db.databases_count", &[&self.databases.len().to_string()]),
            ));
            if self.databases.is_empty() && !self.busy && self.error.is_none() {
                col = col.child(hint(t, i18n::t("db.no_databases_readonly")));
            }
            for (i, db) in self.databases.iter().enumerate() {
                let selected = self.selected_db == Some(i);
                col = col.child(self.nav_row(cx, RowAct::Db, i, "database", db.clone(), false, selected));
            }
        }

        if self.selected_db.is_some() {
            if matches!(self.engine, Engine::Redis) {
                col = col.child(ui::section_label(
                    t,
                    i18n::tf(
                        "db.keys_count",
                        &[
                            &self.redis_keys.len().to_string(),
                            if self.redis_keys_truncated { "+" } else { "" },
                        ],
                    ),
                ));
                if self.redis_keys.is_empty() && !self.busy {
                    col = col.child(hint(t, i18n::t("db.no_keys")));
                }
                for (i, k) in self.redis_keys.iter().enumerate() {
                    let selected = self.selected_key == Some(i);
                    col = col.child(self.nav_row(cx, RowAct::Key, i, "asterisk", k.clone(), true, selected));
                }
                if self.redis_keys_truncated {
                    col = col.child(hint(
                        t,
                        i18n::tf("db.keys_more", &[&self.redis_keys.len().to_string()]),
                    ));
                }
                if let Some(detail) = &self.redis_detail {
                    col = col.child(self.redis_detail_view(detail));
                }
            } else {
                col = col.child(ui::section_label(
                    t,
                    i18n::tf("db.tables_count", &[&self.r_tables.len().to_string()]),
                ));
                if self.r_tables.is_empty() && !self.busy {
                    col = col.child(hint(t, i18n::t("db.no_tables")));
                }
                for (i, table) in self.r_tables.iter().enumerate() {
                    let selected = self.r_selected_table == Some(i);
                    col = col.child(self.nav_row(cx, RowAct::Table, i, "layers", table.clone(), true, selected));
                }

                if self.r_selected_table.is_some() {
                    col = col.child(ui::section_label(
                        t,
                        i18n::tf("db.columns_count", &[&self.r_columns.len().to_string()]),
                    ));
                    for c in &self.r_columns {
                        col = col.child(column_row(t, c));
                    }
                }

                col = col
                    .child(ui::section_label(t, i18n::t("db.query_label")))
                    .child(self.query_console(cx))
                    .child(self.write_bar(cx))
                    .child(self.plan_view(cx))
                    .child(self.result_table(cx))
                    .child(self.history_section(cx));
            }
        }

        col.into_any_element()
    }

    /// A clickable single-line list row (database / table / Redis key).
    fn nav_row(
        &self,
        cx: &mut Context<Self>,
        act: RowAct,
        i: usize,
        glyph: &'static str,
        label: String,
        mono: bool,
        selected: bool,
    ) -> impl IntoElement {
        let t = &self.theme;
        let tag = match act {
            RowAct::Db => "rdb",
            RowAct::Table => "rtbl",
            RowAct::Key => "rkey",
        };
        let text = div()
            .flex_1()
            .overflow_hidden()
            .text_color(if selected { t.ink } else { t.ink_2 });
        let text = if mono {
            text.font_family(t.mono.clone()).text_size(t.fs_sm)
        } else {
            text
        };
        h_flex()
            .id(SharedString::from(format!("{tag}-{i}")))
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .when(selected, |d| d.bg(t.accent_dim))
            .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| match act {
                    RowAct::Db => this.select_db(i, cx),
                    RowAct::Table => this.open_remote_table(i, cx),
                    RowAct::Key => this.select_key(i, cx),
                }),
            )
            .child(ui::icon(glyph, px(14.0), if selected { t.accent } else { t.muted }))
            .child(text.child(label))
    }

    /// The selected Redis key's type badge, TTL, and value preview.
    fn redis_detail_view(&self, d: &RedisDetail) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .gap(t.sp2)
            .mx(t.sp3)
            .mt(t.sp2)
            .p(t.sp2)
            .rounded(t.radius_sm)
            .bg(t.panel_2)
            .border_1()
            .border_color(t.line_2)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .child(
                        div()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(t.radius_sm)
                            .bg(t.accent_subtle)
                            .font_family(t.mono.clone())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(t.fs_sm)
                            .text_color(t.accent)
                            .child(redis_type_badge(&d.ty)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.ink)
                            .child(d.key.clone()),
                    )
                    .child(
                        div()
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(i18n::tf("db.ttl", &[&d.ttl])),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink_2)
                    .child(if d.value.is_empty() {
                        i18n::t("db.value_empty").to_string()
                    } else {
                        d.value.clone()
                    }),
            )
    }

    /// Query input + Run button (SQLite + MySQL / Postgres).
    fn query_console(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let empty = self.query.is_empty();
        h_flex()
            .gap(t.sp2)
            .px(t.sp3)
            .pb(t.sp2)
            .child(
                div()
                    .track_focus(&self.query_focus)
                    .key_context("SqlQuery")
                    .on_key_down(cx.listener(Self::on_query_key))
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(30.0))
                    .px(t.sp2)
                    .flex()
                    .items_center()
                    .rounded(t.radius_sm)
                    .bg(t.panel_2)
                    .border_1()
                    .border_color(t.line_2)
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .when(empty, |d| d.text_color(t.dim).child(i18n::t("db.select_readonly_ph")))
                    .when(!empty, |d| d.text_color(t.ink).child(self.query.clone())),
            )
            .child(
                div()
                    .id("sql-explain")
                    .px(t.sp2)
                    .py(px(5.0))
                    .rounded(t.radius_sm)
                    .bg(t.panel_2)
                    .text_color(t.ink_2)
                    .text_size(t.fs_ui)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.elev))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| this.run_explain(false, cx)),
                    )
                    .child(i18n::t("db.explain")),
            )
            .when(self.engine != Engine::Sqlite, |row| {
                row.child(
                    div()
                        .id("sql-analyze")
                        .px(t.sp2)
                        .py(px(5.0))
                        .rounded(t.radius_sm)
                        .bg(t.panel_2)
                        .text_color(t.ink_2)
                        .text_size(t.fs_ui)
                        .cursor_pointer()
                        .hover(|s| s.bg(t.elev))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                                this.run_explain(true, cx)
                            }),
                        )
                        .child(i18n::t("db.explain_analyze_short")),
                )
            })
            .child(
                div()
                    .id("sql-run")
                    .px(t.sp3)
                    .py(px(5.0))
                    .rounded(t.radius_sm)
                    .bg(t.accent)
                    .text_color(t.accent_ink)
                    .text_size(t.fs_ui)
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| this.run_query(cx)),
                    )
                    .child(i18n::t("db.run")),
            )
    }

    /// The write-mode bar under the query console (mirrors `DbSqlEditor.tsx`'s
    /// footer): a lock toggle, a hint line, and — when the current statement is
    /// a write and writes are unlocked — the inline "type WRITE" confirmation.
    /// Read-only is the default; `run_query` enforces the guard and re-locks.
    fn write_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let unlocked = self.write_unlocked;
        let q = self.query.trim();
        let show_confirm = unlocked && !q.is_empty() && !is_readonly_sql(q);
        // No lock.svg in the icon set; triangle-alert doubles as the hazard
        // glyph — tinted `warn` when unlocked, muted when read-only.
        let glyph_color = if unlocked { t.warn } else { t.muted };

        let mut row = h_flex()
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .pb(t.sp2)
            .child(
                h_flex()
                    .id("db-write-lock")
                    .items_center()
                    .gap(t.sp1)
                    .px(t.sp2)
                    .py(px(3.0))
                    .rounded(t.radius_sm)
                    .bg(t.panel_2)
                    .text_size(t.fs_ui)
                    .text_color(glyph_color)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.elev))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                            this.write_unlocked = !this.write_unlocked;
                            if !this.write_unlocked {
                                this.write_confirm.clear();
                            }
                            cx.notify();
                        }),
                    )
                    .child(ui::icon("triangle-alert", px(12.0), glyph_color))
                    .child(if unlocked {
                        i18n::t("db.writes_unlocked")
                    } else {
                        i18n::t("db.read_only")
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child(if unlocked {
                        i18n::t("db.dml_will_run")
                    } else {
                        i18n::t("db.unlock_hint")
                    }),
            );

        if show_confirm {
            let empty = self.write_confirm.is_empty();
            row = row.child(
                div()
                    .track_focus(&self.write_confirm_focus)
                    .key_context("SqlWriteConfirm")
                    .on_key_down(cx.listener(Self::on_confirm_key))
                    .w(px(180.0))
                    .h(px(26.0))
                    .px(t.sp2)
                    .flex()
                    .items_center()
                    .rounded(t.radius_sm)
                    .bg(t.panel_2)
                    .border_1()
                    .border_color(t.warn)
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .when(empty, |d| {
                        d.text_color(t.dim).child(i18n::t("db.type_write_ph"))
                    })
                    .when(!empty, |d| {
                        d.text_color(t.ink).child(self.write_confirm.clone())
                    }),
            );
        }
        row
    }

    /// The HISTORY rail: recent queries newest-first. Clicking a row loads its
    /// SQL back into the query box. Hidden when empty.
    fn history_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        if self.history.is_empty() {
            return div().into_any_element();
        }
        let mut col = v_flex().child(ui::section_label(
            t,
            i18n::tf("db.history_count", &[&self.history.len().to_string()]),
        ));
        for (i, e) in self.history.iter().enumerate() {
            let sql = e.sql.clone();
            // Writes have no portable affected-row count, so don't invent one.
            let meta = if e.write {
                i18n::tf("db.hist_write", &[&e.elapsed_ms.to_string()])
            } else {
                i18n::tf(
                    "db.hist_rows",
                    &[&e.rows.to_string(), &e.elapsed_ms.to_string()],
                )
            };
            col = col.child(
                h_flex()
                    .id(SharedString::from(format!("hist-{i}")))
                    .items_start()
                    .gap(t.sp2)
                    .px(t.sp3)
                    .py(px(4.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.query = sql.clone();
                            cx.notify();
                        }),
                    )
                    .child(ui::icon(
                        if e.write { "triangle-alert" } else { "play" },
                        px(12.0),
                        if e.write { t.warn } else { t.muted },
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(
                                div()
                                    .overflow_hidden()
                                    .font_family(t.mono.clone())
                                    .text_size(t.fs_sm)
                                    .text_color(t.ink_2)
                                    .child(e.sql.clone()),
                            )
                            .child(div().text_size(t.fs_sm).text_color(t.muted).child(meta)),
                    ),
            );
        }
        col.into_any_element()
    }

    /// A "Copy TSV" / "Copy CSV" button for the result header. `label` is an
    /// i18n key (`db.copy_tsv` / `db.copy_csv`).
    fn copy_btn(&self, cx: &mut Context<Self>, label: &'static str, csv: bool) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(SharedString::from(if csv { "db-copy-csv" } else { "db-copy-tsv" }))
            .px(t.sp2)
            .py(px(2.0))
            .rounded(t.radius_sm)
            .bg(t.panel_2)
            .text_size(t.fs_sm)
            .text_color(t.ink_2)
            .cursor_pointer()
            .hover(|s| s.bg(t.elev))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| this.copy_result(csv, cx)),
            )
            .child(i18n::t(label))
    }

    /// Copy the current result grid to the clipboard as TSV or CSV.
    fn copy_result(&mut self, csv: bool, cx: &mut Context<Self>) {
        let Some(grid) = &self.result else {
            return;
        };
        if grid.columns.is_empty() {
            return;
        }
        let text = if csv {
            grid_to_csv(grid)
        } else {
            grid_to_tsv(grid)
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// Render the last query / preview result as a scrollable table (capped at
    /// 200 rows). Clicking a row expands its columns inline as key/value pairs.
    /// The EXPLAIN plan tree, shown above the result grid until dismissed.
    fn plan_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let Some(p) = &self.plan else {
            return div().into_any_element();
        };
        let mut body = v_flex()
            .id("db-plan")
            .mx(t.sp3)
            .my(t.sp2)
            .p(t.sp2)
            .gap(px(1.0))
            .rounded(t.radius_sm)
            .bg(t.panel_2)
            .border_1()
            .border_color(t.line_2)
            .overflow_x_scroll();
        if p.text.is_empty() {
            body = body.child(hint(t, i18n::t("db.plan_empty")));
        } else {
            for line in p.text.lines() {
                body = body.child(
                    div()
                        .font_family(t.mono.clone())
                        .text_size(t.fs_sm)
                        .text_color(t.ink_2)
                        .child(line.replace('\t', "    ")),
                );
            }
        }
        v_flex()
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp1)
                    .pr(t.sp3)
                    .child(ui::section_label(
                        t,
                        i18n::tf("db.plan_meta", &[&p.title, &p.elapsed_ms.to_string()]),
                    ))
                    .child(div().flex_1())
                    .child(
                        h_flex()
                            .id("db-plan-close")
                            .items_center()
                            .gap(t.sp1)
                            .px(t.sp2)
                            .py(px(2.0))
                            .rounded(t.radius_sm)
                            .text_size(t.fs_ui)
                            .text_color(t.ink_2)
                            .cursor_pointer()
                            .hover(|s| s.bg(t.elev))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                                    this.plan = None;
                                    cx.notify();
                                }),
                            )
                            .child(ui::icon("close", px(11.0), t.muted))
                            .child(i18n::t("common.close")),
                    ),
            )
            .child(body)
            .into_any_element()
    }

    /// The "Server activity" launcher row (MySQL / Postgres, when connected).
    fn activity_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        h_flex().px(t.sp3).py(t.sp1).child(
            h_flex()
                .id("db-activity-open")
                .items_center()
                .gap(t.sp1)
                .px(t.sp2)
                .py(px(3.0))
                .rounded(t.radius_sm)
                .bg(t.panel_2)
                .text_size(t.fs_ui)
                .text_color(t.ink_2)
                .cursor_pointer()
                .hover(|s| s.bg(t.elev))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _w, cx| this.load_activity(cx)),
                )
                .child(ui::icon("activity", px(12.0), t.muted))
                .child(i18n::t("db.activity")),
        )
    }

    /// One activity-view filter chip.
    fn act_chip(
        &self,
        cx: &mut Context<Self>,
        key: &'static str,
        f: ActFilter,
        id: &'static str,
    ) -> impl IntoElement {
        let t = &self.theme;
        let active = self.activity_filter == f;
        div()
            .id(id)
            .px(t.sp2)
            .py(px(2.0))
            .rounded(t.radius_sm)
            .text_size(t.fs_ui)
            .cursor_pointer()
            .when(active, |d| d.bg(t.accent_dim).text_color(t.accent))
            .when(!active, |d| d.text_color(t.ink_2).hover(|s| s.bg(t.elev)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.activity_filter = f;
                    cx.notify();
                }),
            )
            .child(i18n::t(key))
    }

    /// The server activity (processlist) view: filter chips, refresh / close,
    /// and one card per session with cancel / terminate controls.
    fn activity_view(&self, cx: &mut Context<Self>, rows: &[ProcRow]) -> AnyElement {
        let t = &self.theme;
        let filter = self.activity_filter;
        let visible: Vec<&ProcRow> = rows
            .iter()
            .filter(|r| match filter {
                ActFilter::All => true,
                ActFilter::Active => {
                    let c = r.command.to_ascii_lowercase();
                    c != "sleep" && c != "idle" && !c.starts_with("binlog dump")
                }
                ActFilter::Long => r.time_s >= 5,
            })
            .collect();

        let header = h_flex()
            .items_center()
            .gap(t.sp1)
            .px(t.sp3)
            .py(t.sp1)
            .child(ui::icon("activity", px(13.0), t.accent))
            .child(
                div()
                    .text_size(t.fs_ui)
                    .text_color(t.ink)
                    .child(i18n::t("db.activity_title")),
            )
            .child(
                div()
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(i18n::tf("db.act_count", &[&visible.len().to_string()])),
            )
            .child(div().flex_1())
            .child(self.act_chip(cx, "db.act_all", ActFilter::All, "act-f-all"))
            .child(self.act_chip(cx, "db.act_active", ActFilter::Active, "act-f-active"))
            .child(self.act_chip(cx, "db.act_long", ActFilter::Long, "act-f-long"))
            .child(
                div()
                    .id("act-refresh")
                    .px(t.sp2)
                    .py(px(2.0))
                    .rounded(t.radius_sm)
                    .text_size(t.fs_ui)
                    .text_color(t.ink_2)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.elev))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| this.load_activity(cx)),
                    )
                    .child(i18n::t("common.refresh")),
            )
            .child(
                div()
                    .id("act-close")
                    .px(t.sp2)
                    .py(px(2.0))
                    .rounded(t.radius_sm)
                    .text_size(t.fs_ui)
                    .text_color(t.ink_2)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.elev))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                            this.activity = None;
                            this.kill_confirm = None;
                            cx.notify();
                        }),
                    )
                    .child(i18n::t("common.close")),
            );

        let mut col = v_flex().child(header);
        if visible.is_empty() {
            col = col.child(hint(t, i18n::t("db.act_none")));
        }
        for r in &visible {
            col = col.child(self.activity_card(cx, r));
        }
        col.into_any_element()
    }

    /// One session card in the activity view.
    fn activity_card(&self, cx: &mut Context<Self>, r: &ProcRow) -> impl IntoElement {
        let t = &self.theme;
        let dur = r.time_s;
        let dur_color = if dur >= 60 {
            t.neg
        } else if dur >= 5 {
            t.warn
        } else {
            t.muted
        };
        let armed = self.kill_confirm.as_deref() == Some(r.id.as_str());
        let id_cancel = r.id.clone();
        let id_term = r.id.clone();
        let id_arm = r.id.clone();

        let mut head = h_flex()
            .items_center()
            .gap(t.sp2)
            .child(
                div()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.accent)
                    .child(format!("#{}", r.id)),
            )
            .child(
                div()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink)
                    .child(if r.command.is_empty() {
                        "—".to_string()
                    } else {
                        r.command.clone()
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(format!("{}@{}", r.user, r.host)),
            );
        if !r.db.is_empty() {
            head = head.child(div().text_size(t.fs_sm).text_color(t.dim).child(r.db.clone()));
        }
        head = head.child(
            div()
                .font_family(t.mono.clone())
                .text_size(t.fs_sm)
                .text_color(dur_color)
                .child(format!("{dur}s")),
        );
        if !r.state.is_empty() {
            head = head.child(div().text_size(t.fs_sm).text_color(t.dim).child(r.state.clone()));
        }
        // Cancel (KILL QUERY / pg_cancel_backend) — direct, matching the web.
        head = head
            .child(
                div()
                    .id(SharedString::from(format!("act-cancel-{}", r.id)))
                    .px(t.sp2)
                    .py(px(2.0))
                    .rounded(t.radius_sm)
                    .bg(t.panel)
                    .text_size(t.fs_ui)
                    .text_color(t.ink_2)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.elev))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.kill_proc(id_cancel.clone(), false, cx)
                        }),
                    )
                    .child(i18n::t("db.kill_cancel")),
            )
            // Terminate (KILL / pg_terminate_backend) — two-step confirm.
            .child(if armed {
                div()
                    .id(SharedString::from(format!("act-term-{}", r.id)))
                    .px(t.sp2)
                    .py(px(2.0))
                    .rounded(t.radius_sm)
                    .bg(t.neg)
                    .text_size(t.fs_ui)
                    .text_color(t.accent_ink)
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.kill_proc(id_term.clone(), true, cx)
                        }),
                    )
                    .child(i18n::t("db.kill_confirm_btn"))
            } else {
                div()
                    .id(SharedString::from(format!("act-term-{}", r.id)))
                    .px(t.sp2)
                    .py(px(2.0))
                    .rounded(t.radius_sm)
                    .text_size(t.fs_ui)
                    .text_color(t.neg)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.elev))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.kill_confirm = Some(id_arm.clone());
                            cx.notify();
                        }),
                    )
                    .child(i18n::t("db.kill_terminate"))
            });

        let mut card = v_flex()
            .gap(px(2.0))
            .mx(t.sp3)
            .my(px(2.0))
            .p(t.sp2)
            .rounded(t.radius_sm)
            .bg(t.panel_2)
            .border_1()
            .border_color(t.line_2)
            .child(head);
        if !r.info.is_empty() {
            card = card.child(
                div()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink_2)
                    .overflow_hidden()
                    .child(r.info.clone()),
            );
        }
        card
    }

    /// Clickable foreign-key drill-down rows for the expanded result row,
    /// matching `self.fks` against the result columns by name.
    fn fk_edges(&self, cx: &mut Context<Self>, columns: &[String], row: &[String]) -> Vec<AnyElement> {
        let t = &self.theme;
        let engine = self.engine;
        let mut out: Vec<AnyElement> = Vec::new();
        for fk in &self.fks {
            if fk.columns.is_empty() || fk.columns.len() != fk.ref_columns.len() {
                continue;
            }
            let mut vals: Vec<String> = Vec::new();
            let mut ok = true;
            for c in &fk.columns {
                match columns.iter().position(|x| x == c) {
                    Some(i) => vals.push(row.get(i).cloned().unwrap_or_default()),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok || vals.iter().all(|v| v.is_empty()) {
                continue;
            }
            let table_ref = match (&fk.ref_schema, engine) {
                (Some(s), Engine::Postgres) => {
                    format!("{}.{}", engine.ident(s), engine.ident(&fk.ref_table))
                }
                _ => engine.ident(&fk.ref_table),
            };
            let where_clause = fk
                .ref_columns
                .iter()
                .zip(vals.iter())
                .map(|(c, v)| {
                    let lit = if v.is_empty() {
                        "NULL".to_string()
                    } else {
                        sql_lit(v)
                    };
                    format!("{} = {}", engine.ident(c), lit)
                })
                .collect::<Vec<_>>()
                .join(" AND ");
            let sql = format!("SELECT * FROM {table_ref} WHERE {where_clause} LIMIT 50");
            let summary = fk
                .columns
                .iter()
                .zip(vals.iter())
                .map(|(c, v)| format!("{}={}", c, if v.is_empty() { "NULL" } else { v }))
                .collect::<Vec<_>>()
                .join(", ");
            let label = i18n::tf("db.fk_label", &[&fk.ref_table, &summary]);
            out.push(
                h_flex()
                    .id(SharedString::from(format!("fk-{}", out.len())))
                    .items_center()
                    .gap(t.sp2)
                    .px(t.sp2)
                    .py(px(2.0))
                    .rounded(t.radius_sm)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.navigate_fk(sql.clone(), cx)
                        }),
                    )
                    .child(ui::icon("external-link", px(12.0), t.accent))
                    .child(
                        div()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.accent)
                            .child(label),
                    )
                    .into_any_element(),
            );
        }
        out
    }

    fn result_table(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let Some(r) = &self.result else {
            return div().into_any_element();
        };
        if r.columns.is_empty() {
            // A write (or a zero-row read) returns no grid; there's no portable
            // affected-row count, so report OK + elapsed rather than a count.
            return hint(
                t,
                if r.error.is_some() {
                    i18n::t("db.query_failed").to_string()
                } else {
                    i18n::tf("db.ok_ms", &[&r.elapsed_ms.to_string()])
                },
            )
            .into_any_element();
        }
        let cw = px(150.0);
        let cell = |text: String, header: bool| {
            div()
                .w(cw)
                .flex_none()
                .px(t.sp2)
                .py(px(3.0))
                .overflow_hidden()
                .font_family(t.mono.clone())
                .text_size(t.fs_sm)
                .text_color(if header { t.ink } else { t.ink_2 })
                .when(header, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(text)
        };
        let mut table = v_flex().child(
            h_flex()
                .border_b_1()
                .border_color(t.line)
                .children(r.columns.iter().map(|c| cell(c.clone(), true))),
        );
        for (i, row) in r.rows.iter().take(MAX_RENDER_ROWS).enumerate() {
            let selected = self.expanded_row == Some(i);
            table = table.child(
                h_flex()
                    .id(SharedString::from(format!("res-row-{i}")))
                    .border_b_1()
                    .border_color(t.line)
                    .cursor_pointer()
                    .when(selected, |d| d.bg(t.accent_dim))
                    .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.expanded_row = if this.expanded_row == Some(i) {
                                None
                            } else {
                                Some(i)
                            };
                            cx.notify();
                        }),
                    )
                    .children(row.iter().map(|v| cell(v.clone(), false))),
            );
        }
        let total = r.rows.len();
        let shown = total.min(MAX_RENDER_ROWS);
        // Honest note about what's painted vs. what the query produced.
        let note: Option<String> = if r.truncated {
            Some(i18n::tf(
                "db.rows_capped",
                &[&shown.to_string(), &MAX_GRID_ROWS.to_string()],
            ))
        } else if total > shown {
            Some(i18n::tf(
                "db.rows_shown",
                &[&shown.to_string(), &total.to_string()],
            ))
        } else {
            None
        };
        // The expanded row's key/value detail, rendered full-width below the
        // horizontally-scrolling grid so long values stay readable.
        let detail: Option<AnyElement> = self
            .expanded_row
            .and_then(|i| r.rows.get(i))
            .map(|row| {
                let edges = self.fk_edges(cx, &r.columns, row);
                let base = row_detail(t, &r.columns, row);
                if edges.is_empty() {
                    base.into_any_element()
                } else {
                    let mut fkbox = v_flex()
                        .mx(t.sp3)
                        .mb(t.sp2)
                        .child(ui::section_label(t, i18n::t("db.foreign_keys")));
                    for e in edges {
                        fkbox = fkbox.child(e);
                    }
                    v_flex().child(base).child(fkbox).into_any_element()
                }
            });
        v_flex()
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp1)
                    .pr(t.sp3)
                    .child(ui::section_label(
                        t,
                        i18n::tf(
                            "db.result_count",
                            &[
                                &total.to_string(),
                                if r.truncated { "+" } else { "" },
                                &r.elapsed_ms.to_string(),
                            ],
                        ),
                    ))
                    .child(div().flex_1())
                    .child(self.copy_btn(cx, "db.copy_tsv", false))
                    .child(self.copy_btn(cx, "db.copy_csv", true)),
            )
            .child(
                div()
                    .id("sql-result")
                    .overflow_x_scroll()
                    .px(t.sp3)
                    .child(table),
            )
            .children(detail)
            .children(note.map(|s| hint(t, s).into_any_element()))
            .into_any_element()
    }
}

impl Render for DbPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = self.theme.clone();
        let meta = if self.busy {
            "…".to_string()
        } else if self.engine == Engine::Sqlite {
            self.open_db
                .as_deref()
                .map(file_name)
                .unwrap_or_else(|| Engine::Sqlite.label().to_string())
        } else if let Some(db) = self.selected_db.and_then(|d| self.databases.get(d)) {
            format!("{} · {}", self.engine.label(), db)
        } else {
            self.engine.label().to_string()
        };

        let mut root = v_flex()
            .size_full()
            .child(ui::panel_header(&t, "database", i18n::t("db.title"), meta))
            .child(self.toolbar(cx));

        if let Some(err) = self.error.clone() {
            root = root.child(
                div()
                    .w_full()
                    .px(t.sp3)
                    .py(t.sp2)
                    .border_b_1()
                    .border_color(t.line)
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.neg)
                    .child(err),
            );
        }

        let body = if self.engine.remote() {
            self.remote_body(cx)
        } else {
            self.sqlite_body(cx)
        };

        root.child(
            div()
                .id("db-scroll")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(body),
        )
    }
}

/// A single muted hint line for empty sections.
fn hint(t: &Theme, text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .px(t.sp3)
        .py(t.sp2)
        .text_size(t.fs_sm)
        .text_color(t.dim)
        .child(text.into())
}

/// One column row: mono name, type, and an optional key badge.
fn column_row(t: &Theme, c: &Col) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap(t.sp2)
        .h(px(24.0))
        .px(t.sp3)
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .font_family(t.mono.clone())
                .text_size(t.fs_sm)
                .text_color(t.ink_2)
                .child(c.name.clone()),
        )
        .child(div().text_size(t.fs_sm).text_color(t.muted).child(c.ty.clone()))
        .when(!c.key.is_empty(), |d| {
            d.child(
                div()
                    .px(px(5.0))
                    .py(px(1.0))
                    .rounded(t.radius_sm)
                    .bg(t.accent_subtle)
                    .font_family(t.mono.clone())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(t.fs_sm)
                    .text_color(t.accent)
                    .child(c.key.clone()),
            )
        })
}

/// One result row expanded as a vertical key/value list (column: value).
fn row_detail(t: &Theme, columns: &[String], row: &[String]) -> impl IntoElement {
    let mut col = v_flex()
        .gap(px(2.0))
        .mx(t.sp3)
        .my(t.sp2)
        .p(t.sp2)
        .rounded(t.radius_sm)
        .bg(t.panel_2)
        .border_1()
        .border_color(t.line_2);
    for (i, name) in columns.iter().enumerate() {
        let value = row.get(i).cloned().unwrap_or_default();
        col = col.child(
            h_flex()
                .gap(t.sp2)
                .items_start()
                .child(
                    div()
                        .w(px(120.0))
                        .flex_none()
                        .font_family(t.mono.clone())
                        .text_size(t.fs_sm)
                        .text_color(t.muted)
                        .child(name.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .font_family(t.mono.clone())
                        .text_size(t.fs_sm)
                        .text_color(t.ink)
                        .child(value),
                ),
        );
    }
    col
}

/// The trailing path component (file name) of `path`.
fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// True when `sql` is a single statement: no `;` except an optional trailing
/// one. The whole string is handed to `sqlite3` / `mysql -e` / `psql -c`, which
/// run every `;`-separated statement, so both the read guard and the write path
/// require this to stop a smuggled second statement (`… ; DROP TABLE x`) from
/// riding along. A literal `;` inside a string fails too — conservative on
/// purpose, since safety is the default.
fn is_single_statement(sql: &str) -> bool {
    let trimmed = sql.trim();
    let body = trimmed.strip_suffix(';').unwrap_or(trimmed);
    !body.contains(';')
}

/// True for read-only statements honoured by the panel's read-only default.
/// Also the classifier the write path uses: anything this rejects is treated as
/// a write and gated behind the unlock toggle + "WRITE" confirmation.
fn is_readonly_sql(sql: &str) -> bool {
    if !is_single_statement(sql) {
        return false;
    }
    let trimmed = sql.trim();
    let body = trimmed.strip_suffix(';').unwrap_or(trimmed);
    let head = body
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(
        head.as_str(),
        "SELECT" | "WITH" | "PRAGMA" | "EXPLAIN" | "SHOW" | "DESCRIBE" | "DESC"
    )
}

/// Strip a leading `EXPLAIN` / `EXPLAIN ANALYZE` / `EXPLAIN (...)` the user may
/// have typed, so the panel's own EXPLAIN wrapper never double-wraps.
fn strip_explain_prefix(sql: &str) -> &str {
    let s = sql.trim();
    if s.get(..7).map(|p| p.eq_ignore_ascii_case("explain")) != Some(true) {
        return s;
    }
    let after = &s[7..];
    let rest = after.trim_start();
    // Must be the keyword: followed by whitespace or an options group.
    if !after.starts_with(char::is_whitespace) && !rest.starts_with('(') {
        return s;
    }
    // Drop an optional ANALYZE …
    if rest.get(..7).map(|p| p.eq_ignore_ascii_case("analyze")) == Some(true) {
        let a = &rest[7..];
        if a.starts_with(char::is_whitespace) {
            return a.trim_start();
        }
    }
    // … or an optional (…) options group.
    if let Some(stripped) = rest.strip_prefix('(') {
        if let Some(close) = stripped.find(')') {
            return stripped[close + 1..].trim_start();
        }
    }
    rest
}

/// Render `EXPLAIN QUERY PLAN` output (columns id, parent, notused, detail) as
/// an indented tree by walking each node's parent chain to the root.
fn sqlite_plan_tree(columns: &[String], rows: &[Vec<String>]) -> String {
    let detail_idx = columns.len().saturating_sub(1);
    let mut out = String::new();
    for row in rows {
        let detail = row.get(detail_idx).map(String::as_str).unwrap_or("");
        let depth = if columns.len() >= 4 {
            let mut d = 0usize;
            let mut parent = row.get(1).and_then(|p| p.parse::<i64>().ok()).unwrap_or(0);
            let mut guard = 0;
            while parent != 0 && guard < 32 {
                d += 1;
                let next = rows
                    .iter()
                    .find(|r| r.first().and_then(|i| i.parse::<i64>().ok()) == Some(parent));
                match next.and_then(|r| r.get(1)).and_then(|p| p.parse::<i64>().ok()) {
                    Some(p) => parent = p,
                    None => break,
                }
                guard += 1;
            }
            d
        } else {
            0
        };
        out.push_str(&"  ".repeat(depth));
        out.push_str(detail);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Postgres' activity query — column order pinned to match `parse_proc_rows`
/// (pid, user, host, db, state, secs, wait, query). Whitespace in the query
/// text is collapsed so each backend stays on one output line.
const PG_ACTIVITY_SQL: &str = "SELECT pid, \
coalesce(usename,''), \
coalesce(host(client_addr),''), \
coalesce(datname,''), \
coalesce(state,''), \
coalesce(extract(epoch from (now()-query_start))::int, 0), \
coalesce(wait_event,''), \
regexp_replace(coalesce(query,''), '\\s+', ' ', 'g') \
FROM pg_stat_activity WHERE pid <> pg_backend_pid() ORDER BY 6 DESC NULLS LAST";

/// Parse `SHOW FULL PROCESSLIST` / `pg_stat_activity` rows (no header) into
/// positional [`ProcRow`]s. Both engines emit the same column order.
fn parse_proc_rows(engine: Engine, out: &str) -> Vec<ProcRow> {
    let sep = engine.sep();
    out.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let p: Vec<&str> = l.split(sep).collect();
            let g = |i: usize| p.get(i).copied().unwrap_or("").trim().to_string();
            // MySQL prints SQL NULL as the literal "NULL"; treat it as blank.
            let gz = |i: usize| {
                let v = g(i);
                if v == "NULL" {
                    String::new()
                } else {
                    v
                }
            };
            ProcRow {
                id: g(0),
                user: gz(1),
                host: gz(2),
                db: gz(3),
                command: g(4),
                time_s: g(5).parse().unwrap_or(0),
                state: gz(6),
                info: gz(7),
            }
        })
        .collect()
}

/// The foreign-key metadata query for `table` (MySQL / Postgres). The first
/// column is the constraint name so [`parse_fks`] can group composite keys.
fn fk_sql(engine: Engine, table: &str) -> String {
    match engine {
        Engine::Mysql => format!(
            "SELECT CONSTRAINT_NAME, COLUMN_NAME, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
             FROM information_schema.KEY_COLUMN_USAGE \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = {} \
             AND REFERENCED_TABLE_NAME IS NOT NULL \
             ORDER BY CONSTRAINT_NAME, ORDINAL_POSITION",
            sql_lit(table)
        ),
        Engine::Postgres => format!(
            "SELECT tc.constraint_name, kcu.column_name, ccu.table_name, ccu.column_name, \
             ccu.table_schema \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
             JOIN information_schema.constraint_column_usage ccu \
               ON ccu.constraint_name = tc.constraint_name AND ccu.table_schema = tc.table_schema \
             WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = 'public' \
               AND tc.table_name = {} \
             ORDER BY tc.constraint_name",
            sql_lit(table)
        ),
        _ => String::new(),
    }
}

/// Group FK metadata rows ([constraint, column, ref_table, ref_col, ref_schema?])
/// by constraint name so composite keys become a single multi-column [`Fk`].
fn parse_fks(engine: Engine, out: &str) -> Vec<Fk> {
    let sep = engine.sep();
    let mut fks: Vec<Fk> = Vec::new();
    let mut cur: Option<String> = None;
    for l in out.lines().filter(|l| !l.is_empty()) {
        let p: Vec<&str> = l.split(sep).collect();
        let name = p.first().copied().unwrap_or("").to_string();
        let col = p.get(1).copied().unwrap_or("").to_string();
        let ref_table = p.get(2).copied().unwrap_or("").to_string();
        let ref_col = p.get(3).copied().unwrap_or("").to_string();
        if col.is_empty() || ref_table.is_empty() {
            continue;
        }
        let ref_schema = if matches!(engine, Engine::Postgres) {
            p.get(4).map(|s| s.to_string()).filter(|s| !s.is_empty())
        } else {
            None
        };
        if cur.as_deref() == Some(name.as_str()) {
            if let Some(last) = fks.last_mut() {
                last.columns.push(col);
                last.ref_columns.push(ref_col);
                continue;
            }
        }
        cur = Some(name);
        fks.push(Fk {
            columns: vec![col],
            ref_schema,
            ref_table,
            ref_columns: vec![ref_col],
        });
    }
    fks
}

/// Parse `PRAGMA foreign_key_list(table)` (columns id, seq, table, from, to, …)
/// into grouped [`Fk`]s, one per `id`.
fn parse_sqlite_fks(r: &SqliteQueryResult) -> Vec<Fk> {
    let find = |name: &str| r.columns.iter().position(|c| c.eq_ignore_ascii_case(name));
    let (Some(ci), Some(ctab), Some(cfrom), Some(cto)) =
        (find("id"), find("table"), find("from"), find("to"))
    else {
        return Vec::new();
    };
    let mut fks: Vec<Fk> = Vec::new();
    let mut cur: Option<String> = None;
    for row in &r.rows {
        let id = row.get(ci).cloned().unwrap_or_default();
        let table = row.get(ctab).cloned().unwrap_or_default();
        let from = row.get(cfrom).cloned().unwrap_or_default();
        let to = row.get(cto).cloned().unwrap_or_default();
        if from.is_empty() || table.is_empty() {
            continue;
        }
        if cur.as_deref() == Some(id.as_str()) {
            if let Some(last) = fks.last_mut() {
                last.columns.push(from);
                last.ref_columns.push(to);
                continue;
            }
        }
        cur = Some(id);
        fks.push(Fk {
            columns: vec![from],
            ref_schema: None,
            ref_table: table,
            ref_columns: vec![to],
        });
    }
    fks
}

/// The result grid as TSV — tab-separated columns, newline-separated rows. Any
/// tab / newline / CR inside a cell is flattened to a space so the data can't
/// break the row / column structure.
fn grid_to_tsv(grid: &Grid) -> String {
    fn clean(s: &str) -> String {
        s.chars()
            .map(|c| if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c })
            .collect()
    }
    let mut out = grid
        .columns
        .iter()
        .map(|c| clean(c))
        .collect::<Vec<_>>()
        .join("\t");
    for row in &grid.rows {
        out.push('\n');
        out.push_str(&row.iter().map(|c| clean(c)).collect::<Vec<_>>().join("\t"));
    }
    out
}

/// The result grid as RFC-4180 CSV: a field is quoted only when it contains a
/// comma, double-quote, CR, or LF; embedded quotes are doubled; rows end CRLF.
fn grid_to_csv(grid: &Grid) -> String {
    fn field(s: &str) -> String {
        if s.contains(|c: char| matches!(c, ',' | '"' | '\r' | '\n')) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    }
    let mut out = grid
        .columns
        .iter()
        .map(|c| field(c))
        .collect::<Vec<_>>()
        .join(",");
    for row in &grid.rows {
        out.push_str("\r\n");
        out.push_str(&row.iter().map(|c| field(c)).collect::<Vec<_>>().join(","));
    }
    out
}

/// Where the query-history file lives, mirroring the favorites store in
/// `data.rs`. `None` when no config dir is resolvable.
fn history_path() -> Option<std::path::PathBuf> {
    pier_core::paths::config_dir().map(|d| d.join("pier-x-gpui-sql-history.conf"))
}

/// Load saved query history (newest first). One tab-separated record per line:
/// `rows<TAB>elapsed_ms<TAB>sql`. `write` is re-derived from the SQL so the file
/// stays a plain line list; malformed lines are skipped. SQL never contains a
/// newline (the console accumulates single-line input), so one line per entry
/// round-trips safely.
fn load_history() -> Vec<HistEntry> {
    let Some(p) = history_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        if out.len() >= MAX_HISTORY {
            break;
        }
        let mut parts = line.splitn(3, '\t');
        let rows = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
        let elapsed = parts.next().and_then(|s| s.trim().parse::<u64>().ok());
        let sql = parts.next();
        if let (Some(rows), Some(elapsed_ms), Some(sql)) = (rows, elapsed, sql) {
            if sql.is_empty() {
                continue;
            }
            out.push(HistEntry {
                sql: sql.to_string(),
                rows,
                elapsed_ms,
                write: !is_readonly_sql(sql),
            });
        }
    }
    out
}

/// Persist the query history (best-effort), newest first, capped at
/// [`MAX_HISTORY`]. Mirrors `data.rs`'s favorites writer.
fn save_history(history: &[HistEntry]) {
    let Some(p) = history_path() else {
        return;
    };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = history
        .iter()
        .take(MAX_HISTORY)
        .map(|e| format!("{}\t{}\t{}", e.rows, e.elapsed_ms, e.sql))
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(&p, body);
}

/// Wrap `s` in shell single quotes, escaping embedded single quotes so the
/// remote shell receives it verbatim.
fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A single-quoted SQL string literal (embedded quotes doubled).
fn sql_lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The numeric index in a Redis `dbN` keyspace name, defaulting to `0`.
fn redis_db_index(db: &str) -> String {
    db.strip_prefix("db")
        .filter(|n| n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty())
        .unwrap_or("0")
        .to_string()
}

/// The read command for a Redis key, chosen by its type. Empty for types with
/// no simple preview.
fn redis_value_command(n: &str, ty: &str, key: &str) -> String {
    let k = shq(key);
    match ty {
        "string" => format!("redis-cli -n {n} GET {k} 2>&1"),
        "list" => format!("redis-cli -n {n} LRANGE {k} 0 50 2>&1"),
        "set" => format!("redis-cli -n {n} SMEMBERS {k} 2>&1"),
        "zset" => format!("redis-cli -n {n} ZRANGE {k} 0 50 WITHSCORES 2>&1"),
        "hash" => format!("redis-cli -n {n} HGETALL {k} 2>&1"),
        _ => String::new(),
    }
}

/// Short uppercase badge for a Redis type word.
fn redis_type_badge(ty: &str) -> &'static str {
    match ty {
        "string" => "STR",
        "list" => "LIST",
        "set" => "SET",
        "zset" => "ZSET",
        "hash" => "HASH",
        "stream" => "STRM",
        _ => "—",
    }
}

/// A human TTL: `-1` → no expiry, `-2` → missing, otherwise `<n>s`.
fn human_ttl(raw: &str) -> String {
    match raw.trim() {
        "-1" => i18n::t("db.ttl_no_expiry").to_string(),
        "-2" => i18n::t("db.ttl_missing").to_string(),
        other => match other.parse::<i64>() {
            Ok(n) => format!("{n}s"),
            Err(_) => other.to_string(),
        },
    }
}

/// Use `fallback` when the command produced no usable error text.
fn err_text(out: String, fallback: &str) -> String {
    let trimmed = out.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Parse separator-delimited batch output into a [`Grid`]: first non-empty line
/// is the header, the rest are rows.
fn parse_grid(out: &str, sep: char, elapsed_ms: u64) -> Grid {
    let mut it = out.lines().filter(|l| !l.is_empty());
    let Some(header) = it.next() else {
        return Grid {
            columns: Vec::new(),
            rows: Vec::new(),
            elapsed_ms,
            error: None,
            truncated: false,
        };
    };
    let columns = header.split(sep).map(|s| s.to_string()).collect();
    // Cap rows while parsing so an unbounded `SELECT *` can't pull a whole
    // table into memory; `truncated` records that more rows were dropped.
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    for line in it {
        if rows.len() >= MAX_GRID_ROWS {
            truncated = true;
            break;
        }
        rows.push(line.split(sep).map(|s| s.to_string()).collect());
    }
    Grid {
        columns,
        rows,
        elapsed_ms,
        error: None,
        truncated,
    }
}

/// Scan the working dir (one level) for SQLite database files, sorted.
fn discover_db_files() -> Vec<String> {
    let dir = data::current_dir();
    let mut out = Vec::new();
    if let Ok(read) = std::fs::read_dir(&dir) {
        for e in read.flatten() {
            let path = e.path();
            if !path.is_file() {
                continue;
            }
            let is_db = path
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| matches!(x.to_ascii_lowercase().as_str(), "db" | "sqlite" | "sqlite3"))
                .unwrap_or(false);
            if is_db {
                out.push(path.to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}
