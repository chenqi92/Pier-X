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

use gpui::{div, prelude::*, px, ClickEvent, Context, IntoElement, MouseButton, SharedString};
use gpui_component::{Icon as UiIcon, IconName};

use crate::app::PierApp;
use crate::components::{IconButton, IconButtonSize};
use crate::theme::{
    heights::{GLYPH_SM, TOOLBAR_H},
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_MONO_SMALL, SIZE_UI_LABEL, WEIGHT_MEDIUM},
    ThemeMode,
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

    let session_name = app.active_session_name(cx);
    let session_endpoint = app.active_session_endpoint(cx);

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
        // Spacer — center chip only renders when remote. Keeping the
        // chip's column as `flex_1` means local mode just gets a
        // breathing gap, not an empty slot with weird framing.
        .child(session_chip(t, session_name, session_endpoint))
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

fn session_chip(
    t: &crate::theme::Theme,
    name: Option<SharedString>,
    endpoint: Option<SharedString>,
) -> impl IntoElement {
    let mut wrap = div().flex_1().min_w(px(0.0)).flex().flex_row().items_center();
    let Some(name) = name else {
        // Local mode: keep the center column blank so the toolbar
        // reads as a clean icon rail.
        return wrap;
    };

    let mut chip = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .px(SP_2)
        .py(SP_1)
        .rounded(RADIUS_SM)
        .bg(t.color.accent_subtle)
        .child(
            UiIcon::new(IconName::Globe)
                .size(GLYPH_SM)
                .text_color(t.color.accent),
        )
        .child(
            div()
                .flex_none()
                .text_size(SIZE_UI_LABEL)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child(name),
        );
    if let Some(endpoint) = endpoint {
        chip = chip.child(
            div()
                .flex_none()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_tertiary)
                .child(endpoint),
        );
    }
    wrap = wrap.child(chip);
    wrap
}
