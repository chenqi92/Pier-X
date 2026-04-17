use gpui::{div, prelude::*, px, ClickEvent, Context, IntoElement, SharedString, Window};
use gpui_component::button::{Button as UiButton, ButtonVariants};
use gpui_component::sidebar::{
    Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem,
};
use pier_core::connections::ConnectionStore;
use pier_core::ssh::SshConfig;

use crate::app::route::{DbKind, Route};
use crate::app::workbench::{route_icon, Workbench};
use crate::components::{StatusKind, StatusPill};
use crate::data::ShellSnapshot;
use crate::theme::{
    spacing::{SP_3, SP_4},
    theme,
    typography::{SIZE_CAPTION, SIZE_SMALL, WEIGHT_MEDIUM},
    ThemeMode,
};
use crate::views::welcome::WelcomeView;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;

pub struct PierApp {
    route: Route,
    main_route: Route,
    snapshot: ShellSnapshot,
    connections: Vec<SshConfig>,
    workbench: Option<Workbench>,
}

impl PierApp {
    pub fn new() -> Self {
        let connections = ConnectionStore::load_default()
            .map(|store| store.connections)
            .unwrap_or_default();
        // Always default to Welcome — it's the cover regardless of saved
        // connections; the dock is reachable via its buttons.
        Self {
            route: Route::Welcome,
            main_route: Route::Dashboard,
            snapshot: ShellSnapshot::load(),
            connections,
            workbench: None,
        }
    }

    pub fn sync_route_from_panel(&mut self, route: Route) {
        self.route = route;
        if route.is_primary() {
            self.main_route = route;
        }
    }

    fn navigate(this: &mut Self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        if route.is_primary() {
            this.main_route = route;
        }

        if route != this.route {
            this.route = route;
            // Re-probe data on tab change so SSH list, git status, etc.
            // pick up filesystem changes without an explicit watcher.
            this.refresh(route);
            if route != Route::Welcome {
                this.ensure_workbench(window, cx);
                if let Some(workbench) = this.workbench.as_ref() {
                    workbench.sync(this.main_route, this.route, window, cx);
                }
            }
            cx.notify();
        }
    }

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

    fn ensure_workbench(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workbench.is_none() {
            let workbench = Workbench::new(cx.entity().downgrade(), window, cx);
            workbench.sync(self.main_route, self.route, window, cx);
            self.workbench = Some(workbench);
        }
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
            _ => self.render_dock(win, cx).into_any_element(),
        }
    }
}

impl PierApp {
    fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let connections = self.connections.clone();
        let on_new_ssh = Box::new(
            cx.listener(|this, _ev: &ClickEvent, w, cx| Self::navigate(this, Route::Ssh, w, cx)),
        )
            as Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;
        let on_open_terminal = Box::new(cx.listener(|this, _ev: &ClickEvent, _w, cx| {
            Self::navigate(this, Route::Terminal, _w, cx)
        }))
            as Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;

        WelcomeView::new(connections, on_new_ssh, on_open_terminal)
    }

    fn render_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_workbench(window, cx);
        let t = theme(cx).clone();
        let route = self.route;
        let dock = self
            .workbench
            .as_ref()
            .expect("workbench must exist after ensure_workbench")
            .dock();

        div()
            .size_full()
            .bg(t.color.bg_canvas)
            .text_color(t.color.text_primary)
            .font_family(t.font_ui.clone())
            .flex()
            .flex_col()
            .child(self.render_topbar(&t, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(self.render_sidebar(&t, route, cx))
                    .child(div().flex_1().min_w(px(0.0)).child(dock)),
            )
            .child(self.render_statusbar(&t, route))
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
