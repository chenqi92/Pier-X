pub mod colors;
pub mod radius;
pub mod shadow;
pub mod spacing;
pub mod terminal;
pub mod typography;

pub use colors::ColorSet;

use gpui::{font, App, Font, FontFeatures, Global, SharedString};
use pier_core::settings::{
    AppSettings, AppearanceMode, SettingsStore, TerminalCursorStyle, TerminalThemePreset,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Clone)]
pub struct Theme {
    pub settings: AppSettings,
    pub mode: ThemeMode,
    pub color: ColorSet,
    pub font_ui: SharedString,
    pub font_mono: SharedString,
}

impl Global for Theme {}

impl Theme {
    pub fn from_settings(settings: AppSettings) -> Self {
        let font_mono = settings_terminal_font_family(&settings);
        let mode = match settings.appearance_mode {
            AppearanceMode::Dark => ThemeMode::Dark,
            AppearanceMode::Light => ThemeMode::Light,
        };
        let color = match mode {
            ThemeMode::Dark => ColorSet::dark(),
            ThemeMode::Light => ColorSet::light(),
        };

        Self {
            settings,
            mode,
            color,
            font_ui: "Inter".into(),
            font_mono: font_mono.into(),
        }
    }
}

pub fn init(cx: &mut App) {
    let settings = SettingsStore::load_default()
        .map(|store| store.settings)
        .unwrap_or_default();
    cx.set_global(Theme::from_settings(settings));
}

pub fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}

pub fn toggle(cx: &mut App) {
    let next_mode = match cx.global::<Theme>().settings.appearance_mode {
        AppearanceMode::Dark => AppearanceMode::Light,
        AppearanceMode::Light => AppearanceMode::Dark,
    };
    update_settings(cx, move |settings| {
        settings.appearance_mode = next_mode;
    });
}

pub fn update_settings(cx: &mut App, update: impl FnOnce(&mut AppSettings)) {
    let previous = cx.global::<Theme>().settings.clone();
    let mut next = previous.clone();
    update(&mut next);
    synchronize_default_terminal_theme(&previous, &mut next);
    normalize_settings(&mut next);

    if let Err(err) = (SettingsStore {
        version: pier_core::settings::CURRENT_SETTINGS_SCHEMA_VERSION,
        settings: next.clone(),
    })
    .save_default()
    {
        eprintln!("[pier-x] failed to persist settings: {err}");
    }

    cx.set_global(Theme::from_settings(next));
    crate::ui_kit::sync_theme(cx);
    cx.refresh_windows();
}

pub fn terminal_font_size(cx: &App) -> f32 {
    cx.global::<Theme>().settings.terminal_font_size as f32
}

pub fn terminal_cursor_style(cx: &App) -> TerminalCursorStyle {
    cx.global::<Theme>().settings.terminal_cursor_style
}

pub fn terminal_cursor_blink(cx: &App) -> bool {
    cx.global::<Theme>().settings.terminal_cursor_blink
}

pub fn terminal_opacity(cx: &App) -> f32 {
    f32::from(cx.global::<Theme>().settings.terminal_opacity_pct) / 100.0
}

pub fn terminal_font_ligatures(cx: &App) -> bool {
    cx.global::<Theme>().settings.terminal_font_ligatures
}

pub fn terminal_font_for_family(family: &SharedString, ligatures: bool) -> Font {
    let mut mono = font(family.clone());
    if !ligatures {
        mono.features = FontFeatures::disable_ligatures();
    }
    mono
}

pub fn available_terminal_font_families() -> &'static [&'static str] {
    &[
        "JetBrains Mono",
        "Cascadia Code",
        "Fira Code",
        "Source Code Pro",
        "Consolas",
        "Menlo",
        "Monaco",
        "Courier New",
    ]
}

fn normalize_settings(settings: &mut AppSettings) {
    settings.terminal_font_size = settings.terminal_font_size.clamp(10, 24);
    settings.terminal_opacity_pct = settings.terminal_opacity_pct.clamp(30, 100);
    if settings.terminal_font_family.trim().is_empty() {
        settings.terminal_font_family = "JetBrains Mono".to_string();
    }
}

fn synchronize_default_terminal_theme(previous: &AppSettings, next: &mut AppSettings) {
    if previous.appearance_mode == next.appearance_mode {
        return;
    }

    next.terminal_theme_preset = match (previous.terminal_theme_preset, next.appearance_mode) {
        (TerminalThemePreset::DefaultDark, AppearanceMode::Light) => {
            TerminalThemePreset::DefaultLight
        }
        (TerminalThemePreset::DefaultLight, AppearanceMode::Dark) => {
            TerminalThemePreset::DefaultDark
        }
        (preset, _) => preset,
    };
}

fn settings_terminal_font_family(settings: &AppSettings) -> String {
    let family = settings.terminal_font_family.trim();
    if family.is_empty() {
        "JetBrains Mono".to_string()
    } else {
        family.to_string()
    }
}
