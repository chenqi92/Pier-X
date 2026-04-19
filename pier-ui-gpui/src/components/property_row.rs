#![allow(dead_code)]

//! Compact label : value row for inspector-style panels.
//!
//! Left column is a caption-sized label (text.tertiary); right column
//! is arbitrary content — a plain string, a `text::mono` endpoint, a
//! `StatusPill`, or a `Button`. Rows sit flush against each other; put
//! a `Separator::horizontal()` between groups when needed.

use gpui::{
    div, prelude::*, px, AnyElement, ElementId, IntoElement, ParentElement, Pixels, SharedString,
    Window,
};
use gpui_component::{Icon as UiIcon, IconName};

use crate::components::text;
use crate::theme::{
    heights::{ICON_SM, INSPECTOR_ROW_H, ROW_MD_H, ROW_SM_H},
    spacing::{SP_1_5, SP_2, SP_3},
    theme,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PropertyRowVariant {
    /// 22px — default inspector density (matches SKILL.md §4 "列表项高度 24px").
    Tight,
    /// 24px — a hair taller for rows with a button / pill on the right.
    Default,
    /// 28px — matches navigation rows, use when the label needs to
    /// breathe or the value has a multi-segment pill cluster.
    Wide,
}

#[derive(IntoElement)]
pub struct PropertyRow {
    id: ElementId,
    label: SharedString,
    value: Option<AnyElement>,
    icon: Option<IconName>,
    variant: PropertyRowVariant,
    label_width: Pixels,
}

impl PropertyRow {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            value: None,
            icon: None,
            variant: PropertyRowVariant::Tight,
            label_width: px(120.0),
        }
    }

    pub fn value(mut self, value: impl IntoElement) -> Self {
        self.value = Some(value.into_any_element());
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn tight(mut self) -> Self {
        self.variant = PropertyRowVariant::Tight;
        self
    }

    pub fn default_height(mut self) -> Self {
        self.variant = PropertyRowVariant::Default;
        self
    }

    pub fn wide(mut self) -> Self {
        self.variant = PropertyRowVariant::Wide;
        self
    }

    pub fn label_width(mut self, width: Pixels) -> Self {
        self.label_width = width;
        self
    }
}

impl RenderOnce for PropertyRow {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let row_h = match self.variant {
            PropertyRowVariant::Tight => INSPECTOR_ROW_H,
            PropertyRowVariant::Default => ROW_SM_H,
            PropertyRowVariant::Wide => ROW_MD_H,
        };

        let mut label_col = div()
            .flex_none()
            .w(self.label_width)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1_5)
            .text_color(t.color.text_tertiary);

        if let Some(icon) = self.icon {
            label_col = label_col.child(div().flex_none().child(UiIcon::new(icon).size(ICON_SM)));
        }
        label_col = label_col.child(text::caption(self.label).secondary().truncate());

        let mut row = div()
            .id(self.id)
            .w_full()
            .h(row_h)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_3)
            .px(SP_2)
            .child(label_col);

        if let Some(value) = self.value {
            row = row.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .overflow_hidden()
                    .child(value),
            );
        } else {
            row = row.child(div().flex_1().min_w(px(0.0)));
        }

        row
    }
}
