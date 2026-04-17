use std::{collections::HashMap, rc::Rc, sync::Arc};

use gpui::{
    div, px, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, WeakEntity, Window,
};
use gpui_component::{
    dock::{DockArea, DockItem, Panel, PanelControl, PanelEvent, PanelStyle, PanelView},
    Icon as UiIcon, IconName,
};

use crate::{
    app::{route::Route, PierApp},
    data::ShellSnapshot,
    views::{
        dashboard::DashboardView, database::DatabaseView, git::GitView, ssh::SshView,
        terminal::TerminalPanel,
    },
};

pub struct Workbench {
    dock: Entity<DockArea>,
    panels: HashMap<Route, Arc<dyn PanelView>>,
}

impl Workbench {
    pub fn new(app: WeakEntity<PierApp>, window: &mut Window, cx: &mut Context<PierApp>) -> Self {
        let on_activated: ActivationHandler = Rc::new(move |route, _window, cx| {
            let _ = app.update(cx, |this, cx| {
                this.sync_route_from_panel(route);
                this.refresh(route);
                cx.notify();
            });
        });

        let dock = cx.new(|cx| {
            DockArea::new("pier-workbench", Some(1), window, cx).panel_style(PanelStyle::TabBar)
        });

        let panels = [
            Route::Dashboard,
            Route::Terminal,
            Route::Git,
            Route::Ssh,
            Route::mysql(),
            Route::postgres(),
            Route::redis(),
            Route::sqlite(),
        ]
        .into_iter()
        .map(|route| {
            let panel: Arc<dyn PanelView> = match route {
                Route::Terminal => {
                    let panel = cx.new(|cx| TerminalPanel::new(on_activated.clone(), cx));
                    Arc::new(panel)
                }
                _ => {
                    let panel = cx.new(|cx| WorkbenchPanel::new(route, on_activated.clone(), cx));
                    Arc::new(panel)
                }
            };
            (route, panel)
        })
        .collect();

        Self { dock, panels }
    }

    pub fn dock(&self) -> Entity<DockArea> {
        self.dock.clone()
    }

    pub fn sync(
        &self,
        main_route: Route,
        selected_route: Route,
        window: &mut Window,
        cx: &mut Context<PierApp>,
    ) {
        self.dock.update(cx, |dock, cx| {
            let weak_dock = cx.entity().downgrade();

            dock.set_center(
                DockItem::tabs(self.center_panels(), &weak_dock, window, cx)
                    .active_index(Self::center_index(main_route)),
                window,
                cx,
            );
            dock.set_right_dock(
                DockItem::tabs(self.database_panels(), &weak_dock, window, cx)
                    .active_index(Self::database_index(selected_route)),
                Some(px(320.0)),
                true,
                window,
                cx,
            );
            dock.set_toggle_button_visible(false, cx);
        });
    }

    fn center_panels(&self) -> Vec<Arc<dyn PanelView>> {
        vec![
            self.panel(Route::Dashboard),
            self.panel(Route::Terminal),
            self.panel(Route::Git),
            self.panel(Route::Ssh),
        ]
    }

    fn database_panels(&self) -> Vec<Arc<dyn PanelView>> {
        vec![
            self.panel(Route::mysql()),
            self.panel(Route::postgres()),
            self.panel(Route::redis()),
            self.panel(Route::sqlite()),
        ]
    }

    fn panel(&self, route: Route) -> Arc<dyn PanelView> {
        self.panels
            .get(&route)
            .cloned()
            .expect("panel route must exist")
    }

    fn center_index(route: Route) -> usize {
        match route {
            Route::Dashboard => 0,
            Route::Terminal => 1,
            Route::Git => 2,
            Route::Ssh => 3,
            _ => 0,
        }
    }

    fn database_index(route: Route) -> usize {
        match route {
            Route::Database(kind) => kind.index(),
            _ => 0,
        }
    }
}

pub type ActivationHandler = Rc<dyn Fn(Route, &mut Window, &mut App)>;

struct WorkbenchPanel {
    route: Route,
    focus_handle: FocusHandle,
    on_activated: ActivationHandler,
}

impl WorkbenchPanel {
    fn new(route: Route, on_activated: ActivationHandler, cx: &mut App) -> Self {
        Self {
            route,
            focus_handle: cx.focus_handle(),
            on_activated,
        }
    }
}

impl Panel for WorkbenchPanel {
    fn panel_name(&self) -> &'static str {
        self.route.panel_name()
    }

    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(self.route.label())
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(route_icon(self.route).size(px(14.0)))
            .child(self.route.label())
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> Option<PanelControl> {
        None
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if active {
            (self.on_activated)(self.route, window, cx);
        }
    }
}

impl EventEmitter<PanelEvent> for WorkbenchPanel {}

impl Focusable for WorkbenchPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkbenchPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        match self.route {
            Route::Welcome => unreachable!("welcome is not a dock panel"),
            Route::Dashboard => DashboardView::new(ShellSnapshot::load()).into_any_element(),
            Route::Terminal => unreachable!("terminal uses a dedicated dock panel"),
            Route::Git => GitView::new().into_any_element(),
            Route::Ssh => SshView::new().into_any_element(),
            Route::Database(kind) => DatabaseView::new(kind).into_any_element(),
        }
    }
}

pub fn route_icon(route: Route) -> UiIcon {
    match route {
        Route::Welcome | Route::Dashboard => UiIcon::new(IconName::LayoutDashboard),
        Route::Terminal => UiIcon::new(IconName::SquareTerminal),
        Route::Git => asset_icon("icons/git-branch.svg"),
        Route::Ssh => asset_icon("icons/server.svg"),
        Route::Database(_) => asset_icon("icons/database.svg"),
    }
}

fn asset_icon(path: &'static str) -> UiIcon {
    UiIcon::empty().path(path)
}
