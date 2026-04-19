//! SFTP file-operation modals (P1-5 Phase A, step 3).
//!
//! Mirrors the [`crate::views::edit_connection`] / [`database_form`]
//! shape: `open_*_dialog` constructs persistent `InputState` entities,
//! pre-fills as needed, and pops a confirm dialog whose OK callback
//! synthesizes a [`SftpMutationKind`] and calls
//! `PierApp::schedule_sftp_mutation` so the mutation runs on the
//! background executor.
//!
//! Two modals here:
//!   * [`open_mkdir_dialog`] — single Name input + Save / Cancel
//!   * [`open_rename_dialog`] — single New Name input pre-filled with
//!     the current basename + Save / Cancel
//!
//! Delete confirmation lives in `state.rs::confirm_sftp_delete` (it
//! reuses Pier's standard Danger confirm template, no text input).
//! Upload / download go through OS file pickers (no Pier dialog).
//!
//! Wired to the sftp_browser hover icons in commit 4 of the series.

use std::path::Path;

use gpui::{div, prelude::*, px, App, Entity, IntoElement, SharedString, WeakEntity, Window};
use gpui_component::{
    input::{Input, InputState},
    WindowExt as _,
};

use crate::app::ssh_session::SftpMutationKind;
use crate::app::PierApp;
use crate::theme::{
    spacing::{SP_1, SP_2, SP_3},
    theme,
    typography::{SIZE_CAPTION, WEIGHT_MEDIUM},
};

/// Open the "New folder" modal. The new directory is created in
/// `parent_dir` (the SFTP browser's current cwd, owned and cloned in).
/// On Save the OK handler builds `SftpMutationKind::Mkdir` with the
/// joined path and dispatches it via `PierApp::schedule_sftp_mutation`.
pub fn open_mkdir_dialog(
    window: &mut Window,
    cx: &mut App,
    app: WeakEntity<PierApp>,
    parent_dir: String,
) {
    let name = cx.new(|c| InputState::new(window, c).placeholder("new-folder"));

    let title: SharedString = "New folder".into();
    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = build_single_field_body(app_cx, "Folder name", &name);
        let on_ok_name = name.clone();
        let on_ok_parent = parent_dir.clone();
        let weak = app.clone();
        dialog
            .title(title.clone())
            .w(px(380.0))
            .confirm()
            .button_props(
                gpui_component::dialog::DialogButtonProps::default()
                    .ok_text("Create")
                    .cancel_text("Cancel"),
            )
            .on_ok(move |_, _w, app_cx| {
                let raw = on_ok_name.read(app_cx).value().to_string();
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    log::warn!("sftp_dialogs::mkdir: empty name (save aborted)");
                    return true;
                }
                if !is_safe_basename(trimmed) {
                    log::warn!(
                        "sftp_dialogs::mkdir: unsafe basename {trimmed:?} (must not contain '/')"
                    );
                    return true;
                }
                let path = join_remote(&on_ok_parent, trimmed);
                let kind = SftpMutationKind::Mkdir { path };
                let _ = weak.update(app_cx, |this, cx| {
                    this.schedule_sftp_mutation(kind, cx);
                });
                true
            })
            .child(body)
    });
}

