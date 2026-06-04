// Logs panel — live tail of Pier-X's own log file with client-side controls.
//
// The active file comes from `pier_core::logging::log_file_path()` (set when
// the runtime initialises the file logger; `None` means logging was never
// started, so there is nothing to show). A controlled ~1s loop — modelled on
// shell.rs::monitor_panel — tails the file on the background executor (never in
// render): the first pass seeds from the last `TAIL_LINES` lines, later passes
// read only the bytes appended since (so a rotation/truncation re-seeds and a
// quiet log doesn't repaint). Lines are coloured by their `[LEVEL]` tag, drawn
// in the mono face, and the body stays pinned to the newest line.
//
// On top of the tail the toolbar offers what the web LogViewerPanel exposes:
// level chips (All/Info/Warn/Error) and a text filter that narrow the visible
// rows client-side, plus pause (freeze appends), clear (empty the buffer; only
// newly-written lines appear after) and jump-to-bottom controls.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Context, Div, FocusHandle, Hsla, KeyDownEvent, MouseButton,
    MouseDownEvent, ScrollHandle, SharedString, Window,
};
use gpui_component::{h_flex, v_flex};

use crate::theme::Theme;
use crate::ui;

/// How many trailing lines to keep in view (the panel contract: ~500).
const TAIL_LINES: usize = 500;

/// Severity parsed from a line's `[LEVEL]` tag — used only to pick a colour.
#[derive(Clone, Copy, PartialEq)]
enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
    Other,
}

impl Level {
    /// Extract the level from a logger line shaped `<ts> [LEVEL] [src] msg`
    /// (see `pier_core::logging`). The level is the first bracketed token;
    /// verbose records carry a trailing `+` (e.g. `[INFO+]`).
    fn parse(line: &str) -> Self {
        let tag = line
            .split_once('[')
            .and_then(|(_, rest)| rest.split_once(']'))
            .map(|(tag, _)| tag.trim().trim_end_matches('+'))
            .unwrap_or("");
        match tag {
            "ERROR" => Level::Error,
            "WARN" => Level::Warn,
            "INFO" => Level::Info,
            "DEBUG" => Level::Debug,
            "TRACE" => Level::Trace,
            _ => Level::Other,
        }
    }

    fn color(self, t: &Theme) -> Hsla {
        match self {
            Level::Error => t.neg,
            Level::Warn => t.warn,
            Level::Info => t.info,
            Level::Debug | Level::Trace => t.muted,
            Level::Other => t.ink_2,
        }
    }
}

/// Toolbar level chip. `All` passes everything; the rest match a single level
/// (Debug/Trace/Other stay visible only under `All`, matching the chip set).
#[derive(Clone, Copy, PartialEq)]
enum LevelFilter {
    All,
    Info,
    Warn,
    Error,
}

impl LevelFilter {
    fn accepts(self, lvl: Level) -> bool {
        match self {
            LevelFilter::All => true,
            LevelFilter::Info => lvl == Level::Info,
            LevelFilter::Warn => lvl == Level::Warn,
            LevelFilter::Error => lvl == Level::Error,
        }
    }
}

/// A toolbar control action dispatched from an icon button.
#[derive(Clone, Copy)]
enum Tool {
    PauseToggle,
    Clear,
    Bottom,
}

/// Outcome of one background tail read.
enum TailRead {
    /// Replace the buffer (first load, or after a rotation/truncation).
    Reset(Vec<String>),
    /// Append these newly-written complete lines.
    Append(Vec<String>),
    /// Nothing new since last pass.
    Idle,
}

pub struct LogsPanel {
    theme: Theme,
    /// The active log file, or `None` if logging was never initialised.
    path: Option<PathBuf>,
    /// The most recent tail (oldest first, newest last).
    lines: Vec<String>,
    /// Bytes of `path` already consumed by the tail loop.
    offset: u64,
    /// Whether the first (seeding) read has run; gates incremental reads.
    seeded: bool,
    /// Whether the first background read has resolved (distinguishes the
    /// initial "reading" state from a genuinely empty log).
    loaded: bool,
    /// When true the tail loop stops appending so the view freezes.
    paused: bool,
    /// Active level chip.
    level_filter: LevelFilter,
    /// Live text filter over visible rows (matched case-insensitively).
    query: String,
    /// Focus for the inline filter input.
    focus: FocusHandle,
    /// Bumped on Clear so an in-flight read started before the clear is
    /// consumed (offset advances) but its lines are dropped.
    generation: u64,
    /// Keeps the body scrolled to the newest line as the tail grows.
    scroll: ScrollHandle,
}

