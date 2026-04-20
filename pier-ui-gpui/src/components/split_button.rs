#![allow(dead_code)]

//! Split button — a primary action joined to a chevron trigger that
//! opens a popover listing alternative actions. Picking an option
//! fires its callback immediately (the caller is expected to remember
//! the choice and swap the primary label on the next render).
//!
//! Matches the Pier "提交 / 提交并推送" composite: one chip with the
//! default action on the left and a chevron on the right to switch
//! between commit-only and commit-and-push.

use std::rc::Rc;

use gpui::{
    div, prelude::*, px, App, ClickEvent, Corner, ElementId, IntoElement, Pixels, Rgba,
    SharedString, Window,
};
use gpui_component::{popover::Popover, tooltip::Tooltip, Icon as UiIcon, IconName, Selectable};

use crate::theme::{
    heights::{BUTTON_MD_H, BUTTON_SM_H, GLYPH_SM, ICON_SM},
    radius::{RADIUS_MD, RADIUS_SM},
    shadow,
    spacing::{SP_1, SP_2, SP_3},
    theme,
    typography::{SIZE_UI_LABEL, WEIGHT_MEDIUM, WEIGHT_REGULAR},
    ui_font_with,
};

use super::button::ButtonSize;

#[derive(Clone)]
pub struct SplitButtonOption {
    value: SharedString,
    label: SharedString,
}

