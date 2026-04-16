use gpui::{div, prelude::*, AnyElement, IntoElement, ParentElement, Pixels, Window};

use crate::theme::{radius::RADIUS_MD, spacing::SP_4, theme};

#[derive(IntoElement)]
pub struct Card {
    children: Vec<AnyElement>,
    padding: Pixels,
}

impl Card {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            padding: SP_4,
        }
    }

    pub fn padding(mut self, p: Pixels) -> Self {
        self.padding = p;
        self
    }
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for Card {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Card {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        div()
            .flex()
            .flex_col()
            .p(self.padding)
            .bg(t.color.bg_surface)
            .border_1()
            .border_color(t.color.border_subtle)
            .rounded(RADIUS_MD)
            .children(self.children)
    }
}
