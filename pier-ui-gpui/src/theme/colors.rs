#![allow(dead_code)]

use gpui::{rgb, rgba, Hsla, Rgba};

use crate::theme::ThemeMode;

/// IDEA-style 8-color palette used by the Git commit-graph component.
/// Literals live in the theme module only, per the component rules.
const IDEA_GRAPH_PALETTE: [Rgba; 8] = [
    Rgba {
        r: 0.27,
        g: 0.69,
        b: 0.35,
        a: 1.0,
    }, // 0 = main (green)
    Rgba {
        r: 0.25,
        g: 0.58,
        b: 0.96,
        a: 1.0,
    }, // blue
    Rgba {
        r: 0.87,
        g: 0.42,
        b: 0.12,
        a: 1.0,
    }, // orange
    Rgba {
        r: 0.68,
        g: 0.35,
        b: 0.82,
        a: 1.0,
    }, // purple
    Rgba {
        r: 0.94,
        g: 0.33,
        b: 0.31,
        a: 1.0,
    }, // red
    Rgba {
        r: 0.16,
        g: 0.71,
        b: 0.76,
        a: 1.0,
    }, // teal
    Rgba {
        r: 0.89,
        g: 0.68,
        b: 0.12,
        a: 1.0,
    }, // yellow
    Rgba {
        r: 0.85,
        g: 0.35,
        b: 0.60,
        a: 1.0,
    }, // pink
];

#[derive(Clone, Copy)]
pub struct ColorSet {
    pub bg_canvas: Rgba,
    pub bg_panel: Rgba,
    pub bg_surface: Rgba,
    pub bg_elevated: Rgba,
    pub bg_hover: Rgba,
    pub bg_active: Rgba,
    pub bg_selected: Rgba,

    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub text_tertiary: Rgba,
    pub text_disabled: Rgba,
    pub text_inverse: Rgba,

    pub border_subtle: Rgba,
    pub border_default: Rgba,
    pub border_strong: Rgba,
    pub border_focus: Rgba,

    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub accent_muted: Rgba,
    pub accent_subtle: Rgba,

    pub status_success: Rgba,
    pub status_warning: Rgba,
    pub status_error: Rgba,
    pub status_info: Rgba,

    /// IDEA-style 8-color graph lane palette. Index 0 = the repo's
    /// main branch (green); 1..7 = auxiliary branches. Matches the
    /// sibling Pier app exactly so a user switching between Pier
    /// and Pier-X sees identical graph colouring.
    pub graph_palette: [Rgba; 8],
}

impl ColorSet {
    pub fn dark() -> Self {
        Self {
            bg_canvas: rgb(0x0e0f11),
            bg_panel: rgb(0x16181b),
            bg_surface: rgb(0x1c1e22),
            bg_elevated: rgb(0x22252a),
            bg_hover: rgba(0xffff_ff0a),
            bg_active: rgba(0xffff_ff0f),
            // Alpha 0x1f (~0.12) matches SwiftUI `.accentColor.opacity(0.12)`
            // the reference app uses for selected sidebar/list rows.
            bg_selected: rgba(0x3574_f01f),

            text_primary: rgb(0xe8eaed),
            text_secondary: rgb(0xb4b8bf),
            text_tertiary: rgb(0x868a91),
            text_disabled: rgb(0x5a5e66),
            text_inverse: rgb(0x16181b),

            // Bumped 0x0d → 0x14 so 1px hairlines stay visible — GPUI
            // cannot render 0.5px borders, so we compensate with alpha.
            border_subtle: rgba(0xffff_ff14),
            border_default: rgba(0xffff_ff1f),
            border_strong: rgba(0xffff_ff2e),
            border_focus: rgb(0x3574f0),

            accent: rgb(0x3574f0),
            accent_hover: rgb(0x4f8aff),
            accent_muted: rgba(0x3574_f029),
            accent_subtle: rgba(0x3574_f014),

            status_success: rgb(0x5fb865),
            status_warning: rgb(0xf0a83a),
            status_error: rgb(0xfa6675),
            status_info: rgb(0x3574f0),
            graph_palette: IDEA_GRAPH_PALETTE,
        }
    }

    pub fn light() -> Self {
        Self {
            bg_canvas: rgb(0xfbfcfd),
            bg_panel: rgb(0xf6f7f9),
            bg_surface: rgb(0xffffff),
            bg_elevated: rgb(0xffffff),
            bg_hover: rgba(0x0000_000a),
            bg_active: rgba(0x0000_000f),
            bg_selected: rgba(0x3574_f01a),

            text_primary: rgb(0x1e1f22),
            text_secondary: rgb(0x454850),
            text_tertiary: rgb(0x6c707e),
            text_disabled: rgb(0xa7a9b0),
            text_inverse: rgb(0xffffff),

            // Bumped 0x0f → 0x17 so 1px hairlines stay visible on
            // bright canvas (same reasoning as dark mode).
            border_subtle: rgba(0x0000_0017),
            border_default: rgba(0x0000_0022),
            border_strong: rgba(0x0000_0036),
            border_focus: rgb(0x3574f0),

            accent: rgb(0x3574f0),
            accent_hover: rgb(0x4f8aff),
            accent_muted: rgba(0x3574_f029),
            accent_subtle: rgba(0x3574_f014),

            status_success: rgb(0x5fb865),
            status_warning: rgb(0xf0a83a),
            status_error: rgb(0xfa6675),
            status_info: rgb(0x3574f0),
            graph_palette: IDEA_GRAPH_PALETTE,
        }
    }

    /// Re-tint every accent-derived slot (`accent`, `accent_hover`,
    /// `accent_muted`, `accent_subtle`, `border_focus`, `bg_selected`,
    /// `status_info`) with a platform-supplied accent RGB.
    ///
    /// Per SKILL.md §2 the `single chromatic accent` rule still holds:
    /// the system just tells us *which* color fills the accent slot
    /// at runtime. Alpha relationships (muted=0.16, subtle=0.08,
    /// bg_selected=0.12 dark / 0.10 light) are preserved.
    pub fn with_system_accent(mut self, (r, g, b): (u8, u8, u8), mode: ThemeMode) -> Self {
        let base = Rgba {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: 1.0,
        };
        let hover = brighten(base, 0.12);
        let muted = with_alpha(base, 0.16);
        let subtle = with_alpha(base, 0.08);
        let selected_alpha = match mode {
            ThemeMode::Dark => 0.12,
            ThemeMode::Light => 0.10,
        };

        self.accent = base;
        self.accent_hover = hover;
        self.accent_muted = muted;
        self.accent_subtle = subtle;
        self.border_focus = base;
        self.bg_selected = with_alpha(base, selected_alpha);
        self.status_info = base;
        self
    }
}

fn with_alpha(color: Rgba, alpha: f32) -> Rgba {
    Rgba { a: alpha, ..color }
}

/// Shift the accent one step lighter for hover, independent of hue.
/// Converts via HSLA so we don't bleach saturated system accents
/// (e.g. macOS "Graphite" would go white under a pure-RGB lighten).
fn brighten(color: Rgba, amount: f32) -> Rgba {
    let hsla: Hsla = color.into();
    let next = Hsla {
        l: (hsla.l + amount).clamp(0.0, 1.0),
        ..hsla
    };
    let out: Rgba = next.into();
    out
}
