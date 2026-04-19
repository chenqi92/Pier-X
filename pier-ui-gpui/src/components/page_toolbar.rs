#![allow(dead_code)]

//! One-row action bar. Nothing about it is specific to a page header —
//! it can be embedded inside `PageHeader` as the trailing slot or used
//! alone for "search + action + sort" style strips (e.g. SFTP browser).
//!
//! Two slots:
//! - `leading` — flows to the left (search field, filter toggles)
//! - `trailing` — flows to the right (primary/secondary/danger buttons)
//!
//! Leave a slot empty by simply never calling its `child()` / `children()`
//! builder.

use gpui::{div, prelude::*, AnyElement, IntoElement, Window};

use crate::theme::spacing::{SP_1, SP_2};

#[derive(IntoElement)]
pub struct PageToolbar {
    leading: Vec<AnyElement>,
    trailing: Vec<AnyElement>,
}

impl PageToolbar {
    pub fn new() -> Self {
        Self {
            leading: Vec::new(),
            trailing: Vec::new(),
        }
    }

    pub fn leading(mut self, child: impl IntoElement) -> Self {
        self.leading.push(child.into_any_element());
        self
    }

    pub fn trailing(mut self, child: impl IntoElement) -> Self {
        self.trailing.push(child.into_any_element());
        self
    }

    pub fn trailing_many(mut self, children: impl IntoIterator<Item = AnyElement>) -> Self {
        self.trailing.extend(children);
        self
    }
}

impl Default for PageToolbar {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for PageToolbar {
    fn render(self, _: &mut Window, _cx: &mut gpui::App) -> impl IntoElement {
        let leading_box = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1)
            .children(self.leading);
        let trailing_box = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .children(self.trailing);

        div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .gap(SP_2)
            .child(leading_box)
            .child(div().flex_1())
            .child(trailing_box)
    }
}
