// Markdown panel — rendered preview of the active markdown file.
//
// STUB: render a styled placeholder. Implement a rendered view via pier-core's
// markdown service (parse to blocks, paint headings/lists/code). See the
// per-panel prompt for the contract.

use gpui::prelude::*;
use gpui::{Context, Window};
use gpui_component::v_flex;

use crate::theme::Theme;
use crate::ui;

pub struct MarkdownPanel {
    #[allow(dead_code)]
    theme: Theme,
}

impl MarkdownPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
        }
    }
}

impl Render for MarkdownPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .size_full()
            .child(ui::panel_header(t, "file-text", "MARKDOWN", ""))
            .child(ui::empty_state(t, "Markdown panel — not yet implemented"))
    }
}
