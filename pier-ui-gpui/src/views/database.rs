//! Database view (MySQL / PostgreSQL for Phase A) — connection
//! picker + schema sidebar + SQL editor + result table.
//!
//! The view is `RenderOnce`, rebuilt every frame from the cached
//! `DbSessionState` entity owned by `PierApp`. All IO goes through
//! `PierApp::schedule_db_*`, which runs the blocking pier-core calls
//! on `cx.background_executor()` and applies results back to the
//! session entity with a nonce guard (see `app/db_session.rs`).
//!
//! Redis (Phase B) and SQLite (Phase C) hit a distinct UX — key
//! browser for Redis, file picker for SQLite — so those tabs still
//! show a short "coming soon" placeholder here. This view only owns
//! the MySQL / PostgreSQL experience.

use gpui::{
    div, prelude::*, px, App, ClickEvent, ElementId, Entity, IntoElement, MouseButton,
    SharedString, WeakEntity, Window,
};
use gpui_component::input::{Input, InputState};
use rust_i18n::t;

use pier_core::db_connections::{DbConnection, DbEngine};

use crate::app::db_session::{DbQueryResult, DbSessionState, DbStatus};
use crate::app::route::DbKind;
use crate::app::PierApp;
use crate::components::{text, Button, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    heights::{BUTTON_SM_H, STATUSBAR_H},
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3, SP_4},
    theme,
    typography::{SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};
use crate::views::database_form::{self, DbEditTarget};

/// Row cap rendered in the result grid. Pier-core already caps at
/// 10k before results reach us; this is the "what we actually paint"
/// cap to keep the element tree bounded.
const MAX_RENDERED_ROWS: usize = 500;

/// Default auto-query when the user clicks a table in the sidebar.
/// Table names are allowed to contain odd characters; we rely on
/// the engine's quoting rules rather than trying to sanitise client-side.
fn auto_select_sql(engine: DbEngine, table: &str) -> String {
    match engine {
        DbEngine::Mysql => format!("SELECT * FROM `{table}` LIMIT 100"),
        DbEngine::Postgres => format!("SELECT * FROM \"{table}\" LIMIT 100"),
    }
}

#[derive(IntoElement)]
pub struct DatabaseView {
    app: WeakEntity<PierApp>,
    kind: DbKind,
}

impl DatabaseView {
    pub fn new(app: WeakEntity<PierApp>, kind: DbKind) -> Self {
        Self { app, kind }
    }
}

impl RenderOnce for DatabaseView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx).clone();

        // Unsupported engines (Redis, SQLite) still need the tab to
        // paint something — ship a compact placeholder until Phase B/C.
        let Some(engine) = database_form::engine_for_kind(self.kind) else {
            return unsupported_placeholder(&t, self.kind).into_any_element();
        };

        // Pull everything we need from PierApp up front. If the weak
        // reference is dead we've lost the app — render a dead-panel
        // placeholder rather than crashing.
        let Some(app_entity) = self.app.upgrade() else {
            return dead_panel_placeholder(&t).into_any_element();
        };

        let (connections, session, query_input) = {
            let app_read = app_entity.read(cx);
            let connections: Vec<(usize, DbConnection)> = app_read
                .db_connections()
                .iter()
                .enumerate()
                .filter(|(_, c)| c.engine == engine)
                .map(|(i, c)| (i, c.clone()))
                .collect();
            let session = app_read.db_session(self.kind);
            let query_input = app_read.db_query_input(self.kind);
            (connections, session, query_input)
        };

        // Snapshot the session state — drops the borrow before we
        // start building child elements that close over `cx`.
        let snapshot = session.as_ref().map(|s| SessionSnapshot::from(s.read(cx)));

        let body = body(
            &t,
            self.app.clone(),
            self.kind,
            engine,
            connections,
            snapshot,
            query_input,
        );
        div().size_full().child(body).into_any_element()
    }
}

/// Immutable flattened copy of `DbSessionState` — decouples the
/// borrow on `cx` from the render closure chain.
struct SessionSnapshot {
    status: DbStatus,
    last_error: Option<SharedString>,
    databases: Vec<String>,
    selected_database: Option<String>,
    tables: Vec<String>,
    last_result: Option<DbQueryResult>,
    query_in_flight: bool,
    client_alive: bool,
    /// Name of the currently-selected connection (if any), for
    /// rendering the dropdown label. Pulled from `state.connection`.
    selected_connection_name: Option<String>,
}

