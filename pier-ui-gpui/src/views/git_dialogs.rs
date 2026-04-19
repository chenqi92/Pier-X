//! Git manager modals — Pier's branch-row icons pop a floating
//! dialog instead of an inline overlay panel. Each `open_*_dialog`
//! is called from a branch-row icon click, builds a scrollable
//! content body from the existing manager panel helper, and
//! dispatches it via `window.open_dialog(...)`.
//!
//! The actions themselves still dispatch through the shared
//! `PierApp::schedule_git_action` pipeline — the dialog is purely
//! a UI vessel.

use std::sync::Arc;

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

/// Edit message dialog — only meaningful on HEAD (commit --amend).
/// Pre-fills the current commit message; Save dispatches
/// `schedule_git_commit_amend`.
pub fn open_edit_message_dialog(
    window: &mut Window,
    cx: &mut App,
    app: WeakEntity<PierApp>,
    short_hash: String,
    current_message: String,
) {
    let title: SharedString = format!("{}  {}", t!("App.Git.ctx_edit_message"), short_hash).into();
    let placeholder: SharedString = t!("App.Git.commit_placeholder").to_string().into();
    let input = cx.new(|c| {
        gpui_component::input::InputState::new(window, c)
            .multi_line(true)
            .placeholder(placeholder)
    });
    input.update(cx, |s, c| s.set_value(current_message, window, c));
    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = edit_message_dialog_body(app_cx, &input);
        let ok_input = input.clone();
        let ok_weak = app.clone();
        dialog
            .title(title.clone())
            .w(px(640.0))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .confirm()
            .button_props(
                gpui_component::dialog::DialogButtonProps::default()
                    .ok_text(t!("App.Git.ctx_edit_save").to_string())
                    .cancel_text(t!("App.Common.cancel").to_string()),
            )
            .on_ok(move |_, _w, app_cx| {
                let text = ok_input.read(app_cx).value().to_string();
                if text.trim().is_empty() {
                    return true;
                }
                let _ = ok_weak.update(app_cx, |this, cx| {
                    this.schedule_git_commit_amend(text, cx);
                });
                true
            })
            .child(body)
    });
}

fn edit_message_dialog_body(
    cx: &mut App,
    input: &gpui::Entity<gpui_component::input::InputState>,
) -> gpui::AnyElement {
    let t = theme(cx).clone();
    div()
        .w_full()
        .min_h(px(220.0))
        .px(SP_3)
        .py(SP_2)
        .bg(t.color.bg_panel)
        .child(
            crate::components::InlineInput::new(input)
                .tone(crate::components::InlineInputTone::Inset),
        )
        .into_any_element()
}

/// Per-file commit diff dialog. Caller has already loaded the
/// unified diff text; this thin wrapper pops it inside the shared
/// Pier-style `DiffDialog` renderer.
pub fn open_commit_file_diff_dialog(
    window: &mut Window,
    cx: &mut App,
    title_text: String,
    diff_text: String,
) {
    open_pier_diff_dialog(window, cx, title_text, diff_text);
}

/// Full-screen Pier-style diff dialog. Parses the unified diff
/// once up front, keeps the parsed view-state on the shared
/// [`DiffDialogState`] entity, and re-renders each frame from
/// there — gives the inline / side-by-side toggle persistence.
pub fn open_pier_diff_dialog(
    window: &mut Window,
    cx: &mut App,
    title_text: String,
    diff_text: String,
) {
    let parsed = parse_unified_diff(&diff_text);
    let title: SharedString = title_text.into();
    let state = cx.new(|_| DiffDialogState {
        mode: DiffDisplayMode::Inline,
        lines: Arc::new(parsed),
    });
    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = render_diff_dialog_body(title.clone(), state.clone(), app_cx);
        dialog
            .title("")
            .w(px(1000.0))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(body)
    });
}

// ─── Pier-style diff data model ─────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffDisplayMode {
    Inline,
    SideBySide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiffLineKind {
    Context,
    Addition,
    Deletion,
    Header,
}

#[derive(Clone)]
struct ParsedDiffLine {
    text: String,
    kind: DiffLineKind,
    old_ln: Option<u32>,
    new_ln: Option<u32>,
}

pub struct DiffDialogState {
    mode: DiffDisplayMode,
    lines: Arc<Vec<ParsedDiffLine>>,
}

