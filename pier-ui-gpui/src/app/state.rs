//! Three-pane shell mirroring `Pier/PierApp/Sources/Views/MainWindow/MainView.swift`.
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │ Toolbar: [☰L]  Pier-X            [+] [☰R] [☀/☾] │
//! ├──────────┬───────────────────────┬──────────────┤
//! │   Left   │       Center          │     Right    │
//! │ Files /  │  Welcome cover        │ 10 mode      │
//! │ Servers  │   OR Terminal         │ container +  │
//! │          │                       │ icon sidebar │
//! └──────────┴───────────────────────┴──────────────┘
//! ```

use std::rc::Rc;

use gpui::{
    div, prelude::*, px, AnyElement, ClickEvent, Context, Entity, IntoElement, Window,
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
    spacing::{SP_2, SP_3, SP_4},
    theme,
    typography::{SIZE_CAPTION, SIZE_SMALL, WEIGHT_MEDIUM},
    ThemeMode,
};
use crate::views::left_panel::{
    icons as toolbar_icons, LeftPanel, ServerSelector, TabSelector,
};
use crate::views::right_panel::{ModeSelector, RightPanel};
use crate::views::terminal::TerminalPanel;
use crate::views::welcome::WelcomeView;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;

pub struct PierApp {
    // ─── 3-pane state ───
    left_visible: bool,
    right_visible: bool,
    left_tab: LeftTab,
    right_mode: RightMode,
    /// Whether the user has opened a terminal session yet. When false the
    /// center column shows the Welcome cover; when true it renders the cached
    /// `terminal` entity. Set to true on the first `open_terminal`.
    terminal_open: bool,

    // ─── Backend data (re-loaded on relevant events) ───
    snapshot: ShellSnapshot,
    connections: Vec<SshConfig>,

    // ─── Cached child entities ───
    /// PTY-backed terminal panel; lazy-created the first time the user opens
    /// a session so the underlying shell process survives panel toggles.
    terminal: Option<Entity<TerminalPanel>>,
}

impl PierApp {
    pub fn new() -> Self {
        let connections = ConnectionStore::load_default()
            .map(|s| s.connections)
            .unwrap_or_default();
        Self {
            left_visible: true,
            right_visible: true,
            left_tab: LeftTab::Files,
            right_mode: RightMode::Markdown,
            terminal_open: false,
            snapshot: ShellSnapshot::load(),
            connections,
            terminal: None,
        }
    }

    fn ensure_terminal(&mut self, cx: &mut Context<Self>) {
        if self.terminal.is_some() {
            return;
        }
        let on_activated: ActivationHandler = Rc::new(|_, _, _| {});
        let entity = cx.new(|cx| TerminalPanel::new(on_activated, cx));
        self.terminal = Some(entity);
    }

    fn open_terminal(&mut self, cx: &mut Context<Self>) {
        self.ensure_terminal(cx);
        if !self.terminal_open {
            self.terminal_open = true;
        }
        cx.notify();
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

        // Build child elements first to release self/cx borrows in order.
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

        let mut row = div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h(px(0.0));
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
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    // Phase 1: single terminal session. Multi-tab dispatch
                    // (terminal vs SSH chooser) lands in Phase 2 alongside
                    // a real `NewTabChooserView` mirror.
                    this.open_terminal(cx);
                }),
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
                eprintln!("[pier] server clicked idx={idx} (open-tab dialog lands in Phase 2)");
            }));

        LeftPanel::new(
            self.left_tab,
            self.connections.clone(),
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
        if self.terminal_open {
            // ensure_terminal already ran inside open_terminal; double-check
            // for the (impossible) case where state is desynced.
            self.ensure_terminal(cx);
            return self
                .terminal
                .as_ref()
                .expect("terminal was just ensured")
                .clone()
                .into_any_element();
        }

        // Welcome cover state — uses the existing WelcomeView, but its
        // buttons map to 3-pane semantics instead of dispatching routes.
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
            |this, _ev: &ClickEvent, _, cx| this.open_terminal(cx),
        ));

        // Wrap in a centered container (mirrors Pier's empty-state visual).
        div()
            .size_full()
            .bg(t.color.bg_canvas)
            .flex()
            .items_center()
            .justify_center()
            .p(SP_4)
            .child(WelcomeView::new(connections, on_new_ssh, on_open_terminal))
            .into_any_element()
    }
}
