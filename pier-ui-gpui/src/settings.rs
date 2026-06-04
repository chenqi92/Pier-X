// Pier-X GPUI spike — Settings view.
//
// An independent View hosted by shell.rs's overlay layer (see
// `Shell::overlay_layer`). It owns its own page state and reads the
// values it shows straight from `crate::data` / the global `Theme`, so
// shell.rs only has to construct it once and render it.
//
// Appearance is interactive (it flips the global theme). The other
// pages surface the real values the app is running with — fonts from
// the design tokens, the saved connection list, the resolved git
// identity, and the bound keyboard shortcuts — rather than fabricating
// editable controls the spike has no plumbing to persist.

use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, FontWeight, MouseButton, MouseDownEvent, SharedString, Window};
use gpui_component::{h_flex, v_flex};

use pier_core::ssh::AuthMethod;

use crate::data;
use crate::theme::Theme;
use crate::ui;

/// One settings page.
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Appearance,
    Typography,
    Terminal,
    Editor,
    Connections,
    Profiles,
    Git,
    SshKeys,
    Keymap,
    Diagnostics,
    Privacy,
    Security,
    General,
    About,
}

/// Left-nav entries: (page, icon stem, label).
const PAGES: &[(Page, &str, &str)] = &[
    (Page::Appearance, "sun", "Appearance"),
    (Page::Typography, "a-large-small", "Typography"),
    (Page::Terminal, "square-terminal", "Terminal"),
    (Page::Editor, "file-text", "Editor"),
    (Page::Connections, "server", "Connections"),
    (Page::Profiles, "user", "Profiles"),
    (Page::Git, "git-branch", "Git"),
    (Page::SshKeys, "asterisk", "SSH Keys"),
    (Page::Keymap, "command", "Keymap"),
    (Page::Diagnostics, "activity", "Diagnostics"),
    (Page::Privacy, "eye-off", "Privacy"),
    (Page::Security, "shield", "Security"),
    (Page::General, "settings-2", "General"),
    (Page::About, "info", "About"),
];

/// The keyboard shortcuts bound in main.rs: (chord, action).
const KEYMAP: &[(&str, &str)] = &[
    ("Ctrl+Shift+P", "Command Palette"),
    ("Ctrl+Shift+T", "New Terminal"),
    ("Ctrl+Shift+W", "Close Tab"),
    ("Ctrl+Shift+L", "Toggle Theme"),
    ("Ctrl+,", "Settings"),
];

