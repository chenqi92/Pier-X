use gpui::{div, prelude::*, IntoElement, SharedString, Window};

use crate::theme::{
    theme,
    typography::{SIZE_SMALL, WEIGHT_MEDIUM},
};

#[derive(IntoElement)]
pub struct SectionLabel {
    text: SharedString,
    centered: bool,
}

impl SectionLabel {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            centered: false,
        }
    }

    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }
}

impl RenderOnce for SectionLabel {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let mut el = div()
            .text_size(SIZE_SMALL)
            .font_weight(WEIGHT_MEDIUM)
            .font_family(t.font_ui.clone())
            .text_color(t.color.text_tertiary)
            .child(self.text);
        if self.centered {
            el = el.text_center();
        }
        el
    }
}
