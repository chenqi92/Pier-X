use gpui::{div, prelude::*, AnyElement, IntoElement, ParentElement, Pixels, Window};

use crate::theme::{radius::RADIUS_MD, spacing::SP_4, theme};

/// Vertical card container. Defaults to `padding = SP_4` and *no*
/// inter-child gap — call `.gap(SP_X)` to have the card space its
/// own children instead of forcing every view to wrap them in an
/// extra `flex_col().gap(...)` shell.
#[derive(IntoElement)]
pub struct Card {
    children: Vec<AnyElement>,
    padding: Pixels,
    gap: Option<Pixels>,
}

impl Card {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            padding: SP_4,
            gap: None,
        }
    }

    pub fn padding(mut self, p: Pixels) -> Self {
        self.padding = p;
        self
    }

    /// Apply a fixed gap between direct children of the card. Use
    /// this instead of wrapping your children in another `flex_col`.
    pub fn gap(mut self, g: Pixels) -> Self {
        self.gap = Some(g);
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
        let mut el = div()
            .flex()
            .flex_col()
            .p(self.padding)
            .bg(t.color.bg_surface)
            .border_1()
            .border_color(t.color.border_subtle)
            .rounded(RADIUS_MD);
        if let Some(gap) = self.gap {
            el = el.gap(gap);
        }
        el.children(self.children)
    }
}
