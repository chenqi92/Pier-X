//! Central registry for the app's rebindable keyboard shortcuts.
//!
//! `pier-core` holds the user-assigned keystrokes as an opaque
//! `BTreeMap<String, String>` so the backend stays UI-agnostic. This
//! module is the UI layer's lookup table: it maps an action ID
//! (e.g. `"new_tab"`) to the GPUI `Action` type + its default
//! keystroke + its human-readable label / key context.
//!
//! The same module also drives the "Shortcuts" tab in the settings
//! dialog — both the binding preview and the capture / save flow go
//! through here.

use gpui::{App, KeyBinding, Keystroke, SharedString};
use pier_core::settings::AppSettings;
use rust_i18n::t;

use crate::app::{
    CloseActiveTab, NewTab, OpenSettings, ToggleLeftPanel, ToggleRightPanel, ToggleTheme,
};

/// Stable identifier for every bindable action. Also the key under
/// which the user's override is stored in
/// `AppSettings::keybindings`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActionId {
    ToggleTheme,
    ToggleLeftPanel,
    ToggleRightPanel,
    OpenSettings,
    NewTab,
    CloseActiveTab,
}

impl ActionId {
    /// Full set in display order.
    pub const ALL: [Self; 6] = [
        Self::NewTab,
        Self::CloseActiveTab,
        Self::OpenSettings,
        Self::ToggleLeftPanel,
        Self::ToggleRightPanel,
        Self::ToggleTheme,
    ];

    /// Settings-store key.
    pub fn storage_id(self) -> &'static str {
        match self {
            Self::ToggleTheme => "toggle_theme",
            Self::ToggleLeftPanel => "toggle_left_panel",
            Self::ToggleRightPanel => "toggle_right_panel",
            Self::OpenSettings => "open_settings",
            Self::NewTab => "new_tab",
            Self::CloseActiveTab => "close_active_tab",
        }
    }

    /// Out-of-the-box keystroke string (GPUI 0.2 parser syntax).
    pub fn default_keystroke(self) -> &'static str {
        match self {
            Self::ToggleTheme => "cmd-shift-l",
            Self::ToggleLeftPanel => "cmd-\\",
            Self::ToggleRightPanel => "cmd-shift-\\",
            Self::OpenSettings => "cmd-,",
            Self::NewTab => "cmd-t",
            Self::CloseActiveTab => "cmd-shift-w",
        }
    }

    /// Scope this action applies in. `None` = global (fires anywhere,
    /// e.g. the theme toggle). `Some("PierApp")` = only when the
    /// shell root is focused.
    pub fn key_context(self) -> Option<&'static str> {
        match self {
            Self::ToggleTheme => None,
            _ => Some("PierApp"),
        }
    }

    /// Localized label for the settings dialog.
    pub fn label(self) -> SharedString {
        let key = match self {
            Self::ToggleTheme => "App.Settings.Shortcuts.toggle_theme",
            Self::ToggleLeftPanel => "App.Settings.Shortcuts.toggle_left_panel",
            Self::ToggleRightPanel => "App.Settings.Shortcuts.toggle_right_panel",
            Self::OpenSettings => "App.Settings.Shortcuts.open_settings",
            Self::NewTab => "App.Settings.Shortcuts.new_tab",
            Self::CloseActiveTab => "App.Settings.Shortcuts.close_active_tab",
        };
        t!(key).into()
    }

    /// Build a `KeyBinding` for this action at `keystroke`. `KeyBinding::new`
    /// takes a concrete `A: Action`, so each variant dispatches to its
    /// matching type — we cannot share a single generic path here.
    fn to_binding(self, keystroke: &str) -> KeyBinding {
        let ctx = self.key_context();
        match self {
            Self::ToggleTheme => KeyBinding::new(keystroke, ToggleTheme, ctx),
            Self::ToggleLeftPanel => KeyBinding::new(keystroke, ToggleLeftPanel, ctx),
            Self::ToggleRightPanel => KeyBinding::new(keystroke, ToggleRightPanel, ctx),
            Self::OpenSettings => KeyBinding::new(keystroke, OpenSettings, ctx),
            Self::NewTab => KeyBinding::new(keystroke, NewTab, ctx),
            Self::CloseActiveTab => KeyBinding::new(keystroke, CloseActiveTab, ctx),
        }
    }
}

/// Resolve the effective keystroke for `action` — user override
/// takes priority, otherwise fall back to the built-in default.
pub fn resolved_keystroke(settings: &AppSettings, action: ActionId) -> String {
    settings
        .keybindings
        .get(action.storage_id())
        .cloned()
        .unwrap_or_else(|| action.default_keystroke().to_string())
}

/// Clear and rebind every known action from the current settings.
/// Called at app startup and whenever the Shortcuts tab saves a
/// change.
pub fn apply_all(cx: &mut App, settings: &AppSettings) {
    cx.clear_key_bindings();
    let mut bindings: Vec<KeyBinding> = Vec::with_capacity(ActionId::ALL.len());
    for action in ActionId::ALL {
        let stroke = resolved_keystroke(settings, action);
        bindings.push(action.to_binding(stroke.as_str()));
    }
    cx.bind_keys(bindings);
}

/// Format a captured `Keystroke` as the on-disk string GPUI's
/// `KeyBinding::new` parser accepts. Mirrors Zed's `cmd-shift-p`
/// / `ctrl-alt-h` syntax.
pub fn format_keystroke(keystroke: &Keystroke) -> String {
    let mut parts: Vec<&'static str> = Vec::new();
    if keystroke.modifiers.control {
        parts.push("ctrl");
    }
    if keystroke.modifiers.alt {
        parts.push("alt");
    }
    if keystroke.modifiers.shift {
        parts.push("shift");
    }
    if keystroke.modifiers.platform {
        parts.push("cmd");
    }
    if keystroke.modifiers.function {
        parts.push("fn");
    }
    let mut out = parts.join("-");
    if !out.is_empty() {
        out.push('-');
    }
    out.push_str(keystroke.key.as_str());
    out
}

/// True if the keystroke represents a bare modifier press (shift,
/// ctrl, alt, cmd). These show up while the user is still building
/// a combo; the capture UI ignores them.
pub fn is_modifier_only(keystroke: &Keystroke) -> bool {
    matches!(
        keystroke.key.as_str(),
        "shift" | "control" | "alt" | "option" | "cmd" | "command" | "meta" | "fn" | "function"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn ks(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.into(),
            key_char: None,
        }
    }

    #[test]
    fn format_pure_letter() {
        let out = format_keystroke(&ks("a", Modifiers::default()));
        assert_eq!(out, "a");
    }

    #[test]
    fn format_cmd_shift_p() {
        let mods = Modifiers {
            platform: true,
            shift: true,
            ..Modifiers::default()
        };
        let out = format_keystroke(&ks("p", mods));
        assert_eq!(out, "shift-cmd-p");
    }

    #[test]
    fn defaults_cover_every_action() {
        for action in ActionId::ALL {
            assert!(!action.default_keystroke().is_empty());
            assert!(!action.storage_id().is_empty());
        }
    }

    #[test]
    fn resolved_prefers_override() {
        let mut settings = AppSettings::default();
        settings
            .keybindings
            .insert("new_tab".into(), "cmd-n".into());
        assert_eq!(resolved_keystroke(&settings, ActionId::NewTab), "cmd-n");
        assert_eq!(
            resolved_keystroke(&settings, ActionId::OpenSettings),
            "cmd-,"
        );
    }
}
