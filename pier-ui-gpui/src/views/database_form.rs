//! New / edit database connection dialog (P0-1 Phase A, step 4).
//!
//! Mirrors [`crate::views::edit_connection`] but for MySQL /
//! PostgreSQL instead of SSH. Opened as a modal from the database
//! view's "New connection" / "Edit" buttons. On Save, writes the
//! config to `db-connections.json` and the password to the OS
//! keychain under `pier-x.db.{engine}.{name}`.
//!
//! The engine variant is locked by the tab the form was opened from
//! — a MySQL tab only ever edits MySQL connections. Redis / SQLite
//! tabs use different editors in Phase B / C.

// Step 5 wires `open(..)` into the `Add / Edit` buttons on the
// database view. Until then this module compiles but is unreferenced.
#![allow(dead_code)]

use std::rc::Rc;

use gpui::{div, prelude::*, px, App, Entity, IntoElement, SharedString, WeakEntity, Window};
use gpui_component::{
    input::{Input, InputState},
    WindowExt as _,
};
use pier_core::db_connections::{DbConnection, DbConnectionStore, DbEngine};

use crate::app::route::DbKind;
use crate::app::PierApp;
use crate::components::text;
use crate::theme::{
    spacing::{SP_1, SP_2, SP_3},
    theme,
    typography::{SIZE_CAPTION, WEIGHT_MEDIUM},
};

/// What this open() invocation is for — a brand-new entry, or an
/// edit of the existing entry at `idx`.
#[derive(Clone)]
pub enum DbEditTarget {
    /// Append a fresh connection.
    Add,
    /// Replace connection at `idx` with the edited fields. `original`
    /// supplies the pre-fill values.
    Edit {
        /// Index into `PierApp::db_connections`.
        idx: usize,
        /// Original record, used to pre-fill the form.
        original: DbConnection,
    },
}

/// Map a UI tab's `DbKind` to the persisted `DbEngine`. Redis /
/// SQLite tabs return `None` because they don't use this form.
pub fn engine_for_kind(kind: DbKind) -> Option<DbEngine> {
    match kind {
        DbKind::Mysql => Some(DbEngine::Mysql),
        DbKind::Postgres => Some(DbEngine::Postgres),
        DbKind::Redis | DbKind::Sqlite => None,
    }
}

/// Open the database connection editor as a modal dialog. No-op if
/// `kind` is Redis or SQLite (their editors are separate and won't
/// land until Phase B / C).
pub fn open(
    window: &mut Window,
    cx: &mut App,
    app: WeakEntity<PierApp>,
    kind: DbKind,
    target: DbEditTarget,
) {
    let Some(engine) = engine_for_kind(kind) else {
        log::warn!("database_form::open called for unsupported kind {kind:?}");
        return;
    };

    // Inputs allocated once outside the builder closure so they
    // persist across dialog re-renders.
    let name = cx.new(|c| InputState::new(window, c).placeholder("e.g. prod"));
    let host = cx.new(|c| InputState::new(window, c).placeholder("127.0.0.1"));
    let port = cx.new(|c| InputState::new(window, c).placeholder("3306"));
    let user = cx.new(|c| InputState::new(window, c).placeholder("root"));
    let password = cx.new(|c| {
        InputState::new(window, c)
            .masked(true)
            .placeholder("password (stored in OS keychain)")
    });
    let database =
        cx.new(|c| InputState::new(window, c).placeholder("optional — default database to USE"));

    match &target {
        DbEditTarget::Add => {
            port.update(cx, |s, c| {
                s.set_value(engine.default_port().to_string(), window, c)
            });
        }
        DbEditTarget::Edit { original, .. } => {
            name.update(cx, |s, c| s.set_value(original.name.clone(), window, c));
            host.update(cx, |s, c| s.set_value(original.host.clone(), window, c));
            user.update(cx, |s, c| s.set_value(original.user.clone(), window, c));
            port.update(cx, |s, c| s.set_value(original.port.to_string(), window, c));
            if let Some(db) = original.database.clone() {
                database.update(cx, |s, c| s.set_value(db, window, c));
            }
            if let Some(id) = original.credential_id.as_deref() {
                if let Ok(Some(pw)) = pier_core::credentials::get(id) {
                    password.update(cx, |s, c| s.set_value(pw, window, c));
                }
            }
        }
    }

    let inputs = Rc::new(Inputs {
        name,
        host,
        port,
        user,
        password,
        database,
    });
    let title: SharedString = match &target {
        DbEditTarget::Add => format!("New {} connection", engine.as_str()).into(),
        DbEditTarget::Edit { original, .. } => format!("Edit · {}", original.name).into(),
    };

    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = build_body(app_cx, &inputs);
        let on_ok_inputs = inputs.clone();
        let on_ok_target = target.clone();
        let weak = app.clone();
        dialog
            .title(title.clone())
            .w(px(440.0))
            .confirm()
            .button_props(
                gpui_component::dialog::DialogButtonProps::default()
                    .ok_text("Save")
                    .cancel_text("Cancel"),
            )
            .on_ok(move |_, _w, app_cx| {
                save(&on_ok_inputs, engine, &on_ok_target, &weak, app_cx);
                true
            })
            .child(body)
    });
}

