use gpui::{div, prelude::*, IntoElement, SharedString, Window};
use gpui_component::{Icon as UiIcon, IconName};

use crate::theme::{
    heights::ICON_SM,
    spacing::SP_2,
    theme,
    typography::{SIZE_SMALL, WEIGHT_EMPHASIS},
    ui_font_with,
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
        // SIZE_SMALL is 10px in the current type ramp; pair with
        // WEIGHT_EMPHASIS to read like SwiftUI's 10pt semibold section
        // header without shouting.
        let label = div()
            .text_size(SIZE_SMALL)
            .text_color(t.color.text_tertiary)
            .font(ui_font_with(&t.font_ui, &t.font_ui_features, WEIGHT_EMPHASIS))
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
            row = row.child(UiIcon::new(icon).size(ICON_SM));
        }
        row.child(label)
    }
}
