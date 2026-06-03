// Pier-X GPUI spike — shell chrome, modelled on the React/Tauri shell.
//
// Layout mirrors the web version (see docs/PRODUCT-SPEC + pier-x-copy/screens/b1.png):
//   ┌───────────────────────── TopBar ─────────────────────────┐
//   │ Sidebar │           TabBar (center + right)              │
//   │ (left)  ├───────────────┬───────────────┬───────────────┤
//   │         │    Center     │  RightPanel   │  ToolStrip(R)  │
//   ├─────────┴───────────────┴───────────────┴───────────────┤
//   │                       StatusBar                          │
//   └──────────────────────────────────────────────────────────┘
// Interactions wired: switch/close tabs, switch right tool, Files/Servers
// sidebar toggle, connection-row selection, collapse right panel — all native
// GPUI state on the Shell entity. The center is the real TerminalView.

use gpui::prelude::*;
use gpui::{
    div, px, svg, Context, Entity, FontWeight, Hsla, MouseButton, MouseDownEvent, Pixels,
    SharedString, Svg, Window,
};
use gpui_component::{h_flex, v_flex, TitleBar};

use crate::terminal::TerminalView;
use crate::theme::Theme;

/// A bundled lucide SVG, sized and tinted. `name` is the file stem under
/// `assets/icons/` (see src/assets.rs); the glyph picks up `color` because the
/// SVGs paint with `currentColor`.
fn icon(name: &str, sz: Pixels, color: Hsla) -> Svg {
    svg()
        .flex_none()
        .w(sz)
        .h(sz)
        .path(SharedString::from(format!("icons/{name}.svg")))
        .text_color(color)
}

#[derive(Clone, Copy, PartialEq)]
enum Svc {
    Markdown,
    Git,
    Monitor,
    Firewall,
    Sftp,
    Log,
    Search,
    Docker,
    Mysql,
    Postgres,
    Redis,
    Sqlite,
    Webserver,
    Software,
}

/// (service, icon stem, full name, category index).
const TOOLS: &[(Svc, &str, &str, u8)] = &[
    (Svc::Markdown, "file-text", "MARKDOWN", 0),
    (Svc::Git, "git-branch", "GIT", 0),
    (Svc::Monitor, "activity", "MONITOR", 1),
    (Svc::Firewall, "shield", "FIREWALL", 1),
    (Svc::Sftp, "folder", "SFTP", 2),
    (Svc::Log, "scroll-text", "LOGS", 2),
    (Svc::Search, "search", "SEARCH", 2),
    (Svc::Docker, "container", "DOCKER", 3),
    (Svc::Mysql, "database", "MYSQL", 4),
    (Svc::Postgres, "database", "POSTGRES", 4),
    (Svc::Redis, "database", "REDIS", 4),
    (Svc::Sqlite, "database", "SQLITE", 4),
    (Svc::Webserver, "server", "WEBSERVER", 5),
    (Svc::Software, "package", "SOFTWARE", 5),
];

#[derive(Clone, Copy, PartialEq)]
enum TabKind {
    Local,
    Ssh,
    Db,
    Markdown,
}

struct Tab {
    title: &'static str,
    kind: TabKind,
}

struct Conn {
    name: &'static str,
    addr: &'static str,
    online: bool,
}

pub struct Shell {
    theme: Theme,
    terminal: Entity<TerminalView>,
    tabs: Vec<Tab>,
    active_tab: usize,
    active_tool: usize,
    show_servers: bool,
    selected_conn: usize,
    right_collapsed: bool,
}

