// SFTP panel — remote file browser over an SSH session.
//
// STUB: render a styled placeholder. Implement a remote directory listing with
// navigation via pier-core's sftp/ssh layer for the selected connection. See
// the per-panel prompt for the contract.

use gpui::prelude::*;
use gpui::{Context, Window};
use gpui_component::v_flex;

use crate::theme::Theme;
use crate::ui;

pub struct SftpPanel {
    #[allow(dead_code)]
    theme: Theme,
}

impl SftpPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
        }
    }
}

impl Render for SftpPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .size_full()
            .child(ui::panel_header(t, "folder", "SFTP", ""))
            .child(ui::empty_state(t, "SFTP panel — not yet implemented"))
    }
}
