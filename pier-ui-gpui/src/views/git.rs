//! Interactive Git panel — reads from the cached [`GitState`] owned by
//! `PierApp`, dispatches user clicks to `schedule_git_*` methods.
//!
//! Layout mirrors the sibling Pier app: a status header, a 4-tab strip
//! (Changes / Graph / Stash / Managers), and a body whose content
//! depends on the active tab. The Managers tab hosts its own sub-tab
//! strip for Branches / Tags / Remotes / Config / Submodules / Rebase
//! / Conflicts.
//!
//! The view is `RenderOnce` — rebuilt every frame — so every render
//! snapshots `GitState` up-front, drops the borrow, and closes over
//! the `WeakEntity<PierApp>` for the action callbacks. No IO happens
//! in `render`.

use gpui::{
    div, prelude::*, px, App, ClickEvent, ElementId, IntoElement, SharedString, WeakEntity, Window,
};
use gpui_component::input::InputState;
use pier_core::git_graph::GraphRow;
use pier_core::services::git::{
    BranchEntry, BranchInfo, CommitDetail, CommitInfo, ConfigEntry, FileStatus, GitFileChange,
    RemoteInfo, ResetMode, StashEntry, SubmoduleInfo, TagInfo,
};
use rust_i18n::t;

use crate::app::git_session::{
    DiffMode, DiffSelection, GitPendingAction, GitState, GitStatus, GitTab, GraphColumn,
    GraphDateRange, GraphFilter, GraphHighlightMode, ManagerTab,
};
use crate::app::PierApp;
use crate::components::{
    compute_graph_col_width, graph_row_canvas, is_head_row, palette_color, text, Button,
    ButtonSize, IconButton, IconButtonSize, IconButtonVariant, InlineInput, InlineInputTone,
    InspectorSection, Separator, StatusKind, StatusPill, TabItem, Tabs,
};
use crate::theme::{
    heights::{BUTTON_SM_H, ICON_MD, ICON_SM},
    radius::{RADIUS_PILL, RADIUS_SM, RADIUS_XS},
    spacing::{SP_0_5, SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_BODY, SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};
use gpui_component::IconName;

/// Maximum number of file-change rows rendered before collapsing
/// into a "+N more" label.
const MAX_CHANGE_ROWS: usize = 50;
/// Log row cap for the embedded "recent commits" card.
const MAX_LOG_ROWS: usize = 30;
/// Maximum graph rows rendered in one pass — paging kicks in after.
const MAX_GRAPH_ROWS: usize = 500;
/// Cap on diff rendered rows (existing legacy cap).
const MAX_DIFF_LINES: usize = 1000;
/// Cap on blame rows rendered.
const MAX_BLAME_LINES: usize = 1000;

#[derive(IntoElement)]
pub struct GitView {
    app: WeakEntity<PierApp>,
}

impl GitView {
    pub fn new(app: WeakEntity<PierApp>) -> Self {
        Self { app }
    }
}

impl RenderOnce for GitView {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx).clone();

        let Some(app_entity) = self.app.upgrade() else {
            return dead_panel(&t).into_any_element();
        };

        // Flag-guarded initial probe — first time the Git mode is
        // rendered after app start, schedule `git status / log /
        // branches / stashes`. Subsequent renders hit the cache.
        app_entity.update(cx, |app, cx| {
            app.schedule_git_initial_refresh(cx);
        });

        let (snapshot, commit_input, stash_input) = {
            let app = app_entity.read(cx);
            let state = app.git_state().read(cx);
            (
                GitSnapshot::from(state),
                app.git_commit_input(),
                app.git_stash_message_input(),
            )
        };

        let weak = self.app.clone();

        match &snapshot.status {
            GitStatus::NotARepo => not_a_repo_layout(&t, &snapshot, weak).into_any_element(),
            GitStatus::Failed if snapshot.repo_path.is_none() => {
                error_layout(&t, &snapshot, weak).into_any_element()
            }
            _ => tab_layout(&t, snapshot, commit_input, stash_input, weak).into_any_element(),
        }
    }
}

// ─── Layout roots ────────────────────────────────────────────────────

fn tab_layout(
    t: &crate::theme::Theme,
    snap: GitSnapshot,
    commit_input: gpui::Entity<InputState>,
    stash_input: gpui::Entity<InputState>,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    let active = snap.tab;

    let body = match active {
        GitTab::Changes => changes_tab_body(t, &snap, commit_input, weak.clone()).into_any_element(),
        GitTab::Graph => graph_tab_body(t, &snap, weak.clone()).into_any_element(),
        GitTab::Stash => stash_tab_body(t, &snap, stash_input, weak.clone()).into_any_element(),
        GitTab::Managers => managers_tab_body(t, &snap, weak.clone()).into_any_element(),
    };

    let mut col = div()
        .w_full()
        .flex()
        .flex_col()
        .bg(t.color.bg_surface)
        .child(header(t, &snap, weak.clone()))
        .child(Separator::horizontal())
        .child(primary_tabs(active, weak.clone()))
        .child(Separator::horizontal());

    // Feedback strips appear once, above the tab body, across all tabs.
    if let Some(msg) = snap.last_confirmation.clone() {
        col = col
            .child(git_feedback_strip(
                t,
                IconName::Check,
                t!("App.Git.last_result"),
                msg,
                false,
            ))
            .child(Separator::horizontal());
    }
    if let Some(err) = snap.action_error.clone() {
        col = col
            .child(git_feedback_strip(
                t,
                IconName::TriangleAlert,
                t!("App.Common.error"),
                err,
                true,
            ))
            .child(Separator::horizontal());
    }
    if let Some(err) = snap.last_error.clone() {
        col = col
            .child(git_feedback_strip(
                t,
                IconName::TriangleAlert,
                t!("App.Common.error"),
                err,
                true,
            ))
            .child(Separator::horizontal());
    }

    col = col.child(body);
    col
}

fn not_a_repo_layout(
    t: &crate::theme::Theme,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .bg(t.color.bg_surface)
        .child(header(t, snap, weak))
        .child(Separator::horizontal())
        .child(
            InspectorSection::new(t!("App.Common.repository"))
                .icon(IconName::Folder)
                .child(
                    div()
                        .px(SP_3)
                        .py(SP_2)
                        .flex()
                        .flex_col()
                        .gap(SP_1_5)
                        .child(text::caption(t!("App.Git.not_a_repo_body")).secondary())
                        .child(div().overflow_hidden().child(text::mono(snap.cwd.clone()))),
                ),
        )
}

fn error_layout(
    t: &crate::theme::Theme,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .bg(t.color.bg_surface)
        .child(header(t, snap, weak))
        .child(Separator::horizontal())
        .child(git_feedback_strip(
            t,
            IconName::TriangleAlert,
            t!("App.Common.error"),
            snap.last_error.clone().unwrap_or_default(),
            true,
        ))
}

fn dead_panel(t: &crate::theme::Theme) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .bg(t.color.bg_surface)
        .child(
            div()
                .px(SP_3)
                .py(SP_2)
                .child(text::caption(t!("App.Common.panel_lost")).secondary()),
        )
        .text_color(t.color.text_primary)
}

// ─── Header + primary tab strip ─────────────────────────────────────

fn header(
    t: &crate::theme::Theme,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let pending_label = snap.pending.as_ref().map(|action| action.label());
    let refresh_busy = snap.pending.is_some();
    let weak_refresh = weak.clone();

    let mut row = div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap(SP_1_5)
        .px(SP_3)
        .py(SP_1_5)
        .text_color(t.color.text_primary)
        .child(status_pill(snap));

    if let Some(label) = pending_label {
        row = row.child(StatusPill::new(
            t!("App.Git.pending", action = label.as_ref()),
            StatusKind::Info,
        ));
    }
    row = row.child(div().flex_1().min_w(px(0.0)));
    if refresh_busy {
        row = row.child(StatusPill::new(t!("App.Git.busy"), StatusKind::Warning));
    } else {
        row = row.child(
            IconButton::new("git-refresh", IconName::RefreshCw)
                .size(IconButtonSize::Sm)
                .variant(IconButtonVariant::Filled)
                .on_click(move |_, _, cx| {
                    let _ = weak_refresh.update(cx, |app, cx| app.schedule_git_refresh(cx));
                }),
        );
    }
    row
}

fn status_pill(snap: &GitSnapshot) -> StatusPill {
    match snap.status {
        GitStatus::Idle => StatusPill::new(t!("App.Common.Status.idle"), StatusKind::Warning),
        GitStatus::Loading => StatusPill::new(t!("App.Common.Status.loading"), StatusKind::Info),
        GitStatus::Ready => StatusPill::new(t!("App.Git.repo_open"), StatusKind::Success),
        GitStatus::NotARepo => StatusPill::new(t!("App.Git.no_repo"), StatusKind::Warning),
        GitStatus::Failed => StatusPill::new(t!("App.Common.error"), StatusKind::Error),
    }
}

fn primary_tabs(active: GitTab, weak: WeakEntity<PierApp>) -> impl IntoElement {
    let mut tabs = Tabs::new().segmented();
    for tab in GitTab::all() {
        let is_active = tab == active;
        let (label_key, icon) = match tab {
            GitTab::Changes => ("App.Git.tab_changes", IconName::Inbox),
            GitTab::Graph => ("App.Git.tab_graph", IconName::GitCommit),
            GitTab::Stash => ("App.Git.tab_stash", IconName::Inspector),
            GitTab::Managers => ("App.Git.tab_managers", IconName::Settings2),
        };
        let w = weak.clone();
        let item = TabItem::new(
            ElementId::Name(format!("git-tab-{}", tab.id_token()).into()),
            t!(label_key),
            is_active,
            move |_, _, cx| {
                let _ = w.update(cx, |app, cx| app.set_git_tab(tab, cx));
            },
        )
        .with_icon(icon);
        tabs = tabs.item(item);
    }
    tabs
}

// ─── Tab: Changes ────────────────────────────────────────────────────

fn changes_tab_body(
    t: &crate::theme::Theme,
    snap: &GitSnapshot,
    commit_input: gpui::Entity<InputState>,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    let mut col = div().w_full().flex().flex_col();

    if let Some(branch) = snap.branch.clone() {
        col = col
            .child(branch_section(
                t,
                &branch,
                &snap.repo_path,
                &snap.branches,
                weak.clone(),
            ))
            .child(Separator::horizontal());
    }
    col = col
        .child(changes_section(
            t,
            &snap.changes,
            snap.diff_selection.as_ref(),
            weak.clone(),
        ))
        .child(Separator::horizontal());
    if snap.diff_selection.is_some() {
        col = col
            .child(diff_section(t, snap, weak.clone()))
            .child(Separator::horizontal());
    }
    col = col
        .child(commit_section(t, &snap.changes, commit_input, weak.clone()))
        .child(Separator::horizontal())
        .child(log_section(t, &snap.log));
    col
}

// ─── Branch card ─────────────────────────────────────────────────────

