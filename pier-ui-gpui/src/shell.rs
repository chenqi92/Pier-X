// Pier-X GPUI spike — shell skeleton.
//
// A representative (non-functional) Pier-X chrome so the look can be judged:
// tool rail + connection sidebar + tab bar + a faux coloured terminal panel +
// status bar, all painted from the ported design tokens. The terminal content
// is hard-coded sample output — it previews what the real GridSnapshot paint
// (M2) will look like. See docs/GPUI-MIGRATION-PLAN.md.

use gpui::prelude::*;
use gpui::{div, px, Context, FontWeight, Hsla, SharedString, Window};
use gpui_component::{h_flex, v_flex};

use crate::theme::Theme;

pub struct Shell {
    theme: Theme,
}

impl Shell {
    pub fn new() -> Self {
        Self {
            theme: Theme::dark(),
        }
    }

    // ── Tool rail (left activity bar) ────────────────────────────
    fn tool_icon(&self, color: Hsla, letter: &'static str, active: bool) -> impl IntoElement {
        let t = &self.theme;
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(28.0))
            .h(px(28.0))
            .rounded(t.radius_md)
            .when(active, |d| d.bg(t.accent_subtle))
            .text_size(t.fs_sm)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(color)
            .child(letter)
    }

    fn tool_rail(&self) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .w(t.toolrail_w)
            .h_full()
            .items_center()
            .py(t.sp2)
            .gap(t.sp1)
            .bg(t.surface)
            .border_r_1()
            .border_color(t.line)
            .child(self.tool_icon(t.svc_docker, "D", true))
            .child(self.tool_icon(t.svc_mysql, "My", false))
            .child(self.tool_icon(t.svc_postgres, "Pg", false))
            .child(self.tool_icon(t.svc_redis, "R", false))
            .child(self.tool_icon(t.svc_monitor, "M", false))
            .child(self.tool_icon(t.svc_log, "L", false))
            .child(self.tool_icon(t.svc_sftp, "S", false))
    }

    // ── Sidebar (connections) ────────────────────────────────────
    fn section_label(&self, text: &'static str) -> impl IntoElement {
        let t = &self.theme;
        div()
            .px(t.sp2)
            .pt(t.sp3)
            .pb(t.sp1)
            .text_size(t.fs_sm)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.muted)
            .child(text)
    }

    fn conn_row(
        &self,
        dot: Hsla,
        name: impl Into<SharedString>,
        sub: impl Into<SharedString>,
        active: bool,
    ) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .h(px(28.0))
            .px(t.sp2)
            .mx(t.sp1)
            .rounded(t.radius_sm)
            .when(active, |d| d.bg(t.accent_dim))
            .child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(dot))
            .child(
                div()
                    .flex_1()
                    .text_color(if active { t.ink } else { t.ink_2 })
                    .child(name.into()),
            )
            .child(
                div()
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(sub.into()),
            )
    }

    fn sidebar(&self) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .w(t.sidebar_w)
            .h_full()
            .bg(t.surface)
            .border_r_1()
            .border_color(t.line)
            // header
            .child(
                h_flex()
                    .items_center()
                    .h(t.panel_header_h)
                    .px(t.sp3)
                    .border_b_1()
                    .border_color(t.line)
                    .text_size(t.fs_sm)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.ink_2)
                    .child("CONNECTIONS"),
            )
            .child(self.section_label("SERVERS"))
            .child(self.conn_row(t.pos, "prod-web-01", "ssh", true))
            .child(self.conn_row(t.pos, "db-primary", "ssh", false))
            .child(self.conn_row(t.muted, "staging-02", "off", false))
            .child(self.section_label("DATABASES"))
            .child(self.conn_row(t.svc_mysql, "shop_main", "mysql", false))
            .child(self.conn_row(t.svc_postgres, "analytics", "pg", false))
            .child(self.section_label("LOCAL"))
            .child(self.conn_row(t.accent, "zsh — Pier-X", "shell", false))
    }

    // ── Tab bar ───────────────────────────────────────────────────
    fn tab(&self, dot: Hsla, label: &'static str, active: bool) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .h_full()
            .px(t.sp3)
            .border_r_1()
            .border_color(t.line)
            .when(active, |d| d.bg(t.bg))
            .when(!active, |d| d.bg(t.surface).text_color(t.muted))
            .child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(dot))
            .child(
                div()
                    .text_color(if active { t.ink } else { t.muted })
                    .child(label),
            )
    }

    fn tab_bar(&self) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .w_full()
            .h(t.tabbar_h)
            .bg(t.surface)
            .border_b_1()
            .border_color(t.line)
            .child(self.tab(t.pos, "prod-web-01", true))
            .child(self.tab(t.accent, "zsh — local", false))
    }

    // ── Faux terminal (previews the M2 GridSnapshot paint) ───────
    fn seg(&self, text: &'static str, color: Hsla) -> impl IntoElement {
        div().text_color(color).child(text)
    }

    fn term_row(&self) -> gpui::Div {
        // A monospace flex row; callers push coloured segments as children.
        h_flex().font_family(self.theme.mono.clone())
    }

    fn terminal(&self) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .bg(t.bg)
            .p(t.sp3)
            .gap(px(2.0))
            .text_size(t.fs_body)
            // prompt + ls
            .child(
                self.term_row()
                    .child(self.seg("chenqi@prod-web-01", t.pos))
                    .child(self.seg(" ~/srv/pier-x", t.info))
                    .child(self.seg(" (main)", t.warn)),
            )
            .child(
                self.term_row()
                    .child(self.seg("$ ", t.muted))
                    .child(self.seg("ls", t.ink)),
            )
            .child(
                self.term_row()
                    .gap(t.sp3)
                    .child(self.seg("Cargo.toml", t.ink_2))
                    .child(self.seg("pier-core", t.accent))
                    .child(self.seg("src-tauri", t.accent))
                    .child(self.seg("docs", t.accent))
                    .child(self.seg("README.md", t.ink_2)),
            )
            // git status
            .child(
                self.term_row()
                    .child(self.seg("$ ", t.muted))
                    .child(self.seg("git status -sb", t.ink)),
            )
            .child(
                self.term_row()
                    .child(self.seg("## ", t.muted))
                    .child(self.seg("main", t.pos))
                    .child(self.seg("...origin/main", t.muted)),
            )
            .child(
                self.term_row()
                    .child(self.seg(" M ", t.warn))
                    .child(self.seg("pier-ui-gpui/src/shell.rs", t.ink_2)),
            )
            .child(
                self.term_row()
                    .child(self.seg("?? ", t.neg))
                    .child(self.seg("pier-ui-gpui/.vendor/", t.ink_2)),
            )
            // build
            .child(
                self.term_row()
                    .child(self.seg("$ ", t.muted))
                    .child(self.seg("cargo build", t.ink)),
            )
            .child(
                self.term_row()
                    .child(self.seg("   Compiling ", t.pos))
                    .child(self.seg("pier-ui-gpui v0.1.0", t.ink_2)),
            )
            .child(
                self.term_row()
                    .child(self.seg("    Finished ", t.pos))
                    .child(self.seg("`dev` in 4.02s", t.ink_2)),
            )
            // live prompt + cursor block
            .child(
                self.term_row()
                    .items_center()
                    .child(self.seg("$ ", t.muted))
                    .child(div().w(px(8.0)).h(px(16.0)).bg(t.ink)),
            )
    }

    // ── Status bar ────────────────────────────────────────────────
    fn status_item(&self, text: impl Into<SharedString>, color: Hsla) -> impl IntoElement {
        div().text_color(color).child(text.into())
    }

    fn status_bar(&self) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .justify_between()
            .w_full()
            .h(t.statusbar_h)
            .px(t.sp3)
            .bg(t.surface)
            .border_t_1()
            .border_color(t.line)
            .text_size(t.fs_sm)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp3)
                    .child(self.status_item("⎇ main", t.ink_2))
                    .child(self.status_item("✓ clean", t.pos)),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp3)
                    .child(self.status_item("ssh prod-web-01", t.muted))
                    .child(self.status_item("120×26", t.muted))
                    .child(self.status_item("UTF-8", t.muted))
                    .child(self.status_item("LF", t.muted)),
            )
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme.clone();
        div()
            .size_full()
            .font_family(t.sans.clone())
            .text_size(t.fs_body)
            .text_color(t.ink)
            .bg(t.bg)
            .child(
                h_flex()
                    .size_full()
                    .child(self.tool_rail())
                    .child(self.sidebar())
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .min_w(px(0.0))
                            .child(self.tab_bar())
                            .child(self.terminal())
                            .child(self.status_bar()),
                    ),
            )
    }
}
