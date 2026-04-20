#![allow(dead_code)]

//! Single-select dropdown. Renders a Pier-X-styled trigger (looks
//! like a [`Button`] with a trailing chevron) and, on click, anchors
//! a popover menu of options below it.
//!
//! Built on top of [`gpui_component::popover::Popover`] — so dismissal
//! (click-outside, `Esc`, item pick) is handled for free. The content
//! menu is rendered with Pier-X theme tokens so it matches the rest
//! of the surface.
//!
//! Use this for "pick one of N" where N is long enough that a
//! [`crate::widgets::SegmentedControl`] would overflow the row, or
//! where the list would read noisily as a chip flex-wrap (N > 5 ish).

use std::rc::Rc;

use gpui::{
    div, prelude::*, px, App, Corner, ElementId, IntoElement, Pixels, SharedString, Styled, Window,
};
use gpui_component::{
    popover::Popover, scroll::ScrollableElement, Icon as UiIcon, IconName, Selectable,
};

use crate::theme::{
    heights::{BUTTON_MD_H, BUTTON_SM_H, GLYPH_SM},
    radius::{RADIUS_MD, RADIUS_SM},
    shadow,
    spacing::{SP_1, SP_2, SP_3},
    theme,
    typography::{SIZE_UI_LABEL, WEIGHT_MEDIUM, WEIGHT_REGULAR},
    ui_font_with,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DropdownSize {
    Sm,
    Md,
}

impl DropdownSize {
    fn height(self) -> Pixels {
        match self {
            Self::Sm => BUTTON_SM_H,
            Self::Md => BUTTON_MD_H,
        }
    }
}

#[derive(Clone)]
pub struct DropdownOption {
    value: SharedString,
    label: SharedString,
}

impl DropdownOption {
    pub fn new(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

type OnChangeCb = Rc<dyn Fn(&SharedString, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Dropdown {
    id: ElementId,
    options: Vec<DropdownOption>,
    value: SharedString,
    placeholder: SharedString,
    leading_icon: Option<IconName>,
    size: DropdownSize,
    width: Option<Pixels>,
    on_change: Option<OnChangeCb>,
}

impl Dropdown {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            options: Vec::new(),
            value: SharedString::default(),
            placeholder: SharedString::default(),
            leading_icon: None,
            size: DropdownSize::Md,
            width: None,
            on_change: None,
        }
    }

    pub fn option(mut self, option: DropdownOption) -> Self {
        self.options.push(option);
        self
    }

    pub fn options(mut self, options: impl IntoIterator<Item = DropdownOption>) -> Self {
        self.options.extend(options);
        self
    }

    pub fn value(mut self, value: impl Into<SharedString>) -> Self {
        self.value = value.into();
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn leading_icon(mut self, icon: IconName) -> Self {
        self.leading_icon = Some(icon);
        self
    }

    pub fn size(mut self, size: DropdownSize) -> Self {
        self.size = size;
        self
    }

    pub fn width(mut self, width: Pixels) -> Self {
        self.width = Some(width);
        self
    }

    pub fn on_change(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Dropdown {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let display_label: SharedString = self
            .options
            .iter()
            .find(|o| o.value == self.value)
            .map(|o| o.label.clone())
            .unwrap_or_else(|| self.placeholder.clone());

        let width = self.width;
        let size = self.size;
        let leading_icon = self.leading_icon;
        let options = self.options.clone();
        let current_value = self.value.clone();
        let on_change = self.on_change.clone();

        let popover_id: SharedString = match &self.id {
            ElementId::Name(n) => SharedString::from(format!("dropdown-{}", n)),
            _ => SharedString::from("dropdown-unnamed"),
        };

        Popover::new(ElementId::Name(popover_id.clone()))
            .anchor(Corner::TopLeft)
            .appearance(false)
            .trigger(DropdownTrigger {
                id: self.id,
                label: display_label,
                selected: false,
                leading_icon,
                size,
                width,
            })
            .content(move |_state, _window, cx| {
                let state_weak = cx.entity().downgrade();
                render_menu(
                    &options,
                    &current_value,
                    on_change.clone(),
                    state_weak,
                    width,
                )
            })
    }
}

// ── Trigger ──────────────────────────────────────────────────

/// The clickable "current value + chevron" chip that opens the
/// popover. Styled like a [`crate::components::Button::secondary`]
/// so it sits comfortably next to buttons in a row.
#[derive(IntoElement)]
pub struct DropdownTrigger {
    id: ElementId,
    label: SharedString,
    selected: bool,
    leading_icon: Option<IconName>,
    size: DropdownSize,
    width: Option<Pixels>,
}

impl Selectable for DropdownTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for DropdownTrigger {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let height = self.size.height();
        let border_color = if self.selected {
            t.color.border_focus
        } else {
            t.color.border_default
        };
        let bg = if self.selected {
            t.color.bg_hover
        } else {
            t.color.bg_surface
        };

        let mut el = div()
            .id(self.id)
            .h(height)
            .px(SP_3)
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(SP_2)
            .rounded(RADIUS_SM)
            .border_1()
            .border_color(border_color)
            .bg(bg)
            .text_size(SIZE_UI_LABEL)
            .text_color(t.color.text_primary)
            .font(ui_font_with(&t.font_ui, &t.font_ui_features, WEIGHT_MEDIUM))
            .cursor_pointer()
            .hover(|s| s.bg(t.color.bg_hover))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(SP_2)
                    .children(self.leading_icon.map(|icon| {
                        UiIcon::new(icon)
                            .size(GLYPH_SM)
                            .text_color(t.color.text_tertiary)
                            .into_any_element()
                    }))
                    .child(div().flex_1().min_w(px(0.0)).truncate().child(self.label)),
            )
            .child(UiIcon::new(IconName::ChevronDown).size(GLYPH_SM));

        if let Some(w) = self.width {
            el = el.w(w);
        } else {
            // Keep a sensible minimum so short labels don't render as
            // a pill that's narrower than the chevron cluster.
            el = el.min_w(px(120.0));
        }
        el
    }
}

// ── Menu content ────────────────────────────────────────────

fn render_menu(
    options: &[DropdownOption],
    current_value: &SharedString,
    on_change: Option<OnChangeCb>,
    state_weak: gpui::WeakEntity<gpui_component::popover::PopoverState>,
    trigger_width: Option<Pixels>,
) -> impl IntoElement {
    // Theme read happens via a child component; we compose pure
    // layout here so we don't need `cx` at this layer.
    DropdownMenu {
        options: options.to_vec(),
        current: current_value.clone(),
        on_change,
        state_weak,
        trigger_width,
    }
}

#[derive(IntoElement)]
struct DropdownMenu {
    options: Vec<DropdownOption>,
    current: SharedString,
    on_change: Option<OnChangeCb>,
    state_weak: gpui::WeakEntity<gpui_component::popover::PopoverState>,
    trigger_width: Option<Pixels>,
}

impl RenderOnce for DropdownMenu {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let mut menu = div()
            .mt(SP_1)
            .flex()
            .flex_col()
            .gap(px(1.0))
            .p(SP_1)
            .max_h(px(280.0))
            .overflow_y_scrollbar()
            .rounded(RADIUS_MD)
            .bg(t.color.bg_elevated)
            .border_1()
            .border_color(t.color.border_subtle)
            .shadow(shadow::popover());

        if let Some(w) = self.trigger_width {
            menu = menu.min_w(w);
        } else {
            menu = menu.min_w(px(180.0));
        }

        for opt in self.options {
            let is_current = opt.value == self.current;
            let value = opt.value.clone();
            let cb = self.on_change.clone();
            let state_weak = self.state_weak.clone();
            let item_id: SharedString = SharedString::from(format!("dd-item-{}", opt.value));

            let fg = if is_current {
                t.color.accent
            } else {
                t.color.text_primary
            };

            let mut item = div()
                .id(ElementId::Name(item_id))
                .h(BUTTON_SM_H)
                .px(SP_2)
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .rounded(RADIUS_SM)
                .text_size(SIZE_UI_LABEL)
                .text_color(fg)
                .font_weight(if is_current {
                    WEIGHT_MEDIUM
                } else {
                    WEIGHT_REGULAR
                })
                .cursor_pointer()
                .hover(|s| s.bg(t.color.bg_hover))
                .on_click(move |_, window, cx| {
                    if let Some(cb) = cb.as_ref() {
                        cb(&value, window, cx);
                    }
                    if let Some(state) = state_weak.upgrade() {
                        state.update(cx, |state, cx| state.dismiss(window, cx));
                    }
                });

            if is_current {
                item = item.child(
                    UiIcon::new(IconName::Check)
                        .size(GLYPH_SM)
                        .text_color(t.color.accent),
                );
            } else {
                // Reserve space where the checkmark would be so both
                // variants align cleanly on the label column.
                item = item.child(div().w(GLYPH_SM).h(GLYPH_SM));
            }

            item = item.child(div().flex_1().min_w(px(0.0)).truncate().child(opt.label));

            menu = menu.child(item);
        }

        menu
    }
}