fn branch_section(
    t: &crate::theme::Theme,
    branch: &BranchInfo,
    repo_path: &Option<SharedString>,
    branches: &[String],
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let tracking: SharedString = if branch.tracking.is_empty() {
        t!("App.Git.no_upstream").into()
    } else {
        branch.tracking.clone().into()
    };
    let pace: SharedString = t!(
        "App.Git.branch_pace",
        ahead = branch.ahead,
        behind = branch.behind
    )
    .into();

    let push_weak = weak.clone();
    let pull_weak = weak.clone();

    let actions = div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_0_5)
        .child(
            Button::secondary("git-pull", t!("App.Git.pull"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let _ = pull_weak.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::Pull, cx);
                    });
                }),
        )
        .child(
            Button::secondary("git-push", t!("App.Git.push"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let _ = push_weak.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::Push, cx);
                    });
                }),
        );

    let info = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_1_5)
        .overflow_hidden()
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_BODY)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child(SharedString::from(branch.name.clone())),
        )
        .child(
            div()
                .flex_none()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(
                    t!("App.Git.branch_tracking", tracking = tracking.as_ref()).to_string(),
                )),
        );
    let pace_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_0_5)
        .child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(pace),
        );
    let path_row = repo_path.as_ref().map(|path| {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .px(SP_3)
            .py(SP_0_5)
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_size(SIZE_MONO_SMALL)
                    .font_family(t.font_mono.clone())
                    .text_color(t.color.text_tertiary)
                    .truncate()
                    .child(SharedString::from(
                        t!("App.Git.repo_path", path = path.as_ref()).to_string(),
                    )),
            )
    });

    let mut section = InspectorSection::new(t!("App.Git.current_branch"))
        .icon(IconName::GitBranch)
        .actions(actions)
        .child(info)
        .child(pace_row);
    if let Some(row) = path_row {
        section = section.child(row);
    }

    let others: Vec<String> = branches
        .iter()
        .filter(|b| b.as_str() != branch.name)
        .cloned()
        .collect();
    if !others.is_empty() {
        section = section.child(
            div()
                .px(SP_3)
                .py(SP_1)
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(t!("App.Git.switch_branch").to_string())),
        );
        for name in others.into_iter().take(24) {
            section = section.child(branch_row(t, name, weak.clone()));
        }
    }

    section
}

fn branch_row(
    t: &crate::theme::Theme,
    name: String,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let name_for_click = name.clone();
    let id = ElementId::Name(SharedString::from(format!("git-checkout-{name}")));
    div()
        .id(ElementId::Name(SharedString::from(format!(
            "git-branch-row-{}",
            short_id(&name)
        ))))
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_0_5)
        .overflow_hidden()
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_secondary)
                .child(SharedString::from(name)),
        )
        .child(
            div().flex_none().child(
                Button::secondary(id, t!("App.Git.checkout"))
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        let branch_name = name_for_click.clone();
                        let _ = weak.update(cx, |app, cx| {
                            app.schedule_git_action(
                                GitPendingAction::CheckoutBranch { name: branch_name },
                                cx,
                            );
                        });
                    }),
            ),
        )
}

// ─── Changes card ────────────────────────────────────────────────────

fn changes_section(
    t: &crate::theme::Theme,
    changes: &[GitFileChange],
    diff_selection: Option<&crate::app::git_session::DiffSelection>,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let staged = changes.iter().filter(|c| c.staged).count();
    let unstaged = changes.len().saturating_sub(staged);

    let stage_all_weak = weak.clone();
    let unstage_all_weak = weak.clone();

    let mut actions = div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .child(StatusPill::new(
            t!("App.Git.staged_count", count = staged),
            if staged > 0 {
                StatusKind::Info
            } else {
                StatusKind::Success
            },
        ))
        .child(StatusPill::new(
            t!("App.Git.unstaged_count", count = unstaged),
            if unstaged > 0 {
                StatusKind::Warning
            } else {
                StatusKind::Success
            },
        ));
    if unstaged > 0 {
        actions = actions.child(
            Button::secondary("git-stage-all", t!("App.Git.stage_all"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let _ = stage_all_weak.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::StageAll, cx);
                    });
                }),
        );
    }
    if staged > 0 {
        actions = actions.child(
            Button::secondary("git-unstage-all", t!("App.Git.unstage_all"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let _ = unstage_all_weak.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::UnstageAll, cx);
                    });
                }),
        );
    }

    let mut section = InspectorSection::new(t!("App.Git.working_tree"))
        .icon(IconName::Inbox)
        .actions(actions);

    if changes.is_empty() {
        return section
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .child(text::caption(t!("App.Git.working_tree_clean")).secondary()),
            )
            .into_any_element();
    }

    for change in changes.iter().take(MAX_CHANGE_ROWS) {
        let is_selected = diff_selection
            .map(|sel| sel.path == change.path && sel.staged == change.staged)
            .unwrap_or(false);
        section = section.child(file_change_row(t, change, is_selected, weak.clone()));
    }
    if changes.len() > MAX_CHANGE_ROWS {
        section = section.child(
            div()
                .px(SP_3)
                .py(SP_1)
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(
                    t!(
                        "App.Git.more_changes",
                        count = changes.len() - MAX_CHANGE_ROWS
                    )
                    .to_string(),
                )),
        );
    }
    section.into_any_element()
}

fn file_change_row(
    t: &crate::theme::Theme,
    change: &GitFileChange,
    is_selected: bool,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let (badge, badge_color) = file_status_badge(t, change.status.clone());
    let path_str = change.path.clone();
    let staged = change.staged;
    let untracked = matches!(change.status, FileStatus::Untracked);

    let stage_weak = weak.clone();
    let unstage_weak = weak.clone();
    let discard_weak = weak.clone();
    let diff_weak = weak.clone();

    let stage_id = ElementId::Name(SharedString::from(format!(
        "git-stage-{}",
        short_id(&path_str)
    )));
    let unstage_id = ElementId::Name(SharedString::from(format!(
        "git-unstage-{}",
        short_id(&path_str)
    )));
    let discard_id = ElementId::Name(SharedString::from(format!(
        "git-discard-{}",
        short_id(&path_str)
    )));
    let diff_id = ElementId::Name(SharedString::from(format!(
        "git-diff-{}-{}",
        if staged { "s" } else { "w" },
        short_id(&path_str)
    )));

    let path_for_stage = path_str.clone();
    let path_for_unstage = path_str.clone();
    let path_for_discard = path_str.clone();
    let path_for_diff = path_str.clone();

    let row_id = ElementId::Name(SharedString::from(format!(
        "git-file-row-{}-{}",
        if staged { "s" } else { "w" },
        short_id(&path_str)
    )));
    let mut row = div()
        .id(row_id)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .px(SP_3)
        .py(SP_0_5)
        .overflow_hidden()
        .when(is_selected, |el| el.bg(t.color.accent_subtle))
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex_none()
                .w(ICON_MD)
                .h(ICON_MD)
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_SM)
                .bg(badge_color)
                .text_color(t.color.text_inverse)
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .child(SharedString::from(badge.to_string())),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(if staged {
                    t.color.text_primary
                } else {
                    t.color.text_secondary
                })
                .child(SharedString::from(path_str.clone())),
        )
        .child(
            div().flex_none().child(
                Button::secondary(
                    diff_id,
                    if is_selected {
                        t!("App.Git.close_diff")
                    } else {
                        t!("App.Git.view_diff")
                    },
                )
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let p = path_for_diff.clone();
                    let currently_open = is_selected;
                    let _ = diff_weak.update(cx, |app, cx| {
                        if currently_open {
                            app.clear_git_diff_selection(cx);
                        } else {
                            app.schedule_git_diff(
                                DiffSelection {
                                    path: p,
                                    staged,
                                    untracked,
                                },
                                cx,
                            );
                        }
                    });
                }),
            ),
        );

    if staged {
        row = row.child(
            div().flex_none().child(
                Button::secondary(unstage_id, t!("App.Git.unstage"))
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        let p = path_for_unstage.clone();
                        let _ = unstage_weak.update(cx, |app, cx| {
                            app.schedule_git_action(GitPendingAction::Unstage { path: p }, cx);
                        });
                    }),
            ),
        );
    } else {
        row = row.child(
            div().flex_none().child(
                Button::secondary(stage_id, t!("App.Git.stage"))
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        let p = path_for_stage.clone();
                        let _ = stage_weak.update(cx, |app, cx| {
                            app.schedule_git_action(GitPendingAction::Stage { path: p }, cx);
                        });
                    }),
            ),
        );
        row = row.child(
            div().flex_none().child(
                Button::danger(discard_id, t!("App.Git.discard"))
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        let p = path_for_discard.clone();
                        let _ = discard_weak.update(cx, |app, cx| {
                            app.schedule_git_action(GitPendingAction::Discard { path: p }, cx);
                        });
                    }),
            ),
        );
    }
    row
}

fn short_id(path: &str) -> String {
    path.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn file_status_badge(t: &crate::theme::Theme, status: FileStatus) -> (&'static str, gpui::Rgba) {
    match status {
        FileStatus::Modified => ("M", t.color.status_warning),
        FileStatus::Added => ("A", t.color.status_success),
        FileStatus::Deleted => ("D", t.color.status_error),
        FileStatus::Renamed => ("R", t.color.status_info),
        FileStatus::Copied => ("C", t.color.status_info),
        FileStatus::Conflicted => ("!", t.color.status_error),
        FileStatus::Untracked => ("?", t.color.text_tertiary),
    }
}

// ─── Commit card ────────────────────────────────────────────────────

fn commit_section(
    _t: &crate::theme::Theme,
    changes: &[GitFileChange],
    input: gpui::Entity<InputState>,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let has_staged = changes.iter().any(|c| c.staged);
    let commit_weak = weak.clone();
    let input_for_click = input.clone();

    let action: gpui::AnyElement = if has_staged {
        Button::primary("git-commit", t!("App.Git.commit_staged"))
            .size(ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                let text: String = input_for_click.read(cx).value().to_string();
                if text.trim().is_empty() {
                    return;
                }
                let _ = commit_weak.update(cx, |app, cx| {
                    app.schedule_git_action(
                        GitPendingAction::Commit {
                            message: text.clone(),
                        },
                        cx,
                    );
                });
            })
            .into_any_element()
    } else {
        StatusPill::new(t!("App.Git.no_staged"), StatusKind::Warning).into_any_element()
    };

    InspectorSection::new(t!("App.Git.commit_section"))
        .icon(IconName::GitCommit)
        .actions(action)
        .child(
            div()
                .px(SP_3)
                .py(SP_1_5)
                .flex()
                .flex_col()
                .gap(SP_1_5)
                .child(InlineInput::new(&input).tone(InlineInputTone::Inset))
                .child(text::caption(t!("App.Git.commit_hint")).secondary()),
        )
}

// ─── Diff card (with inline / side-by-side toggle) ──────────────────

