// Logs panel — log file/stream viewer.
//
// STUB: render a styled placeholder. Implement a tailing log viewer (level
// colouring, follow toggle) via pier-core's logging/log services. See the
// per-panel prompt for the contract.

use gpui::prelude::*;
use gpui::{Context, Window};
use gpui_component::v_flex;

use crate::theme::Theme;
use crate::ui;

pub struct LogsPanel {
    #[allow(dead_code)]
    theme: Theme,
}

impl LogsPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
        }
    }
}

impl Render for LogsPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .size_full()
            .child(ui::panel_header(t, "scroll-text", "LOGS", ""))
            .child(ui::empty_state(t, "Logs panel — not yet implemented"))
    }
}