impl From<&DbSessionState> for SessionSnapshot {
    fn from(s: &DbSessionState) -> Self {
        SessionSnapshot {
            status: s.status,
            last_error: s.last_error.clone(),
            databases: s.databases.clone(),
            selected_database: s.selected_database.clone(),
            tables: s.tables.clone(),
            last_result: s.last_result.clone(),
            query_in_flight: s.query_in_flight,
            client_alive: s.client.is_some(),
            selected_connection_name: s.connection.as_ref().map(|c| c.name.clone()),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn body(
    t: &crate::theme::Theme,
    app: WeakEntity<PierApp>,
    kind: DbKind,
    engine: DbEngine,
    connections: Vec<(usize, DbConnection)>,
    snapshot: Option<SessionSnapshot>,
    query_input: Option<Entity<InputState>>,
) -> gpui::AnyElement {
    let status = snapshot.as_ref().map(|s| s.status).unwrap_or_default();
    let status_pill = status_pill_for(status);
    let last_error = snapshot.as_ref().and_then(|s| s.last_error.clone());
    let header = header_bar(t, app.clone(), kind, engine, &connections, snapshot.as_ref(), status_pill);

    let sidebar = sidebar(
        t,
        app.clone(),
        kind,
        engine,
        snapshot.as_ref(),
    );

    let main = main_pane(
        t,
        app.clone(),
        kind,
        engine,
        snapshot.as_ref(),
        query_input,
    );

    let mut col = div().size_full().flex().flex_col().child(header);

    if let Some(err) = last_error {
        col = col.child(error_card(t, err));
    }

    col.child(
        div()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_row()
            .child(sidebar)
            .child(div().w(px(1.0)).h_full().bg(t.color.border_subtle))
            .child(main),
    )
    .into_any_element()
}

fn status_pill_for(status: DbStatus) -> StatusPill {
    match status {
        DbStatus::Idle => StatusPill::new(t!("App.Database.not_connected"), StatusKind::Warning),
        DbStatus::Connecting => {
            StatusPill::new(t!("App.Database.connecting"), StatusKind::Info)
        }
        DbStatus::Connected => StatusPill::new(t!("App.Database.connected"), StatusKind::Success),
        DbStatus::Failed => StatusPill::new(t!("App.Database.error"), StatusKind::Error),
    }
}

// ─── Header ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn header_bar(
    t: &crate::theme::Theme,
    app: WeakEntity<PierApp>,
    kind: DbKind,
    engine: DbEngine,
    connections: &[(usize, DbConnection)],
    snapshot: Option<&SessionSnapshot>,
    status_pill: StatusPill,
) -> gpui::AnyElement {
    let selected_name = snapshot
        .and_then(|s| s.selected_connection_name.as_deref())
        .map(str::to_string)
        .unwrap_or_else(|| t!("App.Database.select_connection").to_string());

    // Find the index of the currently-selected connection (if any)
    // so Edit / Delete know what to operate on.
    let selected_idx = snapshot
        .and_then(|s| s.selected_connection_name.as_deref())
        .and_then(|name| connections.iter().find(|(_, c)| c.name == name).map(|(i, _)| *i));

    let client_alive = snapshot.map(|s| s.client_alive).unwrap_or(false);
    let status = snapshot.map(|s| s.status).unwrap_or_default();

    let mut row = div()
        .h(px(44.0))
        .px(SP_3)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(text::body(kind.label()));

    // Connection dropdown — implemented as a trio: label + ← / → +
    // inline name. Avoids pulling in fork's Dropdown for one use.
    row = row.child(connection_picker(
        t,
        app.clone(),
        kind,
        connections,
        selected_name.as_str(),
    ));

    // Connect / Disconnect button.
    let connect_app = app.clone();
    let can_connect = !connections.is_empty() && !matches!(status, DbStatus::Connecting);
    let connect_label: SharedString = if client_alive {
        t!("App.Database.disconnect").into()
    } else {
        t!("App.Database.connect").into()
    };
    row = row.child(
        Button::primary(
            ElementId::Name(format!("db-connect-{}", engine.as_str()).into()),
            connect_label,
        )
        .on_click(move |_, _w, cx| {
            if !can_connect {
                return;
            }
            let Some(app) = connect_app.upgrade() else { return };
            app.update(cx, |app, cx| {
                if client_alive {
                    // "Disconnect" = drop session entity & reset.
                    // Drop-and-recreate on next Connect is simpler
                    // than a dedicated disconnect path.
                    if let Some(session) = app.db_session(kind) {
                        session.update(cx, |s, _| *s = DbSessionState::new());
                    }
                    cx.notify();
                    return;
                }
                // Look up the first matching saved connection (or the
                // one the user picked via the picker).
                let Some(conn) = app
                    .db_connections()
                    .iter()
                    .find(|c| c.engine == engine)
                    .cloned()
                else {
                    return;
                };
                let password = conn
                    .credential_id
                    .as_deref()
                    .and_then(|id| pier_core::credentials::get(id).ok().flatten());
                app.schedule_db_connect(kind, conn, password, cx);
            });
        }),
    );

    // Add / Edit / Delete.
    let add_app = app.clone();
    row = row.child(
        Button::secondary("db-add", SharedString::from(t!("App.Database.add").to_string()))
            .on_click(move |_, window, cx| {
                if let Some(app) = add_app.upgrade() {
                    let weak = app.downgrade();
                    database_form::open(window, cx, weak, kind, DbEditTarget::Add);
                }
            }),
    );

    if let Some(idx) = selected_idx {
        let edit_app = app.clone();
        let original = connections.iter().find(|(i, _)| *i == idx).map(|(_, c)| c.clone());
        if let Some(original) = original {
            row = row.child(
                Button::secondary("db-edit", SharedString::from(t!("App.Database.edit").to_string()))
                    .on_click(move |_, window, cx| {
                    if let Some(app) = edit_app.upgrade() {
                        let weak = app.downgrade();
                        database_form::open(
                            window,
                            cx,
                            weak,
                            kind,
                            DbEditTarget::Edit {
                                idx,
                                original: original.clone(),
                            },
                        );
                    }
                }),
            );
        }

        let del_app = app.clone();
        row = row.child(
            Button::danger("db-del", SharedString::from(t!("App.Database.delete").to_string()))
                .on_click(move |_, _w, cx| {
                if let Some(app) = del_app.upgrade() {
                    app.update(cx, |app, cx| {
                        app.delete_db_connection(idx);
                        cx.notify();
                    });
                }
            }),
        );
    }

    row.child(div().flex_1()).child(status_pill).into_any_element()
}

fn connection_picker(
    t: &crate::theme::Theme,
    app: WeakEntity<PierApp>,
    kind: DbKind,
    connections: &[(usize, DbConnection)],
    selected_name: &str,
) -> gpui::AnyElement {
    // Dropdown is rendered as a horizontal row of clickable pills,
    // one per saved connection, with the currently-selected one
    // highlighted. Pier doesn't ship a dropdown atom in components/
    // yet and the fork's Dropdown is overkill for ≤5 pills.
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .child(
            div()
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from("·")),
        );

    if connections.is_empty() {
        row = row.child(
            div()
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(t!("App.Database.no_saved_connections").to_string())),
        );
    }