impl LogsPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let path = pier_core::logging::log_file_path();
        if let Some(p) = path.clone() {
            Self::start_tail(cx, p);
        }
        Self {
            theme: Theme::dark(),
            path,
            lines: Vec::new(),
            offset: 0,
            seeded: false,
            loaded: false,
            paused: false,
            level_filter: LevelFilter::All,
            query: String::new(),
            focus: cx.focus_handle(),
            generation: 0,
            scroll: ScrollHandle::new(),
        }
    }

    /// Tail `path` every ~1s on the background executor while the view is
    /// alive. Each pass first snapshots the control state (pause/offset) on the
    /// main thread, then — unless paused — reads off the render path and writes
    /// the result back. The loop ends when the entity is dropped (`update`
    /// fails). We only notify when the visible content changed so an idle log
    /// doesn't repaint or yank the user's scroll position.
    fn start_tail(cx: &mut Context<Self>, path: PathBuf) {
        cx.spawn(async move |this, cx| loop {
            let Some((paused, offset, seeded, gen0)) = this
                .update(cx, |this, _| {
                    (this.paused, this.offset, this.seeded, this.generation)
                })
                .ok()
            else {
                break; // entity dropped
            };

            if !paused {
                let p = path.clone();
                let (read, new_off) = cx
                    .background_executor()
                    .spawn(async move {
                        let mut off = offset;
                        let r = read_tail(&p, &mut off, seeded);
                        (r, off)
                    })
                    .await;

                let alive = this
                    .update(cx, |this, cx| {
                        let first = !this.loaded;
                        this.loaded = true;
                        let Some(read) = read else {
                            // Transient read error: keep the last snapshot and
                            // re-seed next pass; just leave the initial state.
                            if first {
                                cx.notify();
                            }
                            return;
                        };
                        this.seeded = true;
                        this.offset = new_off;
                        // A clear landed while this read was in flight: the
                        // bytes are now consumed, so drop the stale lines.
                        if this.generation != gen0 {
                            if first {
                                cx.notify();
                            }
                            return;
                        }
                        let mut changed = first;
                        match read {
                            TailRead::Reset(lines) => {
                                if this.lines != lines {
                                    this.lines = lines;
                                    changed = true;
                                }
                            }
                            TailRead::Append(mut lines) => {
                                if !lines.is_empty() {
                                    this.lines.append(&mut lines);
                                    let n = this.lines.len();
                                    if n > TAIL_LINES {
                                        this.lines.drain(0..n - TAIL_LINES);
                                    }
                                    changed = true;
                                }
                            }
                            TailRead::Idle => {}
                        }
                        if changed {
                            if !this.paused {
                                this.scroll.scroll_to_bottom();
                            }
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }

            cx.background_executor().timer(Duration::from_secs(1)).await;
        })
        .detach();
    }

    /// Handle a toolbar control button.
    fn on_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        match tool {
            Tool::PauseToggle => self.paused = !self.paused,
            Tool::Clear => {
                // Empty the buffer; `offset` already sits at EOF, so the tail
                // loop only re-populates with lines written after this point.
                // Bumping the generation discards any read already in flight.
                self.lines.clear();
                self.generation = self.generation.wrapping_add(1);
            }
            Tool::Bottom => self.scroll.scroll_to_bottom(),
        }
        cx.notify();
    }

    fn on_search_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "enter" => return,
            "backspace" => {
                if self.query.pop().is_some() {
                    cx.notify();
                }
                return;
            }
            "escape" => {
                if !self.query.is_empty() {
                    self.query.clear();
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        let m = &ks.modifiers;
        if m.control || m.alt || m.platform {
            return; // leave shortcuts alone
        }
        if let Some(kc) = &ks.key_char {
            if !kc.is_empty() && !kc.chars().any(|c| c.is_control()) {
                self.query.push_str(kc);
                cx.notify();
            }
        }
    }

    /// One level-filter chip, tinted by its own level colour when active.
    fn level_chip(
        &self,
        cx: &mut Context<Self>,
        label: &'static str,
        filter: LevelFilter,
        tone: Hsla,
    ) -> impl IntoElement {
        let t = &self.theme;
        let active = self.level_filter == filter;
        div()
            .id(SharedString::from(format!("logchip-{label}")))
            .px(t.sp2)
            .py(px(2.0))
            .rounded(t.radius_sm)
            .border_1()
            .text_size(t.fs_sm)
            .cursor_pointer()
            .when(active, |d| {
                d.border_color(t.accent)
                    .bg(t.accent_subtle)
                    .text_color(tone)
            })
            .when(!active, |d| {
                d.border_color(t.line)
                    .text_color(t.muted)
                    .hover(|s| s.bg(t.hover))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.level_filter = filter;
                    cx.notify();
                }),
            )
            .child(label)
    }

    /// A small toolbar icon button running a [`Tool`] action.
    fn tool_btn(
        &self,
        cx: &mut Context<Self>,
        key: &'static str,
        glyph: &'static str,
        color: Hsla,
        tool: Tool,
        active: bool,
    ) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(SharedString::from(format!("logtool-{key}")))
            .flex()
            .items_center()
            .justify_center()
            .w(px(24.0))
            .h(px(24.0))
            .rounded(t.radius_sm)
            .cursor_pointer()
            .when(active, |d| d.bg(t.accent_subtle))
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.on_tool(tool, cx);
                }),
            )
            .child(ui::icon(glyph, px(14.0), color))
    }

    /// Chips + control buttons row.
    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp1)
            .w_full()
            .px(t.sp3)
            .py(t.sp2)
            .border_b_1()
            .border_color(t.line)
            .child(self.level_chip(cx, "All", LevelFilter::All, t.accent))
            .child(self.level_chip(cx, "Info", LevelFilter::Info, t.info))
            .child(self.level_chip(cx, "Warn", LevelFilter::Warn, t.warn))
            .child(self.level_chip(cx, "Error", LevelFilter::Error, t.neg))
            .child(div().flex_1())
            .child(self.tool_btn(
                cx,
                "pause",
                if self.paused { "play" } else { "pause" },
                if self.paused { t.warn } else { t.muted },
                Tool::PauseToggle,
                self.paused,
            ))
            .child(self.tool_btn(cx, "clear", "delete", t.muted, Tool::Clear, false))
            .child(self.tool_btn(cx, "bottom", "arrow-down", t.muted, Tool::Bottom, false))
    }

    /// Inline text filter input (focus + caret echo, mirroring search.rs).
    fn search_bar(&self, focused: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let caret = || div().flex_none().w(px(2.0)).h(px(13.0)).bg(t.accent);
        let content: AnyElement = if self.query.is_empty() {
            if focused {
                h_flex().items_center().child(caret()).into_any_element()
            } else {
                div().text_color(t.dim).child("Filter lines…").into_any_element()
            }
        } else {
            let mut row = h_flex()
                .items_center()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(
                    div()
                        .flex_none()
                        .font_family(t.mono.clone())
                        .text_color(t.ink)
                        .child(self.query.clone()),
                );
            if focused {
                row = row.child(caret());
            }
            row.into_any_element()
        };

        h_flex()
            .id("logs-filter")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_search_key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    cx.notify();
                }),
            )
            .items_center()
            .gap(t.sp2)
            .mx(t.sp3)
            .my(t.sp2)
            .px(t.sp2)
            .h(px(28.0))
            .rounded(t.radius_md)
            .bg(t.panel)
            .border_1()
            .border_color(if focused { t.accent } else { t.line })
            .child(ui::icon("search", px(13.0), t.muted))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(content),
            )
            .when(!self.query.is_empty(), |d| {
                d.child(
                    div()
                        .id("logs-filter-clear")
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(18.0))
                        .h(px(18.0))
                        .rounded(t.radius_sm)
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                                this.query.clear();
                                cx.notify();
                            }),
                        )
                        .child(ui::icon("close", px(12.0), t.muted)),
                )
            })
    }

    /// One log line: mono, sized small, tinted by its level. When `q` is set,
    /// matches are highlighted in the accent colour.
    fn line_row(&self, raw: &str, q: &str) -> impl IntoElement {
        let t = &self.theme;
        let base = Level::parse(raw).color(t);
        let row = div()
            .w_full()
            .px(t.sp3)
            .py(px(2.0))
            .font_family(t.mono.clone())
            .text_size(t.fs_sm);
        if q.is_empty() {
            row.text_color(base).child(raw.to_string())
        } else {
            row.child(
                h_flex()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .children(highlight_spans(raw, q, base, t.accent)),
            )
        }
    }
}

