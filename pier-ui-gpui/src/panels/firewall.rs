// Firewall panel — rule listing for the selected host.
//
// STUB: render a styled placeholder. Implement a rule table via pier-core's
// firewall service. See the per-panel prompt for the contract.

use gpui::prelude::*;
use gpui::{Context, Window};
use gpui_component::v_flex;

use crate::theme::Theme;
use crate::ui;

pub struct FirewallPanel {
    #[allow(dead_code)]
    theme: Theme,
}

impl FirewallPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
        }
    }
}

impl Render for FirewallPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .size_full()
            .child(ui::panel_header(t, "shield", "FIREWALL", ""))
            .child(ui::empty_state(t, "Firewall panel — not yet implemented"))
    }
}
