//! Three-pane shell mirroring `Pier/PierApp/Sources/Views/MainWindow/MainView.swift`.
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │ Toolbar: [☰L]  Pier-X            [+] [☰R] [☀/☾] │
//! ├──────────┬───────────────────────┬──────────────┤
//! │   Left   │  Tab bar (terminals)  │     Right    │
//! │ Files /  │  ─────────────────    │ 10 mode      │
//! │ Servers  │  Active terminal      │ container +  │
//! │          │   OR Welcome cover    │ icon sidebar │
//! └──────────┴───────────────────────┴──────────────┘
//! ```

use std::collections::HashSet;
use std::env;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    div, prelude::*, px, AnyElement, ClickEvent, Context, Entity, IntoElement, SharedString,
    Window,
};
use gpui_component::{Icon as UiIcon, IconName};
use pier_core::connections::ConnectionStore;
use pier_core::ssh::SshConfig;

use crate::app::layout::{
    LeftTab, RightMode, LEFT_PANEL_DEFAULT_W, RIGHT_PANEL_DEFAULT_W, TOOLBAR_HEIGHT,
};
use crate::app::ActivationHandler;
use crate::data::ShellSnapshot;
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_CAPTION, SIZE_SMALL, WEIGHT_MEDIUM},
    ThemeMode,
};
use crate::views::file_tree::{
    FileTree, GoUpHandler, OpenFileHandler, ToggleDirHandler,
};
use crate::views::left_panel::{
    icons as toolbar_icons, LeftPanel, ServerSelector, TabSelector,
};
use crate::views::right_panel::{ModeSelector, RightPanel};
use crate::views::terminal::TerminalPanel;
use crate::views::welcome::WelcomeView;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;

pub struct PierApp {
    // ─── Layout state ───
    left_visible: bool,
    right_visible: bool,
    left_tab: LeftTab,
    right_mode: RightMode,

    // ─── Backend snapshots (re-loaded on relevant events) ───
    snapshot: ShellSnapshot,
    connections: Vec<SshConfig>,

    // ─── Terminal sessions (Pier mirror: multi-tab) ───
    terminals: Vec<Entity<TerminalPanel>>,
    active_terminal: Option<usize>,

    // ─── Local file tree (Files tab) ───
    file_tree_root: PathBuf,
    file_tree_expanded: HashSet<PathBuf>,
    /// Last file the user clicked in the tree. Wired into the Markdown
    /// mode in Phase 3; for now just kept so the open-file handler has
    /// somewhere to land.
    last_opened_file: Option<PathBuf>,
}

impl PierApp {
    pub fn new() -> Self {
        let connections = ConnectionStore::load_default()
            .map(|s| s.connections)
            .unwrap_or_default();
        let file_tree_root = env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        Self {
            left_visible: true,
            right_visible: true,
            left_tab: LeftTab::Files,
            right_mode: RightMode::Markdown,
            snapshot: ShellSnapshot::load(),
            connections,
            terminals: Vec::new(),
            active_terminal: None,
            file_tree_root,
            file_tree_expanded: HashSet::new(),
            last_opened_file: None,
        }
    }

    // ─── Terminal session management ───

    fn open_terminal_tab(&mut self, cx: &mut Context<Self>) {
        let on_activated: ActivationHandler = Rc::new(|_, _, _| {});
        let entity = cx.new(|cx| TerminalPanel::new(on_activated, cx));
        self.terminals.push(entity);
        self.active_terminal = Some(self.terminals.len() - 1);
        cx.notify();
    }

    fn activate_terminal_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.terminals.len() {
            self.active_terminal = Some(idx);
            cx.notify();
        }
    }

    fn close_terminal_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.terminals.len() {
            return;
        }
        self.terminals.remove(idx);
        if self.terminals.is_empty() {
            self.active_terminal = None;
        } else {
            // Snap active to a valid index, preferring the previous neighbour.
            let new_active = match self.active_terminal {
                Some(active) if active == idx => idx.saturating_sub(1).min(self.terminals.len() - 1),
                Some(active) if active > idx => active - 1,
                Some(active) => active,
                None => 0,
            };
            self.active_terminal = Some(new_active);
        }
        cx.notify();
    }

    // ─── File tree ───

    fn toggle_dir(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.file_tree_expanded.contains(&path) {
            self.file_tree_expanded.remove(&path);
        } else {
            self.file_tree_expanded.insert(path);
        }
        cx.notify();
    }

    fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // Phase 3 hooks: if .md → switch right mode to Markdown + load.
        // For now we just remember the path and emit an info log so the
        // wiring is testable end-to-end.
        eprintln!("[pier] file opened: {}", path.display());
        if path
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("md"))
            .unwrap_or(false)
        {
            self.right_mode = RightMode::Markdown;
            self.right_visible = true;
        }
        self.last_opened_file = Some(path);
        cx.notify();
    }

    fn cd_up(&mut self, cx: &mut Context<Self>) {
        if let Some(parent) = self.file_tree_root.parent() {
            let parent = parent.to_path_buf();
            // Drop expanded entries that are no longer reachable from the
            // new root — keeps the set small and prevents stale state.
            self.file_tree_expanded
                .retain(|p| p.starts_with(&parent));
            self.file_tree_root = parent;
            cx.notify();
        }
    }

    fn refresh_connections(&mut self) {
        self.connections = ConnectionStore::load_default()
            .map(|s| s.connections)
            .unwrap_or_default();
    }
}

