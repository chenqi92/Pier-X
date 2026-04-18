//! Paint pass for the direct-GPU cell grid. See module docs.

#![allow(dead_code)]

use gpui::{App, Bounds, Pixels, SharedString, Window};

use super::layout::LayoutState;

/// Emit the GPU draw calls for one frame of the terminal cell grid.
///
/// Order is load-bearing:
///
/// 1. selection rects (so they sit beneath cell backgrounds for the
///    inverted cells the legacy path produces)
/// 2. cell backgrounds
/// 3. shaped text per row
/// 4. cursor rect (Bar / Underline only — Block is encoded into the runs)
///
/// Stub for now — Phase 11 step 4 fills it in.
pub(crate) fn run(
    _bounds: Bounds<Pixels>,
    _layout: &LayoutState,
    _font_family: &SharedString,
    _font_size: Pixels,
    _line_height: Pixels,
    _window: &mut Window,
    _cx: &mut App,
) {
}
