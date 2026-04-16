use std::time::SystemTime;

use gpui::{div, prelude::*, IntoElement, SharedString, Window};
use pier_core::connections::ConnectionStore;
use pier_core::paths;
use pier_core::ssh::{AuthMethod, SshConfig};

use crate::components::{text, Button, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_2, SP_3, SP_4},
    theme,
    typography::{SIZE_BODY, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};

#[derive(IntoElement)]
pub struct SshView {
    snapshot: SshSnapshot,
}

impl SshView {
    pub fn new() -> Self {
        Self {
            snapshot: SshSnapshot::probe(),
        }
    }
}

impl Default for SshView {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for SshView {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let snap = self.snapshot;
        let count = snap.connections.len();

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_3)
            .child(text::h2("SSH connections"))
            .child(StatusPill::new(
                format!("{count} saved"),
                if count == 0 {
                    StatusKind::Warning
                } else {
                    StatusKind::Success
                },
            ))
            .child(div().flex_1())
            .child(
                Button::primary("ssh-new", "New connection").on_click(|_, _, _| {
                    eprintln!("[pier] action: New SSH connection (dialog wiring pending)");
                }),
            );

        let store_card = Card::new()
            .padding(SP_3)
            .child(SectionLabel::new("Store"))
            .child(
                div()
                    .text_size(SIZE_MONO_SMALL)
                    .font_family(t.font_mono.clone())
                    .text_color(t.color.text_tertiary)
                    .child(SharedString::from(snap.store_path.clone())),
            )
            .child(
                div()
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child(SharedString::from(snap.store_mtime.clone())),
            );

        let mut list = div().flex().flex_col().gap(SP_2);
        if snap.connections.is_empty() {
            list = list.child(
                Card::new()
                    .child(SectionLabel::new("No saved connections"))
                    .child(text::body(
                        "Click \"New connection\" to add one. The connection list lives at the path shown above and is reloaded when this view re-renders.",
                    ).secondary()),
            );
        } else {
            for (idx, conn) in snap.connections.iter().enumerate() {
                list = list.child(connection_row(t, idx, conn));
            }
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(SP_4)
            .p(SP_4)
            .child(header)
            .child(store_card)
            .child(list)
    }
}

fn connection_row(t: &crate::theme::Theme, idx: usize, conn: &SshConfig) -> impl IntoElement {
    let address: SharedString = format!("{}@{}:{}", conn.user, conn.host, conn.port).into();
    let auth_label: SharedString = match &conn.auth {
        AuthMethod::Agent => "ssh-agent".into(),
        AuthMethod::PublicKeyFile { private_key_path, .. } => {
            format!("key: {private_key_path}").into()
        }
        AuthMethod::KeychainPassword { credential_id } => {
            format!("keychain: {credential_id}").into()
        }
        AuthMethod::DirectPassword { .. } => "inline password".into(),
    };
    let timeout: SharedString = format!("timeout {}s", conn.connect_timeout_secs).into();
    let tags_line: SharedString = if conn.tags.is_empty() {
        "no tags".into()
    } else {
        conn.tags.join(", ").into()
    };

    div()
        .id(("ssh-row", idx))
        .flex()
        .flex_col()
        .gap(SP_1)
        .p(SP_3)
        .rounded(RADIUS_SM)
        .bg(t.color.bg_surface)
        .border_1()
        .border_color(t.color.border_subtle)
        .hover(|s| s.border_color(t.color.border_default))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .child(
                    div()
                        .text_size(SIZE_BODY)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_primary)
                        .child(SharedString::from(conn.name.clone())),
                )
                .child(
                    div()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(tags_line),
                ),
        )
        .child(
            div()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_secondary)
                .child(address),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap(SP_2)
                .child(
                    div()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(auth_label),
                )
                .child(
                    div()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(timeout),
                ),
        )
}

// ─────────────────────────────────────────────────────────
// Snapshot probe (re-runs on every render — cheap, no watcher
// crate. SKILL.md §11 G prefers minimal dependencies.)
// ─────────────────────────────────────────────────────────

struct SshSnapshot {
    connections: Vec<SshConfig>,
    store_path: String,
    store_mtime: String,
}

impl SshSnapshot {
    fn probe() -> Self {
        let connections = ConnectionStore::load_default()
            .map(|s| s.connections)
            .unwrap_or_default();
        let store_path = paths::connections_file()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<no data dir>".into());
        let store_mtime = paths::connections_file()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
            .map(format_mtime)
            .unwrap_or_else(|| "no file yet".into());
        Self {
            connections,
            store_path,
            store_mtime,
        }
    }
}

fn format_mtime(mt: SystemTime) -> String {
    match mt.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => format!("modified {} sec since epoch", d.as_secs()),
        Err(_) => "modified <pre-epoch>".into(),
    }
}

