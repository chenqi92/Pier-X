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

use std::path::PathBuf;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, relative, svg, AnyElement, Context, Entity, Focusable, FontWeight, Hsla, MouseButton,
    MouseDownEvent, Pixels, SharedString, Svg, Window,
};
use gpui_component::{h_flex, v_flex, TitleBar};

use pier_core::services::git::FileStatus;

use crate::data::{self, ConnRow, FileEntry, GitData, MonStat};
use crate::panels::PanelViews;
use crate::terminal::TerminalView;
use crate::theme::Theme;
use crate::ui;

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
pub enum Svc {
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

/// Sub-views inside the Git panel.
#[derive(Clone, Copy, PartialEq)]
enum GitTab {
    Changes,
    History,
    Branches,
    Stash,
}

struct Tab {
    title: String,
    kind: TabKind,
    /// Each tab owns its own terminal session; dropping the tab drops the
    /// entity, which drops PierTerminal and closes the PTY.
    terminal: Entity<TerminalView>,
}

pub struct Shell {
    theme: Theme,
    tabs: Vec<Tab>,
    active_tab: usize,
    active_tool: usize,
    show_servers: bool,
    selected_conn: usize,
    right_collapsed: bool,
    // Real data loaded from pier-core / the local working dir.
    cwd: PathBuf,
    cwd_label: String,
    files: Vec<FileEntry>,
    conns: Vec<ConnRow>,
    git: Option<GitData>,
    git_tab: GitTab,
    git_history: Vec<data::CommitInfo>,
    git_branch_list: Vec<String>,
    git_stashes: Vec<data::StashEntry>,
    /// Transient Push/Pull result line.
    git_msg: Option<String>,
    mon: Option<MonStat>,
    panels: PanelViews,
}

impl Shell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let cwd = data::current_dir();
        let cwd_label = cwd.display().to_string();
        let files = data::list_dir(&cwd);
        let conns = data::load_connections();
        let git = data::git_status(&cwd);
        let tab_title = cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| cwd_label.clone());
        let terminal = cx.new(|cx| TerminalView::new(cx));
        let panels = PanelViews::new(cx);
        Self::start_monitor(cx);
        Self {
            theme: Theme::dark(),
            tabs: vec![Tab { title: tab_title, kind: TabKind::Local, terminal }],
            active_tab: 0,
            // default to Git so the right panel matches the reference screenshot
            active_tool: 1,
            show_servers: false,
            selected_conn: 0,
            right_collapsed: false,
            cwd,
            cwd_label,
            files,
            conns,
            git,
            git_tab: GitTab::Changes,
            git_history: Vec::new(),
            git_branch_list: Vec::new(),
            git_stashes: Vec::new(),
            git_msg: None,
            mon: None,
            panels,
        }
    }

    /// Switch the Git sub-tab, loading its data on demand (local git reads are
    /// fast; this runs in a click handler, never in render).
    fn set_git_tab(&mut self, tab: GitTab, cx: &mut Context<Self>) {
        self.git_tab = tab;
        match tab {
            GitTab::History => self.git_history = data::git_log(&self.cwd, 50),
            GitTab::Branches => self.git_branch_list = data::git_branches(&self.cwd),
            GitTab::Stash => self.git_stashes = data::git_stash(&self.cwd),
            GitTab::Changes => {}
        }
        cx.notify();
    }

    /// Run `git push`/`git pull` off the render path and surface the result.
    fn git_action(&mut self, push: bool, cx: &mut Context<Self>) {
        self.git_msg = Some(if push { "Pushing…".into() } else { "Pulling…".into() });
        cx.notify();
        let cwd = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    if push {
                        data::git_push(&cwd)
                    } else {
                        data::git_pull(&cwd)
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_msg = Some(match res {
                    Ok(s) => {
                        let s = s.trim().to_string();
                        if s.is_empty() {
                            "Done".to_string()
                        } else {
                            s
                        }
                    }
                    Err(e) => e,
                });
                this.git = data::git_status(&this.cwd);
                cx.notify();
            });
        })
        .detach();
    }

    /// Refresh the Monitor snapshot on an interval while the Monitor panel is
    /// the visible tool. Sampling is gated so we don't poll sysinfo when the
    /// panel is hidden.
    fn start_monitor(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let alive = this
                .update(cx, |this, cx| {
                    let showing = matches!(TOOLS[this.active_tool].0, Svc::Monitor)
                        && !this.right_collapsed;
                    if showing {
                        this.mon = Some(data::monitor_snapshot());
                        cx.notify();
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        })
        .detach();
    }

    /// Open a fresh local terminal tab and make it active.
    fn open_terminal_tab(&mut self, cx: &mut Context<Self>) {
        let terminal = cx.new(|cx| TerminalView::new(cx));
        self.tabs.push(Tab {
            title: "pwsh".to_string(),
            kind: TabKind::Local,
            terminal,
        });
        self.active_tab = self.tabs.len() - 1;
        cx.notify();
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
    fn topbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .child(
                        // Theme toggle: moon in dark mode (→ light), sun in light (→ dark).
                        div()
                            .id("theme-toggle")
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(26.0))
                            .h(px(26.0))
                            .rounded(t.radius_sm)
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _: &MouseDownEvent, window, cx| {
                                    let dark = cx.global::<Theme>().dark;
                                    cx.set_global(if dark {
                                        Theme::light()
                                    } else {
                                        Theme::dark()
                                    });
                                    window.refresh();
                                }),
                            )
                            .child(icon(
                                if t.dark { "moon" } else { "sun" },
                                px(15.0),
                                t.ink_2,
                            )),
                    )
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

    fn section_label(&self, text: impl Into<SharedString>) -> impl IntoElement {
        let t = &self.theme;
        div()
            .px(t.sp3)
            .pt(t.sp3)
            .pb(t.sp1)
            .text_size(t.fs_sm)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.muted)
            .child(text.into())
    }

    /// Point the Files tree at a new directory and reload its contents + git.
    fn navigate_to(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.cwd_label = path.display().to_string();
        self.files = data::list_dir(&path);
        self.git = data::git_status(&path);
        self.cwd = path;
        cx.notify();
    }

    fn file_row(&self, cx: &mut Context<Self>, f: &FileEntry) -> impl IntoElement {
        let t = &self.theme;
        let glyph = if f.is_dir { "folder" } else { "file" };
        let glyph_color = if f.is_dir { t.accent } else { t.muted };
        let name = f.name.clone();
        let is_dir = f.is_dir;
        h_flex()
            .id(SharedString::from(format!("file-{}", f.name)))
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .text_color(t.ink_2)
            .hover(|s| s.bg(t.hover))
            .when(is_dir, |d| {
                d.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                        let target = this.cwd.join(&name);
                        this.navigate_to(target, cx);
                    }),
                )
            })
            .child(icon(glyph, px(14.0), glyph_color))
            .child(div().flex_1().overflow_hidden().child(f.name.clone()))
            .child(
                div()
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(f.age.clone()),
            )
    }

    /// The ".." row that ascends to the parent directory.
    fn parent_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .id("file-up")
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .text_color(t.muted)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                    if let Some(parent) = this.cwd.parent().map(|p| p.to_path_buf()) {
                        this.navigate_to(parent, cx);
                    }
                }),
            )
            .child(icon("folder", px(14.0), t.muted))
            .child(div().flex_1().child(".."))
    }

    /// Open a new SSH terminal tab for saved connection `idx`.
    fn open_ssh_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(cfg) = data::connections_raw().into_iter().nth(idx) else {
            return;
        };
        let title = format!("{}@{}", cfg.user, cfg.host);
        let terminal = cx.new(|cx| TerminalView::new_ssh(cx, cfg));
        self.tabs.push(Tab {
            title,
            kind: TabKind::Ssh,
            terminal,
        });
        self.active_tab = self.tabs.len() - 1;
        cx.notify();
    }

    fn conn_row(&self, cx: &mut Context<Self>, idx: usize, c: &ConnRow) -> impl IntoElement {
        let t = &self.theme;
        let selected = self.selected_conn == idx;
        let dot = if c.online { t.pos } else { t.muted };
        h_flex()
            .id(SharedString::from(format!("conn-{idx}")))
            .items_center()
            .gap(t.sp2)
            .h(px(42.0))
            .px(t.sp3)
            .cursor_pointer()
            .when(selected, |d| d.bg(t.accent_dim))
            .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.selected_conn = idx;
                    this.open_ssh_tab(idx, cx);
                }),
            )
            .child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(dot))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .overflow_hidden()
                            .text_color(if selected { t.ink } else { t.ink_2 })
                            .child(c.name.clone()),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(c.addr.clone()),
                    ),
            )
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let body = if self.show_servers {
            let mut col =
                v_flex().child(self.section_label(format!("SERVERS · {}", self.conns.len())));
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
                for (i, c) in self.conns.iter().enumerate() {
                    col = col.child(self.conn_row(cx, i, c));
                }
            }
            col
        } else {
            let mut col = v_flex().child(self.section_label(self.cwd_label.clone()));
            if self.cwd.parent().is_some() {
                col = col.child(self.parent_row(cx));
            }
            for f in &self.files {
                col = col.child(self.file_row(cx, f));
            }
            col
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
            .child(
                div()
                    .id("sidebar-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(body),
            )
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
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    this.active_tab = idx;
                    let handle = this.tabs[idx].terminal.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
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
                    .child(tab.title.clone()),
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
                .id("new-tab")
                .flex()
                .items_center()
                .justify_center()
                .w(px(34.0))
                .h_full()
                .hover(|s| s.bg(t.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                        this.open_terminal_tab(cx);
                    }),
                )
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
                    if matches!(TOOLS[idx].0, Svc::Monitor) {
                        this.mon = Some(data::monitor_snapshot());
                    }
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

    fn panel_header(
        &self,
        glyph: &'static str,
        title: impl Into<SharedString>,
        meta: impl Into<SharedString>,
    ) -> impl IntoElement {
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
                    .child(title.into()),
            )
            .child(div().flex_1())
            .child(div().text_size(t.fs_sm).text_color(t.muted).child(meta.into()))
    }

    fn git_change_row(&self, c: &data::GitChange) -> impl IntoElement {
        let t = &self.theme;
        let (mark, mark_color) = status_style(t, &c.status);
        h_flex()
            .id(SharedString::from(format!("gch-{}", c.path)))
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .child(
                div()
                    .w(px(14.0))
                    .font_family(t.mono.clone())
                    .text_color(mark_color)
                    .child(mark),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink_2)
                    .child(c.path.clone()),
            )
    }

    fn git_chip(
        &self,
        cx: &mut Context<Self>,
        label: &'static str,
        count: Option<usize>,
        tab: GitTab,
    ) -> impl IntoElement {
        let t = &self.theme;
        let active = self.git_tab == tab;
        h_flex()
            .id(SharedString::from(format!("gtab-{label}")))
            .items_center()
            .gap(px(4.0))
            .px(t.sp2)
            .py(px(2.0))
            .text_size(t.fs_ui)
            .cursor_pointer()
            .text_color(if active { t.ink } else { t.muted })
            .when(active, |d| d.border_b_2().border_color(t.accent))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| this.set_git_tab(tab, cx)),
            )
            .child(label)
            .when_some(count, |d, n| {
                d.child(div().text_size(t.fs_sm).text_color(t.muted).child(n.to_string()))
            })
    }

    /// `push = Some(true)` → Push, `Some(false)` → Pull, `None` → inert label.
    fn git_btn(
        &self,
        cx: &mut Context<Self>,
        label: &'static str,
        primary: bool,
        push: Option<bool>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let mut d = div()
            .id(SharedString::from(format!("gbtn-{label}")))
            .px(t.sp3)
            .py(px(4.0))
            .rounded(t.radius_sm)
            .text_size(t.fs_ui)
            .when(primary, |d| d.bg(t.accent).text_color(t.accent_ink))
            .when(!primary, |d| d.bg(t.panel_2).text_color(t.ink_2))
            .child(label);
        match push {
            Some(is_push) => {
                d = d.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                        this.git_action(is_push, cx)
                    }),
                );
            }
            None => d = d.text_color(t.dim),
        }
        d
    }

    fn git_commit_row(&self, c: &data::CommitInfo) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .gap(px(2.0))
            .px(t.sp3)
            .py(t.sp2)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .child(
                        div()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.accent)
                            .child(c.short_hash.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_color(t.ink_2)
                            .child(c.message.clone()),
                    ),
            )
            .child(
                h_flex()
                    .gap(t.sp2)
                    .child(div().text_size(t.fs_sm).text_color(t.muted).child(c.author.clone()))
                    .child(
                        div()
                            .text_size(t.fs_sm)
                            .text_color(t.dim)
                            .child(c.relative_date.clone()),
                    ),
            )
    }

    fn git_branch_row(&self, name: &str, current: bool) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .id(SharedString::from(format!("gbr-{name}")))
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .child(icon("git-branch", px(13.0), if current { t.accent } else { t.muted }))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .font_family(t.mono.clone())
                    .text_color(if current { t.ink } else { t.ink_2 })
                    .child(name.to_string()),
            )
            .when(current, |d| {
                d.child(div().text_size(t.fs_sm).text_color(t.accent).child("current"))
            })
    }

    fn git_stash_row(&self, s: &data::StashEntry) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .gap(px(2.0))
            .px(t.sp3)
            .py(t.sp2)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .child(
                        div()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.accent)
                            .child(s.index.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_color(t.ink_2)
                            .child(s.message.clone()),
                    ),
            )
            .child(
                div()
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child(s.relative_date.clone()),
            )
    }

    fn git_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let Some(git) = &self.git else {
            return v_flex()
                .flex_1()
                .child(self.panel_header("git-branch", "GIT", ""))
                .child(
                    div()
                        .p(t.sp4)
                        .text_color(t.muted)
                        .child("Not a git repository"),
                )
                .into_any_element();
        };
        let total = git.staged.len() + git.unstaged.len();
        let ahead_behind = format!("↑{} ↓{}", git.ahead, git.behind);
        let tracking = if git.tracking.is_empty() {
            "no upstream".to_string()
        } else {
            format!("tracking {}", git.tracking)
        };

        let mut col = v_flex()
            .flex_1()
            .min_h(px(0.0))
            .child(self.panel_header("git-branch", "GIT", git.branch.clone()))
            .child(
                h_flex()
                    .gap(t.sp3)
                    .px(t.sp3)
                    .py(t.sp2)
                    .border_b_1()
                    .border_color(t.line)
                    .child(self.git_chip(cx, "Changes", Some(total), GitTab::Changes))
                    .child(self.git_chip(cx, "History", None, GitTab::History))
                    .child(self.git_chip(cx, "Branches", None, GitTab::Branches))
                    .child(self.git_chip(cx, "Stash", None, GitTab::Stash)),
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
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .font_family(t.mono.clone())
                                    .text_color(t.ink)
                                    .child(git.branch.clone()),
                            )
                            .child(
                                div()
                                    .text_size(t.fs_sm)
                                    .text_color(t.muted)
                                    .child(ahead_behind),
                            ),
                    )
                    .child(div().text_size(t.fs_sm).text_color(t.muted).child(tracking))
                    .child(
                        h_flex()
                            .gap(t.sp2)
                            .pt(t.sp1)
                            .child(self.git_btn(cx, "Push", true, Some(true)))
                            .child(self.git_btn(cx, "Pull", false, Some(false)))
                            .child(self.git_btn(cx, "Fetch", false, None))
                            .child(self.git_btn(cx, "Rebase", false, None)),
                    )
                    .when_some(self.git_msg.clone(), |d, msg| {
                        d.child(
                            div()
                                .text_size(t.fs_sm)
                                .font_family(t.mono.clone())
                                .text_color(t.ink_2)
                                .child(msg),
                        )
                    }),
            );

        match self.git_tab {
            GitTab::Changes => {
                if !git.staged.is_empty() {
                    col = col.child(self.section_label(format!("STAGED · {}", git.staged.len())));
                    for c in &git.staged {
                        col = col.child(self.git_change_row(c));
                    }
                }
                if !git.unstaged.is_empty() {
                    col = col.child(self.section_label(format!("CHANGES · {}", git.unstaged.len())));
                    for c in &git.unstaged {
                        col = col.child(self.git_change_row(c));
                    }
                }
                if total == 0 {
                    col = col.child(
                        div().p(t.sp4).text_color(t.muted).child("Working tree clean"),
                    );
                }
            }
            GitTab::History => {
                col = col.child(self.section_label(format!("HISTORY · {}", self.git_history.len())));
                if self.git_history.is_empty() {
                    col = col.child(div().p(t.sp4).text_color(t.muted).child("No commits"));
                } else {
                    for c in &self.git_history {
                        col = col.child(self.git_commit_row(c));
                    }
                }
            }
            GitTab::Branches => {
                col = col
                    .child(self.section_label(format!("BRANCHES · {}", self.git_branch_list.len())));
                if self.git_branch_list.is_empty() {
                    col = col.child(div().p(t.sp4).text_color(t.muted).child("No branches"));
                } else {
                    for b in &self.git_branch_list {
                        col = col.child(self.git_branch_row(b, b == &git.branch));
                    }
                }
            }
            GitTab::Stash => {
                col = col.child(self.section_label(format!("STASH · {}", self.git_stashes.len())));
                if self.git_stashes.is_empty() {
                    col = col.child(div().p(t.sp4).text_color(t.muted).child("No stashes"));
                } else {
                    for s in &self.git_stashes {
                        col = col.child(self.git_stash_row(s));
                    }
                }
            }
        }
        col.into_any_element()
    }

    // ── Monitor panel (real local metrics) ───────────────────────
    fn meter(
        &self,
        label: &'static str,
        value: String,
        pct: f64,
        color: Hsla,
    ) -> impl IntoElement {
        let t = &self.theme;
        let frac = (pct.clamp(0.0, 100.0) / 100.0) as f32;
        v_flex()
            .gap(px(5.0))
            .px(t.sp3)
            .py(t.sp2)
            .child(
                h_flex()
                    .justify_between()
                    .child(div().text_size(t.fs_ui).text_color(t.ink_2).child(label))
                    .child(
                        div()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(value),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(6.0))
                    .rounded(px(3.0))
                    .bg(t.panel_2)
                    .child(
                        div()
                            .h_full()
                            .w(relative(frac))
                            .rounded(px(3.0))
                            .bg(color),
                    ),
            )
    }

    fn info_row(&self, label: &'static str, value: String) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .justify_between()
            .px(t.sp3)
            .py(px(3.0))
            .child(div().text_size(t.fs_ui).text_color(t.muted).child(label))
            .child(
                div()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink_2)
                    .child(value),
            )
    }

    fn monitor_panel(&self) -> AnyElement {
        let t = &self.theme;
        let Some(m) = &self.mon else {
            return v_flex()
                .flex_1()
                .child(self.panel_header("activity", "MONITOR", ""))
                .child(div().p(t.sp4).text_color(t.muted).child("Sampling…"))
                .into_any_element();
        };
        let gb = |mb: f64| format!("{:.1} GB", mb / 1024.0);
        let mem_pct = if m.mem_total_mb > 0.0 {
            m.mem_used_mb / m.mem_total_mb * 100.0
        } else {
            0.0
        };
        let swap_pct = if m.swap_total_mb > 0.0 {
            m.swap_used_mb / m.swap_total_mb * 100.0
        } else {
            0.0
        };

        let mut col = v_flex()
            .flex_1()
            .min_h(px(0.0))
            .child(self.panel_header("activity", "MONITOR", "localhost"))
            .child(self.meter(
                "CPU",
                format!("{:.0}% · {} cores", m.cpu_pct, m.cpu_count),
                m.cpu_pct,
                level_color(t, m.cpu_pct),
            ))
            .child(self.meter(
                "Memory",
                format!("{} / {}", gb(m.mem_used_mb), gb(m.mem_total_mb)),
                mem_pct,
                level_color(t, mem_pct),
            ));
        if m.swap_total_mb > 0.0 {
            col = col.child(self.meter(
                "Swap",
                format!("{} / {}", gb(m.swap_used_mb), gb(m.swap_total_mb)),
                swap_pct,
                level_color(t, swap_pct),
            ));
        }
        col = col
            .child(self.section_label("SYSTEM"))
            .child(self.info_row("Uptime", m.uptime.clone()))
            .child(self.info_row("Processes", m.proc_count.to_string()));
        if let Some((l1, l5, l15)) = m.load {
            col = col.child(self.info_row("Load", format!("{l1:.2} {l5:.2} {l15:.2}")));
        }
        col = col.child(self.info_row("OS", m.os_label.clone()));
        col.into_any_element()
    }

    fn right_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let (svc, glyph, name, _) = TOOLS[self.active_tool];
        if matches!(svc, Svc::Git) {
            div()
                .id("git-scroll")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(self.git_panel(cx))
                .into_any_element()
        } else if matches!(svc, Svc::Monitor) {
            div()
                .id("mon-scroll")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(self.monitor_panel())
                .into_any_element()
        } else if let Some(view) = self.panels.for_svc(svc) {
            // Each panel View (src/panels/*.rs) owns its own layout and scroll.
            view
        } else {
            v_flex()
                .flex_1()
                .child(self.panel_header(glyph, name, "panel"))
                .child(ui::empty_state(&self.theme, "not implemented"))
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
        let (branch, ahead_behind) = match &self.git {
            Some(g) => (g.branch.clone(), format!("↑{} ↓{}", g.ahead, g.behind)),
            None => ("—".to_string(), String::new()),
        };
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
                            .child(self.status_item(branch, t.ink_2)),
                    )
                    .child(self.status_item(ahead_behind, t.muted))
                    .child(self.status_item("local · pwsh", t.ink_2))
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
        // Pick up the current global theme so dark/light toggles propagate.
        self.theme = cx.global::<Theme>().clone();
        let t = self.theme.clone();
        let active_terminal = self.tabs[self.active_tab].terminal.clone();
        let (cols, rows) = active_terminal.read(cx).size();

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
                    .child(self.right_panel(cx)),
            );
        }
        right_zone = right_zone.child(self.tool_strip(cx));

        v_flex()
            .size_full()
            .font_family(t.sans.clone())
            .text_size(t.fs_body)
            .text_color(t.ink)
            .bg(t.bg)
            .child(self.topbar(cx))
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
                                    .child(active_terminal),
                            ),
                    )
                    .child(right_zone),
            )
            .child(self.status_bar(cols, rows))
    }
}

/// Usage-bar colour: green under 60%, amber under 85%, red above.
fn level_color(t: &Theme, pct: f64) -> Hsla {
    if pct >= 85.0 {
        t.neg
    } else if pct >= 60.0 {
        t.warn
    } else {
        t.pos
    }
}

/// Single-char mark + colour for a git file status.
fn status_style(t: &Theme, s: &FileStatus) -> (&'static str, Hsla) {
    let color = match s {
        FileStatus::Modified => t.warn,
        FileStatus::Added => t.pos,
        FileStatus::Deleted => t.neg,
        FileStatus::Renamed => t.info,
        FileStatus::Untracked => t.muted,
        FileStatus::Conflicted => t.neg,
        FileStatus::Copied => t.info,
    };
    (s.code(), color)
}
