// Pier-X GPUI spike — ported overlay dialogs.
//
// Five web dialogs that the GPUI shell was missing. Each is an
// independent `Entity` (like `SettingsView`) hosted by `shell.rs`'s
// overlay layer; the shell constructs it once, feeds it the data it
// needs through a setter when the overlay opens, and renders it. A
// dialog asks to be dismissed by emitting [`DialogEvent::Close`], which
// the shell subscribes to (see `Shell::new`).
//
// Backend access goes straight to `pier_core` (the same direct-call
// style `panels/docker.rs` uses) so this file never grows `data.rs`.
// Every blocking call (egress probe, health probe, tunnel open, deep
// probe) runs on `cx.background_executor()`; the result is written back
// to the view and `cx.notify()`d. Colours / sizes / fonts come only
// from `self.theme`.
//
// NOTE: the module-wide `dead_code` allow below is temporary. These
// dialogs are constructed + routed by shell.rs's overlay layer, a change
// that lands once the concurrent Git-panel extraction releases shell.rs;
// drop this allow when that wiring is in.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Context, Entity, EventEmitter, FocusHandle, FontWeight, KeyDownEvent,
    MouseButton, MouseDownEvent, SharedString, Window,
};
use gpui_component::{h_flex, v_flex};

use pier_core::connections::ConnectionStore;
use pier_core::egress::{self, EgressKind, EgressProfile};
use pier_core::services::host_health::{
    self, HealthStatus, HostDeepProbeReport, HostHealthReport, HostHealthTarget,
};
use pier_core::ssh::{SshConfig, SshSession, Tunnel};

use crate::data;
use crate::terminal::TerminalView;
use crate::theme::Theme;
use crate::ui;

/// What every dialog emits to ask the shell to dismiss it.
pub enum DialogEvent {
    Close,
}

// ── Shared chrome helpers ────────────────────────────────────────────

/// The modal card frame: fixed-ish panel surface with a border and a
/// large radius. Callers add the header + body children.
fn card(t: &Theme, width: f32) -> gpui::Div {
    div()
        .w(px(width))
        .bg(t.panel)
        .border_1()
        .border_color(t.line_2)
        .rounded(t.radius_lg)
        .overflow_hidden()
}

/// The dismiss "✕" button, wired to emit [`DialogEvent::Close`].
fn close_btn<D>(t: &Theme, cx: &mut Context<D>) -> impl IntoElement
where
    D: Render + EventEmitter<DialogEvent>,
{
    div()
        .id("dlg-close")
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(24.0))
        .h(px(24.0))
        .rounded(t.radius_sm)
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_this, _: &MouseDownEvent, _w, cx| cx.emit(DialogEvent::Close)),
        )
        .child(ui::icon("close", px(14.0), t.muted))
}

/// Header row: accent glyph + bold title + spacer + close button.
fn header<D>(
    t: &Theme,
    cx: &mut Context<D>,
    glyph: &'static str,
    title: impl Into<SharedString>,
) -> impl IntoElement
where
    D: Render + EventEmitter<DialogEvent>,
{
    h_flex()
        .items_center()
        .gap(t.sp2)
        .w_full()
        .h(px(40.0))
        .px(t.sp4)
        .border_b_1()
        .border_color(t.line)
        .child(ui::icon(glyph, px(16.0), t.accent))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.ink)
                .child(title.into()),
        )
        .child(close_btn(t, cx))
}

/// A pill button that runs `on_click`. `primary` paints it accent.
fn btn<D, F>(
    t: &Theme,
    cx: &mut Context<D>,
    id: &str,
    label: impl Into<SharedString>,
    primary: bool,
    on_click: F,
) -> impl IntoElement
where
    D: Render + 'static,
    F: Fn(&mut D, &mut Window, &mut Context<D>) + 'static,
{
    div()
        .id(SharedString::from(id.to_string()))
        .px(t.sp3)
        .py(px(5.0))
        .rounded(t.radius_sm)
        .text_size(t.fs_ui)
        .cursor_pointer()
        .when(primary, |d| d.bg(t.accent).text_color(t.accent_ink))
        .when(!primary, |d| d.bg(t.panel_2).text_color(t.ink_2).hover(|s| s.bg(t.elev)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, window, cx| on_click(this, window, cx)),
        )
        .child(label.into())
}

/// A single-line text box echoing `value`, accent-bordered when active.
/// Clicking it runs `on_focus` (which the caller uses to mark the field
/// active + focus the dialog).
fn input_box<D, F>(
    t: &Theme,
    cx: &mut Context<D>,
    id: &str,
    value: &str,
    placeholder: &'static str,
    active: bool,
    on_focus: F,
) -> impl IntoElement
where
    D: Render + 'static,
    F: Fn(&mut D, &mut Window, &mut Context<D>) + 'static,
{
    let empty = value.is_empty();
    let shown = value.to_string();
    div()
        .id(SharedString::from(id.to_string()))
        .h(px(30.0))
        .px(t.sp2)
        .flex()
        .items_center()
        .rounded(t.radius_sm)
        .bg(t.panel_2)
        .border_1()
        .border_color(if active { t.accent } else { t.line_2 })
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, window, cx| on_focus(this, window, cx)),
        )
        .when(empty, |d| d.text_color(t.dim).child(placeholder))
        .when(!empty, |d| d.text_color(t.ink).child(shown))
}