impl Default for PierApp {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for PierApp {
    fn render(&mut self, _win: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();

        let toolbar = self.render_toolbar(&t, cx);
        let left = self
            .left_visible
            .then(|| self.render_left(cx))
            .map(IntoElement::into_any_element);
        let center = self.render_center(&t, cx);
        let right = self
            .right_visible
            .then(|| self.render_right(cx))
            .map(IntoElement::into_any_element);

        let mut row = div().flex().flex_row().flex_1().min_h(px(0.0));
        if let Some(panel) = left {
            row = row.child(div().w(LEFT_PANEL_DEFAULT_W).h_full().child(panel));
        }
        row = row.child(div().flex_1().min_w(px(0.0)).h_full().child(center));
        if let Some(panel) = right {
            row = row.child(div().w(RIGHT_PANEL_DEFAULT_W).h_full().child(panel));
        }

        div()
            .size_full()
            .bg(t.color.bg_canvas)
            .text_color(t.color.text_primary)
            .font_family(t.font_ui.clone())
            .flex()
            .flex_col()
            .child(toolbar)
            .child(row)
    }
}

// ─────────────────────────────────────────────────────────
// Toolbar
// ─────────────────────────────────────────────────────────

impl PierApp {
    fn render_toolbar(
        &self,
        t: &crate::theme::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let toggle_left_icon = if self.left_visible {
            toolbar_icons::TOGGLE_LEFT_OPEN
        } else {
            toolbar_icons::TOGGLE_LEFT_CLOSED
        };
        let toggle_right_icon = if self.right_visible {
            toolbar_icons::TOGGLE_RIGHT_OPEN
        } else {
            toolbar_icons::TOGGLE_RIGHT_CLOSED
        };
        let theme_icon = if t.mode == ThemeMode::Dark {
            toolbar_icons::SUN
        } else {
            toolbar_icons::MOON
        };

        div()
            .h(TOOLBAR_HEIGHT)
            .px(SP_3)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .bg(t.color.bg_panel)
            .border_b_1()
            .border_color(t.color.border_subtle)
            .child(toolbar_icon_button(
                t,
                "tb-toggle-left",
                toggle_left_icon,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.left_visible = !this.left_visible;
                    cx.notify();
                }),
            ))
            .child(
                div()
                    .text_size(SIZE_SMALL)
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(t.color.text_primary)
                    .child("Pier-X"),
            )
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .text_color(t.color.text_tertiary)
                    .child(self.snapshot.workspace_path.clone()),
            )
            .child(div().flex_1())
            .child(toolbar_icon_button(
                t,
                "tb-new-tab",
                toolbar_icons::NEW_TAB,
                cx.listener(|this, _: &ClickEvent, _, cx| this.open_terminal_tab(cx)),
            ))
            .child(toolbar_icon_button(
                t,
                "tb-toggle-right",
                toggle_right_icon,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.right_visible = !this.right_visible;
                    cx.notify();
                }),
            ))
            .child(toolbar_icon_button(
                t,
                "tb-toggle-theme",
                theme_icon,
                |_: &ClickEvent, _, app| {
                    crate::theme::toggle(app);
                    crate::ui_kit::sync_theme(app);
                },
            ))
    }
}

fn toolbar_icon_button(
    t: &crate::theme::Theme,
    id: &'static str,
    icon: IconName,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(28.0))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .text_color(t.color.text_secondary)
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(on_click)
        .child(UiIcon::new(icon).size(px(14.0)))
}

// ─────────────────────────────────────────────────────────
// Left / Center / Right
// ─────────────────────────────────────────────────────────

