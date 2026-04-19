#![allow(dead_code)]

//! Horizontal tab row. Collects `TabItem`s — each an (id, label, optional
//! icon, active flag, click handler) tuple — and lays them out as a
//! single row with a bottom rule.
//!
//! This unifies three hand-written tab strips: left panel Files/Servers,
//! terminal session tabs, and any other place where "pick one of a small
//! list" is the interaction. Call sites build `TabItem` instances and
//! feed them via `Tabs::new(...).items(...)`.
//!
//! The API intentionally keeps click handlers per-item (as boxed
//! closures) so each tab can close over caller-specific state without
//! the component having to know anything about its parent type.

use gpui::{
    div, prelude::*, App, ClickEvent, ElementId, IntoElement, SharedString, Window,
};
use gpui_component::{Icon as UiIcon, IconName};

use crate::theme::{
    heights::{ICON_SM, ROW_SM_H},
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3},
    theme, ui_font_with,
    typography::{SIZE_UI_LABEL, WEIGHT_MEDIUM},
};

pub struct TabItem {
    pub id: ElementId,
    pub label: SharedString,
    pub icon: Option<IconName>,
    pub active: bool,
    pub on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
}

impl TabItem {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        active: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            active,
            on_click: Box::new(on_click),
        }
    }

    pub fn with_icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }
}

#[derive(IntoElement)]
pub struct Tabs {
    items: Vec<TabItem>,
}

impl Tabs {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn item(mut self, item: TabItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = TabItem>) -> Self {
        self.items.extend(items);
        self
    }
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for Tabs {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let active_color = t.color.accent;
        let idle_color = t.color.text_secondary;
        let hover_bg = t.color.bg_hover;
        let hover_fg = t.color.text_primary;

        let mut row = div()
            .w_full()
            .h(ROW_SM_H)
            .px(SP_2)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1)
            .bg(t.color.bg_surface)
            .border_b_1()
            .border_color(t.color.border_subtle);

        for item in self.items {
            let fg = if item.active { active_color } else { idle_color };
            let mut el = div()
                .id(item.id)
                .h(ROW_SM_H)
                .px(SP_3)
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_1_5)
                .rounded(RADIUS_SM)
                .text_size(SIZE_UI_LABEL)
                .text_color(fg)
                .font(ui_font_with(&t.font_ui, &t.font_ui_features, WEIGHT_MEDIUM))
                .cursor_pointer()
                .hover(move |s| s.bg(hover_bg).text_color(hover_fg));

            if let Some(icon) = item.icon {
                el = el.child(UiIcon::new(icon).size(ICON_SM).text_color(fg));
            }
            el = el.child(item.label);
            el = el.on_click(move |ev, win, cx| (item.on_click)(ev, win, cx));

            row = row.child(el);
        }

        row
    }
}