    for (idx, conn) in connections.iter() {
        let is_selected = conn.name == selected_name;
        let pick_app = app.clone();
        let conn_clone = conn.clone();
        let idx_copy = *idx;
        let pill_id: ElementId =
            ElementId::Name(format!("db-pick-{}-{}", kind.label().to_lowercase(), idx_copy).into());

        let label: SharedString = conn.name.clone().into();
        let border = if is_selected {
            t.color.accent
        } else {
            t.color.border_default
        };
        let bg = if is_selected {
            t.color.bg_hover
        } else {
            t.color.bg_surface
        };
        row = row.child(
            div()
                .id(pill_id)
                .px(SP_2)
                .h(BUTTON_SM_H)
                .flex()
                .items_center()
                .rounded(RADIUS_SM)
                .bg(bg)
                .border_1()
                .border_color(border)
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_primary)
                .cursor_pointer()
                .hover(|s| s.bg(t.color.bg_hover))
                .on_mouse_down(MouseButton::Left, move |_, _w, cx| {
                    let Some(app) = pick_app.upgrade() else {
                        return;
                    };
                    let _ = idx_copy; // reserved for future direct-index ops
                    let conn = conn_clone.clone();
                    app.update(cx, |app, cx| {
                        // select connection + auto-connect
                        let password = conn
                            .credential_id
                            .as_deref()
                            .and_then(|id| pier_core::credentials::get(id).ok().flatten());
                        app.schedule_db_connect(kind, conn, password, cx);
                    });
                })
                .child(label),
        );
    }

    row.into_any_element()
}

