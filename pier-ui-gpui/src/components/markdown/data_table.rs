use gpui::{div, prelude::*, relative, AnyElement, App, IntoElement, Window};

use crate::theme::{
    radius::RADIUS_MD,
    spacing::{SP_2, SP_3},
    theme,
    typography::{SIZE_BODY, WEIGHT_MEDIUM, WEIGHT_REGULAR},
    ui_font_with, Theme,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownTableAlign {
    Left,
    Center,
    Right,
}

/// Bordered Markdown table. Cells wrap their content — we intentionally
/// do not add horizontal scroll: markdown tables carry prose, not tabular
/// data, so natural wrapping matches how GitHub / IntelliJ / VSCode
/// render them and avoids the "horizontal scrollbar hijacks outer
/// vertical scroll" interaction bug that `overflow_x_scrollbar` created.
///
/// Callers pre-render each cell as an `AnyElement` (the view owns the
/// `MarkdownText → StyledText` conversion); this component handles the
/// border grid, header emphasis, and per-column text alignment.
#[derive(IntoElement)]
pub struct MarkdownDataTable {
    aligns: Vec<MarkdownTableAlign>,
    header: Vec<AnyElement>,
    rows: Vec<Vec<AnyElement>>,
}

impl MarkdownDataTable {
    pub fn new(aligns: Vec<MarkdownTableAlign>) -> Self {
        Self {
            aligns,
            header: Vec::new(),
            rows: Vec::new(),
        }
    }

    pub fn header(mut self, cells: Vec<AnyElement>) -> Self {
        self.header = cells;
        self
    }

    pub fn row(mut self, cells: Vec<AnyElement>) -> Self {
        self.rows.push(cells);
        self
    }
}

impl RenderOnce for MarkdownDataTable {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let Self {
            aligns,
            header,
            rows,
        } = self;

        let has_header = !header.is_empty();
        let mut container = div()
            .w_full()
            .border_1()
            .border_color(t.color.border_subtle)
            .rounded(RADIUS_MD)
            .overflow_hidden()
            .flex()
            .flex_col();

        if has_header {
            container = container.child(render_row(header, &aligns, true, t));
        }
        for (ix, row) in rows.into_iter().enumerate() {
            let mut row_el = div();
            if ix > 0 || has_header {
                row_el = row_el.border_t_1().border_color(t.color.border_subtle);
            }
            container = container.child(row_el.child(render_row(row, &aligns, false, t)));
        }

        container
    }
}

fn render_row(
    cells: Vec<AnyElement>,
    aligns: &[MarkdownTableAlign],
    header: bool,
    t: &Theme,
) -> impl IntoElement {
    let weight = if header {
        WEIGHT_MEDIUM
    } else {
        WEIGHT_REGULAR
    };
    // Default cross-axis alignment in flex is stretch, so each cell
    // grows to match the tallest one — no `items_stretch` helper needed.
    div()
        .w_full()
        .flex()
        .flex_row()
        .children(cells.into_iter().enumerate().map(|(ix, cell)| {
            let align = aligns.get(ix).copied().unwrap_or(MarkdownTableAlign::Left);
            // Each cell gets an equal `flex_1` share with `min_w(0)` so
            // long content wraps inside the cell instead of pushing the
            // table past the panel width.
            let mut base = div().flex_1().min_w(gpui::px(0.0)).px(SP_3).py(SP_2);
            if ix > 0 {
                base = base.border_l_1().border_color(t.color.border_subtle);
            }
            if header {
                base = base.bg(t.color.bg_hover);
            }
            let mut content = div()
                .w_full()
                .text_size(SIZE_BODY)
                .line_height(relative(1.45))
                .text_color(t.color.text_primary)
                .font(ui_font_with(&t.font_ui, &t.font_ui_features, weight))
                .child(cell);
            content = match align {
                MarkdownTableAlign::Left => content,
                MarkdownTableAlign::Center => content.text_center(),
                MarkdownTableAlign::Right => content.text_right(),
            };
            base.child(content)
        }))
}