fn diff_section(
    t: &crate::theme::Theme,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let selection = snap
        .diff_selection
        .as_ref()
        .expect("diff_section called without a selection");

    let close_weak = weak.clone();
    let mode_weak_inline = weak.clone();
    let mode_weak_side = weak.clone();
    let current_mode = snap.diff_mode;

    let side_label: SharedString = if selection.untracked {
        t!("App.Git.diff_side_untracked").into()
    } else if selection.staged {
        t!("App.Git.diff_side_staged").into()
    } else {
        t!("App.Git.diff_side_worktree").into()
    };

    let actions = div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .child(StatusPill::new(side_label, StatusKind::Info))
        .child(mode_toggle(
            "git-diff-inline",
            t!("App.Git.diff_inline"),
            current_mode == DiffMode::Inline,
            move |_, _, cx| {
                let _ = mode_weak_inline.update(cx, |app, cx| {
                    app.set_git_diff_mode(DiffMode::Inline, cx);
                });
            },
        ))
        .child(mode_toggle(
            "git-diff-side",
            t!("App.Git.diff_side_by_side"),
            current_mode == DiffMode::SideBySide,
            move |_, _, cx| {
                let _ = mode_weak_side.update(cx, |app, cx| {
                    app.set_git_diff_mode(DiffMode::SideBySide, cx);
                });
            },
        ))
        .child(
            Button::ghost("git-diff-close", t!("App.Git.close_diff"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let _ = close_weak.update(cx, |app, cx| {
                        app.clear_git_diff_selection(cx);
                    });
                }),
        );

    let mut section = InspectorSection::new(t!("App.Git.diff_section"))
        .icon(IconName::Inspector)
        .eyebrow(selection.path.clone())
        .actions(actions);

    if snap.diff_loading {
        return section
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .child(text::caption(t!("App.Git.diff_loading")).secondary()),
            )
            .into_any_element();
    }
    if let Some(err) = snap.diff_error.clone() {
        return section
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .overflow_hidden()
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.status_error)
                    .child(err),
            )
            .into_any_element();
    }
    let Some(text_body) = snap.diff_output.clone() else {
        return section
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .child(text::caption(t!("App.Git.diff_empty")).secondary()),
            )
            .into_any_element();
    };
    if text_body.is_empty() {
        return section
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .child(text::caption(t!("App.Git.diff_empty")).secondary()),
            )
            .into_any_element();
    }

    match snap.diff_mode {
        DiffMode::Inline => {
            let lines: Vec<&str> = text_body.lines().take(MAX_DIFF_LINES).collect();
            let total_lines = text_body.lines().count();
            for (idx, line) in lines.iter().enumerate() {
                section = section.child(diff_line_row(t, idx, line));
            }
            if total_lines > MAX_DIFF_LINES {
                section = section.child(diff_truncated_row(t, total_lines));
            }
        }
        DiffMode::SideBySide => {
            let pairs = split_side_by_side(text_body.as_ref(), MAX_DIFF_LINES);
            let total_hunks = pairs.len();
            for (idx, (left, right)) in pairs.iter().enumerate() {
                section = section.child(diff_side_row(t, idx, left, right));
            }
            if total_hunks >= MAX_DIFF_LINES {
                section = section.child(diff_truncated_row(t, total_hunks));
            }
        }
    }
    section.into_any_element()
}

fn mode_toggle(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    if active {
        Button::primary(id, label).size(ButtonSize::Sm).on_click(on_click)
    } else {
        Button::ghost(id, label).size(ButtonSize::Sm).on_click(on_click)
    }
}

fn diff_truncated_row(t: &crate::theme::Theme, total_lines: usize) -> impl IntoElement {
    div()
        .px(SP_3)
        .py(SP_1)
        .text_size(SIZE_SMALL)
        .text_color(t.color.text_tertiary)
        .child(SharedString::from(
            t!(
                "App.Git.diff_truncated",
                shown = MAX_DIFF_LINES,
                total = total_lines
            )
            .to_string(),
        ))
}

fn diff_line_row(t: &crate::theme::Theme, index: usize, line: &str) -> impl IntoElement {
    let color = diff_line_color(t, line);
    div()
        .id(("git-diff-line", index))
        .flex()
        .flex_row()
        .px(SP_3)
        .py(SP_0_5)
        .overflow_hidden()
        .text_size(SIZE_MONO_SMALL)
        .font_family(t.font_mono.clone())
        .text_color(color)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .child(SharedString::from(line.to_string())),
        )
}

fn diff_side_row(
    t: &crate::theme::Theme,
    index: usize,
    left: &str,
    right: &str,
) -> impl IntoElement {
    let left_color = diff_line_color(t, left);
    let right_color = diff_line_color(t, right);
    let left_bg = diff_line_bg(t, left);
    let right_bg = diff_line_bg(t, right);
    div()
        .id(("git-diff-pair", index))
        .flex()
        .flex_row()
        .gap(SP_1)
        .px(SP_3)
        .py(SP_0_5)
        .overflow_hidden()
        .text_size(SIZE_MONO_SMALL)
        .font_family(t.font_mono.clone())
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .px(SP_1)
                .bg(left_bg)
                .text_color(left_color)
                .child(SharedString::from(left.to_string())),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .px(SP_1)
                .bg(right_bg)
                .text_color(right_color)
                .child(SharedString::from(right.to_string())),
        )
}

fn diff_line_color(t: &crate::theme::Theme, line: &str) -> gpui::Rgba {
    if line.starts_with("@@") {
        t.color.status_info
    } else if line.starts_with('+') && !line.starts_with("+++") {
        t.color.status_success
    } else if line.starts_with('-') && !line.starts_with("---") {
        t.color.status_error
    } else if line.starts_with("diff --git") || line.starts_with("index ") {
        t.color.text_tertiary
    } else {
        t.color.text_secondary
    }
}

fn diff_line_bg(t: &crate::theme::Theme, line: &str) -> gpui::Rgba {
    if line.starts_with('+') && !line.starts_with("+++") {
        gpui::Rgba {
            a: 0.08,
            ..t.color.status_success
        }
    } else if line.starts_with('-') && !line.starts_with("---") {
        gpui::Rgba {
            a: 0.08,
            ..t.color.status_error
        }
    } else {
        gpui::Rgba::default()
    }
}

/// Walk a unified diff and split into `(left, right)` pairs for the
/// side-by-side renderer. Headers (`@@`, `diff --git`) land on both
/// sides; `-` lines go left-only, `+` lines go right-only, context
/// lines go on both.
fn split_side_by_side(text: &str, max_pairs: usize) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut deletes: Vec<String> = Vec::new();
    let mut inserts: Vec<String> = Vec::new();
    for line in text.lines() {
        if out.len() >= max_pairs {
            break;
        }
        if line.starts_with("@@") || line.starts_with("diff --git") || line.starts_with("index ")
            || line.starts_with("+++") || line.starts_with("---")
        {
            flush_pairs(&mut out, &mut deletes, &mut inserts);
            out.push((line.to_string(), line.to_string()));
            continue;
        }
        if let Some(rest) = line.strip_prefix('+') {
            inserts.push(format!("+{rest}"));
        } else if let Some(rest) = line.strip_prefix('-') {
            deletes.push(format!("-{rest}"));
        } else {
            flush_pairs(&mut out, &mut deletes, &mut inserts);
            out.push((line.to_string(), line.to_string()));
        }
    }
    flush_pairs(&mut out, &mut deletes, &mut inserts);
    out
}

fn flush_pairs(out: &mut Vec<(String, String)>, deletes: &mut Vec<String>, inserts: &mut Vec<String>) {
    let n = deletes.len().max(inserts.len());
    for i in 0..n {
        let left = deletes.get(i).cloned().unwrap_or_default();
        let right = inserts.get(i).cloned().unwrap_or_default();
        out.push((left, right));
    }
    deletes.clear();
    inserts.clear();
}

// ─── Tab: Stash (promoted to its own tab) ────────────────────────────

fn stash_tab_body(
    t: &crate::theme::Theme,
    snap: &GitSnapshot,
    stash_input: gpui::Entity<InputState>,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .child(stash_section(t, &snap.stashes, stash_input, weak))
}

fn stash_section(
    t: &crate::theme::Theme,
    stashes: &[StashEntry],
    input: gpui::Entity<InputState>,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let push_weak = weak.clone();
    let input_for_click = input.clone();

    let push_btn = Button::secondary("git-stash-push", t!("App.Git.stash_push"))
        .size(ButtonSize::Sm)
        .on_click(move |_, _, cx| {
            let text: String = input_for_click.read(cx).value().to_string();
            let _ = push_weak.update(cx, |app, cx| {
                app.schedule_git_action(GitPendingAction::StashPush { message: text }, cx);
            });
        });

    let mut section = InspectorSection::new(t!("App.Git.stash_section"))
        .icon(IconName::Inspector)
        .actions(push_btn)
        .child(
            div()
                .px(SP_3)
                .py(SP_1_5)
                .child(InlineInput::new(&input).tone(InlineInputTone::Inset)),
        );

    if stashes.is_empty() {
        return section
            .child(
                div()
                    .px(SP_3)
                    .py(SP_1)
                    .child(text::caption(t!("App.Git.no_stashes")).secondary()),
            )
            .into_any_element();
    }
    for stash in stashes.iter().take(50) {
        section = section.child(stash_row(t, stash, weak.clone()));
    }
    if stashes.len() > 50 {
        section = section.child(
            div()
                .px(SP_3)
                .py(SP_1)
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(
                    t!("App.Git.more_stashes", count = stashes.len() - 50).to_string(),
                )),
        );
    }
    section.into_any_element()
}

fn stash_row(
    t: &crate::theme::Theme,
    stash: &StashEntry,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let apply_weak = weak.clone();
    let pop_weak = weak.clone();
    let drop_weak = weak.clone();

    let idx_apply = stash.index.clone();
    let idx_pop = stash.index.clone();
    let idx_drop = stash.index.clone();

    let safe_id = short_id(&stash.index);
    let apply_id = ElementId::Name(SharedString::from(format!("git-stash-apply-{safe_id}")));
    let pop_id = ElementId::Name(SharedString::from(format!("git-stash-pop-{safe_id}")));
    let drop_id = ElementId::Name(SharedString::from(format!("git-stash-drop-{safe_id}")));

    let row_id = ElementId::Name(SharedString::from(format!("git-stash-row-{}", safe_id)));
    div()
        .id(row_id)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .px(SP_3)
        .py(SP_0_5)
        .overflow_hidden()
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex_none()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(stash.index.clone())),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_primary)
                .child(SharedString::from(stash.message.clone())),
        )
        .child(
            div()
                .flex_none()
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(stash.relative_date.clone())),
        )
        .child(
            div().flex_none().child(
                Button::secondary(apply_id, t!("App.Git.apply"))
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        let id = idx_apply.clone();
                        let _ = apply_weak.update(cx, |app, cx| {
                            app.schedule_git_action(GitPendingAction::StashApply { index: id }, cx);
                        });
                    }),
            ),
        )
        .child(
            div().flex_none().child(
                Button::secondary(pop_id, t!("App.Git.pop"))
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        let id = idx_pop.clone();
                        let _ = pop_weak.update(cx, |app, cx| {
                            app.schedule_git_action(GitPendingAction::StashPop { index: id }, cx);
                        });
                    }),
            ),
        )
        .child(
            div().flex_none().child(
                Button::danger(drop_id, t!("App.Git.drop"))
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        let id = idx_drop.clone();
                        let _ = drop_weak.update(cx, |app, cx| {
                            app.schedule_git_action(GitPendingAction::StashDrop { index: id }, cx);
                        });
                    }),
            ),
        )
}

