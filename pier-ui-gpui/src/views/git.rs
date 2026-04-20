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
    div, list, prelude::*, px, App, ClickEvent, ElementId, IntoElement, ListState, MouseButton,
    SharedString, WeakEntity, Window,
};
use gpui_component::input::InputState;
use gpui_component::scroll::ScrollableElement;
use pier_core::git_graph::GraphRow;
use pier_core::services::git::{
    BranchEntry, BranchInfo, CommitDetail, CommitInfo, ConfigEntry, FileStatus, GitFileChange,
    RemoteInfo, ResetMode, StashEntry, SubmoduleInfo, TagInfo,
};
use rust_i18n::t;

use crate::app::git_session::{
    clamp_git_footer_height, CommitActionMode, DiffMode, DiffSelection, GitPendingAction, GitState,
    GitStatus, GitTab, GraphColumn, GraphDateRange, GraphFilter, GraphHighlightMode,
    GraphResizableColumn, ManagerTab,
};
use crate::app::PierApp;
use crate::components::{
    compute_graph_col_width, graph_row_canvas, is_head_row, palette_color, text, Button,
    ButtonSize, CommitComposer, Dropdown, DropdownOption, DropdownSize, IconButton,
    IconButtonSize, IconButtonVariant, InlineInput, InlineInputTone, InspectorSection,
    Separator, SplitButton, SplitButtonOption, StatusKind, StatusPill, TabItem, Tabs,
};
use crate::theme::{
    heights::{BUTTON_SM_H, ICON_MD, ICON_SM},
    radius::{RADIUS_PILL, RADIUS_SM, RADIUS_XS},
    spacing::{SP_0_5, SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_BODY, SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};
use crate::views::git_dialogs;
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

        let (snapshot, commit_input, stash_input, graph_search_input) = {
            let app = app_entity.read(cx);
            let state = app.git_state().read(cx);
            (
                GitSnapshot::from(state),
                app.git_commit_input(),
                app.git_stash_message_input(),
                app.git_graph_search_input(),
            )
        };

        let weak = self.app.clone();

        match &snapshot.status {
            GitStatus::NotARepo => not_a_repo_layout(&t, &snapshot, weak).into_any_element(),
            GitStatus::Failed if snapshot.repo_path.is_none() => {
                error_layout(&t, &snapshot, weak).into_any_element()
            }
            _ => tab_layout(
                &t,
                snapshot,
                commit_input,
                stash_input,
                graph_search_input,
                weak,
                cx,
            )
            .into_any_element(),
        }
    }
}

// ─── Layout roots ────────────────────────────────────────────────────

