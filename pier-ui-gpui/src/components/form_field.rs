use gpui::{
    div, prelude::*, AnyElement, IntoElement, ParentElement, RenderOnce, SharedString, Window,
};

use crate::components::text;
use crate::theme::{
    radius::RADIUS_MD,
    spacing::{SP_0_5, SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_CAPTION, WEIGHT_MEDIUM},
};

/// Vertical form primitive used by modal sheets and compact editors.
///
/// Unlike [`crate::components::SettingRow`], this keeps labels above
/// controls to match the Swift reference's compact `Form` / sheet
/// rhythm for connection editors and small dialogs.
#[derive(IntoElement, Default)]
pub struct FormField {
    label: Option<SharedString>,
    description: Option<SharedString>,
    help: Option<SharedString>,
    children: Vec<AnyElement>,
}

impl FormField {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: Some(label.into()),
            description: None,
            help: None,
            children: Vec::new(),
        }
    }

    pub fn unlabeled() -> Self {
        Self {
            label: None,
            description: None,
            help: None,
            children: Vec::new(),
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn help(mut self, help: impl Into<SharedString>) -> Self {
        self.help = Some(help.into());
        self
    }
}

impl ParentElement for FormField {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for FormField {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);

        let mut col = div().w_full().flex().flex_col().gap(SP_1);

        if self.label.is_some() || self.description.is_some() {
            let mut header = div().w_full().flex().flex_col().gap(SP_0_5);
            if let Some(label) = self.label {
                header = header.child(
                    div()
                        .text_size(SIZE_CAPTION)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_secondary)
                        .child(label),
                );
            }
            if let Some(description) = self.description {
                header = header.child(text::small(description).secondary());
            }
            col = col.child(header);
        }

        col = col.child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(SP_1_5)
                .children(self.children),
        );

        if let Some(help) = self.help {
            col = col.child(text::small(help).secondary());
        }

        col
    }
}

#[derive(IntoElement, Default)]
pub struct FormSection {
    title: Option<SharedString>,
    description: Option<SharedString>,
    children: Vec<AnyElement>,
}

impl FormSection {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: Some(title.into()),
            description: None,
            children: Vec::new(),
        }
    }

    pub fn untitled() -> Self {
        Self {
            title: None,
            description: None,
            children: Vec::new(),
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
}

impl ParentElement for FormSection {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for FormSection {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);

        let mut outer = div().w_full().flex().flex_col().gap(SP_2);

        if self.title.is_some() || self.description.is_some() {
            let mut header = div().w_full().flex().flex_col().gap(SP_1);
            if let Some(title) = self.title {
                header = header.child(
                    div()
                        .text_size(SIZE_CAPTION)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_secondary)
                        .child(title),
                );
            }
            if let Some(description) = self.description {
                header = header.child(text::small(description).secondary());
            }
            outer = outer.child(header);
        }

        outer.child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(SP_3)
                .p(SP_3)
                .rounded(RADIUS_MD)
                .bg(t.color.bg_panel)
                .border_1()
                .border_color(t.color.border_subtle)
                .children(self.children),
        )
    }
}
