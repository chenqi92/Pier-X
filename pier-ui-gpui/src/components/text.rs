use gpui::{div, prelude::*, IntoElement, SharedString};

use crate::theme::{
    theme,
    typography::{
        SIZE_BODY, SIZE_BODY_LARGE, SIZE_CAPTION, SIZE_DISPLAY, SIZE_H1, SIZE_H2, SIZE_H3,
        SIZE_MONO_CODE, SIZE_SMALL, WEIGHT_MEDIUM, WEIGHT_REGULAR,
    },
};

#[allow(dead_code)]
pub enum TextRole {
    Display,
    H1,
    H2,
    H3,
    BodyLarge,
    Body,
    Caption,
    Small,
    Mono,
}

#[derive(IntoElement)]
pub struct Text {
    label: SharedString,
    role: TextRole,
    centered: bool,
    secondary: bool,
}

impl Text {
    pub fn new(role: TextRole, label: impl Into<SharedString>) -> Self {
        Self {
            role,
            label: label.into(),
            centered: false,
            secondary: false,
        }
    }

    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }

    pub fn secondary(mut self) -> Self {
        self.secondary = true;
        self
    }
}

impl RenderOnce for Text {
    fn render(self, _: &mut gpui::Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);

        let (size, weight, mono) = match self.role {
            TextRole::Display => (SIZE_DISPLAY, WEIGHT_MEDIUM, false),
            TextRole::H1 => (SIZE_H1, WEIGHT_MEDIUM, false),
            TextRole::H2 => (SIZE_H2, WEIGHT_MEDIUM, false),
            TextRole::H3 => (SIZE_H3, WEIGHT_MEDIUM, false),
            TextRole::BodyLarge => (SIZE_BODY_LARGE, WEIGHT_REGULAR, false),
            TextRole::Body => (SIZE_BODY, WEIGHT_REGULAR, false),
            TextRole::Caption => (SIZE_CAPTION, WEIGHT_MEDIUM, false),
            TextRole::Small => (SIZE_SMALL, WEIGHT_REGULAR, false),
            TextRole::Mono => (SIZE_MONO_CODE, WEIGHT_REGULAR, true),
        };

        let color = if self.secondary {
            t.color.text_secondary
        } else {
            t.color.text_primary
        };

        let family = if mono {
            t.font_mono.clone()
        } else {
            t.font_ui.clone()
        };

        let mut el = div()
            .text_size(size)
            .font_weight(weight)
            .font_family(family)
            .text_color(color)
            .child(self.label);
        if self.centered {
            el = el.text_center();
        }
        el
    }
}

pub fn display(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::Display, s)
}

pub fn h1(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::H1, s)
}

pub fn h2(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::H2, s)
}

pub fn h3(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::H3, s)
}

pub fn body(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::Body, s)
}

pub fn caption(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::Caption, s)
}

pub fn mono(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::Mono, s)
}
