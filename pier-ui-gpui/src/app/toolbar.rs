//! App toolbar rail.
//!
//! Lives above everything else. Three columns:
//!
//! 1. **Leading** — IconButton rail for opening/collapsing the left pane
//!    and for quick actions (new tab).
//! 2. **Center** — `SessionContext`: the *name* of what the user is
//!    currently looking at (active SSH session or "Local"), paired with
//!    a mono workspace path beneath. This is the one piece of text that
//!    tells the user where they are, which the previous toolbar buried
//!    inside a mono caption-only path string.
//! 3. **Trailing** — IconButton rail for the right pane toggle,
//!    settings, theme.
//!
//! Pills that used to live in the status bar (theme mode, right-mode
//! name) were absorbed here: theme is expressed by the sun/moon icon
//! button, and the right-mode context is displayed in the right-panel
//! PageHeader itself — both resolving to "statusbar has fewer things
//! shouting at you."

use gpui::{div, prelude::*, ClickEvent, Context, IntoElement, MouseButton, SharedString};
use gpui_component::IconName;
use rust_i18n::t;

use crate::app::PierApp;
use crate::components::{text, IconButton, IconButtonSize, MetaLine};
use crate::theme::{
    heights::TOOLBAR_H,
    spacing::{SP_2, SP_3},
    theme, ThemeMode,
};
use crate::views::left_panel_view::icons as toolbar_icons;

/// Render the top application rail. Called from `PierApp::render`.
pub fn render(app: &PierApp, cx: &mut Context<PierApp>) -> impl IntoElement {
    let t = theme(cx);
    let toggle_left_icon = if app.left_visible() {
        toolbar_icons::TOGGLE_LEFT_OPEN
    } else {
        toolbar_icons::TOGGLE_LEFT_CLOSED
    };
    let toggle_right_icon = if app.right_visible() {
        toolbar_icons::TOGGLE_RIGHT_OPEN
    } else {
        toolbar_icons::TOGGLE_RIGHT_CLOSED
    };
    let theme_icon = if t.mode == ThemeMode::Dark {
        toolbar_icons::SUN
    } else {
        toolbar_icons::MOON
    };

    let session_label: SharedString = match app.active_session_name(cx) {
        Some(name) => name,
        None => t!("App.Common.local").into(),
    };
    let workspace_label = app.workspace_path();

    div()
        .h(TOOLBAR_H)
        .px(SP_3)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .bg(t.color.bg_panel)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(
            IconButton::new("tb-toggle-left", toggle_left_icon)
                .size(IconButtonSize::Sm)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_left_pane(cx);
                })),
        )
        .child(
            IconButton::new("tb-new-tab", toolbar_icons::NEW_TAB)
                .size(IconButtonSize::Sm)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.open_new_tab_chooser(window, cx);
                })),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_start()
                .justify_center()
                .child(text::ui_label(session_label))
                .child(MetaLine::new(workspace_label).tertiary()),
        )
        .child(
            IconButton::new("tb-toggle-right", toggle_right_icon)
                .size(IconButtonSize::Sm)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_right_pane(cx);
                })),
        )
        .child(
            IconButton::new("tb-open-settings", IconName::Settings)
                .size(IconButtonSize::Sm)
                .on_click(|_: &ClickEvent, window, app| {
                    log::info!("toolbar: open settings dialog");
                    crate::views::settings_dialog::open(window, app);
                }),
        )
        .child(
            IconButton::new("tb-toggle-theme", theme_icon)
                .size(IconButtonSize::Sm)
                .on_click(|_: &ClickEvent, _window, app| {
                    crate::theme::toggle(app);
                    crate::ui_kit::sync_theme(app);
                }),
        )
        .on_mouse_down(MouseButton::Left, |_, window, _| {
            window.prevent_default();
        })
}