// ─── Recent-commits log (kept on the Changes tab) ───────────────────

fn log_section(t: &crate::theme::Theme, log: &[CommitInfo]) -> impl IntoElement {
    let count_pill = StatusPill::new(
        t!("App.Git.entries_count", count = log.len()),
        StatusKind::Info,
    );
    let mut section = InspectorSection::new(t!("App.Git.recent_commits"))
        .icon(IconName::GalleryVerticalEnd)
        .actions(count_pill);

    if log.is_empty() {
        return section
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .child(text::caption(t!("App.Git.no_commits")).secondary()),
            )
            .into_any_element();
    }
    for c in log.iter().take(MAX_LOG_ROWS) {
        section = section.child(commit_row(t, c));
    }
    section.into_any_element()
}

fn commit_row(t: &crate::theme::Theme, c: &CommitInfo) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .h(BUTTON_SM_H)
        .px(SP_3)
        .overflow_hidden()
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex_none()
                .w(px(64.0))
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(c.short_hash.clone())),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_primary)
                .child(SharedString::from(c.message.clone())),
        )
        .child(
            div()
                .flex_none()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .truncate()
                .child(SharedString::from(c.author.clone())),
        )
        .child(
            div()
                .flex_none()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(c.relative_date.clone())),
        )
}

// ─── Feedback strip (confirmation / error) ──────────────────────────

fn git_feedback_strip(
    t: &crate::theme::Theme,
    icon: IconName,
    title: impl Into<SharedString>,
    message: SharedString,
    is_error: bool,
) -> impl IntoElement {
    let icon_color = if is_error {
        t.color.status_error
    } else {
        t.color.status_success
    };
    let message_color = if is_error {
        t.color.status_error
    } else {
        t.color.text_secondary
    };
    div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_1_5)
        .child(
            div()
                .flex_none()
                .text_color(icon_color)
                .child(gpui_component::Icon::new(icon).size(ICON_SM)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(SP_0_5)
                .child(text::caption(title).secondary())
                .child(
                    div()
                        .overflow_hidden()
                        .text_size(SIZE_SMALL)
                        .font_family(t.font_mono.clone())
                        .text_color(message_color)
                        .child(message),
                ),
        )
}

// ─── Tab: Graph ──────────────────────────────────────────────────────

fn graph_tab_body(
    t: &crate::theme::Theme,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    let mut col = div().w_full().flex().flex_col();
    col = col.child(graph_toolbar(t, &snap.graph, weak.clone()));
    col = col.child(Separator::horizontal());

    if let Some(err) = snap.graph.error.clone() {
        col = col.child(git_feedback_strip(
            t,
            IconName::TriangleAlert,
            t!("App.Common.error"),
            err,
            true,
        ));
        return col;
    }
    if snap.graph.loading && snap.graph.rows.is_empty() {
        col = col.child(
            div()
                .px(SP_3)
                .py(SP_3)
                .child(text::caption(t!("App.Common.Status.loading")).secondary()),
        );
        return col;
    }
    if snap.graph.rows.is_empty() {
        col = col.child(
            div()
                .px(SP_3)
                .py(SP_3)
                .child(text::caption(t!("App.Git.graph_empty")).secondary()),
        );
        return col;
    }

    // Graph rows — paginated client-side with a visible cap.
    let col_w = compute_graph_col_width(&snap.graph.rows).max(60.0);
    let rows_to_render: Vec<&GraphRow> = snap.graph.rows.iter().take(MAX_GRAPH_ROWS).collect();

    let mut list = div()
        .w_full()
        .flex()
        .flex_col()
        .id("git-graph-list");
    let head_hash = snap
        .graph
        .rows
        .first()
        .map(|r| r.hash.clone())
        .unwrap_or_default();
    for (idx, row) in rows_to_render.iter().enumerate() {
        let is_selected = snap.graph.selected.as_deref() == Some(row.hash.as_str());
        let is_head = row.hash == head_hash;
        let dimmed = should_dim_row(row, snap, is_head);
        let zebra = snap.graph.zebra_stripes && idx % 2 == 1;
        list = list.child(graph_row_element(
            t,
            row,
            col_w,
            dimmed,
            is_selected,
            zebra,
            snap,
            weak.clone(),
        ));
        if is_selected {
            if let Some(detail) = snap.commit_detail.detail.as_ref() {
                list = list.child(commit_detail_strip(t, detail, snap, weak.clone()));
            } else if snap.commit_detail.loading {
                list = list.child(
                    div()
                        .px(SP_3)
                        .py(SP_1_5)
                        .child(text::caption(t!("App.Common.Status.loading")).secondary()),
                );
            } else if let Some(err) = snap.commit_detail.error.clone() {
                list = list.child(git_feedback_strip(
                    t,
                    IconName::TriangleAlert,
                    t!("App.Common.error"),
                    err,
                    true,
                ));
            }
        }
    }
    if snap.graph.has_more {
        let load_weak = weak.clone();
        list = list.child(
            div()
                .w_full()
                .px(SP_3)
                .py(SP_2)
                .flex()
                .flex_row()
                .justify_center()
                .child(
                    Button::ghost("git-graph-load-more", t!("App.Git.graph_load_more"))
                        .size(ButtonSize::Sm)
                        .on_click(move |_, _, cx| {
                            let _ = load_weak.update(cx, |app, cx| {
                                app.schedule_git_graph(false, cx);
                            });
                        }),
                ),
        );
    } else if !snap.graph.rows.is_empty() {
        list = list.child(
            div()
                .w_full()
                .px(SP_3)
                .py(SP_2)
                .flex()
                .flex_row()
                .justify_center()
                .child(text::caption(t!("App.Git.graph_all_loaded")).secondary()),
        );
    }

    col.child(list)
}

