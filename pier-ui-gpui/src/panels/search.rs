// Search panel — remote code/content search over an SSH session.
//
// Flow: pick a saved connection (data::connections_raw) → connect off the
// render path (data::connect_blocking) → type a query and press Enter to run
// pier-core's code_search service (search_blocking) on a background task. Hits
// are grouped by file and listed mono-styled with the matched term highlighted.
// All blocking work (connect, search) runs via cx.background_executor so the
// render path never blocks; results land in View state and trigger cx.notify().

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Context, Div, FocusHandle, Hsla, KeyDownEvent, MouseButton,
    MouseDownEvent, SharedString, Window,
};
use gpui_component::{h_flex, v_flex};

use pier_core::services::code_search::{self, SearchEngine, SearchHit, SearchOpts, SearchOutput};
use pier_core::ssh::{SshConfig, SshSession};

use crate::data;
use crate::theme::Theme;
use crate::ui;

/// Hard cap on hits pulled back to the UI per search.
const MAX_HITS: usize = 500;

pub struct SearchPanel {
    theme: Theme,
    focus: FocusHandle,
    /// Saved SSH configs offered in the connection selector.
    conns: Vec<SshConfig>,
    /// Index into `conns` last acted on (selected / connecting / connected).
    selected: Option<usize>,
    /// Live session once a connect succeeds.
    session: Option<SshSession>,
    connecting: bool,
    conn_error: Option<String>,
    /// Live query input buffer.
    query: String,
    /// The query that produced `result` (used to highlight hits).
    last_query: String,
    searching: bool,
    search_error: Option<String>,
    result: Option<SearchOutput>,
    /// Bumped on every connect/search so stale background results are dropped.
    generation: u64,
    /// Set when a connect succeeds so render moves focus into the query box.
    focus_input_pending: bool,
}

