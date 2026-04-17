//! Left panel — Files / Servers tab switcher.
//!
//! Mirrors `Pier/PierApp/Sources/Views/LeftPanel/LeftPanelView.swift`.
//! Phase 1 ships the tab switcher + Servers list (ported from the previous
//! full-page SshView). The Files tab is a placeholder until Phase 2 lands
//! the lazy-load file tree.

use std::rc::Rc;

use gpui::{div, prelude::*, px, App, IntoElement, SharedString, Window};
use gpui_component::Icon as UiIcon;
use pier_core::ssh::{AuthMethod, SshConfig};

use crate::app::layout::LeftTab;
use crate::components::{text, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_BODY, SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};

pub type TabSelector = Rc<dyn Fn(&LeftTab, &mut Window, &mut App) + 'static>;
pub type ServerSelector = Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct LeftPanel {
    active_tab: LeftTab,
    connections: Vec<SshConfig>,
    on_select_tab: TabSelector,
    on_select_server: ServerSelector,
}

impl LeftPanel {
    pub fn new(
        active_tab: LeftTab,
        connections: Vec<SshConfig>,
        on_select_tab: TabSelector,
        on_select_server: ServerSelector,
    ) -> Self {
        Self {
            active_tab,
            connections,
            on_select_tab,
            on_select_server,
        }
    }
}

impl RenderOnce for LeftPanel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let LeftPanel {
            active_tab,
            connections,
            on_select_tab,
            on_select_server,
        } = self;

        let body = match active_tab {
            LeftTab::Files => render_files_placeholder(t).into_any_element(),
            LeftTab::Servers => render_servers_list(t, &connections, on_select_server)
                .into_any_element(),
        };

        div()
            .h_full()
            .flex()
            .flex_col()
            .bg(t.color.bg_panel)
            .border_r_1()
            .border_color(t.color.border_subtle)
            .child(render_tab_bar(t, active_tab, on_select_tab))
            .child(
                div()
                    .h(px(1.0))
                    .w_full()
                    .bg(t.color.border_subtle),
            )
            .child(div().flex_1().min_h(px(0.0)).child(body))
    }
}

fn render_tab_bar(
    t: &crate::theme::Theme,
    active: LeftTab,
    on_select: TabSelector,
) -> impl IntoElement {
    let mut row = div()
        .h(px(32.0))
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1);

    for tab in LeftTab::ALL {
        let is_active = tab == active;
        let select = on_select.clone();
        let label = tab.label();
        let id_str: SharedString = format!("left-tab-{}", tab.id()).into();
        let icon = match tab {
            LeftTab::Files => UiIcon::new(icons::FILES),
            LeftTab::Servers => UiIcon::new(icons::SERVERS),
        };

        let mut btn = div()
            .id(gpui::ElementId::Name(id_str))
            .h(px(22.0))
            .px(SP_2)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1_5)
            .rounded(RADIUS_SM)
            .text_size(SIZE_CAPTION)
            .font_weight(WEIGHT_MEDIUM)
            .cursor_pointer()
            .text_color(if is_active {
                t.color.accent
            } else {
                t.color.text_secondary
            })
            .hover(|s| s.bg(t.color.bg_hover))
            .on_click(move |_, w, app| select(&tab, w, app))
            .child(icon.size(px(12.0)))
            .child(label);

        if is_active {
            btn = btn.bg(t.color.accent_subtle);
        }
        row = row.child(btn);
    }
    row.bg(t.color.bg_panel)
}

fn render_files_placeholder(t: &crate::theme::Theme) -> impl IntoElement {
    div()
        .p(SP_3)
        .flex()
        .flex_col()
        .gap(SP_2)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .child(SectionLabel::new("Local files"))
                .child(StatusPill::new("placeholder", StatusKind::Warning)),
        )
        .child(
            text::body(
                "Local file tree (lazy load + search + drag-drop) lands in Phase 2. \
                 Mirrors `Pier/PierApp/Sources/Views/FilePanel/LocalFileView.swift`.",
            )
            .secondary(),
        )
        .child(text::body("Tip: drop a file onto the terminal to insert its path.").secondary())
        .child(div().h(px(1.0)).w_full().bg(t.color.border_subtle))
        .child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child("Coming soon: search, filter, right-click, drag-and-drop"),
        )
}

fn render_servers_list(
    t: &crate::theme::Theme,
    connections: &[SshConfig],
    on_select: ServerSelector,
) -> impl IntoElement {
    let mut col = div().p(SP_2).flex().flex_col().gap(SP_1);

    let header = Card::new()
        .padding(SP_2)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .child(SectionLabel::new("Saved connections"))
                .child(StatusPill::new(
                    format!("{} entries", connections.len()),
                    if connections.is_empty() {
                        StatusKind::Warning
                    } else {
                        StatusKind::Success
                    },
                )),
        )
        .child(
            text::body("Click a connection to inspect; new-tab dialog wiring lands in Phase 2.")
                .secondary(),
        );
    col = col.child(header);

    if connections.is_empty() {
        col = col.child(
            Card::new()
                .padding(SP_2)
                .child(SectionLabel::new("No saved SSH connections"))
                .child(
                    text::body(
                        "Connections from `~/.config/pier-x/connections.json` will list here.",
                    )
                    .secondary(),
                ),
        );
        return col;
    }

    for (idx, conn) in connections.iter().enumerate() {
        col = col.child(server_row(t, idx, conn, on_select.clone()));
    }
    col
}

fn server_row(
    t: &crate::theme::Theme,
    idx: usize,
    conn: &SshConfig,
    on_select: ServerSelector,
) -> impl IntoElement {
    let address: SharedString = format!("{}@{}:{}", conn.user, conn.host, conn.port).into();
    let auth: SharedString = match &conn.auth {
        AuthMethod::Agent => "agent".into(),
        AuthMethod::PublicKeyFile { .. } => "key".into(),
        AuthMethod::KeychainPassword { .. } => "keychain".into(),
        AuthMethod::DirectPassword { .. } => "password".into(),
    };
    let name: SharedString = conn.name.clone().into();
    let id_str: SharedString = format!("left-server-{idx}").into();

    div()
        .id(gpui::ElementId::Name(id_str))
        .flex()
        .flex_col()
        .gap(SP_1)
        .px(SP_2)
        .py(SP_1_5)
        .rounded(RADIUS_SM)
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(move |_, w, app| on_select(&idx, w, app))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .child(
                    div()
                        .text_size(SIZE_BODY)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_primary)
                        .child(name),
                )
                .child(
                    div()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(auth),
                ),
        )
        .child(
            div()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_secondary)
                .child(address),
        )
}

/// Re-exported icon helpers used by both the left-panel tab bar and the
/// top toolbar in `app/state.rs`. Centralised so glyph swaps land in one
/// place.
pub mod icons {
    use gpui_component::IconName;

    pub const FILES: IconName = IconName::Folder;
    pub const SERVERS: IconName = IconName::Globe;
    pub const TOGGLE_LEFT_OPEN: IconName = IconName::PanelLeftClose;
    pub const TOGGLE_LEFT_CLOSED: IconName = IconName::PanelLeftOpen;
    pub const TOGGLE_RIGHT_OPEN: IconName = IconName::PanelRightClose;
    pub const TOGGLE_RIGHT_CLOSED: IconName = IconName::PanelRightOpen;
    pub const NEW_TAB: IconName = IconName::Plus;
    pub const SUN: IconName = IconName::Sun;
    pub const MOON: IconName = IconName::Moon;
}

