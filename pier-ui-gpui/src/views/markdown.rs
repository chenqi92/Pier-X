//! Markdown preview — Phase 3 replacement for the right-panel placeholder.
//!
//! Mirrors `Pier/PierApp/Sources/Views/RightPanel/MarkdownPreviewView.swift`
//! at MVP fidelity:
//!   - reads the file synchronously (small files only — Markdown previews
//!     ≤ 1 MB are typical)
//!   - parses with `pulldown-cmark` (CommonMark + tables/strikethrough)
//!   - flattens events to a `Vec<Block>` and renders each block with the
//!     existing component / theme tokens
//!
//! Deferred (Pier parity, follow-on PRs):
//!   - syntax highlighting inside fenced code blocks
//!   - clickable links, image rendering
//!   - GFM task-list checkboxes, footnotes
//!   - in-place edit mode toggle
//!   - file watcher (currently re-reads on every render — fine for now)

use std::path::PathBuf;

use gpui::{div, prelude::*, px, IntoElement, SharedString, Window};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use rust_i18n::t;

use crate::components::{text, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3, SP_4},
    theme,
    typography::{
        SIZE_BODY, SIZE_CAPTION, SIZE_MONO_CODE, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM,
    },
};

/// Files larger than this are truncated with an explanatory banner — keeps
/// the synchronous read on the render path bounded.
const MAX_RENDER_BYTES: usize = 2 * 1024 * 1024;

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
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let Some(path) = self.file_path else {
            return empty_state(t).into_any_element();
        };

        // Header strip (path + status pill).
        let path_label: SharedString = path.display().to_string().into();

        match std::fs::read(&path) {
            Ok(bytes) => {
                let truncated = bytes.len() > MAX_RENDER_BYTES;
                let read_slice = &bytes[..bytes.len().min(MAX_RENDER_BYTES)];
                let source = String::from_utf8_lossy(read_slice).into_owned();
                let blocks = parse_blocks(&source);

                let mut col = div().flex().flex_col();
                col = col.child(file_header(t, &path_label, bytes.len(), truncated));

                let mut body = div().px(SP_4).py(SP_3).flex().flex_col().gap(SP_3);
                if truncated {
                    body = body.child(
                        Card::new()
                            .padding(SP_2)
                            .child(SectionLabel::new(t!("App.Markdown.truncated")))
                            .child(
                                text::body(t!(
                                    "App.Markdown.truncated_message",
                                    shown_kb = MAX_RENDER_BYTES / 1024,
                                    total_kb = bytes.len() / 1024
                                ))
                                .secondary(),
                            ),
                    );
                }
                if blocks.is_empty() {
                    body = body.child(text::body(t!("App.Markdown.empty_document")).secondary());
                } else {
                    for block in blocks {
                        body = body.child(render_block(t, &block));
                    }
                }
                col.child(body).into_any_element()
            }
            Err(err) => div()
                .flex()
                .flex_col()
                .child(file_header(t, &path_label, 0, false))
                .child(
                    div().p(SP_4).child(
                        Card::new()
                            .padding(SP_3)
                            .child(SectionLabel::new(t!("App.Markdown.cannot_read_file")))
                            .child(text::body(SharedString::from(format!("{err}"))).secondary()),
                    ),
                )
                .into_any_element(),
        }
    }
}

// ─────────────────────────────────────────────────────────
// Header / empty
// ─────────────────────────────────────────────────────────

fn file_header(
    t: &crate::theme::Theme,
    path: &SharedString,
    bytes: usize,
    truncated: bool,
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
        StatusPill::new(size_label.clone(), StatusKind::Info)
    };

    div()
        .h(px(28.0))
        .px(SP_3)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_tertiary)
                .child(path.clone()),
        )
        .child(status)
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
            div()
                .text_size(SIZE_BODY)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_secondary)
                .child(SharedString::from(
                    t!("App.Markdown.no_file_selected").to_string(),
                )),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .child(SharedString::from(t!("App.Markdown.empty_hint").to_string())),
        )
}

// ─────────────────────────────────────────────────────────
// Block model + walker
// ─────────────────────────────────────────────────────────

