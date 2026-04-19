#![allow(dead_code)]

//! Commit graph row painter — IDEA-style lane rendering.
//!
//! The expensive layout work (lane assignment, column positioning,
//! segment & arrow generation) already lives in
//! `pier_core::git_graph::compute_graph_layout`. This component only
//! *paints*: given a single laid-out `GraphRow`, it returns a GPUI
//! `canvas(...)` element that strokes the row's segments, draws the
//! chevron arrows, and paints the commit dot (with a concentric ring
//! for the HEAD commit).
//!
//! The palette comes from `theme.color.graph_palette` — there is **no**
//! rgba literal in this file. Colors are tokens; the IDEA palette
//! itself lives in `theme/colors.rs`.

use gpui::{canvas, point, prelude::*, px, Pixels, PathBuilder, Rgba};
use pier_core::git_graph::GraphRow;

use crate::theme::Theme;

/// Width of a single lane (horizontal distance between adjacent
/// branch columns), matching Pier's `laneW = 14`.
pub const LANE_WIDTH: f32 = 14.0;
/// Height of a single graph row, matching Pier's `rowH = 22`.
pub const ROW_HEIGHT: f32 = 22.0;
/// Radius of the commit dot, matching Pier's `dotR = 3.5`.
pub const DOT_RADIUS: f32 = 3.5;
/// Stroke width of segments & arrows — thin but visible.
pub const STROKE_WIDTH: f32 = 1.5;

/// Detect whether this row is HEAD — refs string contains `HEAD`
/// or an `→ branch` decoration in Pier's convention. Pier-X emits
/// `HEAD -> main` style decorations, so matching on "HEAD" is enough.
pub fn is_head_row(row: &GraphRow) -> bool {
    row.refs.contains("HEAD")
}

/// Pick a palette color by (possibly negative) color index.
/// Wraps into the 0..8 range defensively.
pub fn palette_color(t: &Theme, color_index: i32) -> Rgba {
    let palette = &t.color.graph_palette;
    let idx = color_index.rem_euclid(palette.len() as i32) as usize;
    palette[idx]
}

/// Paint one graph row onto a canvas of exact width/height. Segments,
/// arrows, then the commit dot (HEAD gets an extra ring). Returns
/// a fully-styled canvas element ready to embed in a flex row.
pub fn graph_row_canvas(
    row: &GraphRow,
    t: &Theme,
    col_width: f32,
    dim_factor: f32,
) -> impl gpui::IntoElement {
    // Snapshot everything the paint closure needs to own.
    let segments = row.segments.clone();
    let arrows = row.arrows.clone();
    let node_column = row.node_column;
    let color_index = row.color_index;
    let palette = t.color.graph_palette;
    let head = is_head_row(row);

    let lane_w = LANE_WIDTH;
    let row_h = ROW_HEIGHT;
    let width_px = col_width.max(lane_w);

    canvas(
        move |_bounds, _window, _cx| {},
        move |bounds, _prepaint, window, _cx| {
            let origin = bounds.origin;
            let dim = dim_factor.clamp(0.0, 1.0);

            // 1. Segments
            for seg in &segments {
                let color = apply_alpha(
                    palette[(seg.color_index.rem_euclid(palette.len() as i32)) as usize],
                    dim,
                );
                let mut builder = PathBuilder::stroke(px(STROKE_WIDTH));
                builder.move_to(point(
                    origin.x + px(seg.x_top),
                    origin.y + px(seg.y_top),
                ));
                builder.line_to(point(
                    origin.x + px(seg.x_bottom),
                    origin.y + px(seg.y_bottom),
                ));
                if let Ok(path) = builder.build() {
                    window.paint_path(path, color);
                }
            }

            // 2. Arrows (chevron, ˄ for up / ˅ for down)
            for arr in &arrows {
                let color = apply_alpha(
                    palette[(arr.color_index.rem_euclid(palette.len() as i32)) as usize],
                    dim,
                );
                let arm_len = 5.0_f32;
                let half_w = 4.0_f32;
                let mut builder = PathBuilder::stroke(px(2.0));
                if arr.is_down {
                    // ˅ down chevron
                    builder.move_to(point(
                        origin.x + px(arr.x - half_w),
                        origin.y + px(arr.y - arm_len),
                    ));
                    builder.line_to(point(origin.x + px(arr.x), origin.y + px(arr.y)));
                    builder.line_to(point(
                        origin.x + px(arr.x + half_w),
                        origin.y + px(arr.y - arm_len),
                    ));
                } else {
                    // ˄ up chevron
                    builder.move_to(point(
                        origin.x + px(arr.x - half_w),
                        origin.y + px(arr.y + arm_len),
                    ));
                    builder.line_to(point(origin.x + px(arr.x), origin.y + px(arr.y)));
                    builder.line_to(point(
                        origin.x + px(arr.x + half_w),
                        origin.y + px(arr.y + arm_len),
                    ));
                }
                if let Ok(path) = builder.build() {
                    window.paint_path(path, color);
                }
            }

            // 3. Commit dot
            let dot_color = apply_alpha(
                palette[(color_index.rem_euclid(palette.len() as i32)) as usize],
                dim,
            );
            let cx_pos = node_column as f32 * lane_w + lane_w / 2.0 + 4.0;
            let cy_pos = row_h / 2.0;

            // HEAD: concentric ring around the dot (IDEA style).
            if head {
                if let Some(ring) = circle_stroke(
                    origin.x + px(cx_pos),
                    origin.y + px(cy_pos),
                    DOT_RADIUS + 2.0,
                    1.5,
                ) {
                    window.paint_path(ring, dot_color);
                }
            }
            if let Some(dot) = circle_fill(
                origin.x + px(cx_pos),
                origin.y + px(cy_pos),
                DOT_RADIUS,
            ) {
                window.paint_path(dot, dot_color);
            }

            let _ = width_px; // kept for future hit-testing
        },
    )
    .w(px(width_px))
    .h(px(row_h))
}

