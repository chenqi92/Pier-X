use gpui::{div, prelude::*, IntoElement, SharedString, Window};

use crate::components::{text, Card, SectionLabel};
use crate::data::ShellSnapshot;
use crate::theme::{
    spacing::{SP_2, SP_4, SP_6},
    theme,
};

#[derive(IntoElement)]
pub struct WorkbenchView {
    snapshot: ShellSnapshot,
}

impl WorkbenchView {
    pub fn new(snapshot: ShellSnapshot) -> Self {
        Self { snapshot }
    }
}

impl RenderOnce for WorkbenchView {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let s = self.snapshot;
        div()
            .size_full()
            .bg(t.color.bg_canvas)
            .text_color(t.color.text_primary)
            .font_family(t.font_ui.clone())
            .p(SP_6)
            .gap(SP_4)
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(SP_2)
                    .child(text::h2("Pier-X workspace"))
                    .child(text::body("Native Rust shell with direct pier-core integration.").secondary()),
            )
            .child(metric_card("Core", s.core_version.clone(), s.workspace_path.clone()))
            .child(metric_card(
                "Git Workspace",
                s.git_branch.clone(),
                join_lines(&s.repo_root, &s.git_detail),
            ))
            .child(metric_card(
                "Connections",
                s.connections_value.clone(),
                s.connections_detail.clone(),
            ))
            .child(metric_card(
                "Local Machine",
                s.local_machine_value.clone(),
                s.local_machine_detail.clone(),
            ))
            .child(metric_card(
                "App Paths",
                s.path_value.clone(),
                s.path_detail.clone(),
            ))
            .child(
                Card::new()
                    .child(SectionLabel::new("Next slices"))
                    .child(text::body("1. Replace this dashboard with a dock/workbench layout."))
                    .child(text::body("2. Wire terminal sessions directly from pier-core without IPC."))
                    .child(text::body("3. Migrate Git, SSH, and data panels as native GPUI views.")),
            )
    }
}

fn metric_card(title: impl Into<SharedString>, value: SharedString, detail: SharedString) -> Card {
    Card::new()
        .child(SectionLabel::new(title))
        .child(text::h2(value))
        .child(text::body(detail).secondary())
}

fn join_lines(a: &SharedString, b: &SharedString) -> SharedString {
    format!("repo: {a}\n{b}").into()
}