fn tab_layout(
    t: &crate::theme::Theme,
    snap: GitSnapshot,
    commit_input: gpui::Entity<InputState>,
    stash_input: gpui::Entity<InputState>,
    graph_search_input: gpui::Entity<InputState>,
    weak: WeakEntity<PierApp>,
    cx: &mut App,
) -> gpui::Stateful<gpui::Div> {
    let active = snap.tab;
    // Managers are floating dialogs now (see `git_dialogs.rs`) so
    // the overlay field is kept around only for legacy snapshot
    // serialisation and never drives the body any more.
    let manager_open: Option<ManagerTab> = None;

    // Feedback strips — coloured banners floating above the tab
    // body. Each has its own ✕ dismiss so the user can clear them
    // without waiting for the next action to overwrite the state.
    let confirm_dismiss_weak = weak.clone();
    let confirm_strip = snap.last_confirmation.clone().map(|msg| {
        git_feedback_strip(
            t,
            "git-fb-ok",
            IconName::Check,
            msg,
            false,
            move |_, _, cx| {
                let _ = confirm_dismiss_weak.update(cx, |app, cx| {
                    app.clear_git_last_confirmation(cx);
                });
            },
        )
    });
    let action_err_dismiss_weak = weak.clone();
    let action_err_strip = snap.action_error.clone().map(|err| {
        git_feedback_strip(
            t,
            "git-fb-action-err",
            IconName::WarningFill,
            err,
            true,
            move |_, _, cx| {
                let _ = action_err_dismiss_weak.update(cx, |app, cx| {
                    app.clear_git_action_error(cx);
                });
            },
        )
    });
    let probe_err_dismiss_weak = weak.clone();
    let probe_err_strip = snap.last_error.clone().map(|err| {
        git_feedback_strip(
            t,
            "git-fb-probe-err",
            IconName::WarningFill,
            err,
            true,
            move |_, _, cx| {
                let _ = probe_err_dismiss_weak.update(cx, |app, cx| {
                    app.clear_git_last_error(cx);
                });
            },
        )
    });

    // Body: either the active tab or the manager overlay.
    let body: gpui::AnyElement = if let Some(panel) = manager_open {
        manager_overlay(t, panel, &snap, weak.clone()).into_any_element()
    } else {
        match active {
            GitTab::Changes => changes_tab_body(t, &snap, weak.clone()).into_any_element(),
            GitTab::History => graph_tab_body(t, &snap, graph_search_input.clone(), weak.clone())
                .into_any_element(),
            GitTab::Stash => stash_tab_body(t, &snap, stash_input, weak.clone()).into_any_element(),
            GitTab::Conflicts => conflicts_tab_body(t, &snap, weak.clone()).into_any_element(),
        }
    };

    let mut col = div()
        .size_full()
        .flex()
        .flex_col()
        .bg(t.color.bg_surface)
        .child(header(t, &snap, weak.clone()))
        .child(Separator::horizontal());

    if let Some(el) = confirm_strip {
        col = col.child(el).child(Separator::horizontal());
    }
    if let Some(el) = action_err_strip {
        col = col.child(el).child(Separator::horizontal());
    }
    if let Some(el) = probe_err_strip {
        col = col.child(el).child(Separator::horizontal());
    }

    // Branch row is the Pier-native action bar: branch name, pace
    // pills, then icon buttons for pull / push and manager popups.
    if let Some(branch) = snap.branch.clone() {
        col = col
            .child(branch_actions_row(
                t,
                &branch,
                &snap.branches,
                manager_open,
                weak.clone(),
            ))
            .child(Separator::horizontal());
    }

    // Wrap the tab body in a flex-1 container so the graph list /
    // changes list / stash list all fill the space between the tabs
    // and the commit footer. Without this `gpui::list` gets 0 height
    // from its `flex_grow()` chain and the graph stays blank.
    col = col
        .child(primary_tabs(active, weak.clone()))
        .child(Separator::horizontal())
        .child(
            div()
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .overflow_hidden()
                .child(body),
        );

    // Sticky commit footer — only relevant while the user is on
    // the Changes tab. Always render so the user can type a commit
    // message before staging. The splitter above it is draggable;
    // the container captures global mouse-move / mouse-up so the
    // drag doesn't die when the cursor leaves the splitter.
    if active == GitTab::Changes && manager_open.is_none() {
        col = col
            .child(commit_footer_splitter(t, weak.clone()))
            .child(commit_footer(
                &snap.changes,
                commit_input,
                weak.clone(),
                cx,
                snap.footer_height,
                snap.commit_action_mode,
            ));
    }

    // Drag listeners are only attached WHILE a drag is in flight.
    // Otherwise the root captures every mouse move / mouse up and
    // makes the input / tab bar / graph rows feel laggy — and in
    // some cases, GPUI's input subsystem sees the mouse-up capture
    // and drops the input's pending keystroke state.
    let footer_dragging = snap.footer_dragging;
    let graph_dragging = snap.graph.column_dragging;
    let root = col.id("git-panel-root");
    if !footer_dragging && !graph_dragging {
        return root;
    }

    if footer_dragging {
        let move_weak = weak.clone();
        let up_weak = weak.clone();
        return root
            .cursor_row_resize()
            .on_mouse_move(move |ev, _, cx| {
                let y = ev.position.y.to_f64() as f32;
                let _ = move_weak.update(cx, |app, cx| app.update_git_footer_drag(y, cx));
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                let _ = up_weak.update(cx, |app, cx| app.end_git_footer_drag(cx));
            });
    }

    let move_weak = weak.clone();
    let up_weak = weak.clone();
    root.cursor_col_resize()
        .on_mouse_move(move |ev, _, cx| {
            let x = ev.position.x.to_f64() as f32;
            let _ = move_weak.update(cx, |app, cx| app.update_git_graph_column_drag(x, cx));
        })
        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
            let _ = up_weak.update(cx, |app, cx| app.end_git_graph_column_drag(cx));
        })
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
                // SF `folder.fill` on the repo section header in Pier.
                .icon(IconName::FolderFill)
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
        .child(header(t, snap, weak.clone()))
        .child(Separator::horizontal())
        .child(git_feedback_strip(
            t,
            "git-fb-error-layout",
            IconName::WarningFill,
            snap.last_error.clone().unwrap_or_default(),
            true,
            {
                let w = weak.clone();
                move |_, _, cx| {
                    let _ = w.update(cx, |app, cx| app.clear_git_last_error(cx));
                }
            },
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
    let mut tabs = Tabs::new();
    for tab in GitTab::all() {
        let is_active = tab == active;
        // Per-tab icon mapping mirrors Pier's GitPanelView segmented
        // picker: Changes=`tray.full` (filled inbox), History=commit
        // glyph, Stash=`square.stack.3d.up.fill`, Conflicts=warning
        // (outline — filled would scream "error" on every load).
        let (label_key, icon) = match tab {
            GitTab::Changes => ("App.Git.tab_changes", IconName::TrayFill),
            GitTab::History => ("App.Git.tab_graph", IconName::GitCommit),
            GitTab::Stash => ("App.Git.tab_stash", IconName::StackFill),
            GitTab::Conflicts => ("App.Git.mgr_conflicts", IconName::TriangleAlert),
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

/// One-line branch action bar — lives directly under the header.
/// Layout: branch pill + ahead/behind pills + fetch/pull/push icons
/// + manager-popup icons (branches / tags / remotes / config /
/// submodules / rebase). Toggling a manager icon opens the
/// overlay panel above the tab body.
fn branch_actions_row(
    t: &crate::theme::Theme,
    branch: &BranchInfo,
    _branches: &[String],
    open_panel: Option<ManagerTab>,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let tracking = if branch.tracking.is_empty() {
        String::new()
    } else {
        format!(" → {}", branch.tracking)
    };
    let name_line = SharedString::from(format!("{}{}", branch.name, tracking));

    let mut row = div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap(SP_1)
        .px(SP_3)
        .py(SP_1);

    // Pill opens the branches manager dialog — mirrors Pier's
    // clickable "current branch" chip in the top-right chrome.
    let pill_weak = weak.clone();
    row = row.child(
        div()
            .id("git-branch-pill")
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1)
            .px(SP_1_5)
            .py(SP_0_5)
            .rounded(RADIUS_SM)
            .cursor_pointer()
            .hover(|s| s.bg(t.color.bg_hover))
            .tooltip(|win, cx| {
                gpui_component::tooltip::Tooltip::new(t!("App.Git.switch_branch").to_string())
                    .build(win, cx)
            })
            .on_click(move |_, win, cx| {
                let app = pill_weak.clone();
                git_dialogs::open_branches_dialog(win, cx, app);
            })
            .child(
                div()
                    .flex_none()
                    .text_color(t.color.accent)
                    .child(gpui_component::Icon::new(IconName::GitBranch).size(ICON_SM)),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(SIZE_SMALL)
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(t.color.text_primary)
                    .child(name_line),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(t.color.text_tertiary)
                    .child(gpui_component::Icon::new(IconName::ChevronDown).size(ICON_SM)),
            ),
    );

    if branch.ahead > 0 || branch.behind > 0 {
        row = row.child(
            div()
                .flex_none()
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_tertiary)
                .px(SP_1)
                .child(SharedString::from(format!(
                    "↑{} ↓{}",
                    branch.ahead, branch.behind
                ))),
        );
    }

    row = row.child(div().flex_1().min_w(px(0.0)));

    // Icon action buttons — left-to-right: floating manager
    // dialogs, then fetch / pull / push. Each icon pops a modal
    // via `window.open_dialog`, matching Pier's behaviour.
    let _ = open_panel; // legacy overlay signal, unused now
                        // Manager-dialog trigger icons — aligned with Pier's
                        // GitBranchManagerView / GitTagManagerView / … SF Symbol choices:
                        //   Tags → `tag.fill`, Config → `gearshape.fill`,
                        //   Rebase → `arrow.triangle.2.circlepath`.
    for (mgr, icon, tooltip_key) in [
        (
            ManagerTab::Branches,
            IconName::GitBranch,
            "App.Git.mgr_branches",
        ),
        (ManagerTab::Tags, IconName::TagFill, "App.Git.mgr_tags"),
        (ManagerTab::Remotes, IconName::Globe, "App.Git.mgr_remotes"),
        (
            ManagerTab::Submodules,
            IconName::Container,
            "App.Git.mgr_submodules",
        ),
        (ManagerTab::Config, IconName::GearFill, "App.Git.mgr_config"),
        (
            ManagerTab::Rebase,
            IconName::ArrowsCounterClockwise,
            "App.Git.mgr_rebase",
        ),
    ] {
        let w = weak.clone();
        row = row.child(
            IconButton::new(
                ElementId::Name(format!("git-mgr-icon-{}", mgr.id_token()).into()),
                icon,
            )
            .size(IconButtonSize::Sm)
            .variant(IconButtonVariant::Ghost)
            .tooltip(t!(tooltip_key))
            .on_click(move |_, win, cx| {
                let app = w.clone();
                match mgr {
                    ManagerTab::Branches => git_dialogs::open_branches_dialog(win, cx, app),
                    ManagerTab::Tags => git_dialogs::open_tags_dialog(win, cx, app),
                    ManagerTab::Remotes => git_dialogs::open_remotes_dialog(win, cx, app),
                    ManagerTab::Submodules => git_dialogs::open_submodules_dialog(win, cx, app),
                    ManagerTab::Config => git_dialogs::open_config_dialog(win, cx, app),
                    ManagerTab::Rebase => git_dialogs::open_rebase_dialog(win, cx, app),
                    ManagerTab::Conflicts => {} // routed to top-level tab instead
                }
            }),
        );
    }

    // Fetch / Pull / Push — Pier uses `arrow.clockwise` / `arrow.down.doc.fill` /
    // `arrow.up.doc.fill` respectively on these exact controls. The file-
    // arrow icons read instantly as "fetch the file contents" vs a generic
    // arrow glyph.
    let fetch_w = weak.clone();
    row = row.child(
        IconButton::new("git-row-fetch", IconName::RefreshCw)
            .size(IconButtonSize::Sm)
            .variant(IconButtonVariant::Ghost)
            .tooltip(t!("App.Git.remote_fetch"))
            .on_click(move |_, _, cx| {
                let _ = fetch_w.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::RemoteFetch { name: None }, cx);
                });
            }),
    );
    let pull_w = weak.clone();
    row = row.child(
        IconButton::new("git-row-pull", IconName::FileArrowDownFill)
            .size(IconButtonSize::Sm)
            .variant(IconButtonVariant::Ghost)
            .tooltip(t!("App.Git.pull"))
            .on_click(move |_, _, cx| {
                let _ = pull_w.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::Pull, cx);
                });
            }),
    );
    let push_w = weak.clone();
    row = row.child(
        IconButton::new("git-row-push", IconName::FileArrowUpFill)
            .size(IconButtonSize::Sm)
            .variant(IconButtonVariant::Ghost)
            .tooltip(t!("App.Git.push"))
            .on_click(move |_, _, cx| {
                let _ = push_w.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::Push, cx);
                });
            }),
    );

    row
}

