//! Platform-specific window chrome policies.
//!
//! Pier-X uses a unified, transparent titlebar on macOS so the
//! traffic-light cluster can sit inside the shell's top rail. That
//! setup regresses on Windows: hiding the native caption strips out
//! the close/minimize/maximize affordances and their system menu
//! semantics unless we reimplement them ourselves.
//!
//! Keep the policy here so every window chooses its chrome the same
//! way: macOS stays unified; Windows falls back to the native title
//! bar; other platforms preserve the current custom-drawn behavior.

use gpui::TitlebarOptions;
use rust_i18n::t;

/// Titlebar policy for the main shell window.
pub fn main_window_titlebar() -> TitlebarOptions {
    #[cfg(target_os = "macos")]
    {
        return TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(gpui::point(gpui::px(12.0), gpui::px(10.0))),
        };
    }

    #[cfg(target_os = "windows")]
    {
        return TitlebarOptions {
            title: Some("Pier-X".into()),
            appears_transparent: false,
            traffic_light_position: None,
        };
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: None,
        }
    }
}

/// Titlebar policy for the standalone settings window.
pub fn settings_window_titlebar() -> TitlebarOptions {
    #[cfg(target_os = "macos")]
    {
        return TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(gpui::point(gpui::px(12.0), gpui::px(14.0))),
        };
    }

    #[cfg(target_os = "windows")]
    {
        return TitlebarOptions {
            title: Some(t!("App.Settings.title").to_string().into()),
            appears_transparent: false,
            traffic_light_position: None,
        };
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: None,
        }
    }
}

/// Windows keeps the native caption, so the in-content faux titlebar
/// should be omitted there.
pub fn shows_embedded_settings_titlebar() -> bool {
    !cfg!(target_os = "windows")
}
