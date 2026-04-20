use std::rc::Rc;

use gpui::{div, prelude::*, px, App, ClickEvent, IntoElement, SharedString, Window};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Icon as UiIcon, IconName};
use pier_core::ssh::SshConfig;
use rust_i18n::t;

use crate::components::{
    text, Button, ButtonSize, Card, IconBadge, MetaLine, SectionLabel, StatusKind, StatusPill,
};
use crate::data::ShellSnapshot;
use crate::theme::{
    heights::ICON_SM,
    radius::RADIUS_SM,
    spacing::{SP_0_5, SP_1, SP_1_5, SP_2, SP_3, SP_4, SP_6},
    theme,
    typography::WEIGHT_REGULAR,
    ui_font_with, ThemeMode,
};

pub type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
pub type OnSelectRecent = Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>;

const WELCOME_DECK_MAX_W: gpui::Pixels = px(820.0);
const WELCOME_ACTION_BUTTON_W: gpui::Pixels = px(168.0);

#[derive(IntoElement)]
pub struct WelcomeView {
    connections: Vec<SshConfig>,
    on_new_ssh: OnClick,
    on_open_terminal: OnClick,
    on_open_recent: OnSelectRecent,
}

impl WelcomeView {
    pub fn new(
        connections: Vec<SshConfig>,
        on_new_ssh: OnClick,
        on_open_terminal: OnClick,
        on_open_recent: OnSelectRecent,
    ) -> Self {
        Self {
            connections,
            on_new_ssh,
            on_open_terminal,
            on_open_recent,
        }
    }
}

impl RenderOnce for WelcomeView {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx).clone();
        let count = self.connections.len();
        let mode_label: SharedString = if t.mode == ThemeMode::Dark {
            t!("App.Welcome.mode_dark").into()
        } else {
            t!("App.Welcome.mode_light").into()
        };
        let snapshot = ShellSnapshot::load();
        let WelcomeView {
            connections,
            on_new_ssh,
            on_open_terminal,
            on_open_recent,
        } = self;

        let deck = div()
            .w_full()
            .max_w(WELCOME_DECK_MAX_W)
            .px(SP_4)
            .pt(SP_6)
            .pb(SP_6)
            .flex()
            .flex_col()
            .gap(SP_3)
            .child(render_header_card(
                snapshot.workspace_path,
                count,
                mode_label,
                on_new_ssh,
                on_open_terminal,
            ))
            .child(render_recent_card(&t, &connections, on_open_recent));

        div()
            .size_full()
            .bg(t.color.bg_canvas)
            .text_color(t.color.text_primary)
            .font(ui_font_with(
                &t.font_ui,
                &t.font_ui_features,
                WEIGHT_REGULAR,
            ))
            .overflow_y_scrollbar()
            .child(div().w_full().flex().justify_center().child(deck))
    }
}

fn render_header_card(
    workspace_path: SharedString,
    connection_count: usize,
    mode_label: SharedString,
    on_new_ssh: OnClick,
    on_open_terminal: OnClick,
) -> Card {
    let connection_status = if connection_count > 0 {
        StatusKind::Success
    } else {
        StatusKind::Warning
    };

    Card::new()
        .padding(SP_3)
        .gap(SP_3)
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(SP_3)
                .child(
                    div()
                        .min_w(px(260.0))
                        .flex_1()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(SP_3)
                        .child(IconBadge::accent())
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .flex()
                                .flex_col()
                                .gap(SP_1)
                                .child(text::h2(t!("App.Welcome.title")).truncate())
                                .child(
                                    MetaLine::new(workspace_path)
                                        .with_icon(IconName::FolderFill)
                                        .tertiary(),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .justify_end()
                        .gap(SP_2)
                        .child(
                            Button::primary("welcome-new-ssh", t!("App.Welcome.Actions.new_ssh"))
                                .size(ButtonSize::Md)
                                .width(WELCOME_ACTION_BUTTON_W)
                                .leading_icon(IconName::Network)
                                .on_click(move |ev, win, app| on_new_ssh(ev, win, app)),
                        )
                        .child(
                            Button::secondary(
                                "welcome-local-term",
                                t!("App.Welcome.Actions.open_local_terminal"),
                            )
                            .size(ButtonSize::Md)
                            .width(WELCOME_ACTION_BUTTON_W)
                            .leading_icon(IconName::SquareTerminal)
                            .on_click(move |ev, win, app| on_open_terminal(ev, win, app)),
                        ),
                ),
        )
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(SP_1_5)
                .child(StatusPill::new(
                    t!("App.Welcome.Recent.saved_count", count = connection_count),
                    connection_status,
                ))
                .child(StatusPill::new(mode_label, StatusKind::Info))
                .child(StatusPill::new(
                    format!("gpui {}", env!("CARGO_PKG_VERSION")),
                    StatusKind::Success,
                ))
                .child(StatusPill::new(
                    format!("core {}", pier_core::VERSION),
                    StatusKind::Success,
                )),
        )
}

fn render_recent_card(
    t: &crate::theme::Theme,
    connections: &[SshConfig],
    on_open_recent: OnSelectRecent,
) -> Card {
    let count = connections.len();
    let header = div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(SP_2)
        .child(SectionLabel::new(t!("App.Welcome.Recent.title")).with_icon(IconName::Network))
        .child(text::caption(t!("App.Welcome.Recent.saved_count", count = count)).secondary());

    let card = Card::new().padding(SP_3).gap(SP_3).child(header);
    if connections.is_empty() {
        return card.child(text::body(t!("App.Welcome.Recent.empty")).secondary());
    }

    let mut list = div().w_full().flex().flex_col().gap(SP_2);
    for (idx, conn) in connections.iter().take(6).enumerate() {
        list = list.child(connection_row(t, idx, conn, on_open_recent.clone()));
    }

    card.child(list)
}

fn connection_row(
    t: &crate::theme::Theme,
    idx: usize,
    conn: &SshConfig,
    on_open_recent: OnSelectRecent,
) -> impl IntoElement {
    let host_line: SharedString = format!("{}@{}:{}", conn.user, conn.host, conn.port).into();
    let name: SharedString = conn.name.clone().into();
    let id: SharedString = format!("welcome-conn-{idx}").into();
    div()
        .id(gpui::ElementId::Name(id))
        .w_full()
        .min_h(px(52.0))
        .px(SP_3)
        .py(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_3)
        .rounded(RADIUS_SM)
        .bg(t.color.bg_panel)
        .border_1()
        .border_color(t.color.border_subtle)
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover).border_color(t.color.border_default))
        .on_click(move |_, w, app| on_open_recent(&idx, w, app))
        .child(
            div()
                .w(px(22.0))
                .h(px(22.0))
                .rounded(px(4.0))
                .bg(t.color.accent_subtle)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    UiIcon::new(IconName::Network)
                        .size(ICON_SM)
                        .text_color(t.color.accent),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(SP_0_5)
                .child(text::ui_label(name).truncate())
                .child(text::mono(host_line).secondary().truncate()),
        )
}
