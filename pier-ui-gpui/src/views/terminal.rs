use std::{
    cell::RefCell,
    env,
    ffi::c_void,
    rc::Rc,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use gpui::{
    canvas, div, prelude::*, px, App, Bounds, ClipboardItem, Context, CursorStyle, EventEmitter,
    FocusHandle, Focusable, Hsla, IntoElement, KeyDownEvent, Keystroke, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, ScrollDelta, ScrollWheelEvent,
    SharedString, Size, TextRun, UnderlineStyle, WeakEntity, Window,
};
use gpui_component::{
    dock::{Panel, PanelControl, PanelEvent, TabPanel},
    Icon as UiIcon, IconName,
};
use pier_core::{
    settings::{TerminalCursorStyle, TerminalThemePreset},
    terminal::{Cell, Color as TerminalColor, GridSnapshot, NotifyEvent, PierTerminal},
};

use crate::{
    app::{route::Route, ActivationHandler},
    components::{terminal_grid, terminal_grid::LayoutState, StatusKind},
    theme::{
        spacing::SP_3,
        terminal::{
            terminal_bg_color, terminal_hex_color, terminal_indexed_hex, terminal_palette,
            TerminalPalette,
        },
        terminal_cursor_blink, terminal_font_for_family, terminal_font_size, theme,
        typography::{SIZE_CAPTION, WEIGHT_EMPHASIS, WEIGHT_REGULAR},
    },
};

const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 32;
const MIN_COLS: u16 = 64;
const MAX_COLS: u16 = 220;
const MIN_ROWS: u16 = 18;
const MAX_ROWS: u16 = 72;
const SCROLLBACK_LIMIT: usize = 20_000;
const BASE_TERMINAL_FONT_SIZE_PX: f32 = 13.0;
const BASE_CELL_WIDTH_PX: f32 = 8.2;
const BASE_CELL_HEIGHT_PX: f32 = 18.0;
const TERMINAL_MIN_HEIGHT: f32 = 220.0;
const WINDOW_CHROME_WIDTH: f32 = 356.0;
const WINDOW_CHROME_HEIGHT: f32 = 212.0;
const MAX_OSC52_CLIPBOARD_BYTES: usize = 1_000_000;
const BELL_FLASH_MS: u64 = 180;
const TERMINAL_REFRESH_MS: u64 = 33;
const TERMINAL_DIAGNOSTIC_INTERVAL_MS: u64 = 1_000;
const CURSOR_BLINK_MS: u64 = 530;
static NEXT_TERMINAL_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Clone)]
pub(crate) struct TerminalLine {
    pub(crate) text: SharedString,
    pub(crate) runs: Vec<TextRun>,
    pub(crate) cell_spans: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TerminalRunStyle {
    fg_hex: u32,
    bg_hex: u32,
    bold: bool,
    underline: bool,
    apply_background_opacity: bool,
}

#[derive(Clone, Copy)]
struct TerminalRun {
    style: TerminalRunStyle,
    len: usize,
    cell_span: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TerminalCellPosition {
    pub(crate) row: usize,
    pub(crate) col: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct TerminalSelection {
    pub(crate) anchor: TerminalCellPosition,
    pub(crate) head: TerminalCellPosition,
}

impl TerminalSelection {
    fn collapsed(position: TerminalCellPosition) -> Self {
        Self {
            anchor: position,
            head: position,
        }
    }

    fn is_empty(self) -> bool {
        self.anchor == self.head
    }

    pub(crate) fn normalized(self) -> (TerminalCellPosition, TerminalCellPosition) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TerminalSnapshotKey {
    generation: usize,
    scrollback_offset: usize,
    cols: u16,
    rows: u16,
}

#[derive(Clone, PartialEq, Eq)]
struct TerminalRenderKey {
    snapshot: TerminalSnapshotKey,
    selection: Option<TerminalSelection>,
    terminal_theme_preset: TerminalThemePreset,
    terminal_opacity_pct: u8,
    font_family: String,
    font_ligatures: bool,
    cursor_style: TerminalCursorStyle,
    cursor_visible: bool,
}

struct TerminalRenderCache {
    key: TerminalRenderKey,
    snapshot: GridSnapshot,
    lines: Vec<TerminalLine>,
}

struct TerminalDiagnostics {
    last_report_at: Instant,
    refresh_ticks: u32,
    notify_requests: u32,
    generation_changes: u32,
    render_calls: u32,
    resize_events: u32,
    window_bounds_events: u32,
    input_events: u32,
}

impl Default for TerminalDiagnostics {
    fn default() -> Self {
        Self {
            last_report_at: Instant::now(),
            refresh_ticks: 0,
            notify_requests: 0,
            generation_changes: 0,
            render_calls: 0,
            resize_events: 0,
            window_bounds_events: 0,
            input_events: 0,
        }
    }
}

impl TerminalDiagnostics {
    fn note_refresh_tick(&mut self) {
        self.refresh_ticks = self.refresh_ticks.saturating_add(1);
    }

    fn note_notify(&mut self) {
        self.notify_requests = self.notify_requests.saturating_add(1);
    }

    fn note_generation_change(&mut self) {
        self.generation_changes = self.generation_changes.saturating_add(1);
    }

    fn note_render(&mut self) {
        self.render_calls = self.render_calls.saturating_add(1);
    }

    fn note_resize(&mut self) {
        self.resize_events = self.resize_events.saturating_add(1);
    }

    fn note_window_bounds_event(&mut self) {
        self.window_bounds_events = self.window_bounds_events.saturating_add(1);
    }

    fn note_input(&mut self) {
        self.input_events = self.input_events.saturating_add(1);
    }

    fn maybe_report(
        &mut self,
        terminal_id: usize,
        active: bool,
        alive: bool,
        generation: usize,
        scrollback_offset: usize,
    ) {
        if self.last_report_at.elapsed() < Duration::from_millis(TERMINAL_DIAGNOSTIC_INTERVAL_MS) {
            return;
        }

        let suspicious = self.render_calls > 45
            && self.generation_changes == 0
            && (self.notify_requests > 10 || self.resize_events > 10);

        if suspicious {
            log::warn!(
                "terminal[{terminal_id}] hot-loop suspect active={active} alive={alive} generation={generation} renders={} refresh_ticks={} notifies={} generation_changes={} resize_events={} window_bounds={} inputs={} scrollback={scrollback_offset}",
                self.render_calls,
                self.refresh_ticks,
                self.notify_requests,
                self.generation_changes,
                self.resize_events,
                self.window_bounds_events,
                self.input_events,
            );
        } else if self.render_calls > 0
            || self.notify_requests > 0
            || self.generation_changes > 0
            || self.resize_events > 0
            || self.input_events > 0
        {
            log::info!(
                "terminal[{terminal_id}] stats active={active} alive={alive} generation={generation} renders={} refresh_ticks={} notifies={} generation_changes={} resize_events={} window_bounds={} inputs={} scrollback={scrollback_offset}",
                self.render_calls,
                self.refresh_ticks,
                self.notify_requests,
                self.generation_changes,
                self.resize_events,
                self.window_bounds_events,
                self.input_events,
            );
        }

        self.last_report_at = Instant::now();
        self.refresh_ticks = 0;
        self.notify_requests = 0;
        self.generation_changes = 0;
        self.render_calls = 0;
        self.resize_events = 0;
        self.window_bounds_events = 0;
        self.input_events = 0;
    }
}

#[derive(Default)]
struct NotifyState {
    generation: AtomicUsize,
    exited: AtomicBool,
}

extern "C" fn terminal_notify(user_data: *mut c_void, event: u32) {
    if user_data.is_null() {
        return;
    }

    // SAFETY: `user_data` always points at `TerminalPanel::notify_state`.
    let state = unsafe { &*(user_data as *const NotifyState) };
    state.generation.fetch_add(1, Ordering::Relaxed);
    if event == NotifyEvent::Exited as u32 {
        state.exited.store(true, Ordering::Relaxed);
    }
}

pub struct TerminalPanel {
    terminal_id: usize,
    focus_handle: FocusHandle,
    on_activated: ActivationHandler,
    shell_path: SharedString,
    terminal: Option<PierTerminal>,
    last_error: Option<SharedString>,
    terminal_title: Option<SharedString>,
    applied_window_title: Option<SharedString>,
    ssh_target: Option<SharedString>,
    bell_flash_until: Option<Instant>,
    scrollback_offset: usize,
    refresh_loop_started: bool,
    resize_observer_started: bool,
    panel_active: bool,
    surface_bounds: Rc<RefCell<Option<Bounds<Pixels>>>>,
    last_surface_size_key: Option<(i32, i32)>,
    selection: Option<TerminalSelection>,
    selection_dragging: bool,
    render_cache: Option<TerminalRenderCache>,
    terminal_font_size_px: f32,
    cell_width_px: f32,
    cell_height_px: f32,
    terminal_cursor_style: TerminalCursorStyle,
    terminal_cursor_blink: bool,
    terminal_theme_preset: TerminalThemePreset,
    terminal_opacity_pct: u8,
    terminal_font_ligatures: bool,
    cursor_blink_anchor: Instant,
    last_cursor_visible: bool,
    diagnostics: TerminalDiagnostics,
    notify_state: Box<NotifyState>,
}

impl TerminalPanel {
    /// Public escape hatch for callers (e.g. the Servers list) that want to
    /// shove a command into the PTY right after creating a tab. Bytes go
    /// through the same `write_input` path as keystrokes, so PTY back-pressure
    /// + error reporting behave identically.
    pub fn send_input(&mut self, s: &str, cx: &mut Context<Self>) {
        if self.write_input(s.as_bytes()) {
            cx.notify();
        }
    }

    pub fn new(on_activated: ActivationHandler, cx: &mut Context<Self>) -> Self {
        let shell_path: SharedString = preferred_shell().into();
        let notify_state = Box::<NotifyState>::default();
        let terminal_id = NEXT_TERMINAL_ID.fetch_add(1, Ordering::Relaxed);

        let mut panel = Self {
            terminal_id,
            focus_handle: cx.focus_handle(),
            on_activated,
            shell_path,
            terminal: None,
            last_error: None,
            terminal_title: None,
            applied_window_title: None,
            ssh_target: None,
            bell_flash_until: None,
            scrollback_offset: 0,
            refresh_loop_started: false,
            resize_observer_started: false,
            panel_active: false,
            surface_bounds: Rc::new(RefCell::new(None)),
            last_surface_size_key: None,
            selection: None,
            selection_dragging: false,
            render_cache: None,
            terminal_font_size_px: BASE_TERMINAL_FONT_SIZE_PX,
            cell_width_px: BASE_CELL_WIDTH_PX,
            cell_height_px: BASE_CELL_HEIGHT_PX,
            terminal_cursor_style: TerminalCursorStyle::Block,
            terminal_cursor_blink: true,
            terminal_theme_preset: TerminalThemePreset::DefaultDark,
            terminal_opacity_pct: 100,
            terminal_font_ligatures: false,
            cursor_blink_anchor: Instant::now(),
            last_cursor_visible: true,
            diagnostics: TerminalDiagnostics::default(),
            notify_state,
        };
        log::info!(
            "terminal[{terminal_id}] panel created shell={}",
            panel.shell_path
        );
        panel.start_terminal((DEFAULT_COLS, DEFAULT_ROWS));
        panel
    }

    fn start_terminal(&mut self, size: (u16, u16)) {
        let user_data = self.notify_state.as_mut() as *mut NotifyState as *mut c_void;
        match PierTerminal::new(size.0, size.1, &self.shell_path, terminal_notify, user_data) {
            Ok(term) => {
                term.set_scrollback_limit(SCROLLBACK_LIMIT);
                self.notify_state.generation.store(0, Ordering::Relaxed);
                self.notify_state.exited.store(false, Ordering::Relaxed);
                self.scrollback_offset = 0;
                self.last_error = None;
                self.terminal_title = None;
                self.applied_window_title = None;
                self.ssh_target = None;
                self.bell_flash_until = None;
                self.render_cache = None;
                self.reset_cursor_blink();
                self.terminal = Some(term);
                self.clamp_selection_to_terminal();
                log::info!(
                    "terminal[{}] started shell={} size={}x{}",
                    self.terminal_id,
                    self.shell_path,
                    size.0,
                    size.1
                );
            }
            Err(err) => {
                self.last_error = Some(
                    format!("Failed to start terminal with `{}`: {err}", self.shell_path).into(),
                );
                self.terminal_title = None;
                self.ssh_target = None;
                self.bell_flash_until = None;
                self.render_cache = None;
                self.last_cursor_visible = false;
                self.terminal = None;
                log::error!(
                    "terminal[{}] failed to start shell={}: {err}",
                    self.terminal_id,
                    self.shell_path
                );
            }
        }
    }

    fn sync_terminal_preferences(
        &mut self,
        font_size_px: f32,
        cursor_style: TerminalCursorStyle,
        cursor_blink: bool,
        theme_preset: TerminalThemePreset,
        opacity_pct: u8,
        font_ligatures: bool,
    ) -> bool {
        let normalized_font_size = font_size_px.clamp(10.0, 24.0);
        let cell_width = terminal_cell_width_px(normalized_font_size);
        let cell_height = terminal_cell_height_px(normalized_font_size);
        let changed = (self.terminal_font_size_px - normalized_font_size).abs() > f32::EPSILON
            || (self.cell_width_px - cell_width).abs() > f32::EPSILON
            || (self.cell_height_px - cell_height).abs() > f32::EPSILON
            || self.terminal_cursor_style != cursor_style
            || self.terminal_cursor_blink != cursor_blink
            || self.terminal_theme_preset != theme_preset
            || self.terminal_opacity_pct != opacity_pct
            || self.terminal_font_ligatures != font_ligatures;

        self.terminal_font_size_px = normalized_font_size;
        self.cell_width_px = cell_width;
        self.cell_height_px = cell_height;
        self.terminal_theme_preset = theme_preset;
        self.terminal_opacity_pct = opacity_pct;
        self.terminal_font_ligatures = font_ligatures;
        if self.terminal_cursor_style != cursor_style || self.terminal_cursor_blink != cursor_blink
        {
            self.terminal_cursor_style = cursor_style;
            self.terminal_cursor_blink = cursor_blink;
            self.reset_cursor_blink();
        }

        changed
    }

    fn reset_cursor_blink(&mut self) {
        self.cursor_blink_anchor = Instant::now();
        self.last_cursor_visible = true;
    }

    fn clear_selection(&mut self) -> bool {
        let had_selection = self.selection.is_some();
        self.selection = None;
        self.selection_dragging = false;
        had_selection
    }

    fn cursor_visible(&self) -> bool {
        let Some(term) = self.terminal.as_ref() else {
            return false;
        };
        if !term.is_alive() || self.scrollback_offset != 0 {
            return false;
        }
        if !self.terminal_cursor_blink {
            return true;
        }

        let elapsed_ms = Instant::now()
            .saturating_duration_since(self.cursor_blink_anchor)
            .as_millis() as u64;
        ((elapsed_ms / CURSOR_BLINK_MS) % 2) == 0
    }

    fn terminal_opacity(&self) -> f32 {
        f32::from(self.terminal_opacity_pct) / 100.0
    }

    fn ensure_refresh_loop(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.refresh_loop_started {
            return;
        }
        self.refresh_loop_started = true;
        let this = cx.entity().downgrade();

        window
            .spawn(cx, async move |cx| {
                let mut seen_generation = 0usize;

                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(TERMINAL_REFRESH_MS))
                        .await;

                    let still_alive = this
                        .update_in(cx, |this, window, cx| {
                            this.diagnostics.note_refresh_tick();
                            let current = this.notify_state.generation.load(Ordering::Relaxed);
                            let mut should_notify = false;
                            if this.sync_surface_resize(window, cx) {
                                this.diagnostics.note_resize();
                                should_notify = true;
                            }
                            if current != seen_generation {
                                seen_generation = current;
                                this.diagnostics.note_generation_change();
                                this.handle_terminal_side_effects(cx);
                                // Window title needs the PTY title / ssh
                                // target picked up by handle_terminal_side_effects;
                                // do it here (not in render — Phase 10 perf).
                                this.sync_window_title(window);
                                should_notify = true;
                            }
                            if this.update_transient_state() {
                                should_notify = true;
                            }
                            if should_notify {
                                this.diagnostics.note_notify();
                                cx.notify();
                            }
                            let alive = this
                                .terminal
                                .as_ref()
                                .map(PierTerminal::is_alive)
                                .unwrap_or(false);
                            this.diagnostics.maybe_report(
                                this.terminal_id,
                                this.panel_active,
                                alive,
                                current,
                                this.scrollback_offset,
                            );
                        })
                        .is_ok();

                    if !still_alive {
                        break;
                    }
                }
            })
            .detach();
    }

    fn ensure_resize_observer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.resize_observer_started {
            return;
        }
        self.resize_observer_started = true;

        cx.observe_window_bounds(window, |this, window, cx| {
            this.diagnostics.note_window_bounds_event();
            if this.resize_for_window(window, cx) {
                this.diagnostics.note_resize();
                cx.notify();
            }
        })
        .detach();
    }