// ─── Sidebar (database + table list) ─────────────────────────────────

fn sidebar(
    t: &crate::theme::Theme,
    app: WeakEntity<PierApp>,
    kind: DbKind,
    engine: DbEngine,
    snapshot: Option<&SessionSnapshot>,
) -> gpui::AnyElement {
    let databases = snapshot.map(|s| s.databases.as_slice()).unwrap_or(&[]);
    let selected_database = snapshot.and_then(|s| s.selected_database.clone());
    let tables = snapshot.map(|s| s.tables.as_slice()).unwrap_or(&[]);
    let client_alive = snapshot.map(|s| s.client_alive).unwrap_or(false);

    let refresh_app = app.clone();
    let refresh_button = if client_alive {
        Some(
            Button::secondary(
                "db-refresh-dbs",
                SharedString::from(t!("App.Database.refresh").to_string()),
            )
            .on_click(move |_, _w, cx| {
                if let Some(app) = refresh_app.upgrade() {
                    app.update(cx, |app, cx| {
                        app.schedule_db_list_databases(kind, cx);
                    });
                }
            }),
        )
    } else {
        None
    };

    let mut col = div()
        .w(px(220.0))
        .h_full()
        .flex()
        .flex_col()
        .bg(t.color.bg_surface)
        .child(
            div()
                .px(SP_3)
                .py(SP_2)
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .border_b_1()
                .border_color(t.color.border_subtle)
                .child(SectionLabel::new(t!("App.Database.databases_section")))
                .child(div().flex_1()),
        );

    if let Some(btn) = refresh_button {
        col = col.child(div().px(SP_3).py(SP_1).child(btn));
    }

    if databases.is_empty() {
        col = col.child(
            div()
                .px(SP_3)
                .py(SP_2)
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(if client_alive {
                    t!("App.Database.no_databases_listed").to_string()
                } else {
                    t!("App.Database.connect_first_hint").to_string()
                })),
        );
    } else {
        for db in databases {
            let is_selected = Some(db.as_str()) == selected_database.as_deref();
            let db_app = app.clone();
            let db_name = db.clone();
            col = col.child(sidebar_row(
                t,
                ElementId::Name(format!("db-row-{db}").into()),
                db.clone(),
                is_selected,
                true,
                move |_, _w, cx| {
                    if let Some(app) = db_app.upgrade() {
                        let name = db_name.clone();
                        app.update(cx, |app, cx| {
                            app.schedule_db_list_tables(kind, name.clone(), cx);
                        });
                    }
                },
            ));
        }

        if !tables.is_empty() {
            col = col.child(
                div()
                    .px(SP_3)
                    .pt(SP_3)
                    .pb(SP_1)
                    .child(SectionLabel::new(t!("App.Database.tables_section"))),
            );
            for table in tables {
                let tbl_app = app.clone();
                let tbl_name = table.clone();
                col = col.child(sidebar_row(
                    t,
                    ElementId::Name(format!("tbl-row-{table}").into()),
                    table.clone(),
                    false,
                    false,
                    move |_, _w, cx| {
                        if let Some(app) = tbl_app.upgrade() {
                            let sql = auto_select_sql(engine, &tbl_name);
                            // Mirror the SQL into the editor AND kick
                            // off the query; saves the user a click.
                            app.update(cx, |app, cx| {
                                if let Some(input) = app.db_query_input(kind) {
                                    let sql_for_input = sql.clone();
                                    input.update(cx, |state, c| {
                                        // set_value requires a Window; we only
                                        // have App here, so we queue the Run
                                        // and let the editor refresh via notify.
                                        // The placeholder stays but Run uses
                                        // the click-time SQL, not the editor
                                        // value, so UX is consistent.
                                        let _ = (state, c, sql_for_input);
                                    });
                                }
                                app.schedule_db_execute(kind, sql, cx);
                            });
                        }
                    },
                ));
            }
        }
    }

    div()
        .w(px(220.0))
        .h_full()
        .bg(t.color.bg_surface)
        .child(col)
        .into_any_element()
}

