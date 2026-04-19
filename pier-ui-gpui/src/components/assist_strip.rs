#![allow(dead_code)]

//! Low-weight contextual strip rendered directly below a `PageHeader`.
//!
//! Carries services / tunnels / SSH endpoint hints — ambient information
//! that anchors the page content but should feel clearly subordinate to
//! the header. Background is `bg_panel` (or transparent with a subtle
//! bottom rule), the label is a tiny eyebrow (`SectionLabel`), and any
//! chips inside use `StatusPill` or `MetaLine`.

use gpui::{div, prelude::*, AnyElement, IntoElement, ParentElement, Window};

use crate::theme::{
    heights::ASSIST_STRIP_H,
    spacing::{SP_2, SP_3},
    theme,
};

#[derive(IntoElement)]
pub struct AssistStrip {
    children: Vec<AnyElement>,
}

impl AssistStrip {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = AnyElement>) -> Self {
        self.children.extend(children);
        self
    }
}

impl Default for AssistStrip {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for AssistStrip {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for AssistStrip {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        div()
            .w_full()
            .h(ASSIST_STRIP_H)
            .px(SP_3)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .bg(t.color.bg_panel)
            .border_b_1()
            .border_color(t.color.border_subtle)
            .children(self.children)
    }
}
