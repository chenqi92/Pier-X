use gpui::{div, prelude::*, IntoElement, SharedString, Window};

use crate::theme::{
    heights::{PILL_DOT, PILL_H},
    radius::RADIUS_PILL,
    spacing::{SP_1, SP_2},
    theme, ui_font_with,
    typography::{SIZE_CAPTION, WEIGHT_MEDIUM},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Success,
    Warning,
    Error,
    Info,
}

#[derive(IntoElement)]
pub struct StatusPill {
    label: SharedString,
    status: StatusKind,
}

impl StatusPill {
    pub fn new(label: impl Into<SharedString>, status: StatusKind) -> Self {
        Self {
            label: label.into(),
            status,
        }
    }
}

impl RenderOnce for StatusPill {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let dot = match self.status {
            StatusKind::Success => t.color.status_success,
            StatusKind::Warning => t.color.status_warning,
            StatusKind::Error => t.color.status_error,
            StatusKind::Info => t.color.status_info,
        };
        // SKILL.md §9: status pill 高度 18px、左侧 6px dot、12px caption 字。
        // Pill label goes through `.font(...)` (not `.font_family(...)`)
        // so Inter's cv01/ss03 reach the platform shaper — important for
        // caption-sized labels that lean hard on 1/l/I disambiguation.
        div()
            .h(PILL_H)
            .px(SP_2)
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(SP_1)
            .bg(t.color.bg_surface)
            .border_1()
            .border_color(t.color.border_subtle)
            .rounded(RADIUS_PILL)
            .child(
                div()
                    .w(PILL_DOT)
                    .h(PILL_DOT)
                    .rounded(RADIUS_PILL)
                    .bg(dot),
            )
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .text_color(t.color.text_secondary)
                    .font(ui_font_with(&t.font_ui, &t.font_ui_features, WEIGHT_MEDIUM))
                    .child(self.label),
            )
    }
}
