// Software panel — installed packages / package manager view.
//
// STUB: render a styled placeholder. Implement via pier-core's package_manager /
// package_mirror services. See the per-panel prompt for the contract.

use gpui::prelude::*;
use gpui::{Context, Window};
use gpui_component::v_flex;

use crate::theme::Theme;
use crate::ui;

pub struct SoftwarePanel {
    #[allow(dead_code)]
    theme: Theme,
}

impl SoftwarePanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
        }
    }
}

impl Render for SoftwarePanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .size_full()
            .child(ui::panel_header(t, "package", "SOFTWARE", ""))
            .child(ui::empty_state(t, "Software panel — not yet implemented"))
    }
}
