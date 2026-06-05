// Pier-X GPUI spike — real terminal view (M2 + M3).
//
// Spawns a local shell through pier-core's PierTerminal (the same emulator/PTY
// the Tauri app uses) and paints its GridSnapshot directly with GPUI. Output
// wakes us through the C-FFI notify callback (sets a dirty flag); a per-frame
// poll task pulls a fresh snapshot when dirty. Key events are mapped to bytes
// and written back to the PTY. See docs/GPUI-MIGRATION-PLAN.md.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc;
use gpui::prelude::*;
use gpui::{
    div, px, App, Context, Div, FocusHandle, Focusable, FontWeight, Hsla, KeyDownEvent, Keystroke,
    MouseButton, MouseDownEvent, ScrollDelta, ScrollWheelEvent, SharedString, Window,
};
use gpui_component::v_flex;

use pier_core::ssh::{SshConfig, SshSession};
use pier_core::terminal::{Color, GridSnapshot, PierTerminal};

use crate::data;
use crate::theme::Theme;

// Initial grid; replaced on first paint once the real viewport is known.
const COLS: u16 = 100;
const ROWS: u16 = 32;
// Monospace cell metrics relative to font size. CHAR_RATIO is the advance
// width of the mono font (Consolas ≈ 0.55–0.6 em); LINE_RATIO is the line
// height we pin explicitly so row math matches what we paint.
const CHAR_RATIO: f32 = 0.6;
const LINE_RATIO: f32 = 1.35;

/// C-FFI notify callback handed to PierTerminal. Runs on the PTY reader thread.
/// `user_data` is `Arc::as_ptr` of the view's `dirty` flag, which the view
/// keeps alive and which outlives the terminal (drop order: `term` before
/// `dirty`). We only ever read the AtomicBool — no refcount touch.
extern "C" fn notify_dirty(user_data: *mut c_void, _event: u32) {
    if user_data.is_null() {
        return;
    }
    let flag = unsafe { &*(user_data as *const AtomicBool) };
    flag.store(true, Ordering::Release);
}

pub struct TerminalView {
    // Declared first so it drops (and joins its reader thread) before `session`
    // and `dirty`: the PTY borrows the SSH channel and the notify callback reads
    // the dirty flag, so both must outlive the terminal.
    term: Option<PierTerminal>,
    /// Kept alive for the lifetime of an SSH terminal so its shell channel stays
    /// open; `None` for local terminals.
    session: Option<SshSession>,
    snapshot: Option<GridSnapshot>,
    dirty: Arc<AtomicBool>,
    theme: Theme,
    focus: FocusHandle,
    did_focus: bool,
    error: Option<String>,
    /// Pre-ready message (e.g. "Connecting to host…") shown until `term` exists.
    status: Option<String>,
    cols: u16,
    rows: u16,
    scroll_offset: usize,
    /// Smart-mode captured at construction (`data::smart_mode()`): drives the
    /// OSC 133 shell-integration init and gates the completion / syntax /
    /// autosuggest overlays. Fixed for the terminal's lifetime — toggling the
    /// setting affects terminals opened afterwards.
    smart: bool,
}

