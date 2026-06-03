// Firewall panel — read-only firewall / listening-port overview for a host.
//
// Flow: pick a saved SSH connection, open a blocking session off the render
// path, then run pier-core's firewall probe once. The resulting
// FirewallSnapshot (backend, listening ports, interface counters, default
// policies) is cached on the View and rendered from there — render never
// blocks. This panel only reads; rule add/remove is out of scope here.

use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, FontWeight, MouseButton, MouseDownEvent, SharedString, Window};
use gpui_component::{h_flex, v_flex};

use pier_core::services::firewall::{FirewallBackend, FirewallSnapshot, ListeningPort};
use pier_core::ssh::SshConfig;

use crate::data;
use crate::theme::Theme;
use crate::ui;

pub struct FirewallPanel {
    theme: Theme,
    /// Saved SSH configs, loaded once at construction (cheap JSON read).
    conns: Vec<SshConfig>,
    /// Index of the connection the user picked, if any.
    selected: Option<usize>,
    /// A connect + probe is in flight.
    loading: bool,
    /// Last successful snapshot for `selected`.
    snapshot: Option<FirewallSnapshot>,
    /// Connect / probe failure, shown as a single neg-coloured line.
    error: Option<String>,
    /// Bumped per request so a stale background result can't overwrite a
    /// newer selection.
    generation: u64,
}

impl FirewallPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
            conns: data::connections_raw(),
            selected: None,
            loading: false,
            snapshot: None,
            error: None,
            generation: 0,
        }
    }

    /// Pick a connection and kick off the off-thread connect + probe.
    fn select(&mut self, idx: usize, cx: &mut Context<Self>) {
        self.selected = Some(idx);
        self.loading = true;
        self.snapshot = None;
        self.error = None;
        self.generation += 1;
        let gen = self.generation;
        let cfg = self.conns[idx].clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let session = data::connect_blocking(&cfg)?;
                    pier_core::services::firewall::snapshot_blocking(&session)
                        .map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != gen {
                    return; // a newer selection superseded this request
                }
                this.loading = false;
                match result {
                    Ok(snap) => {
                        this.snapshot = Some(snap);
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Clear the current host and return to the connection picker.
    fn back(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.loading = false;
        self.snapshot = None;
        self.error = None;
        self.generation += 1;
        cx.notify();
    }

    /// The server picker — one selectable row per saved connection.
    fn connection_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let mut col =
            v_flex().child(ui::section_label(t, format!("SELECT SERVER · {}", self.conns.len())));
        for (i, c) in self.conns.iter().enumerate() {
            col = col.child(self.conn_row(cx, i, c));
        }
        col
    }

    fn conn_row(&self, cx: &mut Context<Self>, idx: usize, c: &SshConfig) -> impl IntoElement {
        let t = &self.theme;
        let selected = self.selected == Some(idx);
        let addr = format!("{}@{}:{}", c.user, c.host, c.port);
        h_flex()
            .id(SharedString::from(format!("fwconn-{idx}")))
            .items_center()
            .gap(t.sp2)
            .h(px(42.0))
            .px(t.sp3)
            .when(selected, |d| d.bg(t.accent_dim))
            .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| this.select(idx, cx)),
            )
            .child(ui::icon("server", px(14.0), if selected { t.accent } else { t.muted }))
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

    /// A "‹ Servers · host" strip that returns to the picker.
    fn back_bar(&self, cx: &mut Context<Self>, host: String) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .id("fw-back")
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .py(t.sp2)
            .border_b_1()
            .border_color(t.line)
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| this.back(cx)),
            )
            .child(ui::icon("chevron-left", px(14.0), t.muted))
            .child(
                div()
                    .text_size(t.fs_ui)
                    .text_color(t.ink_2)
                    .child(format!("Servers · {host}")),
            )
    }

    /// One listening-port row: open dot, port + proto + process (all mono),
    /// bind address dimmed on the right.
    fn port_row(&self, idx: usize, p: &ListeningPort) -> impl IntoElement {
        let t = &self.theme;
        let process = if p.process.is_empty() {
            "—".to_string()
        } else {
            p.process.clone()
        };
        h_flex()
            .id(SharedString::from(format!("fwport-{idx}")))
            .items_center()
            .gap(t.sp2)
            .h(px(24.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .child(ui::status_dot(t.pos))
            .child(
                div()
                    .w(px(52.0))
                    .flex_none()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink_2)
                    .child(p.local_port.to_string()),
            )
            .child(
                div()
                    .w(px(34.0))
                    .flex_none()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(p.proto.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink_2)
                    .child(process),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child(p.local_addr.clone()),
            )
    }

    /// The rendered snapshot: backend posture, default policies, listening
    /// ports, and interface counters.
    fn snapshot_body(&self, cx: &mut Context<Self>, snap: &FirewallSnapshot) -> impl IntoElement {
        let t = &self.theme;
        let host = self
            .selected
            .map(|i| self.conns[i].host.clone())
            .unwrap_or_default();

        let user = if snap.user.is_empty() {
            "—".to_string()
        } else if snap.root {
            format!("{} (root)", snap.user)
        } else {
            snap.user.clone()
        };
        let uname = if snap.uname.is_empty() {
            "—".to_string()
        } else {
            snap.uname.clone()
        };

        let mut col = v_flex()
            .child(self.back_bar(cx, host))
            .child(ui::section_label(t, "FIREWALL"))
            // Backend posture: active/inactive dot + name + state text.
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .px(t.sp3)
                    .py(px(3.0))
                    .child(ui::status_dot(if snap.backend_active {
                        t.pos
                    } else {
                        t.muted
                    }))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.ink_2)
                            .child(backend_name(snap.backend)),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(if snap.backend_active {
                                "active"
                            } else {
                                "inactive"
                            }),
                    ),
            );
        if !snap.backend_status.is_empty() {
            col = col.child(ui::info_row(t, "Status", snap.backend_status.clone()));
        }
        col = col
            .child(ui::info_row(t, "User", user))
            .child(ui::info_row(t, "Kernel", uname));

        if !snap.default_policies.is_empty() {
            col = col.child(ui::section_label(t, "DEFAULT POLICY"));
            for (chain, policy) in &snap.default_policies {
                col = col.child(ui::info_row(t, chain.clone(), policy.clone()));
            }
        }

        col = col.child(ui::section_label(
            t,
            format!("LISTENING PORTS · {}", snap.listening.len()),
        ));
        if snap.listening.is_empty() {
            col = col.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child("None readable (try a root session)"),
            );
        } else {
            for (i, p) in snap.listening.iter().enumerate() {
                col = col.child(self.port_row(i, p));
            }
        }

        col = col.child(ui::section_label(
            t,
            format!("INTERFACES · {}", snap.interfaces.len()),
        ));
        for iface in &snap.interfaces {
            col = col.child(ui::info_row(
                t,
                iface.iface.clone(),
                format!(
                    "↓{} ↑{}",
                    fmt_bytes(iface.rx_bytes),
                    fmt_bytes(iface.tx_bytes)
                ),
            ));
        }

        col.pb(t.sp3)
    }

    /// Header right-aligned meta: host while connected, else server count.
    fn header_meta(&self) -> String {
        match (&self.snapshot, self.selected) {
            (Some(_), Some(i)) => self.conns[i].host.clone(),
            (None, Some(i)) if self.loading => self.conns[i].host.clone(),
            _ => format!("{} servers", self.conns.len()),
        }
    }
}