/// Manager overlay — wraps a single manager panel (Branches / Tags /
/// Remotes / Config / Submodules / Rebase) above the tab body with a
/// small breadcrumb header + close button. Mirrors Pier's "click
/// manager icon → panel floats on top" behaviour.
fn manager_overlay(
    t: &crate::theme::Theme,
    panel: ManagerTab,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    let close_w = weak.clone();
    let title: SharedString = match panel {
        ManagerTab::Branches => t!("App.Git.mgr_branches").into(),
        ManagerTab::Tags => t!("App.Git.mgr_tags").into(),
        ManagerTab::Remotes => t!("App.Git.mgr_remotes").into(),
        ManagerTab::Config => t!("App.Git.mgr_config").into(),
        ManagerTab::Submodules => t!("App.Git.mgr_submodules").into(),
        ManagerTab::Rebase => t!("App.Git.mgr_rebase").into(),
        ManagerTab::Conflicts => t!("App.Git.mgr_conflicts").into(),
    };

    let header = div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .px(SP_3)
        .py(SP_1)
        .bg(t.color.bg_panel)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(SIZE_SMALL)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child(title),
        )
        .child(
            IconButton::new("git-mgr-close", IconName::Close)
                .size(IconButtonSize::Sm)
                .variant(IconButtonVariant::Ghost)
                .on_click(move |_, _, cx| {
                    let _ = close_w.update(cx, |app, cx| {
                        app.set_git_manager_panel(None, cx);
                    });
                }),
        );

    let body: gpui::AnyElement = if snap.managers.loading && snap.managers.branches.is_empty() {
        div()
            .px(SP_3)
            .py(SP_3)
            .child(text::caption(t!("App.Common.Status.loading")).secondary())
            .into_any_element()
    } else {
        match panel {
            ManagerTab::Branches => {
                branches_manager(t, &snap.managers.branches, weak.clone()).into_any_element()
            }
            ManagerTab::Tags => {
                tags_manager(t, &snap.managers.tags, weak.clone()).into_any_element()
            }
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
        }
    };

    div().w_full().flex().flex_col().child(header).child(body)
}

/// Standalone Conflicts tab body — wraps the conflicts_manager so
/// the user can resolve merges without going through the overlay.
fn conflicts_tab_body(
    t: &crate::theme::Theme,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .child(conflicts_manager(t, &snap.managers.conflicts, weak))
}

/// Sticky commit footer — the Pier-native resizable commit pane that
/// sits below the tab body. The bordered editor surface lives above a
/// detached action row, matching sibling Pier's layout and keeping
/// the Stage All / Commit controls visually anchored to the bottom.
fn commit_footer(
    changes: &[GitFileChange],
    input: gpui::Entity<InputState>,
    weak: WeakEntity<PierApp>,
    cx: &App,
    height: f32,
    mode: CommitActionMode,
) -> impl IntoElement {
    let has_unstaged = changes.iter().any(|c| !c.staged);
    let staged = changes.iter().any(|c| c.staged);
    let has_message = !input.read(cx).value().trim().is_empty();
    let can_submit = staged && has_message;
    let stage_all_weak = weak.clone();

    // Keep Stage All anchored on the left like sibling Pier. When the
    // worktree is clean (or everything is already staged) the button
    // stays in place but disables instead of disappearing.
    let stage_all_btn: gpui::AnyElement =
        Button::ghost("git-footer-stage-all", t!("App.Git.stage_all"))
            .size(ButtonSize::Sm)
            .disabled(!has_unstaged)
            .tooltip(t!("App.Git.stage_all"))
            .on_click(move |_, _, cx| {
                let _ = stage_all_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::StageAll, cx);
                });
            })
            .into_any_element();

    // Label + tooltip derive from the currently-selected mode so the
    // primary half of the split-button reads as the action it will
    // fire.
    let (primary_label_key, primary_tooltip_key) = match mode {
        CommitActionMode::Commit => ("App.Git.commit_staged", "App.Git.commit_staged"),
        CommitActionMode::CommitAndPush => ("App.Git.commit_and_push", "App.Git.commit_and_push"),
    };
    let primary_label: SharedString = t!(primary_label_key).to_string().into();

    let input_for_primary = input.clone();
    let primary_weak = weak.clone();
    let input_for_pick = input.clone();
    let pick_weak = weak.clone();

    let split = SplitButton::new("git-footer-commit", primary_label)
        .size(ButtonSize::Sm)
        .disabled(!can_submit)
        .tooltip(t!(primary_tooltip_key))
        .current(mode.id())
        .option(SplitButtonOption::new(
            CommitActionMode::Commit.id(),
            t!("App.Git.commit_staged").to_string(),
        ))
        .option(SplitButtonOption::new(
            CommitActionMode::CommitAndPush.id(),
            t!("App.Git.commit_and_push").to_string(),
        ))
        .on_primary_click(move |_, _, cx| {
            let text: String = input_for_primary.read(cx).value().to_string();
            if !staged || text.trim().is_empty() {
                return;
            }
            let _ = primary_weak.update(cx, |app, cx| match mode {
                CommitActionMode::Commit => {
                    app.schedule_git_action(
                        GitPendingAction::Commit {
                            message: text.clone(),
                        },
                        cx,
                    );
                }
                CommitActionMode::CommitAndPush => {
                    app.schedule_git_commit_and_push(text.clone(), cx);
                }
            });
        })
        .on_pick(move |value, _, cx| {
            let Some(picked) = CommitActionMode::from_id(value) else {
                return;
            };
            let text: String = input_for_pick.read(cx).value().to_string();
            let _ = pick_weak.update(cx, |app, cx| {
                app.set_git_commit_action_mode(picked, cx);
                if !staged || text.trim().is_empty() {
                    return;
                }
                match picked {
                    CommitActionMode::Commit => {
                        app.schedule_git_action(
                            GitPendingAction::Commit {
                                message: text.clone(),
                            },
                            cx,
                        );
                    }
                    CommitActionMode::CommitAndPush => {
                        app.schedule_git_commit_and_push(text.clone(), cx);
                    }
                }
            });
        });

    div()
        .flex_none()
        .w_full()
        .h(px(clamp_git_footer_height(height)))
        .flex()
        .flex_col()
        .px(SP_2)
        .pt(SP_1)
        .pb(SP_1_5)
        .child(
            div().flex_1().min_h(px(0.0)).w_full().child(
                CommitComposer::new(&input)
                    .bottom_left(stage_all_btn)
                    .bottom_right(split),
            ),
        )
}

/// Thin drag bar between the tab body and the commit footer —
/// captures `mouse_down` to start a drag, the parent panel (via
/// `tab_layout`) then streams `mouse_move` / `mouse_up` into
/// PierApp. Styled as a 4px hairline with a row-resize cursor on
/// hover.
fn commit_footer_splitter(t: &crate::theme::Theme, weak: WeakEntity<PierApp>) -> impl IntoElement {
    // 6 px hit target with a 1 px visible hairline in the middle.
    // Matches Pier's `SideBySideContainerView` divider grammar:
    // wider invisible hit area, thin visible rule.
    div()
        .id("git-footer-splitter")
        .w_full()
        .h(px(6.0))
        .flex()
        .flex_col()
        .justify_center()
        .bg(t.color.bg_panel)
        .hover(|s| s.bg(t.color.accent_muted))
        .cursor_row_resize()
        .child(div().w_full().h(px(1.0)).bg(t.color.border_default))
        .on_mouse_down(MouseButton::Left, move |ev, _, cx| {
            let y = ev.position.y.to_f64() as f32;
            let _ = weak.update(cx, |app, cx| app.begin_git_footer_drag(y, cx));
        })
}

// ─── Tab: Changes ────────────────────────────────────────────────────

fn changes_tab_body(
    t: &crate::theme::Theme,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    // Pier parity: render staged above unstaged as two standalone
    // sections. Staged is capped (see CHANGES_STAGED_MAX_H) so a huge
    // stage set doesn't starve the unstaged area; unstaged fills the
    // remaining height. If neither side has changes we still render
    // the unstaged shell so users see the "working_tree_clean" hint.
    let staged: Vec<&GitFileChange> = snap.changes.iter().filter(|c| c.staged).collect();
    let unstaged: Vec<&GitFileChange> = snap.changes.iter().filter(|c| !c.staged).collect();

    let mut col = div().size_full().flex().flex_col().min_h(px(0.0));

    if !staged.is_empty() {
        col = col.child(changes_list_view(
            t,
            &staged,
            true,
            snap.diff_selection.as_ref(),
            Some(crate::theme::heights::CHANGES_STAGED_MAX_H),
            weak.clone(),
        ));
    }

    col = col.child(changes_list_view(
        t,
        &unstaged,
        false,
        snap.diff_selection.as_ref(),
        None,
        weak.clone(),
    ));

    if snap.diff_selection.is_some() {
        col = col
            .child(Separator::horizontal())
            .child(diff_section(t, snap, weak.clone()));
    }
    col
}

