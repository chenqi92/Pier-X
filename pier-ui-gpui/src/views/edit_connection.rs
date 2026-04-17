//! New / edit SSH connection dialog.
//!
//! Mirrors `Pier/PierApp/Sources/Views/Connection/*` editor sheet at MVP
//! fidelity:
//!   - inputs: name, host, port, user, optional first tag (for grouping)
//!   - auth: `AuthMethod::Agent` only (covers ssh-agent + ~/.ssh/config users
//!     without needing a Keychain dance — covers ~80% of dev workflows)
//!   - saves through [`ConnectionStore::save_default`] so the JSON file is
//!     written atomically + read back on the next render
//!
//! Deferred (later phases):
//!   - DirectPassword + masked input
//!   - PublicKeyFile picker
//!   - Edit existing entry (currently always appends a new one)
//!   - Delete entry

use gpui::{div, prelude::*, px, App, Entity, IntoElement, SharedString, WeakEntity, Window};
use gpui_component::{
    input::{Input, InputState},
    WindowExt as _,
};
use pier_core::connections::ConnectionStore;
use pier_core::ssh::{AuthMethod, SshConfig};

use crate::app::PierApp;
use crate::components::text;
use crate::theme::{
    spacing::{SP_1, SP_2, SP_3},
    theme,
    typography::{SIZE_CAPTION, WEIGHT_MEDIUM},
};

/// Open the connection editor as a modal dialog.
pub fn open(window: &mut Window, cx: &mut App, app: WeakEntity<PierApp>) {
    // Create inputs once, capture into the dialog builder so user input
    // survives across re-renders.
    let name = cx.new(|c| InputState::new(window, c).placeholder("e.g. prod-db"));
    let host = cx.new(|c| InputState::new(window, c).placeholder("e.g. db.example.com"));
    let port = cx.new(|c| InputState::new(window, c).placeholder("22"));
    let user = cx.new(|c| InputState::new(window, c).placeholder("e.g. deploy"));
    let group = cx.new(|c| InputState::new(window, c).placeholder("optional — groups in sidebar"));

    // Pre-fill port with `22` so a one-tag-per-host workflow is one less
    // keystroke. set_value needs window+cx to refresh blink state.
    port.update(cx, |state, c| state.set_value("22", window, c));

    let inputs = Inputs {
        name,
        host,
        port,
        user,
        group,
    };

    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = build_body(app_cx, &inputs);
        let on_ok_inputs = inputs.clone();
        let weak = app.clone();
        dialog
            .title("New SSH connection")
            .w(px(440.0))
            .confirm()
            .button_props(
                gpui_component::dialog::DialogButtonProps::default()
                    .ok_text("Save")
                    .cancel_text("Cancel"),
            )
            .on_ok(move |_, _w, app_cx| {
                save(&on_ok_inputs, &weak, app_cx);
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
    group: Entity<InputState>,
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
        .child(field(&t, "Group (tag)", &inputs.group))
        .child(
            text::body(
                "Authentication uses ssh-agent (~/.ssh/config + agent forwarding apply). \
                 Password / key-file editors land in a follow-on PR.",
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
        .child(
            div()
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_secondary)
                .child(SharedString::from(label)),
        )
        .child(Input::new(state))
}

fn save(inputs: &Inputs, app: &WeakEntity<PierApp>, cx: &mut App) {
    let name = inputs.name.read(cx).value().to_string();
    let host = inputs.host.read(cx).value().to_string();
    let user = inputs.user.read(cx).value().to_string();
    let port_str = inputs.port.read(cx).value().to_string();
    let group = inputs.group.read(cx).value().to_string();

    if name.trim().is_empty() || host.trim().is_empty() || user.trim().is_empty() {
        eprintln!("[pier] save: name / host / user are required");
        return;
    }
    let port: u16 = port_str.trim().parse().unwrap_or(22);

    let mut conf = SshConfig::new(name.trim(), host.trim(), user.trim());
    conf.port = port;
    conf.auth = AuthMethod::Agent;
    if !group.trim().is_empty() {
        conf.tags = vec![group.trim().to_string()];
    }

    let mut store = ConnectionStore::load_default().unwrap_or_default();
    store.connections.push(conf);
    if let Err(err) = store.save_default() {
        eprintln!("[pier] save connection failed: {err}");
        return;
    }

    let _ = app.update(cx, |this, cx| {
        this.refresh_connections();
        cx.notify();
    });
}
