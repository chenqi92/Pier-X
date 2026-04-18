pub mod colors;
pub mod radius;
pub mod shadow;
pub mod spacing;
pub mod terminal;
pub mod typography;

pub use colors::ColorSet;

use gpui::{font, App, Font, FontFeatures, Global, SharedString, Window, WindowAppearance};
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
    pub fn from_settings(settings: AppSettings, appearance: WindowAppearance) -> Self {
        let font_mono = settings_terminal_font_family(&settings);
        let mode = theme_mode_for_settings(&settings, appearance);
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
    let mut settings = SettingsStore::load_default()
        .map(|store| store.settings)
        .unwrap_or_default();
    normalize_settings(&mut settings);
    crate::i18n::apply_settings_locale(&settings);
    cx.set_global(Theme::from_settings(settings, cx.window_appearance()));
}

pub fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}

/// Clone of the currently applied settings payload — handy when
/// bootstrapping code outside the `Theme` machinery (e.g.
/// `keybindings::apply_all` at startup) needs a read-only snapshot.
pub fn current_settings(cx: &App) -> AppSettings {
    cx.global::<Theme>().settings.clone()
}

pub fn toggle(cx: &mut App) {
    let next_mode = match cx.global::<Theme>().mode {
        ThemeMode::Dark => AppearanceMode::Light,
        ThemeMode::Light => AppearanceMode::Dark,
    };
    update_settings(cx, move |settings| {
        settings.appearance_mode = next_mode;
    });
}

pub fn sync_system_appearance(window: Option<&mut Window>, cx: &mut App) {
    if !cx.has_global::<Theme>() {
        return;
    }

    let settings = cx.global::<Theme>().settings.clone();
    if settings.appearance_mode != AppearanceMode::System {
        return;
    }

    let appearance = window
        .as_ref()
        .map(|window| window.appearance())
        .unwrap_or_else(|| cx.window_appearance());
    let next_mode = theme_mode_for_settings(&settings, appearance);

    if cx.global::<Theme>().mode == next_mode {
        return;
    }

    cx.set_global(Theme::from_settings(settings, appearance));
    crate::ui_kit::sync_theme(cx);
    cx.refresh_windows();
}

pub fn update_settings(cx: &mut App, update: impl FnOnce(&mut AppSettings)) {
    let appearance = cx.window_appearance();
    let previous = cx.global::<Theme>().settings.clone();
    let mut next = previous.clone();
    update(&mut next);
    synchronize_default_terminal_theme(&previous, &mut next, appearance);
    normalize_settings(&mut next);
    crate::i18n::apply_settings_locale(&next);

    if let Err(err) = (SettingsStore {
        version: pier_core::settings::CURRENT_SETTINGS_SCHEMA_VERSION,
        settings: next.clone(),
    })
    .save_default()
    {
        eprintln!("[pier-x] failed to persist settings: {err}");
    }

    let next_for_binds = next.clone();
    cx.set_global(Theme::from_settings(next, appearance));
    crate::ui_kit::sync_theme(cx);
    // Rebind keyboard shortcuts — if the caller just changed one,
    // the new binding should take effect immediately. For other
    // setting changes this is a no-op (same bindings reapplied).
    crate::app::keybindings::apply_all(cx, &next_for_binds);
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
    settings.ui_locale = crate::i18n::normalize_locale_preference(&settings.ui_locale);
    settings.terminal_font_size = settings.terminal_font_size.clamp(10, 24);
    settings.terminal_opacity_pct = settings.terminal_opacity_pct.clamp(30, 100);
    if settings.terminal_font_family.trim().is_empty() {
        settings.terminal_font_family = "JetBrains Mono".to_string();
    }
}

fn synchronize_default_terminal_theme(
    previous: &AppSettings,
    next: &mut AppSettings,
    appearance: WindowAppearance,
) {
    if theme_mode_for_settings(previous, appearance) == theme_mode_for_settings(next, appearance) {
        return;
    }

    next.terminal_theme_preset = match (
        previous.terminal_theme_preset,
        theme_mode_for_settings(next, appearance),
    ) {
        (TerminalThemePreset::DefaultDark, ThemeMode::Light) => TerminalThemePreset::DefaultLight,
        (TerminalThemePreset::DefaultLight, ThemeMode::Dark) => TerminalThemePreset::DefaultDark,
        (preset, _) => preset,
    };
}

fn theme_mode_for_settings(settings: &AppSettings, appearance: WindowAppearance) -> ThemeMode {
    match settings.appearance_mode {
        AppearanceMode::System => theme_mode_for_window_appearance(appearance),
        AppearanceMode::Dark => ThemeMode::Dark,
        AppearanceMode::Light => ThemeMode::Light,
    }
}

fn theme_mode_for_window_appearance(appearance: WindowAppearance) -> ThemeMode {
    match appearance {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeMode::Dark,
        WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeMode::Light,
    }
}

fn settings_terminal_font_family(settings: &AppSettings) -> String {
    let family = settings.terminal_font_family.trim();
    if family.is_empty() {
        "JetBrains Mono".to_string()
    } else {
        family.to_string()
    }
}
