#![allow(dead_code)]

//! Commit composer — a Pier-style resizable commit pane with a
//! dedicated multi-line editor surface above a detached action row.
//! The editor keeps its own bordered shell; stage-all and commit
//! actions sit below it on the panel background, matching sibling
//! Pier's source-control layout.
//!
//! Callers fill the two slots with fully-constructed elements
//! (typically a [`super::Button`] and a [`super::split_button::SplitButton`]).

use gpui::{
    div, prelude::*, px, AnyElement, App, Entity, Focusable, IntoElement, RenderOnce, Window,
};
use gpui_component::input::{Input, InputState};

use crate::theme::{
    radius::RADIUS_MD,
    spacing::{SP_1, SP_1_5, SP_2},
    theme,
    typography::SIZE_CAPTION,
};

#[derive(IntoElement)]
pub struct CommitComposer {
    state: Entity<InputState>,
    bottom_left: Option<AnyElement>,
    bottom_right: Option<AnyElement>,
}

impl CommitComposer {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            state: state.clone(),
            bottom_left: None,
            bottom_right: None,
        }
    }

    pub fn bottom_left(mut self, el: impl IntoElement) -> Self {
        self.bottom_left = Some(el.into_any_element());
        self
    }

    pub fn bottom_right(mut self, el: impl IntoElement) -> Self {
        self.bottom_right = Some(el.into_any_element());
        self
    }
}

impl RenderOnce for CommitComposer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        let border = if focused {
            t.color.accent
        } else {
            t.color.border_subtle
        };

        let input = Input::new(&self.state)
            .appearance(false)
            .bordered(false)
            .focus_bordered(false)
            .h_full()
            .w_full()
            .text_size(SIZE_CAPTION)
            .text_color(t.color.text_primary);

        let editor = div()
            .w_full()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .px(SP_2)
            .py(SP_1)
            .rounded(RADIUS_MD)
            .bg(t.color.bg_canvas)
            .border_1()
            .border_color(border)
            .child(div().flex_1().min_h(px(0.0)).w_full().child(input));

        // Bottom action row — outside the editor shell so the
        // resizable commit area reads like the sibling Pier app:
        // text surface first, actions second.
        let bottom = div()
            .flex_none()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(SP_2)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(SP_1)
                    .children(self.bottom_left),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(SP_1)
                    .children(self.bottom_right),
            );

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(SP_1_5)
            .child(editor)
            .child(bottom)
    }
}
