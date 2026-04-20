use gpui::{
    div, prelude::*, relative, App, ClipboardItem, ElementId, IntoElement, SharedString, Window,
};
use gpui_component::{scroll::ScrollableElement, IconName};
use rust_i18n::t;

use crate::components::{text, Button, ButtonSize};
use crate::theme::{
    heights::ROW_SM_H,
    radius::RADIUS_MD,
    spacing::{SP_1, SP_2, SP_3},
    theme,
    typography::{SIZE_MONO_SMALL, WEIGHT_REGULAR},
};

/// Fenced-code block with a compact language tag and a copy button.
///
/// `index` seeds the scroll-region / copy-button element ids — callers
/// iterate blocks with `enumerate()` and feed the position in. Blocks
/// are cached above (see `views/markdown.rs::load_markdown_document`),
/// so the index is stable across renders for the same document.
#[derive(IntoElement)]
pub struct MarkdownCodeBlock {
    index: usize,
    language: Option<SharedString>,
    code: SharedString,
}

impl MarkdownCodeBlock {
    pub fn new(index: usize, code: impl Into<SharedString>) -> Self {
        Self {
            index,
            language: None,
            code: code.into(),
        }
    }

    pub fn language(mut self, lang: Option<SharedString>) -> Self {
        self.language = lang;
        self
    }
}

impl RenderOnce for MarkdownCodeBlock {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let lang_label: SharedString = self
            .language
            .clone()
            .unwrap_or_else(|| SharedString::from("code"));
        let code_for_copy = self.code.to_string();

        let lines: Vec<SharedString> = if self.code.is_empty() {
            vec![" ".into()]
        } else {
            self.code
                .split('\n')
                .map(|line| {
                    if line.is_empty() {
                        " ".into()
                    } else {
                        line.to_string().into()
                    }
                })
                .collect()
        };

        let scroll_id: ElementId = ("markdown-code-scroll", self.index).into();
        let copy_id: ElementId = ("markdown-code-copy", self.index).into();

        div()
            .w_full()
            .flex()
            .flex_col()
            .bg(t.color.bg_surface)
            .border_1()
            .border_color(t.color.border_default)
            .rounded(RADIUS_MD)
            .overflow_hidden()
            .child(
                div()
                    .h(ROW_SM_H)
                    .px(SP_3)
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(SP_2)
                    .bg(t.color.bg_hover)
                    .border_b_1()
                    .border_color(t.color.border_subtle)
                    .child(text::small(lang_label).secondary())
                    .child(
                        Button::secondary(copy_id, t!("App.Markdown.copy"))
                            .size(ButtonSize::Sm)
                            .leading_icon(IconName::Copy)
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    code_for_copy.clone(),
                                ));
                            }),
                    ),
            )
            .child(
                div().id(scroll_id).w_full().overflow_x_scrollbar().child(
                    div()
                        .min_w_full()
                        .px(SP_3)
                        .py(SP_3)
                        .flex()
                        .flex_col()
                        .gap(SP_1)
                        .children(lines.into_iter().map(|line| {
                            div()
                                .whitespace_nowrap()
                                .text_size(SIZE_MONO_SMALL)
                                .line_height(relative(1.5))
                                .text_color(t.color.text_primary)
                                .font_family(t.font_mono.clone())
                                .font_weight(WEIGHT_REGULAR)
                                .child(line)
                        })),
                ),
            )
    }
}
