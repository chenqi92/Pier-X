//! Interactive Git panel — reads from the cached [`GitState`] owned by
//! `PierApp`, dispatches user clicks to `schedule_git_action`.
//!
//! The view is `RenderOnce` — rebuilt every frame — so every render
//! snapshots `GitState` up-front, drops the borrow, and closes over
//! the `WeakEntity<PierApp>` for the action callbacks. No IO happens
//! in `render`; the initial `git status / log / branches / stashes`
//! round-trip is scheduled at the top of `render` via
//! `PierApp::schedule_git_initial_refresh`, which is flag-guarded.

use gpui::{
    div, prelude::*, px, App, ElementId, IntoElement, SharedString, WeakEntity, Window,
};
use gpui_component::input::{Input, InputState};
use pier_core::services::git::{
    BranchInfo, CommitInfo, FileStatus, GitFileChange, StashEntry,
};
use rust_i18n::t;

use crate::app::git_session::{DiffSelection, GitPendingAction, GitState, GitStatus};
use crate::app::PierApp;
use crate::components::{text, Button, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    heights::{BUTTON_SM_H, ICON_MD},
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3, SP_4},
    theme,
    typography::{SIZE_BODY, SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};
use gpui_component::IconName;

/// Maximum number of file-change rows rendered before collapsing
/// into a "+N more" label. 50 keeps the element tree bounded while
/// showing more than the old 20-row cap.
const MAX_CHANGE_ROWS: usize = 50;

/// Log row cap — backend already fetches at `log_limit` (30).
const MAX_LOG_ROWS: usize = 30;

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

        // Dead-panel fallback: if PierApp is gone we can't read state.
        let Some(app_entity) = self.app.upgrade() else {
            return dead_panel(&t).into_any_element();
        };

        // Flag-guarded initial probe — first time the Git mode is
        // rendered after app start, schedule `git status / log /
        // branches / stashes`. Subsequent renders hit the cache.
        app_entity.update(cx, |app, cx| {
            app.schedule_git_initial_refresh(cx);
        });

        // Snapshot the state + inputs up-front so child elements
        // can close over owned data without holding `cx` borrows.
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
            _ => repo_layout(&t, snapshot, commit_input, stash_input, weak)
                .into_any_element(),
        }
    }
}

// ─── Layout helpers ──────────────────────────────────────────────────

fn header(t: &crate::theme::Theme, snap: &GitSnapshot, weak: WeakEntity<PierApp>) -> impl IntoElement {
    let pending_label = snap.pending.as_ref().map(|action| action.label());
    let refresh_busy = snap.pending.is_some();
    let weak_refresh = weak.clone();

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_3)
        .child(text::h2(t!("App.Git.title")))
        .child(status_pill(snap));

    if let Some(label) = pending_label {
        row = row.child(StatusPill::new(
            t!("App.Git.pending", action = label.as_ref()),
            StatusKind::Info,
        ));
    }

    let refresh_btn: gpui::AnyElement = if refresh_busy {
        StatusPill::new(t!("App.Git.busy"), StatusKind::Warning).into_any_element()
    } else {
        Button::secondary("git-refresh", t!("App.Common.refresh"))
            .on_click(move |_, _, cx| {
                let _ = weak_refresh.update(cx, |app, cx| app.schedule_git_refresh(cx));
            })
            .into_any_element()
    };

    // Push refresh to the right.
    row.child(div().flex_1()).child(refresh_btn).text_color(t.color.text_primary)
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

