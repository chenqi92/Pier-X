use gpui::{rgb, Hsla};
use pier_core::settings::TerminalThemePreset;

#[derive(Clone, Copy)]
pub struct TerminalPalette {
    pub preset: TerminalThemePreset,
    pub name: &'static str,
    pub background_hex: u32,
    pub foreground_hex: u32,
    pub cursor_fg_hex: u32,
    pub cursor_bg_hex: u32,
    pub selection_fg_hex: u32,
    pub selection_bg_hex: u32,
    pub ansi_16: [u32; 16],
}

const DEFAULT_DARK: TerminalPalette = TerminalPalette {
    preset: TerminalThemePreset::DefaultDark,
    name: "Default Dark",
    background_hex: 0x0e0f11,
    foreground_hex: 0xe8eaed,
    cursor_fg_hex: 0x0e0f11,
    cursor_bg_hex: 0xe8eaed,
    selection_fg_hex: 0xf4f7fb,
    selection_bg_hex: 0x214283,
    ansi_16: [
        0x1c1e22, 0xfa6675, 0x5fb865, 0xf0a83a, 0x3574f0, 0xc678dd, 0x56b6c2, 0xb4b8bf, 0x5a5e66,
        0xff8593, 0x7fcf85, 0xffc15c, 0x5e92ff, 0xd894ed, 0x7fc8d1, 0xe8eaed,
    ],
};

const DEFAULT_LIGHT: TerminalPalette = TerminalPalette {
    preset: TerminalThemePreset::DefaultLight,
    name: "Default Light",
    background_hex: 0xfbfcfd,
    foreground_hex: 0x1e1f22,
    cursor_fg_hex: 0xfbfcfd,
    cursor_bg_hex: 0x1e1f22,
    selection_fg_hex: 0x16233a,
    selection_bg_hex: 0xd8e6ff,
    ansi_16: [
        0x1e1f22, 0xc43b3b, 0x2d8f2d, 0xa88b1a, 0x2f63d1, 0x9a33b8, 0x228b99, 0xbec2c8, 0x6c707e,
        0xe15a5a, 0x45a545, 0xc6aa31, 0x4d7df0, 0xb55ad3, 0x39a4b1, 0xf3f5f7,
    ],
};

const SOLARIZED_DARK: TerminalPalette = TerminalPalette {
    preset: TerminalThemePreset::SolarizedDark,
    name: "Solarized Dark",
    background_hex: 0x002b36,
    foreground_hex: 0x839496,
    cursor_fg_hex: 0x002b36,
    cursor_bg_hex: 0x93a1a1,
    selection_fg_hex: 0xfdf6e3,
    selection_bg_hex: 0x073642,
    ansi_16: [
        0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, 0x002b36,
        0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
    ],
};

const DRACULA: TerminalPalette = TerminalPalette {
    preset: TerminalThemePreset::Dracula,
    name: "Dracula",
    background_hex: 0x282a36,
    foreground_hex: 0xf8f8f2,
    cursor_fg_hex: 0x282a36,
    cursor_bg_hex: 0xf8f8f2,
    selection_fg_hex: 0xf8f8f2,
    selection_bg_hex: 0x44475a,
    ansi_16: [
        0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2, 0x6272a4,
        0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
    ],
};

const MONOKAI: TerminalPalette = TerminalPalette {
    preset: TerminalThemePreset::Monokai,
    name: "Monokai",
    background_hex: 0x272822,
    foreground_hex: 0xf8f8f2,
    cursor_fg_hex: 0x272822,
    cursor_bg_hex: 0xf8f8f2,
    selection_fg_hex: 0xf8f8f2,
    selection_bg_hex: 0x49483e,
    ansi_16: [
        0x272822, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf8f8f2, 0x75715e,
        0xfb6c94, 0xc4f966, 0xfce566, 0x8ceeff, 0xcfb9ff, 0xb8fff9, 0xffffff,
    ],
};

