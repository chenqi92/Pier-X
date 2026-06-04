// Firewall panel — firewall / listening-port overview for a host.
//
// Flow: pick a saved SSH connection, open a blocking session off the render
// path, then run pier-core's firewall probe. The resulting FirewallSnapshot
// (backend, listening ports, interface counters, default policies, raw
// iptables dumps) is cached on the View with the live session and rendered
// from there — render never blocks.
//
// The body is split into four inline tabs that mirror the web panel:
//   • Listening — open TCP/UDP sockets, each with a bind-scope badge and a
//     trailing "Block" button that writes the backend-appropriate deny
//     command to the clipboard (command-injection style; never executed).
//   • Rules — host posture + default policies + `iptables-save` rules grouped
//     by chain, rendered read-only.
//   • Mappings — DNAT / port-forward rules parsed out of the nat table.
//   • Traffic — per-interface RX/TX rates, sampled every 2 s while the tab is
//     visible, with a simple bar sparkline over the recent history.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, ClipboardItem, Context, FontWeight, Hsla, MouseButton, MouseDownEvent,
    SharedString, Window,
};
use gpui_component::{h_flex, v_flex};

use pier_core::services::firewall::{
    FirewallBackend, FirewallSnapshot, InterfaceCounter, ListeningPort,
};
use pier_core::ssh::{SshConfig, SshSession};

use crate::data;
use crate::theme::Theme;
use crate::ui;

/// Traffic-tab poll cadence; matches the web panel's 2 s interval.
const TRAFFIC_POLL: Duration = Duration::from_millis(2000);
/// How many rate samples to keep per interface for the sparkline.
const RATE_HISTORY_LEN: usize = 48;

/// The four inline pages, switched via the top chips.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FwTab {
    Listening,
    Rules,
    Mappings,
    Traffic,
}

impl FwTab {
    fn label(self) -> &'static str {
        match self {
            FwTab::Listening => "Listening",
            FwTab::Rules => "Rules",
            FwTab::Mappings => "Mappings",
            FwTab::Traffic => "Traffic",
        }
    }
}

const FW_TABS: [FwTab; 4] = [
    FwTab::Listening,
    FwTab::Rules,
    FwTab::Mappings,
    FwTab::Traffic,
];

pub struct FirewallPanel {
    theme: Theme,
    /// Saved SSH configs, loaded once at construction (cheap JSON read).
    conns: Vec<SshConfig>,
    /// Index of the connection the user picked, if any.
    selected: Option<usize>,
    /// A connect + probe is in flight.
    loading: bool,
    /// Live session for the selected host, kept so the Traffic tab can re-probe.
    session: Option<SshSession>,
    /// Last successful snapshot for `selected`.
    snapshot: Option<FirewallSnapshot>,
    /// Connect / probe failure, shown as a single neg-coloured line.
    error: Option<String>,
    /// Bumped per connect so a stale background result can't overwrite a
    /// newer selection.
    generation: u64,
    /// Currently visible inline tab.
    tab: FwTab,
    /// Per-interface current RX/TX byte rates (bytes/sec).
    rates: HashMap<String, (f64, f64)>,
    /// Per-interface recent RX/TX rate history for the sparklines.
    history: HashMap<String, (VecDeque<f64>, VecDeque<f64>)>,
    /// Bumped to invalidate a running Traffic poll loop.
    traffic_gen: u64,
    /// Last command copied to the clipboard, shown as a transient note.
    copied: Option<String>,
    /// Bumped per copy so an older auto-clear timer can't clear a newer note.
    copy_gen: u64,
}