fn repo_layout(
    t: &crate::theme::Theme,
    snap: GitSnapshot,
    commit_input: gpui::Entity<InputState>,
    stash_input: gpui::Entity<InputState>,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    let mut col = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(SP_4)
        .p(SP_4)
        .child(header(t, &snap, weak.clone()));

    if let Some(msg) = snap.last_confirmation.clone() {
        col = col.child(confirmation_card(t, msg));
    }
    if let Some(err) = snap.action_error.clone() {
        col = col.child(error_card(t, err));
    }
    if let Some(err) = snap.last_error.clone() {
        col = col.child(error_card(t, err));
    }

    if let Some(branch) = snap.branch.clone() {
        col = col.child(branch_card(t, &branch, &snap.repo_path, &snap.branches, weak.clone()));
    }
    col = col.child(changes_card(
        t,
        &snap.changes,
        snap.diff_selection.as_ref(),
        weak.clone(),
    ));
    if snap.diff_selection.is_some() {
        col = col.child(diff_card(t, &snap, weak.clone()));
    }
    col = col.child(commit_card(t, &snap.changes, commit_input, weak.clone()));
    col = col.child(stash_card(t, &snap.stashes, stash_input, weak.clone()));
    col = col.child(log_card(t, &snap.log));
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
        .gap(SP_4)
        .p(SP_4)
        .child(header(t, snap, weak))
        .child(
            Card::new()
                .child(
                    SectionLabel::new(t!("App.Common.repository"))
                        .with_icon(IconName::Folder),
                )
                .child(text::body(t!("App.Git.not_a_repo_body")).secondary())
                .child(div().overflow_hidden().child(text::mono(snap.cwd.clone()))),
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
        .gap(SP_4)
        .p(SP_4)
        .child(header(t, snap, weak))
        .child(error_card(t, snap.last_error.clone().unwrap_or_default()))
}

fn dead_panel(t: &crate::theme::Theme) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(SP_4)
        .p(SP_4)
        .child(text::h2(t!("App.Git.title")))
        .child(text::body(t!("App.Common.panel_lost")).secondary())
        .text_color(t.color.text_primary)
}

// ─── Card: current branch + switcher ────────────────────────────────

fn branch_card(
    t: &crate::theme::Theme,
    branch: &BranchInfo,
    repo_path: &Option<SharedString>,
    branches: &[String],
    weak: WeakEntity<PierApp>,
) -> Card {
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

    let mut card = Card::new()
        .child(
            SectionLabel::new(t!("App.Git.current_branch"))
                .with_icon(IconName::GitBranch),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
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
                            t!("App.Git.branch_tracking", tracking = tracking.as_ref())
                                .to_string(),
                        )),
                ),
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
                        .child(pace),
                )
                .child(div().flex_1())
                .child(
                    Button::secondary("git-pull", t!("App.Git.pull"))
                        .on_click(move |_, _, cx| {
                            let _ = pull_weak.update(cx, |app, cx| {
                                app.schedule_git_action(GitPendingAction::Pull, cx);
                            });
                        }),
                )
                .child(
                    Button::secondary("git-push", t!("App.Git.push"))
                        .on_click(move |_, _, cx| {
                            let _ = push_weak.update(cx, |app, cx| {
                                app.schedule_git_action(GitPendingAction::Push, cx);
                            });
                        }),
                ),
        );

    if let Some(path) = repo_path {
        card = card.child(
            div()
                .overflow_hidden()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_tertiary)
                .truncate()
                .child(SharedString::from(
                    t!("App.Git.repo_path", path = path.as_ref()).to_string(),
                )),
        );
    }

    // Branch switcher: list other branches, excluding the current.
    let others: Vec<String> = branches
        .iter()
        .filter(|b| b.as_str() != branch.name)
        .cloned()
        .collect();

    if !others.is_empty() {
        card = card.child(div().pt(SP_2).child(
            SectionLabel::new(t!("App.Git.switch_branch")).with_icon(IconName::Map),
        ));
        for name in others.into_iter().take(24) {
            card = card.child(branch_row(t, name, weak.clone()));
        }
    }

    card
}

