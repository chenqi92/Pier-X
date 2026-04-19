#![allow(dead_code)]

//! Single-pixel hairline used to carve an inspector panel into
//! flush-edge sections — no rounded card, no padding, just a rule
//! between rows. Horizontal runs full-width; vertical runs full-height
//! and is typically used between DataCell columns.

use gpui::{div, prelude::*, IntoElement, Window};

use crate::theme::{heights::HAIRLINE, theme};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SeparatorAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SeparatorVariant {
    Subtle,
    Default,
    Strong,
}

#[derive(IntoElement)]
pub struct Separator {
    axis: SeparatorAxis,
    variant: SeparatorVariant,
}

impl Separator {
    pub fn horizontal() -> Self {
        Self {
            axis: SeparatorAxis::Horizontal,
            variant: SeparatorVariant::Subtle,
        }
    }

    pub fn vertical() -> Self {
        Self {
            axis: SeparatorAxis::Vertical,
            variant: SeparatorVariant::Subtle,
        }
    }

    pub fn default_strength(mut self) -> Self {
        self.variant = SeparatorVariant::Default;
        self
    }

    pub fn strong(mut self) -> Self {
        self.variant = SeparatorVariant::Strong;
        self
    }
}

impl RenderOnce for Separator {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let color = match self.variant {
            SeparatorVariant::Subtle => t.color.border_subtle,
            SeparatorVariant::Default => t.color.border_default,
            SeparatorVariant::Strong => t.color.border_strong,
        };
        let mut el = div().flex_none().bg(color);
        el = match self.axis {
            SeparatorAxis::Horizontal => el.w_full().h(HAIRLINE),
            SeparatorAxis::Vertical => el.h_full().w(HAIRLINE),
        };
        el
    }
}