    fn resize_for_window(&mut self, window: &Window, cx: &App) -> bool {
        self.sync_terminal_preferences(
            terminal_font_size(cx),
            theme(cx).settings.terminal_cursor_style,
            terminal_cursor_blink(cx),
            theme(cx).settings.terminal_theme_preset,
            theme(cx).settings.terminal_opacity_pct,
            theme(cx).settings.terminal_font_ligatures,
        );

        let Some(term) = self.terminal.as_mut() else {
            return false;
        };

        let measured_bounds = *self.surface_bounds.borrow();
        let (width, height) = if let Some(bounds) = measured_bounds {
            (f32::from(bounds.size.width), f32::from(bounds.size.height))
        } else {
            // Before the terminal surface is painted, fall back to the window viewport.
            let viewport = window.viewport_size();
            (
                (f32::from(viewport.width) - WINDOW_CHROME_WIDTH)
                    .max(self.cell_width_px * MIN_COLS as f32),
                (f32::from(viewport.height) - WINDOW_CHROME_HEIGHT)
                    .max(self.cell_height_px * MIN_ROWS as f32),
            )
        };

        let cols = (width / self.cell_width_px)
            .floor()
            .clamp(MIN_COLS as f32, MAX_COLS as f32) as u16;
        let rows = (height / self.cell_height_px)
            .floor()
            .clamp(MIN_ROWS as f32, MAX_ROWS as f32) as u16;

        if term.size() == (cols, rows) {
            return false;
        }

        match term.resize(cols, rows) {
            Ok(()) => {
                self.clamp_scrollback();
                self.clamp_selection_to_terminal();
                true
            }
            Err(err) => {
                self.last_error =
                    Some(format!("Failed to resize terminal to {cols}x{rows}: {err}").into());
                log::warn!(
                    "terminal[{}] resize failed target={}x{}: {err}",
                    self.terminal_id,
                    cols,
                    rows
                );
                true
            }
        }
    }

