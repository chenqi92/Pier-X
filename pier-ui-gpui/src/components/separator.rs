use gpui::{div, prelude::*, px, IntoElement, Window};

use crate::theme::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SeparatorAxis {
    Horizontal,
    Vertical,
}

#[derive(IntoElement)]
pub struct Separator {
    axis: SeparatorAxis,
}

impl Separator {
    pub fn horizontal() -> Self {
        Self {
            axis: SeparatorAxis::Horizontal,
        }
    }

    pub fn vertical() -> Self {
        Self {
            axis: SeparatorAxis::Vertical,
        }
    }
}

impl RenderOnce for Separator {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        match self.axis {
            SeparatorAxis::Horizontal => div().w_full().h(px(1.0)).bg(t.color.border_subtle),
            SeparatorAxis::Vertical => div().h_full().w(px(1.0)).bg(t.color.border_subtle),
        }
    }
}
