#![allow(dead_code)]

use gpui::{div, prelude::*, IntoElement, SharedString};

use crate::theme::{
    theme, ui_font_with,
    typography::{
        SIZE_BODY, SIZE_BODY_LARGE, SIZE_CAPTION, SIZE_DISPLAY, SIZE_H1, SIZE_H2, SIZE_H3,
        SIZE_MONO_CODE, SIZE_SMALL, SIZE_UI_LABEL, WEIGHT_MEDIUM, WEIGHT_REGULAR,
    },
};

#[allow(dead_code)]
pub enum TextRole {
    Display,
    H1,
    H2,
    H3,
    UiLabel,
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
    truncate: bool,
}

impl Text {
    pub fn new(role: TextRole, label: impl Into<SharedString>) -> Self {
        Self {
            role,
            label: label.into(),
            centered: false,
            secondary: false,
            truncate: false,
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

    /// Truncate the text to one line with an ellipsis on overflow.
    ///
    /// Critical for CJK text inside a flex row: without `whitespace_nowrap`
    /// (which this enables via GPUI's `.truncate()`), a long Chinese path
    /// will wrap *per character* and render vertically (see the SFTP bug
    /// in the layout plan). Callers should also ensure the containing
    /// flex child has `.flex_1().min_w(px(0.0))` so the truncate has a
    /// bounded width to shrink into.
    pub fn truncate(mut self) -> Self {
        self.truncate = true;
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
            TextRole::UiLabel => (SIZE_UI_LABEL, WEIGHT_MEDIUM, false),
            TextRole::BodyLarge => (SIZE_BODY_LARGE, WEIGHT_REGULAR, false),
            TextRole::Body => (SIZE_BODY, WEIGHT_REGULAR, false),
            TextRole::Caption => (SIZE_CAPTION, WEIGHT_REGULAR, false),
            TextRole::Small => (SIZE_SMALL, WEIGHT_REGULAR, false),
            TextRole::Mono => (SIZE_MONO_CODE, WEIGHT_REGULAR, true),
        };

        let color = if self.secondary {
            t.color.text_secondary
        } else {
            t.color.text_primary
        };

        let mut el = div().text_size(size).text_color(color);
        if mono {
            // Mono is terminal/code text — it intentionally doesn't share
            // the UI FontFeatures (cv01/ss03 are Inter-specific and would
            // do nothing against a mono family anyway). Go family+weight.
            el = el.font_family(t.font_mono.clone()).font_weight(weight);
        } else {
            // `.font(...)` is the only GPUI path that writes FontFeatures
            // through to the platform shaper — `.font_family(...)` alone
            // drops features. Use the cached UI font for every role so
            // cv01/ss03 actually activate on Inter.
            el = el.font(ui_font_with(&t.font_ui, &t.font_ui_features, weight));
        }

        let mut el = el.child(self.label);
        if self.centered {
            el = el.text_center();
        }
        if self.truncate {
            el = el.truncate();
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

pub fn ui_label(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::UiLabel, s)
}

pub fn body_large(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::BodyLarge, s)
}

pub fn body(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::Body, s)
}

pub fn caption(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::Caption, s)
}

pub fn small(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::Small, s)
}

pub fn mono(s: impl Into<SharedString>) -> Text {
    Text::new(TextRole::Mono, s)
}