fn parse_unified_diff(raw: &str) -> Vec<ParsedDiffLine> {
    let mut out = Vec::new();
    let mut old_ln: u32 = 0;
    let mut new_ln: u32 = 0;
    for line in raw.split('\n') {
        if line.starts_with("@@") {
            // @@ -a,b +c,d @@
            let mut parts = line.split_whitespace();
            parts.next(); // @@
            if let (Some(old_part), Some(new_part)) = (parts.next(), parts.next()) {
                old_ln = old_part
                    .trim_start_matches('-')
                    .split(',')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                new_ln = new_part
                    .trim_start_matches('+')
                    .split(',')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            }
            out.push(ParsedDiffLine {
                text: line.to_string(),
                kind: DiffLineKind::Header,
                old_ln: None,
                new_ln: None,
            });
        } else if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ")
            || line.starts_with("index ")
            || line.starts_with("new file mode")
            || line.starts_with("deleted file mode")
        {
            // File-header lines are omitted from the rendered body
            // — the dialog title already shows the path.
            continue;
        } else if let Some(rest) = line.strip_prefix('+') {
            out.push(ParsedDiffLine {
                text: rest.to_string(),
                kind: DiffLineKind::Addition,
                old_ln: None,
                new_ln: Some(new_ln),
            });
            new_ln += 1;
        } else if let Some(rest) = line.strip_prefix('-') {
            out.push(ParsedDiffLine {
                text: rest.to_string(),
                kind: DiffLineKind::Deletion,
                old_ln: Some(old_ln),
                new_ln: None,
            });
            old_ln += 1;
        } else if let Some(rest) = line.strip_prefix(' ') {
            out.push(ParsedDiffLine {
                text: rest.to_string(),
                kind: DiffLineKind::Context,
                old_ln: Some(old_ln),
                new_ln: Some(new_ln),
            });
            old_ln += 1;
            new_ln += 1;
        }
    }
    out
}