impl SplitButtonOption {
    pub fn new(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

type PrimaryCb = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type PickCb = Rc<dyn Fn(&SharedString, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct SplitButton {
    id: ElementId,
    primary_label: SharedString,
    tooltip: Option<SharedString>,
    current_value: Option<SharedString>,
    options: Vec<SplitButtonOption>,
    disabled: bool,
    size: ButtonSize,
    on_primary_click: Option<PrimaryCb>,
    on_pick: Option<PickCb>,
}

impl SplitButton {
    pub fn new(id: impl Into<ElementId>, primary_label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            primary_label: primary_label.into(),
            tooltip: None,
            current_value: None,
            options: Vec::new(),
            disabled: false,
            size: ButtonSize::Sm,
            on_primary_click: None,
            on_pick: None,
        }
    }

    pub fn option(mut self, option: SplitButtonOption) -> Self {
        self.options.push(option);
        self
    }

    pub fn options(mut self, options: impl IntoIterator<Item = SplitButtonOption>) -> Self {
        self.options.extend(options);
        self
    }

    pub fn current(mut self, value: impl Into<SharedString>) -> Self {
        self.current_value = Some(value.into());
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_primary_click(
        mut self,
        f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_primary_click = Some(Rc::new(f));
        self
    }

    pub fn on_pick(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for SplitButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let height = match self.size {
            ButtonSize::Sm => BUTTON_SM_H,
            ButtonSize::Md => BUTTON_MD_H,
        };
        let px_main = match self.size {
            ButtonSize::Sm => SP_2,
            ButtonSize::Md => SP_3,
        };

        let disabled = self.disabled;
        let (bg, fg, hover_bg) = if disabled {
            (
                gpui::Rgba {
                    a: 0.35,
                    ..t.color.accent
                },
                t.color.text_disabled,
                gpui::Rgba {
                    a: 0.35,
                    ..t.color.accent
                },
            )
        } else {
            (t.color.accent, t.color.text_inverse, t.color.accent_hover)
        };

        let primary_id: SharedString = match &self.id {
            ElementId::Name(n) => SharedString::from(format!("{}-primary", n)),
            _ => SharedString::from("split-btn-primary"),
        };
        let chevron_id: SharedString = match &self.id {
            ElementId::Name(n) => SharedString::from(format!("{}-chevron", n)),
            _ => SharedString::from("split-btn-chevron"),
        };
        let popover_id: SharedString = match &self.id {
            ElementId::Name(n) => SharedString::from(format!("{}-popover", n)),
            _ => SharedString::from("split-btn-popover"),
        };

        let font = ui_font_with(&t.font_ui, &t.font_ui_features, WEIGHT_MEDIUM);
        let primary_cb = self.on_primary_click.clone();
        let tooltip_text = self.tooltip.clone();

        let mut main = div()
            .id(ElementId::Name(primary_id))
            .h(height)
            .px(px_main)
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_l(RADIUS_SM)
            .bg(bg)
            .text_size(SIZE_UI_LABEL)
            .text_color(fg)
            .font(font.clone())
            .child(self.primary_label.clone());

        if !disabled {
            main = main.cursor_pointer().hover(move |s| s.bg(hover_bg));
            if let Some(cb) = primary_cb {
                main = main.on_click(move |ev, win, cx| cb(ev, win, cx));
            }
        }

        if let Some(text) = tooltip_text {
            main = main.tooltip(move |win, cx| Tooltip::new(text.clone()).build(win, cx));
        }

        let seam_color = if disabled {
            gpui::Rgba {
                a: 0.2,
                ..t.color.text_inverse
            }
        } else {
            gpui::Rgba {
                a: 0.25,
                ..t.color.text_inverse
            }
        };
        let seam = div().flex_none().w(px(1.0)).h(height).bg(seam_color);

        let chevron = if disabled {
            ChevronTrigger {
                id: ElementId::Name(chevron_id),
                height,
                bg,
                fg,
                hover_bg,
                disabled: true,
                selected: false,
            }
            .into_any_element()
        } else {
            let options = self.options.clone();
            let current = self.current_value.clone();
            let on_pick = self.on_pick.clone();
            Popover::new(ElementId::Name(popover_id))
                .anchor(Corner::TopRight)
                .appearance(false)
                .trigger(ChevronTrigger {
                    id: ElementId::Name(chevron_id),
                    height,
                    bg,
                    fg,
                    hover_bg,
                    disabled: false,
                    selected: false,
                })
                .content(move |_state, _win, cx| {
                    let state_weak = cx.entity().downgrade();
                    SplitButtonMenu {
                        options: options.clone(),
                        current: current.clone(),
                        on_pick: on_pick.clone(),
                        state_weak,
                    }
                })
                .into_any_element()
        };

        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .child(main)
            .child(seam)
            .child(chevron)
    }
}

// ── Chevron trigger ────────────────────────────────────────────

#[derive(IntoElement)]
struct ChevronTrigger {
    id: ElementId,
    height: Pixels,
    bg: Rgba,
    fg: Rgba,
    hover_bg: Rgba,
    disabled: bool,
    selected: bool,
}

impl Selectable for ChevronTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for ChevronTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut el = div()
            .id(self.id)
            .h(self.height)
            .w(self.height)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_r(RADIUS_SM)
            .bg(self.bg)
            .text_color(self.fg)
            .child(UiIcon::new(IconName::ChevronDown).size(ICON_SM));
        if !self.disabled {
            let hover_bg = self.hover_bg;
            el = el.cursor_pointer().hover(move |s| s.bg(hover_bg));
        }
        el
    }
}

// ── Popover menu ───────────────────────────────────────────────

#[derive(IntoElement)]
struct SplitButtonMenu {
    options: Vec<SplitButtonOption>,
    current: Option<SharedString>,
    on_pick: Option<PickCb>,
    state_weak: gpui::WeakEntity<gpui_component::popover::PopoverState>,
}

impl RenderOnce for SplitButtonMenu {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let mut menu = div()
            .mt(SP_1)
            .flex()
            .flex_col()
            .gap(px(1.0))
            .p(SP_1)
            .rounded(RADIUS_MD)
            .bg(t.color.bg_elevated)
            .border_1()
            .border_color(t.color.border_subtle)
            .shadow(shadow::popover())
            .min_w(px(180.0));

        for opt in self.options {
            let is_current = self
                .current
                .as_ref()
                .map(|c| *c == opt.value)
                .unwrap_or(false);
            let value = opt.value.clone();
            let cb = self.on_pick.clone();
            let state_weak = self.state_weak.clone();
            let item_id: SharedString = SharedString::from(format!("split-item-{}", opt.value));

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
                item = item.child(div().w(GLYPH_SM).h(GLYPH_SM));
            }

            item = item.child(div().flex_1().min_w(px(0.0)).truncate().child(opt.label));

            menu = menu.child(item);
        }

        menu
    }
}
