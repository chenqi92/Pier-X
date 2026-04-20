use gpui::{div, prelude::*, px, AnyElement, App, IntoElement, ParentElement, Pixels, Window};

use crate::theme::{
    radius::RADIUS_PILL,
    spacing::{SP_1, SP_2, SP_3},
    theme,
};

// Accent-rule width for blockquotes — SKILL.md §6 ruler weight, not a
// generic border. Kept as a local literal because it's the SKILL.md
// token for this specific visual (per CLAUDE.md §Rule 1).
const QUOTE_RULE_W: Pixels = px(3.0);

/// Accent-ruled vertical rail + content column.
///
/// Children are rendered as a stacked column with `SP_2` gaps — the
/// same rhythm as paragraph blocks. Use `ParentElement`:
///
/// ```ignore
/// MarkdownBlockquote::new().child(text::body(line).into_any_element())
/// ```
#[derive(IntoElement, Default)]
pub struct MarkdownBlockquote {
    children: Vec<AnyElement>,
}

impl MarkdownBlockquote {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ParentElement for MarkdownBlockquote {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for MarkdownBlockquote {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        div()
            .w_full()
            .flex()
            .flex_row()
            .gap(SP_3)
            .child(
                div()
                    .w(QUOTE_RULE_W)
                    .h_full()
                    .bg(t.color.accent_muted)
                    .rounded(RADIUS_PILL),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .px(SP_1)
                    .flex()
                    .flex_col()
                    .gap(SP_2)
                    .children(self.children),
            )
    }
}
