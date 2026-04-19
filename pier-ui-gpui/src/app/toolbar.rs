//! App toolbar rail.
//!
//! Single-row layout:
//!
//! 1. **Leading** — icon-only controls: left-pane toggle, new-tab chooser.
//! 2. **Center** — session chip. Only shown when there is an active
//!    remote session, keeping local-only work quiet like the Swift shell.
//! 3. **Trailing** — right-pane toggle, settings, theme.

use gpui::{
    div, prelude::*, px, ClickEvent, Context, IntoElement, MouseButton, Rgba, SharedString,
};
use gpui_component::IconName;
use pier_core::ssh::SshConfig;

use crate::app::ssh_session::ConnectStatus;
use crate::app::PierApp;
use crate::components::{IconButton, IconButtonSize};
use crate::theme::{
    heights::TOOLBAR_H,
    radius::{RADIUS_MD, RADIUS_PILL},
    spacing::{SP_0_5, SP_1, SP_1_5, SP_2},
    theme,
    typography::{SIZE_CAPTION, SIZE_UI_LABEL, WEIGHT_MEDIUM},
    ui_font_with, ThemeMode,
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
    let session_chip = app.active_session_ref().map(|session_entity| {
        let session = session_entity.read(cx);
        let title = toolbar_session_title(&session.config);
        let endpoint = toolbar_session_endpoint(&session.config);
        let dot_color = toolbar_session_color(session.status, t);

        div()
            .px(SP_2)
            .py(SP_0_5)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1_5)
            .rounded(RADIUS_MD)
            .bg(t.color.bg_surface)
            .border_1()
            .border_color(t.color.border_subtle)
            .child(
                div()
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded(RADIUS_PILL)
                    .bg(dot_color),
            )
            .child(
                div()
                    .text_size(SIZE_UI_LABEL)
                    .font(ui_font_with(&t.font_ui, &t.font_ui_features, WEIGHT_MEDIUM))
                    .text_color(t.color.text_primary)
                    .child(title),
            )
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .font_family(t.font_mono.clone())
                    .text_color(t.color.text_tertiary)
                    .child(endpoint),
            )
            .into_any_element()
    });

    let mut row = div()
        .h(TOOLBAR_H)
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
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
        );

    if let Some(chip) = session_chip {
        row = row.child(div().flex_1()).child(chip).child(div().flex_1());
    } else {
        row = row.child(div().flex_1());
    }

    row.child(
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

fn toolbar_session_title(config: &SshConfig) -> SharedString {
    if config.name.trim().is_empty() {
        config.host.clone().into()
    } else {
        config.name.clone().into()
    }
}

fn toolbar_session_endpoint(config: &SshConfig) -> SharedString {
    if config.port == 22 {
        format!("{}@{}", config.user, config.host).into()
    } else {
        format!("{}@{}:{}", config.user, config.host, config.port).into()
    }
}

fn toolbar_session_color(status: ConnectStatus, t: &crate::theme::Theme) -> Rgba {
    match status {
        ConnectStatus::Connected => t.color.status_success,
        ConnectStatus::Connecting | ConnectStatus::Refreshing => t.color.accent,
        ConnectStatus::Failed => t.color.status_error,
        ConnectStatus::Idle => t.color.text_tertiary,
    }
}