impl TerminalView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let dirty = Arc::new(AtomicBool::new(true));
        let focus = cx.focus_handle();
        let smart = data::smart_mode();

        // Smart mode launches the shell with OSC 133 / OSC 7 integration so the
        // emulator exposes prompt_end / awaiting_input / cwd for the overlays.
        // On Windows this is a no-op in pier-core (no cmd.exe OSC 133) and falls
        // back to a plain shell — the overlays simply never activate.
        let (term, error) = match PierTerminal::new_with_smart(
            COLS,
            ROWS,
            "powershell.exe",
            smart,
            notify_dirty,
            Arc::as_ptr(&dirty) as *mut c_void,
        ) {
            Ok(t) => (Some(t), None),
            Err(e) => (None, Some(format!("failed to start shell: {e}"))),
        };

        Self::spawn_poll(cx);

        Self {
            term,
            session: None,
            snapshot: None,
            dirty,
            theme: Theme::dark(),
            focus,
            did_focus: false,
            error,
            status: None,
            cols: COLS,
            rows: ROWS,
            scroll_offset: 0,
            smart,
        }
    }

    /// A terminal backed by an SSH shell channel to `cfg`. The connect + channel
    /// open run on the background executor; the `PierTerminal` is built on the
    /// main thread once the channel is ready (so it binds to this view's dirty
    /// flag). Until then the view shows a "Connecting…" status.
    ///
    /// `prompt_tx` carries the interactive host-key prompt: an unknown or
    /// changed key sends a request to the shell overlay and blocks this connect
    /// task (on the background thread) until the user decides. Known / trusted
    /// hosts connect silently. See [`data::connect_blocking_prompt`].
    pub fn new_ssh(
        cx: &mut Context<Self>,
        cfg: SshConfig,
        prompt_tx: mpsc::UnboundedSender<data::HostKeyPrompt>,
    ) -> Self {
        let dirty = Arc::new(AtomicBool::new(true));
        let focus = cx.focus_handle();
        let label = format!("{}@{}", cfg.user, cfg.host);
        let smart = data::smart_mode();

        Self::spawn_poll(cx);

        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let session = data::connect_blocking_prompt(&cfg, prompt_tx)?;
                    let pty = session
                        .open_shell_channel_blocking(COLS, ROWS)
                        .map_err(|e| e.to_string())?;
                    Ok::<_, String>((session, pty))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok((session, pty)) => {
                        match PierTerminal::with_pty(
                            Box::new(pty),
                            COLS,
                            ROWS,
                            notify_dirty,
                            Arc::as_ptr(&this.dirty) as *mut c_void,
                        ) {
                            Ok(mut term) => {
                                // Sync to whatever size the live viewport settled on.
                                let _ = term.resize(this.cols, this.rows);
                                // Install OSC 133 / OSC 7 integration on the remote
                                // shell so prompt_end / awaiting_input / cwd populate
                                // for the smart-mode overlays.
                                if this.smart {
                                    let _ = term
                                        .write(&pier_core::terminal::smart::remote_init_payload());
                                }
                                this.term = Some(term);
                                this.session = Some(session);
                                this.status = None;
                                this.dirty.store(true, Ordering::Release);
                            }
                            Err(e) => {
                                this.error = Some(format!("failed to start remote shell: {e}"))
                            }
                        }
                    }
                    Err(e) => this.error = Some(format!("connect failed: {e}")),
                }
                cx.notify();
            });
        })
        .detach();

        Self {
            term: None,
            session: None,
            snapshot: None,
            dirty,
            theme: Theme::dark(),
            focus,
            did_focus: false,
            error: None,
            status: Some(format!("Connecting to {label}…")),
            cols: COLS,
            rows: ROWS,
            scroll_offset: 0,
            smart,
        }
    }

    /// Per-frame poll: when the reader thread flagged new output, pull a fresh
    /// snapshot and request a repaint. Coalesced to one snapshot copy per ~16ms.
    fn spawn_poll(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let alive = this
                .update(cx, |this, cx| {
                    if this.term.is_some() && this.dirty.swap(false, Ordering::AcqRel) {
                        let offset = this.scroll_offset;
                        if let Some(term) = &this.term {
                            let snap = term.snapshot_view(offset);
                            this.snapshot = Some(snap);
                        }
                        cx.notify();
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        })
        .detach();
    }

    /// Recompute the grid size from the live viewport and resize the PTY when
    /// it changed. Derives the terminal area deterministically from the window
    /// content size minus the surrounding chrome (tool rail + sidebar + tab bar
    /// + status bar + padding). Couples to the shell layout in shell.rs — fine
    /// for the spike; a measured-bounds approach is the eventual fix.
    fn fit_to(&mut self, window: &Window) {
        let fs = f32::from(self.theme.fs_body);
        let trw = f32::from(self.theme.toolrail_w);
        let sbw = f32::from(self.theme.sidebar_w);
        let tbh = f32::from(self.theme.tabbar_h);
        let sbh = f32::from(self.theme.statusbar_h);
        let pad = f32::from(self.theme.sp3) * 2.0;

        let vp = window.viewport_size();
        let char_w = fs * CHAR_RATIO;
        let line_h = (fs * LINE_RATIO).round();
        let avail_w = f32::from(vp.width) - trw - sbw - 2.0 - pad;
        let avail_h = f32::from(vp.height) - tbh - sbh - 2.0 - pad;

        let cols = ((avail_w / char_w).floor() as i32).clamp(20, 400) as u16;
        let rows = ((avail_h / line_h).floor() as i32).clamp(5, 200) as u16;
        if (cols, rows) != (self.cols, self.rows) {
            self.cols = cols;
            self.rows = rows;
            if let Some(term) = &mut self.term {
                let _ = term.resize(cols, rows);
            }
            self.dirty.store(true, Ordering::Release);
        }
    }

    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// The live SSH session backing this terminal, or `None` for a local shell.
    /// `SshSession` is `Clone` (Arc over a multiplexed russh connection), so the
    /// returned handle shares the one connection the interactive shell uses —
    /// reuse it for service detection / port forwarding instead of dialing again.
    #[allow(dead_code)]
    pub fn session(&self) -> Option<SshSession> {
        self.session.clone()
    }

    /// Feed `text` to the PTY as if it were typed. Used by the Broadcast
    /// dialog to fan one command into many SSH sessions at once. No-op
    /// until the shell channel is ready (a still-connecting tab is skipped).
    pub fn send_input(&mut self, text: &str) {
        // Broadcast must never write a local terminal: only accept fanned-in
        // input on tabs backed by a live SSH session (defends D3 alongside the
        // shell's live-target derivation and the dialog's own per-target gate).
        if self.session.is_none() {
            return;
        }
        if let Some(term) = &self.term {
            let _ = term.write(text.as_bytes());
            // Nudge the poll loop so the echoed input shows promptly.
            self.dirty.store(true, Ordering::Release);
        }
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let line_h = (f32::from(self.theme.fs_body) * LINE_RATIO).max(1.0);
        let dy = match ev.delta {
            ScrollDelta::Lines(p) => p.y,
            ScrollDelta::Pixels(p) => f32::from(p.y) / line_h,
        };
        // dy > 0 → wheel up → reveal older scrollback → larger offset.
        let step = dy.round() as i64;
        if step == 0 {
            return;
        }
        let next = (self.scroll_offset as i64 + step).max(0) as usize;
        if next != self.scroll_offset {
            self.scroll_offset = next;
            self.dirty.store(true, Ordering::Release);
            cx.notify();
        }
    }

    fn fg_of(&self, c: &Color) -> Hsla {
        match c {
            Color::Default => self.theme.ink,
            Color::Indexed(n) => ansi_color(*n),
            Color::Rgb(r, g, b) => rgb_u8(*r, *g, *b),
        }
    }

    fn bg_of(&self, c: &Color) -> Option<Hsla> {
        match c {
            Color::Default => None,
            Color::Indexed(n) => Some(ansi_color(*n)),
            Color::Rgb(r, g, b) => Some(rgb_u8(*r, *g, *b)),
        }
    }

    /// One styled run of same-attribute cells → a text span.
    fn span(&self, text: String, fg: Hsla, bg: Option<Hsla>, bold: bool) -> Div {
        let mut d = div().text_color(fg);
        if let Some(b) = bg {
            d = d.bg(b);
        }
        if bold {
            d = d.font_weight(FontWeight::BOLD);
        }
        d.child(SharedString::from(text))
    }

    /// Build one grid row, coalescing adjacent same-style cells into runs and
    /// rendering the cursor cell inverted.
    fn build_row(&self, snap: &GridSnapshot, r: usize) -> Div {
        let cols = snap.cols as usize;
        let base = r * cols;
        let cursor_here = snap.cursor_y as usize == r;
        let cur_x = snap.cursor_x as usize;

        let mut spans: Vec<Div> = Vec::new();
        let mut i = 0;
        while i < cols {
            // Cursor cell: its own inverted single-char span.
            if cursor_here && i == cur_x {
                let ch = glyph(snap.cells[base + i].ch);
                spans.push(
                    div()
                        .bg(self.theme.ink)
                        .text_color(self.theme.bg)
                        .child(SharedString::from(ch.to_string())),
                );
                i += 1;
                continue;
            }

            let cell0 = &snap.cells[base + i];
            let fg = self.fg_of(&cell0.fg);
            let bg = self.bg_of(&cell0.bg);
            let bold = cell0.bold;

            let mut run = String::new();
            while i < cols {
                if cursor_here && i == cur_x {
                    break;
                }
                let cell = &snap.cells[base + i];
                if self.fg_of(&cell.fg) != fg
                    || self.bg_of(&cell.bg) != bg
                    || cell.bold != bold
                {
                    break;
                }
                run.push(glyph(cell.ch));
                i += 1;
            }
            spans.push(self.span(run, fg, bg, bold));
        }

        div().flex().flex_row().children(spans)
    }

    fn grid(&self, snap: &GridSnapshot) -> Div {
        let rows: Vec<Div> = (0..snap.rows as usize)
            .map(|r| self.build_row(snap, r))
            .collect();
        v_flex().children(rows)
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let bytes = keystroke_to_bytes(&ev.keystroke);
        if bytes.is_empty() {
            return;
        }
        if let Some(term) = &self.term {
            let _ = term.write(&bytes);
        }
        // Nudge the poll loop to refresh promptly even if the backend notify
        // hasn't fired yet.
        self.dirty.store(true, Ordering::Release);
        cx.notify();
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        // Grab focus on first paint so typing works without a click.
        if !self.did_focus {
            self.did_focus = true;
            window.focus(&self.focus, cx);
        }
        self.fit_to(window);

        let t = self.theme.clone();
        let line_h = (f32::from(t.fs_body) * LINE_RATIO).round();
        let body = match (&self.error, &self.snapshot) {
            (Some(err), _) => div().text_color(t.neg).child(err.clone()).into_any_element(),
            (None, Some(snap)) => self.grid(snap).into_any_element(),
            (None, None) => div()
                .text_color(t.muted)
                .child(
                    self.status
                        .clone()
                        .unwrap_or_else(|| "starting shell…".to_string()),
                )
                .into_any_element(),
        };

        div()
            .track_focus(&self.focus)
            .key_context("PierTerminal")
            .on_key_down(cx.listener(Self::on_key))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                }),
            )
            .size_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(t.bg)
            .p(t.sp3)
            .font_family(t.mono.clone())
            .text_size(t.fs_body)
            .line_height(px(line_h))
            .text_color(t.ink)
            .child(body)
    }
}