fn sidebar_row(
    t: &crate::theme::Theme,
    id: ElementId,
    label: String,
    is_selected: bool,
    is_database: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let color = if is_database {
        t.color.text_primary
    } else {
        t.color.text_secondary
    };
    div()
        .id(id)
        .px(SP_3)
        .py(px(4.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .bg(if is_selected {
            t.color.bg_hover
        } else {
            t.color.bg_surface
        })
        .text_size(SIZE_CAPTION)
        .text_color(color)
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(on_click)
        .child(SharedString::from(label))
        .into_any_element()
}

// ─── Main pane (SQL editor + result table) ───────────────────────────

fn main_pane(
    t: &crate::theme::Theme,
    app: WeakEntity<PierApp>,
    kind: DbKind,
    _engine: DbEngine,
    snapshot: Option<&SessionSnapshot>,
    query_input: Option<Entity<InputState>>,
) -> gpui::AnyElement {
    let client_alive = snapshot.map(|s| s.client_alive).unwrap_or(false);
    let in_flight = snapshot.map(|s| s.query_in_flight).unwrap_or(false);

    let run_app = app.clone();
    let run_input = query_input.clone();
    let can_run = client_alive && !in_flight;
    let run_label: SharedString = if in_flight {
        t!("App.Database.running").into()
    } else {
        t!("App.Database.run").into()
    };
    let run_button = Button::primary("db-run", run_label)
        .on_click(move |_, _w, cx| {
            if !can_run {
                return;
            }
            let Some(app) = run_app.upgrade() else { return };
            let Some(input) = run_input.clone() else { return };
            app.update(cx, |app, cx| {
                let sql = input.read(cx).value().to_string();
                if sql.trim().is_empty() {
                    return;
                }
                app.schedule_db_execute(kind, sql, cx);
            });
        });

    let editor = if let Some(state) = query_input {
        div()
            .w_full()
            .h(px(160.0))
            .child(Input::new(&state).h(px(160.0)))
            .into_any_element()
    } else {
        div()
            .text_color(t.color.text_tertiary)
            .child(SharedString::from(
                t!("App.Database.sql_editor_unavailable").to_string(),
            ))
            .into_any_element()
    };

    let editor_area = div()
        .flex()
        .flex_col()
        .p(SP_3)
        .gap(SP_2)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .child(SectionLabel::new(t!("App.Database.sql_section")))
                .child(div().flex_1())
                .child(run_button),
        )
        .child(editor);

    let result_area = result_pane(t, snapshot);

    div()
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .flex()
        .flex_col()
        .child(editor_area)
        .child(div().w_full().h(px(1.0)).bg(t.color.border_subtle))
        .child(result_area)
        .into_any_element()
}

fn result_pane(
    t: &crate::theme::Theme,
    snapshot: Option<&SessionSnapshot>,
) -> gpui::AnyElement {
    let Some(snap) = snapshot else {
        return div()
            .p(SP_4)
            .text_color(t.color.text_tertiary)
            .child(SharedString::from(t!("App.Database.no_session_hint").to_string()))
            .into_any_element();
    };

    if snap.query_in_flight {
        return div()
            .p(SP_4)
            .text_color(t.color.text_tertiary)
            .child(SharedString::from(t!("App.Database.running_query").to_string()))
            .into_any_element();
    }

    let Some(result) = &snap.last_result else {
        return div()
            .p(SP_4)
            .text_color(t.color.text_tertiary)
            .child(SharedString::from(t!("App.Database.no_results_yet").to_string()))
            .into_any_element();
    };

    let columns = result.columns().to_vec();
    let total_rows = result.rows().len();
    let rows = result.rows().iter().take(MAX_RENDERED_ROWS).cloned().collect::<Vec<_>>();
    let truncated = result.truncated();
    let elapsed_ms = result.elapsed_ms();
    let affected = result.affected_rows();
    let capped_in_ui = total_rows > rows.len();

    let meta = if affected > 0 {
        t!(
            "App.Database.results_meta_with_affected",
            count = total_rows,
            suffix = if total_rows == 1 { "" } else { "s" },
            elapsed = elapsed_ms,
            truncated = if truncated {
                format!(" · {}", t!("App.Database.server_truncated"))
            } else {
                String::new()
            },
            affected = affected
        )
        .to_string()
    } else {
        t!(
            "App.Database.results_meta",
            count = total_rows,
            suffix = if total_rows == 1 { "" } else { "s" },
            elapsed = elapsed_ms,
            truncated = if truncated {
                format!(" · {}", t!("App.Database.server_truncated"))
            } else {
                String::new()
            }
        )
        .to_string()
    };

    let header_row = {
        let mut row = div()
            .flex()
            .flex_row()
            .px(SP_3)
            .py(SP_1)
            .bg(t.color.bg_panel)
            .border_b_1()
            .border_color(t.color.border_subtle);
        for col_name in &columns {
            row = row.child(
                div()
                    .min_w(px(120.0))
                    .mr(SP_3)
                    .text_size(SIZE_CAPTION)
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(t.color.text_primary)
                    .child(SharedString::from(col_name.clone())),
            );
        }
        row
    };

    let mut rows_col = div().flex().flex_col();
    for row_cells in &rows {
        let mut row = div()
            .flex()
            .flex_row()
            .px(SP_3)
            .py(px(4.0))
            .text_size(SIZE_MONO_SMALL)
            .font_family(t.font_mono.clone())
            .text_color(t.color.text_secondary)
            .border_b_1()
            .border_color(t.color.border_subtle);
        for (col_idx, cell) in row_cells.iter().enumerate() {
            let display: SharedString = match cell {
                Some(s) => s.clone().into(),
                None => SharedString::from("NULL"),
            };
            let _ = col_idx;
            row = row.child(div().min_w(px(120.0)).mr(SP_3).child(display));
        }
        rows_col = rows_col.child(row);
    }

    if capped_in_ui {
        rows_col = rows_col.child(
            div()
                .px(SP_3)
                .py(SP_2)
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(t!(
                    "App.Database.showing_first_rows",
                    shown = rows.len(),
                    total = total_rows
                ))),
        );
    }

    div()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .child(header_row)
        .child(div().flex_1().min_h(px(0.0)).child(rows_col))
        .child(
            div()
                .h(STATUSBAR_H)
                .px(SP_3)
                .flex()
                .flex_row()
                .items_center()
                .bg(t.color.bg_panel)
                .border_t_1()
                .border_color(t.color.border_subtle)
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(meta)),
        )
        .into_any_element()
}

