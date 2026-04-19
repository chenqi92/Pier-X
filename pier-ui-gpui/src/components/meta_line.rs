#![allow(dead_code)]

//! One-line `[icon] mono-text` row.
//!
//! Used wherever you want to surface a stable piece of context — a
//! file path, an endpoint, a version, a hash — without the visual
//! weight of a `StatusPill` (which is reserved for dynamic state).
//! Typical spots: PageHeader subtitle, SessionSummaryCard endpoint
//! row, a list-item detail beneath a bold title.

use gpui::{div, prelude::*, IntoElement, SharedString, Window};
use gpui_component::{Icon as UiIcon, IconName};

use crate::theme::{
    heights::ICON_SM,
    spacing::SP_1,
    theme,
    typography::{SIZE_MONO_SMALL, WEIGHT_REGULAR},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MetaColor {
    Secondary,
    Tertiary,
}

#[derive(IntoElement)]
pub struct MetaLine {
    text: SharedString,
    icon: Option<IconName>,
    color: MetaColor,
}

impl MetaLine {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            icon: None,
            color: MetaColor::Secondary,
        }
    }

    pub fn with_icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn tertiary(mut self) -> Self {
        self.color = MetaColor::Tertiary;
        self
    }
}

impl RenderOnce for MetaLine {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let color = match self.color {
            MetaColor::Secondary => t.color.text_secondary,
            MetaColor::Tertiary => t.color.text_tertiary,
        };

        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .min_w(gpui::px(0.0))
            .gap(SP_1)
            .text_color(color);

        if let Some(icon) = self.icon {
            row = row.child(
                div()
                    .flex_none()
                    .child(UiIcon::new(icon).size(ICON_SM)),
            );
        }

        // MetaLine values are usually mono paths/endpoints — long strings
        // would otherwise wrap per-character (CJK) or per-word (ASCII)
        // inside a flex child. Force truncate so the caller only has to
        // ensure the surrounding row gives us a bounded width.
        row.child(
            div()
                .flex_1()
                .min_w(gpui::px(0.0))
                .truncate()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .font_weight(WEIGHT_REGULAR)
                .text_color(color)
                .child(self.text),
        )
    }
}
