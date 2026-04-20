//! Persisted application settings.
//!
//! This is the cross-platform replacement for the old Pier app's
//! `AppStorage` preferences. The active GPUI shell loads these values at
//! startup and writes them back whenever the user changes settings.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths;

/// Current on-disk schema version for the settings file.
pub const CURRENT_SETTINGS_SCHEMA_VERSION: u32 = 2;

/// UI appearance preference.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceMode {
    /// Follow the operating system appearance.
    #[default]
    System,
    /// Use Pier-X's dark theme.
    Dark,
    /// Use Pier-X's light theme.
    Light,
}

/// Terminal cursor shape preference.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCursorStyle {
    /// Solid block cursor.
    #[default]
    Block,
    /// Thin underline cursor.
    Underline,
    /// Thin bar cursor.
    Bar,
}

/// Terminal palette preset, ported from the sibling Pier app.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TerminalThemePreset {
    /// Pier-X's default dark palette.
    #[default]
    DefaultDark,
    /// Pier-X's default light palette.
    DefaultLight,
    /// Solarized-inspired dark palette.
    SolarizedDark,
    /// Dracula palette.
    Dracula,
    /// Monokai palette.
    Monokai,
    /// Nord palette.
    Nord,
}

/// User-facing application settings that the shell can apply directly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSettings {
    /// Theme preference for the shell chrome.
    #[serde(default)]
    pub appearance_mode: AppearanceMode,
    /// UI locale preference. `"system"` follows the OS locale.
    #[serde(default = "default_ui_locale")]
    pub ui_locale: String,
    /// UI font family name. `None` falls back to the default (Inter).
    /// Setting a non-Inter family disables Inter-specific OpenType features
    /// (cv01, ss03).
    #[serde(default)]
    pub ui_font_family: Option<String>,
    /// Terminal font family name.
    #[serde(default = "default_terminal_font_family")]
    pub terminal_font_family: String,
    /// Terminal font size in points / px-equivalent UI units.
    #[serde(default = "default_terminal_font_size")]
    pub terminal_font_size: u16,
    /// Cursor shape inside the terminal.
    #[serde(default)]
    pub terminal_cursor_style: TerminalCursorStyle,
    /// Whether the terminal cursor should blink.
    #[serde(default = "default_cursor_blink")]
    pub terminal_cursor_blink: bool,
    /// Terminal palette preset.
    #[serde(default)]
    pub terminal_theme_preset: TerminalThemePreset,
    /// Terminal surface opacity as an integer percentage.
    #[serde(default = "default_terminal_opacity_pct")]
    pub terminal_opacity_pct: u8,
    /// Whether terminal ligatures should stay enabled.
    #[serde(default = "default_terminal_font_ligatures")]
    pub terminal_font_ligatures: bool,
    /// Install Pier-X's shell-integration rc into the user's local
    /// `~/.bashrc` / `~/.zshrc` so OSC 7 cwd reporting + `ssh`
    /// hijacking work for manual `ssh` commands typed in a local
    /// shell tab. Off by default — writing to dotfiles is an opt-in
    /// action. Mirrored (no-op) across platforms where shell rc
    /// injection doesn't apply.
    #[serde(default)]
    pub terminal_shell_integration: bool,
    /// Height (in integer logical pixels) of the Git panel's commit
    /// input footer. Persisted across restarts so the splitter
    /// position the user dragged is preserved. Default 120 px
    /// matches Pier. Stored as `u16` to keep `AppSettings: Eq`
    /// intact; the GPUI shell restores values within `80..=300` and
    /// falls back to 120 for legacy out-of-range values.
    #[serde(default = "default_git_footer_height")]
    pub git_footer_height: u16,
    /// User-assigned keystrokes, keyed by a UI-layer action ID (e.g.
    /// `"new_tab"`, `"toggle_left_panel"`). An empty map falls back to
    /// the UI layer's built-in defaults. The value format is whatever
    /// the UI layer's `KeyBinding` parser accepts (GPUI 0.2 uses
    /// `"cmd-shift-l"` style).
    ///
    /// We deliberately keep the action ID as a plain `String` here so
    /// pier-core stays UI-agnostic — it doesn't know what a
    /// `ToggleTheme` action is.
    #[serde(default)]
    pub keybindings: BTreeMap<String, String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            git_footer_height: default_git_footer_height(),
            appearance_mode: AppearanceMode::System,
            ui_locale: default_ui_locale(),
            ui_font_family: None,
            terminal_font_family: default_terminal_font_family(),
            terminal_font_size: default_terminal_font_size(),
            terminal_cursor_style: TerminalCursorStyle::Block,
            terminal_cursor_blink: default_cursor_blink(),
            terminal_theme_preset: TerminalThemePreset::DefaultDark,
            terminal_opacity_pct: default_terminal_opacity_pct(),
            terminal_font_ligatures: default_terminal_font_ligatures(),
            terminal_shell_integration: false,
            keybindings: BTreeMap::new(),
        }
    }
}

/// Top-level on-disk settings document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingsStore {
    /// Schema version stamped on disk.
    pub version: u32,
    /// Actual settings payload.
    #[serde(default)]
    pub settings: AppSettings,
}

