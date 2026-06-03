// Web server panel — nginx/apache/caddy site + status overview.
//
// STUB: render a styled placeholder. Implement via pier-core's web_server /
// nginx / apache / caddy services. See the per-panel prompt for the contract.

use gpui::prelude::*;
use gpui::{Context, Window};
use gpui_component::v_flex;

use crate::theme::Theme;
use crate::ui;

pub struct WebserverPanel {
    #[allow(dead_code)]
    theme: Theme,
}

impl WebserverPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
        }
    }
}

impl Render for WebserverPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .size_full()
            .child(ui::panel_header(t, "server", "WEBSERVER", ""))
            .child(ui::empty_state(t, "Web server panel — not yet implemented"))
    }
}
