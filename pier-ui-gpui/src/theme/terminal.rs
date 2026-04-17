use gpui::{rgb, Hsla};

use crate::theme::ThemeMode;

const ANSI_16: [u32; 16] = [
    0x1c1e22, 0xfa6675, 0x5fb865, 0xf0a83a, 0x3574f0, 0xc678dd, 0x56b6c2, 0xb4b8bf, 0x5a5e66,
    0xff8593, 0x7fcf85, 0xffc15c, 0x5e92ff, 0xd894ed, 0x7fc8d1, 0xe8eaed,
];

pub fn terminal_default_fg_hex(mode: ThemeMode) -> u32 {
    match mode {
        ThemeMode::Dark => 0xe8eaed,
        ThemeMode::Light => 0x1e1f22,
    }
}

pub fn terminal_default_bg_hex(mode: ThemeMode) -> u32 {
    match mode {
        ThemeMode::Dark => 0x0e0f11,
        ThemeMode::Light => 0xfbfcfd,
    }
}

pub fn terminal_cursor_fg_hex(mode: ThemeMode) -> u32 {
    terminal_default_bg_hex(mode)
}

pub fn terminal_cursor_bg_hex(mode: ThemeMode) -> u32 {
    terminal_default_fg_hex(mode)
}

pub fn terminal_selection_fg_hex(mode: ThemeMode) -> u32 {
    match mode {
        ThemeMode::Dark => 0xf4f7fb,
        ThemeMode::Light => 0x16233a,
    }
}

pub fn terminal_selection_bg_hex(mode: ThemeMode) -> u32 {
    match mode {
        ThemeMode::Dark => 0x214283,
        ThemeMode::Light => 0xd8e6ff,
    }
}

pub fn terminal_indexed_hex(index: u8) -> u32 {
    match index {
        0..=15 => ANSI_16[index as usize],
        16..=231 => xterm_cube_hex(index),
        232..=255 => xterm_gray_hex(index),
    }
}

pub fn terminal_hex_color(hex: u32) -> Hsla {
    rgb(hex).into()
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
    fn ansi_palette_keeps_intellij_blue() {
        assert_eq!(terminal_indexed_hex(4), 0x3574f0);
        assert_eq!(terminal_indexed_hex(12), 0x5e92ff);
    }

    #[test]
    fn xterm_cube_colors_expand_after_ansi16() {
        assert_eq!(terminal_indexed_hex(16), 0x000000);
        assert_eq!(terminal_indexed_hex(21), 0x0000ff);
        assert_eq!(terminal_indexed_hex(46), 0x00ff00);
    }

    #[test]
    fn xterm_gray_range_is_monotonic() {
        assert_eq!(terminal_indexed_hex(232), 0x080808);
        assert_eq!(terminal_indexed_hex(255), 0xeeeeee);
    }

    #[test]
    fn selection_palette_stays_legible() {
        assert_eq!(terminal_selection_bg_hex(ThemeMode::Dark), 0x214283);
        assert_eq!(terminal_selection_fg_hex(ThemeMode::Light), 0x16233a);
    }
}
