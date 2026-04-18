//! New / edit SSH connection dialog.
//!
//! Mirrors `Pier/PierApp/Sources/Views/Connection/*` editor sheet at MVP
//! fidelity. Phase 6 adds:
//!   - edit-existing-entry mode (`EditTarget::Edit(idx, …)`)
//!   - auth radio: Agent (default) vs DirectPassword (masked input)
//!   - `Phase 5`'s save-to-disk via [`ConnectionStore::save_default`]
//!
//! Deferred (later phases):
//!   - PublicKeyFile picker
//!   - KeychainPassword wiring (uses `pier_core::credentials`)
//!   - Field-level validation messaging (currently silent reject on blank
//!     name/host/user; see [`save`])

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{div, prelude::*, px, App, Entity, IntoElement, SharedString, WeakEntity, Window};
use gpui_component::{
    input::{Input, InputState},
    radio::{Radio, RadioGroup},
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

/// What this open() invocation is for — append a brand new entry, or
/// replace an existing one at the given index.
#[derive(Clone)]
pub enum EditTarget {
    Add,
    Edit { idx: usize, original: SshConfig },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AuthMode {
    Agent,
    Password,
    KeyFile,
    Keychain,
}

impl AuthMode {
    fn from_index(i: usize) -> Self {
        match i {
            0 => AuthMode::Agent,
            1 => AuthMode::Password,
            2 => AuthMode::KeyFile,
            _ => AuthMode::Keychain,
        }
    }
    fn index(self) -> usize {
        match self {
            AuthMode::Agent => 0,
            AuthMode::Password => 1,
            AuthMode::KeyFile => 2,
            AuthMode::Keychain => 3,
        }
    }
}

/// Open the connection editor as a modal dialog.
pub fn open(window: &mut Window, cx: &mut App, app: WeakEntity<PierApp>, target: EditTarget) {
    // Inputs created once outside the builder closure → persist across
    // dialog re-renders.
    let name = cx.new(|c| InputState::new(window, c).placeholder("e.g. prod-db"));
    let host = cx.new(|c| InputState::new(window, c).placeholder("e.g. db.example.com"));
    let port = cx.new(|c| InputState::new(window, c).placeholder("22"));
    let user = cx.new(|c| InputState::new(window, c).placeholder("e.g. deploy"));
    let group = cx.new(|c| InputState::new(window, c).placeholder("optional — groups in sidebar"));
    let password = cx.new(|c| {
        InputState::new(window, c)
            .masked(true)
            .placeholder("password")
    });
    let key_path = cx.new(|c| InputState::new(window, c).placeholder("e.g. ~/.ssh/id_ed25519"));
    let key_passphrase = cx.new(|c| {
        InputState::new(window, c)
            .masked(true)
            .placeholder("optional passphrase")
    });
    let keychain_password = cx.new(|c| {
        InputState::new(window, c)
            .masked(true)
            .placeholder("password (stored in OS keychain)")
    });

    let initial_mode = match &target {
        EditTarget::Edit {
            original:
                SshConfig {
                    auth: AuthMethod::DirectPassword { password: pw },
                    ..
                },
            ..
        } => {
            password.update(cx, |s, c| s.set_value(pw.clone(), window, c));
            AuthMode::Password
        }
        EditTarget::Edit {
            original:
                SshConfig {
                    auth:
                        AuthMethod::PublicKeyFile {
                            private_key_path,
                            passphrase_credential_id,
                        },
                    ..
                },
            ..
        } => {
            key_path.update(cx, |s, c| s.set_value(private_key_path.clone(), window, c));
            // Look up passphrase from keyring if previously stored.
            if let Some(id) = passphrase_credential_id {
                if let Ok(Some(pp)) = pier_core::credentials::get(id) {
                    key_passphrase.update(cx, |s, c| s.set_value(pp, window, c));
                }
            }
            AuthMode::KeyFile
        }
        EditTarget::Edit {
            original:
                SshConfig {
                    auth: AuthMethod::KeychainPassword { credential_id },
                    ..
                },
            ..
        } => {
            if let Ok(Some(pw)) = pier_core::credentials::get(credential_id) {
                keychain_password.update(cx, |s, c| s.set_value(pw, window, c));
            }
            AuthMode::Keychain
        }
        _ => AuthMode::Agent,
    };

    // Pre-fill the rest if editing.
    if let EditTarget::Edit { original, .. } = &target {
        name.update(cx, |s, c| s.set_value(original.name.clone(), window, c));
        host.update(cx, |s, c| s.set_value(original.host.clone(), window, c));
        user.update(cx, |s, c| s.set_value(original.user.clone(), window, c));
        port.update(cx, |s, c| s.set_value(original.port.to_string(), window, c));
        if let Some(tag) = original.tags.first() {
            group.update(cx, |s, c| s.set_value(tag.clone(), window, c));
        }
    } else {
        // Default port for fresh entries — saves a keystroke.
        port.update(cx, |s, c| s.set_value("22", window, c));
    }

    let inputs = Inputs {
        name,
        host,
        port,
        user,
        group,
        password,
        key_path,
        key_passphrase,
        keychain_password,
    };
    let auth_mode = Rc::new(RefCell::new(initial_mode));
    let title: SharedString = match &target {
        EditTarget::Add => "New SSH connection".into(),
        EditTarget::Edit { original, .. } => format!("Edit · {}", original.name).into(),
    };

    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = build_body(app_cx, &inputs, auth_mode.clone());
        let on_ok_inputs = inputs.clone();
        let on_ok_mode = auth_mode.clone();
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
                save(
                    &on_ok_inputs,
                    *on_ok_mode.borrow(),
                    &on_ok_target,
                    &weak,
                    app_cx,
                );
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
    password: Entity<InputState>,
    key_path: Entity<InputState>,
    key_passphrase: Entity<InputState>,
    keychain_password: Entity<InputState>,
}

fn build_body(cx: &App, inputs: &Inputs, auth_mode: Rc<RefCell<AuthMode>>) -> impl IntoElement {
    let t = theme(cx).clone();
    let current_mode = *auth_mode.borrow();

    let mode_for_click = auth_mode.clone();
    let radio_group = RadioGroup::horizontal("auth-mode")
        .selected_index(Some(current_mode.index()))
        .child(Radio::new("auth-agent").label("ssh-agent"))
        .child(Radio::new("auth-password").label("password"))
        .child(Radio::new("auth-key").label("key file"))
        .child(Radio::new("auth-keychain").label("keychain"))
        .on_click(move |idx, _w, app| {
            *mode_for_click.borrow_mut() = AuthMode::from_index(*idx);
            // Force the dialog body closure to rerun so per-mode fields
            // appear/disappear. `refresh_windows` is the simplest hook —
            // the dialog stack lives in Root which re-renders on any
            // window refresh.
            app.refresh_windows();
        });

    let mut col = div()
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
            div()
                .flex()
                .flex_col()
                .gap(SP_1)
                .child(label_text(&t, "Authentication"))
                .child(radio_group),
        );

    col = match current_mode {
        AuthMode::Agent => col.child(
            text::body(
                "Uses ssh-agent (~/.ssh/config + agent forwarding apply). \
                 No secret is stored by Pier-X.",
            )
            .secondary(),
        ),
        AuthMode::Password => col.child(field(&t, "Password", &inputs.password)).child(
            text::body(
                "Stored in plaintext inside connections.json. Use \
                     \"keychain\" if you want OS-level secret storage.",
            )
            .secondary(),
        ),
        AuthMode::KeyFile => col
            .child(field(&t, "Private key path", &inputs.key_path))
            .child(field(&t, "Passphrase (optional)", &inputs.key_passphrase))
            .child(
                text::body(
                    "OpenSSH private key on disk. Passphrase is stored in \
                     the OS keychain when present, never in connections.json.",
                )
                .secondary(),
            ),
        AuthMode::Keychain => col
            .child(field(&t, "Password", &inputs.keychain_password))
            .child(
                text::body(
                    "Password is written to the OS keychain on save. \
                     connections.json only holds an opaque credential id.",
                )
                .secondary(),
            ),
    };
    col
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
    mode: AuthMode,
    target: &EditTarget,
    app: &WeakEntity<PierApp>,
    cx: &mut App,
) {
    let name = inputs.name.read(cx).value().to_string();
    let host = inputs.host.read(cx).value().to_string();
    let user = inputs.user.read(cx).value().to_string();
    let port_str = inputs.port.read(cx).value().to_string();
    let group = inputs.group.read(cx).value().to_string();
    let password = inputs.password.read(cx).value().to_string();
    let key_path = inputs.key_path.read(cx).value().to_string();
    let key_passphrase = inputs.key_passphrase.read(cx).value().to_string();
    let keychain_password = inputs.keychain_password.read(cx).value().to_string();

    if name.trim().is_empty() || host.trim().is_empty() || user.trim().is_empty() {
        eprintln!("[pier] save: name / host / user are required");
        return;
    }
    let port: u16 = port_str.trim().parse().unwrap_or(22);

    // Compose AuthMethod, writing to OS keychain where appropriate.
    // Reuse existing credential_id when editing so Keychain entries stay
    // stable across saves (avoids accumulating dangling secrets).
    let existing = match target {
        EditTarget::Edit { original, .. } => Some(original),
        _ => None,
    };
    let auth = match mode {
        AuthMode::Agent => AuthMethod::Agent,
        AuthMode::Password => AuthMethod::DirectPassword { password },
        AuthMode::KeyFile => {
            if key_path.trim().is_empty() {
                eprintln!("[pier] save: key file path is required");
                return;
            }
            // Passphrase optional. When present, store in keychain under a
            // stable id derived from the connection name so re-edits hit
            // the same entry.
            let passphrase_credential_id = if key_passphrase.is_empty() {
                None
            } else {
                let id = existing
                    .and_then(|c| match &c.auth {
                        AuthMethod::PublicKeyFile {
                            passphrase_credential_id: Some(id),
                            ..
                        } => Some(id.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| format!("pier-x.passphrase.{}", name.trim()));
                if let Err(err) = pier_core::credentials::set(&id, &key_passphrase) {
                    eprintln!("[pier] keychain write failed: {err}");
                    return;
                }
                Some(id)
            };
            AuthMethod::PublicKeyFile {
                private_key_path: key_path.trim().to_string(),
                passphrase_credential_id,
            }
        }
        AuthMode::Keychain => {
            if keychain_password.is_empty() {
                eprintln!("[pier] save: keychain password is required");
                return;
            }
            let credential_id = existing
                .and_then(|c| match &c.auth {
                    AuthMethod::KeychainPassword { credential_id } => Some(credential_id.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| format!("pier-x.password.{}", name.trim()));
            if let Err(err) = pier_core::credentials::set(&credential_id, &keychain_password) {
                eprintln!("[pier] keychain write failed: {err}");
                return;
            }
            AuthMethod::KeychainPassword { credential_id }
        }
    };

    let mut conf = SshConfig::new(name.trim(), host.trim(), user.trim());
    conf.port = port;
    conf.auth = auth;
    if !group.trim().is_empty() {
        conf.tags = vec![group.trim().to_string()];
    }

    let mut store = ConnectionStore::load_default().unwrap_or_default();
    match target {
        EditTarget::Add => {
            store.connections.push(conf);
        }
        EditTarget::Edit { idx, .. } => {
            if *idx < store.connections.len() {
                store.connections[*idx] = conf;
            } else {
                // Stale index — fall back to append rather than dropping
                // the user's edits.
                store.connections.push(conf);
            }
        }
    }
    if let Err(err) = store.save_default() {
        eprintln!("[pier] save connection failed: {err}");
        return;
    }

    let _ = app.update(cx, |this, cx| {
        this.refresh_connections();
        cx.notify();
    });
}
