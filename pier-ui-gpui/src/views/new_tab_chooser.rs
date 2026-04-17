//! "New tab" chooser dialog — invoked from the toolbar / tab-bar `+` button.
//!
//! Mirrors `Pier/PierApp/Sources/Views/MainWindow/MainView.swift`'s
//! `NewTabChooserView` sheet: pick a Local terminal or one of the saved SSH
//! connections, click → tab opens + sheet dismisses.
//!
//! Phase 4 scope:
//!   - Local terminal row (always present)
//!   - Saved SSH list (clickable; reuses [`PierApp::open_ssh_terminal`])
//!   - ESC / overlay click dismisses (gpui-component default behaviour)
//!
//! Deferred:
//!   - Inline new-connection editor inside the dialog
//!   - Group-aware rendering (when ServerGroup support lands)

use gpui::{div, prelude::*, px, App, ClickEvent, IntoElement, SharedString, WeakEntity, Window};
use gpui_component::{Icon as UiIcon, IconName, WindowExt as _};
use pier_core::ssh::SshConfig;

use crate::app::PierApp;
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_2, SP_3},
    theme,
    typography::{SIZE_BODY, SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};

/// Open the chooser as a modal dialog on the given window.
///
/// `connections` is captured by clone so the dialog body doesn't have to
/// re-read [`pier_core::connections::ConnectionStore`] every render.
pub fn open(
    window: &mut Window,
    cx: &mut App,
    app: WeakEntity<PierApp>,
    connections: Vec<SshConfig>,
) {
    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = build_body(app_cx, app.clone(), &connections);
        dialog
            .title("New tab")
            .w(px(440.0))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(body)
    });
}

fn build_body(
    cx: &App,
    app: WeakEntity<PierApp>,
    connections: &[SshConfig],
) -> impl IntoElement {
    let t = theme(cx).clone();

    let mut col = div().flex().flex_col().gap(SP_1);

    // ── Local terminal row ──
    col = col.child(local_row(&t, app.clone()));

    if !connections.is_empty() {
        col = col.child(
            div()
                .px(SP_3)
                .pt(SP_2)
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_tertiary)
                .child("Saved SSH connections"),
        );
        for (idx, conn) in connections.iter().enumerate() {
            col = col.child(ssh_row(&t, app.clone(), idx, conn));
        }
    } else {
        col = col.child(
            div()
                .px(SP_3)
                .py(SP_2)
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(
                    "No saved SSH connections yet — add one to ~/.config/pier-x/connections.json.",
                ),
        );
    }
    col
}

fn local_row(t: &crate::theme::Theme, app: WeakEntity<PierApp>) -> impl IntoElement {
    let on_click = move |_: &ClickEvent, w: &mut Window, app_cx: &mut App| {
        let _ = app.update(app_cx, |this, cx| this.open_terminal_tab(cx));
        w.close_dialog(app_cx);
    };
    row_shell(t, "nt-local", on_click)
        .child(icon_cell(t, IconName::SquareTerminal))
        .child(label_cell(t, "Local terminal", default_shell_hint()))
}

fn ssh_row(
    t: &crate::theme::Theme,
    app: WeakEntity<PierApp>,
    idx: usize,
    conn: &SshConfig,
) -> impl IntoElement {
    let id_str: SharedString = format!("nt-ssh-{idx}").into();
    let on_click = move |_: &ClickEvent, w: &mut Window, app_cx: &mut App| {
        let _ = app.update(app_cx, |this, cx| this.open_ssh_terminal(idx, cx));
        w.close_dialog(app_cx);
    };
    let name: SharedString = conn.name.clone().into();
    let address: SharedString = format!("{}@{}:{}", conn.user, conn.host, conn.port).into();

    row_shell(t, id_str, on_click)
        .child(icon_cell(t, IconName::Globe))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .text_size(SIZE_BODY)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_primary)
                        .child(name),
                )
                .child(
                    div()
                        .text_size(SIZE_MONO_SMALL)
                        .font_family(t.font_mono.clone())
                        .text_color(t.color.text_secondary)
                        .child(address),
                ),
        )
}

fn row_shell(
    t: &crate::theme::Theme,
    id: impl Into<gpui::ElementId>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .min_h(px(40.0))
        .px(SP_3)
        .py(SP_1)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .rounded(RADIUS_SM)
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(on_click)
}

fn icon_cell(t: &crate::theme::Theme, name: IconName) -> impl IntoElement {
    div()
        .w(px(24.0))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .bg(t.color.accent_subtle)
        .text_color(t.color.accent)
        .child(UiIcon::new(name).size(px(14.0)))
}

fn label_cell(
    t: &crate::theme::Theme,
    primary: &'static str,
    secondary: SharedString,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .text_size(SIZE_BODY)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child(primary),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(secondary),
        )
}

fn default_shell_hint() -> SharedString {
    std::env::var("SHELL")
        .map(|s| {
            std::path::Path::new(&s)
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or(&s)
                .to_string()
                .into()
        })
        .unwrap_or_else(|_| "system shell".into())
}
