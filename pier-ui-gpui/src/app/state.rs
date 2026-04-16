use gpui::{div, prelude::*, px, ClickEvent, Context, IntoElement, SharedString, Window};
use pier_core::connections::ConnectionStore;
use pier_core::ssh::SshConfig;

use crate::app::route::{DbKind, Route};
use crate::components::{Button, NavItem, StatusKind, StatusPill};
use crate::data::ShellSnapshot;
use crate::theme::{
    spacing::{SP_2, SP_3, SP_4},
    theme,
    typography::{SIZE_CAPTION, SIZE_SMALL, WEIGHT_MEDIUM},
    ThemeMode,
};
use crate::views::dashboard::DashboardView;
use crate::views::database::DatabaseView;
use crate::views::git::GitView;
use crate::views::ssh::SshView;
use crate::views::terminal::TerminalView;
use crate::views::welcome::WelcomeView;

pub struct PierApp {
    route: Route,
    snapshot: ShellSnapshot,
    connections: Vec<SshConfig>,
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
            snapshot: ShellSnapshot::load(),
            connections,
        }
    }

    fn navigate(this: &mut Self, route: Route, cx: &mut Context<Self>) {
        if route != this.route {
            this.route = route;
            // Re-probe data on tab change so SSH list, git status, etc.
            // pick up filesystem changes without an explicit watcher.
            this.refresh(route);
            cx.notify();
        }
    }

    fn refresh(&mut self, route: Route) {
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
}

impl Default for PierApp {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for PierApp {
    fn render(&mut self, _win: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.route {
            Route::Welcome => self.render_welcome(cx).into_any_element(),
            _ => self.render_dock(cx).into_any_element(),
        }
    }
}

impl PierApp {
    fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let connections = self.connections.clone();
        let on_new_ssh = Box::new(cx.listener(
            |this, _ev: &ClickEvent, _w, cx| Self::navigate(this, Route::Ssh, cx),
        )) as Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;
        let on_open_terminal = Box::new(cx.listener(
            |this, _ev: &ClickEvent, _w, cx| Self::navigate(this, Route::Terminal, cx),
        )) as Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;

        WelcomeView::new(connections, on_new_ssh, on_open_terminal)
    }

    fn render_dock(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let route = self.route;
        let canvas = match route {
            Route::Welcome => unreachable!(),
            Route::Dashboard => DashboardView::new(self.snapshot.clone()).into_any_element(),
            Route::Terminal => TerminalView::new().into_any_element(),
            Route::Git => GitView::new().into_any_element(),
            Route::Ssh => SshView::new().into_any_element(),
            Route::Database(kind) => DatabaseView::new(kind).into_any_element(),
        };

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
                    .child(div().flex_1().min_w(px(0.0)).child(canvas)),
            )
            .child(self.render_statusbar(&t, route))
    }

    fn render_topbar(
        &self,
        t: &crate::theme::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
                    .child(SharedString::from(self.snapshot.workspace_path.clone())),
            )
            .child(div().flex_1())
            .child(
                Button::ghost("topbar-back-welcome", "Welcome").on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| Self::navigate(this, Route::Welcome, cx),
                )),
            )
            .child(Button::ghost("topbar-toggle-theme", theme_label).on_click(|_, _, cx| {
                crate::theme::toggle(cx)
            }))
    }

    fn render_sidebar(
        &self,
        t: &crate::theme::Theme,
        active: Route,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let nav = |route: Route| -> Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static> {
            Box::new(cx.listener(move |this, _: &ClickEvent, _, cx| {
                Self::navigate(this, route, cx)
            }))
        };

        let primary = [
            Route::Dashboard,
            Route::Terminal,
            Route::Git,
            Route::Ssh,
        ];

        let mut col = div()
            .w(px(208.0))
            .h_full()
            .flex()
            .flex_col()
            .gap(SP_2)
            .p(SP_3)
            .bg(t.color.bg_panel)
            .border_r_1()
            .border_color(t.color.border_subtle);

        col = col.child(sidebar_section_label(t, "Workspace"));
        for route in primary {
            col = col.child(
                NavItem::new(route.id(), route.label())
                    .active(active == route)
                    .on_click(nav(route)),
            );
        }

        col = col.child(div().h(px(8.0))); // spacer
        col = col.child(sidebar_section_label(t, "Databases"));
        for kind in DbKind::ALL {
            let route = Route::Database(kind);
            col = col.child(
                NavItem::new(route.id(), kind.label())
                    .active(active == route)
                    .on_click(nav(route)),
            );
        }

        col
    }

    fn render_statusbar(
        &self,
        t: &crate::theme::Theme,
        active: Route,
    ) -> impl IntoElement {
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
                    .child(SharedString::from(self.snapshot.core_version.clone())),
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

fn sidebar_section_label(t: &crate::theme::Theme, label: &'static str) -> impl IntoElement {
    div()
        .px(SP_3)
        .text_size(SIZE_CAPTION)
        .font_weight(WEIGHT_MEDIUM)
        .text_color(t.color.text_tertiary)
        .child(label)
}

