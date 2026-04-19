//! App status rail, living at the bottom.
//!
//! The previous statusbar crammed three `StatusPill`s in here (terminal
//! count, right-panel mode, theme) plus a saved-connections caption.
//! Per the plan, **pills are reserved for dynamic state** — the theme
//! is not a status, the right-panel mode is already labelled in its
//! own PageHeader, and connection count is a static fact that has a
//! better home in the left panel's Servers group header.
//!
//! What remains is strictly ambient:
//! - **Leading** — terminal activity caption (`3/5`, `No terminal`).
//!   Upgraded to a warning pill only when there are no active tabs so
//!   the strip stays visually quiet 99 % of the time.
//! - **Trailing** — app version (mono caption).
//!
//! Call sites with something genuinely urgent (a persistent connect
//! error, say) should add their own conditional pill via `children()`
//! in a follow-up — the rail has room.

use gpui::{div, prelude::*, IntoElement};
use rust_i18n::t;

use crate::app::PierApp;
use crate::components::{text, StatusKind, StatusPill};
use crate::theme::{
    heights::STATUSBAR_H,
    spacing::{SP_2, SP_3},
    theme,
};

pub fn render(app: &PierApp, cx: &gpui::App) -> impl IntoElement {
    let t = theme(cx);
    let term_count = app.terminals_len();

    let version_label = format!("gpui {}", env!("CARGO_PKG_VERSION"));

    let mut row = div()
        .h(STATUSBAR_H)
        .px(SP_3)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .bg(t.color.bg_panel)
        .border_t_1()
        .border_color(t.color.border_subtle);

    // Terminal indicator: pill when empty (warn state), plain caption
    // otherwise so the rail stays quiet.
    if term_count == 0 {
        row = row.child(StatusPill::new(
            t!("App.Shell.no_terminal").to_string(),
            StatusKind::Warning,
        ));
    } else {
        let label = match app.active_terminal() {
            Some(i) if i < term_count => t!(
                "App.Shell.terminal_position",
                current = i + 1,
                total = term_count
            )
            .to_string(),
            _ => t!("App.Shell.no_active_tab").to_string(),
        };
        row = row.child(text::caption(label).secondary());
    }

    row.child(div().flex_1())
        .child(text::caption(version_label).secondary())
}
