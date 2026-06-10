// Shared design-system widgets for panels.
//
// Every right-side panel (src/panels/*.rs) builds its chrome from these so the
// look stays consistent: icons tinted via the bundled SVG set, the standard
// panel header, section labels, usage meters, and key/value info rows. All take
// a `&Theme` and reference only token values — never raw colours/sizes.

use gpui::prelude::*;
use gpui::{div, px, relative, svg, FontWeight, Hsla, Pixels, SharedString, Svg};
use gpui_component::{h_flex, v_flex};

use crate::theme::Theme;

/// A bundled lucide SVG, sized and tinted (the SVGs paint with `currentColor`).
/// `name` is the file stem under `assets/icons/` (see src/assets.rs).
pub fn icon(name: &str, sz: Pixels, color: Hsla) -> Svg {
    svg()
        .flex_none()
        .w(sz)
        .h(sz)
        .path(SharedString::from(format!("icons/{name}.svg")))
        .text_color(color)
}

/// Standard panel header: accent glyph + mono title + right-aligned meta.
pub fn panel_header(
    t: &Theme,
    glyph: &'static str,
    title: impl Into<SharedString>,
    meta: impl Into<SharedString>,
) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap(t.sp2)
        .w_full()
        .h(t.panel_header_h)
        .px(t.sp3)
        .border_b_1()
        .border_color(t.line)
        .child(icon(glyph, px(15.0), t.accent))
        .child(
            div()
                .font_family(t.mono.clone())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.ink)
                .child(title.into()),
        )
        .child(div().flex_1())
        .child(div().text_size(t.fs_sm).text_color(t.muted).child(meta.into()))
}

/// Uppercase muted section divider label.
pub fn section_label(t: &Theme, text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .px(t.sp3)
        .pt(t.sp3)
        .pb(t.sp1)
        .text_size(t.fs_sm)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.muted)
        .child(text.into())
}

/// A labelled horizontal usage bar (0–100%), coloured by [`level_color`].
pub fn meter(t: &Theme, label: impl Into<SharedString>, value: impl Into<SharedString>, pct: f64) -> impl IntoElement {
    let frac = (pct.clamp(0.0, 100.0) / 100.0) as f32;
    let color = level_color(t, pct);
    v_flex()
        .gap(px(5.0))
        .px(t.sp3)
        .py(t.sp2)
        .child(
            h_flex()
                .justify_between()
                .child(div().text_size(t.fs_ui).text_color(t.ink_2).child(label.into()))
                .child(
                    div()
                        .font_family(t.mono.clone())
                        .text_size(t.fs_sm)
                        .text_color(t.muted)
                        .child(value.into()),
                ),
        )
        .child(
            div()
                .w_full()
                .h(px(6.0))
                .rounded(px(3.0))
                .bg(t.panel_2)
                .child(div().h_full().w(relative(frac)).rounded(px(3.0)).bg(color)),
        )
}

/// A key/value row: muted label left, mono value right.
pub fn info_row(t: &Theme, label: impl Into<SharedString>, value: impl Into<SharedString>) -> impl IntoElement {
    h_flex()
        .justify_between()
        .px(t.sp3)
        .py(px(3.0))
        .child(div().text_size(t.fs_ui).text_color(t.muted).child(label.into()))
        .child(
            div()
                .font_family(t.mono.clone())
                .text_size(t.fs_sm)
                .text_color(t.ink_2)
                .child(value.into()),
        )
}

/// A small status dot.
pub fn status_dot(color: Hsla) -> impl IntoElement {
    div().w(px(7.0)).h(px(7.0)).rounded_full().bg(color)
}

/// Centered empty/placeholder body for panels with no data yet.
pub fn empty_state(t: &Theme, text: impl Into<SharedString>) -> impl IntoElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(t.sp2)
        .child(div().text_color(t.dim).child(text.into()))
}

/// Usage-bar colour: green under 60%, amber under 85%, red above.
pub fn level_color(t: &Theme, pct: f64) -> Hsla {
    if pct >= 85.0 {
        t.neg
    } else if pct >= 60.0 {
        t.warn
    } else {
        t.pos
    }
}
