//! Right-panel SFTP browser — Phase 7 replacement for the placeholder.
//!
//! Mirrors `Pier/PierApp/Sources/Views/RightPanel/RemoteFileView.*` (the
//! UI-side view; backend lives in [`crate::app::ssh_session`]).
//!
//! Lifecycle:
//!   1. User clicks an SSH connection in the Servers list → PierApp creates
//!      an `Entity<SshSessionState>` and stashes it as the active session;
//!      `schedule_remote_bootstrap` kicks the SSH connect off on a
//!      background-executor task.
//!   2. When the user opens the Sftp tab (or navigates a directory),
//!      `schedule_sftp_refresh` runs the `connect_blocking +
//!      open_sftp_blocking + list_dir_blocking` chain inside
//!      `cx.background_executor().spawn(..)`, so the UI thread never blocks
//!      — the status pill flips through `Connecting → Refreshing → Connected`
//!      and the empty-list region shows "(loading remote directory...)"
//!      while the listing is in flight.
//!   3. Subsequent listings reuse the cached SFTP channel; only the
//!      `list_dir_blocking` call runs on the background task.
//!
//! Deferred (later phases):
//!   - download / upload buttons + drag-and-drop into Files panel
//!   - rename / delete / mkdir context menu

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{div, prelude::*, px, App, Entity, IntoElement, SharedString, Window};
use gpui_component::{scroll::ScrollableElement, Icon as UiIcon, IconName};
use rust_i18n::t;

use crate::app::ssh_session::{ConnectStatus, RemoteEntry, SshSessionState};
use crate::components::{
    text, Card, IconButton, IconButtonSize, IconButtonVariant, SectionLabel, StatusKind, StatusPill,
};
use crate::theme::{
    heights::{GLYPH_SM, ICON_SM, ROW_MD_H, ROW_SM_H},
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3, SP_4},
    theme,
    typography::{SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};

pub type NavigateHandler = Rc<dyn Fn(&PathBuf, &mut Window, &mut App) + 'static>;
pub type GoUpHandler = Rc<dyn Fn(&(), &mut Window, &mut App) + 'static>;
/// Click handler for "+folder" / "↑upload" header buttons. The
/// button itself doesn't need any payload; PierApp pulls the
/// session's cwd for itself when minting the mutation.
pub type HeaderActionHandler = Rc<dyn Fn(&(), &mut Window, &mut App) + 'static>;
/// Click handler for hover-only per-row icons. The variant tells
/// PierApp which mutation to dispatch + carries the row's
/// path/name/is_dir context (cheaper than passing each through its
/// own handler type).
pub type RowActionHandler = Rc<dyn Fn(&RowAction, &mut Window, &mut App) + 'static>;

/// Action requested by the per-row hover icons.
#[derive(Clone, Debug)]
pub enum RowAction {
    /// User clicked the rename ✎ icon.
    Rename {
        /// Full remote path of the entry being renamed.
        path: String,
        /// Current basename, used to pre-fill the rename modal.
        name: String,
    },
    /// User clicked the delete 🗑 icon.
    Delete {
        /// Full remote path of the entry to delete.
        path: String,
        /// Basename, used in the confirm-dialog title / detail text.
        name: String,
        /// True for directories — PierApp picks `DeleteDir` (server
        /// will reject non-empty) vs `DeleteFile` accordingly.
        is_dir: bool,
    },
    /// User clicked the download ⬇ icon. Only emitted for files —
    /// `remote_row` doesn't draw the icon for directories.
    Download {
        /// Full remote path of the file to download.
        path: String,
        /// Basename, used as the suggested local filename.
        name: String,
    },
}

#[derive(IntoElement)]
pub struct SftpBrowser {
    /// Active SSH session held by PierApp. None = no server picked yet.
    state: Option<Entity<SshSessionState>>,
    on_navigate: NavigateHandler,
    on_go_up: GoUpHandler,
    on_mkdir: HeaderActionHandler,
    on_upload: HeaderActionHandler,
    on_row_action: RowActionHandler,
}