    fn current_surface_size_key(&self) -> Option<(i32, i32)> {
        let bounds = (*self.surface_bounds.borrow())?;
        Some((
            f32::from(bounds.size.width).round() as i32,
            f32::from(bounds.size.height).round() as i32,
        ))
    }

    fn sync_surface_resize(&mut self, window: &Window, cx: &App) -> bool {
        let surface_size_key = self.current_surface_size_key();
        if surface_size_key == self.last_surface_size_key {
            return false;
        }

        self.last_surface_size_key = surface_size_key;
        if surface_size_key.is_none() {
            return false;
        }

        self.resize_for_window(window, cx)
    }

    fn clamp_scrollback(&mut self) {
        if let Some(term) = self.terminal.as_ref() {
            self.scrollback_offset = self.scrollback_offset.min(term.scrollback_len());
        } else {
            self.scrollback_offset = 0;
        }
    }

    fn handle_terminal_side_effects(&mut self, cx: &mut Context<Self>) {
        self.clamp_scrollback();
        self.sync_terminal_title();
        self.sync_ssh_target();
        self.sync_bell_flash();
        self.apply_osc52_clipboard(cx);
        self.reset_cursor_blink();
    }

    fn update_transient_state(&mut self) -> bool {
        let mut changed = false;

        if self
            .bell_flash_until
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            self.bell_flash_until = None;
            changed = true;
        }

        let cursor_visible = self.cursor_visible();
        if self.last_cursor_visible != cursor_visible {
            self.last_cursor_visible = cursor_visible;
            changed = true;
        }

        changed
    }

    fn clamp_selection_to_terminal(&mut self) {
        let Some(term) = self.terminal.as_ref() else {
            self.selection = None;
            self.selection_dragging = false;
            return;
        };

        let (cols, rows) = term.size();
        if cols == 0 || rows == 0 {
            self.selection = None;
            self.selection_dragging = false;
            return;
        }

        if let Some(selection) = self.selection.as_mut() {
            let max_row = rows as usize - 1;
            let max_col = cols as usize - 1;
            selection.anchor.row = selection.anchor.row.min(max_row);
            selection.anchor.col = selection.anchor.col.min(max_col);
            selection.head.row = selection.head.row.min(max_row);
            selection.head.col = selection.head.col.min(max_col);
        }
    }

    fn on_terminal_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        if !self.surface_contains_point(event.position) {
            return;
        }
        self.focus_handle.focus(window);

        let Some(position) = self.selection_position_for_point(event.position, false) else {
            self.selection = None;
            self.selection_dragging = false;
            return;
        };

        if event.click_count >= 3 {
            self.selection = self.line_selection_at(position);
            self.selection_dragging = false;
            cx.notify();
            return;
        }

        if event.click_count >= 2 {
            self.selection = self.word_selection_at(position);
            self.selection_dragging = false;
            cx.notify();
            return;
        }

        if event.modifiers.shift {
            if let Some(selection) = self.selection.as_mut() {
                selection.head = position;
            } else {
                self.selection = Some(TerminalSelection::collapsed(position));
            }
        } else {
            self.selection = Some(TerminalSelection::collapsed(position));
        }
        self.selection_dragging = true;