/// A `label` above `inner`.
fn labeled(t: &Theme, label: impl Into<SharedString>, inner: impl IntoElement) -> impl IntoElement {
    v_flex()
        .gap(px(3.0))
        .child(div().text_size(t.fs_sm).text_color(t.muted).child(label.into()))
        .child(inner)
}

/// A small note / hint line in dim text.
fn note(t: &Theme, text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .px(t.sp1)
        .text_size(t.fs_sm)
        .text_color(t.dim)
        .child(text.into())
}

/// Read the printable character from a key event, skipping modifier
/// chords and control codes. Shared by every dialog's key handler.
fn typed_char(ev: &KeyDownEvent) -> Option<String> {
    let m = &ev.keystroke.modifiers;
    if m.control || m.alt || m.platform {
        return None;
    }
    let kc = ev.keystroke.key_char.as_ref()?;
    if kc.is_empty() || kc.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(kc.clone())
}

// ═══════════════════════════════════════════════════════════════════
// Broadcast — fan one command into many SSH tabs.
// ═══════════════════════════════════════════════════════════════════

/// One candidate target the shell hands us: a tab's terminal plus a
/// snapshot of whether it has a live SSH session.
pub struct BroadcastTarget {
    pub tab_index: usize,
    pub title: String,
    pub terminal: Entity<TerminalView>,
    pub live: bool,
}

pub struct BroadcastDialog {
    targets: Vec<BroadcastTarget>,
    /// Picked tab indices (a subset of live targets).
    picked: HashSet<usize>,
    command: String,
    append_newline: bool,
    status: Option<String>,
    focus: FocusHandle,
    theme: Theme,
}

impl BroadcastDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            targets: Vec::new(),
            picked: HashSet::new(),
            command: String::new(),
            append_newline: true,
            status: None,
            focus: cx.focus_handle(),
            theme: Theme::dark(),
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Refresh the candidate tabs (called by the shell on open). Pre-picks
    /// every live target so the common "all hosts" case is one click away.
    pub fn set_targets(&mut self, targets: Vec<BroadcastTarget>, cx: &mut Context<Self>) {
        self.picked = targets.iter().filter(|t| t.live).map(|t| t.tab_index).collect();
        self.targets = targets;
        self.status = None;
        cx.notify();
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        if self.command.is_empty() {
            self.status = Some("Type a command first.".to_string());
            cx.notify();
            return;
        }
        // PTYs take a carriage return for Enter (see terminal::keystroke_to_bytes).
        let text = if self.append_newline {
            format!("{}\r", self.command)
        } else {
            self.command.clone()
        };
        let mut sent = 0usize;
        for tgt in &self.targets {
            if tgt.live && self.picked.contains(&tgt.tab_index) {
                tgt.terminal.update(cx, |tv, _cx| tv.send_input(&text));
                sent += 1;
            }
        }
        self.status = Some(if sent == 0 {
            "Pick at least one live SSH tab.".to_string()
        } else {
            format!("Broadcast sent to {sent} session(s).")
        });
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            "escape" => {
                cx.emit(DialogEvent::Close);
                return;
            }
            "enter" => {
                self.send(cx);
                return;
            }
            "backspace" => {
                if self.command.pop().is_some() {
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        if let Some(kc) = typed_char(ev) {
            self.command.push_str(&kc);
            cx.notify();
        }
    }

    fn target_row(&self, cx: &mut Context<Self>, tgt: &BroadcastTarget) -> impl IntoElement {
        let t = &self.theme;
        let idx = tgt.tab_index;
        let live = tgt.live;
        let checked = self.picked.contains(&idx);
        let mut row = h_flex()
            .id(SharedString::from(format!("bc-{idx}")))
            .items_center()
            .gap(t.sp2)
            .px(t.sp2)
            .py(px(4.0))
            .rounded(t.radius_sm);
        if live {
            row = row.cursor_pointer().hover(|s| s.bg(t.hover)).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    if !this.picked.remove(&idx) {
                        this.picked.insert(idx);
                    }
                    cx.notify();
                }),
            );
        }
        // Checkbox: an accent-filled box when picked, else an empty outline.
        let mut box_el = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(15.0))
            .h(px(15.0))
            .rounded(t.radius_sm)
            .border_1()
            .border_color(if checked { t.accent } else { t.line_3 });
        if checked {
            box_el = box_el.bg(t.accent).child(ui::icon("check", px(11.0), t.accent_ink));
        }
        row.child(box_el)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_color(if live { t.ink_2 } else { t.dim })
                    .child(tgt.title.clone()),
            )
            .when(!live, |d| {
                d.child(div().flex_none().text_size(t.fs_sm).text_color(t.dim).child("(not connected)"))
            })
    }
}

impl EventEmitter<DialogEvent> for BroadcastDialog {}