impl FirewallPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
            conns: data::connections_raw(),
            selected: None,
            loading: false,
            session: None,
            snapshot: None,
            error: None,
            generation: 0,
            tab: FwTab::Listening,
            rates: HashMap::new(),
            history: HashMap::new(),
            traffic_gen: 0,
            copied: None,
            copy_gen: 0,
        }
    }

    /// Pick a connection and kick off the off-thread connect + probe.
    fn select(&mut self, idx: usize, cx: &mut Context<Self>) {
        self.selected = Some(idx);
        self.loading = true;
        self.session = None;
        self.snapshot = None;
        self.error = None;
        self.tab = FwTab::Listening;
        self.rates.clear();
        self.history.clear();
        self.generation += 1;
        self.traffic_gen += 1; // stop any poll left over from a prior host
        let gen = self.generation;
        let cfg = self.conns[idx].clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let session = data::connect_blocking(&cfg)?;
                    let snap = pier_core::services::firewall::snapshot_blocking(&session)
                        .map_err(|e| e.to_string())?;
                    Ok::<(SshSession, FirewallSnapshot), String>((session, snap))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != gen {
                    return; // a newer selection superseded this request
                }
                this.loading = false;
                match result {
                    Ok((session, snap)) => {
                        this.session = Some(session);
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
        self.session = None;
        self.snapshot = None;
        self.error = None;
        self.tab = FwTab::Listening;
        self.rates.clear();
        self.history.clear();
        self.generation += 1;
        self.traffic_gen += 1;
        cx.notify();
    }

    /// Switch the visible tab; starts the Traffic poll when entering Traffic.
    fn set_tab(&mut self, tab: FwTab, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        if tab == FwTab::Traffic {
            self.start_traffic_poll(cx);
        }
        cx.notify();
    }

    /// Re-probe the host every `TRAFFIC_POLL` while the Traffic tab is open,
    /// diffing interface counters into per-second rates. The loop ends when the
    /// view is dropped, the tab changes, or a newer poll supersedes it.
    fn start_traffic_poll(&mut self, cx: &mut Context<Self>) {
        self.traffic_gen += 1;
        let mine = self.traffic_gen;
        cx.spawn(async move |this, cx| loop {
            // Grab a session clone only while we're still the active poll on
            // the Traffic tab; otherwise stop.
            let session = match this.update(cx, |this, _cx| {
                if this.traffic_gen != mine || this.tab != FwTab::Traffic {
                    None
                } else {
                    this.session.clone()
                }
            }) {
                Ok(Some(s)) => s,
                _ => break,
            };
            let snap = cx
                .background_executor()
                .spawn(async move {
                    pier_core::services::firewall::snapshot_blocking(&session)
                        .map_err(|e| e.to_string())
                })
                .await;
            let alive = this
                .update(cx, |this, cx| {
                    if this.traffic_gen != mine {
                        return;
                    }
                    match snap {
                        Ok(s) => {
                            this.ingest_traffic(s);
                            this.error = None;
                        }
                        // Keep the last good snapshot on a transient failure so
                        // the view doesn't collapse; just surface the message.
                        Err(e) => this.error = Some(e),
                    }
                    cx.notify();
                })
                .is_ok();
            if !alive {
                break;
            }
            cx.background_executor().timer(TRAFFIC_POLL).await;
        })
        .detach();
    }

    /// Fold a fresh snapshot in: derive per-interface rates from the previous
    /// snapshot's counters, append to the history rings, then store it.
    fn ingest_traffic(&mut self, snap: FirewallSnapshot) {
        if let Some(prev) = self.snapshot.as_ref() {
            let dt_ms = snap.captured_at_ms.saturating_sub(prev.captured_at_ms);
            if prev.captured_at_ms > 0 && dt_ms > 0 {
                let dt = dt_ms as f64 / 1000.0;
                let mut next = HashMap::new();
                for cur in &snap.interfaces {
                    let Some(p) = prev.interfaces.iter().find(|x| x.iface == cur.iface) else {
                        continue;
                    };
                    let d_rx = cur.rx_bytes.saturating_sub(p.rx_bytes) as f64;
                    let d_tx = cur.tx_bytes.saturating_sub(p.tx_bytes) as f64;
                    let rx = if d_rx > 0.0 { d_rx / dt } else { 0.0 };
                    let tx = if d_tx > 0.0 { d_tx / dt } else { 0.0 };
                    next.insert(cur.iface.clone(), (rx, tx));
                    let entry = self.history.entry(cur.iface.clone()).or_default();
                    push_capped(&mut entry.0, rx, RATE_HISTORY_LEN);
                    push_capped(&mut entry.1, tx, RATE_HISTORY_LEN);
                }
                self.rates = next;
            }
        }
        self.snapshot = Some(snap);
    }

    /// Build the backend-appropriate deny command and put it on the clipboard.
    /// Nothing is executed — the user pastes and runs it in their terminal.
    fn copy_block_cmd(&mut self, proto: String, port: u16, cx: &mut Context<Self>) {
        let (backend, root) = self
            .snapshot
            .as_ref()
            .map(|s| (s.backend, s.root))
            .unwrap_or((FirewallBackend::None, false));
        let cmd = build_block_cmd(backend, &proto, port, !root);
        cx.write_to_clipboard(ClipboardItem::new_string(cmd.clone()));
        self.copied = Some(cmd);
        self.copy_gen += 1;
        let mine = self.copy_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(4)).await;
            let _ = this.update(cx, |this, cx| {
                if this.copy_gen == mine {
                    this.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    // ── Connection picker ────────────────────────────────────────────

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

    // ── Host view chrome ─────────────────────────────────────────────

    /// Backend posture strip: dot + backend name + active/inactive state.
    fn backend_strip(&self, snap: &FirewallSnapshot) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .py(t.sp2)
            .border_b_1()
            .border_color(t.line)
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
                    .text_color(if snap.backend_active { t.pos } else { t.muted })
                    .child(if snap.backend_active {
                        "active"
                    } else {
                        "inactive"
                    }),
            )
    }

    /// The Listening / Rules / Mappings / Traffic chip row.
    fn tab_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let mut row = h_flex()
            .gap(t.sp1)
            .px(t.sp3)
            .py(t.sp2)
            .border_b_1()
            .border_color(t.line);
        for tab in FW_TABS {
            row = row.child(self.tab_chip(cx, tab));
        }
        row
    }

    fn tab_chip(&self, cx: &mut Context<Self>, tab: FwTab) -> impl IntoElement {
        let t = &self.theme;
        let active = self.tab == tab;
        div()
            .id(SharedString::from(format!("fwtab-{}", tab.label())))
            .px(t.sp2)
            .py(px(3.0))
            .rounded(t.radius_sm)
            .cursor_pointer()
            .text_size(t.fs_ui)
            .when(active, |d| d.bg(t.accent_dim).text_color(t.ink))
            .when(!active, |d| d.text_color(t.muted).hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| this.set_tab(tab, cx)),
            )
            .child(tab.label())
    }

    /// Transient "Copied: <cmd>" note shown after a Block button is pressed.
    fn copied_note(&self, cmd: &str) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .py(t.sp2)
            .bg(t.accent_subtle)
            .border_b_1()
            .border_color(t.line)
            .child(ui::icon("circle-check", px(13.0), t.accent))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink_2)
                    .child(format!("Copied: {cmd}")),
            )
    }

    /// One-line neg-coloured error banner kept inside the host view.
    fn error_note(&self, err: &str) -> impl IntoElement {
        let t = &self.theme;
        div()
            .px(t.sp3)
            .py(t.sp2)
            .text_size(t.fs_ui)
            .text_color(t.neg)
            .child(format!("✗ {err}"))
    }

    // ── Small shared atoms ───────────────────────────────────────────

    /// A coloured pill: tone text on a faint tint of the same tone.
    fn badge(&self, text: impl Into<SharedString>, fg: Hsla) -> impl IntoElement {
        let t = &self.theme;
        div()
            .flex_none()
            .px(px(5.0))
            .py(px(1.0))
            .rounded(t.radius_sm)
            .bg(tint(fg, 0.14))
            .text_size(t.fs_sm)
            .text_color(fg)
            .child(text.into())
    }

    /// A dim, padded line for empty states and footnotes.
    fn note(&self, text: impl Into<SharedString>) -> impl IntoElement {
        let t = &self.theme;
        div()
            .px(t.sp3)
            .py(t.sp2)
            .text_size(t.fs_sm)
            .text_color(t.dim)
            .child(text.into())
    }

    // ── Listening tab ────────────────────────────────────────────────

    fn listening_tab(&self, cx: &mut Context<Self>, snap: &FirewallSnapshot) -> AnyElement {
        let t = &self.theme;
        let mut col = v_flex().child(ui::section_label(
            t,
            format!("LISTENING PORTS · {}", snap.listening.len()),
        ));
        if snap.listening.is_empty() {
            col = col.child(self.note("None readable (try a root session)"));
        } else {
            for (i, p) in snap.listening.iter().enumerate() {
                col = col.child(self.port_row(cx, i, p));
            }
        }
        col.pb(t.sp3).into_any_element()
    }

    /// One listening socket: scope-tinted dot, addr:port · proto, scope badge,
    /// process/pid sub-line, and a trailing Block button.
    fn port_row(&self, cx: &mut Context<Self>, idx: usize, p: &ListeningPort) -> impl IntoElement {
        let t = &self.theme;
        let (scope_label, scope_color) = bind_scope(&self.theme, &p.local_addr);
        let proc = if p.process.is_empty() {
            "(unknown — root needed)".to_string()
        } else {
            p.process.clone()
        };
        let sub = match p.pid {
            Some(pid) => format!("{proc} · pid {pid}"),
            None => proc,
        };
        h_flex()
            .id(SharedString::from(format!("fwport-{idx}")))
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .py(px(6.0))
            .hover(|s| s.bg(t.hover))
            .child(ui::status_dot(scope_color))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .items_center()
                            .gap(t.sp1)
                            .child(
                                div()
                                    .font_family(t.mono.clone())
                                    .text_size(t.fs_sm)
                                    .text_color(t.ink_2)
                                    .child(format!("{}:{}", p.local_addr, p.local_port)),
                            )
                            .child(
                                div()
                                    .font_family(t.mono.clone())
                                    .text_size(t.fs_sm)
                                    .text_color(t.muted)
                                    .child(format!("· {}", p.proto)),
                            )
                            .child(self.badge(scope_label, scope_color)),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(sub),
                    ),
            )
            .child(self.block_btn(cx, idx, p))
    }

    /// A red "Block" pill that copies the deny command to the clipboard.
    fn block_btn(
        &self,
        cx: &mut Context<Self>,
        idx: usize,
        p: &ListeningPort,
    ) -> impl IntoElement {
        let t = &self.theme;
        let port = p.local_port;
        let proto = if p.proto.starts_with("udp") {
            "udp".to_string()
        } else {
            "tcp".to_string()
        };
        h_flex()
            .id(SharedString::from(format!("fwblock-{idx}")))
            .flex_none()
            .items_center()
            .gap(px(3.0))
            .px(t.sp2)
            .py(px(3.0))
            .rounded(t.radius_sm)
            .cursor_pointer()
            .text_size(t.fs_sm)
            .text_color(t.neg)
            .hover(|s| s.bg(tint(t.neg, 0.12)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.copy_block_cmd(proto.clone(), port, cx)
                }),
            )
            .child(ui::icon("copy", px(11.0), t.neg))
            .child("Block")
    }

    // ── Rules tab ────────────────────────────────────────────────────

    fn rules_tab(&self, snap: &FirewallSnapshot) -> AnyElement {
        let t = &self.theme;
        let mut col = v_flex();

        // Host posture: status / user / kernel.
        col = col.child(ui::section_label(t, "HOST"));
        if !snap.backend_status.is_empty() {
            col = col.child(ui::info_row(t, "Status", snap.backend_status.clone()));
        }
        col = col
            .child(ui::info_row(t, "User", user_label(snap)))
            .child(ui::info_row(t, "Kernel", kernel_label(snap)));

        // Default chain policies as coloured badges.
        if !snap.default_policies.is_empty() {
            col = col.child(ui::section_label(t, "DEFAULT POLICY"));
            let mut row = h_flex().flex_wrap().gap(t.sp1).px(t.sp3).py(t.sp1);
            for (chain, policy) in &snap.default_policies {
                // Coloured by security posture, not action: a DROP default policy
                // is deny-by-default (the safe stance) so it reads green here —
                // the deliberate inverse of `action_tone`, where a DROP *rule*
                // blocks traffic and reads red. REJECT denies too but advertises
                // the host, so amber.
                let color = match policy.as_str() {
                    "DROP" => t.pos,
                    "REJECT" => t.warn,
                    _ => t.muted,
                };
                row = row.child(self.badge(format!("{chain}: {policy}"), color));
            }
            col = col.child(row);
        }

        // Filter-table rules grouped by chain.
        let rules = parse_rules(&snap.rules_v4);
        let groups = group_filter_rules(&rules);
        let filter_count: usize = groups.iter().map(|(_, rs)| rs.len()).sum();
        col = col.child(ui::section_label(t, format!("FILTER RULES · {filter_count}")));
        if groups.is_empty() {
            col = col.child(self.note(if snap.rules_v4.is_empty() {
                "No rules readable (try a root session)"
            } else {
                "No filter rules — only default policies apply"
            }));
        } else {
            for (chain, rs) in &groups {
                col = col.child(self.chain_head(chain, rs.len()));
                for r in rs {
                    col = col.child(self.rule_row(r));
                }
            }
        }

        // IPv6 rules, if any were readable.
        let v6 = parse_rules(&snap.rules_v6);
        if !v6.is_empty() {
            col = col.child(ui::section_label(t, format!("IPv6 RULES · {}", v6.len())));
            for r in &v6 {
                col = col.child(self.rule_row(r));
            }
        }

        col.pb(t.sp3).into_any_element()
    }

    fn chain_head(&self, chain: &str, count: usize) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .pt(t.sp2)
            .pb(px(2.0))
            .child(self.badge(chain.to_string(), t.info))
            .child(
                div()
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(format!("{count} rules")),
            )
    }

    fn rule_row(&self, r: &ParsedRule) -> impl IntoElement {
        let t = &self.theme;
        let tone = action_tone(t, &r.action);
        let action = if r.action.is_empty() {
            "—".to_string()
        } else {
            r.action.clone()
        };
        h_flex()
            .items_start()
            .gap(t.sp2)
            .px(t.sp3)
            .py(px(4.0))
            .hover(|s| s.bg(t.hover))
            .child(self.badge(action, tone))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .gap(px(1.0))
                    .child(
                        div()
                            .text_size(t.fs_ui)
                            .text_color(t.ink_2)
                            .child(rule_summary(r)),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.dim)
                            .child(r.body.clone()),
                    ),
            )
            .when(r.pkts > 0, |d| {
                d.child(
                    div()
                        .flex_none()
                        .font_family(t.mono.clone())
                        .text_size(t.fs_sm)
                        .text_color(t.muted)
                        .child(r.pkts.to_string()),
                )
            })
    }

    // ── Mappings tab ─────────────────────────────────────────────────

    fn mappings_tab(&self, snap: &FirewallSnapshot) -> AnyElement {
        let t = &self.theme;
        let maps = parse_mappings(&snap.nat_v4);
        let mut col = v_flex().child(ui::section_label(t, format!("PORT MAPPINGS · {}", maps.len())));
        if maps.is_empty() {
            col = col.child(self.note("No DNAT / port mappings detected"));
        } else {
            for m in &maps {
                col = col.child(self.mapping_row(m));
            }
        }
        col = col.child(
            div()
                .px(t.sp3)
                .py(t.sp2)
                .text_size(t.fs_sm)
                .text_color(t.dim)
                .child(
                    "DOCKER chain rules are managed by the Docker daemon — edit container port \
                     maps via the Docker panel.",
                ),
        );
        col.pb(t.sp3).into_any_element()
    }

    fn mapping_row(&self, m: &PortMapping) -> impl IntoElement {
        let t = &self.theme;
        let chain_color = if m.chain == "DOCKER" { t.info } else { t.muted };
        h_flex()
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .py(px(6.0))
            .hover(|s| s.bg(t.hover))
            .child(ui::icon("arrow-right", px(14.0), t.accent))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .gap(px(1.0))
                    .child(
                        div()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.ink_2)
                            .child(format!(
                                ":{}/{} → {}:{}",
                                m.external_port, m.proto, m.internal_addr, m.internal_port
                            )),
                    )
                    .child(h_flex().child(self.badge(m.chain.clone(), chain_color))),
            )
    }

    // ── Traffic tab ──────────────────────────────────────────────────

    fn traffic_tab(&self, snap: &FirewallSnapshot) -> AnyElement {
        let t = &self.theme;
        let mut col = v_flex().child(ui::section_label(
            t,
            format!("INTERFACES · {}", snap.interfaces.len()),
        ));
        if snap.interfaces.is_empty() {
            col = col.child(self.note("No interfaces detected"));
        } else {
            for iface in &snap.interfaces {
                col = col.child(self.iface_row(iface));
            }
        }
        col = col.child(
            div()
                .px(t.sp3)
                .py(t.sp2)
                .text_size(t.fs_sm)
                .text_color(t.dim)
                .child("Sampling /proc/net/dev every 2 s while this tab is visible. Loopback is hidden."),
        );
        col.pb(t.sp3).into_any_element()
    }

    fn iface_row(&self, iface: &InterfaceCounter) -> impl IntoElement {
        let t = &self.theme;
        let (rx, tx) = self
            .rates
            .get(&iface.iface)
            .copied()
            .unwrap_or((-1.0, -1.0));
        let hist = self.history.get(&iface.iface);
        v_flex()
            .gap(px(4.0))
            .px(t.sp3)
            .py(t.sp2)
            .border_b_1()
            .border_color(t.line)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .child(ui::icon("activity", px(13.0), t.accent))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_ui)
                            .text_color(t.ink_2)
                            .child(iface.iface.clone()),
                    )
                    .child(self.rate_tag("arrow-down", t.info, rx))
                    .child(self.rate_tag("arrow-up", t.warn, tx)),
            )
            .child(self.sparkline(hist.map(|h| &h.0), t.info))
            .child(self.sparkline(hist.map(|h| &h.1), t.warn))
            .child(
                div()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child(format!(
                        "Σ ↓{} ↑{}",
                        fmt_bytes(iface.rx_bytes),
                        fmt_bytes(iface.tx_bytes)
                    )),
            )
    }

    fn rate_tag(&self, glyph: &'static str, color: Hsla, bps: f64) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(px(2.0))
            .child(ui::icon(glyph, px(11.0), color))
            .child(
                div()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(fmt_bps(bps)),
            )
    }

    /// A simple bottom-anchored bar chart over the recent rate history. Renders
    /// a flat baseline until at least two samples have accumulated.
    fn sparkline(&self, values: Option<&VecDeque<f64>>, color: Hsla) -> AnyElement {
        let t = &self.theme;
        let height = 22.0_f32;
        let vals: Vec<f64> = values.map(|v| v.iter().copied().collect()).unwrap_or_default();
        if vals.len() < 2 {
            return div()
                .w_full()
                .h(px(height))
                .flex()
                .items_center()
                .child(div().w_full().h(px(1.0)).bg(t.line))
                .into_any_element();
        }
        let max = vals.iter().copied().fold(1.0_f64, f64::max);
        let mut row = h_flex().w_full().h(px(height)).items_end().gap(px(1.0));
        for v in vals {
            let frac = (v / max).clamp(0.0, 1.0) as f32;
            let bar_h = (frac * height).max(1.0);
            row = row.child(div().flex_1().h(px(bar_h)).rounded(px(1.0)).bg(color));
        }
        row.into_any_element()
    }

    /// The full tabbed view for a connected host (fixed chrome + scroll body).
    fn host_view(&self, cx: &mut Context<Self>, snap: &FirewallSnapshot) -> impl IntoElement {
        let host = self
            .selected
            .map(|i| self.conns[i].host.clone())
            .unwrap_or_default();

        let mut col = v_flex()
            .flex_1()
            .min_h(px(0.0))
            .child(self.back_bar(cx, host))
            .child(self.backend_strip(snap));
        if let Some(cmd) = self.copied.clone() {
            col = col.child(self.copied_note(&cmd));
        }
        if let Some(err) = self.error.clone() {
            col = col.child(self.error_note(&err));
        }
        col = col.child(self.tab_chips(cx));

        let content: AnyElement = match self.tab {
            FwTab::Listening => self.listening_tab(cx, snap),
            FwTab::Rules => self.rules_tab(snap),
            FwTab::Mappings => self.mappings_tab(snap),
            FwTab::Traffic => self.traffic_tab(snap),
        };

        col.child(
            div()
                .id("fw-scroll")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(content),
        )
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
        self.theme = cx.global::<Theme>().clone();
        let t = &self.theme;
        let meta = self.header_meta();

        let mut root = v_flex()
            .size_full()
            .child(ui::panel_header(t, "shield", "FIREWALL", meta));

        if let Some(snap) = self.snapshot.clone() {
            root = root.child(self.host_view(cx, &snap));
        } else {
            let inner: AnyElement = if self.loading {
                let host = self
                    .selected
                    .map(|i| self.conns[i].host.clone())
                    .unwrap_or_default();
                ui::empty_state(t, format!("Connecting to {host} …")).into_any_element()
            } else if let Some(err) = self.error.clone() {
                v_flex()
                    .child(self.error_note(&err))
                    .child(self.connection_selector(cx))
                    .into_any_element()
            } else if self.conns.is_empty() {
                ui::empty_state(t, "No saved connections").into_any_element()
            } else {
                self.connection_selector(cx).into_any_element()
            };
            root = root.child(
                div()
                    .id("fw-scroll-sel")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(inner),
            );
        }

        root
    }
}

