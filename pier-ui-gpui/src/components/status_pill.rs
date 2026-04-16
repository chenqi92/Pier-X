use gpui::{div, prelude::*, px, IntoElement, SharedString, Window};

use crate::theme::{
    radius::RADIUS_PILL,
    spacing::{SP_1, SP_2},
    theme,
    typography::{SIZE_CAPTION, WEIGHT_MEDIUM},
};

#[derive(Clone, Copy, PartialEq, Eq)]
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
        // SKILL.md §9: status pill 高度 18px、左侧 6px dot、12px caption 字
        div()
            .h(px(18.0))
            .px(SP_2)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1)
            .bg(t.color.bg_surface)
            .border_1()
            .border_color(t.color.border_subtle)
            .rounded(RADIUS_PILL)
            .child(div().w(px(6.0)).h(px(6.0)).rounded(px(3.0)).bg(dot))
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .font_weight(WEIGHT_MEDIUM)
                    .font_family(t.font_ui.clone())
                    .text_color(t.color.text_secondary)
                    .child(self.label),
            )
    }
}