impl SearchPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
            focus: cx.focus_handle(),
            conns: data::connections_raw(),
            selected: None,
            session: None,
            connecting: false,
            conn_error: None,
            query: String::new(),
            last_query: String::new(),
            searching: false,
            search_error: None,
            result: None,
            generation: 0,
            focus_input_pending: false,
        }
    }

    /// Open a blocking SSH session to `conns[idx]` on a background task.
    fn connect(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(cfg) = self.conns.get(idx).cloned() else {
            return;
        };
        self.selected = Some(idx);
        self.session = None;
        self.connecting = true;
        self.conn_error = None;
        self.result = None;
        self.search_error = None;
        self.generation += 1;
        let gen = self.generation;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { data::connect_blocking(&cfg) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != gen {
                    return; // superseded by a newer connect/search
                }
                this.connecting = false;
                match res {
                    Ok(session) => {
                        this.session = Some(session);
                        this.focus_input_pending = true;
                    }
                    Err(e) => this.conn_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Drop the active session and return to the connection selector.
    fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.session = None;
        self.result = None;
        self.search_error = None;
        self.generation += 1; // cancel any in-flight search
        cx.notify();
    }

    /// Run the current query against the live session on a background task.
    fn run_search(&mut self, cx: &mut Context<Self>) {
        let query = self.query.trim().to_string();
        if query.is_empty() {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.searching = true;
        self.search_error = None;
        self.result = None;
        self.last_query = query.clone();
        self.generation += 1;
        let gen = self.generation;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let opts = SearchOpts {
                cwd: String::new(), // empty → $HOME server-side
                query,
                case_insensitive: true,
                regex: false,
                whole_word: false,
                glob: String::new(),
                max_hits: MAX_HITS,
            };
            let res = cx
                .background_executor()
                .spawn(async move { code_search::search_blocking(&session, opts) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != gen {
                    return; // superseded
                }
                this.searching = false;
                match res {
                    Ok(out) => this.result = Some(out),
                    Err(e) => this.search_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "enter" => {
                self.run_search(cx);
                return;
            }
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

    // ── Connection selector ──────────────────────────────────────
    fn conn_row(&self, cx: &mut Context<Self>, idx: usize, c: &SshConfig) -> impl IntoElement {
        let t = &self.theme;
        let selected = self.selected == Some(idx);
        let connecting = selected && self.connecting;
        let dot = if connecting { t.warn } else { t.muted };
        let addr = format!("{}@{}:{}", c.user, c.host, c.port);
        h_flex()
            .id(SharedString::from(format!("search-conn-{idx}")))
            .items_center()
            .gap(t.sp2)
            .h(px(40.0))
            .px(t.sp3)
            .when(selected, |d| d.bg(t.accent_dim))
            .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.connect(idx, cx);
                }),
            )
            .child(ui::status_dot(dot))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .overflow_hidden()
                            .text_color(if selected { t.ink } else { t.ink_2 })
                            .child(c.name.clone()),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(addr),
                    ),
            )
    }

    /// The compact bar shown while connected, or the full selector list.
    fn connection_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;

        if self.session.is_some() {
            if let Some(c) = self.selected.and_then(|i| self.conns.get(i)) {
                let addr = format!("{}@{}:{}", c.user, c.host, c.port);
                return h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .w_full()
                    .h(px(36.0))
                    .px(t.sp3)
                    .border_b_1()
                    .border_color(t.line)
                    .child(ui::status_dot(t.pos))
                    .child(
                        div()
                            .overflow_hidden()
                            .text_color(t.ink)
                            .child(c.name.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(addr),
                    )
                    .child(
                        div()
                            .id("search-disconnect")
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(22.0))
                            .h(px(22.0))
                            .rounded(t.radius_sm)
                            .hover(|s| s.bg(t.hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                                    this.disconnect(cx);
                                }),
                            )
                            .child(ui::icon("close", px(14.0), t.muted)),
                    )
                    .into_any_element();
            }
        }

        let mut col = v_flex().child(ui::section_label(t, format!("CONNECTIONS · {}", self.conns.len())));
        if self.conns.is_empty() {
            col = col.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child("No saved connections"),
            );
        } else {
            for (i, c) in self.conns.iter().enumerate() {
                col = col.child(self.conn_row(cx, i, c));
            }
        }
        if self.connecting {
            col = col.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child("Connecting…"),
            );
        }
        if let Some(err) = &self.conn_error {
            col = col.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_sm)
                    .text_color(t.neg)
                    .child(err.clone()),
            );
        }
        col.into_any_element()
    }

    // ── Query input ──────────────────────────────────────────────
    fn search_bar(&self, focused: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let caret = || div().flex_none().w(px(2.0)).h(px(15.0)).bg(t.accent);
        let content: AnyElement = if self.query.is_empty() {
            if focused {
                h_flex().items_center().child(caret()).into_any_element()
            } else {
                div().text_color(t.dim).child("Search code…").into_any_element()
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
            .id("search-input")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
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
            .px(t.sp3)
            .h(px(32.0))
            .rounded(t.radius_md)
            .bg(t.panel)
            .border_1()
            .border_color(if focused { t.accent } else { t.line })
            .child(ui::icon("search", px(14.0), t.muted))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(content),
            )
    }

    // ── Results ──────────────────────────────────────────────────
    fn file_header(&self, file: &str, count: usize) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .pt(t.sp3)
            .pb(t.sp1)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink_2)
                    .child(file.to_string()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(count.to_string()),
            )
    }

    fn hit_row(&self, h: &SearchHit) -> impl IntoElement {
        let t = &self.theme;
        let shown = h.text.trim().to_string();
        h_flex()
            .id(SharedString::from(format!("search-hit-{}-{}", h.file, h.line)))
            .items_start()
            .gap(t.sp2)
            .px(t.sp3)
            .py(px(1.0))
            .overflow_hidden()
            .hover(|s| s.bg(t.hover))
            .child(
                div()
                    .flex_none()
                    .min_w(px(30.0))
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(h.line.to_string()),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .children(highlight_spans(t, &shown, &self.last_query)),
            )
    }

    fn results_body(&self) -> AnyElement {
        let t = &self.theme;
        let note = |color: Hsla, text: String| {
            div()
                .px(t.sp3)
                .py(t.sp3)
                .text_size(t.fs_sm)
                .text_color(color)
                .child(text)
                .into_any_element()
        };

        if self.searching {
            return note(t.muted, "Searching…".to_string());
        }
        if let Some(err) = &self.search_error {
            return note(t.neg, err.clone());
        }
        let Some(out) = &self.result else {
            return ui::empty_state(t, "Type a query and press Enter").into_any_element();
        };

        match out.engine {
            SearchEngine::None => {
                return note(t.muted, "No search tool on remote (install ripgrep)".to_string());
            }
            SearchEngine::CwdMissing => {
                return note(t.neg, "Working directory not found".to_string());
            }
            _ => {}
        }
        if out.hits.is_empty() {
            return ui::empty_state(t, "No matches").into_any_element();
        }

        let mut col = v_flex().pb(t.sp3);
        if !out.cwd.is_empty() {
            col = col.child(
                div()
                    .px(t.sp3)
                    .pt(t.sp2)
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .overflow_hidden()
                    .child(out.cwd.clone()),
            );
        }
        for (file, hits) in group_hits(&out.hits) {
            col = col.child(self.file_header(file, hits.len()));
            for h in hits {
                col = col.child(self.hit_row(h));
            }
        }
        if out.truncated {
            col = col.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_sm)
                    .text_color(t.warn)
                    .child("Results truncated — refine your query"),
            );
        }
        col.into_any_element()
    }
}

