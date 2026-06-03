// Database panel — shared by MySQL / PostgreSQL / Redis / SQLite tools.
//
// STUB: render a styled placeholder. Implement a schema/table tree plus a
// read-only result preview via pier-core's db services. Respect the product's
// read-only default. See the per-panel prompt for the contract.

use gpui::prelude::*;
use gpui::{Context, Window};
use gpui_component::v_flex;

use crate::theme::Theme;
use crate::ui;

pub struct DbPanel {
    #[allow(dead_code)]
    theme: Theme,
}

impl DbPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
        }
    }
}

impl Render for DbPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .size_full()
            .child(ui::panel_header(t, "database", "DATABASE", ""))
            .child(ui::empty_state(t, "Database panel — not yet implemented"))
    }
}