/// Open the "Rename" modal. Pre-fills the input with the current
/// basename (`original_name`); on Save constructs `from = original_path`
/// and `to = parent(original_path) + "/" + trimmed_new_name` so the
/// rename always stays in the same directory.
///
/// We deliberately don't let the user move across directories from
/// this dialog — that's a Phase B feature (`mv` semantics need their
/// own UX so users don't accidentally relocate big trees).
pub fn open_rename_dialog(
    window: &mut Window,
    cx: &mut App,
    app: WeakEntity<PierApp>,
    original_path: String,
    original_name: String,
) {
    let name = cx.new(|c| InputState::new(window, c).placeholder("new name"));
    name.update(cx, |s, c| s.set_value(original_name.clone(), window, c));

    let title: SharedString = format!("Rename · {original_name}").into();
    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = build_single_field_body(app_cx, "New name", &name);
        let on_ok_name = name.clone();
        let on_ok_path = original_path.clone();
        let on_ok_original = original_name.clone();
        let weak = app.clone();
        dialog
            .title(title.clone())
            .w(px(380.0))
            .confirm()
            .button_props(
                gpui_component::dialog::DialogButtonProps::default()
                    .ok_text("Rename")
                    .cancel_text("Cancel"),
            )
            .on_ok(move |_, _w, app_cx| {
                let raw = on_ok_name.read(app_cx).value().to_string();
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    log::warn!("sftp_dialogs::rename: empty new name (save aborted)");
                    return true;
                }
                if !is_safe_basename(trimmed) {
                    log::warn!(
                        "sftp_dialogs::rename: unsafe basename {trimmed:?} (must not contain '/')"
                    );
                    return true;
                }
                if trimmed == on_ok_original {
                    // No-op rename — close the dialog without dispatching.
                    return true;
                }
                let parent = parent_of(&on_ok_path);
                let to = join_remote(&parent, trimmed);
                let kind = SftpMutationKind::Rename {
                    from: on_ok_path.clone(),
                    to,
                };
                let _ = weak.update(app_cx, |this, cx| {
                    this.schedule_sftp_mutation(kind, cx);
                });
                true
            })
            .child(body)
    });
}

fn build_single_field_body(
    cx: &App,
    label: &'static str,
    state: &Entity<InputState>,
) -> impl IntoElement {
    let t = theme(cx).clone();
    div()
        .flex()
        .flex_col()
        .gap(SP_1)
        .pt(SP_2)
        .child(
            div()
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_secondary)
                .child(SharedString::from(label)),
        )
        .child(Input::new(state))
        .child(
            div().pt(SP_2).pl(SP_3).text_color(t.color.text_tertiary).child(
                SharedString::from("Slashes are not allowed — use the file tree to navigate first."),
            ),
        )
}

/// Reject names that contain a path separator. SFTP servers
/// typically silently reject `/` in `mkdir`/`rename`, but better to
/// catch it client-side with a clear log line.
fn is_safe_basename(name: &str) -> bool {
    !name.contains('/') && !name.contains('\\') && name != "." && name != ".."
}

/// Strip the last path segment from a remote (POSIX) path string,
/// returning everything before the trailing `/`. Empty / root inputs
/// fall back to `"."` so callers always get a useable parent.
fn parent_of(path: &str) -> String {
    let p = Path::new(path);
    p.parent()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ".".to_string())
}

/// POSIX-style join: ensure exactly one `/` between `parent` and `name`.
fn join_remote(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        return name.to_string();
    }
    if parent.ends_with('/') {
        format!("{parent}{name}")
    } else {
        format!("{parent}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_basename_rejects_slashes_and_dot_segments() {
        assert!(is_safe_basename("notes.txt"));
        assert!(is_safe_basename("a-b_c"));
        assert!(!is_safe_basename("a/b"));
        assert!(!is_safe_basename("a\\b"));
        assert!(!is_safe_basename("."));
        assert!(!is_safe_basename(".."));
    }

    #[test]
    fn parent_of_strips_basename_with_posix_root_fallback() {
        assert_eq!(parent_of("/var/log/app.log"), "/var/log");
        assert_eq!(parent_of("/foo"), "/");
        assert_eq!(parent_of("foo"), ".");
        assert_eq!(parent_of(""), ".");
    }

    #[test]
    fn join_remote_handles_trailing_slash() {
        assert_eq!(join_remote("/var/log", "app.log"), "/var/log/app.log");
        assert_eq!(join_remote("/var/log/", "app.log"), "/var/log/app.log");
        assert_eq!(join_remote(".", "x"), "./x");
        assert_eq!(join_remote("", "x"), "x");
    }
}
