use gpui::{div, prelude::*, App, Entity, Focusable, IntoElement, RenderOnce, Window};
use gpui_component::{
    input::{Input, InputState},
    Icon as UiIcon, IconName,
};

use crate::theme::{
    heights::BUTTON_MD_H,
    radius::RADIUS_MD,
    spacing::{SP_1_5, SP_2},
    theme,
    typography::SIZE_MONO_SMALL,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InlineInputTone {
    Surface,
    Inset,
}

#[derive(IntoElement)]
pub struct InlineInput {
    state: Entity<InputState>,
    leading_icon: Option<IconName>,
    tone: InlineInputTone,
    cleanable: bool,
    mono: bool,
}

impl InlineInput {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            state: state.clone(),
            leading_icon: None,
            tone: InlineInputTone::Surface,
            cleanable: false,
            mono: false,
        }
    }

    pub fn leading_icon(mut self, icon: IconName) -> Self {
        self.leading_icon = Some(icon);
        self
    }

    pub fn tone(mut self, tone: InlineInputTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn cleanable(mut self) -> Self {
        self.cleanable = true;
        self
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
}

impl RenderOnce for InlineInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        let bg = match self.tone {
            InlineInputTone::Surface => t.color.bg_surface,
            InlineInputTone::Inset => t.color.bg_canvas,
        };
        let border = if focused {
            t.color.accent
        } else {
            t.color.border_subtle
        };
        let icon_color = if focused {
            t.color.accent
        } else {
            t.color.text_tertiary
        };

        let mut input = Input::new(&self.state)
            .appearance(false)
            .bordered(false)
            .focus_bordered(false)
            .cleanable(self.cleanable)
            .w_full()
            .text_color(t.color.text_primary);

        if self.mono {
            input = input
                .font_family(t.font_mono.clone())
                .text_size(SIZE_MONO_SMALL);
        }

        let mut row = div()
            .w_full()
            .min_h(BUTTON_MD_H)
            .px(SP_2)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1_5)
            .rounded(RADIUS_MD)
            .bg(bg)
            .border_1()
            .border_color(border);

        if let Some(icon) = self.leading_icon {
            row = row.child(
                UiIcon::new(icon)
                    .size(crate::theme::heights::GLYPH_MD)
                    .text_color(icon_color),
            );
        }

        row.child(div().flex_1().min_w(gpui::px(0.0)).child(input))
    }
}