/// Printable glyph for a cell char: blanks out NUL/control codes.
fn glyph(ch: char) -> char {
    if ch == '\0' || ch.is_control() {
        ' '
    } else {
        ch
    }
}

fn rgb_u8(r: u8, g: u8, b: u8) -> Hsla {
    gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | (b as u32)).into()
}

/// xterm 256-colour palette → Hsla. 0–15 base, 16–231 cube, 232–255 grayscale.
fn ansi_color(n: u8) -> Hsla {
    const BASE16: [u32; 16] = [
        0x000000, 0xcd3131, 0x2faf5b, 0xc7a23a, 0x3b6fd6, 0xa452c9, 0x2aa4b8, 0xd0d0d0, 0x686868,
        0xff5a5f, 0x3dd68c, 0xffb547, 0x4aa3ff, 0xc49eff, 0x56e0c8, 0xffffff,
    ];
    match n {
        0..=15 => gpui::rgb(BASE16[n as usize]).into(),
        16..=231 => {
            let v = n - 16;
            let steps = [0u8, 95, 135, 175, 215, 255];
            let r = steps[(v / 36 % 6) as usize];
            let g = steps[(v / 6 % 6) as usize];
            let b = steps[(v % 6) as usize];
            rgb_u8(r, g, b)
        }
        232..=255 => {
            let shade = 8 + (n - 232) * 10;
            rgb_u8(shade, shade, shade)
        }
    }
}