/// Internal IR for the renderer. CommonMark events are flattened into this
/// shape so each variant maps cleanly to a styled GPUI block.
enum Block {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph(String),
    /// Lang hint comes from ```rust fences etc. Empty when absent.
    Code {
        lang: String,
        content: String,
    },
    /// `ordered` toggles `1. 2. 3.` vs `•` markers.
    List {
        ordered: bool,
        items: Vec<String>,
    },
    Quote(String),
    Rule,
}

fn parse_blocks(source: &str) -> Vec<Block> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(source, opts);
    let mut blocks: Vec<Block> = Vec::new();
    let mut state = WalkerState::default();

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => state.start_heading(level_to_u8(level)),
                Tag::Paragraph => state.start_paragraph(),
                Tag::CodeBlock(kind) => state.start_code_block(match kind {
                    CodeBlockKind::Fenced(lang) => lang.into_string(),
                    CodeBlockKind::Indented => String::new(),
                }),
                Tag::List(start) => state.start_list(start.is_some()),
                Tag::Item => state.start_item(),
                Tag::BlockQuote(_) => state.start_quote(),
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Heading(_) => state.end_heading(&mut blocks),
                TagEnd::Paragraph => state.end_paragraph(&mut blocks),
                TagEnd::CodeBlock => state.end_code_block(&mut blocks),
                TagEnd::List(_) => state.end_list(&mut blocks),
                TagEnd::Item => state.end_item(),
                TagEnd::BlockQuote(_) => state.end_quote(&mut blocks),
                _ => {}
            },
            Event::Text(t) => state.push_text(&t),
            Event::Code(t) => state.push_text(&format!("`{}`", t.as_ref())),
            Event::SoftBreak => state.push_text(" "),
            Event::HardBreak => state.push_text("\n"),
            Event::Rule => blocks.push(Block::Rule),
            _ => {}
        }
    }
    blocks
}

#[derive(Default)]
struct WalkerState {
    /// Stack of open container types so we know where text lands.
    container: Vec<Container>,
    /// Buffer for the currently-building leaf block.
    text: String,
    /// In-progress lists (nesting allowed but flattened to top-level for now).
    list: Option<ListBuilder>,
    /// In-progress code block.
    code_lang: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    Heading(u8),
    Paragraph,
    CodeBlock,
    List,
    Item,
    Quote,
}

struct ListBuilder {
    ordered: bool,
    items: Vec<String>,
}

impl WalkerState {
    fn start_heading(&mut self, level: u8) {
        self.container.push(Container::Heading(level));
        self.text.clear();
    }
    fn end_heading(&mut self, out: &mut Vec<Block>) {
        let level = match self.container.pop() {
            Some(Container::Heading(l)) => l,
            _ => 1,
        };
        out.push(Block::Heading {
            level,
            text: std::mem::take(&mut self.text),
        });
    }

    fn start_paragraph(&mut self) {
        self.container.push(Container::Paragraph);
        // Don't clobber if we're inside a list item — the item's text
        // should aggregate paragraph chunks.
        if !self.in_item() {
            self.text.clear();
        }
    }
    fn end_paragraph(&mut self, out: &mut Vec<Block>) {
        self.container.pop();
        if self.in_item() {
            return;
        }
        if self.in_quote() {
            return;
        }
        let text = std::mem::take(&mut self.text);
        if !text.trim().is_empty() {
            out.push(Block::Paragraph(text));
        }
    }

    fn start_code_block(&mut self, lang: String) {
        self.container.push(Container::CodeBlock);
        self.code_lang = Some(lang);
        self.text.clear();
    }
    fn end_code_block(&mut self, out: &mut Vec<Block>) {
        self.container.pop();
        let lang = self.code_lang.take().unwrap_or_default();
        out.push(Block::Code {
            lang,
            content: std::mem::take(&mut self.text),
        });
    }

    fn start_list(&mut self, ordered: bool) {
        self.container.push(Container::List);
        if self.list.is_none() {
            self.list = Some(ListBuilder {
                ordered,
                items: Vec::new(),
            });
        }
    }
    fn end_list(&mut self, out: &mut Vec<Block>) {
        self.container.pop();
        if self.container.iter().any(|c| matches!(c, Container::List)) {
            // still nested; finalise outer later
            return;
        }
        if let Some(builder) = self.list.take() {
            if !builder.items.is_empty() {
                out.push(Block::List {
                    ordered: builder.ordered,
                    items: builder.items,
                });
            }
        }
    }

