pub mod actions;
pub mod route;
pub mod state;

use std::rc::Rc;

use gpui::{App, Window};
use gpui_component::{Icon as UiIcon, IconName};

pub use actions::ToggleTheme;
pub use route::Route;
pub use state::PierApp;

/// Callback signature kept around so [`crate::views::terminal::TerminalPanel`]
/// retains its constructor shape from the dock-based draft. The canvas-driven
/// shell passes a no-op closure — the terminal panel doesn't need to notify
/// anyone when it becomes active because it's the only thing on the canvas
/// when [`Route::Terminal`] is selected.
pub type ActivationHandler = Rc<dyn Fn(Route, &mut Window, &mut App)>;

/// Resolve a sidebar / tab icon for a route.
///
/// `LayoutDashboard` and `SquareTerminal` are bundled with `gpui-component`
/// (lucide set); the rest come from the SVGs embedded by [`crate::assets`].
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
