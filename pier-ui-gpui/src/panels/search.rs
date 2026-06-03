// Search panel — code/content search results.
//
// STUB: render a styled placeholder. Implement a query input + grouped result
// list via pier-core's code_search/search service. See the per-panel prompt for
// the contract.

use gpui::prelude::*;
use gpui::{Context, Window};
use gpui_component::v_flex;

use crate::theme::Theme;
use crate::ui;

pub struct SearchPanel {
    #[allow(dead_code)]
    theme: Theme,
}

impl SearchPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
        }
    }
}

impl Render for SearchPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .size_full()
            .child(ui::panel_header(t, "search", "SEARCH", ""))
            .child(ui::empty_state(t, "Search panel — not yet implemented"))
    }
}
