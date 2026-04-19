//! Markdown preview backed by GPUI Component's async `TextView`.
//!
//! This keeps resize / pane-drag interactions smooth: the Markdown AST is
//! cached and reparsed off the hot render path when the source changes,
//! instead of rebuilding every block on every repaint.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use gpui::{div, prelude::*, px, rems, App, ClipboardItem, IntoElement, SharedString, Window};
use gpui_component::{text::TextView, ActiveTheme as _, IconName};
use rust_i18n::t;

use crate::components::{text, Button, ButtonSize, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    heights::ROW_SM_H,
    radius::RADIUS_LG,
    spacing::{SP_2, SP_3, SP_4},
    theme,
};

/// Files larger than this are truncated with an explanatory banner — keeps
/// the synchronous read on the render path bounded.
const MAX_RENDER_BYTES: usize = 2 * 1024 * 1024;
const MARKDOWN_READER_MAX_W: gpui::Pixels = px(760.0);

#[derive(Clone)]
enum MarkdownDocument {
    Ready {
        bytes_len: usize,
        truncated: bool,
        source: SharedString,
    },
    Error(SharedString),
}

#[derive(IntoElement)]
pub struct MarkdownView {
    file_path: Option<PathBuf>,
}

impl MarkdownView {
    pub fn new(file_path: Option<PathBuf>) -> Self {
        Self { file_path }
    }
}

impl RenderOnce for MarkdownView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx).clone();
        let Some(path) = self.file_path else {
            return empty_state(&t).into_any_element();
        };

        let path_label: SharedString = path.display().to_string().into();
        let cache_key = markdown_cache_key(&path);
        let path_for_state = path.clone();
        let document = window.use_keyed_state(cache_key, cx, move |_, _| {
            load_markdown_document(path_for_state.as_path())
        });

        match document.read(cx).clone() {
            MarkdownDocument::Ready {
                bytes_len,
                truncated,
                source,
            } => markdown_document_view(&t, &path_label, bytes_len, truncated, source, window, cx)
                .into_any_element(),
            MarkdownDocument::Error(err) => div()
                .flex()
                .flex_col()
                .child(markdown_reader_shell(
                    &t,
                    file_header(&t, &path_label, 0, false, None),
                    div().p(SP_4).child(
                        Card::new()
                            .padding(SP_3)
                            .child(SectionLabel::new(t!("App.Markdown.cannot_read_file")))
                            .child(text::body(err).secondary()),
                    ),
                ))
                .into_any_element(),
        }
    }
}

fn markdown_document_view(
    t: &crate::theme::Theme,
    path_label: &SharedString,
    bytes_len: usize,
    truncated: bool,
    source: SharedString,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let mut body = div().w_full().px(SP_4).py(SP_4).flex().flex_col().gap(SP_4);
    if truncated {
        body = body.child(
            Card::new()
                .padding(SP_2)
                .child(SectionLabel::new(t!("App.Markdown.truncated")))
                .child(
                    text::body(t!(
                        "App.Markdown.truncated_message",
                        shown_kb = MAX_RENDER_BYTES / 1024,
                        total_kb = bytes_len / 1024
                    ))
                    .secondary(),
                ),
        );
    }

    if source.trim().is_empty() {
        body = body.child(text::body(t!("App.Markdown.empty_document")).secondary());
    } else {
        body = body.child(
            div().w_full().min_w(px(0.0)).child(
                TextView::markdown("markdown-preview", source.clone(), window, cx)
                    .style(markdown_text_style(t, cx))
                    .selectable(true)
                    .code_block_actions(|block, _, _cx| {
                        let code = block.code();
                        let copy_id: SharedString = format!(
                            "markdown-copy-{}",
                            stable_hash(&(block.lang(), code.clone()))
                        )
                        .into();
                        let mut actions = div().flex().flex_row().items_center().gap(SP_2);
                        if let Some(lang) = block.lang() {
                            actions = actions.child(StatusPill::new(lang, StatusKind::Info));
                        }
                        actions.child(
                            Button::secondary(copy_id, t!("App.Markdown.copy"))
                                .size(ButtonSize::Sm)
                                .leading_icon(IconName::Copy)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        code.to_string(),
                                    ));
                                }),
                        )
                    }),
            ),
        );
    }

    markdown_reader_shell(
        t,
        file_header(t, path_label, bytes_len, truncated, Some(source)),
        body,
    )
}

