// Pier-X GPUI spike — shell skeleton.
//
// A representative (non-functional) Pier-X chrome so the look can be judged:
// tool rail + connection sidebar + tab bar + a faux coloured terminal panel +
// status bar, all painted from the ported design tokens. The terminal content
// is hard-coded sample output — it previews what the real GridSnapshot paint
// (M2) will look like. See docs/GPUI-MIGRATION-PLAN.md.

use gpui::prelude::*;
use gpui::{div, px, Context, Entity, FontWeight, Hsla, SharedString, Window};
use gpui_component::{h_flex, v_flex};

use crate::terminal::TerminalView;
use crate::theme::Theme;

pub struct Shell {
    theme: Theme,
    terminal: Entity<TerminalView>,
}

impl Shell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
            terminal: cx.new(|cx| TerminalView::new(cx)),
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

    // ── Status bar ────────────────────────────────────────────────
    fn status_item(&self, text: impl Into<SharedString>, color: Hsla) -> impl IntoElement {
        div().text_color(color).child(text.into())
    }

    fn status_bar(&self, cols: u16, rows: u16) -> impl IntoElement {
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
                    .child(self.status_item(format!("{cols}×{rows}"), t.muted))
                    .child(self.status_item("UTF-8", t.muted))
                    .child(self.status_item("LF", t.muted)),
            )
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme.clone();
        let (cols, rows) = self.terminal.read(cx).size();
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
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .w_full()
                                    .child(self.terminal.clone()),
                            )
                            .child(self.status_bar(cols, rows)),
                    ),
            )
    }
}
