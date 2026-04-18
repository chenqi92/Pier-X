//! Layout pass for the direct-GPU cell grid. See module docs.
//!
//! This file is intentionally GPUI-light: it only depends on geometry types
//! (`Bounds`, `Pixels`, `Point`, `Size`, `Hsla`) and `TextRun`. That keeps
//! `build` unit-testable without booting a window.

// Phase 11 step 2 (scaffolding): types and stubs land here unused; the
// view switches over in step 5. The allow holds across steps 3 and 4
// while only the unit tests reference these items.
#![allow(dead_code)]

use gpui::{Bounds, Hsla, Pixels, Point, SharedString, Size, TextRun};

use crate::views::terminal::{TerminalLine, TerminalSelection};

/// Result of the layout pass — everything `paint::run` needs to emit one
/// frame's worth of `paint_quad` + `ShapedLine::paint` calls.
#[derive(Clone, Default)]
pub(crate) struct LayoutState {
    pub bg_rects: Vec<BgRect>,
    pub selection_rects: Vec<Bounds<Pixels>>,
    pub rows: Vec<BatchedRow>,
    pub cursor: Option<CursorRect>,
    pub cell_size: Size<Pixels>,
}

/// One visible terminal row, ready to feed `text_system().shape_line(..)`.
/// `runs` always carries `background_color: None` — backgrounds live in
/// `LayoutState::bg_rects` so we don't depend on `ShapedLine::paint`'s
/// implementation-defined behavior around per-run backgrounds.
#[derive(Clone)]
pub(crate) struct BatchedRow {
    pub origin: Point<Pixels>,
    pub text: SharedString,
    pub runs: Vec<TextRun>,
}

#[derive(Clone, Copy)]
pub(crate) struct BgRect {
    pub bounds: Bounds<Pixels>,
    pub color: Hsla,
}

#[derive(Clone, Copy)]
pub(crate) struct CursorRect {
    pub bounds: Bounds<Pixels>,
    pub color: Hsla,
    pub style: CursorPaintStyle,
}

/// Only the cursor styles `paint::run` actually draws — `Block` is encoded
/// upstream by `render_lines` (the cursor cell's foreground / background
/// are swapped inside the `TextRun`s) so it never reaches the paint pass.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorPaintStyle {
    Underline,
    Bar,
}

/// Build a `LayoutState` from the same `Vec<TerminalLine>` the legacy
/// StyledText path consumes. Stub for now — Phase 11 step 3 fills it in.
pub(crate) fn build(
    _lines: &[TerminalLine],
    _cursor_cell: Option<(usize, usize)>,
    _cursor: Option<(CursorPaintStyle, Hsla)>,
    _selection: Option<TerminalSelection>,
    _selection_color: Hsla,
    cell_size: Size<Pixels>,
    _origin: Point<Pixels>,
) -> LayoutState {
    LayoutState {
        cell_size,
        ..LayoutState::default()
    }
}