impl Render for BroadcastDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = self.theme.clone();

        let mut targets = v_flex().gap(px(2.0));
        if self.targets.is_empty() {
            targets = targets.child(note(&t, "No SSH tabs open."));
        } else {
            for tgt in &self.targets {
                targets = targets.child(self.target_row(cx, tgt));
            }
        }

        let toggle_id = if self.append_newline { "nl-on" } else { "nl-off" };
        let nl = self.append_newline;
        let newline_row = h_flex()
            .id(toggle_id)
            .items_center()
            .gap(t.sp2)
            .px(t.sp1)
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                    this.append_newline = !this.append_newline;
                    cx.notify();
                }),
            )
            .child({
                let mut b = div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .w(px(15.0))
                    .h(px(15.0))
                    .rounded(t.radius_sm)
                    .border_1()
                    .border_color(if nl { t.accent } else { t.line_3 });
                if nl {
                    b = b.bg(t.accent).child(ui::icon("check", px(11.0), t.accent_ink));
                }
                b
            })
            .child(
                div()
                    .text_size(t.fs_ui)
                    .text_color(t.ink_2)
                    .child("Append newline (run immediately on each host)"),
            );

        card(&t, 480.0)
            .track_focus(&self.focus)
            .key_context("Broadcast")
            .on_key_down(cx.listener(Self::on_key))
            .child(header(&t, cx, "square-terminal", "Broadcast to terminals"))
            .child(
                v_flex()
                    .p(t.sp4)
                    .gap(t.sp3)
                    .child(ui::section_label(&t, "TARGETS"))
                    .child(targets)
                    .child(labeled(
                        &t,
                        "COMMAND",
                        input_box(&t, cx, "bc-cmd", &self.command, "e.g. uptime ; df -h", false, |this, window, cx| {
                            window.focus(&this.focus, cx);
                            cx.notify();
                        }),
                    ))
                    .child(newline_row)
                    .child(
                        h_flex()
                            .items_center()
                            .gap(t.sp2)
                            .child(ui::icon("triangle-alert", px(13.0), t.warn))
                            .child(note(&t, "Fans the same command into every checked tab. Enter to send.")),
                    )
                    .when_some(self.status.clone(), |d, s| {
                        d.child(div().px(t.sp1).text_size(t.fs_sm).text_color(t.ink_2).child(s))
                    })
                    .child(
                        h_flex()
                            .justify_end()
                            .child(btn(&t, cx, "bc-send", "Send", true, |this, _w, cx| this.send(cx))),
                    ),
            )
    }
}

// ═══════════════════════════════════════════════════════════════════
// Egress profiles — CRUD over the connection store's egress list.
// ═══════════════════════════════════════════════════════════════════

#[derive(Clone, Copy, PartialEq)]
enum DraftKind {
    Direct,
    Socks5,
    Http,
    SshJump,
    Wireguard,
}

#[derive(Clone, Copy, PartialEq)]
enum EgressField {
    Name,
    Host,
    Port,
    ConfPath,
}

pub struct EgressDialog {
    profiles: Vec<EgressProfile>,
    /// Index into `profiles` being edited; `None` = a fresh draft.
    selected: Option<usize>,
    name: String,
    kind: DraftKind,
    host: String,
    port: String,
    via: String,
    conf_path: String,
    /// Saved SSH connection names, for the SSH-jump picker.
    conn_names: Vec<String>,
    field: EgressField,
    focus: FocusHandle,
    error: Option<String>,
    probe: Option<String>,
    probing: bool,
    theme: Theme,
}