fn branch_row(t: &crate::theme::Theme, name: String, weak: WeakEntity<PierApp>) -> impl IntoElement {
    let name_for_click = name.clone();
    let id = ElementId::Name(SharedString::from(format!("git-checkout-{name}")));
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .py(SP_1)
        .overflow_hidden()
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
                Button::secondary(id, t!("App.Git.checkout")).on_click(move |_, _, cx| {
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

// ─── Card: working-tree changes + per-file actions ──────────────────

fn changes_card(
    t: &crate::theme::Theme,
    changes: &[GitFileChange],
    diff_selection: Option<&crate::app::git_session::DiffSelection>,
    weak: WeakEntity<PierApp>,
) -> Card {
    let staged = changes.iter().filter(|c| c.staged).count();
    let unstaged = changes.len().saturating_sub(staged);

    let stage_all_weak = weak.clone();
    let unstage_all_weak = weak.clone();

    let mut header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .child(
            SectionLabel::new(t!("App.Git.working_tree")).with_icon(IconName::Inbox),
        )
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
        ))
        .child(div().flex_1());

    if unstaged > 0 {
        header = header.child(
            Button::secondary("git-stage-all", t!("App.Git.stage_all")).on_click(move |_, _, cx| {
                let _ = stage_all_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::StageAll, cx);
                });
            }),
        );
    }
    if staged > 0 {
        header = header.child(
            Button::secondary("git-unstage-all", t!("App.Git.unstage_all")).on_click(move |_, _, cx| {
                let _ = unstage_all_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::UnstageAll, cx);
                });
            }),
        );
    }

    let mut card = Card::new().child(header);

    if changes.is_empty() {
        card = card.child(text::body(t!("App.Git.working_tree_clean")).secondary());
        return card;
    }

    for change in changes.iter().take(MAX_CHANGE_ROWS) {
        let is_selected = diff_selection
            .map(|sel| sel.path == change.path && sel.staged == change.staged)
            .unwrap_or(false);
        card = card.child(file_change_row(t, change, is_selected, weak.clone()));
    }
    if changes.len() > MAX_CHANGE_ROWS {
        card = card.child(
            div()
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
    card
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

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .py(SP_1)
        .px(SP_1)
        .rounded(RADIUS_SM)
        .overflow_hidden()
        .when(is_selected, |el| el.bg(t.color.bg_panel))
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

    // Per-file actions.
    if staged {
        row = row.child(div().flex_none().child(
            Button::secondary(unstage_id, t!("App.Git.unstage")).on_click(move |_, _, cx| {
                let p = path_for_unstage.clone();
                let _ = unstage_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::Unstage { path: p }, cx);
                });
            }),
        ));
    } else {
        row = row.child(div().flex_none().child(
            Button::secondary(stage_id, t!("App.Git.stage")).on_click(move |_, _, cx| {
                let p = path_for_stage.clone();
                let _ = stage_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::Stage { path: p }, cx);
                });
            }),
        ));
        row = row.child(div().flex_none().child(
            Button::danger(discard_id, t!("App.Git.discard")).on_click(move |_, _, cx| {
                let p = path_for_discard.clone();
                let _ = discard_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::Discard { path: p }, cx);
                });
            }),
        ));
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

// ─── Card: commit staged changes ────────────────────────────────────