impl PierApp {
    fn render_left(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let on_select_tab: TabSelector = Rc::new(cx.listener(
            |this, tab: &LeftTab, _, cx| {
                this.left_tab = *tab;
                if *tab == LeftTab::Servers {
                    this.refresh_connections();
                }
                cx.notify();
            },
        ));
        let on_select_server: ServerSelector =
            Rc::new(cx.listener(|_, idx: &usize, _, _| {
                eprintln!("[pier] server clicked idx={idx} (open-tab dialog lands in Phase 3)");
            }));

        let on_toggle_dir: ToggleDirHandler = Rc::new(cx.listener(
            |this, path: &PathBuf, _, cx| this.toggle_dir(path.clone(), cx),
        ));
        let on_open_file: OpenFileHandler = Rc::new(cx.listener(
            |this, path: &PathBuf, _, cx| this.open_file(path.clone(), cx),
        ));
        let on_go_up: GoUpHandler =
            Rc::new(cx.listener(|this, _: &(), _, cx| this.cd_up(cx)));

        let file_tree = FileTree::new(
            self.file_tree_root.clone(),
            self.file_tree_expanded.clone(),
            on_toggle_dir,
            on_open_file,
            on_go_up,
        );

        LeftPanel::new(
            self.left_tab,
            self.connections.clone(),
            file_tree,
            on_select_tab,
            on_select_server,
        )
    }

    fn render_right(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let on_select_mode: ModeSelector = Rc::new(cx.listener(
            |this, mode: &RightMode, _, cx| {
                this.right_mode = *mode;
                cx.notify();
            },
        ));
        RightPanel::new(self.right_mode, self.snapshot.clone(), on_select_mode)
    }

    fn render_center(
        &mut self,
        t: &crate::theme::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(active) = self.active_terminal {
            let tab_count = self.terminals.len();
            let active_term = self.terminals[active].clone();

            let tab_bar = render_terminal_tab_bar(t, active, tab_count, cx);

            return div()
                .h_full()
                .flex()
                .flex_col()
                .child(tab_bar)
                .child(div().flex_1().min_h(px(0.0)).child(active_term))
                .into_any_element();
        }

        // Welcome cover state — uses the existing WelcomeView, but its
        // buttons map to 3-pane semantics (open Servers tab / open terminal).
        let connections = self.connections.clone();
        let on_new_ssh: ClickHandler = Box::new(cx.listener(
            |this, _ev: &ClickEvent, _, cx| {
                this.left_tab = LeftTab::Servers;
                this.left_visible = true;
                this.refresh_connections();
                cx.notify();
            },
        ));
        let on_open_terminal: ClickHandler = Box::new(cx.listener(
            |this, _ev: &ClickEvent, _, cx| this.open_terminal_tab(cx),
        ));

        div()
            .size_full()
            .bg(t.color.bg_canvas)
            .flex()
            .items_center()
            .justify_center()
            .child(WelcomeView::new(connections, on_new_ssh, on_open_terminal))
            .into_any_element()
    }
}

// ─────────────────────────────────────────────────────────
// Terminal tab bar
// ─────────────────────────────────────────────────────────

fn render_terminal_tab_bar(
    t: &crate::theme::Theme,
    active: usize,
    count: usize,
    cx: &mut Context<PierApp>,
) -> impl IntoElement {
    let mut row = div()
        .h(px(28.0))
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .bg(t.color.bg_panel)
        .border_b_1()
        .border_color(t.color.border_subtle);

    for idx in 0..count {
        let is_active = idx == active;
        let label: SharedString = format!("Terminal {}", idx + 1).into();
        let tab_id: SharedString = format!("term-tab-{idx}").into();
        let close_id: SharedString = format!("term-close-{idx}").into();

        let on_select = cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.activate_terminal_tab(idx, cx);
        });
        let on_close = cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.close_terminal_tab(idx, cx);
        });

        let mut tab = div()
            .id(gpui::ElementId::Name(tab_id))
            .h(px(22.0))
            .px(SP_2)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1_5)
            .rounded(RADIUS_SM)
            .text_size(SIZE_CAPTION)
            .font_weight(WEIGHT_MEDIUM)
            .text_color(if is_active {
                t.color.text_primary
            } else {
                t.color.text_secondary
            })
            .cursor_pointer()
            .hover(|s| s.bg(t.color.bg_hover))
            .on_click(on_select)
            .child(UiIcon::new(IconName::SquareTerminal).size(px(12.0)))
            .child(label)
            .child(
                div()
                    .id(gpui::ElementId::Name(close_id))
                    .w(px(14.0))
                    .h(px(14.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(2.0))
                    .text_color(t.color.text_tertiary)
                    .hover(|s| s.bg(t.color.bg_active).text_color(t.color.text_primary))
                    .on_click(on_close)
                    .child(UiIcon::new(IconName::Close).size(px(10.0))),
            );

        if is_active {
            tab = tab.bg(t.color.bg_surface);
        }
        row = row.child(tab);
    }

    // Inline "+" at end-of-row for quick new tab without going to toolbar.
    let on_new = cx.listener(|this, _: &ClickEvent, _, cx| this.open_terminal_tab(cx));
    row.child(
        div()
            .id("term-tab-plus")
            .w(px(22.0))
            .h(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_SM)
            .text_color(t.color.text_secondary)
            .cursor_pointer()
            .hover(|s| s.bg(t.color.bg_hover))
            .on_click(on_new)
            .child(UiIcon::new(IconName::Plus).size(px(12.0))),
    )
}