/// One staged-or-unstaged section: coloured-dot header with count and
/// a single trailing action ("Stage all" / "Unstage all"), then a
/// scrolling list of file rows. `max_h` bounds the section (used for
/// staged in Pier) — when None the section is flex-1 and fills the
/// remaining vertical space.
fn changes_list_view(
    t: &crate::theme::Theme,
    files: &[&GitFileChange],
    staged: bool,
    diff_selection: Option<&crate::app::git_session::DiffSelection>,
    max_h: Option<gpui::Pixels>,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let count = files.len();
    let (title, dot_color): (SharedString, gpui::Rgba) = if staged {
        (
            t!("App.Git.staged_count", count = count).to_string().into(),
            t.color.status_success,
        )
    } else {
        (
            t!("App.Git.unstaged_count", count = count)
                .to_string()
                .into(),
            t.color.status_warning,
        )
    };

    let action_weak = weak.clone();
    let action: gpui::AnyElement = if staged && count > 0 {
        Button::ghost("git-section-unstage-all", t!("App.Git.unstage_all"))
            .size(ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                let _ = action_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::UnstageAll, cx);
                });
            })
            .into_any_element()
    } else if !staged && count > 0 {
        Button::ghost("git-section-stage-all", t!("App.Git.stage_all"))
            .size(ButtonSize::Sm)
            .on_click(move |_, _, cx| {
                let _ = action_weak.update(cx, |app, cx| {
                    app.schedule_git_action(GitPendingAction::StageAll, cx);
                });
            })
            .into_any_element()
    } else {
        div().flex_none().into_any_element()
    };

    let header = div()
        .w_full()
        .flex_none()
        .h(crate::theme::heights::INSPECTOR_HEADER_H)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_3)
        .bg(t.color.bg_panel)
        .child(
            div()
                .flex_none()
                .w(crate::theme::heights::PILL_DOT)
                .h(crate::theme::heights::PILL_DOT)
                .rounded(RADIUS_PILL)
                .bg(dot_color),
        )
        .child(text::caption(title).secondary())
        .child(div().flex_1().min_w(px(0.0)))
        .child(div().flex_none().child(action));

    // Body: scrolling list of rows. Empty unstaged section still
    // shows the "clean" hint so the panel never looks blank.
    let body_id: ElementId = if staged {
        ElementId::Name(SharedString::from("git-staged-body"))
    } else {
        ElementId::Name(SharedString::from("git-unstaged-body"))
    };
    let mut body = div()
        .id(body_id)
        .w_full()
        .flex_1()
        .min_h(px(0.0))
        .overflow_y_scroll()
        .flex()
        .flex_col();
    if files.is_empty() {
        body = body.child(
            div()
                .px(SP_3)
                .py(SP_2)
                .child(text::caption(t!("App.Git.working_tree_clean")).secondary()),
        );
    } else {
        for change in files.iter().take(MAX_CHANGE_ROWS) {
            let is_selected = diff_selection
                .map(|sel| sel.path == change.path && sel.staged == change.staged)
                .unwrap_or(false);
            body = body.child(file_change_row(t, change, is_selected, weak.clone()));
        }
        if files.len() > MAX_CHANGE_ROWS {
            body = body.child(
                div()
                    .px(SP_3)
                    .py(SP_1)
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child(SharedString::from(
                        t!(
                            "App.Git.more_changes",
                            count = files.len() - MAX_CHANGE_ROWS
                        )
                        .to_string(),
                    )),
            );
        }
    }

    let mut section = div().w_full().flex().flex_col().min_h(px(0.0));
    if let Some(cap) = max_h {
        section = section.flex_none().max_h(cap);
    } else {
        section = section.flex_1();
    }
    section
        .child(header)
        .child(Separator::horizontal())
        .child(body)
}

// ─── Branch card (legacy — kept `fn branch_section` stubbed out
//     until the remaining callers are removed; see `#[allow]`).

#[allow(dead_code)]
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

#[allow(dead_code)]
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
// The single "working_tree" InspectorSection has been replaced by the
// split staged / unstaged layout in `changes_list_view` above (matches
// Pier's GitPanelView layering). The old helper is gone; if you land
// here looking for it, see `changes_tab_body`.

fn file_change_row(
    t: &crate::theme::Theme,
    change: &GitFileChange,
    _is_selected: bool,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let (badge, badge_color) = file_status_badge(t, change.status.clone());
    let path_str = change.path.clone();
    let staged = change.staged;
    let untracked = matches!(change.status, FileStatus::Untracked);

    // Split "src/foo/bar.rs" → ("bar.rs", "src/foo") so the row
    // reads "<filename>   <relative dir>" the way Pier shows it.
    let (name, parent) = split_path_for_display(&path_str);

    let stage_weak = weak.clone();
    let unstage_weak = weak.clone();
    let discard_weak = weak.clone();
    let dbl_weak = weak.clone();

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

    let path_for_stage = path_str.clone();
    let path_for_unstage = path_str.clone();
    let path_for_discard = path_str.clone();
    let path_for_dbl = path_str.clone();

    let row_id = ElementId::Name(SharedString::from(format!(
        "git-file-row-{}-{}",
        if staged { "s" } else { "w" },
        short_id(&path_str)
    )));
    let row = div()
        .id(row_id)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .px(SP_3)
        .py(SP_0_5)
        .overflow_hidden()
        .hover(|s| s.bg(t.color.bg_hover))
        .cursor_pointer()
        // Double-click opens the full diff dialog — matches Pier's
        // behaviour (DiffWindowController.show on file row click).
        .on_mouse_down(MouseButton::Left, move |ev, win, cx| {
            if ev.click_count < 2 {
                return;
            }
            let p = path_for_dbl.clone();
            let _ = dbl_weak.update(cx, |app, cx| {
                app.open_git_file_diff_dialog(p, staged, untracked, win, cx);
            });
        })
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
                .flex_none()
                .truncate()
                .text_size(SIZE_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(if staged {
                    t.color.text_primary
                } else {
                    t.color.text_secondary
                })
                .child(SharedString::from(name)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(parent)),
        );

    // Per-file icon actions — Pier parity. Ghost icon buttons so
    // the row stays visually quiet; hover reveals the tint.
    let mut row = row;
    if staged {
        row = row.child(
            IconButton::new(unstage_id, IconName::Minus)
                .size(IconButtonSize::Sm)
                .variant(IconButtonVariant::Ghost)
                .tooltip(t!("App.Git.unstage"))
                .on_click(move |_, _, cx| {
                    let p = path_for_unstage.clone();
                    let _ = unstage_weak.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::Unstage { path: p }, cx);
                    });
                }),
        );
    } else {
        row = row.child(
            IconButton::new(stage_id, IconName::Plus)
                .size(IconButtonSize::Sm)
                .variant(IconButtonVariant::Ghost)
                .tooltip(t!("App.Git.stage"))
                .on_click(move |_, _, cx| {
                    let p = path_for_stage.clone();
                    let _ = stage_weak.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::Stage { path: p }, cx);
                    });
                }),
        );
        row = row.child(
            IconButton::new(discard_id, IconName::Delete)
                .size(IconButtonSize::Sm)
                .variant(IconButtonVariant::Ghost)
                .tooltip(t!("App.Git.discard"))
                .on_click(move |_, _, cx| {
                    let p = path_for_discard.clone();
                    let _ = discard_weak.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::Discard { path: p }, cx);
                    });
                }),
        );
    }
    row
}