// ── Free helpers (pure) ──────────────────────────────────────────────

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

fn user_label(snap: &FirewallSnapshot) -> String {
    if snap.user.is_empty() {
        "—".to_string()
    } else if snap.root {
        format!("{} (root)", snap.user)
    } else {
        snap.user.clone()
    }
}

fn kernel_label(snap: &FirewallSnapshot) -> String {
    if snap.uname.is_empty() {
        "—".to_string()
    } else {
        snap.uname.clone()
    }
}

/// Classify a bind address into an exposure label + tone. `0.0.0.0`/`::`/`*`
/// are internet-reachable (warn); loopback is local-only (muted); anything
/// else is treated as LAN (info).
fn bind_scope(t: &Theme, addr: &str) -> (&'static str, Hsla) {
    if addr == "0.0.0.0" || addr == "::" || addr == "*" {
        ("Public", t.warn)
    } else if addr == "127.0.0.1" || addr == "::1" || addr.starts_with("127.") {
        ("Local", t.muted)
    } else {
        ("LAN", t.info)
    }
}

/// The backend-appropriate "deny this port" command. Prefixed with `sudo`
/// when the SSH user isn't root.
fn build_block_cmd(backend: FirewallBackend, proto: &str, port: u16, needs_sudo: bool) -> String {
    let sudo = if needs_sudo { "sudo " } else { "" };
    match backend {
        FirewallBackend::Ufw => format!("{sudo}ufw deny {port}/{proto}"),
        FirewallBackend::Firewalld => format!(
            "{sudo}firewall-cmd --permanent --remove-port={port}/{proto} && {sudo}firewall-cmd --reload"
        ),
        FirewallBackend::Nftables => {
            format!("{sudo}nft add rule inet filter input {proto} dport {port} drop")
        }
        _ => format!("{sudo}iptables -I INPUT -p {proto} --dport {port} -j DROP"),
    }
}

