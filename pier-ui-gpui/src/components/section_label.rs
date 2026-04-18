use gpui::{div, prelude::*, px, IntoElement, SharedString, Window};
use gpui_component::{Icon as UiIcon, IconName};

use crate::theme::{
    spacing::SP_2,
    theme,
    typography::{SIZE_SMALL, WEIGHT_MEDIUM},
};

#[derive(IntoElement)]
pub struct SectionLabel {
    text: SharedString,
    centered: bool,
    icon: Option<IconName>,
}

impl SectionLabel {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            centered: false,
            icon: None,
        }
    }

    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }

    /// Prepend a small icon accent before the label text. The icon inherits
    /// the tertiary text color so it stays subtle — it's a visual anchor,
    /// not a status cue.
    pub fn with_icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }
}

impl RenderOnce for SectionLabel {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let label = div()
            .text_size(SIZE_SMALL)
            .font_weight(WEIGHT_MEDIUM)
            .font_family(t.font_ui.clone())
            .text_color(t.color.text_tertiary)
            .child(self.text);

        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .text_color(t.color.text_tertiary);
        if self.centered {
            row = row.justify_center();
        }
        if let Some(icon) = self.icon {
            row = row.child(UiIcon::new(icon).size(px(14.0)));
        }
        row.child(label)
    }
}
