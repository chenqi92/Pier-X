// Markdown panel — native rendered preview of a local markdown file.
//
// Loads CHANGELOG.md / README.md from the launch directory via pier-core's
// markdown service, parses it into block/inline elements, and paints them with
// GPUI primitives (headings, paragraphs, bullet/ordered/task lists, blockquotes,
// fenced code on `t.panel_2`, horizontal rules). The parser is intentionally
// small but handles the constructs a real README/CHANGELOG uses; raw HTML tags
// are stripped to their text so the common centered-logo header degrades to
// plain prose. File IO + parsing run on a background task; render only paints.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, FontWeight, Hsla, Pixels, SharedString, Window};
use gpui_component::{h_flex, v_flex};

use crate::data;
use crate::theme::Theme;
use crate::ui;

pub struct MarkdownPanel {
    theme: Theme,
    state: PanelState,
}

/// Load lifecycle for the previewed file.
enum PanelState {
    Loading,
    Empty,
    Error(String),
    Loaded { file: String, blocks: Vec<Block> },
}

impl MarkdownPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Resolve the directory on the main thread (cheap), then do the file
        // find + read + parse off the render path and notify when it lands.
        let dir = data::current_dir();
        cx.spawn(async move |this, cx| {
            let state = cx
                .background_executor()
                .spawn(async move { load_doc(dir) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.state = state;
                cx.notify();
            });
        })
        .detach();

        Self {
            theme: Theme::dark(),
            state: PanelState::Loading,
        }
    }
}

impl Render for MarkdownPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = &self.theme;
        let (meta, body): (String, AnyElement) = match &self.state {
            PanelState::Loading => (
                String::new(),
                div()
                    .p(t.sp4)
                    .text_color(t.muted)
                    .child("Loading…")
                    .into_any_element(),
            ),
            PanelState::Empty => (
                String::new(),
                ui::empty_state(t, "No CHANGELOG.md or README.md in this folder")
                    .into_any_element(),
            ),
            PanelState::Error(e) => (
                String::new(),
                div()
                    .p(t.sp4)
                    .text_color(t.neg)
                    .child(e.clone())
                    .into_any_element(),
            ),
            PanelState::Loaded { file, blocks } => {
                let mut col = v_flex().w_full().p(t.sp4).gap(t.sp3);
                for (i, b) in blocks.iter().enumerate() {
                    col = col.child(render_block(t, i, b));
                }
                (file.clone(), col.into_any_element())
            }
        };

        v_flex()
            .size_full()
            .child(ui::panel_header(t, "file-text", "MARKDOWN", meta))
            .child(
                div()
                    .id("md-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}

// ── Loading ──────────────────────────────────────────────────────

/// Find the first preview-worthy markdown file under `dir` and parse it.
fn load_doc(dir: PathBuf) -> PanelState {
    const CANDIDATES: &[&str] = &[
        "CHANGELOG.md",
        "README.md",
        "Readme.md",
        "readme.md",
        "README.markdown",
        "CHANGELOG",
    ];
    for name in CANDIDATES {
        let path = dir.join(name);
        if path.is_file() {
            return match pier_core::markdown::load_file(&path) {
                Ok(src) => PanelState::Loaded {
                    file: (*name).to_string(),
                    blocks: parse_blocks(&src),
                },
                Err(e) => PanelState::Error(format!("{name}: {e}")),
            };
        }
    }
    PanelState::Empty
}

// ── Parsed model ─────────────────────────────────────────────────

enum Block {
    Heading(u8, Vec<Span>),
    Paragraph(Vec<Span>),
    Bullet {
        indent: usize,
        task: Option<bool>,
        spans: Vec<Span>,
    },
    Ordered {
        indent: usize,
        num: String,
        spans: Vec<Span>,
    },
    Code(String),
    Quote(Vec<Span>),
    Rule,
}

enum Span {
    Text(String),
    Strong(String),
    Emph(String),
    Strike(String),
    Code(String),
    Link(String),
}

// ── Block parser ─────────────────────────────────────────────────

fn parse_blocks(src: &str) -> Vec<Block> {
    let lines: Vec<&str> = src.lines().collect();
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Vec<String> = Vec::new();

    // Drain accumulated soft-wrapped paragraph lines into one Paragraph block.
    let flush = |para: &mut Vec<String>, blocks: &mut Vec<Block>| {
        if para.is_empty() {
            return;
        }
        let text = para.join(" ");
        para.clear();
        let spans = parse_inline(text.trim());
        if !spans.is_empty() {
            blocks.push(Block::Paragraph(spans));
        }
    };

    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let traw = raw.trim_start();

        // Fenced code: collect raw lines verbatim until the closing fence.
        if traw.starts_with("```") || traw.starts_with("~~~") {
            flush(&mut para, &mut blocks);
            let fence = if traw.starts_with("```") { "```" } else { "~~~" };
            i += 1;
            let mut code: Vec<&str> = Vec::new();
            while i < lines.len() {
                if lines[i].trim_start().starts_with(fence) {
                    i += 1;
                    break;
                }
                code.push(lines[i]);
                i += 1;
            }
            blocks.push(Block::Code(code.join("\n")));
            continue;
        }

        // Everything else: drop HTML tags to their inner text first, so a
        // centered-logo `<div>…</div>` header degrades to readable prose.
        let line = strip_html_tags(raw);
        let t = line.trim();

        if t.is_empty() {
            flush(&mut para, &mut blocks);
            i += 1;
            continue;
        }
        if let Some((level, rest)) = atx_heading(t) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Heading(level, parse_inline(rest)));
            i += 1;
            continue;
        }
        if is_hr(t) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Rule);
            i += 1;
            continue;
        }
        if let Some(rest) = t.strip_prefix('>') {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Quote(parse_inline(rest.trim())));
            i += 1;
            continue;
        }
        if let Some((indent, task, rest)) = parse_bullet(&line) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Bullet {
                indent,
                task,
                spans: parse_inline(&rest),
            });
            i += 1;
            continue;
        }
        if let Some((indent, num, rest)) = parse_ordered(&line) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Ordered {
                indent,
                num,
                spans: parse_inline(&rest),
            });
            i += 1;
            continue;
        }

        para.push(t.to_string());
        i += 1;
    }
    flush(&mut para, &mut blocks);
    blocks
}

