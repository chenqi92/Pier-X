//! Layout pass for the direct-GPU cell grid. See module docs.
//!
//! This file is intentionally GPUI-light: it only depends on geometry types
//! (`Bounds`, `Pixels`, `Point`, `Size`, `Hsla`) and `TextRun`. That keeps
//! `build` unit-testable without booting a window.

// Phase 11 step 2 (scaffolding): types and stubs land here unused; the
// view switches over in step 5. The allow holds across steps 3 and 4
// while only the unit tests reference these items.
#![allow(dead_code)]

use gpui::{Bounds, Hsla, Pixels, Point, SharedString, Size, TextRun, px};
use pier_core::terminal::GridSnapshot;

use crate::views::terminal::TerminalLine;

/// Result of the layout pass — everything `paint::run` needs to emit one
/// frame's worth of `paint_quad` + `ShapedLine::paint` calls.
///
/// Selection is not a separate field: the upstream `render_terminal_line`
/// already encodes it into each selected cell's `TextRun.background_color`,
/// so it falls out of `bg_rects` for free.
#[derive(Clone, Default)]
pub(crate) struct LayoutState {
    pub bg_rects: Vec<BgRect>,
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CursorPaintStyle {
    Underline,
    Bar,
}

const CURSOR_BAR_WIDTH_PX: f32 = 2.0;
const CURSOR_UNDERLINE_HEIGHT_PX: f32 = 2.0;

/// Build a `LayoutState` from the same `Vec<TerminalLine>` the legacy
/// StyledText path consumes.
///
/// `cursor_paint` is `None` when no overlay rect should be drawn — that
/// covers (a) cursor hidden / blinking off, (b) Block cursor (already
/// encoded into runs by `render_terminal_line`), and (c) scrollback view.
///
/// Cell-span math counts UTF-8 chars per run, which is correct for the ASCII
/// + Latin output that dominates terminal sessions. Wide CJK characters take
/// 2 cells in the source `GridSnapshot` but only 1 char in the rendered
/// `TerminalLine.text` (the second cell holds `\0` and is skipped upstream).
/// They will under-position later cells in the same row by 1 column each.
/// Phase 11 ships with this known limitation; Phase 12 may extend
/// `TerminalLine` with a per-run cell span if it matters in practice.
pub(crate) fn build(
    lines: &[TerminalLine],
    snapshot: &GridSnapshot,
    cursor_paint: Option<(CursorPaintStyle, Hsla)>,
    cell_size: Size<Pixels>,
    origin: Point<Pixels>,
) -> LayoutState {
    let mut bg_rects: Vec<BgRect> = Vec::new();
    let mut rows: Vec<BatchedRow> = Vec::with_capacity(lines.len());

    for (row_idx, line) in lines.iter().enumerate() {
        let row_y = origin.y + cell_size.height * row_idx as f32;
        let text_str: &str = line.text.as_ref();
        let mut byte_offset = 0usize;
        let mut col_offset = 0usize;
        let mut stripped_runs: Vec<TextRun> = Vec::with_capacity(line.runs.len());

        for run in &line.runs {
            let run_end = (byte_offset + run.len).min(text_str.len());
            let run_substr = &text_str[byte_offset..run_end];
            let cell_span = run_substr.chars().count();

            if let Some(bg) = run.background_color {
                bg_rects.push(BgRect {
                    bounds: Bounds {
                        origin: Point {
                            x: origin.x + cell_size.width * col_offset as f32,
                            y: row_y,
                        },
                        size: Size {
                            width: cell_size.width * cell_span as f32,
                            height: cell_size.height,
                        },
                    },
                    color: bg,
                });
            }

            let mut stripped = run.clone();
            stripped.background_color = None;
            stripped_runs.push(stripped);

            byte_offset = run_end;
            col_offset += cell_span;
        }

        rows.push(BatchedRow {
            origin: Point {
                x: origin.x,
                y: row_y,
            },
            text: line.text.clone(),
            runs: stripped_runs,
        });
    }

    let cursor = cursor_paint.and_then(|(style, color)| {
        if snapshot.cols == 0 || snapshot.rows == 0 {
            return None;
        }
        let col = (snapshot.cursor_x as usize).min(snapshot.cols.saturating_sub(1) as usize);
        let row = (snapshot.cursor_y as usize).min(snapshot.rows.saturating_sub(1) as usize);
        let left = origin.x + cell_size.width * col as f32;
        let top = origin.y + cell_size.height * row as f32;
        let bounds = match style {
            CursorPaintStyle::Underline => Bounds {
                origin: Point {
                    x: left,
                    y: top + cell_size.height - px(CURSOR_UNDERLINE_HEIGHT_PX),
                },
                size: Size {
                    width: cell_size.width,
                    height: px(CURSOR_UNDERLINE_HEIGHT_PX),
                },
            },
            CursorPaintStyle::Bar => Bounds {
                origin: Point { x: left, y: top },
                size: Size {
                    width: px(CURSOR_BAR_WIDTH_PX),
                    height: cell_size.height,
                },
            },
        };
        Some(CursorRect {
            bounds,
            color,
            style,
        })
    });

    LayoutState {
        bg_rects,
        rows,
        cursor,
        cell_size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{font, hsla};
    use pier_core::terminal::Cell;

    fn make_snapshot(cols: u16, rows: u16, cursor_x: u16, cursor_y: u16) -> GridSnapshot {
        GridSnapshot {
            cols,
            rows,
            cursor_x,
            cursor_y,
            bracketed_paste_mode: false,
            cells: vec![Cell::default(); (cols as usize) * (rows as usize)],
        }
    }

    fn run(text_bytes: usize, bg: Option<Hsla>) -> TextRun {
        TextRun {
            len: text_bytes,
            font: font("test"),
            color: hsla(0.0, 0.0, 1.0, 1.0),
            background_color: bg,
            underline: None,
            strikethrough: None,
        }
    }

    fn line(text: &str, runs: Vec<TextRun>) -> TerminalLine {
        TerminalLine {
            text: SharedString::from(text.to_string()),
            runs,
        }
    }

    fn cell_size(w: f32, h: f32) -> Size<Pixels> {
        Size {
            width: px(w),
            height: px(h),
        }
    }

    #[test]
    fn empty_runs_produce_no_bg_rects() {
        let lines = vec![line("hello", vec![run(5, None)])];
        let snap = make_snapshot(80, 1, 0, 0);
        let state = build(&lines, &snap, None, cell_size(8.0, 18.0), Point::default());

        assert!(state.bg_rects.is_empty());
        assert_eq!(state.rows.len(), 1);
        assert_eq!(state.rows[0].text.as_ref(), "hello");
        assert_eq!(state.rows[0].runs.len(), 1);
        assert!(state.rows[0].runs[0].background_color.is_none());
    }

    #[test]
    fn runs_with_bg_emit_rects_at_correct_columns() {
        let red = hsla(0.0, 1.0, 0.5, 1.0);
        let blue = hsla(0.6, 1.0, 0.5, 1.0);
        // "ab" red, "cd" no bg, "ef" blue → cols 0-1 red rect, cols 4-5 blue rect.
        let lines = vec![line(
            "abcdef",
            vec![run(2, Some(red)), run(2, None), run(2, Some(blue))],
        )];
        let snap = make_snapshot(80, 1, 0, 0);
        let state = build(&lines, &snap, None, cell_size(10.0, 20.0), Point::default());

        assert_eq!(state.bg_rects.len(), 2);
        let r0 = &state.bg_rects[0];
        assert_eq!(r0.color, red);
        assert_eq!(f32::from(r0.bounds.origin.x), 0.0);
        assert_eq!(f32::from(r0.bounds.origin.y), 0.0);
        assert_eq!(f32::from(r0.bounds.size.width), 20.0);
        assert_eq!(f32::from(r0.bounds.size.height), 20.0);

        let r1 = &state.bg_rects[1];
        assert_eq!(r1.color, blue);
        assert_eq!(f32::from(r1.bounds.origin.x), 40.0);
        assert_eq!(f32::from(r1.bounds.size.width), 20.0);
    }

    #[test]
    fn rows_get_correct_y_origins() {
        let lines = vec![
            line("a", vec![run(1, None)]),
            line("b", vec![run(1, None)]),
            line("c", vec![run(1, None)]),
        ];
        let snap = make_snapshot(80, 3, 0, 0);
        let state = build(&lines, &snap, None, cell_size(8.0, 18.0), Point::default());

        assert_eq!(state.rows.len(), 3);
        assert_eq!(f32::from(state.rows[0].origin.y), 0.0);
        assert_eq!(f32::from(state.rows[1].origin.y), 18.0);
        assert_eq!(f32::from(state.rows[2].origin.y), 36.0);
    }

    #[test]
    fn origin_offset_is_applied_to_rects_and_rows() {
        let red = hsla(0.0, 1.0, 0.5, 1.0);
        let lines = vec![line("ab", vec![run(2, Some(red))])];
        let snap = make_snapshot(80, 1, 0, 0);
        let origin = Point {
            x: px(10.0),
            y: px(5.0),
        };
        let state = build(&lines, &snap, None, cell_size(8.0, 18.0), origin);

        assert_eq!(f32::from(state.bg_rects[0].bounds.origin.x), 10.0);
        assert_eq!(f32::from(state.bg_rects[0].bounds.origin.y), 5.0);
        assert_eq!(f32::from(state.rows[0].origin.x), 10.0);
        assert_eq!(f32::from(state.rows[0].origin.y), 5.0);
    }

    #[test]
    fn underline_cursor_anchored_to_bottom_of_cell() {
        let lines = vec![line(" ", vec![run(1, None)])];
        let snap = make_snapshot(80, 24, 5, 3);
        let cursor_color = hsla(0.5, 1.0, 0.5, 1.0);
        let state = build(
            &lines,
            &snap,
            Some((CursorPaintStyle::Underline, cursor_color)),
            cell_size(8.0, 18.0),
            Point::default(),
        );

        let c = state.cursor.expect("cursor should be present");
        assert_eq!(c.style, CursorPaintStyle::Underline);
        assert_eq!(c.color, cursor_color);
        assert_eq!(f32::from(c.bounds.origin.x), 40.0);
        assert_eq!(f32::from(c.bounds.origin.y), 3.0 * 18.0 + 16.0);
        assert_eq!(f32::from(c.bounds.size.width), 8.0);
        assert_eq!(f32::from(c.bounds.size.height), 2.0);
    }

    #[test]
    fn bar_cursor_spans_full_cell_height() {
        let lines = vec![line(" ", vec![run(1, None)])];
        let snap = make_snapshot(80, 24, 0, 0);
        let cursor_color = hsla(0.5, 1.0, 0.5, 1.0);
        let state = build(
            &lines,
            &snap,
            Some((CursorPaintStyle::Bar, cursor_color)),
            cell_size(8.0, 18.0),
            Point::default(),
        );

        let c = state.cursor.expect("cursor should be present");
        assert_eq!(c.style, CursorPaintStyle::Bar);
        assert_eq!(f32::from(c.bounds.size.width), 2.0);
        assert_eq!(f32::from(c.bounds.size.height), 18.0);
    }

    #[test]
    fn no_cursor_paint_means_no_cursor_rect() {
        let lines = vec![line(" ", vec![run(1, None)])];
        let snap = make_snapshot(80, 24, 5, 3);
        let state = build(&lines, &snap, None, cell_size(8.0, 18.0), Point::default());
        assert!(state.cursor.is_none());
    }

    #[test]
    fn cursor_clamped_to_grid_bounds() {
        let lines = vec![line(" ", vec![run(1, None)])];
        // cursor outside the declared grid — clamp to last cell.
        let snap = make_snapshot(80, 24, 999, 999);
        let cursor_color = hsla(0.5, 1.0, 0.5, 1.0);
        let state = build(
            &lines,
            &snap,
            Some((CursorPaintStyle::Bar, cursor_color)),
            cell_size(8.0, 18.0),
            Point::default(),
        );

        let c = state.cursor.expect("cursor should be present");
        assert_eq!(f32::from(c.bounds.origin.x), 79.0 * 8.0);
        assert_eq!(f32::from(c.bounds.origin.y), 23.0 * 18.0);
    }

    #[test]
    fn zero_dimension_snapshot_drops_cursor() {
        let lines: Vec<TerminalLine> = Vec::new();
        let snap = make_snapshot(0, 0, 0, 0);
        let cursor_color = hsla(0.5, 1.0, 0.5, 1.0);
        let state = build(
            &lines,
            &snap,
            Some((CursorPaintStyle::Bar, cursor_color)),
            cell_size(8.0, 18.0),
            Point::default(),
        );
        assert!(state.cursor.is_none());
    }

    #[test]
    fn run_bytes_overflow_is_clamped_not_panicked() {
        // Defensive: if upstream ever emits a run with len > text bytes (it
        // shouldn't), build() should clamp instead of slicing past the end.
        let lines = vec![line("ab", vec![run(99, None)])];
        let snap = make_snapshot(80, 1, 0, 0);
        let state = build(&lines, &snap, None, cell_size(8.0, 18.0), Point::default());
        assert_eq!(state.rows.len(), 1);
        assert_eq!(state.rows[0].text.as_ref(), "ab");
    }
}
