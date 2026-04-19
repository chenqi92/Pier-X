use gpui::{
    div, prelude::*, px, AnyElement, IntoElement, ParentElement, Pixels, SharedString, Window,
};

use crate::components::text;
use crate::theme::spacing::{SP_0_5, SP_2, SP_4};

#[derive(IntoElement)]
pub struct SettingRow {
    title: SharedString,
    description: Option<SharedString>,
    title_width: Pixels,
    align_top: bool,
    controls: Vec<AnyElement>,
}

impl SettingRow {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            description: None,
            title_width: px(216.0),
            align_top: false,
            controls: Vec::new(),
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn align_top(mut self) -> Self {
        self.align_top = true;
        self
    }

    #[allow(dead_code)]
    pub fn title_width(mut self, width: Pixels) -> Self {
        self.title_width = width;
        self
    }
}

impl ParentElement for SettingRow {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.controls.extend(elements);
    }
}

impl RenderOnce for SettingRow {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        let mut summary = div()
            .w(self.title_width)
            .flex_none()
            .flex()
            .flex_col()
            .gap(SP_0_5)
            .child(text::ui_label(self.title));

        if let Some(description) = self.description {
            summary = summary.child(text::caption(description).secondary());
        }

        let controls = div()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_row()
            .child(div().flex_1().min_w(px(0.0)))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(SP_2)
                    .when(self.align_top, |el| el.items_start())
                    .when(!self.align_top, |el| el.items_center())
                    .children(self.controls),
            );

        div()
            .w_full()
            .flex()
            .flex_row()
            .gap(SP_4)
            .when(self.align_top, |el| el.items_start())
            .when(!self.align_top, |el| el.items_center())
            .child(summary)
            .child(controls)
    }
}
