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

use crate::data;
use crate::theme::Theme;
use crate::ui;

/// One settings page.
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Appearance,
    Typography,
    Terminal,
    Connections,
    Git,
    Keymap,
}

/// Left-nav entries: (page, icon stem, label).
const PAGES: &[(Page, &str, &str)] = &[
    (Page::Appearance, "sun", "Appearance"),
    (Page::Typography, "a-large-small", "Typography"),
    (Page::Terminal, "square-terminal", "Terminal"),
    (Page::Connections, "server", "Connections"),
    (Page::Git, "git-branch", "Git"),
    (Page::Keymap, "command", "Keymap"),
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
            Page::Git => v_flex()
                .child(ui::section_label(t, "IDENTITY"))
                .child(ui::info_row(t, "User name", dash(&self.git_name)))
                .child(ui::info_row(t, "Email", dash(&self.git_email)))
                .child(ui::section_label(t, "REPOSITORY"))
                .child(ui::info_row(t, "Path", self.repo.clone()))
                .into_any_element(),
            Page::Keymap => {
                let mut col = v_flex().child(ui::section_label(t, "SHORTCUTS"));
                for (chord, action) in KEYMAP {
                    col = col.child(ui::info_row(t, *action, *chord));
                }
                col.into_any_element()
            }
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
