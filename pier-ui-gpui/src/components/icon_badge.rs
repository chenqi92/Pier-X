use gpui::{div, prelude::*, px, IntoElement, Window};

use crate::theme::{radius::RADIUS_MD, theme};

/// 28×28 圆角矩形 + 中央 8×8 蓝点。Welcome 视图的品牌徽章。
/// 对照 docs/legacy-qml-reference/shell/WelcomeView.qml line 40-54。
#[derive(IntoElement)]
pub struct IconBadge;

impl IconBadge {
    pub fn accent() -> Self {
        Self
    }
}

impl RenderOnce for IconBadge {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        div()
            .w(px(28.0))
            .h(px(28.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_MD)
            .bg(t.color.accent_subtle)
            .child(
                div()
                    .w(px(8.0))
                    .h(px(8.0))
                    .rounded(px(4.0))
                    .bg(t.color.accent),
            )
    }
}
