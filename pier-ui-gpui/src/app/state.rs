use std::rc::Rc;

use gpui::{
    div, prelude::*, px, AnyElement, ClickEvent, Context, Entity, IntoElement, SharedString,
    Window,
};
use gpui_component::button::{Button as UiButton, ButtonVariants};
use gpui_component::sidebar::{
    Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem,
};
use pier_core::connections::ConnectionStore;
use pier_core::ssh::SshConfig;

use crate::app::route::{DbKind, Route};
use crate::app::{route_icon, ActivationHandler};
use crate::components::{StatusKind, StatusPill};
use crate::data::ShellSnapshot;
use crate::theme::{
    spacing::{SP_3, SP_4},
    theme,
    typography::{SIZE_CAPTION, SIZE_SMALL, WEIGHT_MEDIUM},
    ThemeMode,
};
use crate::views::dashboard::DashboardView;
use crate::views::database::DatabaseView;
use crate::views::git::GitView;
use crate::views::ssh::SshView;
use crate::views::terminal::TerminalPanel;
use crate::views::welcome::WelcomeView;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;

pub struct PierApp {
    route: Route,
    snapshot: ShellSnapshot,
    connections: Vec<SshConfig>,
    /// Cached so the PTY survives across renders / route changes.
    /// Lazy-created on the first navigation to [`Route::Terminal`].
    terminal: Option<Entity<TerminalPanel>>,
}

impl PierApp {
    pub fn new() -> Self {
        let connections = ConnectionStore::load_default()
            .map(|store| store.connections)
            .unwrap_or_default();
        Self {
            route: Route::Welcome,
            snapshot: ShellSnapshot::load(),
            connections,
            terminal: None,
        }
    }

    fn navigate(this: &mut Self, route: Route, _window: &mut Window, cx: &mut Context<Self>) {
        if route == this.route {
            return;
        }
        this.route = route;
        this.refresh(route);
        cx.notify();
    }

    /// Re-probe filesystem-backed data so the chosen tab is always fresh.
    /// Acts as a zero-dependency replacement for a `notify`-based watcher.
    pub(crate) fn refresh(&mut self, route: Route) {
        match route {
            Route::Welcome | Route::Dashboard => {
                self.snapshot = ShellSnapshot::load();
                self.connections = ConnectionStore::load_default()
                    .map(|s| s.connections)
                    .unwrap_or_default();
            }
            Route::Ssh => {
                self.connections = ConnectionStore::load_default()
                    .map(|s| s.connections)
                    .unwrap_or_default();
            }
            _ => {}
        }
    }

    fn terminal_entity(&mut self, cx: &mut Context<Self>) -> Entity<TerminalPanel> {
        if let Some(t) = self.terminal.as_ref() {
            return t.clone();
        }
        // The dock-based draft used `on_activated` to re-sync the route from
        // the panel; in the canvas-only shell the panel is only mounted when
        // its route is already active, so a no-op is correct.
        let on_activated: ActivationHandler = Rc::new(|_, _, _| {});
        let terminal = cx.new(|cx| TerminalPanel::new(on_activated, cx));
        self.terminal = Some(terminal.clone());
        terminal
    }
}

impl Default for PierApp {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for PierApp {
    fn render(&mut self, win: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.route {
            Route::Welcome => self.render_welcome(cx).into_any_element(),
            _ => self.render_workbench(win, cx).into_any_element(),
        }
    }
}

impl PierApp {
    fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let connections = self.connections.clone();
        let on_new_ssh = Box::new(cx.listener(
            |this, _ev: &ClickEvent, w, cx| Self::navigate(this, Route::Ssh, w, cx),
        )) as ClickHandler;
        let on_open_terminal = Box::new(cx.listener(
            |this, _ev: &ClickEvent, w, cx| Self::navigate(this, Route::Terminal, w, cx),
        )) as ClickHandler;

        WelcomeView::new(connections, on_new_ssh, on_open_terminal)
    }