    fn start_item(&mut self) {
        self.container.push(Container::Item);
        self.text.clear();
    }
    fn end_item(&mut self) {
        self.container.pop();
        let text = std::mem::take(&mut self.text);
        if let Some(builder) = self.list.as_mut() {
            builder.items.push(text);
        }
    }

    fn start_quote(&mut self) {
        self.container.push(Container::Quote);
        self.text.clear();
    }
    fn end_quote(&mut self, out: &mut Vec<Block>) {
        self.container.pop();
        let text = std::mem::take(&mut self.text);
        if !text.trim().is_empty() {
            out.push(Block::Quote(text));
        }
    }

    fn push_text(&mut self, s: &str) {
        self.text.push_str(s);
    }

    fn in_item(&self) -> bool {
        self.container.iter().any(|c| matches!(c, Container::Item))
    }
    fn in_quote(&self) -> bool {
        self.container.iter().any(|c| matches!(c, Container::Quote))
    }
}

fn level_to_u8(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

// ─────────────────────────────────────────────────────────
// Block renderer
// ─────────────────────────────────────────────────────────

fn render_block(t: &crate::theme::Theme, block: &Block) -> gpui::AnyElement {
    match block {
        Block::Heading { level, text: s } => match level {
            1 => text::h1(SharedString::from(s.clone())).into_any_element(),
            2 => text::h2(SharedString::from(s.clone())).into_any_element(),
            3 => text::h3(SharedString::from(s.clone())).into_any_element(),
            _ => text::body(SharedString::from(s.clone())).into_any_element(),
        },
        Block::Paragraph(s) => text::body(SharedString::from(s.clone())).into_any_element(),
        Block::Code { lang, content } => render_code_block(t, lang, content).into_any_element(),
        Block::List { ordered, items } => render_list(t, *ordered, items).into_any_element(),
        Block::Quote(s) => render_quote(t, s).into_any_element(),
        Block::Rule => div()
            .h(px(1.0))
            .w_full()
            .my(SP_2)
            .bg(t.color.border_default)
            .into_any_element(),
    }
}

fn render_code_block(t: &crate::theme::Theme, lang: &str, content: &str) -> impl IntoElement {
    let mut col = div()
        .p(SP_3)
        .rounded(RADIUS_SM)
        .bg(t.color.bg_panel)
        .border_1()
        .border_color(t.color.border_subtle)
        .flex()
        .flex_col()
        .gap(SP_1);

    if !lang.is_empty() {
        col = col.child(
            div()
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(lang.to_string())),
        );
    }

    // Render each line as its own div so wrap/scroll behaves predictably.
    for line in content.trim_end_matches('\n').lines() {
        col = col.child(
            div()
                .text_size(SIZE_MONO_CODE)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_primary)
                .child(SharedString::from(line.to_string())),
        );
    }
    col
}

fn render_list(t: &crate::theme::Theme, ordered: bool, items: &[String]) -> impl IntoElement {
    let mut col = div().flex().flex_col().gap(SP_1);
    for (idx, item) in items.iter().enumerate() {
        let marker: SharedString = if ordered {
            format!("{}.", idx + 1).into()
        } else {
            "•".into()
        };
        col = col.child(
            div()
                .flex()
                .flex_row()
                .gap(SP_2)
                .child(
                    div()
                        .w(px(20.0))
                        .text_size(SIZE_BODY)
                        .text_color(t.color.text_tertiary)
                        .child(marker),
                )
                .child(
                    div()
                        .flex_1()
                        .child(text::body(SharedString::from(item.clone()))),
                ),
        );
    }
    col
}

fn render_quote(t: &crate::theme::Theme, s: &str) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .gap(SP_2)
        .child(div().w(px(2.0)).h_full().bg(t.color.accent_muted))
        .child(
            div()
                .flex_1()
                .pl(SP_1_5)
                .child(text::body(SharedString::from(s.to_string())).secondary()),
        )
}