fn graph_toolbar(
    t: &crate::theme::Theme,
    graph: &GraphStateSnapshot,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    let mut row = div()
        .w_full()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(SP_1)
        .px(SP_3)
        .py(SP_1_5)
        .bg(t.color.bg_panel);

    // Search box (placeholder — live click re-triggers graph reload).
    let search = graph.filter.search_text.clone().unwrap_or_default();
    let search_label: SharedString = if search.is_empty() {
        t!("App.Git.graph_search_placeholder").into()
    } else {
        SharedString::from(search.clone())
    };
    let cur_filter = graph.filter.clone();
    let weak_search = weak.clone();
    row = row.child(
        filter_chip(
            t,
            "git-graph-search",
            IconName::Search,
            search_label,
            !search.is_empty(),
            move |_, _, cx| {
                // Toggle: clear search if already set.
                let next = GraphFilter {
                    search_text: None,
                    ..cur_filter.clone()
                };
                let _ = weak_search.update(cx, |app, cx| {
                    app.set_git_graph_filter(next.clone(), cx);
                });
            },
        ),
    );

    // Branch chip
    let branch_label: SharedString = graph
        .filter
        .branch
        .clone()
        .map(SharedString::from)
        .unwrap_or_else(|| t!("App.Git.graph_all_branches").into());
    let weak_branch_clear = weak.clone();
    let cur_filter_b = graph.filter.clone();
    row = row.child(filter_chip(
        t,
        "git-graph-branch",
        IconName::GitBranch,
        branch_label,
        graph.filter.branch.is_some(),
        move |_, _, cx| {
            let next = GraphFilter {
                branch: None,
                ..cur_filter_b.clone()
            };
            let _ = weak_branch_clear.update(cx, |app, cx| {
                app.set_git_graph_filter(next.clone(), cx);
            });
        },
    ));

    // Author chip
    let user_label: SharedString = graph
        .filter
        .author
        .clone()
        .map(SharedString::from)
        .unwrap_or_else(|| t!("App.Git.graph_all_users").into());
    let weak_user_clear = weak.clone();
    let cur_filter_u = graph.filter.clone();
    row = row.child(filter_chip(
        t,
        "git-graph-user",
        IconName::User,
        user_label,
        graph.filter.author.is_some(),
        move |_, _, cx| {
            let next = GraphFilter {
                author: None,
                ..cur_filter_u.clone()
            };
            let _ = weak_user_clear.update(cx, |app, cx| {
                app.set_git_graph_filter(next.clone(), cx);
            });
        },
    ));

    // Date range chip
    let date_label: SharedString = match graph.filter.date_range {
        GraphDateRange::All => t!("App.Git.graph_date").into(),
        GraphDateRange::Today => t!("App.Git.graph_date_today").into(),
        GraphDateRange::LastWeek => t!("App.Git.graph_date_week").into(),
        GraphDateRange::LastMonth => t!("App.Git.graph_date_month").into(),
        GraphDateRange::LastYear => t!("App.Git.graph_date_year").into(),
    };
    let weak_date = weak.clone();
    let cur_filter_d = graph.filter.clone();
    row = row.child(filter_chip(
        t,
        "git-graph-date",
        IconName::Calendar,
        date_label,
        graph.filter.date_range != GraphDateRange::All,
        move |_, _, cx| {
            // Cycle through ranges on click.
            let next_range = match cur_filter_d.date_range {
                GraphDateRange::All => GraphDateRange::Today,
                GraphDateRange::Today => GraphDateRange::LastWeek,
                GraphDateRange::LastWeek => GraphDateRange::LastMonth,
                GraphDateRange::LastMonth => GraphDateRange::LastYear,
                GraphDateRange::LastYear => GraphDateRange::All,
            };
            let next = GraphFilter {
                date_range: next_range,
                ..cur_filter_d.clone()
            };
            let _ = weak_date.update(cx, |app, cx| {
                app.set_git_graph_filter(next.clone(), cx);
            });
        },
    ));

    // Path chip
    let path_active = graph.filter.path_filter.is_some();
    let path_label: SharedString = if path_active {
        let path = graph.filter.path_filter.clone().unwrap_or_default();
        let trimmed: String = path.chars().take(24).collect();
        SharedString::from(trimmed)
    } else {
        t!("App.Git.graph_path").into()
    };
    let weak_path = weak.clone();
    let cur_filter_p = graph.filter.clone();
    row = row.child(filter_chip(
        t,
        "git-graph-path",
        IconName::Folder,
        path_label,
        path_active,
        move |_, _, cx| {
            let next = GraphFilter {
                path_filter: None,
                ..cur_filter_p.clone()
            };
            let _ = weak_path.update(cx, |app, cx| {
                app.set_git_graph_filter(next.clone(), cx);
            });
        },
    ));

    // Options toggles — first-parent, no-merges, long edges, sort.
    let weak_fp = weak.clone();
    let cur_fp = graph.filter.clone();
    let fp_label: SharedString = if graph.filter.first_parent_only {
        t!("App.Git.graph_first_parent").into()
    } else {
        t!("App.Git.graph_options").into()
    };
    row = row.child(filter_chip(
        t,
        "git-graph-first-parent",
        IconName::ChartPie,
        fp_label,
        graph.filter.first_parent_only,
        move |_, _, cx| {
            let next = GraphFilter {
                first_parent_only: !cur_fp.first_parent_only,
                ..cur_fp.clone()
            };
            let _ = weak_fp.update(cx, |app, cx| {
                app.set_git_graph_filter(next.clone(), cx);
            });
        },
    ));
    let weak_nm = weak.clone();
    let cur_nm = graph.filter.clone();
    row = row.child(filter_chip(
        t,
        "git-graph-no-merges",
        IconName::Minus,
        t!("App.Git.graph_no_merges"),
        graph.filter.no_merges,
        move |_, _, cx| {
            let next = GraphFilter {
                no_merges: !cur_nm.no_merges,
                ..cur_nm.clone()
            };
            let _ = weak_nm.update(cx, |app, cx| {
                app.set_git_graph_filter(next.clone(), cx);
            });
        },
    ));
    let weak_le = weak.clone();
    let cur_le = graph.filter.clone();
    let le_label: SharedString = if graph.filter.show_long_edges {
        t!("App.Git.graph_expand_lin").into()
    } else {
        t!("App.Git.graph_collapse_lin").into()
    };
    row = row.child(filter_chip(
        t,
        "git-graph-long-edges",
        IconName::ChevronsUpDown,
        le_label,
        graph.filter.show_long_edges,
        move |_, _, cx| {
            let next = GraphFilter {
                show_long_edges: !cur_le.show_long_edges,
                ..cur_le.clone()
            };
            let _ = weak_le.update(cx, |app, cx| {
                app.set_git_graph_filter(next.clone(), cx);
            });
        },
    ));
    let weak_sort = weak.clone();
    let cur_sort = graph.filter.clone();
    let sort_label: SharedString = if graph.filter.sort_by_date {
        t!("App.Git.graph_sort_date").into()
    } else {
        t!("App.Git.graph_sort_topo").into()
    };
    row = row.child(filter_chip(
        t,
        "git-graph-sort",
        IconName::SortDescending,
        sort_label,
        graph.filter.sort_by_date,
        move |_, _, cx| {
            let next = GraphFilter {
                sort_by_date: !cur_sort.sort_by_date,
                ..cur_sort.clone()
            };
            let _ = weak_sort.update(cx, |app, cx| {
                app.set_git_graph_filter(next.clone(), cx);
            });
        },
    ));

    row = row.child(div().flex_1().min_w(px(0.0)));

    // Highlight mode cycling chip
    let hm = graph.highlight_mode;
    let hm_label: SharedString = match hm {
        GraphHighlightMode::None => t!("App.Git.graph_highlight_none").into(),
        GraphHighlightMode::MyCommits => t!("App.Git.graph_highlight_my").into(),
        GraphHighlightMode::MergeCommits => t!("App.Git.graph_highlight_merge").into(),
        GraphHighlightMode::CurrentBranch => t!("App.Git.graph_highlight_branch").into(),
    };
    let weak_hm = weak.clone();
    row = row.child(filter_chip(
        t,
        "git-graph-highlight",
        IconName::Eye,
        hm_label,
        hm != GraphHighlightMode::None,
        move |_, _, cx| {
            let next = match hm {
                GraphHighlightMode::None => GraphHighlightMode::MyCommits,
                GraphHighlightMode::MyCommits => GraphHighlightMode::MergeCommits,
                GraphHighlightMode::MergeCommits => GraphHighlightMode::CurrentBranch,
                GraphHighlightMode::CurrentBranch => GraphHighlightMode::None,
            };
            let _ = weak_hm.update(cx, |app, cx| {
                app.set_git_graph_highlight(next, cx);
            });
        },
    ));

    // Zebra toggle
    let weak_zebra = weak.clone();
    row = row.child(filter_chip(
        t,
        "git-graph-zebra",
        IconName::LayoutDashboard,
        t!("App.Git.graph_zebra"),
        graph.zebra_stripes,
        move |_, _, cx| {
            let _ = weak_zebra.update(cx, |app, cx| app.toggle_git_graph_zebra(cx));
        },
    ));

    // Column toggles
    for (kind, label, active) in [
        (
            GraphColumn::Hash,
            t!("App.Git.graph_col_hash"),
            graph.show_hash_col,
        ),
        (
            GraphColumn::Author,
            t!("App.Git.graph_col_author"),
            graph.show_author_col,
        ),
        (
            GraphColumn::Date,
            t!("App.Git.graph_col_date"),
            graph.show_date_col,
        ),
    ] {
        let weak_col = weak.clone();
        let id = match kind {
            GraphColumn::Hash => "git-graph-col-hash",
            GraphColumn::Author => "git-graph-col-author",
            GraphColumn::Date => "git-graph-col-date",
        };
        row = row.child(filter_chip(
            t,
            id,
            IconName::CaseSensitive,
            label,
            active,
            move |_, _, cx| {
                let _ = weak_col.update(cx, |app, cx| app.toggle_git_graph_column(kind, cx));
            },
        ));
    }

    row
}

fn filter_chip(
    t: &crate::theme::Theme,
    id: impl Into<ElementId>,
    icon: IconName,
    label: impl Into<SharedString>,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (bg, fg) = if active {
        (t.color.accent_subtle, t.color.accent)
    } else {
        (t.color.bg_surface, t.color.text_secondary)
    };
    div()
        .id(id.into())
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_0_5)
        .h(BUTTON_SM_H)
        .px(SP_2)
        .rounded(RADIUS_SM)
        .bg(bg)
        .text_size(SIZE_SMALL)
        .text_color(fg)
        .cursor_pointer()
        .border_1()
        .border_color(t.color.border_subtle)
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex_none()
                .child(gpui_component::Icon::new(icon).size(ICON_SM)),
        )
        .child(div().truncate().max_w(px(180.0)).child(label.into()))
        .on_click(on_click)
}

fn graph_row_element(
    t: &crate::theme::Theme,
    row: &GraphRow,
    col_w: f32,
    dimmed: bool,
    selected: bool,
    zebra: bool,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let hash = row.hash.clone();
    let row_id = ElementId::Name(SharedString::from(format!(
        "git-graph-row-{}",
        short_id(&hash)
    )));
    let click_weak = weak.clone();
    let click_hash = hash.clone();

    let dim_factor = if dimmed { 0.3 } else { 1.0 };
    let row_bg = if selected {
        t.color.accent_subtle
    } else if zebra {
        t.color.bg_hover
    } else {
        t.color.bg_surface
    };

    let dot_color = palette_color(t, row.color_index);
    let hash_text_color = if is_head_row(row) {
        dot_color
    } else {
        t.color.accent
    };

    // Refs as little colored pills
    let mut refs_row = div().flex_none().flex().flex_row().gap(SP_0_5);
    if !row.refs.is_empty() {
        // The `refs` string from pier-core is like "(HEAD -> main, origin/main, tag: v1)"
        // Split by comma.
        let inner = row
            .refs
            .trim_start_matches(" (")
            .trim_end_matches(')');
        for (i, piece) in inner
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .enumerate()
            .take(4)
        {
            let ref_color = palette_color(t, (i as i32) % 8);
            refs_row = refs_row.child(
                div()
                    .flex_none()
                    .px(SP_1)
                    .py(SP_0_5)
                    .rounded(RADIUS_XS)
                    .text_size(SIZE_CAPTION)
                    .font_weight(WEIGHT_MEDIUM)
                    .bg(gpui::Rgba {
                        a: 0.15,
                        ..ref_color
                    })
                    .text_color(ref_color)
                    .child(SharedString::from(piece.to_string())),
            );
        }
    }

    let mut outer = div()
        .id(row_id)
        .flex()
        .flex_row()
        .items_center()
        .h(px(22.0))
        .w_full()
        .bg(row_bg)
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(move |_, _, cx| {
            let h = click_hash.clone();
            let _ = click_weak.update(cx, |app, cx| {
                app.toggle_git_graph_selected(h, cx);
            });
        })
        .child(
            div()
                .flex_none()
                .w(px(col_w))
                .h(px(22.0))
                .child(graph_row_canvas(row, t, col_w, dim_factor)),
        );

    if snap.graph.show_hash_col {
        outer = outer.child(
            div()
                .flex_none()
                .w(px(62.0))
                .px(SP_1)
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(hash_text_color)
                .child(SharedString::from(row.short_hash.clone())),
        );
    }

    outer = outer.child(
        div()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_row()
            .gap(SP_1)
            .items_center()
            .child(refs_row)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(SIZE_SMALL)
                    .text_color(if dimmed {
                        t.color.text_tertiary
                    } else {
                        t.color.text_primary
                    })
                    .child(SharedString::from(row.message.clone())),
            ),
    );

    if snap.graph.show_author_col {
        outer = outer.child(
            div()
                .flex_none()
                .w(px(120.0))
                .px(SP_1)
                .truncate()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(row.author.clone())),
        );
    }
    if snap.graph.show_date_col {
        outer = outer.child(
            div()
                .flex_none()
                .w(px(100.0))
                .px(SP_1)
                .truncate()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(format_timestamp(row.date_timestamp))),
        );
    }

    outer
}

fn format_timestamp(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let diff = now.saturating_sub(ts);
    if diff < 60 {
        format!("{}s", diff)
    } else if diff < 3600 {
        format!("{}m", diff / 60)
    } else if diff < 86_400 {
        format!("{}h", diff / 3600)
    } else if diff < 30 * 86_400 {
        format!("{}d", diff / 86_400)
    } else if diff < 365 * 86_400 {
        format!("{}mo", diff / (30 * 86_400))
    } else {
        format!("{}y", diff / (365 * 86_400))
    }
}

fn should_dim_row(row: &GraphRow, snap: &GitSnapshot, _is_head: bool) -> bool {
    match snap.graph.highlight_mode {
        GraphHighlightMode::None => false,
        GraphHighlightMode::MyCommits => {
            let me = &snap.graph.user_name;
            !me.is_empty() && row.author != *me
        }
        GraphHighlightMode::MergeCommits => row.parents.split(' ').filter(|s| !s.is_empty()).count() < 2,
        GraphHighlightMode::CurrentBranch => row.color_index != 0,
    }
}