fn render_diff_dialog_body(
    title: SharedString,
    state: gpui::Entity<DiffDialogState>,
    cx: &mut App,
) -> gpui::AnyElement {
    let t = theme(cx).clone();
    let snap = state.read(cx);
    let mode = snap.mode;
    let lines = snap.lines.clone();

    // +/- counts (Pier's header stat).
    let additions = lines
        .iter()
        .filter(|l| l.kind == DiffLineKind::Addition)
        .count();
    let deletions = lines
        .iter()
        .filter(|l| l.kind == DiffLineKind::Deletion)
        .count();

    // Picker: [Inline | SideBySide] — styled as Pier's segmented
    // control. Active segment is the accent-filled one.
    let state_inline = state.clone();
    let state_side = state.clone();
    let picker = div()
        .flex_none()
        .flex()
        .flex_row()
        .rounded(crate::theme::radius::RADIUS_SM)
        .bg(t.color.bg_panel)
        .border_1()
        .border_color(t.color.border_subtle)
        .child(diff_mode_segment(
            &t,
            "diff-dlg-inline",
            gpui_component::IconName::ListBullets,
            mode == DiffDisplayMode::Inline,
            move |_, _, cx| {
                state_inline.update(cx, |s, cx| {
                    s.mode = DiffDisplayMode::Inline;
                    cx.notify();
                });
            },
        ))
        .child(diff_mode_segment(
            &t,
            "diff-dlg-side",
            gpui_component::IconName::Stack,
            mode == DiffDisplayMode::SideBySide,
            move |_, _, cx| {
                state_side.update(cx, |s, cx| {
                    s.mode = DiffDisplayMode::SideBySide;
                    cx.notify();
                });
            },
        ));

    let header = div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(crate::theme::spacing::SP_2)
        .px(crate::theme::spacing::SP_3)
        .py(crate::theme::spacing::SP_1_5)
        .child(
            div()
                .flex_none()
                .text_color(t.color.status_warning)
                .child(gpui_component::Icon::new(gpui_component::IconName::FileText)
                    .size(crate::theme::heights::ICON_SM)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(crate::theme::typography::SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .font_weight(crate::theme::typography::WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child(title),
        )
        .child(
            div()
                .flex_none()
                .text_size(crate::theme::typography::SIZE_CAPTION)
                .font_family(t.font_mono.clone())
                .text_color(t.color.status_success)
                .child(SharedString::from(format!("+{additions}"))),
        )
        .child(
            div()
                .flex_none()
                .text_size(crate::theme::typography::SIZE_CAPTION)
                .font_family(t.font_mono.clone())
                .text_color(t.color.status_error)
                .child(SharedString::from(format!("-{deletions}"))),
        )
        .child(picker);

    let body: gpui::AnyElement = if lines.is_empty() {
        div()
            .w_full()
            .min_h(px(400.0))
            .flex()
            .items_center()
            .justify_center()
            .child(
                crate::components::text::caption(t!("App.Git.diff_empty")).secondary(),
            )
            .into_any_element()
    } else {
        match mode {
            DiffDisplayMode::Inline => render_inline_diff(&t, lines.clone()).into_any_element(),
            DiffDisplayMode::SideBySide => {
                render_side_by_side_diff(&t, lines.clone()).into_any_element()
            }
        }
    };

    div()
        .w_full()
        .flex()
        .flex_col()
        .child(header)
        .child(crate::components::Separator::horizontal())
        .child(body)
        .into_any_element()
}

fn diff_mode_segment(
    t: &crate::theme::Theme,
    id: impl Into<gpui::ElementId>,
    icon: gpui_component::IconName,
    active: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (bg, fg) = if active {
        (t.color.accent, t.color.text_inverse)
    } else {
        (gpui::Rgba::default(), t.color.text_secondary)
    };
    div()
        .id(id.into())
        .h(crate::theme::heights::BUTTON_SM_H)
        .w(px(30.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(bg)
        .text_color(fg)
        .cursor_pointer()
        .hover({
            let hover_bg = if active {
                t.color.accent_hover
            } else {
                t.color.bg_hover
            };
            move |s| s.bg(hover_bg)
        })
        .child(gpui_component::Icon::new(icon).size(crate::theme::heights::ICON_SM))
        .on_click(on_click)
}

fn render_inline_diff(
    t: &crate::theme::Theme,
    lines: Arc<Vec<ParsedDiffLine>>,
) -> impl IntoElement {
    let mut col = div()
        .id("git-diff-dlg-inline")
        .w_full()
        .max_h(px(520.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .bg(t.color.bg_canvas);
    for (i, line) in lines.iter().enumerate().take(5000) {
        col = col.child(render_inline_diff_line(t, i, line));
    }
    col
}

fn render_inline_diff_line(
    t: &crate::theme::Theme,
    index: usize,
    line: &ParsedDiffLine,
) -> impl IntoElement {
    let (fg, bg, gutter_char): (gpui::Rgba, gpui::Rgba, &str) = match line.kind {
        DiffLineKind::Addition => (
            t.color.status_success,
            diff_bg(t, DiffLineKind::Addition),
            "+",
        ),
        DiffLineKind::Deletion => (
            t.color.status_error,
            diff_bg(t, DiffLineKind::Deletion),
            "-",
        ),
        DiffLineKind::Header => (t.color.status_info, diff_bg(t, DiffLineKind::Header), "@"),
        DiffLineKind::Context => (t.color.text_primary, gpui::Rgba::default(), " "),
    };
    let old_num: SharedString = line
        .old_ln
        .map(|n| SharedString::from(n.to_string()))
        .unwrap_or_default();
    let new_num: SharedString = line
        .new_ln
        .map(|n| SharedString::from(n.to_string()))
        .unwrap_or_default();
    div()
        .id(("git-diff-ln", index))
        .flex()
        .flex_row()
        .w_full()
        .bg(bg)
        .text_size(crate::theme::typography::SIZE_MONO_SMALL)
        .font_family(t.font_mono.clone())
        .child(
            div()
                .flex_none()
                .w(px(40.0))
                .pr(crate::theme::spacing::SP_1)
                .text_size(crate::theme::typography::SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(old_num),
        )
        .child(
            div()
                .flex_none()
                .w(px(40.0))
                .pr(crate::theme::spacing::SP_1)
                .text_size(crate::theme::typography::SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(new_num),
        )
        .child(
            div()
                .flex_none()
                .w(px(16.0))
                .text_color(fg)
                .font_weight(crate::theme::typography::WEIGHT_EMPHASIS)
                .child(SharedString::from(gutter_char.to_string())),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_color(fg)
                .child(SharedString::from(if line.text.is_empty() {
                    " ".to_string()
                } else {
                    line.text.clone()
                })),
        )
}

fn render_side_by_side_diff(
    t: &crate::theme::Theme,
    lines: Arc<Vec<ParsedDiffLine>>,
) -> impl IntoElement {
    let (left, right) = split_side_by_side(&lines);
    let left_col = {
        let mut col = div()
            .id("git-diff-dlg-side-left")
            .flex_1()
            .max_h(px(520.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .bg(t.color.bg_canvas);
        for (i, line) in left.iter().enumerate().take(5000) {
            col = col.child(render_side_line(t, ("git-diff-l", i), line, /*is_left=*/ true));
        }
        col
    };
    let right_col = {
        let mut col = div()
            .id("git-diff-dlg-side-right")
            .flex_1()
            .max_h(px(520.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .bg(t.color.bg_canvas);
        for (i, line) in right.iter().enumerate().take(5000) {
            col = col.child(render_side_line(t, ("git-diff-r", i), line, /*is_left=*/ false));
        }
        col
    };
    div()
        .w_full()
        .flex()
        .flex_row()
        .child(left_col)
        .child(
            div()
                .w(px(1.0))
                .bg(t.color.border_subtle),
        )
        .child(right_col)
}

fn split_side_by_side(
    lines: &[ParsedDiffLine],
) -> (Vec<ParsedDiffLine>, Vec<ParsedDiffLine>) {
    let mut left: Vec<ParsedDiffLine> = Vec::with_capacity(lines.len());
    let mut right: Vec<ParsedDiffLine> = Vec::with_capacity(lines.len());
    let mut pending_del: Vec<ParsedDiffLine> = Vec::new();
    let mut pending_add: Vec<ParsedDiffLine> = Vec::new();
    let blank = || ParsedDiffLine {
        text: String::new(),
        kind: DiffLineKind::Context,
        old_ln: None,
        new_ln: None,
    };
    let flush = |left: &mut Vec<ParsedDiffLine>,
                 right: &mut Vec<ParsedDiffLine>,
                 pd: &mut Vec<ParsedDiffLine>,
                 pa: &mut Vec<ParsedDiffLine>| {
        let n = pd.len().max(pa.len());
        for i in 0..n {
            left.push(pd.get(i).cloned().unwrap_or_else(blank));
            right.push(pa.get(i).cloned().unwrap_or_else(blank));
        }
        pd.clear();
        pa.clear();
    };
    for line in lines.iter() {
        match line.kind {
            DiffLineKind::Header | DiffLineKind::Context => {
                flush(&mut left, &mut right, &mut pending_del, &mut pending_add);
                left.push(line.clone());
                right.push(line.clone());
            }
            DiffLineKind::Deletion => {
                if !pending_add.is_empty() {
                    flush(&mut left, &mut right, &mut pending_del, &mut pending_add);
                }
                pending_del.push(line.clone());
            }
            DiffLineKind::Addition => pending_add.push(line.clone()),
        }
    }
    flush(&mut left, &mut right, &mut pending_del, &mut pending_add);
    (left, right)
}

fn render_side_line(
    t: &crate::theme::Theme,
    id: impl Into<gpui::ElementId>,
    line: &ParsedDiffLine,
    is_left: bool,
) -> impl IntoElement {
    let (fg, bg) = match line.kind {
        DiffLineKind::Addition => (t.color.status_success, diff_bg(t, DiffLineKind::Addition)),
        DiffLineKind::Deletion => (t.color.status_error, diff_bg(t, DiffLineKind::Deletion)),
        DiffLineKind::Header => (t.color.status_info, diff_bg(t, DiffLineKind::Header)),
        DiffLineKind::Context => (t.color.text_primary, gpui::Rgba::default()),
    };
    let num: SharedString = if is_left {
        line.old_ln
            .map(|n| SharedString::from(n.to_string()))
            .unwrap_or_default()
    } else {
        line.new_ln
            .map(|n| SharedString::from(n.to_string()))
            .unwrap_or_default()
    };
    div()
        .id(id.into())
        .flex()
        .flex_row()
        .w_full()
        .bg(bg)
        .text_size(crate::theme::typography::SIZE_MONO_SMALL)
        .font_family(t.font_mono.clone())
        .child(
            div()
                .flex_none()
                .w(px(40.0))
                .pr(crate::theme::spacing::SP_1)
                .text_size(crate::theme::typography::SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(num),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .px(crate::theme::spacing::SP_1)
                .text_color(fg)
                .child(SharedString::from(if line.text.is_empty() {
                    " ".to_string()
                } else {
                    line.text.clone()
                })),
        )
}

/// Line-background tint for a diff line — matches Pier's
/// `Color.green/red/cyan.opacity(0.08)` (headers use 0.05).
fn diff_bg(t: &crate::theme::Theme, kind: DiffLineKind) -> gpui::Rgba {
    match kind {
        DiffLineKind::Addition => gpui::Rgba {
            a: 0.08,
            ..t.color.status_success
        },
        DiffLineKind::Deletion => gpui::Rgba {
            a: 0.08,
            ..t.color.status_error
        },
        DiffLineKind::Header => gpui::Rgba {
            a: 0.05,
            ..t.color.status_info
        },
        DiffLineKind::Context => gpui::Rgba::default(),
    }
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
        div().flex().flex_row().px(SP_3).py(SP_2).child(
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
