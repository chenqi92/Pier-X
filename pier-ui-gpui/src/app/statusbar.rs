//! App status rail. Mirrors Swift Pier more closely:
//! active session / shell on the left, quiet metadata on the right.

use gpui::{div, prelude::*, px, IntoElement};
use rust_i18n::t;

use crate::app::PierApp;
use crate::components::text;
use crate::theme::{
    heights::STATUSBAR_H,
    radius::RADIUS_PILL,
    spacing::{SP_2, SP_3},
    theme,
    typography::SIZE_CAPTION,
};

pub fn render(app: &PierApp, cx: &gpui::App) -> impl IntoElement {
    let t = theme(cx);
    let term_count = app.terminals_len();
    let active_summary = app.active_terminal_summary(cx);
    let version_label = format!("gpui {}", env!("CARGO_PKG_VERSION"));
    let position_label = (term_count > 0).then(|| {
        let active_idx = app.active_terminal().unwrap_or(0);
        t!(
            "App.Shell.terminal_position",
            current = active_idx.saturating_add(1),
            total = term_count
        )
        .to_string()
    });

    let mut row = div()
        .h(STATUSBAR_H)
        .px(SP_3)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .bg(t.color.bg_panel)
        .border_t_1()
        .border_color(t.color.border_subtle);

    if let Some((label, is_ssh)) = active_summary {
        row = row
            .child(
                div()
                    .w(px(5.0))
                    .h(px(5.0))
                    .rounded(RADIUS_PILL)
                    .bg(if is_ssh {
                        t.color.status_success
                    } else {
                        t.color.accent
                    }),
            )
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .text_color(t.color.text_secondary)
                    .when(is_ssh, |this| this.font_family(t.font_mono.clone()))
                    .child(label),
            );
    } else {
        row = row.child(text::caption(t!("App.Shell.no_terminal")).secondary());
    }

    let row = row.child(div().flex_1());
    let row = if let Some(position) = position_label {
        row.child(text::caption(position).secondary())
    } else {
        row
    };

    row.child(
        div()
            .text_size(SIZE_CAPTION)
            .font_family(t.font_mono.clone())
            .text_color(t.color.text_tertiary)
            .child(version_label),
    )
}
