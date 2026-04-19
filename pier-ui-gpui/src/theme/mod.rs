pub mod colors;
pub mod heights;
pub mod radius;
pub mod shadow;
pub mod spacing;
pub mod terminal;
pub mod typography;

pub use colors::ColorSet;

use std::sync::Arc;

use gpui::{
    font, App, Font, FontFeatures, FontStyle, FontWeight, Global, SharedString, Window,
    WindowAppearance,
};
use pier_core::settings::{
    AppSettings, AppearanceMode, SettingsStore, TerminalCursorStyle, TerminalThemePreset,
};

pub const DEFAULT_UI_FONT_FAMILY: &str = "Inter";

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
    pub font_ui_features: FontFeatures,
    pub font_mono: SharedString,
}

impl Global for Theme {}

impl Theme {
    pub fn from_settings(settings: AppSettings, appearance: WindowAppearance) -> Self {
        let mut settings = settings;
        let font_mono = settings_terminal_font_family(&settings);
        let font_ui = settings_ui_font_family(&settings);
        let font_ui_features = ui_font_features_for_family(&font_ui);
        let mode = theme_mode_for_settings(&settings, appearance);
        align_default_terminal_theme_with_mode(&mut settings, mode);
        let color = match mode {
            ThemeMode::Dark => ColorSet::dark(),
            ThemeMode::Light => ColorSet::light(),
        };

        Self {
            settings,
            mode,
            color,
            font_ui: font_ui.into(),
            font_ui_features,
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

/// UI font families the settings dialog offers in its picker. The
/// default (`DEFAULT_UI_FONT_FAMILY`) always comes first. Non-Inter
/// families disable Inter-specific OpenType features.
pub fn available_ui_font_families() -> &'static [&'static str] {
    &[
        DEFAULT_UI_FONT_FAMILY,
        "SF Pro",
        "Segoe UI",
        ".SystemUIFont",
    ]
}

/// Build the UI `Font` for a given weight, applying the family and
/// Inter-specific OpenType features that the current Theme has cached.
/// This is a convenience for call sites that already have an `&App`
/// (vs. a `&Theme`); view-level code mostly goes through `ui_font_with`
/// directly and hence this helper is currently unused.
#[allow(dead_code)]
/// Call this once per `text_*` helper / view label — `Styled::font(...)`
/// writes the full font (family + features + weight + style) in one
/// shot, which is required for features like cv01 / ss03 to actually
/// reach the platform text shaper (`Styled::font_family` alone drops
/// features).
pub fn ui_font(cx: &App, weight: FontWeight) -> Font {
    let t = theme(cx);
    ui_font_with(&t.font_ui, &t.font_ui_features, weight)
}

pub fn ui_font_with(family: &SharedString, features: &FontFeatures, weight: FontWeight) -> Font {
    Font {
        family: family.clone(),
        features: features.clone(),
        fallbacks: None,
        weight,
        style: FontStyle::Normal,
    }
}

fn ui_font_features_for_family(family: &str) -> FontFeatures {
    if family.eq_ignore_ascii_case(DEFAULT_UI_FONT_FAMILY) {
        // Inter's "alternate 1/l/I" (cv01) and "single-story a" (ss03)
        // are what make the UI read at a glance — the plan refers to
        // this as the "default tacit layer". Non-Inter fallbacks skip
        // these features so they don't hit an unknown tag.
        FontFeatures(Arc::new(vec![
            ("cv01".to_string(), 1),
            ("ss03".to_string(), 1),
        ]))
    } else {
        FontFeatures::default()
    }
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
    let previous_mode = theme_mode_for_settings(previous, appearance);
    let next_mode = theme_mode_for_settings(next, appearance);
    if previous_mode == next_mode {
        return;
    }

    next.terminal_theme_preset = previous.terminal_theme_preset;
    align_default_terminal_theme_with_mode(next, next_mode);
}

fn align_default_terminal_theme_with_mode(settings: &mut AppSettings, mode: ThemeMode) {
    settings.terminal_theme_preset = match (settings.terminal_theme_preset, mode) {
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

fn settings_ui_font_family(settings: &AppSettings) -> String {
    match settings.ui_font_family.as_deref().map(str::trim) {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => DEFAULT_UI_FONT_FAMILY.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_settings_aligns_default_terminal_theme_with_light_mode() {
        let theme = Theme::from_settings(AppSettings::default(), WindowAppearance::Light);

        assert!(matches!(theme.mode, ThemeMode::Light));
        assert_eq!(
            theme.settings.terminal_theme_preset,
            TerminalThemePreset::DefaultLight
        );
    }

    #[test]
    fn from_settings_keeps_custom_terminal_theme_when_mode_changes() {
        let settings = AppSettings {
            appearance_mode: AppearanceMode::Light,
            terminal_theme_preset: TerminalThemePreset::Dracula,
            ..AppSettings::default()
        };

        let theme = Theme::from_settings(settings, WindowAppearance::Dark);

        assert!(matches!(theme.mode, ThemeMode::Light));
        assert_eq!(
            theme.settings.terminal_theme_preset,
            TerminalThemePreset::Dracula
        );
    }

    #[test]
    fn synchronize_default_terminal_theme_flips_between_default_pairs() {
        let previous = AppSettings {
            appearance_mode: AppearanceMode::Dark,
            terminal_theme_preset: TerminalThemePreset::DefaultDark,
            ..AppSettings::default()
        };
        let mut next = AppSettings {
            appearance_mode: AppearanceMode::Light,
            terminal_theme_preset: TerminalThemePreset::DefaultDark,
            ..AppSettings::default()
        };

        synchronize_default_terminal_theme(&previous, &mut next, WindowAppearance::Dark);

        assert_eq!(
            next.terminal_theme_preset,
            TerminalThemePreset::DefaultLight
        );
    }
}