fn file_header(
    t: &crate::theme::Theme,
    path: &SharedString,
    bytes: usize,
    truncated: bool,
    copy_text: Option<SharedString>,
) -> impl IntoElement {
    let size_label: SharedString = if bytes == 0 {
        "—".into()
    } else if bytes < 1024 {
        format!("{bytes} B").into()
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f32 / 1024.0).into()
    } else {
        format!("{:.1} MB", bytes as f32 / (1024.0 * 1024.0)).into()
    };

    let status = if truncated {
        StatusPill::new(t!("App.Markdown.truncated"), StatusKind::Warning)
    } else {
        StatusPill::new(size_label, StatusKind::Info)
    };

    div()
        .h(ROW_SM_H)
        .px(SP_3)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .bg(t.color.bg_surface)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .child(text::mono(path.clone()).secondary().truncate()),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .child(status)
                .when_some(copy_text, |this, text| {
                    this.child(
                        Button::secondary(
                            SharedString::from(format!("markdown-copy-all-{}", stable_hash(&text))),
                            t!("App.Markdown.copy_all"),
                        )
                        .size(ButtonSize::Sm)
                        .leading_icon(IconName::Copy)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                        }),
                    )
                }),
        )
}

fn markdown_reader_shell(
    t: &crate::theme::Theme,
    header: impl IntoElement,
    body: impl IntoElement,
) -> impl IntoElement {
    div()
        .w_full()
        .px(SP_4)
        .py(SP_4)
        .flex()
        .flex_col()
        .items_center()
        .child(
            div()
                .w_full()
                .max_w(MARKDOWN_READER_MAX_W)
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .bg(t.color.bg_panel)
                .border_1()
                .border_color(t.color.border_subtle)
                .rounded(RADIUS_LG)
                .overflow_hidden()
                .child(header)
                .child(body),
        )
}

fn markdown_text_style(_t: &crate::theme::Theme, cx: &App) -> gpui_component::text::TextViewStyle {
    let mut style = gpui_component::text::TextViewStyle::default()
        .paragraph_gap(rems(0.7))
        .heading_font_size(|level, base| match level {
            1..=3 => base,
            4 => base * 0.9,
            _ => base * 0.8,
        });
    style.highlight_theme = cx.theme().highlight_theme.clone();
    style.is_dark = cx.theme().is_dark();
    style
}

fn load_markdown_document(path: &Path) -> MarkdownDocument {
    match std::fs::read(path) {
        Ok(bytes) => {
            let truncated = bytes.len() > MAX_RENDER_BYTES;
            let read_slice = &bytes[..bytes.len().min(MAX_RENDER_BYTES)];
            MarkdownDocument::Ready {
                bytes_len: bytes.len(),
                truncated,
                source: String::from_utf8_lossy(read_slice).into_owned().into(),
            }
        }
        Err(err) => MarkdownDocument::Error(SharedString::from(format!("{err}"))),
    }
}

fn markdown_cache_key(path: &Path) -> SharedString {
    let signature = std::fs::metadata(path)
        .ok()
        .map(|meta| {
            let modified = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis())
                .unwrap_or_default();
            format!("{}:{modified}", meta.len())
        })
        .unwrap_or_else(|| "missing".to_string());
    format!("markdown-preview:{}:{signature}", path.display()).into()
}

fn stable_hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn empty_state(t: &crate::theme::Theme) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(SP_2)
        .p(SP_4)
        .text_color(t.color.text_tertiary)
        .child(
            div().child(
                text::body(t!("App.Markdown.no_file_selected"))
                    .secondary()
                    .centered(),
            ),
        )
        .child(text::small(t!("App.Markdown.empty_hint")).centered())
}
