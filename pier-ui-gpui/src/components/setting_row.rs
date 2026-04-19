//! A single "label + optional description on the left, control on
//! the right" row — the atom of the Settings dialog.
//!
//! Mirrors the Tauri shell's `.settings__row` from the deleted
//! stylesheet (and macOS System Settings / iOS Settings):
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │ Language                         [System] [English] [中] │
//! │ Applied immediately. System is …                         │
//! └──────────────────────────────────────────────────────────┘
//!      ↑ flex-1 label column                  ↑ flex-none ctl
//! ```
//!
//! - The label takes the natural flex share; description wraps
//!   below it in a smaller, secondary style.
//! - Controls (a chip group, a toggle, a stepper, …) sit flush
//!   right via `justify_between` so the row reads at a glance.
//! - Minimum height ≈ 40 px keeps dense lists aligned even when
//!   descriptions are `None`.
//!
//! Earlier this component fixed the label column at 216 px, which
//! made the dialog feel like a form (a table really) and left
//! awkward empty space for short labels. The flexible layout
//! matches the live reference and the Tauri archive.

use gpui::{div, prelude::*, px, AnyElement, IntoElement, ParentElement, SharedString, Window};

use crate::components::text;
use crate::theme::spacing::{SP_0_5, SP_2, SP_4};

#[derive(IntoElement)]
pub struct SettingRow {
    title: SharedString,
    description: Option<SharedString>,
    /// If set, pushes the controls cluster to align with the top of
    /// the label stack instead of vertically centering. Useful when
    /// a description wraps to several lines and the control would
    /// otherwise drift down.
    align_top: bool,
    controls: Vec<AnyElement>,
}

impl SettingRow {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            description: None,
            align_top: false,
            controls: Vec::new(),
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn align_top(mut self) -> Self {
        self.align_top = true;
        self
    }
}

impl ParentElement for SettingRow {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.controls.extend(elements);
    }
}

impl RenderOnce for SettingRow {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        // Label stack — title on top (body, primary) + optional
        // description below (caption, secondary). `flex_1` lets it
        // claim the leftover, but `min_w(px(180))` is the real fix
        // for the CJK-per-character bug: without a minimum, a wide
        // controls cluster (e.g. 8 font chips) collapses the label
        // column to ~1 char, triggering per-character wrapping of
        // Chinese titles/descriptions. See text::Text::truncate doc
        // for the full story.
        let mut label = div()
            .flex_1()
            .min_w(px(180.0))
            .flex()
            .flex_col()
            .gap(SP_0_5)
            .child(text::ui_label(self.title));

        if let Some(description) = self.description {
            label = label.child(text::caption(description).secondary());
        }

        // Controls cluster — flush right, capped at ~55% of row
        // width. `flex_shrink` lets it yield when the label has
        // long text, so neither side pushes the other off-screen.
        let controls = div()
            .flex_none()
            .max_w(px(460.0))
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(SP_2)
            .when(self.align_top, |el| el.items_start())
            .when(!self.align_top, |el| el.items_center())
            .justify_end()
            .children(self.controls);

        // Row shell — horizontal with big gap between label and
        // controls; 40 px min height so empty-description rows
        // align cleanly with rich-description rows above / below.
        div()
            .w_full()
            .min_h(px(40.0))
            .py(SP_2)
            .flex()
            .flex_row()
            .justify_between()
            .gap(SP_4)
            .when(self.align_top, |el| el.items_start())
            .when(!self.align_top, |el| el.items_center())
            .child(label)
            .child(controls)
    }
}
