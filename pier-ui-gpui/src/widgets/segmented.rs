//! Segmented control — horizontal row of equal-width items where
//! exactly one is visually raised. Visually matches the
//! Docker-panel segmented picker from the Swift reference
//! (containers / images / volumes); also useful for any "pick one
//! of three views of the same data" interaction.
//!
//! Not to be confused with [`crate::components::tabs::Tabs`]:
//! `Tabs` is a navigation strip that switches *sections* (left
//! panel Files/Servers, terminal tab bar) and draws a bottom
//! rule; `SegmentedControl` is an in-panel *mode* switch that
//! draws a unified pill with the selected item raised.

use std::rc::Rc;

use gpui::{
    div, prelude::*, px, App, ClickEvent, ElementId, IntoElement, ParentElement, RenderOnce,
    SharedString, Styled, Window,
};

use crate::theme::{
    radius::{RADIUS_MD, RADIUS_SM},
    spacing::{SP_0_5, SP_2, SP_3},
    theme,
    typography::{SIZE_SMALL, WEIGHT_MEDIUM, WEIGHT_REGULAR},
};

pub struct SegmentedItem {
    pub id: ElementId,
    pub label: SharedString,
    pub selected: bool,
    pub on_click: Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
}

impl SegmentedItem {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        selected: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            selected,
            on_click: Rc::new(on_click),
        }
    }
}

#[derive(IntoElement)]
pub struct SegmentedControl {
    items: Vec<SegmentedItem>,
}

impl SegmentedControl {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn item(mut self, item: SegmentedItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = SegmentedItem>) -> Self {
        self.items.extend(items);
        self
    }
}

impl Default for SegmentedControl {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for SegmentedControl {
    fn render(self, _w: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);

        // Track = subtle recessed surface with a hairline border.
        // Selected item = elevated surface tinted by accent so the
        // active segment reads unambiguously at a glance.
        let track_bg = t.color.bg_surface;
        let selected_bg = t.color.bg_panel;
        let selected_fg = t.color.accent;
        let idle_fg = t.color.text_tertiary;
        let hover_fg = t.color.text_primary;

        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_0_5)
            .p(SP_0_5)
            .rounded(RADIUS_MD)
            .bg(track_bg)
            .border_1()
            .border_color(t.color.border_subtle);

        for item in self.items {
            let is_selected = item.selected;
            let fg = if is_selected { selected_fg } else { idle_fg };
            let weight = if is_selected {
                WEIGHT_MEDIUM
            } else {
                WEIGHT_REGULAR
            };
            let on_click = item.on_click.clone();

            let mut seg = div()
                .id(item.id)
                .flex_1()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .px(SP_3)
                .py(px(6.0))
                .rounded(RADIUS_SM)
                .text_size(SIZE_SMALL)
                .text_color(fg)
                .font_weight(weight)
                .cursor_pointer()
                .child(item.label);

            if is_selected {
                seg = seg
                    .bg(selected_bg)
                    .border_1()
                    .border_color(t.color.border_subtle);
            } else {
                seg = seg
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .hover(move |s| s.text_color(hover_fg));
            }
            seg = seg.on_click(move |ev, win, cx| on_click(ev, win, cx));

            row = row.child(seg);
        }

        // Keep the segmented bar at a predictable height so a
        // series of them stacked (rare, but possible) still reads
        // as auxiliary chrome rather than eating vertical rhythm.
        div()
            .h(px(32.0))
            .w_full()
            .flex()
            .items_center()
            .child(row.w_full())
    }
}