fn commit_detail_strip(
    t: &crate::theme::Theme,
    detail: &CommitDetail,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let weak_ctx_new_branch = weak.clone();
    let weak_ctx_new_tag = weak.clone();
    let weak_ctx_cherry = weak.clone();
    let weak_ctx_revert = weak.clone();
    let weak_ctx_reset_mixed = weak.clone();
    let weak_ctx_reset_hard = weak.clone();
    let weak_ctx_checkout = weak.clone();

    let hash_for_cb_branch = detail.hash.clone();
    let hash_for_cb_tag = detail.hash.clone();
    let hash_for_cb_cherry = detail.hash.clone();
    let hash_for_cb_revert = detail.hash.clone();
    let hash_for_cb_reset_mixed = detail.hash.clone();
    let hash_for_cb_reset_hard = detail.hash.clone();
    let hash_for_cb_checkout = detail.hash.clone();
    let is_unpushed = snap.graph.unpushed.contains(&detail.hash);
    let parent = snap
        .graph
        .rows
        .iter()
        .find(|r| r.hash == detail.hash)
        .and_then(|r| r.parents.split(' ').next().map(|s| s.to_string()));

    let action_row = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(SP_1)
        .px(SP_3)
        .py(SP_1_5)
        .child(
            Button::secondary("git-ctx-checkout", t!("App.Git.ctx_checkout_revision"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let h = hash_for_cb_checkout.clone();
                    let _ = weak_ctx_checkout.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::CheckoutHash { hash: h }, cx);
                    });
                }),
        )
        .child(
            Button::secondary("git-ctx-new-branch", t!("App.Git.ctx_new_branch"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    // The Managers → Branches tab drives branch creation. From
                    // here we emit a create at the current hash, named
                    // `branch-<shorthash>` as a sane default.
                    let h = hash_for_cb_branch.clone();
                    let name = format!("branch-{}", &h.chars().take(7).collect::<String>());
                    let _ = weak_ctx_new_branch.update(cx, |app, cx| {
                        app.schedule_git_action(
                            GitPendingAction::BranchCreate {
                                name,
                                base: Some(h),
                            },
                            cx,
                        );
                    });
                }),
        )
        .child(
            Button::secondary("git-ctx-new-tag", t!("App.Git.ctx_new_tag"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let h = hash_for_cb_tag.clone();
                    let name = format!("tag-{}", &h.chars().take(7).collect::<String>());
                    let _ = weak_ctx_new_tag.update(cx, |app, cx| {
                        app.schedule_git_action(
                            GitPendingAction::TagCreate {
                                name,
                                message: String::new(),
                                at: Some(h),
                            },
                            cx,
                        );
                    });
                }),
        )
        .child(
            Button::secondary("git-ctx-cherry", t!("App.Git.ctx_cherry_pick"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let h = hash_for_cb_cherry.clone();
                    let _ = weak_ctx_cherry.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::CherryPick { hash: h }, cx);
                    });
                }),
        )
        .child(
            Button::secondary("git-ctx-revert", t!("App.Git.ctx_revert"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let h = hash_for_cb_revert.clone();
                    let _ = weak_ctx_revert.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::Revert { hash: h }, cx);
                    });
                }),
        )
        .child(
            Button::secondary("git-ctx-reset-mixed", t!("App.Git.reset_mixed"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let h = hash_for_cb_reset_mixed.clone();
                    let _ = weak_ctx_reset_mixed.update(cx, |app, cx| {
                        app.schedule_git_action(
                            GitPendingAction::Reset {
                                mode: ResetMode::Mixed,
                                target: h,
                            },
                            cx,
                        );
                    });
                }),
        )
        .child(
            Button::danger("git-ctx-reset-hard", t!("App.Git.reset_hard"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let h = hash_for_cb_reset_hard.clone();
                    let _ = weak_ctx_reset_hard.update(cx, |app, cx| {
                        app.schedule_git_action(
                            GitPendingAction::Reset {
                                mode: ResetMode::Hard,
                                target: h,
                            },
                            cx,
                        );
                    });
                }),
        );

    let mut detail_col = div()
        .flex()
        .flex_col()
        .bg(t.color.bg_panel)
        .border_t_1()
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(SP_2)
                .px(SP_3)
                .py(SP_1_5)
                .child(
                    div()
                        .text_size(SIZE_MONO_SMALL)
                        .font_family(t.font_mono.clone())
                        .text_color(t.color.accent)
                        .child(SharedString::from(detail.short_hash.clone())),
                )
                .child(
                    div()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_secondary)
                        .child(SharedString::from(format!(
                            "{} <{}>",
                            detail.author, detail.author_email
                        ))),
                )
                .child(
                    div()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(SharedString::from(detail.date.clone())),
                ),
        )
        .child(
            div()
                .px(SP_3)
                .py(SP_1)
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_primary)
                .child(SharedString::from(detail.message.clone())),
        );

    if !detail.files.is_empty() {
        detail_col = detail_col.child(
            div()
                .px(SP_3)
                .py(SP_0_5)
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(
                    t!(
                        "App.Git.detail_files_changed",
                        count = detail.files.len()
                    )
                    .to_string(),
                )),
        );
        for (i, f) in detail.files.iter().enumerate().take(24) {
            detail_col = detail_col.child(
                div()
                    .id(("git-detail-file", i))
                    .flex()
                    .flex_row()
                    .gap(SP_1)
                    .px(SP_3)
                    .py(SP_0_5)
                    .items_center()
                    .child(
                        div()
                            .flex_none()
                            .text_size(SIZE_CAPTION)
                            .font_family(t.font_mono.clone())
                            .text_color(t.color.status_success)
                            .child(SharedString::from(format!("+{}", f.additions))),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(SIZE_CAPTION)
                            .font_family(t.font_mono.clone())
                            .text_color(t.color.status_error)
                            .child(SharedString::from(format!("-{}", f.deletions))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(SIZE_SMALL)
                            .font_family(t.font_mono.clone())
                            .text_color(t.color.accent)
                            .child(SharedString::from(f.path.clone())),
                    ),
            );
        }
    }

    detail_col = detail_col.child(action_row);

    if is_unpushed {
        let weak_undo = weak.clone();
        let h_undo = detail.hash.clone();
        let weak_drop2 = weak.clone();
        let h_drop2 = detail.hash.clone();
        let parent_for_drop = parent.clone();
        detail_col = detail_col.child(
            div()
                .flex()
                .flex_row()
                .gap(SP_1)
                .px(SP_3)
                .pb(SP_1_5)
                .child(
                    Button::ghost("git-ctx-undo", t!("App.Git.ctx_undo_commit"))
                        .size(ButtonSize::Sm)
                        .on_click(move |_, _, cx| {
                            let h = h_undo.clone();
                            let _ = weak_undo.update(cx, |app, cx| {
                                app.schedule_git_action(GitPendingAction::UndoCommit { hash: h }, cx);
                            });
                        }),
                )
                .child(
                    Button::danger("git-ctx-drop", t!("App.Git.ctx_drop_commit"))
                        .size(ButtonSize::Sm)
                        .on_click(move |_, _, cx| {
                            let h = h_drop2.clone();
                            let p = parent_for_drop.clone();
                            let _ = weak_drop2.update(cx, |app, cx| {
                                app.schedule_git_action(
                                    GitPendingAction::DropCommit { hash: h, parent: p },
                                    cx,
                                );
                            });
                        }),
                ),
        );
    }
    detail_col
}

// ─── Tab: Managers ───────────────────────────────────────────────────

fn managers_tab_body(
    t: &crate::theme::Theme,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    let mut col = div().w_full().flex().flex_col();
    col = col.child(managers_tab_strip(snap.manager_tab, weak.clone()));
    col = col.child(Separator::horizontal());

    if snap.managers.loading && snap.managers.branches.is_empty() {
        col = col.child(
            div()
                .px(SP_3)
                .py(SP_3)
                .child(text::caption(t!("App.Common.Status.loading")).secondary()),
        );
        return col;
    }
    if let Some(err) = snap.managers.error.clone() {
        col = col.child(git_feedback_strip(
            t,
            IconName::TriangleAlert,
            t!("App.Common.error"),
            err,
            true,
        ));
    }

    let body = match snap.manager_tab {
        ManagerTab::Branches => branches_manager(t, &snap.managers.branches, weak.clone())
            .into_any_element(),
        ManagerTab::Tags => tags_manager(t, &snap.managers.tags, weak.clone()).into_any_element(),
        ManagerTab::Remotes => {
            remotes_manager(t, &snap.managers.remotes, weak.clone()).into_any_element()
        }
        ManagerTab::Config => {
            config_manager(t, &snap.managers.config, &snap.managers, weak.clone())
                .into_any_element()
        }
        ManagerTab::Submodules => {
            submodules_manager(t, &snap.managers.submodules, weak.clone()).into_any_element()
        }
        ManagerTab::Rebase => rebase_manager(t, weak.clone()).into_any_element(),
        ManagerTab::Conflicts => {
            conflicts_manager(t, &snap.managers.conflicts, weak.clone()).into_any_element()
        }
    };
    col = col.child(body);
    col
}

fn managers_tab_strip(active: ManagerTab, weak: WeakEntity<PierApp>) -> impl IntoElement {
    let mut tabs = Tabs::new().segmented();
    for tab in ManagerTab::all() {
        let is_active = tab == active;
        let (label_key, icon) = match tab {
            ManagerTab::Branches => ("App.Git.mgr_branches", IconName::GitBranch),
            ManagerTab::Tags => ("App.Git.mgr_tags", IconName::BookOpen),
            ManagerTab::Remotes => ("App.Git.mgr_remotes", IconName::Globe),
            ManagerTab::Config => ("App.Git.mgr_config", IconName::Settings),
            ManagerTab::Submodules => ("App.Git.mgr_submodules", IconName::Container),
            ManagerTab::Rebase => ("App.Git.mgr_rebase", IconName::Undo),
            ManagerTab::Conflicts => ("App.Git.mgr_conflicts", IconName::TriangleAlert),
        };
        let w = weak.clone();
        let item = TabItem::new(
            ElementId::Name(format!("git-mgr-{}", tab.id_token()).into()),
            t!(label_key),
            is_active,
            move |_, _, cx| {
                let _ = w.update(cx, |app, cx| app.set_git_manager_tab(tab, cx));
            },
        )
        .with_icon(icon);
        tabs = tabs.item(item);
    }
    tabs
}

// Shared inline-input helper — creates a read-only placeholder display
// for now. The form-style inline editing is delegated to the user
// via existing commit_input / stash_input patterns (adapted later).
fn text_input_line(
    t: &crate::theme::Theme,
    value: impl Into<SharedString>,
) -> impl IntoElement {
    div()
        .flex_1()
        .min_w(px(0.0))
        .h(BUTTON_SM_H)
        .px(SP_2)
        .flex()
        .items_center()
        .rounded(RADIUS_SM)
        .bg(t.color.bg_panel)
        .border_1()
        .border_color(t.color.border_subtle)
        .text_size(SIZE_SMALL)
        .text_color(t.color.text_tertiary)
        .child(value.into())
}

