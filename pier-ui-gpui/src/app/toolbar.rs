//! App toolbar rail.
//!
//! Single-row layout:
//!
//! 1. **Leading** — icon-only controls: left-pane toggle, new-tab chooser.
//! 2. **Center** — empty spacer. Session identity lives in the window
//!    titlebar (above) and per-tab strip (below); showing a chip here
//!    as well was redundant and jittered the toolbar every time a
//!    connection came up.
//! 3. **Trailing** — right-pane toggle, settings, theme.

use gpui::{div, prelude::*, px, ClickEvent, Context, IntoElement, MouseButton};
use gpui_component::IconName;

use crate::app::PierApp;
use crate::components::{IconButton, IconButtonSize};
use crate::theme::{
    heights::TOOLBAR_H,
    spacing::{SP_1, SP_2},
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

    // No session chip in the centre — the title-bar (the host window's
    // native traffic-light area) already shows the active session's
    // identity, and creating a new remote connection used to cause the
    // chip to flash into this row and shove the right-side buttons
    // around. Requested: remove this "duplicated identity" region.

    // Leave room for the macOS traffic-light cluster (window opens with
    // `appears_transparent` + `traffic_light_position = (12, 10)` in
    // `main.rs`). 76 px covers 3×14 px buttons + gaps + right breathing
    // room so the first toolbar button clears them.
    let leading_inset = if cfg!(target_os = "macos") {
        px(76.0)
    } else {
        SP_2
    };

    let row = div()
        .h(TOOLBAR_H)
        .pl(leading_inset)
        .pr(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .bg(t.color.bg_panel)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(
            IconButton::new("tb-toggle-left", toggle_left_icon)
                .size(IconButtonSize::Md)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_left_pane(cx);
                })),
        )
        .child(
            IconButton::new("tb-new-tab", toolbar_icons::NEW_TAB)
                .size(IconButtonSize::Md)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.open_new_tab_chooser(window, cx);
                })),
        )
        .child(div().flex_1());

    row.child(
        IconButton::new("tb-toggle-right", toggle_right_icon)
            .size(IconButtonSize::Md)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_right_pane(cx);
            })),
    )
    .child(
        IconButton::new("tb-open-settings", IconName::Settings)
            .size(IconButtonSize::Md)
            .on_click(|_: &ClickEvent, window, app| {
                log::info!("toolbar: open settings dialog");
                crate::views::settings_dialog::open(window, app);
            }),
    )
    .child(
        IconButton::new("tb-toggle-theme", theme_icon)
            .size(IconButtonSize::Md)
            .on_click(|_: &ClickEvent, _window, app| {
                crate::theme::toggle(app);
                crate::ui_kit::sync_theme(app);
            }),
    )
    .on_mouse_down(MouseButton::Left, |_, window, _| {
        window.prevent_default();
    })
}