impl SftpBrowser {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: Option<Entity<SshSessionState>>,
        on_navigate: NavigateHandler,
        on_go_up: GoUpHandler,
        on_mkdir: HeaderActionHandler,
        on_upload: HeaderActionHandler,
        on_row_action: RowActionHandler,
    ) -> Self {
        Self {
            state,
            on_navigate,
            on_go_up,
            on_mkdir,
            on_upload,
            on_row_action,
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
            on_mkdir,
            on_upload,
            on_row_action,
        } = self;

        let Some(state_entity) = state else {
            return empty_state(t).into_any_element();
        };

        // Pull everything from the cached state. NEVER call refresh here —
        // refresh runs `connect_blocking` + `list_dir_blocking` and even
        // though those calls live inside `cx.background_executor().spawn`
        // (see `schedule_sftp_refresh` in app/state.rs), kicking them off
        // from render would still violate CLAUDE.md Rule 6. PierApp
        // triggers refresh from the click handlers (open_ssh_terminal /
        // navigate_sftp / sftp_cd_up); render just reflects whatever
        // `entries` / `status` / `last_error` the cached state carries.
        let (cwd_label, host_label, entries, status_pill, last_error, is_loading) = {
            let s = state_entity.read(cx);
            let cwd_label: SharedString = s.cwd.display().to_string().into();
            let host_label: SharedString = format!("{}@{}", s.config.user, s.config.host).into();
            let entries: Vec<RemoteEntry> = s.entries.clone();
            let status_pill = match &s.status {
                ConnectStatus::Idle => {
                    StatusPill::new(t!("App.Common.Status.idle"), StatusKind::Warning)
                }
                ConnectStatus::Connecting => {
                    StatusPill::new(t!("App.Common.Status.connecting"), StatusKind::Info)
                }
                ConnectStatus::Refreshing => {
                    StatusPill::new(t!("App.Common.Status.loading"), StatusKind::Info)
                }
                ConnectStatus::Connected => {
                    StatusPill::new(t!("App.Common.Status.connected"), StatusKind::Success)
                }
                ConnectStatus::Failed => {
                    StatusPill::new(t!("App.Common.Status.error"), StatusKind::Error)
                }
            };
            let last_error = s.last_error.clone();
            let is_loading = s.is_loading();
            (
                cwd_label,
                host_label,
                entries,
                status_pill,
                last_error,
                is_loading,
            )
        };

        let header = div()
            .h(ROW_MD_H)
            .px(SP_2)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1_5)
            .border_b_1()
            .border_color(t.color.border_subtle)
            .child(
                IconButton::new("sftp-up", IconName::ArrowUp)
                    .size(IconButtonSize::Sm)
                    .variant(IconButtonVariant::Filled)
                    .on_click({
                        let go_up = on_go_up.clone();
                        move |_, w, app| go_up(&(), w, app)
                    }),
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
            .child(status_pill)
            .child(
                IconButton::new("sftp-mkdir", IconName::Plus)
                    .size(IconButtonSize::Sm)
                    .variant(IconButtonVariant::Filled)
                    .on_click({
                        let on_mkdir = on_mkdir.clone();
                        move |_, w, app| on_mkdir(&(), w, app)
                    }),
            )
            .child(
                IconButton::new("sftp-upload", IconName::ArrowUp)
                    .size(IconButtonSize::Sm)
                    .variant(IconButtonVariant::Filled)
                    .on_click({
                        let on_upload = on_upload.clone();
                        move |_, w, app| on_upload(&(), w, app)
                    }),
            );

        let mut body = div().flex().flex_col().px(SP_2).py(SP_2).gap(SP_1);

        if let Some(err) = last_error {
            body = body.child(
                div().px(SP_3).py(SP_2).child(
                    Card::new()
                        .padding(SP_2)
                        .child(SectionLabel::new(t!("App.Common.error")))
                        .child(text::body(SharedString::from(err)).secondary()),
                ),
            );
        }

        if entries.is_empty() {
            let empty_label = if is_loading {
                t!("App.Sftp.loading_directory").to_string()
            } else {
                t!("App.Sftp.empty_directory").to_string()
            };
            body = body.child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child(empty_label),
            );
        } else {
            for entry in entries {
                body = body.child(remote_row(
                    t,
                    &entry,
                    on_navigate.clone(),
                    on_row_action.clone(),
                ));
            }
        }

        div()
            .h_full()
            .flex()
            .flex_col()
            .child(div().bg(t.color.bg_surface).child(header))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .child(body),
            )
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
                .child(SharedString::from(
                    t!("App.Sftp.no_active_session_title").to_string(),
                )),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .child(SharedString::from(
                    t!("App.Sftp.no_active_session_body").to_string(),
                )),
        )
        .into_any_element()
}