        cx.notify();
    }

    fn on_terminal_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.surface_contains_point(event.position) {
            return;
        }
        if !self.selection_dragging || event.pressed_button != Some(MouseButton::Left) {
            return;
        }

        if self.update_selection_head(event.position, true) {
            cx.notify();
        }
    }

    fn on_terminal_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_selection(event.position, false, cx);
    }

    fn on_terminal_mouse_up_out(
        &mut self,
        event: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_selection(event.position, true, cx);
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.surface_contains_point(event.position) {
            return;
        }
        let Some(term) = self.terminal.as_ref() else {
            return;
        };

        let delta = match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => f32::from(pixels.y) / self.cell_height_px,
        };
        let step = if delta.abs() < 1.0 {
            delta.signum() as isize
        } else {
            delta.round() as isize
        };
        if step == 0 {
            return;
        }

        let max_offset = term.scrollback_len();
        if step > 0 {
            self.scrollback_offset = (self.scrollback_offset + step as usize).min(max_offset);
        } else {
            self.scrollback_offset = self.scrollback_offset.saturating_sub(step.unsigned_abs());
        }

        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.has_selection() && wants_copy_selection(&event.keystroke) {
            self.copy_selection_to_clipboard(cx);
            cx.stop_propagation();
            return;
        }

        if wants_clipboard_paste(&event.keystroke) {
            let selection_cleared = self.clear_selection();
            if selection_cleared || self.paste_from_clipboard(cx) {
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }

        if let Some(bytes) = translate_keystroke(&event.keystroke) {
            let selection_cleared = self.clear_selection();
            if selection_cleared || self.write_input(&bytes) {
                cx.notify();
            }
        }

        cx.stop_propagation();
    }

    fn has_selection(&self) -> bool {
        self.selection
            .is_some_and(|selection| !selection.is_empty())
    }

    fn selection_position_for_point(
        &self,
        position: gpui::Point<Pixels>,
        clamp: bool,
    ) -> Option<TerminalCellPosition> {
        let term = self.terminal.as_ref()?;
        let bounds = (*self.surface_bounds.borrow())?;

        let (cols, rows) = term.size();
        if cols == 0 || rows == 0 {
            return None;
        }

        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        if width <= 0.0 || height <= 0.0 {
            return None;
        }

        let local_x = f32::from(position.x) - f32::from(bounds.left());
        let local_y = f32::from(position.y) - f32::from(bounds.top());

        if !clamp && (local_x < 0.0 || local_y < 0.0 || local_x >= width || local_y >= height) {
            return None;
        }

        let clamped_x = local_x.clamp(0.0, (width - 1.0).max(0.0));
        let clamped_y = local_y.clamp(0.0, (height - 1.0).max(0.0));
        let col = (clamped_x / self.cell_width_px)
            .floor()
            .clamp(0.0, f32::from(cols.saturating_sub(1))) as usize;
        let row = (clamped_y / self.cell_height_px)
            .floor()
            .clamp(0.0, f32::from(rows.saturating_sub(1))) as usize;

        Some(TerminalCellPosition { row, col })
    }

    fn update_selection_head(&mut self, position: gpui::Point<Pixels>, clamp: bool) -> bool {
        let Some(head) = self.selection_position_for_point(position, clamp) else {
            return false;
        };

        let Some(selection) = self.selection.as_mut() else {
            return false;
        };
        if selection.head == head {
            return false;
        }

        selection.head = head;
        true
    }

    fn surface_contains_point(&self, position: gpui::Point<Pixels>) -> bool {
        self.selection_position_for_point(position, false).is_some()
    }

    fn finish_selection(
        &mut self,
        position: gpui::Point<Pixels>,
        clamp: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = self.selection_dragging && self.update_selection_head(position, clamp);
        self.selection_dragging = false;

        if self.selection.is_some_and(TerminalSelection::is_empty) {
            self.selection = None;
        }

        if changed || self.selection.is_none() {
            cx.notify();
        }
    }

    fn copy_selection_to_clipboard(&self, cx: &mut Context<Self>) {
        let Some(text) = self.selected_text() else {
            return;
        };

        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn apply_osc52_clipboard(&self, cx: &mut Context<Self>) {
        let Some(term) = self.terminal.as_ref() else {
            return;
        };
        let Some(payload) = term.take_osc52_clipboard() else {
            return;
        };
        let Some(text) = decode_osc52_clipboard_payload(&payload) else {
            return;
        };

        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn sync_terminal_title(&mut self) {
        self.terminal_title = self
            .terminal
            .as_ref()
            .and_then(PierTerminal::window_title)
            .map(Into::into);
    }

    fn sync_ssh_target(&mut self) {
        let Some(term) = self.terminal.as_ref() else {
            self.ssh_target = None;
            return;
        };

        if let Some((host, user, port)) = term.take_ssh_detected() {
            self.ssh_target = Some(format_ssh_target(&user, &host, port).into());
        }

        if term.take_ssh_exit_detected() {
            self.ssh_target = None;
        }
    }

    fn sync_bell_flash(&mut self) {
        let Some(term) = self.terminal.as_ref() else {
            self.bell_flash_until = None;
            return;
        };

        if term.take_bell_pending() {
            self.bell_flash_until = Some(Instant::now() + Duration::from_millis(BELL_FLASH_MS));
        }
    }

    fn selected_text(&self) -> Option<String> {
        let selection = self.selection?;
        if selection.is_empty() {
            return None;
        }

        let snapshot = self.visible_snapshot()?;
        let text = extract_selection_text(&snapshot, selection);
        (!text.is_empty()).then_some(text)
    }

    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return false;
        };

        let bracketed_mode = self
            .terminal
            .as_ref()
            .map(PierTerminal::bracketed_paste_mode)
            .unwrap_or(false);
        let bytes = encode_terminal_paste(&text, bracketed_mode);
        if bytes.is_empty() {
            return false;
        }

        self.write_input(&bytes)
    }

    fn visible_snapshot(&self) -> Option<GridSnapshot> {
        let snapshot_key = self.current_snapshot_key()?;
        if let Some(cache) = self.render_cache.as_ref() {
            if cache.key.snapshot == snapshot_key {
                return Some(cache.snapshot.clone());
            }
        }

        self.terminal
            .as_ref()
            .map(|term| term.snapshot_view(self.scrollback_offset))
    }

    fn current_snapshot_key(&self) -> Option<TerminalSnapshotKey> {
        let term = self.terminal.as_ref()?;
        let (cols, rows) = term.size();
        Some(TerminalSnapshotKey {
            generation: self.notify_state.generation.load(Ordering::Relaxed),
            scrollback_offset: self.scrollback_offset,
            cols,
            rows,
        })
    }

    fn current_render_key(&self, t: &crate::theme::Theme) -> Option<TerminalRenderKey> {
        Some(TerminalRenderKey {
            snapshot: self.current_snapshot_key()?,
            selection: self.selection.filter(|selection| !selection.is_empty()),
            terminal_theme_preset: self.terminal_theme_preset,
            terminal_opacity_pct: self.terminal_opacity_pct,
            font_family: t.font_mono.to_string(),
            font_ligatures: self.terminal_font_ligatures,
            cursor_style: self.terminal_cursor_style,
            cursor_visible: self.cursor_visible(),
        })
    }

    fn word_selection_at(&self, position: TerminalCellPosition) -> Option<TerminalSelection> {
        let snapshot = self.visible_snapshot()?;
        select_word_at(&snapshot, position)
    }

    fn line_selection_at(&self, position: TerminalCellPosition) -> Option<TerminalSelection> {
        let snapshot = self.visible_snapshot()?;
        select_line_at(&snapshot, position)
    }

    fn write_input(&mut self, bytes: &[u8]) -> bool {
        let Some(term) = self.terminal.as_ref() else {
            return false;
        };

        if !bytes.is_empty() {
            self.diagnostics.note_input();
        }
        let cursor_was_hidden = !self.last_cursor_visible;
        if let Err(err) = term.write(bytes) {
            self.last_error = Some(format!("Failed to write to PTY: {err}").into());
            log::warn!("terminal[{}] PTY write failed: {err}", self.terminal_id);
            true
        } else {
            self.reset_cursor_blink();
            cursor_was_hidden
        }
    }

    /// Kept for future feature surfaces (e.g. command palette / metrics
    /// overlay). The slim status line in `render_status_line` uses inline
    /// labels instead of `StatusPill` to keep the per-render element
    /// budget low.
    #[allow(dead_code)]
    fn terminal_status(&self) -> (SharedString, StatusKind) {
        if let Some(error) = self.last_error.as_ref() {
            return (format!("Error: {error}").into(), StatusKind::Error);
        }

        match self.terminal.as_ref() {
            Some(term) if term.is_alive() => ("PTY: live".into(), StatusKind::Success),
            Some(_) => ("PTY: exited".into(), StatusKind::Warning),
            None => ("PTY: unavailable".into(), StatusKind::Error),
        }
    }

    fn bell_flashing(&self) -> bool {
        self.bell_flash_until
            .is_some_and(|deadline| deadline > Instant::now())
    }

    #[allow(dead_code)]
    fn scrollback_label(&self) -> SharedString {
        let retained = self
            .terminal
            .as_ref()
            .map(|term| term.scrollback_len())
            .unwrap_or(0);
        format!("scrollback: {}/{}", self.scrollback_offset, retained).into()
    }

    fn terminal_size_label(&self) -> SharedString {
        let (cols, rows) = self
            .terminal
            .as_ref()
            .map(PierTerminal::size)
            .unwrap_or((DEFAULT_COLS, DEFAULT_ROWS));
        format!("{cols} x {rows}").into()
    }

    fn session_label(&self) -> SharedString {
        self.ssh_target
            .clone()
            .unwrap_or_else(|| "local PTY".into())
    }

    #[allow(dead_code)]
    fn terminal_title_label(&self) -> SharedString {
        self.terminal_title
            .clone()
            .unwrap_or_else(|| "shell default".into())
    }

    fn sync_window_title(&mut self, window: &mut Window) {
        let terminal_title = self.terminal_title.as_ref().map(ToString::to_string);
        let ssh_target = self.ssh_target.as_ref().map(ToString::to_string);
        let desired: SharedString =
            build_window_title(terminal_title.as_deref(), ssh_target.as_deref()).into();
        if self.applied_window_title.as_ref() == Some(&desired) {
            return;
        }

        window.set_window_title(&desired);
        self.applied_window_title = Some(desired);
    }

    fn render_lines(&mut self, t: &crate::theme::Theme) -> Vec<TerminalLine> {
        let palette = terminal_palette(self.terminal_theme_preset);
        let Some(term) = self.terminal.as_ref() else {
            self.render_cache = None;
            return vec![fallback_terminal_line(
                "Terminal unavailable",
                &t.font_mono,
                self.terminal_font_ligatures,
                palette,
                self.terminal_opacity(),
            )];
        };

        let Some(key) = self.current_render_key(t) else {
            return vec![fallback_terminal_line(
                "Terminal unavailable",
                &t.font_mono,
                self.terminal_font_ligatures,
                palette,
                self.terminal_opacity(),
            )];
        };

        if let Some(cache) = self.render_cache.as_ref() {
            if cache.key == key {
                return cache.lines.clone();
            }
        }

        let snapshot = term.snapshot_view(self.scrollback_offset);
        let lines = render_terminal_lines(
            &snapshot,
            palette,
            &t.font_mono,
            self.terminal_font_ligatures,
            self.terminal_opacity(),
            key.cursor_visible && key.cursor_style == TerminalCursorStyle::Block,
            key.selection,
            self.render_cache.as_ref(),
            &key,
        );

        self.render_cache = Some(TerminalRenderCache {
            key,
            snapshot,
            lines: lines.clone(),
        });

        lines
    }

    /// Bundle everything the direct-GPU cell-grid paint pipeline needs in a
    /// single accessor so `render()` can hand it to the canvas closure
    /// without further `&self` access.
    ///
    /// `snapshot` is `None` only when there is no live PTY (the fallback
    /// `lines` will carry the "Terminal unavailable" message in that case).
    /// `cursor_paint` is `None` for hidden / Block / scrollback cursor —
    /// see [`cursor_paint_for_render_key`] for the mapping rules.
    fn paint_input(&mut self, t: &crate::theme::Theme) -> PaintInput {
        let palette = terminal_palette(self.terminal_theme_preset);
        let lines = self.render_lines(t);
        let snapshot = self.render_cache.as_ref().map(|cache| cache.snapshot.clone());
        let cursor_paint = self
            .current_render_key(t)
            .as_ref()
            .and_then(|key| cursor_paint_for_render_key(key, palette));
        PaintInput {
            lines,
            snapshot,
            cursor_paint,
            cell_width_px: self.cell_width_px,
            cell_height_px: self.cell_height_px,
            font_family: t.font_mono.clone(),
            font_size_px: self.terminal_font_size_px,
        }
    }
}

/// Everything the direct-GPU cell-grid paint pass consumes for one frame.
/// Built once per render and moved into the canvas prepaint closure.
struct PaintInput {
    lines: Vec<TerminalLine>,
    snapshot: Option<GridSnapshot>,
    cursor_paint: Option<(crate::components::terminal_grid::CursorPaintStyle, Hsla)>,
    cell_width_px: f32,
    cell_height_px: f32,
    font_family: SharedString,
    font_size_px: f32,
}

/// Map a `TerminalRenderKey` to the cursor overlay the paint pass should
/// draw. Returns `None` for hidden / Block / scrollback cursor — Block is
/// already encoded by `render_terminal_line` (the cursor cell's fg/bg are
/// swapped inside its `TextRun`s) so it must not also be drawn as a quad.
fn cursor_paint_for_render_key(
    key: &TerminalRenderKey,
    palette: &TerminalPalette,
) -> Option<(crate::components::terminal_grid::CursorPaintStyle, Hsla)> {
    if !key.cursor_visible {
        return None;
    }
    let style = match key.cursor_style {
        TerminalCursorStyle::Block => return None,
        TerminalCursorStyle::Underline => crate::components::terminal_grid::CursorPaintStyle::Underline,
        TerminalCursorStyle::Bar => crate::components::terminal_grid::CursorPaintStyle::Bar,
    };
    Some((style, terminal_hex_color(palette.cursor_bg_hex)))
}

impl Drop for TerminalPanel {
    fn drop(&mut self) {
        self.terminal.take();
    }
}

impl Panel for TerminalPanel {
    fn panel_name(&self) -> &'static str {
        Route::Terminal.panel_name()
    }

    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(Route::Terminal.label())
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(UiIcon::new(IconName::SquareTerminal).size(px(14.0)))
            .child(Route::Terminal.label())
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> Option<PanelControl> {
        None
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel_active == active {
            return;
        }
        self.panel_active = active;

        if active {
            self.ensure_refresh_loop(window, cx);
            self.ensure_resize_observer(window, cx);
            self.resize_for_window(window, cx);
            if !self.focus_handle.is_focused(window) {
                self.focus_handle.focus(window);
            }
            (self.on_activated)(Route::Terminal, window, cx);
        }
    }

    fn on_added_to(
        &mut self,
        _: WeakEntity<TabPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_refresh_loop(window, cx);
        self.ensure_resize_observer(window, cx);
        self.resize_for_window(window, cx);
    }
}