impl EgressDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut s = Self {
            profiles: Vec::new(),
            selected: None,
            name: String::new(),
            kind: DraftKind::Socks5,
            host: String::new(),
            port: String::new(),
            via: String::new(),
            conf_path: String::new(),
            conn_names: Vec::new(),
            field: EgressField::Name,
            focus: cx.focus_handle(),
            error: None,
            probe: None,
            probing: false,
            theme: Theme::dark(),
        };
        s.reload_lists();
        s
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Re-read profiles + connection names from the store (called on open).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_lists();
        cx.notify();
    }

    fn reload_lists(&mut self) {
        self.profiles = ConnectionStore::load_default()
            .map(|s| s.egress_profiles)
            .unwrap_or_default();
        self.conn_names = data::connections_raw().into_iter().map(|c| c.name).collect();
    }

    fn field_buf(&mut self) -> &mut String {
        match self.field {
            EgressField::Name => &mut self.name,
            EgressField::Host => &mut self.host,
            EgressField::Port => &mut self.port,
            EgressField::ConfPath => &mut self.conf_path,
        }
    }

    fn new_draft(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.name.clear();
        self.kind = DraftKind::Socks5;
        self.host.clear();
        self.port.clear();
        self.via.clear();
        self.conf_path.clear();
        self.field = EgressField::Name;
        self.error = None;
        self.probe = None;
        cx.notify();
    }

    fn select(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(p) = self.profiles.get(i) else {
            return;
        };
        self.selected = Some(i);
        self.name = p.name.clone();
        self.host.clear();
        self.port.clear();
        self.via.clear();
        self.conf_path.clear();
        match &p.kind {
            EgressKind::None => self.kind = DraftKind::Direct,
            EgressKind::Socks5 { host, port, .. } => {
                self.kind = DraftKind::Socks5;
                self.host = host.clone();
                self.port = port.to_string();
            }
            EgressKind::Http { host, port, .. } => {
                self.kind = DraftKind::Http;
                self.host = host.clone();
                self.port = port.to_string();
            }
            EgressKind::SshJump { via_connection } => {
                self.kind = DraftKind::SshJump;
                self.via = via_connection.clone();
            }
            EgressKind::Wireguard { conf_path } => {
                self.kind = DraftKind::Wireguard;
                self.conf_path = conf_path.clone();
            }
            // ExternalVpn isn't edited in the spike — show it as Direct.
            EgressKind::ExternalVpn { .. } => self.kind = DraftKind::Direct,
        }
        self.field = EgressField::Name;
        self.error = None;
        self.probe = None;
        cx.notify();
    }

    fn port_num(&self) -> u16 {
        self.port.trim().parse().unwrap_or(1080)
    }

    fn build_kind(&self) -> EgressKind {
        match self.kind {
            DraftKind::Direct => EgressKind::None,
            DraftKind::Socks5 => EgressKind::Socks5 {
                host: self.host.trim().to_string(),
                port: self.port_num(),
                auth: None,
            },
            DraftKind::Http => EgressKind::Http {
                host: self.host.trim().to_string(),
                port: self.port_num(),
                auth: None,
            },
            DraftKind::SshJump => EgressKind::SshJump {
                via_connection: self.via.trim().to_string(),
            },
            DraftKind::Wireguard => EgressKind::Wireguard {
                conf_path: self.conf_path.trim().to_string(),
            },
        }
    }

    /// A slug id unique among the other profiles (so two same-named
    /// profiles don't silently overwrite each other on save).
    fn unique_id(&self, name: &str) -> String {
        let base: String = name
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let base = base.trim_matches('-').to_string();
        let base = if base.is_empty() { "egress".to_string() } else { base };
        let exists = |id: &str| self.profiles.iter().any(|p| p.id == id);
        if !exists(&base) {
            return base;
        }
        let mut n = 2;
        loop {
            let candidate = format!("{base}-{n}");
            if !exists(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Name must not be empty.".to_string());
        }
        match self.kind {
            DraftKind::Socks5 | DraftKind::Http if self.host.trim().is_empty() => {
                Err("Proxy host must not be empty.".to_string())
            }
            DraftKind::SshJump if self.via.trim().is_empty() => {
                Err("Choose a saved SSH connection to jump through.".to_string())
            }
            _ => Ok(()),
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.validate() {
            self.error = Some(e);
            self.probe = None;
            cx.notify();
            return;
        }
        let id = match self.selected {
            Some(i) => self.profiles[i].id.clone(),
            None => self.unique_id(&self.name),
        };
        let profile = EgressProfile {
            id: id.clone(),
            name: self.name.trim().to_string(),
            kind: self.build_kind(),
            dns: None,
        };
        let mut store = ConnectionStore::load_default().unwrap_or_default();
        store.upsert_egress(profile);
        match store.save_default() {
            Ok(()) => {
                self.reload_lists();
                self.selected = self.profiles.iter().position(|p| p.id == id);
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        cx.notify();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.selected else {
            return;
        };
        let id = self.profiles[i].id.clone();
        let mut store = ConnectionStore::load_default().unwrap_or_default();
        store.remove_egress(&id);
        match store.save_default() {
            Ok(()) => {
                self.reload_lists();
                self.new_draft(cx);
            }
            Err(e) => {
                self.error = Some(e.to_string());
                cx.notify();
            }
        }
    }

    /// Probe TCP reachability through the drafted profile (target
    /// 1.1.1.1:443, 5s). Most meaningful for SOCKS5/HTTP; ssh-jump and
    /// wireguard need a live context the dialog doesn't carry, so they
    /// report the underlying error.
    fn test(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.validate() {
            self.error = Some(e);
            self.probe = None;
            cx.notify();
            return;
        }
        let profile = EgressProfile {
            id: "probe".to_string(),
            name: "probe".to_string(),
            kind: self.build_kind(),
            dns: None,
        };
        self.probing = true;
        self.error = None;
        self.probe = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    egress::probe_tcp_blocking(
                        Some(&profile),
                        "1.1.1.1",
                        443,
                        Duration::from_secs(5),
                        None,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.probing = false;
                let ms = outcome.elapsed.as_millis();
                this.probe = Some(match outcome.result {
                    Ok(()) => format!("Reached 1.1.1.1:443 in {ms} ms"),
                    Err(e) => format!("Failed ({ms} ms): {e}"),
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            "escape" => {
                cx.emit(DialogEvent::Close);
                return;
            }
            "enter" => {
                self.save(cx);
                return;
            }
            "backspace" => {
                if self.field_buf().pop().is_some() {
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        if let Some(kc) = typed_char(ev) {
            self.field_buf().push_str(&kc);
            cx.notify();
        }
    }

    fn kind_label(k: DraftKind) -> &'static str {
        match k {
            DraftKind::Direct => "Direct",
            DraftKind::Socks5 => "SOCKS5",
            DraftKind::Http => "HTTP",
            DraftKind::SshJump => "SSH jump",
            DraftKind::Wireguard => "WireGuard",
        }
    }

    fn kind_seg(&self, cx: &mut Context<Self>, k: DraftKind) -> impl IntoElement {
        let t = &self.theme;
        let active = self.kind == k;
        div()
            .id(SharedString::from(format!("ek-{}", Self::kind_label(k))))
            .px(t.sp2)
            .py(px(4.0))
            .rounded(t.radius_sm)
            .text_size(t.fs_ui)
            .cursor_pointer()
            .when(active, |d| d.bg(t.accent).text_color(t.accent_ink))
            .when(!active, |d| d.bg(t.panel_2).text_color(t.ink_2).hover(|s| s.bg(t.elev)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.kind = k;
                    this.probe = None;
                    cx.notify();
                }),
            )
            .child(Self::kind_label(k))
    }

    fn profile_row(&self, cx: &mut Context<Self>, i: usize, p: &EgressProfile) -> impl IntoElement {
        let t = &self.theme;
        let selected = self.selected == Some(i);
        let kind = kind_tag(&p.kind);
        h_flex()
            .id(SharedString::from(format!("eprow-{i}")))
            .items_center()
            .gap(t.sp2)
            .px(t.sp2)
            .py(px(5.0))
            .rounded(t.radius_sm)
            .cursor_pointer()
            .when(selected, |d| d.bg(t.accent_dim))
            .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| this.select(i, cx)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(div().text_color(if selected { t.ink } else { t.ink_2 }).child(p.name.clone()))
                    .child(div().text_size(t.fs_sm).text_color(t.muted).child(kind)),
            )
    }

    fn via_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        if self.conn_names.is_empty() {
            return note(t, "No saved SSH connections to jump through.").into_any_element();
        }
        let mut wrap = h_flex().flex_wrap().gap(px(4.0));
        for name in &self.conn_names {
            let active = self.via == *name;
            let pick = name.clone();
            wrap = wrap.child(
                div()
                    .id(SharedString::from(format!("evia-{name}")))
                    .px(t.sp2)
                    .py(px(3.0))
                    .rounded(t.radius_sm)
                    .text_size(t.fs_ui)
                    .cursor_pointer()
                    .when(active, |d| d.bg(t.accent).text_color(t.accent_ink))
                    .when(!active, |d| d.bg(t.panel_2).text_color(t.ink_2).hover(|s| s.bg(t.elev)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.via = pick.clone();
                            cx.notify();
                        }),
                    )
                    .child(name.clone()),
            );
        }
        wrap.into_any_element()
    }

    fn kind_fields(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        match self.kind {
            DraftKind::Direct => note(t, "Direct connection — no tunnel.").into_any_element(),
            DraftKind::Socks5 | DraftKind::Http => h_flex()
                .gap(t.sp3)
                .child(
                    div().flex_1().child(labeled(
                        t,
                        "Proxy host",
                        input_box(t, cx, "eg-host", &self.host, "proxy.example.com", self.field == EgressField::Host, |this, window, cx| {
                            this.field = EgressField::Host;
                            window.focus(&this.focus, cx);
                            cx.notify();
                        }),
                    )),
                )
                .child(
                    div().w(px(96.0)).child(labeled(
                        t,
                        "Port",
                        input_box(t, cx, "eg-port", &self.port, "1080", self.field == EgressField::Port, |this, window, cx| {
                            this.field = EgressField::Port;
                            window.focus(&this.focus, cx);
                            cx.notify();
                        }),
                    )),
                )
                .into_any_element(),
            DraftKind::SshJump => labeled(t, "Jump host", self.via_picker(cx)).into_any_element(),
            DraftKind::Wireguard => v_flex()
                .gap(px(3.0))
                .child(labeled(
                    t,
                    "Conf path",
                    input_box(t, cx, "eg-conf", &self.conf_path, "/etc/wireguard/wg0.conf", self.field == EgressField::ConfPath, |this, window, cx| {
                        this.field = EgressField::ConfPath;
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }),
                ))
                .child(note(t, "Empty → ~/.config/pier-x/egress/<id>.conf. wg-quick runs as a system VPN (needs admin)."))
                .into_any_element(),
        }
    }
}

impl EventEmitter<DialogEvent> for EgressDialog {}

impl Render for EgressDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = self.theme.clone();

        // Left: profile list + "New".
        let mut list = v_flex().gap(px(2.0)).child(ui::section_label(&t, format!("PROFILES · {}", self.profiles.len())));
        if self.profiles.is_empty() {
            list = list.child(note(&t, "No egress profiles yet."));
        } else {
            for (i, p) in self.profiles.iter().enumerate() {
                list = list.child(self.profile_row(cx, i, p));
            }
        }
        let left = v_flex()
            .w(px(196.0))
            .flex_none()
            .h_full()
            .p(t.sp2)
            .bg(t.panel_2)
            .border_r_1()
            .border_color(t.line)
            .child(div().flex_1().min_h(px(0.0)).id("egress-list").overflow_y_scroll().child(list))
            .child(
                div().pt(t.sp2).child(btn(&t, cx, "eg-new", "+ New profile", false, |this, _w, cx| this.new_draft(cx))),
            );

        // Right: the draft form.
        let kinds = [DraftKind::Direct, DraftKind::Socks5, DraftKind::Http, DraftKind::SshJump, DraftKind::Wireguard];
        let mut seg = h_flex().flex_wrap().gap(px(4.0));
        for k in kinds {
            seg = seg.child(self.kind_seg(cx, k));
        }

        let mut footer = h_flex().items_center().gap(t.sp2);
        if self.selected.is_some() {
            footer = footer.child(btn(&t, cx, "eg-del", "Delete", false, |this, _w, cx| this.delete(cx)));
        }
        footer = footer
            .child(div().flex_1())
            .child(btn(&t, cx, "eg-test", if self.probing { "Testing…" } else { "Test" }, false, |this, _w, cx| this.test(cx)))
            .child(btn(&t, cx, "eg-save", "Save", true, |this, _w, cx| this.save(cx)));

        let form = v_flex()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .p(t.sp4)
            .gap(t.sp3)
            .child(labeled(
                &t,
                "Name",
                input_box(&t, cx, "eg-name", &self.name, "Office SOCKS", self.field == EgressField::Name, |this, window, cx| {
                    this.field = EgressField::Name;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }),
            ))
            .child(labeled(&t, "Kind", seg))
            .child(self.kind_fields(cx))
            .when_some(self.error.clone(), |d, e| {
                d.child(h_flex().items_center().gap(t.sp2).child(ui::icon("triangle-alert", px(13.0), t.neg)).child(div().text_size(t.fs_sm).text_color(t.neg).child(e)))
            })
            .when_some(self.probe.clone(), |d, p| {
                d.child(div().px(t.sp1).text_size(t.fs_sm).text_color(t.ink_2).child(p))
            })
            .child(div().flex_1())
            .child(footer);

        card(&t, 660.0)
            .h(px(440.0))
            .track_focus(&self.focus)
            .key_context("Egress")
            .on_key_down(cx.listener(Self::on_key))
            .child(header(&t, cx, "shield", "Egress profiles"))
            .child(h_flex().flex_1().min_h(px(0.0)).child(left).child(form))
    }
}

/// One-word kind tag for a profile row.
fn kind_tag(k: &EgressKind) -> &'static str {
    match k {
        EgressKind::None => "direct",
        EgressKind::Socks5 { .. } => "socks5",
        EgressKind::Http { .. } => "http",
        EgressKind::SshJump { .. } => "ssh-jump",
        EgressKind::Wireguard { .. } => "wireguard",
        EgressKind::ExternalVpn { .. } => "external-vpn",
    }
}