/// `#`..`######` heading → (level, trimmed text). `#foo` (no space) is not one.
fn atx_heading(t: &str) -> Option<(u8, &str)> {
    let level = t.chars().take_while(|&c| c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &t[level..];
    if rest.is_empty() {
        return Some((level as u8, ""));
    }
    if !rest.starts_with(' ') {
        return None;
    }
    Some((level as u8, rest.trim().trim_end_matches('#').trim_end()))
}

/// A `---` / `***` / `___` rule (≥3 of one char, spaces ignored).
fn is_hr(t: &str) -> bool {
    let s: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    if s.len() < 3 {
        return false;
    }
    let first = s.chars().next().unwrap();
    matches!(first, '-' | '*' | '_') && s.chars().all(|c| c == first)
}

/// `- ` / `* ` / `+ ` item → (indent level, task-checkbox state, content).
fn parse_bullet(line: &str) -> Option<(usize, Option<bool>, String)> {
    let indent_cols = line.len() - line.trim_start().len();
    let t = line.trim_start();
    let rest = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .or_else(|| t.strip_prefix("+ "))?;
    let indent = (indent_cols / 2).min(6);
    let (task, content) = if let Some(r) = rest.strip_prefix("[ ] ") {
        (Some(false), r)
    } else if let Some(r) = rest
        .strip_prefix("[x] ")
        .or_else(|| rest.strip_prefix("[X] "))
    {
        (Some(true), r)
    } else {
        (None, rest)
    };
    Some((indent, task, content.to_string()))
}

/// `N. ` / `N) ` item → (indent level, number text, content).
fn parse_ordered(line: &str) -> Option<(usize, String, String)> {
    let indent_cols = line.len() - line.trim_start().len();
    let t = line.trim_start();
    let bytes = t.as_bytes();
    let mut k = 0;
    while k < bytes.len() && bytes[k].is_ascii_digit() {
        k += 1;
    }
    if k == 0 || k > 9 || k + 1 >= bytes.len() {
        return None;
    }
    if bytes[k] != b'.' && bytes[k] != b')' {
        return None;
    }
    if bytes[k + 1] != b' ' {
        return None;
    }
    let indent = (indent_cols / 2).min(6);
    Some((indent, t[..k].to_string(), t[k + 2..].to_string()))
}

/// Remove `<tag …>`, `</tag>`, and `<!-- … -->`, keeping inner text. A bare
/// `<` not starting a tag (e.g. `a < b`) is preserved.
fn strip_html_tags(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < n {
        if chars[i] == '<' {
            let mut j = i + 1;
            // HTML comment <!-- … -->
            if j < n && chars[j] == '!' {
                while j < n && chars[j] != '>' {
                    j += 1;
                }
                i = if j < n { j + 1 } else { n };
                continue;
            }
            // Optional closing slash, then a tag name must start with a letter.
            if j < n && chars[j] == '/' {
                j += 1;
            }
            if j < n && chars[j].is_ascii_alphabetic() {
                while j < n && chars[j] != '>' {
                    j += 1;
                }
                i = if j < n { j + 1 } else { n };
                continue;
            }
            out.push('<');
            i += 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

// ── Inline parser ────────────────────────────────────────────────

fn parse_inline(text: &str) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut spans: Vec<Span> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    while i < n {
        let hit = try_code(&chars, i)
            .or_else(|| try_strong(&chars, i))
            .or_else(|| try_strike(&chars, i))
            .or_else(|| try_emph(&chars, i))
            .or_else(|| try_link(&chars, i));
        if let Some((span, next)) = hit {
            if !buf.is_empty() {
                spans.push(Span::Text(std::mem::take(&mut buf)));
            }
            spans.push(span);
            i = next;
        } else {
            buf.push(chars[i]);
            i += 1;
        }
    }
    if !buf.is_empty() {
        spans.push(Span::Text(buf));
    }
    spans
}

fn slice(chars: &[char], a: usize, b: usize) -> String {
    chars[a..b].iter().collect()
}

/// `` `code` `` — first single-backtick pair, non-empty.
fn try_code(chars: &[char], i: usize) -> Option<(Span, usize)> {
    if chars[i] != '`' {
        return None;
    }
    let mut j = i + 1;
    while j < chars.len() && chars[j] != '`' {
        j += 1;
    }
    if j < chars.len() && j > i + 1 {
        return Some((Span::Code(slice(chars, i + 1, j)), j + 1));
    }
    None
}

/// `**strong**` — flanked (no inner-edge whitespace), non-empty.
fn try_strong(chars: &[char], i: usize) -> Option<(Span, usize)> {
    let n = chars.len();
    if !(chars[i] == '*' && i + 1 < n && chars[i + 1] == '*') {
        return None;
    }
    if i + 2 >= n || chars[i + 2].is_whitespace() {
        return None;
    }
    let mut j = i + 2;
    while j + 1 < n {
        if chars[j] == '*' && chars[j + 1] == '*' && !chars[j - 1].is_whitespace() {
            return Some((Span::Strong(slice(chars, i + 2, j)), j + 2));
        }
        j += 1;
    }
    None
}

/// `*emph*` — flanked single star, not the start of `**`.
fn try_emph(chars: &[char], i: usize) -> Option<(Span, usize)> {
    let n = chars.len();
    if chars[i] != '*' || i + 1 >= n {
        return None;
    }
    if chars[i + 1] == '*' || chars[i + 1].is_whitespace() {
        return None;
    }
    let mut j = i + 1;
    while j < n {
        if chars[j] == '*' && !chars[j - 1].is_whitespace() {
            return Some((Span::Emph(slice(chars, i + 1, j)), j + 1));
        }
        j += 1;
    }
    None
}

/// `~~strike~~` — non-empty.
fn try_strike(chars: &[char], i: usize) -> Option<(Span, usize)> {
    let n = chars.len();
    if !(chars[i] == '~' && i + 1 < n && chars[i + 1] == '~') {
        return None;
    }
    let mut j = i + 2;
    while j + 1 < n {
        if chars[j] == '~' && chars[j + 1] == '~' && j > i + 2 {
            return Some((Span::Strike(slice(chars, i + 2, j)), j + 2));
        }
        j += 1;
    }
    None
}

/// `[label](url)` — keeps the label, drops the target.
fn try_link(chars: &[char], i: usize) -> Option<(Span, usize)> {
    let n = chars.len();
    if chars[i] != '[' {
        return None;
    }
    let mut r = i + 1;
    while r < n && chars[r] != ']' {
        r += 1;
    }
    if r >= n || r + 1 >= n || chars[r + 1] != '(' || r == i + 1 {
        return None;
    }
    let mut p = r + 2;
    while p < n && chars[p] != ')' {
        p += 1;
    }
    if p >= n {
        return None;
    }
    Some((Span::Link(slice(chars, i + 1, r)), p + 1))
}

// ── Block rendering ──────────────────────────────────────────────

fn render_block(t: &Theme, idx: usize, b: &Block) -> AnyElement {
    match b {
        Block::Heading(level, spans) => {
            let mut el = v_flex()
                .w_full()
                .child(inline(t, spans, heading_base(t, *level)));
            if *level <= 2 {
                el = el.pb(t.sp1).border_b_1().border_color(t.line);
            }
            el.into_any_element()
        }
        Block::Paragraph(spans) => inline(t, spans, body_base(t)).into_any_element(),
        Block::Bullet {
            indent,
            task,
            spans,
        } => list_row(t, bullet_marker(t, *task), *indent, spans).into_any_element(),
        Block::Ordered {
            indent,
            num,
            spans,
        } => list_row(t, ordered_marker(t, num), *indent, spans).into_any_element(),
        Block::Code(code) => code_block(t, idx, code).into_any_element(),
        Block::Quote(spans) => div()
            .w_full()
            .pl(t.sp3)
            .border_l_2()
            .border_color(t.line_2)
            .child(inline(t, spans, quote_base(t)))
            .into_any_element(),
        Block::Rule => div().w_full().h(px(1.0)).bg(t.line_2).into_any_element(),
    }
}

/// A list item: leading marker + wrapping inline body, indented by nesting.
fn list_row(t: &Theme, marker: AnyElement, indent: usize, spans: &[Span]) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_start()
        .gap(t.sp2)
        .pl(t.sp4 * indent)
        .child(marker)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .child(inline(t, spans, body_base(t))),
        )
}

fn bullet_marker(t: &Theme, task: Option<bool>) -> AnyElement {
    match task {
        None => div()
            .flex_none()
            .pt(px(7.0))
            .child(div().w(px(5.0)).h(px(5.0)).rounded_full().bg(t.muted))
            .into_any_element(),
        Some(done) => div()
            .flex_none()
            .font_family(t.mono.clone())
            .text_size(t.fs_body)
            .text_color(if done { t.pos } else { t.muted })
            .child(if done { "[x]" } else { "[ ]" })
            .into_any_element(),
    }
}

fn ordered_marker(t: &Theme, num: &str) -> AnyElement {
    div()
        .flex_none()
        .min_w(px(16.0))
        .font_family(t.mono.clone())
        .text_size(t.fs_body)
        .text_color(t.muted)
        .child(format!("{num}."))
        .into_any_element()
}

/// Fenced code: mono text on `panel_2`, one div per line, horizontally
/// scrollable so long lines stay reachable instead of wrapping.
fn code_block(t: &Theme, idx: usize, code: &str) -> impl IntoElement {
    let mut col = v_flex()
        .id(SharedString::from(format!("md-code-{idx}")))
        .overflow_x_scroll()
        .w_full()
        .p(t.sp3)
        .rounded(t.radius_md)
        .bg(t.panel_2)
        .border_1()
        .border_color(t.line)
        .font_family(t.mono.clone())
        .text_size(t.fs_sm)
        .text_color(t.ink_2);
    for line in code.split('\n') {
        let text = if line.is_empty() {
            " ".to_string()
        } else {
            line.to_string()
        };
        col = col.child(div().whitespace_nowrap().child(text));
    }
    col
}

// ── Inline rendering ─────────────────────────────────────────────

/// Base text style a block applies to its inline spans.
struct Base {
    size: Pixels,
    color: Hsla,
    weight: FontWeight,
    italic: bool,
}

fn body_base(t: &Theme) -> Base {
    Base {
        size: t.fs_body,
        color: t.ink_2,
        weight: FontWeight::NORMAL,
        italic: false,
    }
}

fn quote_base(t: &Theme) -> Base {
    Base {
        size: t.fs_body,
        color: t.muted,
        weight: FontWeight::NORMAL,
        italic: true,
    }
}

/// Headings differentiate by weight/colour/rule within the existing type scale
/// (no oversized tokens exist, and large type reads poorly in a 360px panel).
fn heading_base(t: &Theme, level: u8) -> Base {
    match level {
        1 => Base {
            size: t.fs_h3,
            color: t.ink,
            weight: FontWeight::BOLD,
            italic: false,
        },
        2 | 3 => Base {
            size: t.fs_h3,
            color: t.ink,
            weight: FontWeight::SEMIBOLD,
            italic: false,
        },
        4 => Base {
            size: t.fs_body,
            color: t.ink,
            weight: FontWeight::SEMIBOLD,
            italic: false,
        },
        _ => Base {
            size: t.fs_ui,
            color: t.muted,
            weight: FontWeight::SEMIBOLD,
            italic: false,
        },
    }
}

/// Lay inline spans out as wrapping word tokens; inline code is a single mono
/// chip. Word-level wrapping keeps prose flowing inside the narrow panel.
fn inline(t: &Theme, spans: &[Span], base: Base) -> impl IntoElement {
    let mut row = h_flex().w_full().flex_wrap().items_center().gap(t.sp1);
    for span in spans {
        match span {
            Span::Code(c) => {
                row = row.child(code_chip(t, c));
            }
            Span::Text(x) => {
                for w in x.split_whitespace() {
                    row = row.child(word(w, base.size, base.color, base.weight, base.italic, false, false));
                }
            }
            Span::Strong(x) => {
                for w in x.split_whitespace() {
                    row = row.child(word(w, base.size, t.ink, FontWeight::SEMIBOLD, base.italic, false, false));
                }
            }
            Span::Emph(x) => {
                for w in x.split_whitespace() {
                    row = row.child(word(w, base.size, base.color, base.weight, true, false, false));
                }
            }
            Span::Strike(x) => {
                for w in x.split_whitespace() {
                    row = row.child(word(w, base.size, t.muted, base.weight, base.italic, true, false));
                }
            }
            Span::Link(x) => {
                for w in x.split_whitespace() {
                    row = row.child(word(w, base.size, t.accent, base.weight, base.italic, false, true));
                }
            }
        }
    }
    row
}

#[allow(clippy::too_many_arguments)]
fn word(
    text: &str,
    size: Pixels,
    color: Hsla,
    weight: FontWeight,
    italic: bool,
    strike: bool,
    underline: bool,
) -> impl IntoElement {
    div()
        .text_size(size)
        .text_color(color)
        .font_weight(weight)
        .when(italic, |d| d.italic())
        .when(strike, |d| d.line_through())
        .when(underline, |d| d.underline())
        .child(text.to_string())
}

fn code_chip(t: &Theme, code: &str) -> impl IntoElement {
    div()
        .font_family(t.mono.clone())
        .text_size(t.fs_sm)
        .text_color(t.ink)
        .bg(t.panel_2)
        .px(t.sp1)
        .rounded(t.radius_sm)
        .whitespace_nowrap()
        .child(code.to_string())
}
