use gpui::{prelude::*, Context, IntoElement, Window};
use pier_core::connections::ConnectionStore;
use pier_core::ssh::SshConfig;

use crate::data::ShellSnapshot;
use crate::views::{welcome::WelcomeView, workbench::WorkbenchView};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Welcome,
    Workbench,
}

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
        let route = if connections.is_empty() {
            Route::Welcome
        } else {
            Route::Workbench
        };
        Self {
            route,
            snapshot: ShellSnapshot::load(),
            connections,
        }
    }
}

impl Default for PierApp {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for PierApp {
    fn render(&mut self, _win: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match self.route {
            Route::Welcome => WelcomeView::new(self.connections.clone()).into_any_element(),
            Route::Workbench => WorkbenchView::new(self.snapshot.clone()).into_any_element(),
        }
    }
}