impl Render for SearchPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        // Move focus into the query box once a connect succeeds.
        if self.focus_input_pending && self.session.is_some() {
            self.focus_input_pending = false;
            window.focus(&self.focus, cx);
        }

        let t = self.theme.clone();
        let meta = match &self.result {
            Some(r) if self.session.is_some() => {
                let n = r.hits.len();
                if r.truncated {
                    format!("{n}+ hits")
                } else if n == 1 {
                    "1 hit".to_string()
                } else {
                    format!("{n} hits")
                }
            }
            _ => String::new(),
        };

        let mut root = v_flex()
            .size_full()
            .child(ui::panel_header(&t, "search", "SEARCH", meta));

        if self.session.is_some() {
            let focused = self.focus.is_focused(window);
            root = root
                .child(self.connection_section(cx))
                .child(self.search_bar(focused, cx))
                .child(
                    div()
                        .id("search-results")
                        .flex_1()
                        .min_h(px(0.0))
                        .overflow_y_scroll()
                        .child(self.results_body()),
                );
        } else {
            root = root.child(
                div()
                    .id("search-conns")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(self.connection_section(cx)),
            );
        }
        root
    }
}

/// Group hits into consecutive same-file runs, preserving engine order.
fn group_hits(hits: &[SearchHit]) -> Vec<(&str, Vec<&SearchHit>)> {
    let mut groups: Vec<(&str, Vec<&SearchHit>)> = Vec::new();
    for h in hits {
        match groups.last_mut() {
            Some(g) if g.0 == h.file => g.1.push(h),
            _ => groups.push((h.file.as_str(), vec![h])),
        }
    }
    groups
}

/// Split `text` into spans, tinting case-insensitive matches of `query` with
/// the accent colour and the rest with `ink_2`. Literal (non-regex) match,
/// mirroring the panel's search mode. ASCII-case-insensitive; byte offsets stay
/// valid because `to_ascii_lowercase` preserves length and char boundaries.
fn highlight_spans(t: &Theme, text: &str, query: &str) -> Vec<Div> {
    let span = |color: Hsla, s: &str| {
        div()
            .flex_none()
            .text_color(color)
            .child(SharedString::from(s.to_string()))
    };
    let q = query.trim();
    if q.is_empty() {
        return vec![span(t.ink_2, text)];
    }
    let hay = text.to_ascii_lowercase();
    let needle = q.to_ascii_lowercase();

    let mut spans: Vec<Div> = Vec::new();
    let mut start = 0usize;
    while start <= text.len() {
        match hay[start..].find(&needle) {
            Some(rel) => {
                let m = start + rel;
                let end = m + needle.len();
                if m > start {
                    spans.push(span(t.ink_2, &text[start..m]));
                }
                spans.push(span(t.accent, &text[m..end]));
                start = end;
            }
            None => {
                if start < text.len() {
                    spans.push(span(t.ink_2, &text[start..]));
                }
                break;
            }
        }
    }
    if spans.is_empty() {
        spans.push(span(t.ink_2, text));
    }
    spans
}
