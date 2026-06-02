// Pier-X design tokens, ported from src/styles/tokens.css.
//
// This is the Rust mirror of the single source of truth on the React side.
// Values here are copied verbatim from tokens.css (dark `:root` and
// `[data-theme="light"]`). When tokens.css changes, change this too —
// eventually a codegen step should derive one from the other.

use gpui::{px, rgb, rgba, Hsla, Pixels, SharedString};

/// Hex `0xRRGGBB` → Hsla.
fn hex(v: u32) -> Hsla {
    rgb(v).into()
}

/// Hex `0xRRGGBBAA` → Hsla (for the few semi-transparent tokens).
fn hexa(v: u32) -> Hsla {
    rgba(v).into()
}

// Font families. tokens.css wants "IBM Plex Sans" / "IBM Plex Mono", but the
// repo only ships those as .woff2 (font-kit needs .ttf/.otf) and they aren't
// installed system-wide. Until we embed the .ttf, fall back to the Windows
// system faces — clean, present, and good enough to judge layout/colour.
// TODO(M0): embed IBM Plex Sans/Mono .ttf as gpui assets and switch these.
const FONT_SANS: &str = "Segoe UI";
const FONT_MONO: &str = "Consolas";

#[derive(Clone)]
pub struct Theme {
    pub dark: bool,

    // Backgrounds (luminance stacking)
    pub bg: Hsla,
    pub surface: Hsla,
    pub surface_2: Hsla,
    pub panel: Hsla,
    pub panel_2: Hsla,
    pub elev: Hsla,

    // Text
    pub ink: Hsla,
    pub ink_2: Hsla,
    pub muted: Hsla,
    pub dim: Hsla,

    // Borders
    pub line: Hsla,
    pub line_2: Hsla,
    pub line_3: Hsla,

    // Accent
    pub accent: Hsla,
    pub accent_ink: Hsla,
    pub accent_dim: Hsla,
    pub accent_hover: Hsla,
    pub accent_subtle: Hsla,

    // Status
    pub pos: Hsla,
    pub neg: Hsla,
    pub warn: Hsla,
    pub info: Hsla,

    // Fonts
    pub sans: SharedString,
    pub mono: SharedString,

    // Type scale (--size-* / --ui-fs*, ui-scale = 1)
    pub fs_h3: Pixels,
    pub fs_body: Pixels,
    pub fs_ui: Pixels,
    pub fs_sm: Pixels,

    // Spacing (--sp-*)
    pub sp1: Pixels,
    pub sp2: Pixels,
    pub sp3: Pixels,
    pub sp4: Pixels,
    pub sp5: Pixels,
    pub sp6: Pixels,

    // Radius (--radius-*)
    pub radius_sm: Pixels,
    pub radius_md: Pixels,
    pub radius_lg: Pixels,

    // Chrome metrics
    pub titlebar_h: Pixels,
    pub statusbar_h: Pixels,
    pub sidebar_w: Pixels,
    pub toolrail_w: Pixels,
    pub tabbar_h: Pixels,
    pub panel_header_h: Pixels,
    pub rightpanel_w: Pixels,

    // Interaction overlays
    pub hover: Hsla,

    // Service brand tints (--svc-*)
    pub svc_docker: Hsla,
    pub svc_mysql: Hsla,
    pub svc_postgres: Hsla,
    pub svc_redis: Hsla,
    pub svc_monitor: Hsla,
    pub svc_log: Hsla,
    pub svc_sftp: Hsla,
}

impl gpui::Global for Theme {}