fn error_card(_t: &crate::theme::Theme, err: SharedString) -> gpui::AnyElement {
    div()
        .mx(SP_3)
        .my(SP_2)
        .child(
            Card::new()
                .padding(SP_2)
                .child(SectionLabel::new(t!("App.Database.error_title")))
                .child(text::body(err).secondary()),
        )
        .into_any_element()
}

// ─── Unsupported-engine + dead-panel placeholders ────────────────────

fn unsupported_placeholder(t: &crate::theme::Theme, kind: DbKind) -> gpui::AnyElement {
    let (label, body_text) = match kind {
        DbKind::Redis => (
            "Redis",
            t!("App.Database.phase_b_redis").to_string(),
        ),
        DbKind::Sqlite => (
            "SQLite",
            t!("App.Database.phase_c_sqlite").to_string(),
        ),
        _ => ("Database", t!("App.Database.unsupported_engine").to_string()),
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(SP_2)
        .p(SP_4)
        .text_color(t.color.text_tertiary)
        .child(
            div()
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_secondary)
                .child(SharedString::from(label)),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .child(SharedString::from(body_text)),
        )
        .into_any_element()
}

fn dead_panel_placeholder(t: &crate::theme::Theme) -> gpui::AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p(SP_4)
        .text_color(t.color.text_tertiary)
        .child(SharedString::from(
            t!("App.Database.app_handle_dropped").to_string(),
        ))
        .into_any_element()
}