fn remote_row(
    t: &crate::theme::Theme,
    entry: &RemoteEntry,
    on_navigate: NavigateHandler,
    on_row_action: RowActionHandler,
) -> impl IntoElement {
    let glyph = if entry.is_dir {
        IconName::Folder
    } else {
        IconName::File
    };
    let id_str: SharedString = format!("sftp-row-{}", entry.path).into();
    let group_name: SharedString = format!("sftp-row-grp-{}", entry.path).into();
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
    let path_string = entry.path.clone();
    let name_string = entry.name.clone();

    div()
        .id(gpui::ElementId::Name(id_str))
        .group(group_name.clone())
        .h(ROW_SM_H)
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .rounded(RADIUS_SM)
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
                .w(ICON_SM)
                .h(ICON_SM)
                .flex()
                .items_center()
                .justify_center()
                .text_color(if entry.is_dir {
                    t.color.accent
                } else {
                    t.color.text_tertiary
                })
                .child(
                    UiIcon::new(glyph)
                        .size(GLYPH_SM)
                        .text_color(if entry.is_dir {
                            t.color.accent
                        } else {
                            t.color.text_tertiary
                        }),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_1)
                .child(
                    div()
                        .min_w(px(0.0))
                        .text_size(SIZE_CAPTION)
                        .font_weight(WEIGHT_MEDIUM)
                        .child(name),
                )
                .when(entry.is_link, |el| {
                    el.child(
                        div()
                            .text_size(SIZE_SMALL)
                            .text_color(t.color.text_tertiary)
                            .child(SharedString::from(t!("App.Sftp.link").to_string())),
                    )
                }),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(size_label),
        )
        .child(row_action_icons(
            t,
            &group_name,
            &path_string,
            &name_string,
            is_dir,
            on_row_action,
        ))
}

/// Hover-only group of action icons (✎ rename / ⬇ download / 🗑 delete).
/// Render as `invisible` by default and `visible` only inside the
/// `.group_hover(group_name)` of the enclosing row, so the icons
/// don't clutter the file list when the user isn't pointing at the
/// row.
fn row_action_icons(
    _t: &crate::theme::Theme,
    group_name: &SharedString,
    path: &str,
    name: &str,
    is_dir: bool,
    on_row_action: RowActionHandler,
) -> impl IntoElement {
    let mut icons = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .invisible()
        .group_hover(group_name.clone(), |s| s.visible());

    // Rename — always available.
    {
        let on_action = on_row_action.clone();
        let id_path = path.to_string();
        let payload = RowAction::Rename {
            path: path.to_string(),
            name: name.to_string(),
        };
        icons = icons.child(
            IconButton::new(
                gpui::ElementId::Name(format!("sftp-row-rename-{id_path}").into()),
                IconName::Replace,
            )
            .size(IconButtonSize::Xs)
            .on_click(move |_, w, app| on_action(&payload, w, app)),
        );
    }

    // Download — files only.
    if !is_dir {
        let on_action = on_row_action.clone();
        let id_path = path.to_string();
        let payload = RowAction::Download {
            path: path.to_string(),
            name: name.to_string(),
        };
        icons = icons.child(
            IconButton::new(
                gpui::ElementId::Name(format!("sftp-row-dl-{id_path}").into()),
                IconName::ArrowDown,
            )
            .size(IconButtonSize::Xs)
            .on_click(move |_, w, app| on_action(&payload, w, app)),
        );
    }

    // Delete — file or dir. Destructive, so visually (through the
    // icon only at this size) we keep it Ghost; confirmation lives
    // in the delete dialog.
    {
        let id_path = path.to_string();
        let payload = RowAction::Delete {
            path: path.to_string(),
            name: name.to_string(),
            is_dir,
        };
        icons = icons.child(
            IconButton::new(
                gpui::ElementId::Name(format!("sftp-row-del-{id_path}").into()),
                IconName::Delete,
            )
            .size(IconButtonSize::Xs)
            .on_click(move |_, w, app| on_row_action(&payload, w, app)),
        );
    }

    icons
}
