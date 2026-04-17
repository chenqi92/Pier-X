pub mod colors;
pub mod radius;
pub mod shadow;
pub mod spacing;
pub mod terminal;
pub mod typography;

pub use colors::ColorSet;

use gpui::{App, Global, SharedString};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Clone)]
pub struct Theme {
    pub mode: ThemeMode,
    pub color: ColorSet,
    pub font_ui: SharedString,
    pub font_mono: SharedString,
}

impl Global for Theme {}

impl Theme {
    pub fn dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            color: ColorSet::dark(),
            font_ui: "Inter".into(),
            font_mono: "JetBrains Mono".into(),
        }
    }

    pub fn light() -> Self {
        Self {
            mode: ThemeMode::Light,
            color: ColorSet::light(),
            font_ui: "Inter".into(),
            font_mono: "JetBrains Mono".into(),
        }
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(Theme::dark());
}

pub fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}

pub fn toggle(cx: &mut App) {
    let next = match cx.global::<Theme>().mode {
        ThemeMode::Dark => Theme::light(),
        ThemeMode::Light => Theme::dark(),
    };
    cx.set_global(next);
    cx.refresh_windows();
}