impl Default for SettingsStore {
    fn default() -> Self {
        Self {
            version: CURRENT_SETTINGS_SCHEMA_VERSION,
            settings: AppSettings::default(),
        }
    }
}

impl SettingsStore {
    /// Load the persisted settings from the standard app config location.
    ///
    /// Missing file returns the default settings.
    pub fn load_default() -> Result<Self, SettingsStoreError> {
        let path = paths::settings_file().ok_or(SettingsStoreError::NoConfigDir)?;
        Self::load_from_path(&path)
    }

    /// Load settings from an explicit path.
    pub fn load_from_path(path: &Path) -> Result<Self, SettingsStoreError> {
        match fs::read(path) {
            Ok(bytes) => {
                let store: Self = serde_json::from_slice(&bytes)?;
                if store.version > CURRENT_SETTINGS_SCHEMA_VERSION {
                    return Err(SettingsStoreError::FutureVersion {
                        found: store.version,
                        supported: CURRENT_SETTINGS_SCHEMA_VERSION,
                    });
                }
                Ok(store)
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    /// Save the current settings to the standard app config location.
    pub fn save_default(&self) -> Result<(), SettingsStoreError> {
        let path = paths::settings_file().ok_or(SettingsStoreError::NoConfigDir)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        self.save_to_path(&path)
    }

    /// Save settings to an explicit path, atomically.
    pub fn save_to_path(&self, path: &Path) -> Result<(), SettingsStoreError> {
        let stamped = Self {
            version: CURRENT_SETTINGS_SCHEMA_VERSION,
            settings: self.settings.clone(),
        };
        let json = serde_json::to_vec_pretty(&stamped)?;

        let tmp_path = tmp_path_for(path);
        fs::write(&tmp_path, json)?;
        fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

/// Errors that can occur loading or saving the settings file.
#[derive(Debug, thiserror::Error)]
pub enum SettingsStoreError {
    /// I/O error reading or writing the file.
    #[error("settings store I/O: {0}")]
    Io(#[from] io::Error),

    /// JSON parse error.
    #[error("settings store JSON: {0}")]
    Json(#[from] serde_json::Error),

    /// The stored schema version is newer than this build understands.
    #[error("settings store version {found} > supported {supported}")]
    FutureVersion {
        /// Version on disk.
        found: u32,
        /// Highest version supported by this build.
        supported: u32,
    },

    /// No usable config directory could be resolved.
    #[error("no usable application config directory")]
    NoConfigDir,
}

fn default_terminal_font_family() -> String {
    "JetBrains Mono".to_string()
}

fn default_ui_locale() -> String {
    "zh-CN".to_string()
}

fn default_terminal_font_size() -> u16 {
    13
}

fn default_cursor_blink() -> bool {
    true
}

fn default_terminal_opacity_pct() -> u8 {
    100
}

fn default_terminal_font_ligatures() -> bool {
    false
}

fn default_git_footer_height() -> u16 {
    120
}

fn tmp_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    match path.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

#[cfg(test)]
mod tests {
    use std::env::temp_dir;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn fresh_tmp(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        temp_dir().join(format!("pier-x-test-settings-{label}-{pid}-{n}.json"))
    }

    #[test]
    fn defaults_match_current_gpui_shell() {
        let settings = AppSettings::default();

        assert_eq!(settings.appearance_mode, AppearanceMode::System);
        assert_eq!(settings.ui_locale, "zh-CN");
        assert_eq!(settings.terminal_font_family, "JetBrains Mono");
        assert_eq!(settings.terminal_font_size, 13);
        assert_eq!(settings.terminal_cursor_style, TerminalCursorStyle::Block);
        assert!(settings.terminal_cursor_blink);
        assert_eq!(
            settings.terminal_theme_preset,
            TerminalThemePreset::DefaultDark
        );
        assert_eq!(settings.terminal_opacity_pct, 100);
        assert!(!settings.terminal_font_ligatures);
    }

    #[test]
    fn round_trips_settings_file() {
        let path = fresh_tmp("roundtrip");
        let store = SettingsStore {
            version: CURRENT_SETTINGS_SCHEMA_VERSION,
            settings: AppSettings {
                appearance_mode: AppearanceMode::Light,
                ui_locale: "zh-CN".into(),
                ui_font_family: Some("SF Pro".into()),
                terminal_font_family: "Cascadia Code".into(),
                terminal_font_size: 15,
                terminal_cursor_style: TerminalCursorStyle::Underline,
                terminal_cursor_blink: false,
                terminal_theme_preset: TerminalThemePreset::Dracula,
                terminal_opacity_pct: 85,
                terminal_font_ligatures: true,
                terminal_shell_integration: true,
                git_footer_height: 140,
                keybindings: BTreeMap::from([
                    ("new_tab".to_string(), "cmd-n".to_string()),
                    ("toggle_theme".to_string(), "cmd-shift-l".to_string()),
                ]),
            },
        };

        store.save_to_path(&path).expect("save settings");
        let loaded = SettingsStore::load_from_path(&path).expect("load settings");

        assert_eq!(loaded, store);
        let _ = fs::remove_file(path);
    }
}