impl EventEmitter<PanelEvent> for TerminalPanel {}

impl Focusable for TerminalPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_refresh_loop(window, cx);
        self.ensure_resize_observer(window, cx);
        self.diagnostics.note_render();

        // ⚠️ Render is paint-only (CLAUDE.md Rule 6). `sync_window_title` and
        // `update_transient_state` used to run here on every PTY echo — they
        // now live in the refresh loop only (see `ensure_refresh_loop`).
        // PTY resize also lives in the refresh loop now, driven by the
        // measured terminal surface size instead of render-time mutation.
        let t = theme(cx).clone();
        let preferences_changed = self.sync_terminal_preferences(
            t.settings.terminal_font_size as f32,
            t.settings.terminal_cursor_style,
            t.settings.terminal_cursor_blink,
            t.settings.terminal_theme_preset,
            t.settings.terminal_opacity_pct,
            t.settings.terminal_font_ligatures,
        );
        if preferences_changed {
            self.render_cache = None;
            self.reset_cursor_blink();
        }
        let bell_active = self.bell_flashing();
        let palette = terminal_palette(self.terminal_theme_preset);
        let surface_bg = terminal_bg_color(palette.background_hex, self.terminal_opacity());
        let border_color = if bell_active {
            t.color.status_warning
        } else if self.focus_handle.is_focused(window) {
            t.color.border_focus
        } else {
            t.color.border_default
        };
        let surface_bounds = Rc::clone(&self.surface_bounds);
        let status_line = self.render_status_line(&t, bell_active);

        // Phase 11: build the direct-GPU paint input AFTER all the &mut self
        // accessors above have run. The canvas closures take ownership and
        // run at paint time without touching `self` again.
        let paint_input = self.paint_input(&t);
        let cell_w = paint_input.cell_width_px;
        let cell_h = paint_input.cell_height_px;
        let font_family = paint_input.font_family.clone();
        let font_size_px = paint_input.font_size_px;

        // Slim shell: status row + grid surface. The 30-element Shell /
        // Input / Session / Title chrome that used to live here was
        // rebuilding on every PTY echo and dominated render cost; the
        // status row carries the same information in 6 inline labels.
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(status_line)
            .child(
                div()
                    .flex_1()
                    .min_h(px((cell_h * MIN_ROWS as f32).max(TERMINAL_MIN_HEIGHT)))
                    .bg(surface_bg)
                    .border_t_1()
                    .border_color(border_color)
                    .overflow_hidden()
                    .cursor(CursorStyle::IBeam)
                    .track_focus(&self.focus_handle)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_terminal_mouse_down))
                    .on_mouse_move(cx.listener(Self::on_terminal_mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_terminal_mouse_up))
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(Self::on_terminal_mouse_up_out),
                    )
                    .on_key_down(cx.listener(Self::on_key_down))
                    .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
                    .child(
                        // The cell grid is one canvas — no inner element tree.
                        // prepaint owns surface_bounds tracking + LayoutState
                        // build (pure CPU); paint walks the LayoutState with
                        // shape_line + paint_quad. See components/terminal_grid/.
                        canvas(
                            move |bounds, _window, _cx| {
                                let mut sb = surface_bounds.borrow_mut();
                                if sb.as_ref() != Some(&bounds) {
                                    *sb = Some(bounds);
                                }
                                let cell_size = Size {
                                    width: px(cell_w),
                                    height: px(cell_h),
                                };
                                terminal_grid::build(
                                    &paint_input.lines,
                                    paint_input.snapshot.as_ref(),
                                    paint_input.cursor_paint,
                                    cell_size,
                                    bounds.origin,
                                )
                            },
                            move |bounds, layout: LayoutState, window, cx| {
                                terminal_grid::run(
                                    bounds,
                                    &layout,
                                    &font_family,
                                    px(font_size_px),
                                    px(cell_h),
                                    window,
                                    cx,
                                );
                            },
                        )
                        .size_full(),
                    ),
            )
    }
}

impl TerminalPanel {
    /// Single-row terminal status strip. Mirrors what Pier shows in the
    /// terminal tab toolbar: shell + size + scrollback + ssh badge + bell.
    /// All values are produced as plain `SharedString`s (no Card / Pill /
    /// SectionLabel rebuild) so the per-render element budget stays low —
    /// see Phase 10 perf notes in CLAUDE.md / commit log.
    fn render_status_line(&self, t: &crate::theme::Theme, bell_active: bool) -> impl IntoElement {
        let shell_label = self.shell_path.clone();
        let size_label = self.terminal_size_label();
        let session_label = self.session_label();
        let scrollback = self.scrollback_offset;
        let scrollback_label: Option<SharedString> =
            (scrollback > 0).then(|| format!("scrollback {scrollback}").into());
        let pty_status = match self.terminal.as_ref() {
            Some(term) if term.is_alive() => None,
            Some(_) => Some(("PTY exited", t.color.status_warning)),
            None => Some(("PTY unavailable", t.color.status_error)),
        };

        let mut row = div()
            .h(px(22.0))
            .px(SP_3)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_3)
            .bg(t.color.bg_panel)
            .text_size(SIZE_CAPTION)
            .font_family(t.font_ui.clone())
            .text_color(t.color.text_tertiary)
            // shell · size — most common columns, always shown.
            .child(div().text_color(t.color.text_secondary).child(shell_label))
            .child(div().child(size_label));

        if let Some(label) = scrollback_label {
            row = row.child(div().child(label));
        }
        row = row.child(
            div()
                .text_color(t.color.accent)
                .child(SharedString::from(format!("· {session_label}"))),
        );
        if let Some((label, color)) = pty_status {
            row = row.child(div().text_color(color).child(label));
        }
        if bell_active {
            row = row.child(div().text_color(t.color.status_warning).child("bell"));
        }
        // Spacer pushes nothing further right; keep it for visual symmetry.
        row.child(div().flex_1())
    }

}

