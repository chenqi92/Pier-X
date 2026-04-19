use std::env;

use pier_core::settings::AppSettings;

pub const LOCALE_PREFERENCE_SYSTEM: &str = "system";
pub const LOCALE_ENGLISH: &str = "en";
pub const LOCALE_ZH_CN: &str = "zh-CN";

pub fn init() {
    let locale = detect_system_locale();
    apply_locale(&locale);
}

pub fn apply_settings_locale(settings: &AppSettings) -> String {
    let locale = resolve_locale_preference(&settings.ui_locale);
    apply_locale(&locale);
    locale
}

pub fn resolve_locale_preference(raw: &str) -> String {
    let normalized = normalize_locale_preference(raw);
    if normalized == LOCALE_PREFERENCE_SYSTEM {
        detect_system_locale()
    } else {
        normalized
    }
}

pub fn normalize_locale_preference(raw: &str) -> String {
    let normalized = raw.trim().replace('_', "-").to_ascii_lowercase();
    match normalized.as_str() {
        "" | "system" | "auto" | "default" => LOCALE_PREFERENCE_SYSTEM.to_string(),
        "en" | "en-us" | "en-gb" | "english" => LOCALE_ENGLISH.to_string(),
        "zh" | "zh-cn" | "zh-hans" | "zh-sg" | "simplified-chinese" => LOCALE_ZH_CN.to_string(),
        _ => LOCALE_PREFERENCE_SYSTEM.to_string(),
    }
}

fn detect_system_locale() -> String {
    for key in ["PIER_X_LOCALE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        let Ok(raw) = env::var(key) else {
            continue;
        };
        if let Some(locale) = normalize_system_locale(&raw) {
            return locale.to_string();
        }
    }

    LOCALE_ENGLISH.to_string()
}

fn normalize_system_locale(raw: &str) -> Option<&'static str> {
    let normalized = raw
        .split('.')
        .next()
        .unwrap_or(raw)
        .trim()
        .replace('_', "-")
        .to_ascii_lowercase();

    if normalized.is_empty() {
        None
    } else if normalized.starts_with("zh") {
        Some(LOCALE_ZH_CN)
    } else if normalized.starts_with("en") {
        Some(LOCALE_ENGLISH)
    } else {
        None
    }
}

fn apply_locale(locale: &str) {
    rust_i18n::set_locale(locale);
    gpui_component::set_locale(locale);
    log::info!("i18n locale={locale}");
}
