//! Git manager modals — Pier's branch-row icons pop a floating
//! dialog instead of an inline overlay panel. Each `open_*_dialog`
//! is called from a branch-row icon click, builds a scrollable
//! content body from the existing manager panel helper, and
//! dispatches it via `window.open_dialog(...)`.
//!
//! The actions themselves still dispatch through the shared
//! `PierApp::schedule_git_action` pipeline — the dialog is purely
//! a UI vessel.

use gpui::{div, prelude::*, px, App, IntoElement, SharedString, WeakEntity, Window};
use gpui_component::WindowExt;
use rust_i18n::t;

use crate::app::git_session::GitPendingAction;
use crate::app::PierApp;
use crate::components::{text, Separator};
use crate::theme::{
    spacing::{SP_2, SP_3},
    theme,
};

/// Shared height / width for every Git manager modal so they feel
/// like one family. 680 × 480 keeps ~18 rows visible without
/// forcing a vertical scroll on typical repos.
const DIALOG_WIDTH: f32 = 680.0;

// ─── Public dialog openers ──────────────────────────────────────────

/// Branch manager modal — full list, create / rename / delete /
/// merge / rebase actions per row.
pub fn open_branches_dialog(window: &mut Window, cx: &mut App, app: WeakEntity<PierApp>) {
    let title: SharedString = t!("App.Git.mgr_branches").to_string().into();
    ensure_managers_loaded(&app, cx);
    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = branches_dialog_body(app.clone(), app_cx);
        dialog
            .title(title.clone())
            .w(px(DIALOG_WIDTH))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(body)
    });
}

/// Tag manager modal.
pub fn open_tags_dialog(window: &mut Window, cx: &mut App, app: WeakEntity<PierApp>) {
    let title: SharedString = t!("App.Git.mgr_tags").to_string().into();
    ensure_managers_loaded(&app, cx);
    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = tags_dialog_body(app.clone(), app_cx);
        dialog
            .title(title.clone())
            .w(px(DIALOG_WIDTH))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(body)
    });
}

/// Remote manager modal.
pub fn open_remotes_dialog(window: &mut Window, cx: &mut App, app: WeakEntity<PierApp>) {
    let title: SharedString = t!("App.Git.mgr_remotes").to_string().into();
    ensure_managers_loaded(&app, cx);
    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = remotes_dialog_body(app.clone(), app_cx);
        dialog
            .title(title.clone())
            .w(px(DIALOG_WIDTH))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(body)
    });
}

/// Git-config modal.
pub fn open_config_dialog(window: &mut Window, cx: &mut App, app: WeakEntity<PierApp>) {
    let title: SharedString = t!("App.Git.mgr_config").to_string().into();
    ensure_managers_loaded(&app, cx);
    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = config_dialog_body(app.clone(), app_cx);
        dialog
            .title(title.clone())
            .w(px(DIALOG_WIDTH))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(body)
    });
}

/// Submodule manager modal.
pub fn open_submodules_dialog(window: &mut Window, cx: &mut App, app: WeakEntity<PierApp>) {
    let title: SharedString = t!("App.Git.mgr_submodules").to_string().into();
    ensure_managers_loaded(&app, cx);
    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = submodules_dialog_body(app.clone(), app_cx);
        dialog
            .title(title.clone())
            .w(px(DIALOG_WIDTH))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(body)
    });
}

/// Rebase-control modal (continue / abort / skip).
pub fn open_rebase_dialog(window: &mut Window, cx: &mut App, app: WeakEntity<PierApp>) {
    let title: SharedString = t!("App.Git.mgr_rebase").to_string().into();
    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = rebase_dialog_body(app.clone(), app_cx);
        dialog
            .title(title.clone())
            .w(px(520.0))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(body)
    });
}

/// Per-file commit diff dialog. Fetches the diff via
/// `schedule_git_action` through a specialised one-shot path —
/// here we re-use `commit_file_diff` synchronously on a background
/// task dispatched by the view's click handler (see the caller
/// site in views/git.rs). The dialog shows the precomputed diff
/// text that the caller has already loaded.
pub fn open_commit_file_diff_dialog(
    window: &mut Window,
    cx: &mut App,
    title_text: String,
    diff_text: String,
) {
    let t: SharedString = title_text.into();
    let text_arc: std::sync::Arc<String> = std::sync::Arc::new(diff_text);
    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = diff_dialog_body(text_arc.clone(), app_cx);
        dialog
            .title(t.clone())
            .w(px(960.0))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(body)
    });
}

// ─── Dialog body builders ───────────────────────────────────────────

fn branches_dialog_body(app: WeakEntity<PierApp>, cx: &mut App) -> gpui::AnyElement {
    let t = theme(cx).clone();
    let branches = app
        .upgrade()
        .map(|e| e.read(cx).git_state().read(cx).managers.branches.clone())
        .unwrap_or_default();
    if branches.is_empty() {
        return empty_body(&t, t!("App.Git.working_tree_clean")).into_any_element();
    }
    let mut body = scroll_body();
    for b in branches.iter().take(500) {
        body = body.child(branch_mgr_row_local(&t, b, app.clone()));
    }
    body.into_any_element()
}

fn tags_dialog_body(app: WeakEntity<PierApp>, cx: &mut App) -> gpui::AnyElement {
    let t = theme(cx).clone();
    let tags = app
        .upgrade()
        .map(|e| e.read(cx).git_state().read(cx).managers.tags.clone())
        .unwrap_or_default();
    if tags.is_empty() {
        return empty_body(&t, t!("App.Git.no_tags")).into_any_element();
    }
    let mut body = scroll_body();
    for tag in tags.iter().take(500) {
        body = body.child(super::git::tag_row_export(&t, tag, app.clone()));
    }
    body.into_any_element()
}