    fn render_workbench(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let route = self.route;

        // Build canvas first (mutable borrow of self for terminal cache)
        // then chrome — each call returns owned elements, releasing the borrow.
        let canvas = self.render_canvas(route, cx);
        let topbar = self.render_topbar(&t, cx);
        let sidebar = self.render_sidebar(&t, route, cx);
        let statusbar = self.render_statusbar(&t, route);

        div()
            .size_full()
            .bg(t.color.bg_canvas)
            .text_color(t.color.text_primary)
            .font_family(t.font_ui.clone())
            .flex()
            .flex_col()
            .child(topbar)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(sidebar)
                    .child(div().flex_1().min_w(px(0.0)).child(canvas)),
            )
            .child(statusbar)
    }

    fn render_canvas(&mut self, route: Route, cx: &mut Context<Self>) -> AnyElement {
        match route {
            // Welcome is rendered as the root by `render`; if we land here
            // (defensive) just show an empty canvas instead of panicking.
            Route::Welcome => div().size_full().into_any_element(),
            Route::Dashboard => DashboardView::new(self.snapshot.clone()).into_any_element(),
            Route::Terminal => self.terminal_entity(cx).into_any_element(),
            Route::Git => GitView::new().into_any_element(),
            Route::Ssh => SshView::new().into_any_element(),
            Route::Database(kind) => DatabaseView::new(kind).into_any_element(),
        }
    }

    fn render_topbar(&self, t: &crate::theme::Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let theme_label: SharedString = if t.mode == ThemeMode::Dark {
            "Switch to light".into()
        } else {
            "Switch to dark".into()
        };
        div()
            .h(px(36.0))
            .px(SP_4)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_3)
            .bg(t.color.bg_panel)
            .border_b_1()
            .border_color(t.color.border_subtle)
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
            .child(
                UiButton::new("topbar-back-welcome")
                    .ghost()
                    .label("Welcome")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        Self::navigate(this, Route::Welcome, window, cx)
                    })),
            )
            .child(
                UiButton::new("topbar-toggle-theme")
                    .ghost()
                    .label(theme_label)
                    .on_click(|_, _, cx| {
                        crate::theme::toggle(cx);
                        crate::ui_kit::sync_theme(cx);
                    }),
            )
    }

    fn render_sidebar(
        &self,
        t: &crate::theme::Theme,
        active: Route,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let nav = |route: Route| -> ClickHandler {
            Box::new(cx.listener(move |this, _: &ClickEvent, window, cx| {
                Self::navigate(this, route, window, cx)
            }))
        };

        let workspace_menu = SidebarMenu::new()
            .child(
                SidebarMenuItem::new(Route::Dashboard.label())
                    .icon(route_icon(Route::Dashboard))
                    .active(active == Route::Dashboard)
                    .on_click(nav(Route::Dashboard)),
            )
            .child(
                SidebarMenuItem::new(Route::Terminal.label())
                    .icon(route_icon(Route::Terminal))
                    .active(active == Route::Terminal)
                    .on_click(nav(Route::Terminal)),
            )
            .child(
                SidebarMenuItem::new(Route::Git.label())
                    .icon(route_icon(Route::Git))
                    .active(active == Route::Git)
                    .on_click(nav(Route::Git)),
            )
            .child(
                SidebarMenuItem::new(Route::Ssh.label())
                    .icon(route_icon(Route::Ssh))
                    .active(active == Route::Ssh)
                    .on_click(nav(Route::Ssh)),
            );

        let mut database_menu = SidebarMenu::new();
        for kind in DbKind::ALL {
            let route = Route::Database(kind);
            database_menu = database_menu.child(
                SidebarMenuItem::new(kind.label())
                    .icon(route_icon(route))
                    .active(active == route)
                    .on_click(nav(route)),
            );
        }

        Sidebar::<SidebarGroup<SidebarMenu>>::left()
            .collapsible(false)
            .header(
                SidebarHeader::new().child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(SIZE_SMALL)
                                .font_weight(WEIGHT_MEDIUM)
                                .text_color(t.color.text_primary)
                                .child("Workbench"),
                        )
                        .child(
                            div()
                                .text_size(SIZE_CAPTION)
                                .text_color(t.color.text_tertiary)
                                .child("Pier-X"),
                        ),
                ),
            )
            .child(SidebarGroup::new("Workspace").child(workspace_menu))
            .child(SidebarGroup::new("Databases").child(database_menu))
            .footer(
                SidebarFooter::new().child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(SIZE_CAPTION)
                                .text_color(t.color.text_tertiary)
                                .child(format!("{} connections", self.connections.len())),
                        )
                        .child(
                            div()
                                .text_size(SIZE_CAPTION)
                                .text_color(t.color.text_tertiary)
                                .child(format!("route: {}", active.id())),
                        ),
                ),
            )
            .w(px(224.0))
    }

    fn render_statusbar(&self, t: &crate::theme::Theme, active: Route) -> impl IntoElement {
        let route_label: SharedString = format!("route: {}", active.id()).into();
        let theme_label: SharedString = if t.mode == ThemeMode::Dark {
            "theme: dark".into()
        } else {
            "theme: light".into()
        };

        div()
            .h(px(24.0))
            .px(SP_3)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_3)
            .bg(t.color.bg_panel)
            .border_t_1()
            .border_color(t.color.border_subtle)
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .text_color(t.color.text_tertiary)
                    .child(self.snapshot.core_version.clone()),
            )
            .child(StatusPill::new(route_label, StatusKind::Info))
            .child(StatusPill::new(theme_label, StatusKind::Success))
            .child(div().flex_1())
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .text_color(t.color.text_tertiary)
                    .child(format!("{} connections", self.connections.len())),
            )
    }
}
