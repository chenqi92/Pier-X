use gpui::{div, prelude::*, px, IntoElement, SharedString, Window};
use pier_core::ssh::SshConfig;

use crate::app::{NewSshRequested, OpenLocalTerminalRequested};
use crate::components::{
    text, Button, Card, IconBadge, SectionLabel, StatusKind, StatusPill,
};
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3, SP_4},
    theme,
    typography::{SIZE_BODY, SIZE_SMALL, WEIGHT_MEDIUM},
    ThemeMode,
};

/// Welcome / cover view — 对照 docs/legacy-qml-reference/shell/WelcomeView.qml 像素移植。
#[derive(IntoElement)]
pub struct WelcomeView {
    connections: Vec<SshConfig>,
}

impl WelcomeView {
    pub fn new(connections: Vec<SshConfig>) -> Self {
        Self { connections }
    }
}

impl RenderOnce for WelcomeView {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let count = self.connections.len();
        let mode_label: SharedString = if t.mode == ThemeMode::Dark {
            "Dark mode".into()
        } else {
            "Light mode".into()
        };

        let column = div()
            .w(px(480.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(SP_4)
            // 1. 品牌徽章
            .child(IconBadge::accent())
            // 2. SectionLabel
            .child(SectionLabel::new("Welcome").centered())
            // 3. Display 标题（28px Inter 510，居中，对照 QML 行 62-70）
            .child(text::h1("Pier-X workspace").centered())
            // 4. Body 描述（13px secondary，居中，宽 420px）
            .child(
                div()
                    .w(px(420.0))
                    .child(text::body(
                        "Open a local terminal or connect to a server to start working.",
                    ).secondary().centered()),
            )
            // 5. 主操作按钮行（PrimaryButton + GhostButton 各 148px 宽）
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_center()
                    .gap(SP_1_5)
                    .child(
                        Button::primary("welcome-new-ssh", "New SSH connection")
                            .width(px(148.0))
                            .on_click(|_, _, cx| cx.dispatch_action(&NewSshRequested)),
                    )
                    .child(
                        Button::ghost("welcome-local-term", "Open local terminal")
                            .width(px(148.0))
                            .on_click(|_, _, cx| cx.dispatch_action(&OpenLocalTerminalRequested)),
                    ),
            )
            // 6. 状态药丸行
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_center()
                    .gap(SP_1_5)
                    .child(StatusPill::new(
                        format!("gpui {}", env!("CARGO_PKG_VERSION")),
                        StatusKind::Success,
                    ))
                    .child(StatusPill::new(
                        format!("core {}", pier_core::VERSION),
                        StatusKind::Success,
                    ))
                    .child(StatusPill::new(mode_label, StatusKind::Info)),
            );

        let column = if count > 0 {
            column.child(render_recent_card(t, &self.connections))
        } else {
            column
        };

        div()
            .size_full()
            .bg(t.color.bg_canvas)
            .text_color(t.color.text_primary)
            .font_family(t.font_ui.clone())
            .flex()
            .items_center()
            .justify_center()
            .child(column)
    }
}

fn render_recent_card(t: &crate::theme::Theme, connections: &[SshConfig]) -> Card {
    let count = connections.len();
    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .child(
            div()
                .text_size(SIZE_BODY)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child("Recent connections"),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(format!("{count} saved")),
        );

    let mut grid = div().flex().flex_row().flex_wrap().gap(SP_2);
    for conn in connections.iter().take(6) {
        grid = grid.child(connection_tile(t, conn));
    }

    Card::new()
        .padding(SP_3)
        .child(header)
        .child(grid)
}

fn connection_tile(t: &crate::theme::Theme, conn: &SshConfig) -> impl IntoElement {
    let host_line: SharedString =
        format!("{}@{}:{}", conn.user, conn.host, conn.port).into();
    let name: SharedString = conn.name.clone().into();
    div()
        .w(px(208.0))
        .h(px(54.0))
        .px(SP_3)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .rounded(RADIUS_SM)
        .bg(t.color.bg_panel)
        .child(
            div()
                .w(px(20.0))
                .h(px(20.0))
                .rounded(px(4.0))
                .bg(t.color.accent_subtle)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded(px(3.0))
                        .bg(t.color.accent),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SP_1)
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
                        .font_family(t.font_mono.clone())
                        .text_color(t.color.text_tertiary)
                        .child(host_line),
                ),
        )
}