impl Theme {
    /// Fields shared across dark/light (typography, spacing, radius, chrome).
    fn shared(dark: bool) -> Self {
        Theme {
            dark,
            // placeholders; colour fields are overwritten by dark()/light()
            bg: hex(0x000000),
            surface: hex(0x000000),
            surface_2: hex(0x000000),
            panel: hex(0x000000),
            panel_2: hex(0x000000),
            elev: hex(0x000000),
            ink: hex(0x000000),
            ink_2: hex(0x000000),
            muted: hex(0x000000),
            dim: hex(0x000000),
            line: hex(0x000000),
            line_2: hex(0x000000),
            line_3: hex(0x000000),
            accent: hex(0x4aa3ff),
            accent_ink: hex(0x000000),
            accent_dim: hex(0x000000),
            accent_hover: hex(0x6eb6ff),
            accent_subtle: hexa(0x4aa3ff14),

            pos: hex(0x3dd68c),
            neg: hex(0xff5a5f),
            warn: hex(0xffb547),
            info: hex(0x7aa2f7),

            sans: FONT_SANS.into(),
            mono: FONT_MONO.into(),

            fs_h3: px(16.0),
            fs_body: px(13.0),
            fs_ui: px(12.0),
            fs_sm: px(11.0),

            sp1: px(4.0),
            sp2: px(8.0),
            sp3: px(12.0),
            sp4: px(16.0),
            sp5: px(20.0),
            sp6: px(24.0),

            radius_sm: px(4.0),
            radius_md: px(6.0),
            radius_lg: px(8.0),

            titlebar_h: px(36.0),
            statusbar_h: px(24.0),
            sidebar_w: px(244.0),
            toolrail_w: px(42.0),
            tabbar_h: px(34.0),
            panel_header_h: px(34.0),
            rightpanel_w: px(360.0),

            // --bg-hover (dark): rgba(255,255,255,0.05). TODO: per-theme.
            hover: hexa(0xffffff0d),

            svc_docker: hex(0x4aa3ff),
            svc_mysql: hex(0xf29d49),
            svc_postgres: hex(0x8fb3ff),
            svc_redis: hex(0xe5484d),
            svc_monitor: hex(0x3dd68c),
            svc_log: hex(0xb48cff),
            svc_sftp: hex(0x8aa0b8),
        }
    }

    pub fn dark() -> Self {
        Theme {
            bg: hex(0x0e1116),
            surface: hex(0x12161d),
            surface_2: hex(0x171c25),
            panel: hex(0x1a202b),
            panel_2: hex(0x222937),
            elev: hex(0x252d3d),

            ink: hex(0xe5e9f0),
            ink_2: hex(0xb9c1cc),
            muted: hex(0x747d8b),
            dim: hex(0x4e5663),

            line: hex(0x242a36),
            line_2: hex(0x2e3542),
            line_3: hex(0x3a4254),

            accent: hex(0x4aa3ff),
            accent_ink: hex(0x0a1420),
            accent_dim: hex(0x1e3a5c),
            accent_hover: hex(0x6eb6ff),
            accent_subtle: hexa(0x4aa3ff14),

            pos: hex(0x3dd68c),
            neg: hex(0xff5a5f),
            warn: hex(0xffb547),
            info: hex(0x7aa2f7),

            ..Self::shared(true)
        }
    }

    pub fn light() -> Self {
        Theme {
            bg: hex(0xf5f3ee),
            surface: hex(0xfbfaf5),
            surface_2: hex(0xf3f1ea),
            panel: hex(0xffffff),
            panel_2: hex(0xf9f7f1),
            elev: hex(0xffffff),

            ink: hex(0x14171d),
            ink_2: hex(0x384050),
            muted: hex(0x6e7585),
            dim: hex(0x9aa0ad),

            line: hex(0xe4e0d4),
            line_2: hex(0xd6d2c4),
            line_3: hex(0xc6c1b0),

            accent: hex(0x4aa3ff),
            accent_ink: hex(0xffffff),
            accent_dim: hex(0xd6e6fa),
            accent_hover: hex(0x6eb6ff),
            accent_subtle: hexa(0x4aa3ff14),

            pos: hex(0x3dd68c),
            neg: hex(0xff5a5f),
            warn: hex(0xffb547),
            info: hex(0x7aa2f7),

            ..Self::shared(false)
        }
    }
}
