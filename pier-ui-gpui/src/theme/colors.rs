use gpui::{rgb, rgba, Rgba};

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
}

impl ColorSet {
    pub fn dark() -> Self {
        Self {
            bg_canvas: rgb(0x0e0f11),
            bg_panel: rgb(0x16181b),
            bg_surface: rgb(0x1c1e22),
            bg_elevated: rgb(0x22252a),
            bg_hover: rgba(0xffffff_0a),
            bg_active: rgba(0xffffff_0f),
            bg_selected: rgba(0x3574f0_29),

            text_primary: rgb(0xe8eaed),
            text_secondary: rgb(0xb4b8bf),
            text_tertiary: rgb(0x868a91),
            text_disabled: rgb(0x5a5e66),
            text_inverse: rgb(0x16181b),

            border_subtle: rgba(0xffffff_0d),
            border_default: rgba(0xffffff_17),
            border_strong: rgba(0xffffff_24),
            border_focus: rgb(0x3574f0),

            accent: rgb(0x3574f0),
            accent_hover: rgb(0x4f8aff),
            accent_muted: rgba(0x3574f0_29),
            accent_subtle: rgba(0x3574f0_14),

            status_success: rgb(0x5fb865),
            status_warning: rgb(0xf0a83a),
            status_error: rgb(0xfa6675),
            status_info: rgb(0x3574f0),
        }
    }

    pub fn light() -> Self {
        Self {
            bg_canvas: rgb(0xfbfcfd),
            bg_panel: rgb(0xf6f7f9),
            bg_surface: rgb(0xffffff),
            bg_elevated: rgb(0xffffff),
            bg_hover: rgba(0x000000_0a),
            bg_active: rgba(0x000000_0f),
            bg_selected: rgba(0x3574f0_1a),

            text_primary: rgb(0x1e1f22),
            text_secondary: rgb(0x454850),
            text_tertiary: rgb(0x6c707e),
            text_disabled: rgb(0xa7a9b0),
            text_inverse: rgb(0xffffff),

            border_subtle: rgba(0x000000_0f),
            border_default: rgba(0x000000_1a),
            border_strong: rgba(0x000000_2e),
            border_focus: rgb(0x3574f0),

            accent: rgb(0x3574f0),
            accent_hover: rgb(0x4f8aff),
            accent_muted: rgba(0x3574f0_29),
            accent_subtle: rgba(0x3574f0_14),

            status_success: rgb(0x5fb865),
            status_warning: rgb(0xf0a83a),
            status_error: rgb(0xfa6675),
            status_info: rgb(0x3574f0),
        }
    }
}
