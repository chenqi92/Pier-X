//! App toolbar rail.
//!
//! Single-row layout:
//!
//! 1. **Leading** — IconButton rail: left-pane toggle, new-tab chooser.
//! 2. **Center** — session context chip. Shown only when there is an
//!    active remote session (so "本地" mode keeps the toolbar quiet).
//!    The chip is a single row: icon + strong session name +
//!    mono-caption endpoint. The window title already carries the
//!    workspace path; we don't duplicate it here.
//! 3. **Trailing** — right pane toggle, settings, theme.

use gpui::{div, prelude::*, ClickEvent, Context, IntoElement, MouseButton};
use gpui_component::IconName;

use crate::app::PierApp;
use crate::components::{IconButton, IconButtonSize};
use crate::theme::{
    heights::TOOLBAR_H,
    radius::RADIUS_MD,
    spacing::{SP_0_5, SP_1, SP_3},
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

    div()
        .h(TOOLBAR_H)
        .px(SP_3)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_3)
        .bg(t.color.bg_panel)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(
            div()
                .px(SP_1)
                .py(SP_0_5)
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_1)
                .rounded(RADIUS_MD)
                .bg(t.color.bg_surface)
                .border_1()
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
                ),
        )
        .child(div().flex_1())
        .child(
            div()
                .px(SP_1)
                .py(SP_0_5)
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_1)
                .rounded(RADIUS_MD)
                .bg(t.color.bg_surface)
                .border_1()
                .border_color(t.color.border_subtle)
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
                ),
        )
        .on_mouse_down(MouseButton::Left, |_, window, _| {
            window.prevent_default();
        })
}