fn remotes_dialog_body(app: WeakEntity<PierApp>, cx: &mut App) -> gpui::AnyElement {
    let t = theme(cx).clone();
    let remotes = app
        .upgrade()
        .map(|e| e.read(cx).git_state().read(cx).managers.remotes.clone())
        .unwrap_or_default();
    let fetch_all_weak = app.clone();
    let mut body = scroll_body().child(
        div()
            .flex()
            .flex_row()
            .px(SP_3)
            .py(SP_2)
            .child(
                crate::components::Button::secondary(
                    "git-dlg-fetch-all",
                    t!("App.Git.remote_fetch_all"),
                )
                .size(crate::components::ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let _ = fetch_all_weak.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::RemoteFetch { name: None }, cx);
                    });
                }),
            ),
    );
    body = body.child(Separator::horizontal());
    if remotes.is_empty() {
        body = body.child(empty_body(&t, t!("App.Git.no_remotes")));
        return body.into_any_element();
    }
    for r in remotes.iter().take(100) {
        body = body.child(super::git::remote_row_export(&t, r, app.clone()));
    }
    body.into_any_element()
}

fn config_dialog_body(app: WeakEntity<PierApp>, cx: &mut App) -> gpui::AnyElement {
    let t = theme(cx).clone();
    let snapshot = app
        .upgrade()
        .map(|e| e.read(cx).git_state().read(cx).managers.clone_shallow())
        .unwrap_or_default();
    let mut body = scroll_body();
    body = body.child(super::git::config_user_strip(&t, &snapshot));
    body = body.child(Separator::horizontal());
    if snapshot.config.is_empty() {
        body = body.child(empty_body(&t, t!("App.Git.no_config")));
        return body.into_any_element();
    }
    for (i, e) in snapshot.config.iter().enumerate().take(500) {
        body = body.child(super::git::config_row_export(&t, i, e, app.clone()));
    }
    body.into_any_element()
}

fn submodules_dialog_body(app: WeakEntity<PierApp>, cx: &mut App) -> gpui::AnyElement {
    let t = theme(cx).clone();
    let subs = app
        .upgrade()
        .map(|e| e.read(cx).git_state().read(cx).managers.submodules.clone())
        .unwrap_or_default();
    let update_weak = app.clone();
    let mut body = scroll_body().child(
        div().flex().flex_row().px(SP_3).py(SP_2).child(
            crate::components::Button::secondary(
                "git-dlg-sub-update",
                t!("App.Git.submodule_update"),
            )
            .size(crate::components::ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                let _ = update_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::SubmoduleUpdate, cx);
                });
            }),
        ),
    );
    body = body.child(Separator::horizontal());
    if subs.is_empty() {
        body = body.child(empty_body(&t, t!("App.Git.no_submodules")));
        return body.into_any_element();
    }
    for (i, s) in subs.iter().enumerate().take(100) {
        body = body.child(super::git::submodule_row_export(&t, i, s, app.clone()));
    }
    body.into_any_element()
}

fn rebase_dialog_body(app: WeakEntity<PierApp>, cx: &mut App) -> gpui::AnyElement {
    let t = theme(cx).clone();
    super::git::rebase_manager_export(&t, app).into_any_element()
}

fn diff_dialog_body(
    text: std::sync::Arc<String>,
    cx: &mut App,
) -> gpui::AnyElement {
    let t = theme(cx).clone();
    let mut body = div()
        .id("git-diff-dlg-body")
        .w_full()
        .max_h(px(520.0))
        .overflow_y_scroll()
        .flex()
        .flex_col();
    for (i, line) in text.lines().take(5000).enumerate() {
        body = body.child(super::git::diff_line_row_export(&t, i, line));
    }
    body.into_any_element()
}

// ─── Internal helpers ──────────────────────────────────────────────

fn scroll_body() -> gpui::Stateful<gpui::Div> {
    div()
        .id("git-dlg-scroll")
        .w_full()
        .max_h(px(480.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
}

fn empty_body(_t: &crate::theme::Theme, label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .px(SP_3)
        .py(SP_3)
        .child(text::caption(label).secondary())
}

/// Trigger a managers refresh if the cache is empty. Safe to call
/// repeatedly — the scheduler guards against double-dispatch.
fn ensure_managers_loaded(app: &WeakEntity<PierApp>, cx: &mut App) {
    if let Some(entity) = app.upgrade() {
        let needs = {
            let a = entity.read(cx);
            let s = a.git_state().read(cx);
            s.managers.branches.is_empty() && !s.managers.loading
        };
        if needs {
            let _ = entity.update(cx, |app, cx| app.schedule_git_managers(cx));
        }
    }
}

// Locally-duplicated branch row so we can expose it without
// making the underlying `branch_mgr_row` in `views/git.rs` public.
fn branch_mgr_row_local(
    t: &crate::theme::Theme,
    b: &pier_core::services::git::BranchEntry,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    super::git::branch_mgr_row_export(t, b, weak)
}

// Re-export trait for callers that want the shallow-clone helper.
impl crate::app::git_session::ManagersState {
    pub fn clone_shallow(&self) -> super::git::ManagersSnapshot {
        super::git::ManagersSnapshot {
            branches: self.branches.clone(),
            tags: self.tags.clone(),
            remotes: self.remotes.clone(),
            config: self.config.clone(),
            submodules: self.submodules.clone(),
            conflicts: self.conflicts.clone(),
            user_name: self.user_name.clone(),
            user_email: self.user_email.clone(),
            loading: self.loading,
            error: self.error.clone(),
        }
    }
}