pub struct SettingsView {
    page: Page,
    conns: Vec<data::ConnRow>,
    git_name: String,
    git_email: String,
    repo: String,
    theme: Theme,
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let cwd = data::current_dir();
        let (git_name, git_email) = data::git_identity(&cwd);
        Self {
            page: Page::Appearance,
            conns: data::load_connections(),
            git_name,
            git_email,
            repo: cwd.display().to_string(),
            theme: cx.global::<Theme>().clone(),
        }
    }

    /// Re-read the data shown (called by the shell when the overlay opens).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let cwd = data::current_dir();
        let (n, e) = data::git_identity(&cwd);
        self.conns = data::load_connections();
        self.git_name = n;
        self.git_email = e;
        self.repo = cwd.display().to_string();
        cx.notify();
    }

    fn nav_item(
        &self,
        cx: &mut Context<Self>,
        page: Page,
        glyph: &'static str,
        label: &'static str,
    ) -> impl IntoElement {
        let t = &self.theme;
        let active = self.page == page;
        h_flex()
            .id(SharedString::from(format!("set-nav-{label}")))
            .items_center()
            .gap(t.sp2)
            .h(px(30.0))
            .px(t.sp2)
            .rounded(t.radius_sm)
            .cursor_pointer()
            .when(active, |d| d.bg(t.accent_dim))
            .when(!active, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.page = page;
                    cx.notify();
                }),
            )
            .child(ui::icon(glyph, px(14.0), if active { t.accent } else { t.muted }))
            .child(
                div()
                    .text_size(t.fs_ui)
                    .text_color(if active { t.ink } else { t.ink_2 })
                    .child(label),
            )
    }

    fn theme_btn(&self, cx: &mut Context<Self>, label: &'static str, want_dark: bool) -> impl IntoElement {
        let t = &self.theme;
        let active = t.dark == want_dark;
        div()
            .id(SharedString::from(format!("set-theme-{label}")))
            .px(t.sp3)
            .py(px(5.0))
            .rounded(t.radius_sm)
            .text_size(t.fs_ui)
            .cursor_pointer()
            .when(active, |d| d.bg(t.accent).text_color(t.accent_ink))
            .when(!active, |d| d.bg(t.panel_2).text_color(t.ink_2))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _: &MouseDownEvent, window, cx| {
                    if cx.global::<Theme>().dark != want_dark {
                        cx.set_global(if want_dark { Theme::dark() } else { Theme::light() });
                        // Persist so the choice survives a restart, mirroring the
                        // shell's ToggleTheme. Load-modify-save keeps the other
                        // UiState fields (layout/cwd) the SettingsView doesn't hold.
                        let mut s = data::load_ui_state();
                        s.dark = want_dark;
                        data::save_ui_state(&s);
                        window.refresh();
                    }
                }),
            )
            .child(label)
    }

    /// A `name — addr` connection row for the Connections page.
    fn conn_line(&self, name: &str, addr: &str) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .py(px(3.0))
            .child(div().flex_1().min_w(px(0.0)).overflow_hidden().text_color(t.ink_2).child(name.to_string()))
            .child(
                div()
                    .flex_none()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(addr.to_string()),
            )
    }

    fn content(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let dash = |s: &str| if s.is_empty() { "—".to_string() } else { s.to_string() };
        // A full-width dim prose line, the same chrome the existing read-only
        // pages use for their "not configurable in this build" notes.
        let note = |s: &str| {
            div()
                .px(t.sp3)
                .py(t.sp2)
                .text_size(t.fs_sm)
                .text_color(t.dim)
                .child(s.to_string())
        };
        match self.page {
            Page::Appearance => v_flex()
                .child(ui::section_label(t, "THEME"))
                .child(
                    h_flex()
                        .gap(t.sp2)
                        .px(t.sp3)
                        .py(t.sp1)
                        .child(self.theme_btn(cx, "Dark", true))
                        .child(self.theme_btn(cx, "Light", false)),
                )
                .child(ui::section_label(t, "ACCENT"))
                .child(ui::info_row(t, "Color", "Pier Blue"))
                .into_any_element(),
            Page::Typography => v_flex()
                .child(ui::section_label(t, "FONTS"))
                .child(ui::info_row(t, "Sans", self.theme.sans.clone()))
                .child(ui::info_row(t, "Mono", self.theme.mono.clone()))
                .child(ui::section_label(t, "SIZE"))
                .child(ui::info_row(t, "Heading", format!("{:.0}px", f32::from(t.fs_h3))))
                .child(ui::info_row(t, "Body", format!("{:.0}px", f32::from(t.fs_body))))
                .child(ui::info_row(t, "UI", format!("{:.0}px", f32::from(t.fs_ui))))
                .child(ui::info_row(t, "Small", format!("{:.0}px", f32::from(t.fs_sm))))
                .into_any_element(),
            Page::Terminal => v_flex()
                .child(ui::section_label(t, "LOCAL SHELL"))
                .child(ui::info_row(t, "Program", "powershell.exe"))
                .child(ui::info_row(t, "Cursor", "Block"))
                .child(ui::info_row(t, "Scrollback", "pier-core emulator"))
                .child(
                    div()
                        .px(t.sp3)
                        .py(t.sp2)
                        .text_size(t.fs_sm)
                        .text_color(t.dim)
                        .child("Cursor / scrollback / bell are not configurable in this build."),
                )
                .into_any_element(),
            Page::Editor => v_flex()
                .child(ui::section_label(t, "SFTP FILE EDITOR"))
                .child(note(
                    "Wrap, line numbers, tab width and on-save trimming live here in the full app. This build has no in-app file editor to configure.",
                ))
                .into_any_element(),
            Page::Connections => {
                let mut col =
                    v_flex().child(ui::section_label(t, format!("SAVED · {}", self.conns.len())));
                if self.conns.is_empty() {
                    col = col.child(
                        div()
                            .px(t.sp3)
                            .py(t.sp2)
                            .text_size(t.fs_sm)
                            .text_color(t.dim)
                            .child("No saved connections"),
                    );
                } else {
                    for c in &self.conns {
                        col = col.child(self.conn_line(&c.name, &c.addr));
                    }
                }
                col.into_any_element()
            }
            Page::Profiles => v_flex()
                .child(ui::section_label(t, "TERMINAL PROFILES"))
                .child(ui::info_row(t, "Default shell", "powershell.exe"))
                .child(note(
                    "New terminals launch the default shell. Saved launch profiles (working directory and startup command) aren't configurable in this build.",
                ))
                .into_any_element(),
            Page::Git => v_flex()
                .child(ui::section_label(t, "IDENTITY"))
                .child(ui::info_row(t, "User name", dash(&self.git_name)))
                .child(ui::info_row(t, "Email", dash(&self.git_email)))
                .child(ui::section_label(t, "REPOSITORY"))
                .child(ui::info_row(t, "Path", self.repo.clone()))
                .into_any_element(),
            Page::SshKeys => {
                // Key paths drawn from saved connections that authenticate with a
                // private-key file. Read-only: this build lists what the saved
                // profiles reference; it neither scans ~/.ssh nor manages keys.
                let mut keys: Vec<(String, String)> = Vec::new();
                for c in data::connections_raw() {
                    let path = match &c.auth {
                        AuthMethod::PublicKeyFile {
                            private_key_path, ..
                        } => Some(private_key_path.clone()),
                        AuthMethod::AutoChain {
                            explicit_key_path: Some(p),
                            ..
                        } => Some(p.clone()),
                        _ => None,
                    };
                    if let Some(path) = path {
                        keys.push((c.name.clone(), path));
                    }
                }
                let mut col = v_flex()
                    .child(ui::section_label(t, format!("IDENTITIES · {}", keys.len())));
                if keys.is_empty() {
                    col = col.child(note("No key-based connections saved"));
                } else {
                    for (name, path) in keys {
                        col = col.child(ui::info_row(t, name, path));
                    }
                }
                col.into_any_element()
            }
            Page::Keymap => {
                let mut col = v_flex().child(ui::section_label(t, "SHORTCUTS"));
                for (chord, action) in KEYMAP {
                    col = col.child(ui::info_row(t, *action, *chord));
                }
                col.into_any_element()
            }
            Page::Diagnostics => v_flex()
                .child(ui::section_label(t, "LOGGING"))
                .child(ui::info_row(t, "Destination", "stderr"))
                .child(note(
                    "Runtime logs and panel errors go to standard error. This build keeps no on-disk log file and has no verbosity switch.",
                ))
                .into_any_element(),
            Page::Privacy => v_flex()
                .child(ui::section_label(t, "LOCAL DATA"))
                .child(note(
                    "Pier-X is offline-first — connection profiles and preferences stay on this device and nothing here is sent anywhere.",
                ))
                .child(ui::section_label(t, "SECRET SCANNING"))
                .child(note(
                    "User-defined secret-scan patterns are a planned feature; this build has nothing to configure.",
                ))
                .into_any_element(),
            Page::Security => v_flex()
                .child(ui::section_label(t, "CREDENTIALS"))
                .child(note(
                    "Connection passwords and key passphrases live in the OS keychain. The Settings view only shows connection metadata and never reveals stored secrets.",
                ))
                .child(ui::section_label(t, "PRIVILEGE ELEVATION"))
                .child(note(
                    "Per-host sudo passwords and the elevation inventory are managed in the full app; this build has no editable security controls.",
                ))
                .into_any_element(),
            Page::General => v_flex()
                .child(ui::section_label(t, "LANGUAGE"))
                .child(ui::info_row(t, "Interface", "English"))
                .child(ui::section_label(t, "STARTUP"))
                .child(ui::info_row(t, "Update check", "Off"))
                .child(note(
                    "Pier-X is offline by default. Language, update checks and developer toggles are fixed in this build.",
                ))
                .into_any_element(),
            Page::About => v_flex()
                .child(ui::section_label(t, "ABOUT"))
                .child(ui::info_row(t, "Version", "0.7.2"))
                .child(ui::info_row(t, "UI engine", "GPUI (native)"))
                .child(ui::info_row(t, "Backend", "pier-core"))
                .into_any_element(),
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = self.theme.clone();

        let mut nav = v_flex()
            .w(px(184.0))
            .h_full()
            .flex_none()
            .p(t.sp2)
            .gap(px(2.0))
            .bg(t.panel_2)
            .border_r_1()
            .border_color(t.line);
        for &(page, glyph, label) in PAGES {
            nav = nav.child(self.nav_item(cx, page, glyph, label));
        }

        v_flex()
            .w(px(640.0))
            .h(px(440.0))
            .bg(t.panel)
            .border_1()
            .border_color(t.line_2)
            .rounded(t.radius_lg)
            .overflow_hidden()
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .w_full()
                    .h(px(40.0))
                    .px(t.sp4)
                    .border_b_1()
                    .border_color(t.line)
                    .child(ui::icon("settings", px(16.0), t.accent))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink)
                            .child("Settings"),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(nav)
                    .child(
                        div()
                            .id("set-content")
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .overflow_y_scroll()
                            .py(t.sp3)
                            .child(self.content(cx)),
                    ),
            )
    }
}
