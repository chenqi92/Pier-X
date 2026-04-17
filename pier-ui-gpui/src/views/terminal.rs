use std::{
    cell::RefCell,
    env,
    ffi::c_void,
    path::{Path, PathBuf},
    rc::Rc,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use gpui::{
    canvas, div, font, prelude::*, px, App, Bounds, ClipboardItem, Context, CursorStyle,
    EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, Keystroke, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, ScrollDelta, ScrollWheelEvent,
    SharedString, StyledText, TextRun, UnderlineStyle, WeakEntity, Window,
};
use gpui_component::{
    dock::{Panel, PanelControl, PanelEvent, TabPanel},
    Icon as UiIcon, IconName,
};
use pier_core::terminal::{Cell, Color as TerminalColor, GridSnapshot, NotifyEvent, PierTerminal};

use crate::{
    app::{route::Route, ActivationHandler},
    components::{text, Card, SectionLabel, StatusKind, StatusPill},
    theme::{
        radius::RADIUS_MD,
        spacing::{SP_1, SP_3, SP_4},
        terminal::{
            terminal_cursor_bg_hex, terminal_cursor_fg_hex, terminal_default_bg_hex,
            terminal_default_fg_hex, terminal_hex_color, terminal_indexed_hex,
            terminal_selection_bg_hex, terminal_selection_fg_hex,
        },
        theme,
        typography::{
            SIZE_CAPTION, SIZE_MONO_CODE, WEIGHT_EMPHASIS, WEIGHT_MEDIUM, WEIGHT_REGULAR,
        },
        ThemeMode,
    },
};

const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 32;
const MIN_COLS: u16 = 64;
const MAX_COLS: u16 = 220;
const MIN_ROWS: u16 = 18;
const MAX_ROWS: u16 = 72;
const SCROLLBACK_LIMIT: usize = 20_000;
const CELL_WIDTH_PX: f32 = 8.2;
const CELL_HEIGHT_PX: f32 = 18.0;
const TERMINAL_MIN_HEIGHT: f32 = 220.0;
const WINDOW_CHROME_WIDTH: f32 = 356.0;
const WINDOW_CHROME_HEIGHT: f32 = 212.0;
const MAX_OSC52_CLIPBOARD_BYTES: usize = 1_000_000;
const BELL_FLASH_MS: u64 = 180;

#[derive(Clone)]
struct TerminalLine {
    text: SharedString,
    runs: Vec<TextRun>,
}

impl TerminalLine {
    fn into_element(self) -> impl IntoElement {
        StyledText::new(self.text).with_runs(self.runs)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TerminalRunStyle {
    fg_hex: u32,
    bg_hex: u32,
    bold: bool,
    underline: bool,
}

#[derive(Clone, Copy)]
struct TerminalRun {
    style: TerminalRunStyle,
    len: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TerminalCellPosition {
    row: usize,
    col: usize,
}

#[derive(Clone, Copy)]
struct TerminalSelection {
    anchor: TerminalCellPosition,
    head: TerminalCellPosition,
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

    fn normalized(self) -> (TerminalCellPosition, TerminalCellPosition) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
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
    surface_bounds: Rc<RefCell<Option<Bounds<Pixels>>>>,
    selection: Option<TerminalSelection>,
    selection_dragging: bool,
    notify_state: Box<NotifyState>,
}

impl TerminalPanel {
    pub fn new(on_activated: ActivationHandler, cx: &mut Context<Self>) -> Self {
        let shell_path: SharedString = preferred_shell().into();
        let notify_state = Box::<NotifyState>::default();

        let mut panel = Self {
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
            surface_bounds: Rc::new(RefCell::new(None)),
            selection: None,
            selection_dragging: false,
            notify_state,
        };
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
                self.terminal = Some(term);
                self.clamp_selection_to_terminal();
            }
            Err(err) => {
                self.last_error = Some(
                    format!("Failed to start terminal with `{}`: {err}", self.shell_path).into(),
                );
                self.terminal_title = None;
                self.ssh_target = None;
                self.bell_flash_until = None;
                self.terminal = None;
            }
        }
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
                        .timer(Duration::from_millis(33))
                        .await;

                    let still_alive = this
                        .update_in(cx, |this, _, cx| {
                            let current = this.notify_state.generation.load(Ordering::Relaxed);
                            let mut should_notify = false;
                            if current != seen_generation {
                                seen_generation = current;
                                this.handle_terminal_side_effects(cx);
                                should_notify = true;
                            }
                            if this.update_transient_state() {
                                should_notify = true;
                            }
                            if should_notify {
                                cx.notify();
                            }
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
            if this.resize_for_window(window) {
                cx.notify();
            }
        })
        .detach();
    }

    fn resize_for_window(&mut self, window: &Window) -> bool {
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
                    .max(CELL_WIDTH_PX * MIN_COLS as f32),
                (f32::from(viewport.height) - WINDOW_CHROME_HEIGHT)
                    .max(CELL_HEIGHT_PX * MIN_ROWS as f32),
            )
        };

        let cols = (width / CELL_WIDTH_PX)
            .floor()
            .clamp(MIN_COLS as f32, MAX_COLS as f32) as u16;
        let rows = (height / CELL_HEIGHT_PX)
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
                true
            }
        }
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
    }

    fn update_transient_state(&mut self) -> bool {
        if self
            .bell_flash_until
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            self.bell_flash_until = None;
            return true;
        }

        false
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
        self.focus_handle.focus(window);
        if event.button != MouseButton::Left {
            cx.stop_propagation();
            return;
        }

        let Some(position) = self.selection_position_for_point(event.position, false) else {
            self.selection = None;
            self.selection_dragging = false;
            cx.stop_propagation();
            return;
        };

        if event.click_count >= 3 {
            self.selection = self.line_selection_at(position);
            self.selection_dragging = false;
            cx.stop_propagation();
            cx.notify();
            return;
        }

        if event.click_count >= 2 {
            self.selection = self.word_selection_at(position);
            self.selection_dragging = false;
            cx.stop_propagation();
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

        cx.stop_propagation();
        cx.notify();
    }

    fn on_terminal_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selection_dragging || event.pressed_button != Some(MouseButton::Left) {
            return;
        }

        if self.update_selection_head(event.position, true) {
            cx.notify();
        }
        cx.stop_propagation();
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
        let Some(term) = self.terminal.as_ref() else {
            return;
        };

        let delta = match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => f32::from(pixels.y) / CELL_HEIGHT_PX,
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

        cx.stop_propagation();
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.has_selection() && wants_copy_selection(&event.keystroke) {
            self.copy_selection_to_clipboard(cx);
            cx.stop_propagation();
            return;
        }

        if wants_clipboard_paste(&event.keystroke) {
            self.selection = None;
            self.paste_from_clipboard(cx);
            cx.stop_propagation();
            return;
        }

        if let Some(bytes) = translate_keystroke(&event.keystroke) {
            self.selection = None;
            self.write_input(&bytes, cx);
            cx.notify();
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
        let col = (clamped_x / CELL_WIDTH_PX)
            .floor()
            .clamp(0.0, f32::from(cols.saturating_sub(1))) as usize;
        let row = (clamped_y / CELL_HEIGHT_PX)
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

        cx.stop_propagation();
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

    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };

        let bracketed_mode = self
            .terminal
            .as_ref()
            .map(PierTerminal::bracketed_paste_mode)
            .unwrap_or(false);
        let bytes = encode_terminal_paste(&text, bracketed_mode);
        if bytes.is_empty() {
            return;
        }

        self.write_input(&bytes, cx);
    }

    fn visible_snapshot(&self) -> Option<GridSnapshot> {
        self.terminal
            .as_ref()
            .map(|term| term.snapshot_view(self.scrollback_offset))
    }

    fn word_selection_at(&self, position: TerminalCellPosition) -> Option<TerminalSelection> {
        let snapshot = self.visible_snapshot()?;
        select_word_at(&snapshot, position)
    }

    fn line_selection_at(&self, position: TerminalCellPosition) -> Option<TerminalSelection> {
        let snapshot = self.visible_snapshot()?;
        select_line_at(&snapshot, position)
    }

    fn write_input(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let Some(term) = self.terminal.as_ref() else {
            return;
        };

        if let Err(err) = term.write(bytes) {
            self.last_error = Some(format!("Failed to write to PTY: {err}").into());
            cx.notify();
        }
    }

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

    fn render_lines(&self, mode: ThemeMode, font_family: &SharedString) -> Vec<TerminalLine> {
        let Some(term) = self.terminal.as_ref() else {
            return vec![fallback_terminal_line(
                "Terminal unavailable",
                font_family,
                mode,
            )];
        };

        let snapshot = term.snapshot_view(self.scrollback_offset);
        let cols = snapshot.cols as usize;
        let rows = snapshot.rows as usize;
        let show_cursor = self.scrollback_offset == 0 && term.is_alive();
        let cursor_row = snapshot.cursor_y as usize;
        let cursor_col = snapshot.cursor_x as usize;
        let selection = self.selection.filter(|selection| !selection.is_empty());

        let mut rendered = Vec::with_capacity(rows);
        for row in 0..rows {
            let mut line = String::with_capacity(cols);
            let mut run_specs = Vec::<TerminalRun>::new();
            let mut current_style = None;
            let mut current_len = 0usize;
            for col in 0..cols {
                let cell = &snapshot.cells[row * cols + col];
                if cell.ch == '\0' {
                    continue;
                }

                let style = resolve_terminal_style(
                    cell,
                    mode,
                    show_cursor && row == cursor_row && col == cursor_col,
                );
                let style = if is_selection_cell(selection, row, col) {
                    selected_terminal_style(style, mode)
                } else {
                    style
                };
                if current_style != Some(style) {
                    push_terminal_run(&mut run_specs, &mut current_style, &mut current_len);
                    current_style = Some(style);
                }
                line.push(cell.ch);
                current_len += cell.ch.len_utf8();
            }

            if line.is_empty() {
                let default_style = default_terminal_style(mode);
                line.push(' ');
                current_style = Some(default_style);
                current_len = 1;
            }

            push_terminal_run(&mut run_specs, &mut current_style, &mut current_len);
            rendered.push(TerminalLine {
                text: line.into(),
                runs: terminal_runs(run_specs, font_family),
            });
        }

        rendered
    }
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
        if active {
            self.ensure_refresh_loop(window, cx);
            self.ensure_resize_observer(window, cx);
            self.resize_for_window(window);
            self.focus_handle.focus(window);
            (self.on_activated)(Route::Terminal, window, cx);
            cx.notify();
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
        self.resize_for_window(window);
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
        self.resize_for_window(window);
        self.update_transient_state();
        self.sync_window_title(window);
        let t = theme(cx).clone();
        let lines = self.render_lines(t.mode, &t.font_mono);
        let (status_label, status_kind) = self.terminal_status();
        let shell_path = self.shell_path.clone();
        let size_label = self.terminal_size_label();
        let scroll_label = self.scrollback_label();
        let session_label = self.session_label();
        let title_label = self.terminal_title_label();
        let ssh_badge = self
            .ssh_target
            .clone()
            .map(|target| SharedString::from(format!("ssh: {target}")));
        let bell_active = self.bell_flashing();
        let border_color = if bell_active {
            t.color.status_warning
        } else if self.focus_handle.is_focused(window) {
            t.color.border_focus
        } else {
            t.color.border_default
        };
        let surface_bounds = Rc::clone(&self.surface_bounds);

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(SP_4)
            .p(SP_4)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(SP_3)
                    .child(text::h2("Terminal"))
                    .child(StatusPill::new(status_label, status_kind))
                    .child(StatusPill::new(scroll_label, StatusKind::Info))
                    .children(
                        ssh_badge
                            .into_iter()
                            .map(|label| StatusPill::new(label, StatusKind::Info)),
                    )
                    .children(bell_active.then_some(StatusPill::new("bell", StatusKind::Warning)))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(SIZE_CAPTION)
                            .font_weight(WEIGHT_MEDIUM)
                            .font_family(t.font_ui.clone())
                            .text_color(t.color.text_tertiary)
                            .child(size_label),
                    ),
            )
            .child(
                Card::new().padding(SP_3).child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap(SP_4)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(SP_1)
                                .child(SectionLabel::new("Shell"))
                                .child(text::mono(shell_path)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(SP_1)
                                .child(SectionLabel::new("Input"))
                                .child(text::body(
                                    "Enter, Tab, Ctrl+C, arrows, Home/End, PgUp/PgDn",
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(SP_1)
                                .child(SectionLabel::new("Session"))
                                .child(text::mono(session_label)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(SP_1)
                                .child(SectionLabel::new("Title"))
                                .child(text::body(title_label)),
                        ),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(TERMINAL_MIN_HEIGHT))
                    .p(SP_3)
                    .rounded(RADIUS_MD)
                    .bg(t.color.bg_canvas)
                    .border_1()
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
                        div()
                            .size_full()
                            .relative()
                            .child(
                                canvas(
                                    move |bounds, window, _| {
                                        let mut surface_bounds = surface_bounds.borrow_mut();
                                        let changed = surface_bounds.as_ref() != Some(&bounds);
                                        if changed {
                                            *surface_bounds = Some(bounds);
                                            window.refresh();
                                        }
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0(),
                            )
                            .child(
                                div()
                                    .size_full()
                                    .flex()
                                    .flex_col()
                                    .gap(px(0.0))
                                    .text_size(SIZE_MONO_CODE)
                                    .line_height(px(CELL_HEIGHT_PX))
                                    .bg(t.color.bg_canvas)
                                    .children(lines.into_iter().map(|line| {
                                        div()
                                            .min_h(px(CELL_HEIGHT_PX))
                                            .whitespace_nowrap()
                                            .child(line.into_element())
                                    })),
                            ),
                    ),
            )
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

fn selected_terminal_style(style: TerminalRunStyle, mode: ThemeMode) -> TerminalRunStyle {
    TerminalRunStyle {
        fg_hex: terminal_selection_fg_hex(mode),
        bg_hex: terminal_selection_bg_hex(mode),
        bold: style.bold,
        underline: style.underline,
    }
}

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
    mode: ThemeMode,
) -> TerminalLine {
    let style = default_terminal_style(mode);
    TerminalLine {
        text: SharedString::from(message.to_string()),
        runs: terminal_runs(
            vec![TerminalRun {
                style,
                len: message.chars().map(char::len_utf8).sum(),
            }],
            font_family,
        ),
    }
}

fn default_terminal_style(mode: ThemeMode) -> TerminalRunStyle {
    TerminalRunStyle {
        fg_hex: terminal_default_fg_hex(mode),
        bg_hex: terminal_default_bg_hex(mode),
        bold: false,
        underline: false,
    }
}

fn resolve_terminal_style(
    cell: &pier_core::terminal::Cell,
    mode: ThemeMode,
    is_cursor: bool,
) -> TerminalRunStyle {
    let mut fg_hex = resolve_terminal_color(cell.fg, terminal_default_fg_hex(mode));
    let mut bg_hex = resolve_terminal_color(cell.bg, terminal_default_bg_hex(mode));

    if cell.reverse {
        std::mem::swap(&mut fg_hex, &mut bg_hex);
    }

    if is_cursor {
        fg_hex = terminal_cursor_fg_hex(mode);
        bg_hex = terminal_cursor_bg_hex(mode);
    }

    TerminalRunStyle {
        fg_hex,
        bg_hex,
        bold: cell.bold,
        underline: cell.underline,
    }
}

fn resolve_terminal_color(color: TerminalColor, default_hex: u32) -> u32 {
    match color {
        TerminalColor::Default => default_hex,
        TerminalColor::Indexed(index) => terminal_indexed_hex(index),
        TerminalColor::Rgb(r, g, b) => ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
    }
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
) {
    if *current_len == 0 {
        return;
    }

    if let Some(style) = current_style.take() {
        runs.push(TerminalRun {
            style,
            len: *current_len,
        });
    }

    *current_len = 0;
}

fn terminal_runs(run_specs: Vec<TerminalRun>, font_family: &SharedString) -> Vec<TextRun> {
    let mut runs = Vec::with_capacity(run_specs.len());
    for run in run_specs {
        let mut mono = font(font_family.clone());
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
            background_color: Some(terminal_hex_color(run.style.bg_hex)),
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

#[cfg(test)]
mod tests {
    use gpui::{Keystroke, Modifiers};
    use pier_core::terminal::Cell;

    use super::{
        build_window_title, decode_osc52_clipboard_payload, encode_terminal_paste,
        extract_selection_text, format_ssh_target, is_selection_cell, normalize_pasted_text,
        select_line_at, select_word_at, translate_keystroke, wants_clipboard_paste,
        wants_copy_selection, GridSnapshot, TerminalCellPosition, TerminalColor, TerminalSelection,
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