/// Bytes/sec with a binary suffix; `—` for an unknown (negative) rate.
fn fmt_bps(bps: f64) -> String {
    if !bps.is_finite() || bps < 0.0 {
        return "—".to_string();
    }
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    if bps >= MB {
        format!("{:.1} MB/s", bps / MB)
    } else if bps >= KB {
        format!("{:.1} KB/s", bps / KB)
    } else {
        format!("{bps:.0} B/s")
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

/// Set the alpha of a token colour to derive a faint tint (badge/hover fill).
fn tint(mut c: Hsla, a: f32) -> Hsla {
    c.a = a;
    c
}

fn push_capped(buf: &mut VecDeque<f64>, v: f64, cap: usize) {
    buf.push_back(v);
    while buf.len() > cap {
        buf.pop_front();
    }
}

// ── iptables-save parsing ────────────────────────────────────────────

/// One `-A CHAIN …` rule lifted out of an `iptables-save -c` dump.
struct ParsedRule {
    table: String,
    chain: String,
    action: String,
    proto: Option<String>,
    source: Option<String>,
    destination: Option<String>,
    dport: Option<String>,
    iface: Option<String>,
    out_iface: Option<String>,
    pkts: u64,
    /// The rule line minus the `[pkts:bytes]` counter prefix.
    body: String,
}

/// The value following `flag` in a token list, if present.
fn flag_val(tokens: &[&str], flag: &str) -> Option<String> {
    tokens
        .iter()
        .position(|t| *t == flag)
        .and_then(|i| tokens.get(i + 1))
        .map(|s| s.to_string())
}

/// Strip a leading `[pkts:bytes]` counter token, returning the packet count.
fn take_counter(tokens: &mut Vec<&str>) -> u64 {
    let is_counter = tokens
        .first()
        .map(|f| f.starts_with('[') && f.ends_with(']') && f.contains(':'))
        .unwrap_or(false);
    if !is_counter {
        return 0;
    }
    let token = tokens.remove(0);
    let inner = &token[1..token.len() - 1];
    inner
        .split_once(':')
        .and_then(|(p, _)| p.parse().ok())
        .unwrap_or(0)
}

/// Parse the `-A` rule lines from an `iptables-save -c` dump. Chain default
/// policies (`:CHAIN …`), table markers (`*filter`), comments and `COMMIT`
/// are skipped.
fn parse_rules(dump: &str) -> Vec<ParsedRule> {
    let mut out = Vec::new();
    let mut table = "filter".to_string();
    for raw in dump.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('*') {
            table = rest.trim().to_string();
            continue;
        }
        if line == "COMMIT" || line.starts_with(':') || line.starts_with('#') {
            continue;
        }
        let mut tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let pkts = take_counter(&mut tokens);
        if tokens.first().copied() != Some("-A") {
            continue;
        }
        let chain = tokens.get(1).copied().unwrap_or("").to_string();
        if chain.is_empty() {
            continue;
        }
        out.push(ParsedRule {
            table: table.clone(),
            chain,
            action: flag_val(&tokens, "-j").unwrap_or_default(),
            proto: flag_val(&tokens, "-p"),
            source: flag_val(&tokens, "-s"),
            destination: flag_val(&tokens, "-d"),
            dport: flag_val(&tokens, "--dport"),
            iface: flag_val(&tokens, "-i"),
            out_iface: flag_val(&tokens, "-o"),
            pkts,
            body: tokens.join(" "),
        });
    }
    out
}

/// Group filter-table rules by chain, preserving first-seen chain order.
fn group_filter_rules(rules: &[ParsedRule]) -> Vec<(String, Vec<&ParsedRule>)> {
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, Vec<&ParsedRule>> = HashMap::new();
    for r in rules {
        if r.table != "filter" {
            continue;
        }
        if !map.contains_key(&r.chain) {
            order.push(r.chain.clone());
        }
        map.entry(r.chain.clone()).or_default().push(r);
    }
    order
        .into_iter()
        .map(|c| {
            let rs = map.remove(&c).unwrap_or_default();
            (c, rs)
        })
        .collect()
}

/// Render a parsed rule as one short English sentence (raw body shown below).
fn rule_summary(r: &ParsedRule) -> String {
    let dir = match r.chain.as_str() {
        "INPUT" => "inbound",
        "OUTPUT" => "outbound",
        "FORWARD" => "forwarded",
        other => other,
    };
    let proto_up = r
        .proto
        .as_deref()
        .filter(|p| *p != "all")
        .map(|p| p.to_uppercase())
        .unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    if let Some(dp) = &r.dport {
        let label = if proto_up.is_empty() { "port" } else { &proto_up };
        parts.push(format!("{label} {dp}"));
    } else if !proto_up.is_empty() {
        parts.push(proto_up.clone());
    }
    if let Some(s) = &r.source {
        parts.push(format!("from {s}"));
    }
    if let Some(d) = &r.destination {
        parts.push(format!("to {d}"));
    }
    if let Some(i) = &r.iface {
        parts.push(format!("on {i}"));
    } else if let Some(o) = &r.out_iface {
        parts.push(format!("via {o}"));
    }
    parts.push(dir.to_string());
    let cond = parts.join(" ");
    match r.action.as_str() {
        "ACCEPT" => format!("Allow {cond}"),
        "DROP" => format!("Drop {cond}"),
        "REJECT" => format!("Reject {cond}"),
        "LOG" => format!("Log {cond}"),
        "DNAT" => format!("DNAT {cond}"),
        "SNAT" => format!("SNAT {cond}"),
        "MASQUERADE" => format!("Masquerade {cond}"),
        "RETURN" => format!("Return {cond}"),
        "" => cond,
        a => format!("{a} {cond}"),
    }
}

/// Pick a tone for a rule action badge.
fn action_tone(t: &Theme, action: &str) -> Hsla {
    match action {
        "ACCEPT" | "RETURN" => t.pos,
        "DROP" => t.neg,
        "REJECT" => t.warn,
        "LOG" | "DNAT" | "SNAT" | "MASQUERADE" => t.info,
        _ => t.muted,
    }
}

/// One DNAT / port-forward rule lifted out of the nat-table dump.
struct PortMapping {
    proto: String,
    external_port: String,
    internal_addr: String,
    internal_port: String,
    /// `DOCKER` for Docker-managed maps, `PREROUTING` for hand-rolled DNAT.
    chain: String,
}

/// Pull `-A DOCKER`/`-A PREROUTING … -j DNAT --to-destination IP:PORT` lines
/// out of an `iptables-save -c -t nat` dump.
fn parse_mappings(dump: &str) -> Vec<PortMapping> {
    let mut out = Vec::new();
    for raw in dump.lines() {
        let mut tokens: Vec<&str> = raw.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let _ = take_counter(&mut tokens);
        if tokens.first().copied() != Some("-A") {
            continue;
        }
        let chain = tokens.get(1).copied().unwrap_or("");
        if chain != "DOCKER" && chain != "PREROUTING" {
            continue;
        }
        if !tokens.iter().any(|t| *t == "DNAT") {
            continue;
        }
        let (Some(proto), Some(dport), Some(dest)) = (
            flag_val(&tokens, "-p"),
            flag_val(&tokens, "--dport"),
            flag_val(&tokens, "--to-destination"),
        ) else {
            continue;
        };
        if proto != "tcp" && proto != "udp" {
            continue;
        }
        let (addr, iport) = split_dest(&dest);
        out.push(PortMapping {
            proto,
            external_port: dport,
            internal_addr: addr,
            internal_port: iport,
            chain: chain.to_string(),
        });
    }
    out
}

/// Split a `--to-destination` value into `(address, port)`.
///
/// Handles IPv4 `1.2.3.4[:port]`, bracketed IPv6 `[fd00::2][:port]`, and bare
/// IPv6 `fd00::2` — a plain colon split mangles the latter two. Port defaults
/// to `"0"` when absent.
fn split_dest(dest: &str) -> (String, String) {
    // Bracketed IPv6: `[addr]` or `[addr]:port`.
    if let Some(rest) = dest.strip_prefix('[') {
        if let Some((addr, tail)) = rest.split_once(']') {
            let port = tail.strip_prefix(':').filter(|p| !p.is_empty());
            return (addr.to_string(), port.unwrap_or("0").to_string());
        }
    }
    // Bare IPv6 (two or more colons, unbracketed) never carries a port.
    if dest.matches(':').count() > 1 {
        return (dest.to_string(), "0".to_string());
    }
    // IPv4 with optional `:port`.
    match dest.split_once(':') {
        Some((addr, port)) => (addr.to_string(), port.to_string()),
        None => (dest.to_string(), "0".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_cmd_per_backend() {
        assert_eq!(
            build_block_cmd(FirewallBackend::Ufw, "tcp", 22, false),
            "ufw deny 22/tcp"
        );
        assert_eq!(
            build_block_cmd(FirewallBackend::Iptables, "tcp", 80, true),
            "sudo iptables -I INPUT -p tcp --dport 80 -j DROP"
        );
        assert_eq!(
            build_block_cmd(FirewallBackend::Nftables, "udp", 53, false),
            "nft add rule inet filter input udp dport 53 drop"
        );
    }

    #[test]
    fn parses_counter_and_rule() {
        let dump = "*filter\n:INPUT ACCEPT [0:0]\n[3:180] -A INPUT -p tcp -m tcp --dport 22 -j ACCEPT\nCOMMIT\n";
        let rules = parse_rules(dump);
        assert_eq!(rules.len(), 1);
        let r = &rules[0];
        assert_eq!(r.chain, "INPUT");
        assert_eq!(r.action, "ACCEPT");
        assert_eq!(r.dport.as_deref(), Some("22"));
        assert_eq!(r.pkts, 3);
        assert_eq!(r.body, "-A INPUT -p tcp -m tcp --dport 22 -j ACCEPT");
        assert_eq!(rule_summary(r), "Allow TCP 22 inbound");
    }

    #[test]
    fn parses_docker_mapping_with_counter() {
        let dump = "[0:0] -A DOCKER ! -i docker0 -p tcp -m tcp --dport 8080 -j DNAT --to-destination 172.17.0.2:80\n";
        let maps = parse_mappings(dump);
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].external_port, "8080");
        assert_eq!(maps[0].internal_addr, "172.17.0.2");
        assert_eq!(maps[0].internal_port, "80");
        assert_eq!(maps[0].chain, "DOCKER");
    }

    #[test]
    fn splits_dnat_destinations() {
        assert_eq!(split_dest("172.17.0.2:80"), ("172.17.0.2".to_string(), "80".to_string()));
        assert_eq!(split_dest("172.17.0.2"), ("172.17.0.2".to_string(), "0".to_string()));
        assert_eq!(split_dest("[fd00::2]:8443"), ("fd00::2".to_string(), "8443".to_string()));
        assert_eq!(split_dest("[fd00::2]"), ("fd00::2".to_string(), "0".to_string()));
        assert_eq!(split_dest("fd00::2"), ("fd00::2".to_string(), "0".to_string()));
    }

    #[test]
    fn parses_ipv6_dnat_mapping() {
        let dump =
            "[0:0] -A PREROUTING -p tcp -m tcp --dport 443 -j DNAT --to-destination [fd00::2]:8443\n";
        let maps = parse_mappings(dump);
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].internal_addr, "fd00::2");
        assert_eq!(maps[0].internal_port, "8443");
        assert_eq!(maps[0].chain, "PREROUTING");
    }
}
