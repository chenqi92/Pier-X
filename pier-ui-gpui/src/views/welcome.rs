use std::rc::Rc;

use gpui::{div, prelude::*, px, App, ClickEvent, IntoElement, SharedString, Window};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Icon as UiIcon, IconName};
use pier_core::ssh::SshConfig;
use rust_i18n::t;

use crate::components::{
    text, Button, ButtonSize, Card, IconBadge, MetaLine, SectionLabel, Separator, StatusKind,
    StatusPill,
};
use crate::data::ShellSnapshot;
use crate::theme::{
    heights::ICON_SM,
    radius::RADIUS_SM,
    spacing::{SP_0_5, SP_1_5, SP_2, SP_3, SP_4, SP_8},
    theme,
    typography::WEIGHT_REGULAR,
    ui_font_with, ThemeMode,
};

pub type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
pub type OnSelectRecent = Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>;

/// Welcome page content measure. The center pane can grow wide, but the
/// onboarding surface should still read as a deliberate workbench deck,
/// not a marketing hero stretched edge-to-edge.
const WELCOME_DECK_MAX_W: gpui::Pixels = px(860.0);

/// Primary-action buttons are intentionally wider than a default dialog
/// button so the launch strip reads like the main command surface of the
/// center pane.
const WELCOME_ACTION_BUTTON_W: gpui::Pixels = px(184.0);

/// Welcome / cover view — re-framed as a workbench deck that visually
/// connects the center pane to the surrounding left / right panels.
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
            .pt(SP_8)
            .pb(SP_8)
            .flex()
            .flex_col()
            .gap(SP_4)
            .child(render_hero_card(
                snapshot.workspace_path,
                count,
                mode_label,
                on_new_ssh,
                on_open_terminal,
            ))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_start()
                    .gap(SP_4)
                    .child(div().min_w(px(280.0)).flex_1().child(render_recent_card(
                        &t,
                        &connections,
                        on_open_recent,
                    )))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(272.0))
                            .child(render_shell_layout_card(&t)),
                    ),
            );

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

fn render_hero_card(
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
        .padding(SP_4)
        .gap(SP_4)
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_start()
                .justify_between()
                .gap(SP_4)
                .child(
                    div()
                        .min_w(px(280.0))
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(SP_2)
                        .child(
                            SectionLabel::new(t!("App.Welcome.section"))
                                .with_icon(IconName::SquareTerminal),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(SP_3)
                                .child(IconBadge::accent())
                                .child(text::h1(t!("App.Welcome.title"))),
                        )
                        .child(text::body(t!("App.Welcome.subtitle")).secondary())
                        .child(MetaLine::new(workspace_path).with_icon(IconName::FolderFill)),
                )
                .child(
                    div()
                        .min_w(px(200.0))
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(SP_2)
                        .child(
                            SectionLabel::new(t!("App.Welcome.quick_status"))
                                .with_icon(IconName::ChartPie),
                        )
                        .child(
                            div()
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
                        ),
                ),
        )
        .child(Separator::horizontal())
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
                        .flex()
                        .flex_row()
                        .flex_wrap()
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
                )
                .child(
                    div()
                        .min_w(px(200.0))
                        .max_w(px(300.0))
                        .child(text::caption(t!("App.Welcome.launch_hint")).secondary()),
                ),
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
    for (idx, conn) in connections.iter().take(5).enumerate() {
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

fn render_shell_layout_card(t: &crate::theme::Theme) -> Card {
    Card::new()
        .padding(SP_3)
        .gap(SP_3)
        .child(SectionLabel::new(t!("App.Welcome.Layout.title")).with_icon(IconName::Inspector))
        .child(text::caption(t!("App.Welcome.Layout.hint")).secondary())
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(SP_2)
                .child(render_shell_lane(
                    t,
                    IconName::FolderFill,
                    t!("App.Welcome.Layout.left_title"),
                    t!("App.Welcome.Layout.left_caption"),
                    false,
                ))
                .child(render_shell_lane(
                    t,
                    IconName::SquareTerminal,
                    t!("App.Welcome.Layout.center_title"),
                    t!("App.Welcome.Layout.center_caption"),
                    true,
                ))
                .child(render_shell_lane(
                    t,
                    IconName::Inspector,
                    t!("App.Welcome.Layout.right_title"),
                    t!("App.Welcome.Layout.right_caption"),
                    false,
                )),
        )
}

fn render_shell_lane(
    t: &crate::theme::Theme,
    icon: IconName,
    title: impl Into<SharedString>,
    caption: impl Into<SharedString>,
    emphasized: bool,
) -> impl IntoElement {
    let title: SharedString = title.into();
    let caption: SharedString = caption.into();
    let (bg, border, icon_bg, icon_fg) = if emphasized {
        (
            t.color.accent_subtle,
            t.color.accent_muted,
            t.color.accent,
            t.color.text_inverse,
        )
    } else {
        (
            t.color.bg_panel,
            t.color.border_subtle,
            t.color.bg_surface,
            t.color.accent,
        )
    };

    div()
        .w_full()
        .p(SP_3)
        .flex()
        .flex_row()
        .items_start()
        .gap(SP_2)
        .rounded(RADIUS_SM)
        .bg(bg)
        .border_1()
        .border_color(border)
        .child(
            div()
                .w(px(22.0))
                .h(px(22.0))
                .flex_none()
                .rounded(RADIUS_SM)
                .bg(icon_bg)
                .flex()
                .items_center()
                .justify_center()
                .child(UiIcon::new(icon).size(ICON_SM).text_color(icon_fg)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(SP_1_5)
                .child(text::ui_label(title).truncate())
                .child(text::caption(caption).secondary()),
        )
}
