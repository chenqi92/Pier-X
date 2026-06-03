// Logs panel — live tail of Pier-X's own log file.
//
// The active file comes from `pier_core::logging::log_file_path()` (set when
// the runtime initialises the file logger; `None` means logging was never
// started, so there is nothing to show). A controlled ~1s loop — modelled on
// shell.rs::monitor_panel — reads the last `TAIL_LINES` lines on the
// background executor (never in render), stores them on the view, and repaints
// only when the content actually changed. Lines are coloured by their
// `[LEVEL]` tag, drawn in the mono face, and the body stays pinned to the
// newest line at the bottom.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::prelude::*;
use gpui::{div, px, Context, Hsla, ScrollHandle, Window};
use gpui_component::v_flex;

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

pub struct LogsPanel {
    theme: Theme,
    /// The active log file, or `None` if logging was never initialised.
    path: Option<PathBuf>,
    /// The most recent tail (oldest first, newest last).
    lines: Vec<String>,
    /// Whether the first background read has resolved (distinguishes the
    /// initial "reading" state from a genuinely empty log).
    loaded: bool,
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
            loaded: false,
            scroll: ScrollHandle::new(),
        }
    }

    /// Re-read the tail every ~1s on the background executor while the view is
    /// alive. The loop ends when the entity is dropped (`update` fails). We
    /// read immediately on entry, then wait between passes, and only notify
    /// when the visible content changed so an idle log doesn't repaint or yank
    /// the user's scroll position.
    fn start_tail(cx: &mut Context<Self>, path: PathBuf) {
        cx.spawn(async move |this, cx| loop {
            let p = path.clone();
            let snapshot = cx
                .background_executor()
                .spawn(async move { read_tail(&p, TAIL_LINES) })
                .await;
            let alive = this
                .update(cx, |this, cx| {
                    let Some(lines) = snapshot else {
                        // Transient read error (e.g. file briefly gone): keep
                        // the last good snapshot, just leave the initial state.
                        if !this.loaded {
                            this.loaded = true;
                            cx.notify();
                        }
                        return;
                    };
                    let first = !this.loaded;
                    let changed = this.lines != lines;
                    this.lines = lines;
                    this.loaded = true;
                    if changed || first {
                        this.scroll.scroll_to_bottom();
                        cx.notify();
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
            cx.background_executor().timer(Duration::from_secs(1)).await;
        })
        .detach();
    }

    /// One log line: mono, sized small, tinted by its level.
    fn line_row(&self, raw: &str) -> impl IntoElement {
        let t = &self.theme;
        div()
            .w_full()
            .px(t.sp3)
            .py(px(2.0))
            .font_family(t.mono.clone())
            .text_size(t.fs_sm)
            .text_color(Level::parse(raw).color(t))
            .child(raw.to_string())
    }
}

impl Render for LogsPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;

        // No log file → logging was never initialised; nothing to tail.
        if self.path.is_none() {
            return v_flex()
                .size_full()
                .child(ui::panel_header(t, "scroll-text", "LOGS", ""))
                .child(ui::empty_state(t, "No log file"));
        }

        // First read hasn't resolved yet.
        if !self.loaded {
            return v_flex()
                .size_full()
                .child(ui::panel_header(t, "scroll-text", "LOGS", ""))
                .child(ui::empty_state(t, "Reading log…"));
        }

        let meta = format!("{} lines", self.lines.len());

        if self.lines.is_empty() {
            return v_flex()
                .size_full()
                .child(ui::panel_header(t, "scroll-text", "LOGS", meta))
                .child(ui::empty_state(t, "Log is empty"));
        }

        let mut col = v_flex().w_full().py(t.sp1);
        for line in &self.lines {
            col = col.child(self.line_row(line));
        }

        v_flex()
            .size_full()
            .child(ui::panel_header(t, "scroll-text", "LOGS", meta))
            .child(
                div()
                    .id("logs-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(col),
            )
    }
}

/// Read the last `max` lines of `path`. Blocking — run only on the background
/// executor. Returns `None` if the file can't be read so the caller can keep
/// the previous snapshot instead of clearing the view.
fn read_tail(path: &Path, max: usize) -> Option<Vec<String>> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let len = lines.len();
    if len > max {
        lines.drain(0..len - max);
    }
    Some(lines)
}