impl Shell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
            terminal: cx.new(|cx| TerminalView::new(cx)),
            tabs: vec![
                Tab { title: "~/code/warehouse-api", kind: TabKind::Local },
                Tab { title: "deploy@prod-web-01", kind: TabKind::Ssh },
                Tab { title: "postgres · analytics", kind: TabKind::Db },
                Tab { title: "CHANGELOG.md", kind: TabKind::Markdown },
            ],
            active_tab: 0,
            // default to Git so the right panel matches the reference screenshot
            active_tool: 1,
            show_servers: false,
            selected_conn: 0,
            right_collapsed: false,
        }
    }

    fn svc_color(&self, s: Svc) -> Hsla {
        let t = &self.theme;
        match s {
            Svc::Markdown => t.svc_log,
            Svc::Git => t.info,
            Svc::Monitor => t.svc_monitor,
            Svc::Firewall => t.warn,
            Svc::Sftp => t.svc_sftp,
            Svc::Log => t.svc_log,
            Svc::Search => t.warn,
            Svc::Docker => t.svc_docker,
            Svc::Mysql => t.svc_mysql,
            Svc::Postgres => t.svc_postgres,
            Svc::Redis => t.svc_redis,
            Svc::Sqlite => t.svc_sftp,
            Svc::Webserver => t.pos,
            Svc::Software => t.svc_log,
        }
    }

    fn tab_icon(kind: TabKind) -> &'static str {
        match kind {
            TabKind::Local => "square-terminal",
            TabKind::Ssh => "terminal",
            TabKind::Db => "database",
            TabKind::Markdown => "file-text",
        }
    }

    // ── TitleBar (client-side window chrome) ─────────────────────
    fn topbar(&self) -> impl IntoElement {
        let t = &self.theme;
        let menu = |label: &'static str| {
            div()
                .px(t.sp2)
                .text_size(t.fs_ui)
                .text_color(t.ink_2)
                .child(label)
        };
        let action = |name: &'static str| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(26.0))
                .h(px(26.0))
                .rounded(t.radius_sm)
                .child(icon(name, px(15.0), t.ink_2))
        };
        // gpui-component TitleBar handles drag + native min/max/close on the
        // right; we fill the draggable area with the menu bar and quick actions.
        TitleBar::new()
            .h(t.titlebar_h)
            .bg(t.surface)
            .border_color(t.line)
            .child(
                h_flex()
                    .items_center()
                    .w_full()
                    .h_full()
                    .gap(t.sp2)
                    .child(
                        div()
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded(t.radius_sm)
                            .bg(t.accent),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink)
                            .child("Pier-X"),
                    )
                    .child(div().text_size(t.fs_sm).text_color(t.muted).child("0.7.2"))
                    .child(div().w(px(8.0)))
                    .child(menu("File"))
                    .child(menu("Edit"))
                    .child(menu("View"))
                    .child(menu("Session"))
                    .child(menu("Help"))
                    .child(div().flex_1())
                    .child(action("command"))
                    .child(action("plus"))
                    .child(action("moon"))
                    .child(action("settings")),
            )
    }

    // ── Sidebar ──────────────────────────────────────────────────
    fn sidebar_tab(
        &self,
        cx: &mut Context<Self>,
        label: &'static str,
        servers: bool,
    ) -> impl IntoElement {
        let t = &self.theme;
        let active = self.show_servers == servers;
        div()
            .id(SharedString::from(format!("sbtab-{label}")))
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .h(t.tabbar_h)
            .text_size(t.fs_ui)
            .text_color(if active { t.ink } else { t.muted })
            .when(active, |d| d.border_b_2().border_color(t.accent))
            .hover(|s| s.text_color(t.ink))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.show_servers = servers;
                    cx.notify();
                }),
            )
            .child(label)
    }

    fn section_label(&self, text: &'static str) -> impl IntoElement {
        let t = &self.theme;
        div()
            .px(t.sp3)
            .pt(t.sp3)
            .pb(t.sp1)
            .text_size(t.fs_sm)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.muted)
            .child(text)
    }

    fn file_row(&self, is_dir: bool, name: &'static str, meta: &'static str) -> impl IntoElement {
        let t = &self.theme;
        let glyph = if is_dir { "folder" } else { "file" };
        let glyph_color = if is_dir { t.accent } else { t.muted };
        h_flex()
            .id(SharedString::from(format!("file-{name}")))
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .text_color(t.ink_2)
            .hover(|s| s.bg(t.hover))
            .child(icon(glyph, px(14.0), glyph_color))
            .child(div().flex_1().child(name))
            .child(div().text_size(t.fs_sm).text_color(t.muted).child(meta))
    }

    fn conn_row(&self, cx: &mut Context<Self>, idx: usize, c: &Conn) -> impl IntoElement {
        let t = &self.theme;
        let selected = self.selected_conn == idx;
        let dot = if c.online { t.pos } else { t.muted };
        h_flex()
            .id(SharedString::from(format!("conn-{idx}")))
            .items_center()
            .gap(t.sp2)
            .h(px(28.0))
            .px(t.sp3)
            .when(selected, |d| d.bg(t.accent_dim))
            .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.selected_conn = idx;
                    cx.notify();
                }),
            )
            .child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(dot))
            .child(
                div()
                    .flex_1()
                    .text_color(if selected { t.ink } else { t.ink_2 })
                    .child(c.name),
            )
            .child(
                div()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(c.addr),
            )
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let body = if self.show_servers {
            let conns = [
                Conn { name: "prod-web-01", addr: "ari@10.0.1.4:22", online: true },
                Conn { name: "db-primary", addr: "ari@10.0.1.9:22", online: true },
                Conn { name: "staging-02", addr: "ari@10.0.2.7:22", online: false },
            ];
            let mut col = v_flex().child(self.section_label("SERVERS"));
            for (i, c) in conns.iter().enumerate() {
                col = col.child(self.conn_row(cx, i, c));
            }
            col
        } else {
            v_flex()
                .child(self.section_label("~/code/warehouse-api"))
                .child(self.file_row(true, ".git", "2d"))
                .child(self.file_row(true, ".github", "5d"))
                .child(self.file_row(true, "migrations", "1h"))
                .child(self.file_row(true, "src", "14m"))
                .child(self.file_row(true, "tests", "3d"))
                .child(self.file_row(true, "docs", "2w"))
                .child(self.file_row(false, ".gitignore", "3w"))
                .child(self.file_row(false, "CHANGELOG.md", "2h"))
                .child(self.file_row(false, "Dockerfile", "5d"))
        };

        v_flex()
            .w(t.sidebar_w)
            .h_full()
            .bg(t.surface)
            .border_r_1()
            .border_color(t.line)
            .child(
                h_flex()
                    .w_full()
                    .border_b_1()
                    .border_color(t.line)
                    .child(self.sidebar_tab(cx, "Files", false))
                    .child(self.sidebar_tab(cx, "Servers", true)),
            )
            .child(body)
    }

    // ── TabBar ────────────────────────────────────────────────────
    fn tab_item(&self, cx: &mut Context<Self>, idx: usize) -> impl IntoElement {
        let t = &self.theme;
        let tab = &self.tabs[idx];
        let active = self.active_tab == idx;
        h_flex()
            .id(SharedString::from(format!("tab-{idx}")))
            .items_center()
            .gap(t.sp2)
            .h_full()
            .px(t.sp3)
            .border_r_1()
            .border_color(t.line)
            .when(active, |d| d.bg(t.bg).border_b_2().border_color(t.accent))
            .when(!active, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.active_tab = idx;
                    cx.notify();
                }),
            )
            .child(icon(
                Self::tab_icon(tab.kind),
                px(14.0),
                if active { t.accent } else { t.muted },
            ))
            .child(
                div()
                    .max_w(px(150.0))
                    .overflow_hidden()
                    .text_color(if active { t.ink } else { t.muted })
                    .child(tab.title),
            )
            .child(
                div()
                    .id(SharedString::from(format!("tabx-{idx}")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(16.0))
                    .h(px(16.0))
                    .rounded(t.radius_sm)
                    .hover(|s| s.bg(t.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            if this.tabs.len() > 1 {
                                this.tabs.remove(idx);
                                if this.active_tab >= this.tabs.len() {
                                    this.active_tab = this.tabs.len() - 1;
                                }
                                cx.notify();
                            }
                        }),
                    )
                    .child(icon("close", px(12.0), t.muted)),
            )
    }

    fn tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let mut row = h_flex()
            .w_full()
            .h(t.tabbar_h)
            .bg(t.surface)
            .border_b_1()
            .border_color(t.line);
        for idx in 0..self.tabs.len() {
            row = row.child(self.tab_item(cx, idx));
        }
        row.child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(34.0))
                .h_full()
                .child(icon("plus", px(15.0), t.muted)),
        )
    }

    // ── Right zone: panel + tool strip ───────────────────────────
    fn tool_btn(&self, cx: &mut Context<Self>, idx: usize) -> impl IntoElement {
        let t = &self.theme;
        let (svc, glyph, _, _) = TOOLS[idx];
        let active = self.active_tool == idx;
        let color = self.svc_color(svc);
        div()
            .id(SharedString::from(format!("tool-{idx}")))
            .flex()
            .items_center()
            .justify_center()
            .w(px(32.0))
            .h(px(32.0))
            .rounded(t.radius_sm)
            .when(active, |d| d.bg(t.accent_dim))
            .when(!active, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.active_tool = idx;
                    this.right_collapsed = false;
                    cx.notify();
                }),
            )
            .child(icon(glyph, px(17.0), if active { color } else { t.muted }))
    }

    fn tool_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let mut col = v_flex()
            .w(t.toolrail_w)
            .h_full()
            .items_center()
            .py(t.sp2)
            .gap(px(2.0))
            .bg(t.surface)
            .border_l_1()
            .border_color(t.line_2);
        let mut prev_cat = TOOLS[0].3;
        for idx in 0..TOOLS.len() {
            let cat = TOOLS[idx].3;
            if cat != prev_cat {
                col = col.child(
                    div()
                        .my(px(2.0))
                        .w(px(20.0))
                        .h(px(1.0))
                        .bg(t.line_2),
                );
                prev_cat = cat;
            }
            col = col.child(self.tool_btn(cx, idx));
        }
        col.child(div().flex_1()).child(
            div()
                .id("collapse")
                .flex()
                .items_center()
                .justify_center()
                .w(px(32.0))
                .h(px(32.0))
                .rounded(t.radius_sm)
                .text_color(t.muted)
                .hover(|s| s.bg(t.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                        this.right_collapsed = !this.right_collapsed;
                        cx.notify();
                    }),
                )
                .child(icon(
                    if self.right_collapsed {
                        "panel-right-open"
                    } else {
                        "panel-right-close"
                    },
                    px(16.0),
                    t.muted,
                )),
        )
    }

    fn panel_header(&self, glyph: &'static str, title: &'static str, meta: &'static str) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .w_full()
            .h(t.panel_header_h)
            .px(t.sp3)
            .border_b_1()
            .border_color(t.line)
            .child(icon(glyph, px(15.0), t.accent))
            .child(
                div()
                    .font_family(t.mono.clone())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.ink)
                    .child(title),
            )
            .child(div().flex_1())
            .child(div().text_size(t.fs_sm).text_color(t.muted).child(meta))
    }

    fn git_change_row(&self, mark: &'static str, mark_color: Hsla, path: &'static str, stat: &'static str, stat_color: Hsla) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .id(SharedString::from(format!("gch-{path}")))
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .child(div().w(px(14.0)).font_family(t.mono.clone()).text_color(mark_color).child(mark))
            .child(div().flex_1().font_family(t.mono.clone()).text_size(t.fs_sm).text_color(t.ink_2).child(path))
            .child(div().font_family(t.mono.clone()).text_size(t.fs_sm).text_color(stat_color).child(stat))
    }

    fn git_panel(&self) -> impl IntoElement {
        let t = &self.theme;
        let chip = |label: &'static str, n: &'static str, active: bool| {
            h_flex()
                .items_center()
                .gap(px(4.0))
                .px(t.sp2)
                .py(px(2.0))
                .text_size(t.fs_ui)
                .text_color(if active { t.ink } else { t.muted })
                .when(active, |d| d.border_b_2().border_color(t.accent))
                .child(label)
                .child(div().text_size(t.fs_sm).text_color(t.muted).child(n))
        };
        let btn = |label: &'static str, primary: bool| {
            div()
                .px(t.sp3)
                .py(px(4.0))
                .rounded(t.radius_sm)
                .text_size(t.fs_ui)
                .when(primary, |d| d.bg(t.accent).text_color(t.accent_ink))
                .when(!primary, |d| d.bg(t.panel_2).text_color(t.ink_2))
                .child(label)
        };
        v_flex()
            .flex_1()
            .min_h(px(0.0))
            .child(self.panel_header("git-branch", "GIT", "feat/ingest-pipeline ↑2"))
            .child(
                h_flex()
                    .gap(t.sp3)
                    .px(t.sp3)
                    .py(t.sp2)
                    .border_b_1()
                    .border_color(t.line)
                    .child(chip("Changes", "8", true))
                    .child(chip("History", "", false))
                    .child(chip("Branches", "3", false))
                    .child(chip("Stash", "1", false)),
            )
            .child(
                v_flex()
                    .m(t.sp3)
                    .p(t.sp3)
                    .gap(t.sp2)
                    .rounded(t.radius_md)
                    .bg(t.panel)
                    .border_1()
                    .border_color(t.line)
                    .child(
                        h_flex()
                            .items_center()
                            .child(div().flex_1().font_family(t.mono.clone()).text_color(t.ink).child("feat/ingest-pipeline"))
                            .child(div().text_size(t.fs_sm).text_color(t.muted).child("↑2 ↓0")),
                    )
                    .child(div().text_size(t.fs_sm).text_color(t.muted).child("tracking origin/feat/ingest-pipeline"))
                    .child(
                        h_flex()
                            .gap(t.sp2)
                            .pt(t.sp1)
                            .child(btn("Push", true))
                            .child(btn("Pull", false))
                            .child(btn("Fetch", false))
                            .child(btn("Rebase", false)),
                    ),
            )
            .child(self.section_label("STAGED · 3"))
            .child(self.git_change_row("M", t.warn, "src/ingest/parse.ts", "+84 -12", t.pos))
            .child(self.git_change_row("A", t.pos, "src/ingest/backpressure.ts", "+142", t.pos))
            .child(self.git_change_row("D", t.neg, "src/ingest/legacy.ts", "-218", t.neg))
            .child(self.section_label("CHANGES · 5"))
            .child(self.git_change_row("M", t.warn, "src/ingest/stream.ts", "+24 -6", t.pos))
            .child(self.git_change_row("M", t.warn, "src/api/routes.ts", "+18 -2", t.pos))
    }

    fn right_panel(&self) -> impl IntoElement {
        let t = &self.theme;
        let (_, glyph, name, _) = TOOLS[self.active_tool];
        if self.active_tool == 1 {
            self.git_panel().into_any_element()
        } else {
            v_flex()
                .flex_1()
                .child(self.panel_header(glyph, name, "panel"))
                .child(
                    div()
                        .p(t.sp4)
                        .text_color(t.muted)
                        .child(SharedString::from(format!("{name} panel — not wired in this spike"))),
                )
                .into_any_element()
        }
    }

    // ── StatusBar ─────────────────────────────────────────────────
    fn status_item(&self, text: impl Into<SharedString>, color: Hsla) -> impl IntoElement {
        div().text_color(color).child(text.into())
    }

    fn status_bar(&self, cols: u16, rows: u16) -> impl IntoElement {
        let t = &self.theme;
        let (_, _, tool_name, _) = TOOLS[self.active_tool];
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
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(icon("git-branch", px(12.0), t.ink_2))
                            .child(self.status_item("feat/ingest-pipeline", t.ink_2)),
                    )
                    .child(self.status_item("↑2 ↓0", t.muted))
                    .child(self.status_item("ssh · russh", t.ink_2))
                    .child(self.status_item(format!("{cols}×{rows}"), t.muted)),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp3)
                    .child(self.status_item(format!("PANEL · {tool_name}"), t.accent))
                    .child(self.status_item("UTF-8", t.muted))
                    .child(self.status_item("● Ready", t.pos))
                    .child(self.status_item("Pier-X v0.7.2", t.muted)),
            )
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme.clone();
        let (cols, rows) = self.terminal.read(cx).size();

        // Right zone: optional panel + always-visible tool strip on the right.
        let mut right_zone = h_flex().h_full();
        if !self.right_collapsed {
            right_zone = right_zone.child(
                v_flex()
                    .w(t.rightpanel_w)
                    .h_full()
                    .bg(t.surface)
                    .border_l_1()
                    .border_color(t.line)
                    .child(self.right_panel()),
            );
        }
        right_zone = right_zone.child(self.tool_strip(cx));

        v_flex()
            .size_full()
            .font_family(t.sans.clone())
            .text_size(t.fs_body)
            .text_color(t.ink)
            .bg(t.bg)
            .child(self.topbar())
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(self.sidebar(cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .min_w(px(0.0))
                            .child(self.tab_bar(cx))
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .w_full()
                                    .child(self.terminal.clone()),
                            ),
                    )
                    .child(right_zone),
            )
            .child(self.status_bar(cols, rows))
    }
}
