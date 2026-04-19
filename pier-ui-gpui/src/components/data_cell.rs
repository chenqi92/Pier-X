#![allow(dead_code)]

//! `DataCell` — the tile that sits inside an inspector grid row.
//!
//! Layout (top → bottom within `INSPECTOR_CELL_H = 56px`):
//!
//! ```text
//! ┌─────────────────────┐
//! │ label (caption)     │ ← text.tertiary, 12px
//! │ 3.2%                │ ← value: H3 16px, tone-colored
//! │ bar ▂▂▂             │ ← optional 3px progress bar
//! │ 1m 0.19 · 5m 0.35   │ ← optional secondary caption
//! └─────────────────────┘
//! ```
//!
//! Cells have **no border of their own** — the row helper
//! (`data_cell_row`) wraps each cell and draws a 1px right border for
//! every cell except the last, so a row reads as a single
//! pixel-separated table (like the reference trading app's
//! Open / High / Low strip) instead of floating chips.

use gpui::{div, prelude::*, px, IntoElement, ParentElement, Rgba, SharedString, Window};

use crate::components::text;
use crate::theme::{
    heights::INSPECTOR_CELL_H,
    spacing::{SP_1, SP_2, SP_3},
    theme,
    typography::{SIZE_H3, WEIGHT_MEDIUM},
    ui_font_with,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DataTone {
    /// Primary text color, no chroma — the default.
    Default,
    /// IntelliJ accent blue — use for CPU / highlighted metric.
    Accent,
    /// status.success green.
    Positive,
    /// status.warning yellow — disk 75-89 %, swap pressure.
    Warning,
    /// status.error red — disk ≥ 90 %, failed probe, etc.
    Negative,
}

#[derive(IntoElement)]
pub struct DataCell {
    label: SharedString,
    value: SharedString,
    secondary: Option<SharedString>,
    bar: Option<f32>,
    bar_color: Option<Rgba>,
    tone: DataTone,
    mono: bool,
}

impl DataCell {
    pub fn new(label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            secondary: None,
            bar: None,
            bar_color: None,
            tone: DataTone::Default,
            mono: false,
        }
    }

    pub fn secondary(mut self, caption: impl Into<SharedString>) -> Self {
        self.secondary = Some(caption.into());
        self
    }

    pub fn bar(mut self, ratio: f32) -> Self {
        self.bar = Some(ratio.clamp(0.0, 1.0));
        self
    }

    pub fn bar_color(mut self, color: Rgba) -> Self {
        self.bar_color = Some(color);
        self
    }

    pub fn tone(mut self, tone: DataTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
}

impl RenderOnce for DataCell {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let value_color = match self.tone {
            DataTone::Default => t.color.text_primary,
            DataTone::Accent => t.color.accent,
            DataTone::Positive => t.color.status_success,
            DataTone::Warning => t.color.status_warning,
            DataTone::Negative => t.color.status_error,
        };
        let bar_fill = self.bar_color.unwrap_or(match self.tone {
            DataTone::Default => t.color.accent,
            _ => value_color,
        });

        let value_font = if self.mono {
            None
        } else {
            Some(ui_font_with(&t.font_ui, &t.font_ui_features, WEIGHT_MEDIUM))
        };

        let mut value_node = div()
            .text_size(SIZE_H3)
            .text_color(value_color)
            .font_weight(WEIGHT_MEDIUM)
            .truncate();
        if self.mono {
            value_node = value_node.font_family(t.font_mono.clone());
        } else if let Some(font) = value_font {
            value_node = value_node.font(font);
        }
        let value_node = value_node.child(self.value);

        let bar_node = self.bar.map(|ratio| {
            div()
                .w_full()
                .h(px(3.0))
                .rounded(px(1.5))
                .bg(t.color.bg_panel)
                .child(
                    div()
                        .h_full()
                        .w(gpui::relative(ratio))
                        .rounded(px(1.5))
                        .bg(bar_fill),
                )
        });

        let secondary_node = self
            .secondary
            .map(|caption| text::caption(caption).secondary().truncate());

        div()
            .flex_1()
            .min_w(px(0.0))
            .h(INSPECTOR_CELL_H)
            .overflow_hidden()
            .px(SP_3)
            .py(SP_2)
            .flex()
            .flex_col()
            .justify_between()
            .gap(SP_1)
            .child(text::caption(self.label).secondary().truncate())
            .child(value_node)
            .when_some(bar_node, |el, bar| el.child(bar))
            .when_some(secondary_node, |el, caption| el.child(caption))
    }
}

/// Horizontal row of DataCells, equally sharing width and separated by
/// 1px border.subtle rules. Wrap this inside an `InspectorSection` so
/// that the row sits flush with the section header and the separator
/// below it.
pub fn data_cell_row(cells: Vec<DataCell>) -> impl IntoElement {
    let last = cells.len().saturating_sub(1);
    // Default flex-row alignment is stretch, which is what we want so
    // every cell pulls to the row's tallest content — giving the
    // vertical dividers a consistent height.
    let mut row = div().w_full().flex().flex_row().overflow_hidden();
    for (idx, cell) in cells.into_iter().enumerate() {
        row = row.child(WrappedCell {
            cell,
            draw_right_border: idx < last,
        });
    }
    row
}

/// Internal wrapper that pairs a `DataCell` with a 1px right border
/// drawn in theme `border_subtle`. Keeping this as its own
/// `IntoElement` lets the wrapper pull `theme(cx)` at render time
/// without forcing `data_cell_row` to carry a `cx` argument.
#[derive(IntoElement)]
struct WrappedCell {
    cell: DataCell,
    draw_right_border: bool,
}

impl RenderOnce for WrappedCell {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let mut wrapper = div().flex_1().min_w(px(0.0)).flex().flex_col();
        if self.draw_right_border {
            wrapper = wrapper.border_r_1().border_color(t.color.border_subtle);
        }
        wrapper.child(self.cell)
    }
}