fn commit_card(
    t: &crate::theme::Theme,
    changes: &[GitFileChange],
    input: gpui::Entity<InputState>,
    weak: WeakEntity<PierApp>,
) -> Card {
    let has_staged = changes.iter().any(|c| c.staged);
    let commit_weak = weak.clone();
    let input_for_click = input.clone();

    let button: gpui::AnyElement = if has_staged {
        Button::primary("git-commit", t!("App.Git.commit_staged"))
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

    Card::new()
        .child(
            SectionLabel::new(t!("App.Git.commit_section")).with_icon(IconName::GitCommit),
        )
        .child(Input::new(&input))
        .child(
            div()
                .pt(SP_2)
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .child(
                    div()
                        .flex_1()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(SharedString::from(t!("App.Git.commit_hint").to_string())),
                )
                .child(button),
        )
}

// ─── Card: stash list + stash push ──────────────────────────────────

fn stash_card(
    t: &crate::theme::Theme,
    stashes: &[StashEntry],
    input: gpui::Entity<InputState>,
    weak: WeakEntity<PierApp>,
) -> Card {
    let push_weak = weak.clone();
    let input_for_click = input.clone();

    let mut card = Card::new()
        .child(
            SectionLabel::new(t!("App.Git.stash_section")).with_icon(IconName::Inspector),
        )
        .child(Input::new(&input))
        .child(
            div()
                .pt(SP_2)
                .flex()
                .flex_row()
                .gap(SP_2)
                .child(div().flex_1())
                .child(
                    Button::secondary("git-stash-push", t!("App.Git.stash_push"))
                        .on_click(move |_, _, cx| {
                            let text: String = input_for_click.read(cx).value().to_string();
                            let _ = push_weak.update(cx, |app, cx| {
                                app.schedule_git_action(
                                    GitPendingAction::StashPush { message: text },
                                    cx,
                                );
                            });
                        }),
                ),
        );

    if stashes.is_empty() {
        card = card.child(text::body(t!("App.Git.no_stashes")).secondary());
        return card;
    }
    for stash in stashes.iter().take(10) {
        card = card.child(stash_row(t, stash, weak.clone()));
    }
    if stashes.len() > 10 {
        card = card.child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(
                    t!(
                        "App.Git.more_stashes",
                        count = stashes.len() - 10
                    )
                    .to_string(),
                )),
        );
    }
    card
}

fn stash_row(t: &crate::theme::Theme, stash: &StashEntry, weak: WeakEntity<PierApp>) -> impl IntoElement {
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

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .py(SP_1)
        .overflow_hidden()
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
        .child(div().flex_none().child(
            Button::secondary(apply_id, t!("App.Git.apply")).on_click(move |_, _, cx| {
                let id = idx_apply.clone();
                let _ = apply_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::StashApply { index: id }, cx);
                });
            }),
        ))
        .child(div().flex_none().child(
            Button::secondary(pop_id, t!("App.Git.pop")).on_click(move |_, _, cx| {
                let id = idx_pop.clone();
                let _ = pop_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::StashPop { index: id }, cx);
                });
            }),
        ))
        .child(div().flex_none().child(
            Button::danger(drop_id, t!("App.Git.drop")).on_click(move |_, _, cx| {
                let id = idx_drop.clone();
                let _ = drop_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::StashDrop { index: id }, cx);
                });
            }),
        ))
}

// ─── Card: unified diff for the selected file ──────────────────────

fn diff_card(
    t: &crate::theme::Theme,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> Card {
    // Safe: caller only invokes `diff_card` when `diff_selection` is
    // `Some` (see `repo_layout`).
    let selection = snap
        .diff_selection
        .as_ref()
        .expect("diff_card called without a selection");

    let close_weak = weak.clone();
    let side_label: SharedString = if selection.untracked {
        t!("App.Git.diff_side_untracked").into()
    } else if selection.staged {
        t!("App.Git.diff_side_staged").into()
    } else {
        t!("App.Git.diff_side_worktree").into()
    };

    let mut card = Card::new()
        .padding(SP_3)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .overflow_hidden()
                .child(
                    SectionLabel::new(t!("App.Git.diff_section"))
                        .with_icon(IconName::Inspector),
                )
                .child(StatusPill::new(side_label, StatusKind::Info))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(SIZE_MONO_SMALL)
                        .font_family(t.font_mono.clone())
                        .text_color(t.color.text_secondary)
                        .child(SharedString::from(selection.path.clone())),
                )
                .child(
                    div().flex_none().child(
                        Button::ghost("git-diff-close", t!("App.Git.close_diff")).on_click(
                            move |_, _, cx| {
                                let _ = close_weak.update(cx, |app, cx| {
                                    app.clear_git_diff_selection(cx);
                                });
                            },
                        ),
                    ),
                ),
        );

    if snap.diff_loading {
        return card.child(text::body(t!("App.Git.diff_loading")).secondary());
    }

    if let Some(err) = snap.diff_error.clone() {
        return card.child(
            div()
                .overflow_hidden()
                .text_size(SIZE_SMALL)
                .text_color(t.color.status_error)
                .child(err),
        );
    }

    let Some(text) = snap.diff_output.clone() else {
        return card.child(text::body(t!("App.Git.diff_empty")).secondary());
    };

    if text.is_empty() {
        return card.child(text::body(t!("App.Git.diff_empty")).secondary());
    }

    // Render the diff one line per row with color coding. Cap the
    // number of lines so a huge diff can't blow the element tree.
    let lines: Vec<&str> = text.lines().take(MAX_DIFF_LINES).collect();
    let total_lines = text.lines().count();

    for (idx, line) in lines.iter().enumerate() {
        card = card.child(diff_line_row(t, idx, line));
    }

    if total_lines > MAX_DIFF_LINES {
        card = card.child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(
                    t!(
                        "App.Git.diff_truncated",
                        shown = MAX_DIFF_LINES,
                        total = total_lines
                    )
                    .to_string(),
                )),
        );
    }

    card
}

