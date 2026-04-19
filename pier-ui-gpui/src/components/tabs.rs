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

use gpui::{div, prelude::*, App, ClickEvent, ElementId, IntoElement, SharedString, Window};
use gpui_component::{Icon as UiIcon, IconName};

use crate::theme::{
    heights::{GLYPH_XS, TAB_PILL_H},
    radius::RADIUS_MD,
    spacing::{SP_0_5, SP_1, SP_1_5, SP_2},
    theme,
    typography::{SIZE_SMALL, WEIGHT_MEDIUM, WEIGHT_REGULAR},
    ui_font_with,
};

/// Visual treatment for the tab strip.
///
/// - `Subtle` — quiet bg tint when active; the Raycast / Linear pattern.
///   Correct for contextual tabs (terminal session switcher, inspector
///   sub-modes) where the surrounding page already telegraphs what the
///   user is looking at.
/// - `Segmented` — solid accent fill + inverse text for the active tab;
///   matches SwiftUI `Picker(.pickerStyle(.segmented))`. Use for primary
///   mode switches ("Files / Servers", "Containers / Images / Volumes")
///   where the user's choice *is* the page's topic.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TabsVariant {
    Subtle,
    Segmented,
}

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
    variant: TabsVariant,
}

impl Tabs {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            variant: TabsVariant::Subtle,
        }
    }

    /// Promote to `Segmented` — solid accent fill for the active tab.
    pub fn segmented(mut self) -> Self {
        self.variant = TabsVariant::Segmented;
        self
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

        // Subtle: quiet tint; Segmented: solid accent fill for a clear
        // primary-mode-picker feel. Inactive tabs stay flat + lighter
        // weight with only a hover bg across both variants.
        let (active_bg, active_fg) = match self.variant {
            TabsVariant::Subtle => (t.color.accent_subtle, t.color.accent),
            TabsVariant::Segmented => (t.color.accent, t.color.text_inverse),
        };
        let idle_fg = t.color.text_secondary;
        let hover_bg = t.color.bg_hover;
        let hover_fg = t.color.text_primary;

        let mut row = div()
            .w_full()
            .px(SP_2)
            .py(SP_1_5)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_0_5)
            .bg(t.color.bg_panel)
            .border_b_1()
            .border_color(t.color.border_subtle);

        for item in self.items {
            let is_active = item.active;
            let fg = if is_active { active_fg } else { idle_fg };
            let weight = if is_active {
                WEIGHT_MEDIUM
            } else {
                WEIGHT_REGULAR
            };
            let mut el = div()
                .id(item.id)
                .h(TAB_PILL_H)
                .px(SP_2)
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_1)
                .rounded(RADIUS_MD)
                .text_size(SIZE_SMALL)
                .text_color(fg)
                .font(ui_font_with(&t.font_ui, &t.font_ui_features, weight))
                .cursor_pointer();

            if is_active {
                el = el.bg(active_bg);
            } else {
                el = el.hover(move |s| s.bg(hover_bg).text_color(hover_fg));
            }

            if let Some(icon) = item.icon {
                el = el.child(UiIcon::new(icon).size(GLYPH_XS).text_color(fg));
            }
            el = el.child(item.label);
            el = el.on_click(move |ev, win, cx| (item.on_click)(ev, win, cx));

            row = row.child(el);
        }

        row
    }
}