impl Render for LogsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = self.theme.clone();

        // No log file → logging was never initialised; nothing to tail.
        if self.path.is_none() {
            return v_flex()
                .size_full()
                .child(ui::panel_header(&t, "scroll-text", "LOGS", ""))
                .child(ui::empty_state(&t, "No log file"));
        }

        // First read hasn't resolved yet.
        if !self.loaded {
            return v_flex()
                .size_full()
                .child(ui::panel_header(&t, "scroll-text", "LOGS", ""))
                .child(ui::empty_state(&t, "Reading log…"));
        }

        // Apply the level chip + text filter to build the visible rows.
        let q = self.query.trim().to_ascii_lowercase();
        let visible: Vec<&String> = self
            .lines
            .iter()
            .filter(|l| {
                self.level_filter.accepts(Level::parse(l))
                    && (q.is_empty() || l.to_ascii_lowercase().contains(&q))
            })
            .collect();

        let total = self.lines.len();
        let filtered = self.level_filter != LevelFilter::All || !q.is_empty();
        let mut meta = if filtered {
            format!("{}/{} lines", visible.len(), total)
        } else {
            format!("{total} lines")
        };
        if self.paused {
            meta.push_str(" · paused");
        }

        let focused = self.focus.is_focused(window);
        let root = v_flex()
            .size_full()
            .child(ui::panel_header(&t, "scroll-text", "LOGS", meta))
            .child(self.toolbar(cx))
            .child(self.search_bar(focused, cx));

        if visible.is_empty() {
            let note = if total == 0 {
                "Log is empty"
            } else {
                "No matching lines"
            };
            return root.child(ui::empty_state(&t, note));
        }

        let mut body = v_flex().w_full().py(t.sp1);
        for line in &visible {
            body = body.child(self.line_row(line, &q));
        }
        root.child(
            div()
                .id("logs-scroll")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(body),
        )
    }
}