/// Maximum number of diff lines rendered before collapsing into a
/// "+N more" label. The worst-case line length is bounded by
/// `MAX_DIFF_LINE_LEN` elsewhere, so 1000 rows keeps the element
/// count predictable.
const MAX_DIFF_LINES: usize = 1000;

fn diff_line_row(t: &crate::theme::Theme, index: usize, line: &str) -> impl IntoElement {
    let color = if line.starts_with("@@") {
        t.color.status_info
    } else if line.starts_with('+') && !line.starts_with("+++") {
        t.color.status_success
    } else if line.starts_with('-') && !line.starts_with("---") {
        t.color.status_error
    } else if line.starts_with("diff --git") || line.starts_with("index ") {
        t.color.text_tertiary
    } else {
        t.color.text_secondary
    };

    div()
        .id(("git-diff-line", index))
        .flex()
        .flex_row()
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

// ─── Card: recent commits (read-only, retained from old view) ───────

fn log_card(t: &crate::theme::Theme, log: &[CommitInfo]) -> Card {
    let mut card = Card::new().padding(SP_3).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .child(
                SectionLabel::new(t!("App.Git.recent_commits"))
                    .with_icon(IconName::GalleryVerticalEnd),
            )
            .child(
                div()
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child(SharedString::from(
                        t!("App.Git.entries_count", count = log.len()).to_string(),
                    )),
            ),
    );
    if log.is_empty() {
        card = card.child(text::body(t!("App.Git.no_commits")).secondary());
        return card;
    }
    for c in log.iter().take(MAX_LOG_ROWS) {
        card = card.child(commit_row(t, c));
    }
    card
}

fn commit_row(t: &crate::theme::Theme, c: &CommitInfo) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .h(BUTTON_SM_H)
        .px(SP_1_5)
        .rounded(RADIUS_SM)
        .overflow_hidden()
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

// ─── Feedback cards (confirmation / error) ──────────────────────────

fn confirmation_card(t: &crate::theme::Theme, msg: SharedString) -> Card {
    Card::new()
        .padding(SP_3)
        .child(
            SectionLabel::new(t!("App.Git.last_result")).with_icon(IconName::Check),
        )
        .child(
            div()
                .overflow_hidden()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_secondary)
                .child(msg),
        )
}

fn error_card(t: &crate::theme::Theme, msg: SharedString) -> Card {
    Card::new()
        .padding(SP_3)
        .child(
            SectionLabel::new(t!("App.Common.error")).with_icon(IconName::TriangleAlert),
        )
        .child(
            div()
                .overflow_hidden()
                .text_size(SIZE_SMALL)
                .text_color(t.color.status_error)
                .child(msg),
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
        }
    }
}