// ═══════════════════════════════════════════════════════════════════
// Port forwarding — local (ssh -L) tunnels over the active session.
// ═══════════════════════════════════════════════════════════════════

#[derive(Clone, Copy, PartialEq)]
enum TunnelField {
    RemoteHost,
    RemotePort,
    LocalPort,
}

/// A live forward the dialog owns. Dropping the [`Tunnel`] closes it.
struct TunnelEntry {
    local_port: u16,
    remote: String,
    tunnel: Tunnel,
}

pub struct TunnelDialog {
    /// `(label, session)` of the active SSH tab, pushed by the shell.
    active: Option<(String, SshSession)>,
    tunnels: Vec<TunnelEntry>,
    remote_host: String,
    remote_port: String,
    local_port: String,
    field: TunnelField,
    focus: FocusHandle,
    error: Option<String>,
    busy: bool,
    theme: Theme,
}

impl TunnelDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            active: None,
            tunnels: Vec::new(),
            remote_host: "127.0.0.1".to_string(),
            remote_port: "5432".to_string(),
            local_port: "0".to_string(),
            field: TunnelField::RemoteHost,
            focus: cx.focus_handle(),
            error: None,
            busy: false,
            theme: Theme::dark(),
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Point the dialog at the active SSH tab (called by the shell on
    /// open). Existing tunnels stay alive — the dialog owns them.
    pub fn set_active(&mut self, active: Option<(String, SshSession)>, cx: &mut Context<Self>) {
        self.active = active;
        self.error = None;
        cx.notify();
    }

    fn field_buf(&mut self) -> &mut String {
        match self.field {
            TunnelField::RemoteHost => &mut self.remote_host,
            TunnelField::RemotePort => &mut self.remote_port,
            TunnelField::LocalPort => &mut self.local_port,
        }
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        let Some((_, session)) = self.active.clone() else {
            self.error = Some("No active SSH session — open an SSH tab first.".to_string());
            cx.notify();
            return;
        };
        let rhost = self.remote_host.trim().to_string();
        if rhost.is_empty() {
            self.error = Some("Remote host must not be empty.".to_string());
            cx.notify();
            return;
        }
        let rport: u16 = match self.remote_port.trim().parse() {
            Ok(p) if p > 0 => p,
            _ => {
                self.error = Some("Remote port must be 1–65535.".to_string());
                cx.notify();
                return;
            }
        };
        let lport: u16 = self.local_port.trim().parse().unwrap_or(0);
        self.busy = true;
        self.error = None;
        cx.notify();
        let rhost_for_open = rhost.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    session
                        .open_local_forward_blocking(lport, &rhost_for_open, rport)
                        .map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match res {
                    Ok(tunnel) => {
                        let local_port = tunnel.local_port();
                        this.tunnels.push(TunnelEntry {
                            local_port,
                            remote: format!("{rhost}:{rport}"),
                            tunnel,
                        });
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn close_tunnel(&mut self, i: usize, cx: &mut Context<Self>) {
        if i < self.tunnels.len() {
            self.tunnels.remove(i); // drops the Tunnel → stops the listener
            cx.notify();
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            "escape" => {
                cx.emit(DialogEvent::Close);
                return;
            }
            "enter" => {
                self.open(cx);
                return;
            }
            "backspace" => {
                if self.field_buf().pop().is_some() {
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        if let Some(kc) = typed_char(ev) {
            self.field_buf().push_str(&kc);
            cx.notify();
        }
    }

    fn tunnel_row(&self, cx: &mut Context<Self>, i: usize, e: &TunnelEntry) -> impl IntoElement {
        let t = &self.theme;
        let alive = e.tunnel.is_alive();
        h_flex()
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .py(px(6.0))
            .rounded(t.radius_sm)
            .bg(t.panel_2)
            .child(ui::status_dot(if alive { t.pos } else { t.neg }))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink_2)
                    .child(format!("127.0.0.1:{} → {}", e.local_port, e.remote)),
            )
            .child(div().flex_none().text_size(t.fs_sm).text_color(if alive { t.muted } else { t.neg }).child(if alive { "alive" } else { "dead" }))
            .child(btn(t, cx, &format!("tun-close-{i}"), "Close", false, move |this, _w, cx| this.close_tunnel(i, cx)))
    }

    fn port_field(&self, cx: &mut Context<Self>, id: &str, value: &str, placeholder: &'static str, which: TunnelField) -> impl IntoElement {
        let t = &self.theme;
        input_box(t, cx, id, value, placeholder, self.field == which, move |this, window, cx| {
            this.field = which;
            window.focus(&this.focus, cx);
            cx.notify();
        })
    }
}

impl EventEmitter<DialogEvent> for TunnelDialog {}

impl Render for TunnelDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = self.theme.clone();

        let via_label = self
            .active
            .as_ref()
            .map(|(l, _)| format!("via {l}"))
            .unwrap_or_else(|| "No active SSH session".to_string());

        let mut active_list = v_flex().gap(px(4.0));
        if self.tunnels.is_empty() {
            active_list = active_list.child(note(&t, "No active tunnels."));
        } else {
            for (i, e) in self.tunnels.iter().enumerate() {
                active_list = active_list.child(self.tunnel_row(cx, i, e));
            }
        }

        let form = h_flex()
            .gap(t.sp3)
            .child(div().flex_1().child(labeled(&t, "Remote host", self.port_field(cx, "tun-rh", &self.remote_host.clone(), "127.0.0.1", TunnelField::RemoteHost))))
            .child(div().w(px(90.0)).child(labeled(&t, "Remote port", self.port_field(cx, "tun-rp", &self.remote_port.clone(), "5432", TunnelField::RemotePort))))
            .child(div().w(px(90.0)).child(labeled(&t, "Local port", self.port_field(cx, "tun-lp", &self.local_port.clone(), "0", TunnelField::LocalPort))));

        card(&t, 480.0)
            .track_focus(&self.focus)
            .key_context("Tunnel")
            .on_key_down(cx.listener(Self::on_key))
            .child(header(&t, cx, "network", "Port forwarding"))
            .child(
                v_flex()
                    .p(t.sp4)
                    .gap(t.sp3)
                    .child(note(&t, "Local forwards only (ssh -L): a local listener proxied into the SSH session."))
                    .child(
                        h_flex()
                            .items_center()
                            .gap(t.sp2)
                            .child(ui::section_label(&t, "ACTIVE TUNNELS"))
                            .child(div().flex_1())
                            .child(div().text_size(t.fs_sm).text_color(t.muted).child(via_label)),
                    )
                    .child(active_list)
                    .child(ui::section_label(&t, "OPEN NEW TUNNEL"))
                    .child(form)
                    .child(note(&t, "Local port 0 lets the OS pick a free port."))
                    .when_some(self.error.clone(), |d, e| {
                        d.child(div().px(t.sp1).text_size(t.fs_sm).text_color(t.neg).child(e))
                    })
                    .child(
                        h_flex().justify_end().child(btn(
                            &t,
                            cx,
                            "tun-open",
                            if self.busy { "Opening…" } else { "Open Tunnel" },
                            true,
                            |this, _w, cx| this.open(cx),
                        )),
                    ),
            )
    }
}

// ═══════════════════════════════════════════════════════════════════
// Host health — TCP reachability across saved connections + deep probe.
// ═══════════════════════════════════════════════════════════════════

pub struct HostsHealthDialog {
    conns: Vec<SshConfig>,
    reports: HashMap<usize, HostHealthReport>,
    deep: HashMap<usize, HostDeepProbeReport>,
    /// Live sessions keyed by `user@host`, pushed by the shell from open
    /// SSH tabs — deep probe rides one of these (no fresh auth).
    sessions: HashMap<String, SshSession>,
    filter: String,
    focus: FocusHandle,
    busy: bool,
    theme: Theme,
}

impl HostsHealthDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            conns: data::connections_raw(),
            reports: HashMap::new(),
            deep: HashMap::new(),
            sessions: HashMap::new(),
            filter: String::new(),
            focus: cx.focus_handle(),
            busy: false,
            theme: Theme::dark(),
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Refresh the saved-connection list + the cached sessions map
    /// (called by the shell on open), then re-probe.
    pub fn set_sessions(&mut self, sessions: HashMap<String, SshSession>, cx: &mut Context<Self>) {
        self.conns = data::connections_raw();
        self.sessions = sessions;
        cx.notify();
        if !self.conns.is_empty() {
            self.refresh(cx);
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.conns.is_empty() {
            return;
        }
        let targets: Vec<HostHealthTarget> = self
            .conns
            .iter()
            .enumerate()
            .map(|(i, c)| HostHealthTarget {
                saved_connection_index: i,
                host: c.host.clone(),
                port: c.port,
            })
            .collect();
        self.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let reports = cx
                .background_executor()
                .spawn(async move { host_health::probe_many_blocking(targets, 3000) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                for r in reports {
                    this.reports.insert(r.saved_connection_index, r);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn session_for(&self, c: &SshConfig) -> Option<SshSession> {
        self.sessions.get(&format!("{}@{}", c.user, c.host)).cloned()
    }

    fn deep_probe(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(c) = self.conns.get(idx) else {
            return;
        };
        let Some(session) = self.session_for(c) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let report = cx
                .background_executor()
                .spawn(async move {
                    // deep_probe is async over the cached session; drive it on
                    // pier-core's shared tokio runtime (gpui's executor is not one).
                    pier_core::ssh::runtime::shared().block_on(host_health::deep_probe(idx, &session))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.deep.insert(idx, report);
                cx.notify();
            });
        })
        .detach();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            "escape" => {
                cx.emit(DialogEvent::Close);
                return;
            }
            "backspace" => {
                if self.filter.pop().is_some() {
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        if let Some(kc) = typed_char(ev) {
            self.filter.push_str(&kc);
            cx.notify();
        }
    }

    fn matches(&self, c: &SshConfig) -> bool {
        let q = self.filter.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        let group = c.group.clone().unwrap_or_default();
        c.name.to_lowercase().contains(&q)
            || c.host.to_lowercase().contains(&q)
            || c.user.to_lowercase().contains(&q)
            || group.to_lowercase().contains(&q)
    }

    fn host_row(&self, cx: &mut Context<Self>, idx: usize, c: &SshConfig) -> impl IntoElement {
        let t = &self.theme;
        let report = self.reports.get(&idx);
        let (dot, status_txt) = match report.map(|r| r.status) {
            Some(HealthStatus::Online) => (t.pos, "Online".to_string()),
            Some(HealthStatus::Offline) => (t.neg, "Offline".to_string()),
            Some(HealthStatus::Timeout) => (t.neg, "Timeout".to_string()),
            Some(HealthStatus::Error) => (t.warn, "Error".to_string()),
            None => (t.muted, if self.busy { "Probing…".to_string() } else { "Unknown".to_string() }),
        };
        let latency = report
            .and_then(|r| r.latency_ms)
            .map(|ms| format!("{ms} ms"))
            .unwrap_or_default();
        let has_session = self.session_for(c).is_some();
        let addr = format!("{}@{}:{}", c.user, c.host, c.port);

        let mut top = h_flex()
            .items_center()
            .gap(t.sp2)
            .child(ui::status_dot(dot))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(div().text_color(t.ink_2).child(c.name.clone()))
                    .child(div().font_family(t.mono.clone()).text_size(t.fs_sm).text_color(t.muted).child(addr)),
            )
            .child(div().flex_none().text_size(t.fs_sm).text_color(t.muted).child(latency))
            .child(div().flex_none().w(px(56.0)).text_size(t.fs_sm).text_color(t.ink_2).child(status_txt));
        if has_session {
            top = top.child(btn(t, cx, &format!("hh-deep-{idx}"), "Deep", false, move |this, _w, cx| this.deep_probe(idx, cx)));
        } else {
            top = top.child(div().flex_none().text_size(t.fs_sm).text_color(t.dim).child("no session"));
        }

        let mut row = v_flex()
            .gap(px(3.0))
            .px(t.sp3)
            .py(px(6.0))
            .rounded(t.radius_sm)
            .hover(|s| s.bg(t.hover))
            .child(top);
        if let Some(d) = self.deep.get(&idx) {
            let line = |label: &str, val: &Option<String>| -> Option<String> {
                val.as_ref().map(|v| format!("{label} {v}"))
            };
            let parts: Vec<String> = [
                line("up", &d.uptime),
                line("load", &d.load_avg),
                d.disk_root_use.as_ref().map(|u| format!("disk {u}")),
                line("·", &d.distro),
            ]
            .into_iter()
            .flatten()
            .collect();
            if !parts.is_empty() {
                row = row.child(div().pl(px(15.0)).text_size(t.fs_sm).text_color(t.muted).child(parts.join("   ")));
            }
        }
        row
    }
}

impl EventEmitter<DialogEvent> for HostsHealthDialog {}

impl Render for HostsHealthDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = self.theme.clone();

        let online = self.reports.values().filter(|r| r.status == HealthStatus::Online).count();
        let down = self
            .reports
            .values()
            .filter(|r| matches!(r.status, HealthStatus::Offline | HealthStatus::Timeout | HealthStatus::Error))
            .count();
        let unknown = self.conns.len().saturating_sub(online + down);
        let summary = format!("Online {online} · Down {down} · Unknown {unknown} · {} total", self.conns.len());

        let mut rows = v_flex().gap(px(2.0));
        let visible: Vec<(usize, &SshConfig)> =
            self.conns.iter().enumerate().filter(|(_, c)| self.matches(c)).collect();
        if self.conns.is_empty() {
            rows = rows.child(note(&t, "No saved SSH connections yet."));
        } else if visible.is_empty() {
            rows = rows.child(note(&t, "No saved connections match your filter."));
        } else {
            for (idx, c) in visible {
                rows = rows.child(self.host_row(cx, idx, c));
            }
        }

        card(&t, 600.0)
            .h(px(460.0))
            .track_focus(&self.focus)
            .key_context("HostsHealth")
            .on_key_down(cx.listener(Self::on_key))
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .w_full()
                    .h(px(40.0))
                    .px(t.sp4)
                    .border_b_1()
                    .border_color(t.line)
                    .child(ui::icon("activity", px(16.0), t.accent))
                    .child(div().flex_1().min_w(px(0.0)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child("Host health"))
                    .child(btn(&t, cx, "hh-refresh", if self.busy { "Probing…" } else { "Refresh" }, false, |this, _w, cx| this.refresh(cx)))
                    .child(close_btn(&t, cx)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        v_flex()
                            .px(t.sp4)
                            .py(t.sp2)
                            .gap(t.sp2)
                            .child(div().text_size(t.fs_sm).text_color(t.muted).child(summary))
                            .child(input_box(&t, cx, "hh-filter", &self.filter, "Filter by name, host, user, group…", false, |this, window, cx| {
                                window.focus(&this.focus, cx);
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1().min_h(px(0.0)).id("hh-list").overflow_y_scroll().px(t.sp2).pb(t.sp2).child(rows)),
            )
    }
}
