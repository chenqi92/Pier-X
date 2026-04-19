//! Horizontal wrap-container for a collection of pills (or any small
//! flex_none chips). Exists because the hand-written version
//!
//! ```ignore
//! div().flex_row().flex_wrap().gap(SP_2).children(pills)
//! ```
//!
//! silently drops pills that don't fit — GPUI's flex_wrap needs the
//! container to have `min_w(0)` and `overflow_hidden` to compute a
//! correct wrap point. Without them a single wide pill squeezes the
//! row and pushes neighboring pills into the adjacent column (the
//! "MySQL pill spilling onto the git rail" bug). This component bakes
//! in those four lines so callers can stop remembering them.

use gpui::{div, prelude::*, px, AnyElement, IntoElement, ParentElement, Window};

use crate::theme::spacing::SP_2;

#[derive(IntoElement)]
pub struct PillCluster {
    children: Vec<AnyElement>,
    gap: gpui::Pixels,
}

impl PillCluster {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            gap: SP_2,
        }
    }

    pub fn gap(mut self, gap: gpui::Pixels) -> Self {
        self.gap = gap;
        self
    }
}

impl Default for PillCluster {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for PillCluster {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for PillCluster {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        div()
            .w_full()
            .min_w(px(0.0))
            .overflow_hidden()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(self.gap)
            .children(self.children)
    }
}
