// Docker panel — read-only container list over an SSH session.
//
// Flow: render a connection selector from the saved SSH configs
// (data::connections_raw). On pick, connect off the render path
// (data::connect_blocking) and list containers
// (pier_core::services::docker::list_containers_blocking) on the
// background executor, then cache the session + rows on the View and
// notify. Per-container start/stop/restart run over the cached session and
// refresh the list.

use gpui::prelude::*;
use gpui::{div, px, Context, FontWeight, MouseButton, MouseDownEvent, SharedString, Window};
use gpui_component::{h_flex, v_flex};

use pier_core::services::docker::{
    list_containers_blocking, restart_blocking, start_blocking, stop_blocking, Container,
};
use pier_core::ssh::{SshConfig, SshSession};

use crate::data;
use crate::theme::Theme;
use crate::ui;

/// A container lifecycle action.
#[derive(Clone, Copy)]
enum CtrOp {
    Start,
    Stop,
    Restart,
}

pub struct DockerPanel {
    theme: Theme,
    /// Saved SSH targets, loaded once at construction.
    conns: Vec<SshConfig>,
    /// Index of the connection currently selected / connecting.
    selected: Option<usize>,
    /// True while the connect + list round-trip is in flight.
    connecting: bool,
    /// Live session for the selected host, cached once connected.
    session: Option<SshSession>,
    /// Container rows from the last successful `docker ps -a`.
    containers: Vec<Container>,
    /// One-line failure from connect or listing, shown in `t.neg`.
    error: Option<String>,
}

impl DockerPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
            conns: data::connections_raw(),
            selected: None,
            connecting: false,
            session: None,
            containers: Vec::new(),
            error: None,
        }
    }

    /// Connect to `conns[idx]` and list its containers off the render
    /// path. The blocking SSH calls run on the background executor; the
    /// result is written back to the View and `cx.notify()`d.
    fn connect(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(cfg) = self.conns.get(idx).cloned() else {
            return;
        };
        self.selected = Some(idx);
        self.connecting = true;
        self.error = None;
        self.session = None;
        self.containers.clear();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let session = data::connect_blocking(&cfg)?;
                    let containers =
                        list_containers_blocking(&session, true).map_err(|e| e.to_string())?;
                    Ok::<(SshSession, Vec<Container>), String>((session, containers))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.connecting = false;
                match result {
                    Ok((session, containers)) => {
                        this.session = Some(session);
                        this.containers = containers;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// One selectable SSH target: status dot + name + `user@host:port`.
    fn conn_row(&self, cx: &mut Context<Self>, idx: usize, c: &SshConfig) -> impl IntoElement {
        let t = &self.theme;
        let selected = self.selected == Some(idx);
        let addr = format!("{}@{}:{}", c.user, c.host, c.port);
        h_flex()
            .id(SharedString::from(format!("dconn-{idx}")))
            .items_center()
            .gap(t.sp2)
            .h(px(42.0))
            .px(t.sp3)
            .when(selected, |d| d.bg(t.accent_dim))
            .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.connect(idx, cx);
                }),
            )
            .child(ui::status_dot(t.muted))
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

    /// Run a lifecycle action on `id`, then refresh the list. Network calls run
    /// on the background executor with a cloned (Arc-backed) session.
    fn container_op(&mut self, op: CtrOp, id: String, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    match op {
                        CtrOp::Start => start_blocking(&session, &id),
                        CtrOp::Stop => stop_blocking(&session, &id),
                        CtrOp::Restart => restart_blocking(&session, &id),
                    }
                    .map_err(|e| e.to_string())?;
                    list_containers_blocking(&session, true).map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(list) => {
                        this.containers = list;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A small icon button running a container op.
    fn ctr_btn(
        &self,
        cx: &mut Context<Self>,
        key: &str,
        glyph: &'static str,
        color: gpui::Hsla,
        op: CtrOp,
        id: String,
    ) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(SharedString::from(format!("dop-{key}")))
            .flex()
            .items_center()
            .justify_center()
            .w(px(20.0))
            .h(px(20.0))
            .rounded(t.radius_sm)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.container_op(op, id.clone(), cx)
                }),
            )
            .child(ui::icon(glyph, px(14.0), color))
    }

    /// One container: running dot + name, then image and ports.
    fn container_row(&self, cx: &mut Context<Self>, c: &Container) -> impl IntoElement {
        let t = &self.theme;
        let dot = if c.is_running() { t.pos } else { t.muted };
        let running = c.is_running();
        let id = c.id.clone();
        let mut meta = h_flex()
            .gap(t.sp2)
            .font_family(t.mono.clone())
            .text_size(t.fs_sm)
            .text_color(t.muted)
            .child(div().overflow_hidden().child(c.image.clone()));
        if !c.ports.is_empty() {
            meta = meta.child(div().overflow_hidden().child(c.ports.clone()));
        }
        h_flex()
            .id(SharedString::from(format!("dctr-{}", c.id)))
            .items_center()
            .gap(t.sp2)
            .py(px(6.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .child(ui::status_dot(dot))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .overflow_hidden()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.ink)
                            .child(c.names.clone()),
                    )
                    .child(meta),
            )
            .when(running, |d| {
                d.child(self.ctr_btn(cx, &format!("stop-{}", c.id), "pause", t.warn, CtrOp::Stop, id.clone()))
                    .child(self.ctr_btn(cx, &format!("rst-{}", c.id), "redo-2", t.info, CtrOp::Restart, id.clone()))
            })
            .when(!running, |d| {
                d.child(self.ctr_btn(cx, &format!("start-{}", c.id), "play", t.pos, CtrOp::Start, id.clone()))
            })
    }
}

impl Render for DockerPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = &self.theme;
        let count = if self.session.is_some() {
            self.containers.len().to_string()
        } else {
            String::new()
        };

        let mut col = v_flex()
            .flex_1()
            .min_h(px(0.0))
            .child(ui::panel_header(t, "container", "DOCKER", count));

        if let Some(err) = &self.error {
            col = col.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_ui)
                    .text_color(t.neg)
                    .child(err.clone()),
            );
        }

        if self.connecting {
            col = col.child(ui::empty_state(t, "Connecting…"));
        } else if self.session.is_some() {
            if self.containers.is_empty() {
                col = col.child(ui::empty_state(t, "No containers"));
            } else {
                col = col.child(ui::section_label(t, format!("CONTAINERS · {}", self.containers.len())));
                for c in &self.containers {
                    col = col.child(self.container_row(cx, c));
                }
            }
        } else if self.conns.is_empty() {
            col = col.child(ui::empty_state(t, "No saved connections"));
        } else {
            col = col.child(ui::section_label(t, format!("SERVERS · {}", self.conns.len())));
            for (i, c) in self.conns.iter().enumerate() {
                col = col.child(self.conn_row(cx, i, c));
            }
        }

        div()
            .id("docker-scroll")
            .size_full()
            .overflow_y_scroll()
            .child(col)
    }
}