/// Tail `path` from `*offset`. The first call (or one after the file shrank,
/// i.e. a rotation/truncation) re-seeds from the last `TAIL_LINES` lines and
/// resumes from the new end; otherwise only the bytes appended since are read,
/// up to the last complete line. Blocking — run only on the background
/// executor. Returns `None` if the file can't be read so the caller can keep
/// the previous snapshot.
fn read_tail(path: &Path, offset: &mut u64, seeded: bool) -> Option<TailRead> {
    use std::io::{Read, Seek, SeekFrom};

    let len = std::fs::metadata(path).ok()?.len();

    // Initial load, or the file shrank: re-seed from the tail.
    if !seeded || len < *offset {
        let content = std::fs::read_to_string(path).ok()?;
        let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
        let n = lines.len();
        if n > TAIL_LINES {
            lines.drain(0..n - TAIL_LINES);
        }
        *offset = len;
        return Some(TailRead::Reset(lines));
    }

    if len == *offset {
        return Some(TailRead::Idle);
    }

    // Read only the bytes appended since last time, up to the last newline so a
    // half-written final line waits for its completion.
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(*offset)).ok()?;
    let mut buf: Vec<u8> = Vec::new();
    file.take(len - *offset).read_to_end(&mut buf).ok()?;
    let consumed = match buf.iter().rposition(|&b| b == b'\n') {
        Some(i) => i + 1,
        None => return Some(TailRead::Idle),
    };
    *offset += consumed as u64;
    let lines: Vec<String> = String::from_utf8_lossy(&buf[..consumed])
        .lines()
        .map(str::to_string)
        .collect();
    Some(TailRead::Append(lines))
}

/// Split `text` into spans, tinting case-insensitive matches of `needle`
/// (already lowercased) with `hit` and the rest with `base`. ASCII-case
/// folding keeps byte offsets valid (length-preserving).
fn highlight_spans(text: &str, needle: &str, base: Hsla, hit: Hsla) -> Vec<Div> {
    let span = |color: Hsla, s: &str| {
        div()
            .flex_none()
            .text_color(color)
            .child(SharedString::from(s.to_string()))
    };
    if needle.is_empty() {
        return vec![span(base, text)];
    }
    let hay = text.to_ascii_lowercase();
    let mut spans: Vec<Div> = Vec::new();
    let mut start = 0usize;
    while start <= text.len() {
        match hay[start..].find(needle) {
            Some(rel) => {
                let m = start + rel;
                let end = m + needle.len();
                if m > start {
                    spans.push(span(base, &text[start..m]));
                }
                spans.push(span(hit, &text[m..end]));
                start = end;
            }
            None => {
                if start < text.len() {
                    spans.push(span(base, &text[start..]));
                }
                break;
            }
        }
    }
    if spans.is_empty() {
        spans.push(span(base, text));
    }
    spans
}
