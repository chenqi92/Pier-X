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
//     the background executor, and run a single READ-ONLY listing command
//     (`SHOW DATABASES` / `SELECT datname …` / `INFO keyspace`) over it. No
//     writes or DDL are ever issued — the panel honours the read-only default.
//
// Connection / query failures surface as a single `t.neg` line under the header.

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Context, FocusHandle, FontWeight, KeyDownEvent, MouseButton,
    MouseDownEvent, SharedString, Window,
};
use gpui_component::{h_flex, v_flex};

use pier_core::services::sqlite::{SqliteClient, SqliteQueryResult};
use pier_core::ssh::SshConfig;

use crate::data;
use crate::theme::Theme;
use crate::ui;

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
    /// stderr is dropped so a missing client surfaces as an empty result.
    fn list_command(self) -> &'static str {
        match self {
            Engine::Mysql => "mysql -N -B -e 'SHOW DATABASES' 2>/dev/null",
            Engine::Postgres => {
                "psql -At -c 'SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname' 2>/dev/null"
            }
            Engine::Redis => "redis-cli INFO keyspace 2>/dev/null",
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
}

/// One column of the selected table, normalised across engines.
struct Col {
    name: String,
    ty: String,
    /// Short key marker (`PK` / `NN`), empty when neither applies.
    key: String,
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
    databases: Vec<String>,

    // SQLite query console.
    query: String,
    query_focus: FocusHandle,
    result: Option<SqliteQueryResult>,

    // Shared.
    busy: bool,
    error: Option<String>,
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
            databases: Vec::new(),
            query: String::new(),
            query_focus: cx.focus_handle(),
            result: None,
            busy: false,
            error: None,
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

