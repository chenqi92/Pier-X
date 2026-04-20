#![allow(dead_code)]

//! Commit composer — a bordered multi-line input with an inline bottom
//! action row. Mirrors the Pier "commit box": input area on top, a
//! slim action bar inside the same bordered shell for stage-all on
//! the left and the commit split-button on the right.
//!
//! Callers fill the two slots with fully-constructed elements
//! (typically a [`super::Button`] and a [`super::split_button::SplitButton`]).

use gpui::{
    div, prelude::*, px, AnyElement, App, Entity, Focusable, IntoElement, RenderOnce, Window,
};
use gpui_component::input::{Input, InputState};

use crate::theme::{
    radius::RADIUS_MD,
    spacing::{SP_1, SP_2},
    theme,
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
            .text_color(t.color.text_primary);

        // Bottom action row — always rendered so the stage-all + commit
        // button anchor at a stable y-offset inside the box even when a
        // slot is empty on that side.
        let bottom = div()
            .flex_none()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(SP_2)
            .pt(SP_1)
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

        // Let the input root stretch to the full draggable footer
        // height so the whole text area is a real focus / key target.
        // Previous iterations left the multi-line input at auto
        // height, which created a large dead zone under the
        // placeholder and made the footer look like it wasn't pinned
        // to the bottom. `min_h(0)` is kept so the flex-1 child can
        // shrink inside a constrained parent without pushing the
        // bottom row out.
        div()
            .size_full()
            .flex()
            .flex_col()
            .px(SP_2)
            .py(SP_1)
            .gap(SP_1)
            .rounded(RADIUS_MD)
            .bg(t.color.bg_canvas)
            .border_1()
            .border_color(border)
            .child(div().flex_1().min_h(px(0.0)).w_full().child(input))
            .child(bottom)
    }
}
