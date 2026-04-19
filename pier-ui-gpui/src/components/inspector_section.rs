#![allow(dead_code)]

//! Flush-edge section for the right-panel inspector grammar.
//!
//! An `InspectorSection` is **not** a rounded card: it draws a
//! 28px-tall title strip (icon + small-caps label + optional actions
//! on the right), a 1px hairline under the title, and then renders its
//! children with zero padding. Sections stack vertically against each
//! other so the whole panel reads as one connected 1-pixel grid (see
//! the reference trading app right column).
//!
//! Expected use:
//!
//! ```ignore
//! div().w_full().flex_col().bg(t.color.bg_surface)
//!     .child(InspectorSection::new("监控 · host").icon(IconName::ChartPie)
//!         .child(data_cell_row(vec![cpu_cell, memory_cell, disk_cell])))
//!     .child(Separator::horizontal())
//!     .child(InspectorSection::new("负载")
//!         .child(PropertyRow::new("load-1m", "1 分钟").value(...))
//!         .child(PropertyRow::new("load-5m", "5 分钟").value(...)))
//! ```

use gpui::{
    div, prelude::*, AnyElement, IntoElement, ParentElement, SharedString, Window,
};
use gpui_component::{Icon as UiIcon, IconName};

use crate::components::{separator::Separator, text};
use crate::theme::{
    heights::{ICON_SM, INSPECTOR_HEADER_H},
    spacing::{SP_2, SP_3},
    theme,
};

#[derive(IntoElement)]
pub struct InspectorSection {
    title: SharedString,
    icon: Option<IconName>,
    actions: Option<AnyElement>,
    eyebrow: Option<SharedString>,
    children: Vec<AnyElement>,
    divider: bool,
}

impl InspectorSection {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            icon: None,
            actions: None,
            eyebrow: None,
            children: Vec::new(),
            divider: true,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Put a small right-aligned control cluster inside the title bar
    /// (typically a single IconButton like Refresh, or a pill cluster).
    pub fn actions(mut self, actions: impl IntoElement) -> Self {
        self.actions = Some(actions.into_any_element());
        self
    }

    /// Secondary caption shown after the title (e.g. "· host" or the
    /// active command for a Logs section).
    pub fn eyebrow(mut self, text: impl Into<SharedString>) -> Self {
        self.eyebrow = Some(text.into());
        self
    }

    /// Suppress the 1px hairline under the title. Default is on.
    pub fn no_divider(mut self) -> Self {
        self.divider = false;
        self
    }
}

impl ParentElement for InspectorSection {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for InspectorSection {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);

        let mut title_row = div()
            .w_full()
            .h(INSPECTOR_HEADER_H)
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .px(SP_3)
            .text_color(t.color.text_tertiary);

        if let Some(icon) = self.icon {
            title_row =
                title_row.child(div().flex_none().child(UiIcon::new(icon).size(ICON_SM)));
        }
        title_row = title_row.child(text::caption(self.title).secondary());
        if let Some(eyebrow) = self.eyebrow {
            title_row = title_row.child(
                div()
                    .flex_1()
                    .min_w(gpui::px(0.0))
                    .overflow_hidden()
                    .child(text::caption(eyebrow).secondary().truncate()),
            );
        } else {
            title_row = title_row.child(div().flex_1().min_w(gpui::px(0.0)));
        }
        if let Some(actions) = self.actions {
            title_row = title_row.child(div().flex_none().child(actions));
        }

        let mut section = div()
            .w_full()
            .flex_none()
            .flex()
            .flex_col()
            .child(title_row);
        if self.divider {
            section = section.child(Separator::horizontal());
        }
        section.children(self.children)
    }
}