/// Map a GPUI keystroke to the bytes a PTY expects.
fn keystroke_to_bytes(ks: &Keystroke) -> Vec<u8> {
    let m = &ks.modifiers;

    // Ctrl + letter → control byte (Ctrl-C = 0x03, etc.)
    if m.control && !m.alt {
        if ks.key.chars().count() == 1 {
            let ch = ks.key.chars().next().unwrap();
            let lc = ch.to_ascii_lowercase();
            if lc.is_ascii_alphabetic() {
                return vec![(lc as u8) - b'a' + 1];
            }
            match ch {
                '[' => return vec![0x1b],
                ' ' => return vec![0],
                _ => {}
            }
        }
    }

    match ks.key.as_str() {
        "enter" => return vec![b'\r'],
        "backspace" => return vec![0x7f],
        "tab" => return vec![b'\t'],
        "escape" => return vec![0x1b],
        "up" => return vec![0x1b, b'[', b'A'],
        "down" => return vec![0x1b, b'[', b'B'],
        "right" => return vec![0x1b, b'[', b'C'],
        "left" => return vec![0x1b, b'[', b'D'],
        "home" => return vec![0x1b, b'[', b'H'],
        "end" => return vec![0x1b, b'[', b'F'],
        "delete" => return vec![0x1b, b'[', b'3', b'~'],
        "space" => return vec![b' '],
        _ => {}
    }

    // Printable text (respects shift / layout via key_char when present).
    if let Some(kc) = &ks.key_char {
        if !kc.is_empty() {
            return kc.as_bytes().to_vec();
        }
    }
    if ks.key.chars().count() == 1 {
        return ks.key.as_bytes().to_vec();
    }
    Vec::new()
}