// ─── Managers: Branches ────────────────────────────────────────────

fn branches_manager(
    t: &crate::theme::Theme,
    branches: &[BranchEntry],
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let mut col = div().w_full().flex().flex_col();
    if branches.is_empty() {
        return col
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .child(text::caption(t!("App.Git.working_tree_clean")).secondary()),
            )
            .into_any_element();
    }

    col = col.child(
        div()
            .px(SP_3)
            .py(SP_1)
            .text_size(SIZE_CAPTION)
            .text_color(t.color.text_tertiary)
            .child(SharedString::from(
                t!("App.Git.entries_count", count = branches.len()).to_string(),
            )),
    );

    for b in branches.iter().take(200) {
        col = col.child(branch_mgr_row(t, b, weak.clone()));
    }
    col.into_any_element()
}

fn branch_mgr_row(
    t: &crate::theme::Theme,
    b: &BranchEntry,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let name = b.name.clone();
    let name_id = short_id(&name);
    let checkout_weak = weak.clone();
    let delete_weak = weak.clone();
    let force_weak = weak.clone();
    let merge_weak = weak.clone();
    let rebase_weak = weak.clone();

    let n_checkout = name.clone();
    let n_delete = name.clone();
    let n_force = name.clone();
    let n_merge = name.clone();
    let n_rebase = name.clone();

    let kind_pill: SharedString = if b.is_current {
        t!("App.Git.branch_current").into()
    } else if b.is_remote {
        t!("App.Git.branch_remote").into()
    } else {
        t!("App.Git.branch_local").into()
    };

    div()
        .id(ElementId::Name(
            format!("git-mgr-branch-{name_id}").into(),
        ))
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .px(SP_3)
        .py(SP_0_5)
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex_none()
                .text_size(SIZE_CAPTION)
                .px(SP_1)
                .py(SP_0_5)
                .rounded(RADIUS_PILL)
                .bg(if b.is_current {
                    t.color.accent_subtle
                } else {
                    t.color.bg_panel
                })
                .text_color(if b.is_current {
                    t.color.accent
                } else {
                    t.color.text_tertiary
                })
                .child(kind_pill),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_primary)
                .child(SharedString::from(b.name.clone())),
        )
        .child(
            div()
                .flex_none()
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(b.tracking.clone())),
        )
        .child(
            Button::secondary(
                ElementId::Name(format!("git-mgr-checkout-{name_id}").into()),
                t!("App.Git.checkout"),
            )
            .size(ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                let name = n_checkout.clone();
                let _ = checkout_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::CheckoutBranch { name }, cx);
                });
            }),
        )
        .child(
            Button::secondary(
                ElementId::Name(format!("git-mgr-merge-{name_id}").into()),
                t!("App.Git.branch_merge"),
            )
            .size(ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                let branch = n_merge.clone();
                let _ = merge_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::Merge { branch }, cx);
                });
            }),
        )
        .child(
            Button::secondary(
                ElementId::Name(format!("git-mgr-rebase-{name_id}").into()),
                t!("App.Git.branch_rebase_onto"),
            )
            .size(ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                let onto = n_rebase.clone();
                let _ = rebase_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::Rebase { onto }, cx);
                });
            }),
        )
        .child(
            Button::secondary(
                ElementId::Name(format!("git-mgr-delete-{name_id}").into()),
                t!("App.Git.branch_delete"),
            )
            .size(ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                let name = n_delete.clone();
                let _ = delete_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::BranchDelete { name, force: false }, cx);
                });
            }),
        )
        .child(
            Button::danger(
                ElementId::Name(format!("git-mgr-force-delete-{name_id}").into()),
                t!("App.Git.branch_force_delete"),
            )
            .size(ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                let name = n_force.clone();
                let _ = force_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::BranchDelete { name, force: true }, cx);
                });
            }),
        )
}

// ─── Managers: Tags ────────────────────────────────────────────────

fn tags_manager(
    t: &crate::theme::Theme,
    tags: &[TagInfo],
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let mut col = div().w_full().flex().flex_col();
    if tags.is_empty() {
        return col
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .child(text::caption(t!("App.Git.no_tags")).secondary()),
            )
            .into_any_element();
    }
    for tag in tags.iter().take(200) {
        col = col.child(tag_row(t, tag, weak.clone()));
    }
    col.into_any_element()
}

fn tag_row(
    t: &crate::theme::Theme,
    tag: &TagInfo,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let name = tag.name.clone();
    let id = short_id(&name);

    let push_weak = weak.clone();
    let del_weak = weak.clone();
    let n_push = name.clone();
    let n_del = name.clone();

    div()
        .id(ElementId::Name(format!("git-mgr-tag-{id}").into()))
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .px(SP_3)
        .py(SP_0_5)
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex_none()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.accent)
                .child(SharedString::from(tag.hash.clone())),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_primary)
                .child(SharedString::from(tag.name.clone())),
        )
        .child(
            div()
                .flex_none()
                .truncate()
                .max_w(px(280.0))
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(tag.message.clone())),
        )
        .child(
            Button::secondary(
                ElementId::Name(format!("git-mgr-tag-push-{id}").into()),
                t!("App.Git.tag_push"),
            )
            .size(ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                let name = n_push.clone();
                let _ = push_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::TagPush { name }, cx);
                });
            }),
        )
        .child(
            Button::danger(
                ElementId::Name(format!("git-mgr-tag-del-{id}").into()),
                t!("App.Git.tag_delete"),
            )
            .size(ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                let name = n_del.clone();
                let _ = del_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::TagDelete { name }, cx);
                });
            }),
        )
}

// ─── Managers: Remotes ─────────────────────────────────────────────

fn remotes_manager(
    t: &crate::theme::Theme,
    remotes: &[RemoteInfo],
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let mut col = div().w_full().flex().flex_col();

    let fetch_all_weak = weak.clone();
    col = col.child(
        div()
            .flex()
            .flex_row()
            .gap(SP_1)
            .px(SP_3)
            .py(SP_1_5)
            .child(
                Button::secondary("git-remote-fetch-all", t!("App.Git.remote_fetch_all"))
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        let _ = fetch_all_weak.update(cx, |app, cx| {
                            app.schedule_git_action(
                                GitPendingAction::RemoteFetch { name: None },
                                cx,
                            );
                        });
                    }),
            ),
    );

    if remotes.is_empty() {
        return col
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .child(text::caption(t!("App.Git.no_remotes")).secondary()),
            )
            .into_any_element();
    }
    for r in remotes.iter().take(64) {
        col = col.child(remote_row(t, r, weak.clone()));
    }
    col.into_any_element()
}

fn remote_row(
    t: &crate::theme::Theme,
    r: &RemoteInfo,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let name = r.name.clone();
    let id = short_id(&name);

    let fetch_weak = weak.clone();
    let remove_weak = weak.clone();
    let n_fetch = name.clone();
    let n_remove = name.clone();

    div()
        .id(ElementId::Name(format!("git-mgr-remote-{id}").into()))
        .flex()
        .flex_col()
        .gap(SP_0_5)
        .px(SP_3)
        .py(SP_1)
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .child(
                    div()
                        .flex_none()
                        .text_size(SIZE_SMALL)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_primary)
                        .child(SharedString::from(r.name.clone())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(SIZE_CAPTION)
                        .font_family(t.font_mono.clone())
                        .text_color(t.color.text_secondary)
                        .child(SharedString::from(r.fetch_url.clone())),
                )
                .child(
                    Button::secondary(
                        ElementId::Name(format!("git-mgr-remote-fetch-{id}").into()),
                        t!("App.Git.remote_fetch"),
                    )
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        let name = n_fetch.clone();
                        let _ = fetch_weak.update(cx, |app, cx| {
                            app.schedule_git_action(
                                GitPendingAction::RemoteFetch { name: Some(name) },
                                cx,
                            );
                        });
                    }),
                )
                .child(
                    Button::danger(
                        ElementId::Name(format!("git-mgr-remote-remove-{id}").into()),
                        t!("App.Git.remote_remove"),
                    )
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        let name = n_remove.clone();
                        let _ = remove_weak.update(cx, |app, cx| {
                            app.schedule_git_action(
                                GitPendingAction::RemoteRemove { name },
                                cx,
                            );
                        });
                    }),
                ),
        )
}

// ─── Managers: Config ──────────────────────────────────────────────

fn config_manager(
    t: &crate::theme::Theme,
    entries: &[ConfigEntry],
    mgrs: &ManagersSnapshot,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let mut col = div().w_full().flex().flex_col();

    // User identity strip (top).
    col = col.child(
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(SP_2)
            .px(SP_3)
            .py(SP_1_5)
            .child(
                div()
                    .flex_none()
                    .text_size(SIZE_CAPTION)
                    .text_color(t.color.text_tertiary)
                    .child(SharedString::from("user.name".to_string())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(SIZE_SMALL)
                    .font_family(t.font_mono.clone())
                    .text_color(t.color.text_primary)
                    .child(SharedString::from(mgrs.user_name.clone())),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(SIZE_CAPTION)
                    .text_color(t.color.text_tertiary)
                    .child(SharedString::from("user.email".to_string())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(SIZE_SMALL)
                    .font_family(t.font_mono.clone())
                    .text_color(t.color.text_primary)
                    .child(SharedString::from(mgrs.user_email.clone())),
            ),
    );

    if entries.is_empty() {
        return col
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .child(text::caption(t!("App.Git.no_config")).secondary()),
            )
            .into_any_element();
    }
    for (i, e) in entries.iter().enumerate().take(200) {
        col = col.child(config_row(t, i, e, weak.clone()));
    }
    col.into_any_element()
}

fn config_row(
    t: &crate::theme::Theme,
    i: usize,
    e: &ConfigEntry,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let key = e.key.clone();
    let is_global = e.scope == "global";
    let unset_weak = weak.clone();
    let k_for_click = key.clone();

    div()
        .id(("git-cfg", i))
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .px(SP_3)
        .py(SP_0_5)
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex_none()
                .text_size(SIZE_CAPTION)
                .px(SP_1)
                .py(SP_0_5)
                .rounded(RADIUS_PILL)
                .bg(t.color.bg_panel)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(e.scope.clone())),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_primary)
                .child(SharedString::from(e.key.clone())),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_secondary)
                .child(SharedString::from(e.value.clone())),
        )
        .child(
            Button::danger(("git-cfg-unset", i), t!("App.Git.config_unset"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let key = k_for_click.clone();
                    let _ = unset_weak.update(cx, |app, cx| {
                        app.schedule_git_action(
                            GitPendingAction::ConfigUnset {
                                key,
                                global: is_global,
                            },
                            cx,
                        );
                    });
                }),
        )
}

// ─── Managers: Submodules ──────────────────────────────────────────

