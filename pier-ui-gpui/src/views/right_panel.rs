//! Right panel — 10 mode container + vertical icon sidebar.
//!
//! Mirrors `Pier/PierApp/Sources/Views/RightPanel/RightPanelView.swift`.
//! Pier-X adds Postgres + SQLite to Pier's 8 modes (per "功能只能多不能少").
//!
//! Modes pulled from existing views:
//!   - Git    → [`crate::views::git::GitView`]
//!   - DBs    → [`crate::views::database::DatabaseView`] (one struct, 4 kinds)
//!
//! Phase-1 placeholders (visual + descriptive, no backend yet):
//!   - Markdown / Monitor / SFTP / Docker / Logs

use std::rc::Rc;

use gpui::{div, prelude::*, px, App, IntoElement, SharedString, Window};
use gpui_component::{Icon as UiIcon, IconName};

use crate::app::layout::{RightContext, RightMode, RIGHT_ICON_BAR_W};
use crate::components::{text, Card, SectionLabel, StatusKind, StatusPill};
use crate::data::ShellSnapshot;
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_2, SP_3, SP_4},
    theme,
    typography::{SIZE_CAPTION, SIZE_MONO_SMALL, WEIGHT_MEDIUM},
};
use crate::views::database::DatabaseView;
use crate::views::git::GitView;

pub type ModeSelector = Rc<dyn Fn(&RightMode, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct RightPanel {
    active_mode: RightMode,
    snapshot: ShellSnapshot,
    on_select_mode: ModeSelector,
}

impl RightPanel {
    pub fn new(
        active_mode: RightMode,
        snapshot: ShellSnapshot,
        on_select_mode: ModeSelector,
    ) -> Self {
        Self {
            active_mode,
            snapshot,
            on_select_mode,
        }
    }
}

impl RenderOnce for RightPanel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let RightPanel {
            active_mode,
            snapshot,
            on_select_mode,
        } = self;

        let body = render_mode_body(t, active_mode, snapshot);

        div()
            .h_full()
            .flex()
            .flex_row()
            .bg(t.color.bg_panel)
            .border_l_1()
            .border_color(t.color.border_subtle)
            // Content
            .child(div().flex_1().min_w(px(0.0)).child(body))
            // Vertical separator
            .child(div().w(px(1.0)).h_full().bg(t.color.border_subtle))
            // Icon sidebar (far right)
            .child(render_icon_sidebar(t, active_mode, on_select_mode))
    }
}

fn render_mode_body(
    t: &crate::theme::Theme,
    mode: RightMode,
    snapshot: ShellSnapshot,
) -> gpui::AnyElement {
    // Status bar (above content) — placeholder for the SSH connection
    // status / detected services strips that Pier renders here.
    let status = mode_status_bar(t, mode);

    let content: gpui::AnyElement = match mode {
        RightMode::Markdown => placeholder(
            "Markdown",
            "Local .md preview with live file watching.",
            "Renders the document at the path most recently posted from the file tree (Phase 2). Until then this card stands in for the renderer.",
        )
        .into_any_element(),
        RightMode::Monitor => monitor_view(t, &snapshot).into_any_element(),
        RightMode::Sftp => placeholder(
            "SFTP",
            "Remote file browser over the active SSH session.",
            "Wired through pier_core::ssh::sftp once the multi-tab terminal session ships (Phase 3).",
        )
        .into_any_element(),
        RightMode::Docker => placeholder(
            "Docker",
            "Containers / images / volumes via SSH exec.",
            "Pier-X has the pier_core::services::docker backend; this view binds to it once SSH session multiplexing lands.",
        )
        .into_any_element(),
        RightMode::Git => GitView::new().into_any_element(),
        RightMode::Mysql
        | RightMode::Postgres
        | RightMode::Redis
        | RightMode::Sqlite => DatabaseView::new(mode.db_kind().expect("db mode"))
            .into_any_element(),
        RightMode::Logs => placeholder(
            "Logs",
            "Tail remote log files via SSH exec.",
            "Multi-file watcher + level filtering + JSON formatting land in Phase 4.",
        )
        .into_any_element(),
    };

    div()
        .h_full()
        .flex()
        .flex_col()
        .child(status)
        .child(div().flex_1().min_h(px(0.0)).child(content))
        .into_any_element()
}