// ─── Internal helpers ──────────────────────────────────────

/// Apply a dim factor to a palette color (used by the "highlight"
/// mode so non-matching rows fade instead of disappearing).
fn apply_alpha(base: Rgba, factor: f32) -> Rgba {
    Rgba {
        r: base.r,
        g: base.g,
        b: base.b,
        a: base.a * factor,
    }
}

/// Build a stroked circle path via two cubic Bézier half-arcs.
/// The curve control offset (0.5523 × radius) is the standard
/// quarter-circle cubic approximation.
fn circle_stroke(
    cx: Pixels,
    cy: Pixels,
    radius: f32,
    width: f32,
) -> Option<gpui::Path<Pixels>> {
    let r = radius;
    let k = r * 0.5522847498;
    let mut b = PathBuilder::stroke(px(width));
    b.move_to(point(cx, cy - px(r)));
    b.cubic_bezier_to(
        point(cx + px(r), cy),
        point(cx + px(k), cy - px(r)),
        point(cx + px(r), cy - px(k)),
    );
    b.cubic_bezier_to(
        point(cx, cy + px(r)),
        point(cx + px(r), cy + px(k)),
        point(cx + px(k), cy + px(r)),
    );
    b.cubic_bezier_to(
        point(cx - px(r), cy),
        point(cx - px(k), cy + px(r)),
        point(cx - px(r), cy + px(k)),
    );
    b.cubic_bezier_to(
        point(cx, cy - px(r)),
        point(cx - px(r), cy - px(k)),
        point(cx - px(k), cy - px(r)),
    );
    b.build().ok()
}

/// Filled circle path, using a tessellated polygon fill (16 sides
/// looks smooth enough for a 7-pixel diameter dot).
fn circle_fill(cx: Pixels, cy: Pixels, radius: f32) -> Option<gpui::Path<Pixels>> {
    let r = radius;
    let k = r * 0.5522847498;
    let mut b = PathBuilder::fill();
    b.move_to(point(cx, cy - px(r)));
    b.cubic_bezier_to(
        point(cx + px(r), cy),
        point(cx + px(k), cy - px(r)),
        point(cx + px(r), cy - px(k)),
    );
    b.cubic_bezier_to(
        point(cx, cy + px(r)),
        point(cx + px(r), cy + px(k)),
        point(cx + px(k), cy + px(r)),
    );
    b.cubic_bezier_to(
        point(cx - px(r), cy),
        point(cx - px(k), cy + px(r)),
        point(cx - px(r), cy + px(k)),
    );
    b.cubic_bezier_to(
        point(cx, cy - px(r)),
        point(cx - px(r), cy - px(k)),
        point(cx - px(k), cy - px(r)),
    );
    b.close();
    b.build().ok()
}

/// Compute the pixel width of the graph column for a set of rows so
/// the flex layout can reserve exactly enough space — any less and
/// the rightmost lane clips; any more and the hash/message columns
/// start further right than necessary.
pub fn compute_graph_col_width(rows: &[GraphRow]) -> f32 {
    let mut max_x = 0.0_f32;
    for r in rows {
        let dot_x = r.node_column as f32 * LANE_WIDTH + LANE_WIDTH / 2.0 + 4.0;
        if dot_x > max_x {
            max_x = dot_x;
        }
        for s in &r.segments {
            if s.x_top > max_x {
                max_x = s.x_top;
            }
            if s.x_bottom > max_x {
                max_x = s.x_bottom;
            }
        }
    }
    (max_x + LANE_WIDTH).max(60.0)
}