fn submodules_manager(
    t: &crate::theme::Theme,
    subs: &[SubmoduleInfo],
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let mut col = div().w_full().flex().flex_col();
    let update_weak = weak.clone();
    col = col.child(
        div()
            .flex()
            .flex_row()
            .gap(SP_1)
            .px(SP_3)
            .py(SP_1_5)
            .child(
                Button::secondary("git-sub-update", t!("App.Git.submodule_update"))
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        let _ = update_weak.update(cx, |app, cx| {
                            app.schedule_git_action(GitPendingAction::SubmoduleUpdate, cx);
                        });
                    }),
            ),
    );
    if subs.is_empty() {
        return col
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .child(text::caption(t!("App.Git.no_submodules")).secondary()),
            )
            .into_any_element();
    }
    for (i, s) in subs.iter().enumerate().take(64) {
        col = col.child(submodule_row(t, i, s, weak.clone()));
    }
    col.into_any_element()
}

fn submodule_row(
    t: &crate::theme::Theme,
    i: usize,
    s: &SubmoduleInfo,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let path = s.path.clone();
    let p_remove = path.clone();
    let rm_weak = weak.clone();
    div()
        .id(("git-sub", i))
        .flex()
        .flex_col()
        .gap(SP_0_5)
        .px(SP_3)
        .py(SP_1)
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .child(
                    div()
                        .flex_none()
                        .text_size(SIZE_SMALL)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_primary)
                        .child(SharedString::from(s.path.clone())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(SIZE_CAPTION)
                        .font_family(t.font_mono.clone())
                        .text_color(t.color.text_secondary)
                        .child(SharedString::from(s.url.clone())),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(SIZE_MONO_SMALL)
                        .font_family(t.font_mono.clone())
                        .text_color(t.color.accent)
                        .child(SharedString::from(format!(
                            "{} {}",
                            &s.hash.chars().take(8).collect::<String>(),
                            s.described
                        ))),
                )
                .child(
                    Button::danger(("git-sub-rm", i), t!("App.Git.submodule_remove"))
                        .size(ButtonSize::Sm)
                        .on_click(move |_, _, cx| {
                            let path = p_remove.clone();
                            let _ = rm_weak.update(cx, |app, cx| {
                                app.schedule_git_action(
                                    GitPendingAction::SubmoduleRemove { path },
                                    cx,
                                );
                            });
                        }),
                ),
        )
}

// ─── Managers: Rebase ──────────────────────────────────────────────

fn rebase_manager(t: &crate::theme::Theme, weak: WeakEntity<PierApp>) -> impl IntoElement {
    let cont_weak = weak.clone();
    let abort_weak = weak.clone();
    let skip_weak = weak.clone();
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_2)
        .child(text::caption(t!("App.Git.rebase_body")).secondary())
        .child(
            div()
                .flex()
                .flex_row()
                .gap(SP_1)
                .child(
                    Button::primary("git-rebase-continue", t!("App.Git.rebase_continue"))
                        .size(ButtonSize::Sm)
                        .on_click(move |_, _, cx| {
                            let _ = cont_weak.update(cx, |app, cx| {
                                app.schedule_git_action(GitPendingAction::RebaseContinue, cx);
                            });
                        }),
                )
                .child(
                    Button::secondary("git-rebase-skip", t!("App.Git.rebase_skip"))
                        .size(ButtonSize::Sm)
                        .on_click(move |_, _, cx| {
                            let _ = skip_weak.update(cx, |app, cx| {
                                app.schedule_git_action(GitPendingAction::RebaseSkip, cx);
                            });
                        }),
                )
                .child(
                    Button::danger("git-rebase-abort", t!("App.Git.rebase_abort"))
                        .size(ButtonSize::Sm)
                        .on_click(move |_, _, cx| {
                            let _ = abort_weak.update(cx, |app, cx| {
                                app.schedule_git_action(GitPendingAction::RebaseAbort, cx);
                            });
                        }),
                ),
        )
        .child(text_input_line(
            t,
            t!("App.Git.rebase_onto_placeholder"),
        ))
}

// ─── Managers: Conflicts ───────────────────────────────────────────

fn conflicts_manager(
    t: &crate::theme::Theme,
    conflicts: &[String],
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let mut col = div().w_full().flex().flex_col();

    let abort_weak = weak.clone();
    col = col.child(
        div()
            .flex()
            .flex_row()
            .gap(SP_1)
            .px(SP_3)
            .py(SP_1_5)
            .child(
                Button::danger("git-merge-abort", t!("App.Git.merge_abort"))
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _, cx| {
                        let _ = abort_weak.update(cx, |app, cx| {
                            app.schedule_git_action(GitPendingAction::MergeAbort, cx);
                        });
                    }),
            ),
    );

    if conflicts.is_empty() {
        return col
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .child(text::caption(t!("App.Git.conflicts_empty")).secondary()),
            )
            .into_any_element();
    }
    col = col.child(
        div()
            .px(SP_3)
            .py(SP_0_5)
            .text_size(SIZE_CAPTION)
            .text_color(t.color.text_tertiary)
            .child(SharedString::from(
                t!(
                    "App.Git.conflicts_count",
                    count = conflicts.len(),
                    suffix = if conflicts.len() == 1 { "" } else { "s" }
                )
                .to_string(),
            )),
    );
    for (i, path) in conflicts.iter().enumerate().take(64) {
        col = col.child(conflict_row(t, i, path, weak.clone()));
    }
    col.into_any_element()
}

fn conflict_row(
    t: &crate::theme::Theme,
    i: usize,
    path: &str,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let p_ours = path.to_string();
    let p_theirs = path.to_string();
    let p_mark = path.to_string();
    let w_ours = weak.clone();
    let w_theirs = weak.clone();
    let w_mark = weak.clone();
    div()
        .id(("git-conflict", i))
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .px(SP_3)
        .py(SP_0_5)
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex_none()
                .w(ICON_MD)
                .h(ICON_MD)
                .rounded(RADIUS_SM)
                .bg(t.color.status_error)
                .text_color(t.color.text_inverse)
                .flex()
                .items_center()
                .justify_center()
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .child(SharedString::from("!".to_string())),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_primary)
                .child(SharedString::from(path.to_string())),
        )
        .child(
            Button::secondary(("git-conflict-ours", i), t!("App.Git.conflicts_resolve_ours"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let path = p_ours.clone();
                    let _ = w_ours.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::ResolveOurs { path }, cx);
                    });
                }),
        )
        .child(
            Button::secondary(
                ("git-conflict-theirs", i),
                t!("App.Git.conflicts_resolve_theirs"),
            )
            .size(ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                let path = p_theirs.clone();
                let _ = w_theirs.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::ResolveTheirs { path }, cx);
                });
            }),
        )
        .child(
            Button::primary(("git-conflict-mark", i), t!("App.Git.conflicts_mark_resolved"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let path = p_mark.clone();
                    let _ = w_mark.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::MarkResolved { path }, cx);
                    });
                }),
        )
}

// ─── Snapshot helper ───────────────────────────────────────────────

#[derive(Clone)]
struct GitSnapshot {
    status: GitStatus,
    cwd: SharedString,
    repo_path: Option<SharedString>,
    branch: Option<BranchInfo>,
    branches: Vec<String>,
    changes: Vec<GitFileChange>,
    log: Vec<CommitInfo>,
    stashes: Vec<StashEntry>,
    pending: Option<GitPendingAction>,
    last_error: Option<SharedString>,
    action_error: Option<SharedString>,
    last_confirmation: Option<SharedString>,
    diff_selection: Option<DiffSelection>,
    diff_output: Option<SharedString>,
    diff_loading: bool,
    diff_error: Option<SharedString>,
    diff_mode: DiffMode,
    tab: GitTab,
    manager_tab: ManagerTab,
    graph: GraphStateSnapshot,
    commit_detail: CommitDetailSnapshot,
    managers: ManagersSnapshot,
}

#[derive(Clone)]
struct GraphStateSnapshot {
    rows: Vec<GraphRow>,
    unpushed: std::collections::HashSet<String>,
    filter: GraphFilter,
    has_more: bool,
    loading: bool,
    error: Option<SharedString>,
    selected: Option<String>,
    show_hash_col: bool,
    show_author_col: bool,
    show_date_col: bool,
    zebra_stripes: bool,
    highlight_mode: GraphHighlightMode,
    user_name: String,
}

#[derive(Clone)]
struct CommitDetailSnapshot {
    detail: Option<CommitDetail>,
    loading: bool,
    error: Option<SharedString>,
}

#[derive(Clone)]
struct ManagersSnapshot {
    branches: Vec<BranchEntry>,
    tags: Vec<TagInfo>,
    remotes: Vec<RemoteInfo>,
    config: Vec<ConfigEntry>,
    submodules: Vec<SubmoduleInfo>,
    conflicts: Vec<String>,
    user_name: String,
    user_email: String,
    loading: bool,
    error: Option<SharedString>,
}

impl From<&GitState> for GitSnapshot {
    fn from(state: &GitState) -> Self {
        Self {
            status: state.status.clone(),
            cwd: state.cwd.display().to_string().into(),
            repo_path: state
                .repo_path
                .as_ref()
                .map(|p| SharedString::from(p.display().to_string())),
            branch: state.branch.clone(),
            branches: state.branches.clone(),
            changes: state.changes.clone(),
            log: state.log.clone(),
            stashes: state.stashes.clone(),
            pending: state.pending.clone(),
            last_error: state.last_error.clone(),
            action_error: state.action_error.clone(),
            last_confirmation: state.last_confirmation.clone(),
            diff_selection: state.diff_selection.clone(),
            diff_output: state.diff_output.clone(),
            diff_loading: state.diff_loading,
            diff_error: state.diff_error.clone(),
            diff_mode: state.diff_mode,
            tab: state.tab,
            manager_tab: state.manager_tab,
            graph: GraphStateSnapshot {
                rows: state.graph.rows.clone(),
                unpushed: state.graph.unpushed.clone(),
                filter: state.graph.filter.clone(),
                has_more: state.graph.has_more,
                loading: state.graph.loading,
                error: state.graph.error.clone(),
                selected: state.graph.selected.clone(),
                show_hash_col: state.graph.show_hash_col,
                show_author_col: state.graph.show_author_col,
                show_date_col: state.graph.show_date_col,
                zebra_stripes: state.graph.zebra_stripes,
                highlight_mode: state.graph.highlight_mode,
                user_name: state.managers.user_name.clone(),
            },
            commit_detail: CommitDetailSnapshot {
                detail: state.commit_detail.detail.clone(),
                loading: state.commit_detail.loading,
                error: state.commit_detail.error.clone(),
            },
            managers: ManagersSnapshot {
                branches: state.managers.branches.clone(),
                tags: state.managers.tags.clone(),
                remotes: state.managers.remotes.clone(),
                config: state.managers.config.clone(),
                submodules: state.managers.submodules.clone(),
                conflicts: state.managers.conflicts.clone(),
                user_name: state.managers.user_name.clone(),
                user_email: state.managers.user_email.clone(),
                loading: state.managers.loading,
                error: state.managers.error.clone(),
            },
        }
    }
}