fn mode_status_bar(t: &crate::theme::Theme, mode: RightMode) -> impl IntoElement {
    let context_label = match mode.context() {
        RightContext::Local => "local",
        RightContext::Remote => "remote (no session)",
    };
    let kind = match mode.context() {
        RightContext::Local => StatusKind::Success,
        RightContext::Remote => StatusKind::Warning,
    };
    div()
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
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child(mode.label()),
        )
        .child(StatusPill::new(context_label, kind))
}

fn monitor_view(t: &crate::theme::Theme, s: &ShellSnapshot) -> impl IntoElement {
    div()
        .p(SP_4)
        .flex()
        .flex_col()
        .gap(SP_3)
        .child(metric_card(t, "Core", &s.core_version, &s.workspace_path))
        .child(metric_card(t, "Git", &s.git_branch, &s.git_detail))
        .child(metric_card(
            t,
            "Connections",
            &s.connections_value,
            &s.connections_detail,
        ))
        .child(metric_card(
            t,
            "Local machine",
            &s.local_machine_value,
            &s.local_machine_detail,
        ))
        .child(metric_card(t, "Paths", &s.path_value, &s.path_detail))
}

fn metric_card(
    _t: &crate::theme::Theme,
    title: &'static str,
    value: &SharedString,
    detail: &SharedString,
) -> Card {
    Card::new()
        .padding(SP_3)
        .child(SectionLabel::new(title))
        .child(text::body(value.clone()))
        .child(text::body(detail.clone()).secondary())
}

fn placeholder(title: &'static str, headline: &'static str, body: &'static str) -> impl IntoElement {
    div()
        .p(SP_4)
        .flex()
        .flex_col()
        .gap(SP_2)
        .child(SectionLabel::new(title))
        .child(text::body(headline))
        .child(text::body(body).secondary())
}

fn render_icon_sidebar(
    t: &crate::theme::Theme,
    active_mode: RightMode,
    on_select: ModeSelector,
) -> impl IntoElement {
    let mut col = div()
        .w(RIGHT_ICON_BAR_W)
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .gap(SP_1)
        .py(SP_2)
        .bg(t.color.bg_panel);

    for mode in RightMode::ALL {
        col = col.child(mode_icon_button(t, mode, mode == active_mode, on_select.clone()));
    }
    col
}

fn mode_icon_button(
    t: &crate::theme::Theme,
    mode: RightMode,
    is_active: bool,
    on_select: ModeSelector,
) -> impl IntoElement {
    let id_str: SharedString = format!("right-icon-{}", mode.id()).into();
    let icon = mode_icon(mode);

    let mut btn = div()
        .id(gpui::ElementId::Name(id_str))
        .w(px(28.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .cursor_pointer()
        .text_color(if is_active {
            t.color.accent
        } else {
            t.color.text_secondary
        })
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(move |_, w, app| on_select(&mode, w, app))
        .child(icon.size(px(16.0)));

    if is_active {
        btn = btn.bg(t.color.accent_subtle);
    }
    btn
}

fn mode_icon(mode: RightMode) -> UiIcon {
    if let Some(asset) = mode.icon_asset() {
        return UiIcon::empty().path(asset);
    }
    let name = match mode {
        RightMode::Markdown => IconName::File,
        RightMode::Monitor => IconName::LayoutDashboard,
        // The remaining modes provide an asset_icon; this is just the
        // exhaustive arm so the match compiles.
        _ => IconName::Frame,
    };
    UiIcon::new(name)
}

// `Card` debug helper for the placeholder — silences an unused warning when
// the placeholder doesn't need additional metadata cards.
#[allow(dead_code)]
fn _hint_card(t: &crate::theme::Theme, label: &'static str) -> Card {
    Card::new().padding(SP_3).child(
        div()
            .text_size(SIZE_MONO_SMALL)
            .font_family(t.font_mono.clone())
            .text_color(t.color.text_tertiary)
            .child(label),
    )
}