/// Split a repo-relative path into `(filename, parent_dir)`. Root
/// files return `(name, "")`. Used by the Changes file row so the
/// row reads "foo.rs   src/bar" — matches Pier's `file.lastPathComponent`
/// + `file.deletingLastPathComponent()` layout.
fn split_path_for_display(path: &str) -> (String, String) {
    if let Some(idx) = path.rfind('/') {
        (path[idx + 1..].to_string(), path[..idx].to_string())
    } else {
        (path.to_string(), String::new())
    }
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

#[allow(dead_code)]
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
        // Pier's DiffView section uses `doc.text.magnifyingglass` — the
        // "examine this file" cue.
        .icon(IconName::FileMagnifyingGlass)
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
        Button::primary(id, label)
            .size(ButtonSize::Sm)
            .on_click(on_click)
    } else {
        Button::ghost(id, label)
            .size(ButtonSize::Sm)
            .on_click(on_click)
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

pub fn diff_line_row_export(t: &crate::theme::Theme, index: usize, line: &str) -> impl IntoElement {
    diff_line_row(t, index, line)
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
        if line.starts_with("@@")
            || line.starts_with("diff --git")
            || line.starts_with("index ")
            || line.starts_with("+++")
            || line.starts_with("---")
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

fn flush_pairs(
    out: &mut Vec<(String, String)>,
    deletes: &mut Vec<String>,
    inserts: &mut Vec<String>,
) {
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
        // Pier's stash UI stamps each entry with `square.stack.3d.up.fill`.
        .icon(IconName::StackFill)
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

#[allow(dead_code)]
fn log_section(t: &crate::theme::Theme, log: &[CommitInfo]) -> impl IntoElement {
    let count_pill = StatusPill::new(
        t!("App.Git.entries_count", count = log.len()),
        StatusKind::Info,
    );
    let mut section = InspectorSection::new(t!("App.Git.recent_commits"))
        // Pier's recent-commits strip uses `clock.arrow.circlepath`
        // — "things that happened recently".
        .icon(IconName::ClockCounterClockwise)
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

#[allow(dead_code)]
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

/// Colored feedback banner — "拉取完成" / "推送失败" style. Shown
/// briefly after every Git action; the user can also dismiss it
/// with the trailing ✕. Success uses a green tint, errors red —
/// alpha 0.12 over the surface so it reads as a banner, not a
/// slab. No title line; the message is the content.
fn git_feedback_strip(
    t: &crate::theme::Theme,
    id: impl Into<ElementId>,
    icon: IconName,
    message: SharedString,
    is_error: bool,
    on_dismiss: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (tint, fg) = if is_error {
        (
            gpui::Rgba {
                a: 0.12,
                ..t.color.status_error
            },
            t.color.status_error,
        )
    } else {
        (
            gpui::Rgba {
                a: 0.12,
                ..t.color.status_success
            },
            t.color.status_success,
        )
    };
    div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_1)
        .bg(tint)
        .child(
            div()
                .flex_none()
                .text_color(fg)
                .child(gpui_component::Icon::new(icon).size(ICON_SM)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .text_size(SIZE_SMALL)
                .text_color(fg)
                .child(message),
        )
        .child(
            IconButton::new(id, IconName::Close)
                .size(IconButtonSize::Sm)
                .variant(IconButtonVariant::Ghost)
                .on_click(on_dismiss),
        )
}

// ─── Tab: Graph ──────────────────────────────────────────────────────

fn graph_tab_body(
    t: &crate::theme::Theme,
    snap: &GitSnapshot,
    search_input: gpui::Entity<InputState>,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    let mut col = div().size_full().flex().flex_col();
    col = col.child(graph_toolbar(t, &snap.graph, search_input, weak.clone()));
    col = col.child(Separator::horizontal());

    if let Some(err) = snap.graph.error.clone() {
        let w = weak.clone();
        col = col.child(git_feedback_strip(
            t,
            "git-fb-graph-err",
            IconName::WarningFill,
            err,
            true,
            move |_, _, cx| {
                let _ = w.update(cx, |app, cx| app.clear_git_action_error(cx));
            },
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

    // Graph rows — variable-height virtualized list. The selected
    // commit's row expands with its inline detail strip right
    // below it (Pier's behaviour), so uniform heights are out —
    // `gpui::list` handles this via re-measurement.
    let col_w = compute_graph_col_width(&snap.graph.rows).max(60.0);
    let rows: std::sync::Arc<Vec<GraphRow>> = std::sync::Arc::new(
        snap.graph
            .rows
            .iter()
            .take(MAX_GRAPH_ROWS)
            .cloned()
            .collect(),
    );
    let selected_hash = snap.graph.selected.clone();
    let graph_snap_for_list = snap.graph.clone();
    let detail_snapshot = snap.commit_detail.clone();
    let unpushed_for_list = snap.graph.unpushed.clone();
    let parents_by_hash: std::sync::Arc<std::collections::HashMap<String, Option<String>>> = {
        let mut m = std::collections::HashMap::with_capacity(rows.len());
        for r in rows.iter() {
            let parent = r
                .parents
                .split(' ')
                .find(|p| !p.is_empty())
                .map(|p| p.to_string());
            m.insert(r.hash.clone(), parent);
        }
        std::sync::Arc::new(m)
    };
    let t_clone = t.clone();
    let weak_for_rows = weak.clone();

    // Auto-paginate once the user scrolls within 20 rows of the
    // bottom. The background fetch appends to the row buffer so
    // scroll position stays stable.
    let has_more = snap.graph.has_more;
    let load_weak_scroll = weak.clone();
    let list_state = snap.graph.list_state.clone();
    list_state.set_scroll_handler(move |ev, _window, cx| {
        if has_more && ev.visible_range.end + 20 >= ev.count {
            let _ = load_weak_scroll.update(cx, |app, cx| {
                app.schedule_git_graph(false, cx);
            });
        }
    });

    let list_element = list(list_state, move |idx, _window, _cx| {
        let Some(row) = rows.get(idx) else {
            return div().into_any_element();
        };
        let is_selected = selected_hash.as_deref() == Some(row.hash.as_str());
        let dimmed = should_dim_row_ns(row, &graph_snap_for_list);
        let zebra = graph_snap_for_list.zebra_stripes && idx % 2 == 1;

        let row_el = graph_row_element(
            &t_clone,
            row,
            col_w,
            dimmed,
            is_selected,
            zebra,
            &graph_snap_for_list,
            weak_for_rows.clone(),
        )
        .into_any_element();

        if !is_selected {
            return row_el;
        }

        // Inline detail — renders directly below the selected row.
        // `commit_detail_strip` needs (hash → parent) lookups +
        // unpushed set; pass pre-built clones to the closure.
        let parent = parents_by_hash.get(&row.hash).cloned().flatten();
        let detail_el: gpui::AnyElement = if let Some(d) = detail_snapshot.detail.as_ref() {
            commit_detail_strip_standalone(
                &t_clone,
                d,
                &unpushed_for_list,
                parent,
                weak_for_rows.clone(),
            )
            .into_any_element()
        } else if detail_snapshot.loading {
            div()
                .px(SP_3)
                .py(SP_1_5)
                .child(text::caption(t!("App.Common.Status.loading")).secondary())
                .into_any_element()
        } else if let Some(err) = detail_snapshot.error.clone() {
            git_feedback_strip(
                &t_clone,
                "git-fb-detail-err",
                IconName::WarningFill,
                err,
                true,
                |_, _, _| {},
            )
            .into_any_element()
        } else {
            div().into_any_element()
        };

        div()
            .w_full()
            .flex()
            .flex_col()
            .child(row_el)
            .child(Separator::horizontal())
            .child(detail_el)
            .into_any_element()
    })
    .flex_1()
    // `gpui::list` needs a bounded vertical axis to virtualize —
    // `min_h(0)` kicks the flex solver so the list takes exactly
    // the space our flex_1 parent hands it (instead of trying to
    // measure every child up-front and collapsing to 0).
    .min_h(px(0.0))
    .w_full()
    .h_full();

    let graph_min_w = graph_content_min_width(col_w, &snap.graph);
    col = col.child(
        div()
            .flex_1()
            .min_h(px(0.0))
            .overflow_x_scrollbar()
            .child(
                div()
                    .w_full()
                    .min_w(px(graph_min_w))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(list_element),
            ),
    );

    // Load-more / all-loaded footer.
    if snap.graph.has_more {
        let load_weak = weak.clone();
        col = col.child(
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
        col = col.child(
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

    col
}

/// Non-snapshot version of `should_dim_row` — operates on the
/// inner `GraphStateSnapshot` so `uniform_list`'s `'static` closure
/// doesn't need to borrow the outer `GitSnapshot`.
fn should_dim_row_ns(row: &GraphRow, g: &GraphStateSnapshot) -> bool {
    match g.highlight_mode {
        GraphHighlightMode::None => false,
        GraphHighlightMode::MyCommits => {
            let me = &g.user_name;
            !me.is_empty() && row.author != *me
        }
        GraphHighlightMode::MergeCommits => {
            row.parents.split(' ').filter(|s| !s.is_empty()).count() < 2
        }
        GraphHighlightMode::CurrentBranch => row.color_index != 0,
    }
}

fn graph_toolbar(
    t: &crate::theme::Theme,
    graph: &GraphStateSnapshot,
    search_input: gpui::Entity<InputState>,
    weak: WeakEntity<PierApp>,
) -> gpui::Div {
    let mut toolbar = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(SP_1)
        .px(SP_3)
        .py(SP_1_5)
        .bg(t.color.bg_panel);

    let mut top_row = div()
        .w_full()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(SP_1);
    let mut middle_row = div()
        .w_full()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(SP_1);
    let mut bottom_row = div()
        .w_full()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(SP_1);

    // Live search input — Enter triggers a reload through
    // `commit_git_graph_search` (subscribed once at app-boot in
    // `PierApp::new`), the ✕ button clears filter + input.
    let mut search_row = div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_0_5)
        .h(BUTTON_SM_H)
        .px(SP_1)
        .min_w(px(180.0))
        .max_w(px(260.0))
        .rounded(RADIUS_SM)
        .bg(t.color.bg_surface)
        .border_1()
        .border_color(t.color.border_subtle)
        .child(
            div()
                .flex_none()
                .text_color(t.color.text_tertiary)
                .child(gpui_component::Icon::new(IconName::Search).size(ICON_SM)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .child(InlineInput::new(&search_input).tone(InlineInputTone::Inset)),
        );
    if graph.filter.search_text.is_some() {
        let clear_weak = weak.clone();
        let cur_filter_clear = graph.filter.clone();
        let clear_input = search_input.clone();
        search_row = search_row.child(
            IconButton::new("git-graph-search-clear", IconName::Close)
                .size(IconButtonSize::Sm)
                .variant(IconButtonVariant::Ghost)
                .on_click(move |_, win, cx| {
                    clear_input.update(cx, |s, c| s.set_value("", win, c));
                    let next = GraphFilter {
                        search_text: None,
                        ..cur_filter_clear.clone()
                    };
                    let _ = clear_weak.update(cx, |app, cx| {
                        app.set_git_graph_filter(next.clone(), cx);
                    });
                }),
        );
    }
    top_row = top_row.child(search_row);

    let mut branch_options = Vec::with_capacity(graph.branches.len() + 1);
    branch_options.push(DropdownOption::new("", t!("App.Git.graph_all_branches")));
    branch_options.extend(
        graph
            .branches
            .iter()
            .cloned()
            .map(|branch| DropdownOption::new(branch.clone(), branch)),
    );
    let cur_filter_b = graph.filter.clone();
    let weak_branch = weak.clone();
    top_row = top_row.child(
        Dropdown::new("git-graph-branch")
            .size(DropdownSize::Sm)
            .width(px(144.0))
            .leading_icon(IconName::GitBranch)
            .placeholder(t!("App.Git.graph_all_branches"))
            .value(graph.filter.branch.clone().unwrap_or_default())
            .options(branch_options)
            .on_change(move |value, _, cx| {
                let next = GraphFilter {
                    branch: if value.is_empty() {
                        None
                    } else {
                        Some(value.to_string())
                    },
                    ..cur_filter_b.clone()
                };
                let _ = weak_branch.update(cx, |app, cx| {
                    app.set_git_graph_filter(next.clone(), cx);
                });
            }),
    );

    let mut author_options = Vec::with_capacity(graph.authors.len() + 1);
    author_options.push(DropdownOption::new("", t!("App.Git.graph_all_users")));
    author_options.extend(
        graph
            .authors
            .iter()
            .cloned()
            .map(|author| DropdownOption::new(author.clone(), author)),
    );
    let cur_filter_u = graph.filter.clone();
    let weak_user = weak.clone();
    top_row = top_row.child(
        Dropdown::new("git-graph-user")
            .size(DropdownSize::Sm)
            .width(px(144.0))
            .leading_icon(IconName::UserFill)
            .placeholder(t!("App.Git.graph_all_users"))
            .value(graph.filter.author.clone().unwrap_or_default())
            .options(author_options)
            .on_change(move |value, _, cx| {
                let next = GraphFilter {
                    author: if value.is_empty() {
                        None
                    } else {
                        Some(value.to_string())
                    },
                    ..cur_filter_u.clone()
                };
                let _ = weak_user.update(cx, |app, cx| {
                    app.set_git_graph_filter(next.clone(), cx);
                });
            }),
    );

    let date_options = [
        GraphDateRange::All,
        GraphDateRange::Today,
        GraphDateRange::LastWeek,
        GraphDateRange::LastMonth,
        GraphDateRange::LastYear,
    ]
    .into_iter()
    .map(|range| DropdownOption::new(range.id(), graph_date_range_label(range)))
    .collect::<Vec<_>>();
    let cur_filter_d = graph.filter.clone();
    let weak_date = weak.clone();
    top_row = top_row.child(
        Dropdown::new("git-graph-date")
            .size(DropdownSize::Sm)
            .width(px(132.0))
            .leading_icon(IconName::Calendar)
            .placeholder(t!("App.Git.graph_date"))
            .value(graph.filter.date_range.id())
            .options(date_options)
            .on_change(move |value, _, cx| {
                let next = GraphFilter {
                    date_range: GraphDateRange::from_id(value.as_ref())
                        .unwrap_or(GraphDateRange::All),
                    ..cur_filter_d.clone()
                };
                let _ = weak_date.update(cx, |app, cx| {
                    app.set_git_graph_filter(next.clone(), cx);
                });
            }),
    );

    let mut path_options = Vec::with_capacity(graph.files.len() + 1);
    path_options.push(DropdownOption::new("", t!("App.Git.graph_path")));
    path_options.extend(
        graph.files
            .iter()
            .cloned()
            .map(|path| DropdownOption::new(path.clone(), path)),
    );
    let cur_filter_p = graph.filter.clone();
    let weak_path = weak.clone();
    middle_row = middle_row.child(
        Dropdown::new("git-graph-path")
            .size(DropdownSize::Sm)
            .width(px(168.0))
            .leading_icon(IconName::FolderFill)
            .placeholder(t!("App.Git.graph_path"))
            .value(graph.filter.path_filter.clone().unwrap_or_default())
            .options(path_options)
            .on_change(move |value, _, cx| {
                let next = GraphFilter {
                    path_filter: if value.is_empty() {
                        None
                    } else {
                        Some(value.to_string())
                    },
                    ..cur_filter_p.clone()
                };
                let _ = weak_path.update(cx, |app, cx| {
                    app.set_git_graph_filter(next.clone(), cx);
                });
            }),
    );

    // Options toggles — first-parent, no-merges, long edges, sort.
    let weak_fp = weak.clone();
    let cur_fp = graph.filter.clone();
    middle_row = middle_row.child(filter_chip(
        t,
        "git-graph-first-parent",
        IconName::ChartPie,
        t!("App.Git.graph_first_parent"),
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
    middle_row = middle_row.child(filter_chip(
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
    middle_row = middle_row.child(filter_chip(
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
    middle_row = middle_row.child(filter_chip(
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

    middle_row = middle_row.child(div().flex_1().min_w(px(0.0)));

    let highlight_options = [
        GraphHighlightMode::None,
        GraphHighlightMode::MyCommits,
        GraphHighlightMode::MergeCommits,
        GraphHighlightMode::CurrentBranch,
    ]
    .into_iter()
    .map(|mode| DropdownOption::new(mode.id(), graph_highlight_label(mode)))
    .collect::<Vec<_>>();
    let weak_hm = weak.clone();
    middle_row = middle_row.child(
        Dropdown::new("git-graph-highlight")
            .size(DropdownSize::Sm)
            .width(px(128.0))
            .leading_icon(IconName::Eye)
            .placeholder(t!("App.Git.graph_highlight"))
            .value(graph.highlight_mode.id())
            .options(highlight_options)
            .on_change(move |value, _, cx| {
                let next = GraphHighlightMode::from_id(value.as_ref())
                    .unwrap_or(GraphHighlightMode::None);
                let _ = weak_hm.update(cx, |app, cx| {
                    app.set_git_graph_highlight(next, cx);
                });
            }),
    );

    // Zebra toggle
    let weak_zebra = weak.clone();
    bottom_row = bottom_row.child(filter_chip(
        t,
        "git-graph-zebra",
        // SF `rectangle.split.2x1` — Pier's "alternate row stripes" cue.
        IconName::Columns,
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
        bottom_row = bottom_row.child(filter_chip(
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

    toolbar = toolbar.child(top_row).child(middle_row).child(bottom_row);
    toolbar
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
    let hover_bg = if active { bg } else { t.color.bg_hover };
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
        .text_size(SIZE_CAPTION)
        .font_weight(WEIGHT_MEDIUM)
        .text_color(fg)
        .cursor_pointer()
        .border_1()
        .border_color(t.color.border_subtle)
        .hover(move |s| s.bg(hover_bg))
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
    graph: &GraphStateSnapshot,
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
        let inner = row.refs.trim_start_matches(" (").trim_end_matches(')');
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

    let right_click_weak = weak.clone();
    let right_click_hash = hash.clone();
    let mut outer = div()
        .id(row_id)
        .flex()
        .flex_row()
        .items_center()
        .h(px(22.0))
        .w_full()
        .bg(row_bg)
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            let h = click_hash.clone();
            let _ = click_weak.update(cx, |app, cx| {
                app.toggle_git_graph_selected(h, cx);
            });
        })
        .on_mouse_down(MouseButton::Right, move |ev, _, cx| {
            let h = right_click_hash.clone();
            let pos = ev.position;
            let _ = right_click_weak.update(cx, |app, cx| {
                app.open_git_commit_menu(h, pos, cx);
            });
        })
        .child(
            div()
                .flex_none()
                .w(px(col_w))
                .h(px(22.0))
                .child(graph_row_canvas(row, t, col_w, dim_factor)),
        );
    if selected {
        outer = outer.hover(|s| s.bg(t.color.accent_subtle));
    } else {
        outer = outer.hover(|s| s.bg(t.color.bg_hover));
    }

    if graph.show_hash_col {
        outer = outer.child(
            div()
                .flex_none()
                .w(px(graph.hash_col_width))
                .px(SP_1)
                .truncate()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(hash_text_color)
                .child(SharedString::from(row.short_hash.clone())),
        );
        outer = outer.child(graph_column_resize_handle(
            t,
            GraphResizableColumn::Hash,
            weak.clone(),
        ));
    }

    outer = outer.child(
        div()
            .flex_1()
            .min_w(px(240.0))
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

    if graph.show_author_col {
        outer = outer.child(
            div()
                .flex_none()
                .w(px(graph.author_col_width))
                .px(SP_1)
                .truncate()
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(row.author.clone())),
        );
        outer = outer.child(graph_column_resize_handle(
            t,
            GraphResizableColumn::Author,
            weak.clone(),
        ));
    }
    if graph.show_date_col {
        outer = outer.child(
            div()
                .flex_none()
                .w(px(graph.date_col_width))
                .px(SP_1)
                .truncate()
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(format_timestamp(row.date_timestamp))),
        );
        outer = outer.child(graph_column_resize_handle(
            t,
            GraphResizableColumn::Date,
            weak.clone(),
        ));
    }

    outer
}

fn graph_content_min_width(graph_w: f32, graph: &GraphStateSnapshot) -> f32 {
    let mut width = graph_w + 320.0;
    if graph.show_hash_col {
        width += graph.hash_col_width + 6.0;
    }
    if graph.show_author_col {
        width += graph.author_col_width + 6.0;
    }
    if graph.show_date_col {
        width += graph.date_col_width + 6.0;
    }
    width
}

fn graph_date_range_label(range: GraphDateRange) -> SharedString {
    match range {
        GraphDateRange::All => t!("App.Git.graph_date").into(),
        GraphDateRange::Today => t!("App.Git.graph_date_today").into(),
        GraphDateRange::LastWeek => t!("App.Git.graph_date_week").into(),
        GraphDateRange::LastMonth => t!("App.Git.graph_date_month").into(),
        GraphDateRange::LastYear => t!("App.Git.graph_date_year").into(),
    }
}

fn graph_highlight_label(mode: GraphHighlightMode) -> SharedString {
    match mode {
        GraphHighlightMode::None => t!("App.Git.graph_highlight_none").into(),
        GraphHighlightMode::MyCommits => t!("App.Git.graph_highlight_my").into(),
        GraphHighlightMode::MergeCommits => t!("App.Git.graph_highlight_merge").into(),
        GraphHighlightMode::CurrentBranch => t!("App.Git.graph_highlight_branch").into(),
    }
}

fn graph_column_resize_handle(
    t: &crate::theme::Theme,
    column: GraphResizableColumn,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(6.0))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_col_resize()
        .hover(|s| s.bg(t.color.bg_hover))
        .child(div().w(px(1.0)).h_full().bg(t.color.border_subtle))
        .on_mouse_down(MouseButton::Left, move |ev, _, cx| {
            cx.stop_propagation();
            let mouse_x = ev.position.x.to_f64() as f32;
            let _ = weak.update(cx, |app, cx| {
                app.begin_git_graph_column_drag(column, mouse_x, cx);
            });
        })
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
        GraphHighlightMode::MergeCommits => {
            row.parents.split(' ').filter(|s| !s.is_empty()).count() < 2
        }
        GraphHighlightMode::CurrentBranch => row.color_index != 0,
    }
}

/// Thin wrapper for callers that already have the outer
/// `GitSnapshot` — extracts the two pieces (unpushed set + parent
/// hash) that the inline detail needs.
fn commit_detail_strip(
    t: &crate::theme::Theme,
    detail: &CommitDetail,
    snap: &GitSnapshot,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    let parent = snap
        .graph
        .rows
        .iter()
        .find(|r| r.hash == detail.hash)
        .and_then(|r| r.parents.split(' ').next().map(|s| s.to_string()));
    commit_detail_strip_standalone(t, detail, &snap.graph.unpushed, parent, weak)
}

fn commit_detail_strip_standalone(
    t: &crate::theme::Theme,
    detail: &CommitDetail,
    unpushed: &std::collections::HashSet<String>,
    parent: Option<String>,
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
    let is_unpushed = unpushed.contains(&detail.hash);

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
                    t!("App.Git.detail_files_changed", count = detail.files.len()).to_string(),
                )),
        );
        for (i, f) in detail.files.iter().enumerate().take(64) {
            let hash_for_file = detail.hash.clone();
            let short_for_file = detail.short_hash.clone();
            let path_for_file = f.path.clone();
            let file_weak = weak.clone();
            detail_col = detail_col.child(
                div()
                    .id(("git-detail-file", i))
                    .flex()
                    .flex_row()
                    .gap(SP_1)
                    .px(SP_3)
                    .py(SP_0_5)
                    .items_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(t.color.bg_hover))
                    .on_click(move |_, win, cx| {
                        let hash = hash_for_file.clone();
                        let path = path_for_file.clone();
                        let short = short_for_file.clone();
                        let title = format!("{short}  {path}");
                        // Pull the client off the state, run the
                        // diff synchronously (a single-file diff is
                        // fast, typically < 50 ms), then pop the
                        // dialog. If the client disappeared between
                        // clicks we silently no-op.
                        let Some(app_entity) = file_weak.upgrade() else {
                            return;
                        };
                        let client = app_entity.read(cx).git_state().read(cx).client.clone();
                        let Some(client) = client else {
                            return;
                        };
                        let text = match client.commit_file_diff(&hash, &path) {
                            Ok(t) => t,
                            Err(e) => format!("(diff failed: {e})"),
                        };
                        git_dialogs::open_commit_file_diff_dialog(win, cx, title, text);
                    })
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
                                app.schedule_git_action(
                                    GitPendingAction::UndoCommit { hash: h },
                                    cx,
                                );
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

// ─── Tab: Managers (legacy — replaced by manager_overlay, retained
//     with #[allow(dead_code)] until fully removed) ─────────────────

#[allow(dead_code)]
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
            "git-fb-managers-err",
            IconName::WarningFill,
            err,
            true,
            |_, _, _| {},
        ));
    }

    let body = match snap.manager_tab {
        ManagerTab::Branches => {
            branches_manager(t, &snap.managers.branches, weak.clone()).into_any_element()
        }
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

#[allow(dead_code)]
fn managers_tab_strip(active: ManagerTab, weak: WeakEntity<PierApp>) -> impl IntoElement {
    let mut tabs = Tabs::new().segmented();
    for tab in ManagerTab::icons() {
        let is_active = tab == active;
        // Keep these glyphs in lockstep with the trigger-icon list
        // ~2600 lines above — both are user-facing references to the
        // same manager dialog, so they must read identically.
        let (label_key, icon) = match tab {
            ManagerTab::Branches => ("App.Git.mgr_branches", IconName::GitBranch),
            ManagerTab::Tags => ("App.Git.mgr_tags", IconName::TagFill),
            ManagerTab::Remotes => ("App.Git.mgr_remotes", IconName::Globe),
            ManagerTab::Config => ("App.Git.mgr_config", IconName::GearFill),
            ManagerTab::Submodules => ("App.Git.mgr_submodules", IconName::Container),
            ManagerTab::Rebase => ("App.Git.mgr_rebase", IconName::ArrowsCounterClockwise),
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
fn text_input_line(t: &crate::theme::Theme, value: impl Into<SharedString>) -> impl IntoElement {
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

/// Alias exported to `git_dialogs.rs` so the branches dialog can
/// reuse the same row renderer without peeking into private helpers.
pub fn branch_mgr_row_export(
    t: &crate::theme::Theme,
    b: &BranchEntry,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    branch_mgr_row(t, b, weak)
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
        .id(ElementId::Name(format!("git-mgr-branch-{name_id}").into()))
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
                    app.schedule_git_action(
                        GitPendingAction::BranchDelete { name, force: false },
                        cx,
                    );
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
                    app.schedule_git_action(
                        GitPendingAction::BranchDelete { name, force: true },
                        cx,
                    );
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

pub fn tag_row_export(
    t: &crate::theme::Theme,
    tag: &TagInfo,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    tag_row(t, tag, weak)
}

fn tag_row(t: &crate::theme::Theme, tag: &TagInfo, weak: WeakEntity<PierApp>) -> impl IntoElement {
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
        div().flex().flex_row().gap(SP_1).px(SP_3).py(SP_1_5).child(
            Button::secondary("git-remote-fetch-all", t!("App.Git.remote_fetch_all"))
                .size(ButtonSize::Sm)
                .on_click(move |_, _, cx| {
                    let _ = fetch_all_weak.update(cx, |app, cx| {
                        app.schedule_git_action(GitPendingAction::RemoteFetch { name: None }, cx);
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

pub fn remote_row_export(
    t: &crate::theme::Theme,
    r: &RemoteInfo,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    remote_row(t, r, weak)
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
                            app.schedule_git_action(GitPendingAction::RemoteRemove { name }, cx);
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

pub fn config_row_export(
    t: &crate::theme::Theme,
    i: usize,
    e: &ConfigEntry,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    config_row(t, i, e, weak)
}

/// User identity top strip (user.name + user.email) extracted so
/// the config dialog can render it above the entry list.
pub fn config_user_strip(t: &crate::theme::Theme, mgrs: &ManagersSnapshot) -> impl IntoElement {
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
        )
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
        div().flex().flex_row().gap(SP_1).px(SP_3).py(SP_1_5).child(
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

pub fn submodule_row_export(
    t: &crate::theme::Theme,
    i: usize,
    s: &SubmoduleInfo,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    submodule_row(t, i, s, weak)
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

pub fn rebase_manager_export(
    t: &crate::theme::Theme,
    weak: WeakEntity<PierApp>,
) -> impl IntoElement {
    rebase_manager(t, weak)
}

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
        .child(text_input_line(t, t!("App.Git.rebase_onto_placeholder")))
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
        div().flex().flex_row().gap(SP_1).px(SP_3).py(SP_1_5).child(
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
            Button::secondary(
                ("git-conflict-ours", i),
                t!("App.Git.conflicts_resolve_ours"),
            )
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
            Button::primary(
                ("git-conflict-mark", i),
                t!("App.Git.conflicts_mark_resolved"),
            )
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
    manager_panel_open: Option<ManagerTab>,
    manager_tab: ManagerTab,
    graph: GraphStateSnapshot,
    commit_detail: CommitDetailSnapshot,
    managers: ManagersSnapshot,
    footer_height: f32,
    footer_dragging: bool,
    commit_action_mode: CommitActionMode,
}

#[derive(Clone)]
struct GraphStateSnapshot {
    rows: Vec<GraphRow>,
    unpushed: std::collections::HashSet<String>,
    branches: Vec<String>,
    authors: Vec<String>,
    files: Vec<String>,
    filter: GraphFilter,
    has_more: bool,
    loading: bool,
    error: Option<SharedString>,
    selected: Option<String>,
    show_hash_col: bool,
    show_author_col: bool,
    show_date_col: bool,
    hash_col_width: f32,
    author_col_width: f32,
    date_col_width: f32,
    zebra_stripes: bool,
    highlight_mode: GraphHighlightMode,
    user_name: String,
    list_state: ListState,
    column_dragging: bool,
}

#[derive(Clone)]
struct CommitDetailSnapshot {
    detail: Option<CommitDetail>,
    loading: bool,
    error: Option<SharedString>,
}

#[derive(Clone, Default)]
pub struct ManagersSnapshot {
    pub branches: Vec<BranchEntry>,
    pub tags: Vec<TagInfo>,
    pub remotes: Vec<RemoteInfo>,
    pub config: Vec<ConfigEntry>,
    pub submodules: Vec<SubmoduleInfo>,
    pub conflicts: Vec<String>,
    pub user_name: String,
    pub user_email: String,
    pub loading: bool,
    pub error: Option<SharedString>,
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
            manager_panel_open: state.manager_panel_open,
            manager_tab: state.manager_tab,
            graph: GraphStateSnapshot {
                rows: state.graph.rows.clone(),
                unpushed: state.graph.unpushed.clone(),
                branches: state.graph.branches.clone(),
                authors: state.graph.authors.clone(),
                files: state.graph.files.clone(),
                filter: state.graph.filter.clone(),
                has_more: state.graph.has_more,
                loading: state.graph.loading,
                error: state.graph.error.clone(),
                selected: state.graph.selected.clone(),
                show_hash_col: state.graph.show_hash_col,
                show_author_col: state.graph.show_author_col,
                show_date_col: state.graph.show_date_col,
                hash_col_width: state.graph.hash_col_width,
                author_col_width: state.graph.author_col_width,
                date_col_width: state.graph.date_col_width,
                zebra_stripes: state.graph.zebra_stripes,
                highlight_mode: state.graph.highlight_mode,
                user_name: state.managers.user_name.clone(),
                list_state: state.graph.list_state.clone(),
                column_dragging: state.graph.column_drag.is_some(),
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
            footer_height: state.footer_height,
            footer_dragging: state.footer_drag.is_some(),
            commit_action_mode: state.commit_action_mode,
        }
    }
}
