use std::env;

pub fn init() {
    let locale = detect_locale();
    rust_i18n::set_locale(&locale);
    gpui_component::set_locale(&locale);
    log::info!("i18n locale={locale}");
}

fn detect_locale() -> String {
    for key in ["PIER_X_LOCALE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        let Ok(raw) = env::var(key) else {
            continue;
        };
        if let Some(locale) = normalize_locale(&raw) {
            return locale.to_string();
        }
    }

    "en".to_string()
}

fn normalize_locale(raw: &str) -> Option<&'static str> {
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
        Some("zh-CN")
    } else if normalized.starts_with("en") {
        Some("en")
    } else {
        None
    }
}