#[derive(Clone)]
struct Inputs {
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    user: Entity<InputState>,
    password: Entity<InputState>,
    database: Entity<InputState>,
}

fn build_body(cx: &App, inputs: &Inputs) -> impl IntoElement {
    let t = theme(cx).clone();
    div()
        .flex()
        .flex_col()
        .gap(SP_3)
        .pt(SP_2)
        .child(field(&t, "Name", &inputs.name))
        .child(field(&t, "Host", &inputs.host))
        .child(
            div()
                .flex()
                .flex_row()
                .gap(SP_2)
                .child(div().flex_1().child(field(&t, "Port", &inputs.port)))
                .child(div().flex_1().child(field(&t, "User", &inputs.user))),
        )
        .child(field(&t, "Password", &inputs.password))
        .child(field(&t, "Database (optional)", &inputs.database))
        .child(
            text::body(
                "Connection metadata is saved to db-connections.json. \
                 Password is written to the OS keychain under pier-x.db.{engine}.{name}.",
            )
            .secondary(),
        )
}

fn field(
    t: &crate::theme::Theme,
    label: &'static str,
    state: &Entity<InputState>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(SP_1)
        .child(label_text(t, label))
        .child(Input::new(state))
}

fn label_text(t: &crate::theme::Theme, label: &'static str) -> impl IntoElement {
    div()
        .text_size(SIZE_CAPTION)
        .font_weight(WEIGHT_MEDIUM)
        .text_color(t.color.text_secondary)
        .child(SharedString::from(label))
}

fn save(
    inputs: &Inputs,
    engine: DbEngine,
    target: &DbEditTarget,
    app: &WeakEntity<PierApp>,
    cx: &mut App,
) {
    let name_raw = inputs.name.read(cx).value().to_string();
    let host_raw = inputs.host.read(cx).value().to_string();
    let user_raw = inputs.user.read(cx).value().to_string();
    let port_str = inputs.port.read(cx).value().to_string();
    let database_raw = inputs.database.read(cx).value().to_string();
    let password = inputs.password.read(cx).value().to_string();

    let name = name_raw.trim().to_string();
    let host = host_raw.trim().to_string();
    let user = user_raw.trim().to_string();
    let database = database_raw.trim();

    if name.is_empty() || host.is_empty() || user.is_empty() {
        log::warn!("db_form: name / host / user are required (save aborted)");
        return;
    }

    let port: u16 = port_str.trim().parse().unwrap_or_else(|_| engine.default_port());

    // Empty password → no credential entry, connect with "" (some
    // local DBs are set up trust-auth or peer-auth).
    let credential_id = if password.is_empty() {
        None
    } else {
        let id = DbConnection::credential_id_for(engine, &name);
        if let Err(err) = pier_core::credentials::set(&id, &password) {
            // Surface as warning but still save the config — the user
            // can re-enter the password later on connect.
            log::warn!("db_form: keychain write failed for {id}: {err}");
        }
        Some(id)
    };

    let conn = DbConnection {
        name,
        engine,
        host,
        port,
        user,
        database: if database.is_empty() {
            None
        } else {
            Some(database.to_string())
        },
        credential_id,
    };

    let mut store = DbConnectionStore::load_default().unwrap_or_default();
    match target {
        DbEditTarget::Add => store.add(conn),
        DbEditTarget::Edit { idx, .. } => {
            if store.replace(*idx, conn.clone()).is_none() {
                // Stale index — append rather than drop the edit.
                store.add(conn);
            }
        }
    }
    if let Err(err) = store.save_default() {
        log::warn!("db_form: save db_connections failed: {err}");
        return;
    }

    let _ = app.update(cx, |this, cx| {
        this.refresh_db_connections();
        cx.notify();
    });
}