impl Render for FirewallPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let meta = self.header_meta();

        let body: AnyElement = if let Some(err) = self.error.clone() {
            v_flex()
                .child(
                    div()
                        .px(t.sp3)
                        .py(t.sp2)
                        .text_size(t.fs_ui)
                        .text_color(t.neg)
                        .child(format!("✗ {err}")),
                )
                .child(self.connection_selector(cx))
                .into_any_element()
        } else if self.loading {
            let host = self
                .selected
                .map(|i| self.conns[i].host.clone())
                .unwrap_or_default();
            ui::empty_state(t, format!("Connecting to {host} …")).into_any_element()
        } else if let Some(snap) = self.snapshot.clone() {
            self.snapshot_body(cx, &snap).into_any_element()
        } else if self.conns.is_empty() {
            ui::empty_state(t, "No saved connections").into_any_element()
        } else {
            self.connection_selector(cx).into_any_element()
        };

        v_flex()
            .size_full()
            .child(ui::panel_header(t, "shield", "FIREWALL", meta))
            .child(
                div()
                    .id("fw-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}

/// Display label for a detected firewall backend.
fn backend_name(b: FirewallBackend) -> &'static str {
    match b {
        FirewallBackend::Firewalld => "firewalld",
        FirewallBackend::Ufw => "ufw",
        FirewallBackend::Nftables => "nftables",
        FirewallBackend::Iptables => "iptables",
        FirewallBackend::None => "none detected",
    }
}

/// Human-readable byte size with a binary suffix (B/K/M/G/T).
fn fmt_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let f = n as f64;
    if f >= TB {
        format!("{:.1}T", f / TB)
    } else if f >= GB {
        format!("{:.1}G", f / GB)
    } else if f >= MB {
        format!("{:.1}M", f / MB)
    } else if f >= KB {
        format!("{:.1}K", f / KB)
    } else {
        format!("{n}B")
    }
}