    /// Run the SQLite query box against the open file (read-only intent).
    fn run_query(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.open_db.clone() else {
            return;
        };
        let sql = self.query.trim().to_string();
        if sql.is_empty() {
            return;
        }
        // Honour the read-only default: only allow read statements.
        let head = sql
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if !matches!(head.as_str(), "SELECT" | "PRAGMA" | "EXPLAIN" | "WITH") {
            self.error = Some("Read-only: only SELECT / WITH / PRAGMA / EXPLAIN".to_string());
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    match SqliteClient::open(&path) {
                        Ok(c) => Ok(c.execute(&sql)),
                        Err(e) => Err(e.to_string()),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match res {
                    Ok(r) => {
                        if let Some(err) = &r.error {
                            this.error = Some(err.clone());
                        }
                        this.result = Some(r);
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ── Actions (all blocking work happens on the background executor) ──

    /// Re-scan the working dir for SQLite files and reload saved connections.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.db_files = discover_db_files();
        self.conns = data::connections_raw();
        cx.notify();
    }

    /// Open a SQLite file and list its tables.
    fn open_sqlite(&mut self, path: String, cx: &mut Context<Self>) {
        self.open_db = Some(path.clone());
        self.tables.clear();
        self.selected_table = None;
        self.columns.clear();
        self.error = None;
        self.busy = true;
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
        self.error = None;
        self.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    match SqliteClient::open(&path) {
                        Ok(c) => c.table_columns(&table).map_err(|e| e.to_string()),
                        Err(e) => Err(e.to_string()),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match res {
                    Ok(cols) => {
                        this.columns = cols
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
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Open an SSH session to the saved connection at `idx` and run the current
    /// engine's read-only database-listing command over it.
    fn connect_remote(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(cfg) = self.conns.get(idx).cloned() else {
            return;
        };
        let engine = self.engine;
        self.selected_conn = Some(idx);
        self.databases.clear();
        self.error = None;
        self.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let session = data::connect_blocking(&cfg)?;
                    let (_code, out) = session
                        .exec_command_blocking(engine.list_command())
                        .map_err(|e| e.to_string())?;
                    Ok::<Vec<String>, String>(engine.parse_list(&out))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match res {
                    Ok(dbs) => this.databases = dbs,
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
                    this.engine = e;
                    this.error = None;
                    cx.notify();
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
                    .child("Reload"),
            )
    }

    // ── Bodies ───────────────────────────────────────────────────

    fn sqlite_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let mut col = v_flex().pb(t.sp3);

        col = col.child(ui::section_label(t, format!("DATABASES · {}", self.db_files.len())));
        if self.db_files.is_empty() {
            col = col.child(hint(t, "No .db / .sqlite files in the working dir"));
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
            col = col.child(ui::section_label(t, format!("TABLES · {}", self.tables.len())));
            if self.tables.is_empty() && !self.busy {
                col = col.child(hint(t, "No tables"));
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
            col = col.child(ui::section_label(t, format!("COLUMNS · {}", self.columns.len())));
            for c in &self.columns {
                col = col.child(column_row(t, c));
            }
        }

        if self.open_db.is_some() {
            col = col
                .child(ui::section_label(t, "QUERY"))
                .child(self.query_console(cx))
                .child(self.result_table());
        }

        col.into_any_element()
    }

    fn remote_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let mut col = v_flex().pb(t.sp3);

        col = col.child(ui::section_label(t, format!("CONNECTIONS · {}", self.conns.len())));
        if self.conns.is_empty() {
            col = col.child(hint(t, "No saved connections"));
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

        if !self.databases.is_empty() {
            col = col.child(ui::section_label(t, format!("DATABASES · {}", self.databases.len())));
            for (i, db) in self.databases.iter().enumerate() {
                col = col.child(
                    h_flex()
                        .id(SharedString::from(format!("rdb-{i}")))
                        .items_center()
                        .gap(t.sp2)
                        .h(px(26.0))
                        .px(t.sp3)
                        .child(ui::icon("database", px(14.0), t.muted))
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .font_family(t.mono.clone())
                                .text_size(t.fs_sm)
                                .text_color(t.ink_2)
                                .child(db.clone()),
                        ),
                );
            }
        } else if self.selected_conn.is_some() && !self.busy && self.error.is_none() {
            col = col.child(hint(t, "No databases reported (read-only listing)"));
        }

        col.into_any_element()
    }

    /// SQLite query input + Run button.
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
                    .when(empty, |d| d.text_color(t.dim).child("SELECT … (read-only)"))
                    .when(!empty, |d| d.text_color(t.ink).child(self.query.clone())),
            )
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
                    .child("Run"),
            )
    }

    /// Render the last query result as a scrollable table (capped at 200 rows).
    fn result_table(&self) -> AnyElement {
        let t = &self.theme;
        let Some(r) = &self.result else {
            return div().into_any_element();
        };
        if r.columns.is_empty() {
            return hint(
                t,
                if r.error.is_some() {
                    "Query failed"
                } else {
                    "OK (no rows returned)"
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
        for row in r.rows.iter().take(200) {
            table = table.child(
                h_flex()
                    .border_b_1()
                    .border_color(t.line)
                    .children(row.iter().map(|v| cell(v.clone(), false))),
            );
        }
        let total = r.rows.len();
        v_flex()
            .child(ui::section_label(
                t,
                format!("RESULT · {} rows · {} ms", total, r.elapsed_ms),
            ))
            .child(
                div()
                    .id("sql-result")
                    .overflow_x_scroll()
                    .px(t.sp3)
                    .child(table),
            )
            .when(total > 200, |d| d.child(hint(t, "Showing first 200 rows")))
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
        } else {
            self.engine.label().to_string()
        };

        let mut root = v_flex()
            .size_full()
            .child(ui::panel_header(&t, "database", "DATABASE", meta))
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
fn hint(t: &Theme, text: &'static str) -> impl IntoElement {
    div()
        .px(t.sp3)
        .py(t.sp2)
        .text_size(t.fs_sm)
        .text_color(t.dim)
        .child(text)
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

/// The trailing path component (file name) of `path`.
fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
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
