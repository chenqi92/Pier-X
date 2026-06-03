// Docker panel — container list + lifecycle controls.
//
// STUB: render a styled placeholder. Implement by listing containers via
// pier-core's docker service and showing name/status/image/ports rows. See the
// per-panel prompt for the contract and pier-core entry points.

use gpui::prelude::*;
use gpui::{Context, Window};
use gpui_component::v_flex;

use crate::theme::Theme;
use crate::ui;

pub struct DockerPanel {
    #[allow(dead_code)]
    theme: Theme,
}

impl DockerPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
        }
    }
}

impl Render for DockerPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .size_full()
            .child(ui::panel_header(t, "container", "DOCKER", ""))
            .child(ui::empty_state(t, "Docker panel — not yet implemented"))
    }
}
