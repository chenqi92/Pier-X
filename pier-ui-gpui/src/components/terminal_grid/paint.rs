//! Paint pass for the direct-GPU cell grid. See module docs.

#![allow(dead_code)]

use gpui::{App, Bounds, Pixels, SharedString, Window, fill};

use super::layout::LayoutState;

/// Emit the GPU draw calls for one frame of the terminal cell grid.
///
/// Order is load-bearing:
///
/// 1. cell backgrounds (selection is encoded into these by upstream
///    `render_terminal_line`, so it falls out for free)
/// 2. shaped text per row
/// 3. cursor rect (Bar / Underline only — Block is encoded into the runs)
///
/// `font_family` is forwarded as the family name in case future
/// per-component font swapping is added; today the runs already carry their
/// own `Font`, so this argument is unused at the call site of `shape_line`.
pub(crate) fn run(
    _bounds: Bounds<Pixels>,
    layout: &LayoutState,
    _font_family: &SharedString,
    font_size: Pixels,
    line_height: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    for bg in &layout.bg_rects {
        window.paint_quad(fill(bg.bounds, bg.color));
    }

    for row in &layout.rows {
        let shaped = window.text_system().shape_line(
            row.text.clone(),
            font_size,
            &row.runs,
            Some(layout.cell_size.width),
        );
        // ShapedLine::paint only fails on missing fonts; in that case the
        // glyphs are already absent and there's nothing useful to fall back
        // to — log and continue so the rest of the frame still paints.
        if let Err(err) = shaped.paint(row.origin, line_height, window, cx) {
            log::warn!("terminal_grid: shape_line paint failed: {err}");
        }
    }

    if let Some(cursor) = layout.cursor {
        window.paint_quad(fill(cursor.bounds, cursor.color));
    }
}