fn preferred_shell() -> String {
    if let Some(shell) = env::var_os("PIER_SHELL") {
        let shell = shell.to_string_lossy().trim().to_string();
        if !shell.is_empty() {
            return shell;
        }
    }

    #[cfg(windows)]
    {
        for candidate in ["pwsh.exe", "powershell.exe"] {
            if command_exists(candidate) {
                return candidate.to_string();
            }
        }

        if let Some(shell) = env::var_os("COMSPEC") {
            let shell = shell.to_string_lossy().trim().to_string();
            if !shell.is_empty() {
                return shell;
            }
        }

        "cmd.exe".to_string()
    }

    #[cfg(not(windows))]
    {
        env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

#[cfg(windows)]
fn command_exists(command: &str) -> bool {
    let command_path = Path::new(command);
    if command_path.is_absolute() {
        return command_path.exists();
    }

    env::var_os("PATH")
        .map(|paths| {
            env::split_paths(&paths).any(|dir| {
                let candidate = PathBuf::from(&dir).join(command);
                candidate.exists()
            })
        })
        .unwrap_or(false)
}

fn translate_keystroke(keystroke: &Keystroke) -> Option<Vec<u8>> {
    let modifiers = keystroke.modifiers;

    if modifiers.control {
        match keystroke.key.as_str() {
            "@" => return Some(vec![0]),
            "space" => return Some(vec![0]),
            "[" => return Some(vec![27]),
            "\\" => return Some(vec![28]),
            "]" => return Some(vec![29]),
            "^" => return Some(vec![30]),
            "_" => return Some(vec![31]),
            key if key.len() == 1 => {
                let ch = key.as_bytes()[0].to_ascii_lowercase();
                if ch.is_ascii_lowercase() {
                    return Some(vec![ch - b'a' + 1]);
                }
            }
            _ => {}
        }
    }

    match keystroke.key.as_str() {
        "enter" => Some(vec![b'\r']),
        "backspace" => Some(vec![0x7f]),
        "tab" => Some(vec![b'\t']),
        "space" => {
            if modifiers.platform || modifiers.function {
                None
            } else if modifiers.alt && !modifiers.control {
                Some(vec![0x1b, b' '])
            } else {
                Some(vec![b' '])
            }
        }
        "escape" => Some(vec![0x1b]),
        "left" => Some(b"\x1b[D".to_vec()),
        "right" => Some(b"\x1b[C".to_vec()),
        "up" => Some(b"\x1b[A".to_vec()),
        "down" => Some(b"\x1b[B".to_vec()),
        "home" => Some(b"\x1b[H".to_vec()),
        "end" => Some(b"\x1b[F".to_vec()),
        "delete" => Some(b"\x1b[3~".to_vec()),
        "pageup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" => Some(b"\x1b[6~".to_vec()),
        "insert" => Some(b"\x1b[2~".to_vec()),
        _ => {
            if modifiers.platform || modifiers.function {
                return None;
            }

            let key_char = keystroke.key_char.as_ref()?;
            if modifiers.alt && !modifiers.control {
                let mut bytes = vec![0x1b];
                bytes.extend_from_slice(key_char.as_bytes());
                Some(bytes)
            } else {
                Some(key_char.as_bytes().to_vec())
            }
        }
    }
}

fn wants_clipboard_paste(keystroke: &Keystroke) -> bool {
    let modifiers = keystroke.modifiers;

    let is_secondary_paste = keystroke.key.eq_ignore_ascii_case("v")
        && modifiers.secondary()
        && !modifiers.alt
        && !modifiers.function;
    let is_shift_insert = keystroke.key == "insert"
        && modifiers.shift
        && !modifiers.control
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function;

    is_secondary_paste || is_shift_insert
}

fn wants_copy_selection(keystroke: &Keystroke) -> bool {
    let modifiers = keystroke.modifiers;
    keystroke.key.eq_ignore_ascii_case("c")
        && modifiers.secondary()
        && !modifiers.alt
        && !modifiers.function
}

fn normalize_pasted_text(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut saw_carriage_return = false;

    for ch in text.chars() {
        match ch {
            '\r' => {
                bytes.push(b'\r');
                saw_carriage_return = true;
            }
            '\n' => {
                if !saw_carriage_return {
                    bytes.push(b'\r');
                }
                saw_carriage_return = false;
            }
            _ => {
                let mut encoded = [0; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
                saw_carriage_return = false;
            }
        }
    }

    bytes
}

fn encode_terminal_paste(text: &str, bracketed_mode: bool) -> Vec<u8> {
    let payload = normalize_pasted_text(text);
    if payload.is_empty() {
        return payload;
    }

    if !bracketed_mode {
        return payload;
    }

    let mut bytes = Vec::with_capacity(payload.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

fn format_ssh_target(user: &str, host: &str, port: u16) -> String {
    let host = host.trim();
    let user = user.trim();
    let endpoint = if port == 22 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };

    if user.is_empty() {
        endpoint
    } else {
        format!("{user}@{endpoint}")
    }
}

fn build_window_title(terminal_title: Option<&str>, ssh_target: Option<&str>) -> String {
    let primary = terminal_title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| ssh_target.map(|target| format!("Terminal · {target}")))
        .unwrap_or_else(|| "Terminal".to_string());

    format!("{primary} · Pier-X")
}

fn decode_osc52_clipboard_payload(raw: &str) -> Option<String> {
    let payload = raw.trim();
    if payload == "?" {
        return None;
    }
    if payload.is_empty() {
        return Some(String::new());
    }

    let mut padded = payload.to_string();
    let remainder = padded.len() % 4;
    if remainder != 0 {
        padded.extend(std::iter::repeat_n('=', 4 - remainder));
    }

    let bytes = BASE64_STANDARD.decode(padded).ok()?;
    if bytes.len() > MAX_OSC52_CLIPBOARD_BYTES {
        return None;
    }

    String::from_utf8(bytes).ok()
}

fn selected_terminal_style(style: TerminalRunStyle, palette: &TerminalPalette) -> TerminalRunStyle {
    TerminalRunStyle {
        fg_hex: palette.selection_fg_hex,
        bg_hex: palette.selection_bg_hex,
        bold: style.bold,
        underline: style.underline,
        apply_background_opacity: true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalSelectionSpan {
    start_col: usize,
    end_col: usize,
}

fn selection_span_for_row(
    selection: Option<TerminalSelection>,
    row: usize,
    cols: usize,
) -> Option<TerminalSelectionSpan> {
    if cols == 0 {
        return None;
    }

    let selection = selection?;
    if selection.is_empty() {
        return None;
    }

    let (start, end) = selection.normalized();
    if row < start.row || row > end.row {
        return None;
    }

    Some(TerminalSelectionSpan {
        start_col: if row == start.row {
            start.col.min(cols - 1)
        } else {
            0
        },
        end_col: if row == end.row {
            end.col.min(cols - 1)
        } else {
            cols - 1
        },
    })
}

fn block_cursor_column_for_row(
    snapshot: &GridSnapshot,
    cursor_visible: bool,
    cursor_style: TerminalCursorStyle,
    row: usize,
) -> Option<usize> {
    if !cursor_visible
        || cursor_style != TerminalCursorStyle::Block
        || row != snapshot.cursor_y as usize
        || row >= snapshot.rows as usize
    {
        return None;
    }

    Some((snapshot.cursor_x as usize).min(snapshot.cols.saturating_sub(1) as usize))
}

fn can_reuse_terminal_line(
    previous: &TerminalRenderCache,
    next_snapshot: &GridSnapshot,
    next_key: &TerminalRenderKey,
    row: usize,
) -> bool {
    if previous.key.terminal_theme_preset != next_key.terminal_theme_preset
        || previous.key.terminal_opacity_pct != next_key.terminal_opacity_pct
        || previous.key.font_family != next_key.font_family
        || previous.key.font_ligatures != next_key.font_ligatures
    {
        return false;
    }
    if previous.snapshot.cols != next_snapshot.cols || previous.snapshot.rows != next_snapshot.rows
    {
        return false;
    }
    if row >= previous.lines.len() {
        return false;
    }

    let cols = next_snapshot.cols as usize;
    let start = row * cols;
    let end = start + cols;
    if previous.snapshot.cells[start..end] != next_snapshot.cells[start..end] {
        return false;
    }
    if block_cursor_column_for_row(
        &previous.snapshot,
        previous.key.cursor_visible,
        previous.key.cursor_style,
        row,
    ) != block_cursor_column_for_row(
        next_snapshot,
        next_key.cursor_visible,
        next_key.cursor_style,
        row,
    ) {
        return false;
    }
    if selection_span_for_row(previous.key.selection, row, cols)
        != selection_span_for_row(next_key.selection, row, cols)
    {
        return false;
    }

    true
}

fn render_terminal_line(
    snapshot: &GridSnapshot,
    row: usize,
    palette: &TerminalPalette,
    font_family: &SharedString,
    font_ligatures: bool,
    background_opacity: f32,
    show_cursor: bool,
    selection: Option<TerminalSelection>,
) -> TerminalLine {
    let cols = snapshot.cols as usize;
    let cursor_col =
        block_cursor_column_for_row(snapshot, show_cursor, TerminalCursorStyle::Block, row);
    let selection_span = selection_span_for_row(selection, row, cols);
    let mut line = String::with_capacity(cols);
    let mut run_specs = Vec::<TerminalRun>::new();
    let mut current_style = None;
    let mut current_len = 0usize;
    let mut current_cell_span = 0usize;

    for col in 0..cols {
        let cell = &snapshot.cells[row * cols + col];
        if cell.ch == '\0' {
            continue;
        }

        let cell_span = visible_cell_span(snapshot, row, col);
        let style = resolve_terminal_style(
            cell,
            palette,
            cursor_col.is_some_and(|cursor| cursor >= col && cursor < col + cell_span),
        );
        let style = if selection_span
            .is_some_and(|span| selection_intersects_visible_cell(span, col, cell_span))
        {
                selected_terminal_style(style, palette)
            } else {
                style
            };
        if current_style != Some(style) {
            push_terminal_run(
                &mut run_specs,
                &mut current_style,
                &mut current_len,
                &mut current_cell_span,
            );
            current_style = Some(style);
        }
        line.push(cell.ch);
        current_len += cell.ch.len_utf8();
        current_cell_span += cell_span;
    }

    if line.is_empty() {
        let default_style = default_terminal_style(palette);
        line.push(' ');
        current_style = Some(default_style);
        current_len = 1;
        current_cell_span = 1;
    }

    push_terminal_run(
        &mut run_specs,
        &mut current_style,
        &mut current_len,
        &mut current_cell_span,
    );
    let cell_spans = terminal_run_cell_spans(&run_specs);
    TerminalLine {
        text: line.into(),
        runs: terminal_runs(run_specs, font_family, font_ligatures, background_opacity),
        cell_spans,
    }
}

fn render_terminal_lines(
    snapshot: &GridSnapshot,
    palette: &TerminalPalette,
    font_family: &SharedString,
    font_ligatures: bool,
    background_opacity: f32,
    show_cursor: bool,
    selection: Option<TerminalSelection>,
    previous_cache: Option<&TerminalRenderCache>,
    key: &TerminalRenderKey,
) -> Vec<TerminalLine> {
    let rows = snapshot.rows as usize;
    let mut rendered = Vec::with_capacity(rows);

    for row in 0..rows {
        if let Some(previous) = previous_cache {
            if can_reuse_terminal_line(previous, snapshot, key, row) {
                rendered.push(previous.lines[row].clone());
                continue;
            }
        }

        rendered.push(render_terminal_line(
            snapshot,
            row,
            palette,
            font_family,
            font_ligatures,
            background_opacity,
            show_cursor,
            selection,
        ));
    }

    rendered
}

#[cfg(test)]
fn is_selection_cell(selection: Option<TerminalSelection>, row: usize, col: usize) -> bool {
    let Some(selection) = selection else {
        return false;
    };
    if selection.is_empty() {
        return false;
    }

    let position = TerminalCellPosition { row, col };
    let (start, end) = selection.normalized();
    position >= start && position <= end
}

fn fallback_terminal_line(
    message: &str,
    font_family: &SharedString,
    font_ligatures: bool,
    palette: &TerminalPalette,
    background_opacity: f32,
) -> TerminalLine {
    let style = default_terminal_style(palette);
    TerminalLine {
        text: SharedString::from(message.to_string()),
        runs: terminal_runs(
            vec![TerminalRun {
                style,
                len: message.chars().map(char::len_utf8).sum(),
                cell_span: message.chars().count(),
            }],
            font_family,
            font_ligatures,
            background_opacity,
        ),
        cell_spans: vec![message.chars().count()],
    }
}

fn visible_cell_span(snapshot: &GridSnapshot, row: usize, col: usize) -> usize {
    let cols = snapshot.cols as usize;
    let mut span = 1usize;
    let mut next = col + 1;
    while next < cols && snapshot.cells[row * cols + next].ch == '\0' {
        span += 1;
        next += 1;
    }
    span
}

fn selection_intersects_visible_cell(
    selection: TerminalSelectionSpan,
    col: usize,
    cell_span: usize,
) -> bool {
    let end_col = col + cell_span.saturating_sub(1);
    selection.start_col <= end_col && selection.end_col >= col
}

fn default_terminal_style(palette: &TerminalPalette) -> TerminalRunStyle {
    TerminalRunStyle {
        fg_hex: palette.foreground_hex,
        bg_hex: palette.background_hex,
        bold: false,
        underline: false,
        apply_background_opacity: true,
    }
}

fn resolve_terminal_style(
    cell: &pier_core::terminal::Cell,
    palette: &TerminalPalette,
    is_cursor: bool,
) -> TerminalRunStyle {
    let mut fg_hex = resolve_terminal_color(cell.fg, palette.foreground_hex, palette);
    let mut bg_hex = resolve_terminal_color(cell.bg, palette.background_hex, palette);

    if cell.reverse {
        std::mem::swap(&mut fg_hex, &mut bg_hex);
    }

    if is_cursor {
        fg_hex = palette.cursor_fg_hex;
        bg_hex = palette.cursor_bg_hex;
    }

    TerminalRunStyle {
        fg_hex,
        bg_hex,
        bold: cell.bold,
        underline: cell.underline,
        apply_background_opacity: !is_cursor,
    }
}

fn resolve_terminal_color(
    color: TerminalColor,
    default_hex: u32,
    palette: &TerminalPalette,
) -> u32 {
    match color {
        TerminalColor::Default => default_hex,
        TerminalColor::Indexed(index) => terminal_indexed_hex(palette, index),
        TerminalColor::Rgb(r, g, b) => ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
    }
}

fn terminal_cell_width_px(font_size_px: f32) -> f32 {
    BASE_CELL_WIDTH_PX * (font_size_px / BASE_TERMINAL_FONT_SIZE_PX)
}

fn terminal_cell_height_px(font_size_px: f32) -> f32 {
    BASE_CELL_HEIGHT_PX * (font_size_px / BASE_TERMINAL_FONT_SIZE_PX)
}

fn extract_selection_text(snapshot: &GridSnapshot, selection: TerminalSelection) -> String {
    if selection.is_empty() || snapshot.cols == 0 || snapshot.rows == 0 {
        return String::new();
    }

    let cols = snapshot.cols as usize;
    let rows = snapshot.rows as usize;
    let (mut start, mut end) = selection.normalized();
    start.row = start.row.min(rows - 1);
    end.row = end.row.min(rows - 1);
    start.col = start.col.min(cols - 1);
    end.col = end.col.min(cols - 1);
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }

    let mut lines = Vec::with_capacity(end.row - start.row + 1);
    for row in start.row..=end.row {
        let start_col = if row == start.row { start.col } else { 0 };
        let end_col = if row == end.row { end.col } else { cols - 1 };
        lines.push(extract_snapshot_line(snapshot, row, start_col, end_col));
    }

    lines.join("\n")
}

fn select_word_at(
    snapshot: &GridSnapshot,
    position: TerminalCellPosition,
) -> Option<TerminalSelection> {
    if snapshot.cols == 0 || snapshot.rows == 0 {
        return None;
    }

    let position = normalize_snapshot_position(snapshot, position)?;
    let ch = snapshot_cell(snapshot, position)?;
    let class = classify_terminal_selection_char(ch.ch);

    let mut start = position;
    while let Some(prev_col) = previous_visible_col(snapshot, start.row, start.col) {
        let prev = snapshot_cell(
            snapshot,
            TerminalCellPosition {
                row: start.row,
                col: prev_col,
            },
        )?;
        if classify_terminal_selection_char(prev.ch) != class {
            break;
        }
        start.col = prev_col;
    }

    let mut end = position;
    while let Some(next_col) = next_visible_col(snapshot, end.row, end.col) {
        let next = snapshot_cell(
            snapshot,
            TerminalCellPosition {
                row: end.row,
                col: next_col,
            },
        )?;
        if classify_terminal_selection_char(next.ch) != class {
            break;
        }
        end.col = next_col;
    }

    Some(TerminalSelection {
        anchor: start,
        head: end,
    })
}

fn select_line_at(
    snapshot: &GridSnapshot,
    position: TerminalCellPosition,
) -> Option<TerminalSelection> {
    if snapshot.cols == 0 || snapshot.rows == 0 {
        return None;
    }

    let row = position.row.min(snapshot.rows as usize - 1);
    let cols = snapshot.cols as usize;
    let mut last_non_blank = None;
    for col in 0..cols {
        let cell = &snapshot.cells[row * cols + col];
        if cell.ch != '\0' && !cell.ch.is_whitespace() {
            last_non_blank = Some(col);
        }
    }

    let end_col = last_non_blank.unwrap_or(0);
    Some(TerminalSelection {
        anchor: TerminalCellPosition { row, col: 0 },
        head: TerminalCellPosition { row, col: end_col },
    })
}

fn extract_snapshot_line(
    snapshot: &GridSnapshot,
    row: usize,
    start_col: usize,
    end_col: usize,
) -> String {
    let cols = snapshot.cols as usize;
    let mut line = String::new();

    for col in start_col..=end_col {
        let cell = &snapshot.cells[row * cols + col];
        if cell.ch != '\0' {
            line.push(cell.ch);
        }
    }

    while line.ends_with(' ') {
        line.pop();
    }

    line
}

fn normalize_snapshot_position(
    snapshot: &GridSnapshot,
    position: TerminalCellPosition,
) -> Option<TerminalCellPosition> {
    if snapshot.cols == 0 || snapshot.rows == 0 {
        return None;
    }

    let row = position.row.min(snapshot.rows as usize - 1);
    let mut col = position.col.min(snapshot.cols as usize - 1);
    while snapshot.cells[row * snapshot.cols as usize + col].ch == '\0' {
        if col == 0 {
            break;
        }
        col -= 1;
    }

    let normalized = TerminalCellPosition { row, col };
    snapshot_cell(snapshot, normalized).map(|_| normalized)
}

fn snapshot_cell(snapshot: &GridSnapshot, position: TerminalCellPosition) -> Option<&Cell> {
    if position.row >= snapshot.rows as usize || position.col >= snapshot.cols as usize {
        return None;
    }

    snapshot
        .cells
        .get(position.row * snapshot.cols as usize + position.col)
}

fn previous_visible_col(snapshot: &GridSnapshot, row: usize, col: usize) -> Option<usize> {
    let mut current = col;
    while current > 0 {
        current -= 1;
        let cell = &snapshot.cells[row * snapshot.cols as usize + current];
        if cell.ch != '\0' {
            return Some(current);
        }
    }
    None
}

fn next_visible_col(snapshot: &GridSnapshot, row: usize, col: usize) -> Option<usize> {
    let cols = snapshot.cols as usize;
    let mut current = col + 1;
    while current < cols {
        let cell = &snapshot.cells[row * cols + current];
        if cell.ch != '\0' {
            return Some(current);
        }
        current += 1;
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectionCharClass {
    Word,
    Whitespace,
    Punctuation,
}

fn classify_terminal_selection_char(ch: char) -> SelectionCharClass {
    if ch.is_whitespace() {
        SelectionCharClass::Whitespace
    } else if is_terminal_word_char(ch) {
        SelectionCharClass::Word
    } else {
        SelectionCharClass::Punctuation
    }
}

fn is_terminal_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '\\' | ':' | '~')
}

fn push_terminal_run(
    runs: &mut Vec<TerminalRun>,
    current_style: &mut Option<TerminalRunStyle>,
    current_len: &mut usize,
    current_cell_span: &mut usize,
) {
    if *current_len == 0 {
        return;
    }

    if let Some(style) = current_style.take() {
        runs.push(TerminalRun {
            style,
            len: *current_len,
            cell_span: *current_cell_span,
        });
    }

    *current_len = 0;
    *current_cell_span = 0;
}

fn terminal_runs(
    run_specs: Vec<TerminalRun>,
    font_family: &SharedString,
    font_ligatures: bool,
    background_opacity: f32,
) -> Vec<TextRun> {
    let mut runs = Vec::with_capacity(run_specs.len());
    for run in run_specs {
        let mut mono = terminal_font_for_family(font_family, font_ligatures);
        mono.weight = if run.style.bold {
            WEIGHT_EMPHASIS
        } else {
            WEIGHT_REGULAR
        };

        let color = terminal_hex_color(run.style.fg_hex);
        runs.push(TextRun {
            len: run.len,
            font: mono,
            color,
            background_color: Some(if run.style.apply_background_opacity {
                terminal_bg_color(run.style.bg_hex, background_opacity)
            } else {
                terminal_hex_color(run.style.bg_hex)
            }),
            underline: run.style.underline.then_some(UnderlineStyle {
                color: Some(color),
                thickness: px(1.0),
                wavy: false,
            }),
            strikethrough: None,
        });
    }
    runs
}

fn terminal_run_cell_spans(run_specs: &[TerminalRun]) -> Vec<usize> {
    run_specs.iter().map(|run| run.cell_span).collect()
}

#[cfg(test)]
mod tests {
    use gpui::{Keystroke, Modifiers};
    use pier_core::terminal::Cell;

    use super::{
        block_cursor_column_for_row, build_window_title, decode_osc52_clipboard_payload,
        encode_terminal_paste, extract_selection_text, format_ssh_target, is_selection_cell,
        normalize_pasted_text, select_line_at, select_word_at, selection_span_for_row,
        translate_keystroke, wants_clipboard_paste, wants_copy_selection, GridSnapshot,
        TerminalCellPosition, TerminalColor, TerminalCursorStyle, TerminalSelection,
    };

    #[test]
    fn maps_ctrl_c_to_etx() {
        let bytes = translate_keystroke(&Keystroke {
            modifiers: Modifiers::control(),
            key: "c".into(),
            key_char: None,
        })
        .expect("ctrl+c should map");

        assert_eq!(bytes, vec![3]);
    }

    #[test]
    fn maps_alt_character_to_escape_prefix() {
        let bytes = translate_keystroke(&Keystroke {
            modifiers: Modifiers::alt(),
            key: "f".into(),
            key_char: Some("f".into()),
        })
        .expect("alt+f should map");

        assert_eq!(bytes, b"\x1bf".to_vec());
    }

    #[test]
    fn maps_spacebar_without_key_char() {
        let bytes = translate_keystroke(&Keystroke {
            modifiers: Modifiers::none(),
            key: "space".into(),
            key_char: None,
        })
        .expect("space should map");

        assert_eq!(bytes, vec![b' ']);
    }

    #[test]
    fn maps_named_navigation_keys() {
        let bytes = translate_keystroke(&Keystroke {
            modifiers: Modifiers::none(),
            key: "left".into(),
            key_char: None,
        })
        .expect("left arrow should map");

        assert_eq!(bytes, b"\x1b[D".to_vec());
    }

    #[test]
    fn treats_secondary_v_as_clipboard_paste() {
        let paste = wants_clipboard_paste(&Keystroke {
            modifiers: Modifiers::secondary_key(),
            key: "v".into(),
            key_char: Some("v".into()),
        });

        assert!(paste);
    }

    #[test]
    fn treats_shift_insert_as_clipboard_paste() {
        let paste = wants_clipboard_paste(&Keystroke {
            modifiers: Modifiers {
                shift: true,
                ..Modifiers::none()
            },
            key: "insert".into(),
            key_char: None,
        });

        assert!(paste);
    }

    #[test]
    fn normalizes_newlines_for_terminal_paste() {
        let bytes = normalize_pasted_text("echo 1\r\necho 2\necho 3");

        assert_eq!(bytes, b"echo 1\recho 2\recho 3".to_vec());
    }

    #[test]
    fn wraps_paste_when_bracketed_mode_is_enabled() {
        let bytes = encode_terminal_paste("echo hi\n", true);

        assert_eq!(bytes, b"\x1b[200~echo hi\r\x1b[201~".to_vec());
    }

    #[test]
    fn decodes_unpadded_osc52_payload() {
        let text = decode_osc52_clipboard_payload("L3RtcC94").expect("decoded OSC 52 text");

        assert_eq!(text, "/tmp/x");
    }

    #[test]
    fn ignores_osc52_clipboard_queries() {
        assert_eq!(decode_osc52_clipboard_payload("?"), None);
    }

    #[test]
    fn treats_secondary_c_as_copy_when_selection_exists() {
        let copy = wants_copy_selection(&Keystroke {
            modifiers: Modifiers::secondary_key(),
            key: "c".into(),
            key_char: Some("c".into()),
        });

        assert!(copy);
    }

    #[test]
    fn selection_cells_are_inclusive() {
        let selection = TerminalSelection {
            anchor: TerminalCellPosition { row: 0, col: 1 },
            head: TerminalCellPosition { row: 1, col: 2 },
        };

        assert!(is_selection_cell(Some(selection), 0, 1));
        assert!(is_selection_cell(Some(selection), 1, 2));
        assert!(!is_selection_cell(Some(selection), 0, 0));
    }

    #[test]
    fn selection_span_expands_across_interior_rows() {
        let selection = TerminalSelection {
            anchor: TerminalCellPosition { row: 0, col: 2 },
            head: TerminalCellPosition { row: 2, col: 1 },
        };

        assert_eq!(
            selection_span_for_row(Some(selection), 0, 6),
            Some(super::TerminalSelectionSpan {
                start_col: 2,
                end_col: 5,
            })
        );
        assert_eq!(
            selection_span_for_row(Some(selection), 1, 6),
            Some(super::TerminalSelectionSpan {
                start_col: 0,
                end_col: 5,
            })
        );
        assert_eq!(
            selection_span_for_row(Some(selection), 2, 6),
            Some(super::TerminalSelectionSpan {
                start_col: 0,
                end_col: 1,
            })
        );
    }

    #[test]
    fn cursor_column_is_only_reported_for_active_row() {
        let snapshot = GridSnapshot {
            cols: 4,
            rows: 2,
            cursor_x: 3,
            cursor_y: 1,
            bracketed_paste_mode: false,
            cells: vec![Cell::default(); 8],
        };

        assert_eq!(
            block_cursor_column_for_row(&snapshot, true, TerminalCursorStyle::Block, 1),
            Some(3)
        );
        assert_eq!(
            block_cursor_column_for_row(&snapshot, true, TerminalCursorStyle::Block, 0),
            None
        );
        assert_eq!(
            block_cursor_column_for_row(&snapshot, false, TerminalCursorStyle::Block, 1),
            None
        );
        assert_eq!(
            block_cursor_column_for_row(&snapshot, true, TerminalCursorStyle::Bar, 1),
            None
        );
    }

    #[test]
    fn extracts_multi_line_selection_without_placeholder_cells() {
        let mut cells = vec![Cell::default(); 8];
        for (index, ch) in ['a', 'b', 'c', ' ', 'd', '\0', 'e', ' ']
            .into_iter()
            .enumerate()
        {
            cells[index] = Cell {
                ch,
                fg: TerminalColor::Default,
                bg: TerminalColor::Default,
                bold: false,
                underline: false,
                reverse: false,
                hyperlink: None,
            };
        }

        let snapshot = GridSnapshot {
            cols: 4,
            rows: 2,
            cursor_x: 0,
            cursor_y: 0,
            bracketed_paste_mode: false,
            cells,
        };
        let selection = TerminalSelection {
            anchor: TerminalCellPosition { row: 0, col: 1 },
            head: TerminalCellPosition { row: 1, col: 2 },
        };

        assert_eq!(extract_selection_text(&snapshot, selection), "bc\nde");
    }

    #[test]
    fn double_click_selects_terminal_word_token() {
        let snapshot = GridSnapshot {
            cols: 9,
            rows: 1,
            cursor_x: 0,
            cursor_y: 0,
            bracketed_paste_mode: false,
            cells: "cd /tmp/x"
                .chars()
                .map(|ch| Cell {
                    ch,
                    fg: TerminalColor::Default,
                    bg: TerminalColor::Default,
                    bold: false,
                    underline: false,
                    reverse: false,
                    hyperlink: None,
                })
                .collect(),
        };

        let selection = select_word_at(&snapshot, TerminalCellPosition { row: 0, col: 4 })
            .expect("word selection");

        assert_eq!(extract_selection_text(&snapshot, selection), "/tmp/x");
    }

    #[test]
    fn triple_click_selects_visible_line_without_trailing_spaces() {
        let snapshot = GridSnapshot {
            cols: 6,
            rows: 1,
            cursor_x: 0,
            cursor_y: 0,
            bracketed_paste_mode: false,
            cells: ['l', 's', ' ', '-', 'l', ' ']
                .into_iter()
                .map(|ch| Cell {
                    ch,
                    fg: TerminalColor::Default,
                    bg: TerminalColor::Default,
                    bold: false,
                    underline: false,
                    reverse: false,
                    hyperlink: None,
                })
                .collect(),
        };

        let selection = select_line_at(&snapshot, TerminalCellPosition { row: 0, col: 2 })
            .expect("line selection");

        assert_eq!(extract_selection_text(&snapshot, selection), "ls -l");
    }

    #[test]
    fn formats_ssh_target_without_default_port_noise() {
        assert_eq!(format_ssh_target("root", "box.local", 22), "root@box.local");
        assert_eq!(format_ssh_target("", "box.local", 2222), "box.local:2222");
    }

    #[test]
    fn builds_window_title_from_terminal_title_or_session() {
        assert_eq!(
            build_window_title(Some("lazygit"), Some("root@box.local")),
            "lazygit · Pier-X"
        );
        assert_eq!(
            build_window_title(None, Some("root@box.local")),
            "Terminal · root@box.local · Pier-X"
        );
        assert_eq!(build_window_title(None, None), "Terminal · Pier-X");
    }
}
