use std::env;

use gpui::{div, prelude::*, px, IntoElement, SharedString, Window};

use crate::components::{text, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    radius::RADIUS_MD,
    spacing::{SP_2, SP_3, SP_4},
    theme,
    typography::SIZE_MONO_CODE,
};

/// Terminal placeholder. PR6 wires the pier-core PTY backend
/// (pier-core/src/terminal/, see PierTerminal::with_pty) and ANSI
/// 16-color rendering. For now we surface the local shell context
/// so the route is actually informative.
#[derive(IntoElement)]
pub struct TerminalView;

impl TerminalView {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TerminalView {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for TerminalView {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let snapshot = LocalShellSnapshot::probe();

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(SP_4)
            .p(SP_4)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(SP_3)
                    .child(text::h2("Terminal"))
                    .child(StatusPill::new("PTY: idle", StatusKind::Warning)),
            )
            .child(
                Card::new()
                    .child(SectionLabel::new("Local shell"))
                    .child(text::body(snapshot.shell.clone())),
            )
            .child(
                Card::new()
                    .child(SectionLabel::new("Working directory"))
                    .child(text::mono(snapshot.cwd.clone())),
            )
            .child(
                Card::new()
                    .child(SectionLabel::new("PATH entries"))
                    .child(text::body(format!(
                        "{} entries on PATH",
                        snapshot.path_entries
                    ))),
            )
            // Mono surface — this is where the ANSI grid will render once
            // PierTerminal::snapshot() is wired in the next slice.
            .child(
                div()
                    .flex_1()
                    .min_h(px(200.0))
                    .p(SP_3)
                    .rounded(RADIUS_MD)
                    .bg(t.color.bg_panel)
                    .border_1()
                    .border_color(t.color.border_subtle)
                    .font_family(t.font_mono.clone())
                    .text_size(SIZE_MONO_CODE)
                    .text_color(t.color.text_secondary)
                    .flex()
                    .flex_col()
                    .gap(SP_2)
                    .child(div().child("$ pier terminal --help"))
                    .child(div().child("# session surface — pier-core PTY wiring lands in PR6.1"))
                    .child(div().child("# input routing, scrollback, ANSI 16-color palette pending")),
            )
    }
}

struct LocalShellSnapshot {
    shell: SharedString,
    cwd: SharedString,
    path_entries: usize,
}

impl LocalShellSnapshot {
    fn probe() -> Self {
        let shell = env::var("SHELL")
            .unwrap_or_else(|_| {
                if cfg!(target_os = "windows") {
                    "cmd.exe".into()
                } else {
                    "sh".into()
                }
            })
            .into();
        let cwd = env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "Unavailable".into())
            .into();
        let path_entries = env::var_os("PATH")
            .map(|p| env::split_paths(&p).count())
            .unwrap_or(0);
        Self {
            shell,
            cwd,
            path_entries,
        }
    }
}