const NORD: TerminalPalette = TerminalPalette {
    preset: TerminalThemePreset::Nord,
    name: "Nord",
    background_hex: 0x2e3440,
    foreground_hex: 0xeceff4,
    cursor_fg_hex: 0x2e3440,
    cursor_bg_hex: 0xeceff4,
    selection_fg_hex: 0xeceff4,
    selection_bg_hex: 0x434c5e,
    ansi_16: [
        0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0, 0x4c566a,
        0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
    ],
};

const ALL_PALETTES: [TerminalPalette; 6] = [
    DEFAULT_DARK,
    DEFAULT_LIGHT,
    SOLARIZED_DARK,
    DRACULA,
    MONOKAI,
    NORD,
];

pub fn available_terminal_palettes() -> &'static [TerminalPalette] {
    &ALL_PALETTES
}

pub fn terminal_palette(preset: TerminalThemePreset) -> &'static TerminalPalette {
    match preset {
        TerminalThemePreset::DefaultDark => &DEFAULT_DARK,
        TerminalThemePreset::DefaultLight => &DEFAULT_LIGHT,
        TerminalThemePreset::SolarizedDark => &SOLARIZED_DARK,
        TerminalThemePreset::Dracula => &DRACULA,
        TerminalThemePreset::Monokai => &MONOKAI,
        TerminalThemePreset::Nord => &NORD,
    }
}

pub fn terminal_indexed_hex(palette: &TerminalPalette, index: u8) -> u32 {
    match index {
        0..=15 => palette.ansi_16[index as usize],
        16..=231 => xterm_cube_hex(index),
        232..=255 => xterm_gray_hex(index),
    }
}

pub fn terminal_hex_color(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub fn terminal_bg_color(hex: u32, opacity: f32) -> Hsla {
    terminal_hex_color(hex).opacity(opacity)
}

fn xterm_cube_hex(index: u8) -> u32 {
    let cube = index - 16;
    let r = cube / 36;
    let g = (cube % 36) / 6;
    let b = cube % 6;
    let levels = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];

    ((levels[r as usize] as u32) << 16)
        | ((levels[g as usize] as u32) << 8)
        | levels[b as usize] as u32
}

fn xterm_gray_hex(index: u8) -> u32 {
    let level = 8 + ((index - 232) as u32 * 10);
    (level << 16) | (level << 8) | level
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dark_keeps_intellij_blue() {
        let palette = terminal_palette(TerminalThemePreset::DefaultDark);
        assert_eq!(terminal_indexed_hex(palette, 4), 0x3574f0);
        assert_eq!(terminal_indexed_hex(palette, 12), 0x5e92ff);
    }

    #[test]
    fn xterm_cube_colors_expand_after_ansi16() {
        let palette = terminal_palette(TerminalThemePreset::DefaultDark);
        assert_eq!(terminal_indexed_hex(palette, 16), 0x000000);
        assert_eq!(terminal_indexed_hex(palette, 21), 0x0000ff);
        assert_eq!(terminal_indexed_hex(palette, 46), 0x00ff00);
    }

    #[test]
    fn xterm_gray_range_is_monotonic() {
        let palette = terminal_palette(TerminalThemePreset::DefaultDark);
        assert_eq!(terminal_indexed_hex(palette, 232), 0x080808);
        assert_eq!(terminal_indexed_hex(palette, 255), 0xeeeeee);
    }

    #[test]
    fn palette_lookup_exposes_all_ported_presets() {
        assert_eq!(available_terminal_palettes().len(), 6);
        assert_eq!(
            terminal_palette(TerminalThemePreset::Dracula).name,
            "Dracula"
        );
        assert_eq!(
            terminal_palette(TerminalThemePreset::Nord).background_hex,
            0x2e3440
        );
    }

    #[test]
    fn background_opacity_scales_alpha() {
        let translucent = terminal_bg_color(0x0e0f11, 0.55);
        assert!((translucent.a - 0.55).abs() < f32::EPSILON);
    }
}
