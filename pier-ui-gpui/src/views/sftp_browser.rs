//! Right-panel SFTP browser — Phase 7 replacement for the placeholder.
//!
//! Mirrors `Pier/PierApp/Sources/Views/RightPanel/RemoteFileView.*` (the
//! UI-side view; backend lives in [`crate::app::ssh_session`]).
//!
//! Lifecycle:
//!   1. User clicks an SSH connection in the Servers list → PierApp creates
//!      an `Entity<SshSessionState>` and stashes it as the active session.
//!   2. First time the right panel renders Sftp mode, this view fires
//!      `state.refresh()` which runs `connect_blocking + list_dir_blocking`
//!      on the calling thread (~1-2s freeze on first connect).
//!   3. Subsequent listings reuse the cached SFTP channel.
//!
//! Deferred (later phases):
//!   - background-thread connect with `Connecting…` placeholder
//!   - download / upload buttons + drag-and-drop into Files panel
//!   - rename / delete / mkdir context menu

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{div, prelude::*, px, App, Entity, IntoElement, SharedString, Window};
use gpui_component::{Icon as UiIcon, IconName};

use crate::app::ssh_session::{ConnectStatus, RemoteEntry, SshSessionState};
use crate::components::{text, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3, SP_4},
    theme,
    typography::{SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};

pub type NavigateHandler = Rc<dyn Fn(&PathBuf, &mut Window, &mut App) + 'static>;
pub type GoUpHandler = Rc<dyn Fn(&(), &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct SftpBrowser {
    /// Active SSH session held by PierApp. None = no server picked yet.
    state: Option<Entity<SshSessionState>>,
    on_navigate: NavigateHandler,
    on_go_up: GoUpHandler,
}

impl SftpBrowser {
    pub fn new(
        state: Option<Entity<SshSessionState>>,
        on_navigate: NavigateHandler,
        on_go_up: GoUpHandler,
    ) -> Self {
        Self {
            state,
            on_navigate,
            on_go_up,
        }
    }
}

impl RenderOnce for SftpBrowser {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let SftpBrowser {
            state,
            on_navigate,
            on_go_up,
        } = self;

        let Some(state_entity) = state else {
            return empty_state(t).into_any_element();
        };

        // Pull everything from the cached state. NEVER call refresh here —
        // refresh runs `connect_blocking` + `list_dir_blocking` which would
        // freeze the UI thread on first SFTP tab open. PierApp triggers
        // refresh from the click handlers (open_ssh_terminal /
        // navigate_sftp / sftp_cd_up) so by the time we render, the cached
        // entries are already populated.
        let (cwd_label, host_label, entries, status_pill, last_error) = {
            let s = state_entity.read(cx);
            let cwd_label: SharedString = s.cwd.display().to_string().into();
            let host_label: SharedString =
                format!("{}@{}", s.config.user, s.config.host).into();
            let entries: Vec<RemoteEntry> = s.entries.clone();
            let status_pill = match &s.status {
                ConnectStatus::Idle => StatusPill::new("idle", StatusKind::Warning),
                ConnectStatus::Connected => StatusPill::new("connected", StatusKind::Success),
                ConnectStatus::Failed(_) => StatusPill::new("error", StatusKind::Error),
            };
            let last_error = s.last_error.clone();
            (cwd_label, host_label, entries, status_pill, last_error)
        };

        let header = div()
            .h(px(28.0))
            .px(SP_3)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .border_b_1()
            .border_color(t.color.border_subtle)
            .child(
                div()
                    .id("sftp-up")
                    .w(px(20.0))
                    .h(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(RADIUS_SM)
                    .text_color(t.color.text_secondary)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.color.bg_hover))
                    .on_click({
                        let go_up = on_go_up.clone();
                        move |_, w, app| go_up(&(), w, app)
                    })
                    .child(UiIcon::new(IconName::ArrowUp).size(px(12.0))),
            )
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(t.color.text_secondary)
                    .child(host_label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(SIZE_MONO_SMALL)
                    .font_family(t.font_mono.clone())
                    .text_color(t.color.text_tertiary)
                    .child(cwd_label),
            )
            .child(status_pill);

        let mut body = div().flex().flex_col().py(SP_1);

        if let Some(err) = last_error {
            body = body.child(
                div().px(SP_3).py(SP_2).child(
                    Card::new()
                        .padding(SP_2)
                        .child(SectionLabel::new("Error"))
                        .child(text::body(SharedString::from(err)).secondary()),
                ),
            );
        }

        if entries.is_empty() {
            body = body.child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child("(empty directory or not yet listed — first listing happens on tab open)"),
            );
        } else {
            for entry in entries {
                body = body.child(remote_row(t, &entry, on_navigate.clone()));
            }
        }

        div()
            .h_full()
            .flex()
            .flex_col()
            .child(header)
            .child(div().flex_1().min_h(px(0.0)).child(body))
            .into_any_element()
    }
}

fn empty_state(t: &crate::theme::Theme) -> gpui::AnyElement {
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
                .child("No active SSH session"),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .child("Click a saved connection in the left panel to attach a session here."),
        )
        .into_any_element()
}

fn remote_row(
    t: &crate::theme::Theme,
    entry: &RemoteEntry,
    on_navigate: NavigateHandler,
) -> impl IntoElement {
    let glyph = if entry.is_dir {
        IconName::Folder
    } else {
        IconName::File
    };
    let id_str: SharedString = format!("sftp-row-{}", entry.path).into();
    let name: SharedString = entry.name.clone().into();
    let size_label: SharedString = if entry.is_dir {
        "—".into()
    } else if entry.size < 1024 {
        format!("{} B", entry.size).into()
    } else if entry.size < 1024 * 1024 {
        format!("{:.1} KB", entry.size as f32 / 1024.0).into()
    } else {
        format!("{:.1} MB", entry.size as f32 / (1024.0 * 1024.0)).into()
    };
    let path_buf = PathBuf::from(entry.path.clone());
    let is_dir = entry.is_dir;

    div()
        .id(gpui::ElementId::Name(id_str))
        .h(px(22.0))
        .px(SP_3)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .text_size(SIZE_CAPTION)
        .text_color(if entry.is_dir {
            t.color.text_primary
        } else {
            t.color.text_secondary
        })
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(move |_, w, app| {
            if is_dir {
                on_navigate(&path_buf, w, app);
            }
        })
        .child(
            div()
                .w(px(14.0))
                .h(px(14.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(if entry.is_dir {
                    t.color.accent
                } else {
                    t.color.text_tertiary
                })
                .child(UiIcon::new(glyph).size(px(12.0))),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .child(name),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(size_label),
        )
}
