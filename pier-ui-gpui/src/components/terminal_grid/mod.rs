//! Direct-GPU terminal cell-grid renderer (Phase 11).
//!
//! Replaces the per-row `StyledText` element tree with one `shape_line` call
//! per visible row plus batched `paint_quad` calls for backgrounds, selection
//! and cursor. The pipeline follows the shape used by Zed's
//! `crates/terminal_view/src/terminal_element.rs`:
//!
//! 1. [`layout::build`] consumes the same `Vec<TerminalLine>` produced by
//!    `TerminalPanel::render_lines` and computes a `LayoutState` — flat lists
//!    of `BgRect` / selection rects / per-row shaping input / optional
//!    cursor rect. Pure data, no GPUI types except geometry and `TextRun`.
//! 2. [`paint::run`] consumes the `LayoutState` inside a `canvas(..)` paint
//!    closure and emits `paint_quad` + `ShapedLine::paint` calls.
//!
//! Both passes are free functions so the enclosing view owns the cache
//! (keyed by the existing `TerminalRenderKey`).

pub(crate) mod layout;
pub(crate) mod paint;

pub(crate) use layout::{CursorPaintStyle, LayoutState, build};
pub(crate) use paint::run;
