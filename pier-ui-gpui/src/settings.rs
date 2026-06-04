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
use crate::i18n::{self, Lang};
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

/// Left-nav entries: (page, icon stem, label-key). The label is an `i18n`
/// key resolved at render time (and reused as a stable element id).
const PAGES: &[(Page, &str, &str)] = &[
    (Page::Appearance, "sun", "set.appearance"),
    (Page::Typography, "a-large-small", "set.typography"),
    (Page::Terminal, "square-terminal", "set.terminal"),
    (Page::Editor, "file-text", "set.editor"),
    (Page::Connections, "server", "set.connections"),
    (Page::Profiles, "user", "set.profiles"),
    (Page::Git, "git-branch", "set.git"),
    (Page::SshKeys, "asterisk", "set.ssh_keys"),
    (Page::Keymap, "command", "set.keymap"),
    (Page::Diagnostics, "activity", "set.diagnostics"),
    (Page::Privacy, "eye-off", "set.privacy"),
    (Page::Security, "shield", "set.security"),
    (Page::General, "settings-2", "set.general"),
    (Page::About, "info", "set.about"),
];

/// The keyboard shortcuts bound in main.rs: (chord, action-key). The action is
/// an `i18n` key resolved at render time.
const KEYMAP: &[(&str, &str)] = &[
    ("Ctrl+Shift+P", "menu.command_palette"),
    ("Ctrl+Shift+T", "tab.new_terminal"),
    ("Ctrl+Shift+W", "menu.close_tab"),
    ("Ctrl+Shift+L", "menu.toggle_theme"),
    ("Ctrl+,", "set.title"),
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
                    .child(i18n::t(label)),
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
            .child(i18n::t(label))
    }

    /// One language option in the General page's switcher. Flips the global
    /// interface language and persists it, mirroring [`Self::theme_btn`].
    fn lang_btn(&self, cx: &mut Context<Self>, lang: Lang) -> impl IntoElement {
        let t = &self.theme;
        let active = i18n::current() == lang;
        div()
            .id(SharedString::from(format!("set-lang-{}", lang.code())))
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
                    if i18n::current() != lang {
                        i18n::set(lang);
                        // Persist alongside the other UiState fields, the same
                        // load-modify-save the theme switch uses.
                        let mut s = data::load_ui_state();
                        s.lang = lang.code().to_string();
                        data::save_ui_state(&s);
                        window.refresh();
                        cx.notify();
                    }
                }),
            )
            .child(lang.label())
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
        let note = |s: SharedString| {
            div()
                .px(t.sp3)
                .py(t.sp2)
                .text_size(t.fs_sm)
                .text_color(t.dim)
                .child(s)
        };
        match self.page {
            Page::Appearance => v_flex()
                .child(ui::section_label(t, i18n::t("set.theme")))
                .child(
                    h_flex()
                        .gap(t.sp2)
                        .px(t.sp3)
                        .py(t.sp1)
                        .child(self.theme_btn(cx, "set.dark", true))
                        .child(self.theme_btn(cx, "set.light", false)),
                )
                .child(ui::section_label(t, i18n::t("set.accent")))
                .child(ui::info_row(t, i18n::t("set.color"), i18n::t("set.pier_blue")))
                .into_any_element(),
            Page::Typography => v_flex()
                .child(ui::section_label(t, i18n::t("set.fonts")))
                .child(ui::info_row(t, i18n::t("set.sans"), self.theme.sans.clone()))
                .child(ui::info_row(t, i18n::t("set.mono"), self.theme.mono.clone()))
                .child(ui::section_label(t, i18n::t("set.size")))
                .child(ui::info_row(t, i18n::t("set.heading"), format!("{:.0}px", f32::from(t.fs_h3))))
                .child(ui::info_row(t, i18n::t("set.body"), format!("{:.0}px", f32::from(t.fs_body))))
                .child(ui::info_row(t, i18n::t("set.ui"), format!("{:.0}px", f32::from(t.fs_ui))))
                .child(ui::info_row(t, i18n::t("set.small"), format!("{:.0}px", f32::from(t.fs_sm))))
                .into_any_element(),
            Page::Terminal => v_flex()
                .child(ui::section_label(t, i18n::t("set.local_shell")))
                .child(ui::info_row(t, i18n::t("set.program"), "powershell.exe"))
                .child(ui::info_row(t, i18n::t("set.cursor"), i18n::t("set.cursor_block")))
                .child(ui::info_row(t, i18n::t("set.scrollback"), i18n::t("set.scrollback_value")))
                .child(note(i18n::t("set.cursor_note")))
                .into_any_element(),
            Page::Editor => v_flex()
                .child(ui::section_label(t, i18n::t("set.editor_section")))
                .child(note(i18n::t("set.editor_note")))
                .into_any_element(),
            Page::Connections => {
                let mut col = v_flex().child(ui::section_label(
                    t,
                    i18n::tf("set.saved", &[&self.conns.len().to_string()]),
                ));
                if self.conns.is_empty() {
                    col = col.child(note(i18n::t("set.no_connections")));
                } else {
                    for c in &self.conns {
                        col = col.child(self.conn_line(&c.name, &c.addr));
                    }
                }
                col.into_any_element()
            }
            Page::Profiles => v_flex()
                .child(ui::section_label(t, i18n::t("set.terminal_profiles")))
                .child(ui::info_row(t, i18n::t("set.default_shell"), "powershell.exe"))
                .child(note(i18n::t("set.profiles_note")))
                .into_any_element(),
            Page::Git => v_flex()
                .child(ui::section_label(t, i18n::t("set.identity")))
                .child(ui::info_row(t, i18n::t("set.user_name"), dash(&self.git_name)))
                .child(ui::info_row(t, i18n::t("set.email"), dash(&self.git_email)))
                .child(ui::section_label(t, i18n::t("set.repository")))
                .child(ui::info_row(t, i18n::t("set.path"), self.repo.clone()))
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
                let mut col = v_flex().child(ui::section_label(
                    t,
                    i18n::tf("set.identities", &[&keys.len().to_string()]),
                ));
                if keys.is_empty() {
                    col = col.child(note(i18n::t("set.no_key_conns")));
                } else {
                    for (name, path) in keys {
                        col = col.child(ui::info_row(t, name, path));
                    }
                }
                col.into_any_element()
            }
            Page::Keymap => {
                let mut col = v_flex().child(ui::section_label(t, i18n::t("set.shortcuts")));
                for (chord, action) in KEYMAP {
                    col = col.child(ui::info_row(t, i18n::t(action), *chord));
                }
                col.into_any_element()
            }
            Page::Diagnostics => v_flex()
                .child(ui::section_label(t, i18n::t("set.logging")))
                .child(ui::info_row(t, i18n::t("set.destination"), "stderr"))
                .child(note(i18n::t("set.diag_note")))
                .into_any_element(),
            Page::Privacy => v_flex()
                .child(ui::section_label(t, i18n::t("set.local_data")))
                .child(note(i18n::t("set.privacy_note1")))
                .child(ui::section_label(t, i18n::t("set.secret_scanning")))
                .child(note(i18n::t("set.privacy_note2")))
                .into_any_element(),
            Page::Security => v_flex()
                .child(ui::section_label(t, i18n::t("set.credentials")))
                .child(note(i18n::t("set.security_note1")))
                .child(ui::section_label(t, i18n::t("set.privilege")))
                .child(note(i18n::t("set.security_note2")))
                .into_any_element(),
            Page::General => {
                let mut langs = h_flex().gap(t.sp2).px(t.sp3).py(t.sp1);
                for l in Lang::all() {
                    langs = langs.child(self.lang_btn(cx, l));
                }
                v_flex()
                    .child(ui::section_label(t, i18n::t("set.language")))
                    .child(langs)
                    .child(ui::section_label(t, i18n::t("set.startup")))
                    .child(ui::info_row(t, i18n::t("set.update_check"), i18n::t("set.off")))
                    .child(note(i18n::t("set.general_note")))
                    .into_any_element()
            }
            Page::About => v_flex()
                .child(ui::section_label(t, i18n::t("set.about_section")))
                .child(ui::info_row(t, i18n::t("set.version"), "0.7.2"))
                .child(ui::info_row(t, i18n::t("set.ui_engine"), "GPUI (native)"))
                .child(ui::info_row(t, i18n::t("set.backend"), "pier-core"))
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
                            .child(i18n::t("set.title")),
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
