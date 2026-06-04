// Docker panel — container + image management over an SSH session.
//
// Flow: render a connection selector from the saved SSH configs
// (data::connections_raw). On pick, connect off the render path
// (data::connect_blocking) and list containers
// (pier_core::services::docker::list_containers_blocking) on the
// background executor, then cache the session + rows on the View and
// notify. Per-container start/stop/restart/remove run over the cached
// session and refresh the list; container logs (`docker logs --tail 200`)
// expand inline; a Containers/Images toggle lists `docker images`. Every
// blocking call runs on cx.background_executor().

use gpui::prelude::*;
use gpui::{div, px, Context, FontWeight, Hsla, MouseButton, MouseDownEvent, SharedString, Window};
use gpui_component::{h_flex, v_flex};

use pier_core::services::docker::{
    exec_blocking, list_containers_blocking, list_images_blocking, remove_blocking,
    restart_blocking, start_blocking, stop_blocking, Container, DockerImage,
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

/// Which resource list the connected view is showing.
#[derive(Clone, Copy, PartialEq)]
enum DockerTab {
    Containers,
    Images,
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
    /// Active resource list (Containers / Images).
    tab: DockerTab,
    /// Image rows from `docker images`, fetched lazily on first Images view.
    images: Vec<DockerImage>,
    /// True once images have been fetched for the current session.
    images_loaded: bool,
    /// True while the `docker images` round-trip is in flight.
    images_loading: bool,
    /// Container id awaiting a remove confirmation, if any.
    confirm_remove: Option<String>,
    /// Container id whose logs are expanded inline, if any.
    logs_for: Option<String>,
    /// Captured `docker logs` output for [`Self::logs_for`].
    logs_text: String,
    /// True while a `docker logs` round-trip is in flight.
    logs_loading: bool,
    /// One-line failure from connect or listing, shown in `t.neg`.
    error: Option<String>,
    /// Bumped per connect so a stale background result (from a host the user
    /// has since switched away from) can't overwrite a newer selection.
    generation: u64,
}

impl DockerPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            // Placeholder; Render reassigns this from cx.global::<Theme>() on
            // the first (and every) frame, before anything is painted.
            theme: Theme::dark(),
            conns: data::connections_raw(),
            selected: None,
            connecting: false,
            session: None,
            containers: Vec::new(),
            tab: DockerTab::Containers,
            images: Vec::new(),
            images_loaded: false,
            images_loading: false,
            confirm_remove: None,
            logs_for: None,
            logs_text: String::new(),
            logs_loading: false,
            error: None,
            generation: 0,
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
        // Reset per-session view state so a new host starts clean.
        self.tab = DockerTab::Containers;
        self.images.clear();
        self.images_loaded = false;
        self.images_loading = false;
        self.confirm_remove = None;
        self.logs_for = None;
        self.logs_text.clear();
        self.logs_loading = false;
        self.generation += 1;
        let gen = self.generation;
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
                if this.generation != gen {
                    return; // a newer selection superseded this connect
                }
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
        let gen = self.generation;
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
                if this.generation != gen {
                    return; // the session changed under us; drop this result
                }
                this.apply_container_list(res);
                cx.notify();
            });
        })
        .detach();
    }

    /// Remove `id` (`docker rm -f`), then refresh the list. Triggered from the
    /// inline confirm row, so the caller already confirmed intent. The trash
    /// button that opens that row is only shown for stopped containers (see
    /// [`Self::container_row`]), matching the web panel — so the force flag
    /// only ever hard-removes an already-stopped container.
    fn remove_container(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        self.confirm_remove = None;
        cx.notify();
        let gen = self.generation;
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    remove_blocking(&session, &id, true).map_err(|e| e.to_string())?;
                    list_containers_blocking(&session, true).map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != gen {
                    return; // the session changed under us; drop this result
                }
                this.apply_container_list(res);
                cx.notify();
            });
        })
        .detach();
    }

    /// Write a refreshed container list (or error) onto the View, and drop a
    /// pending logs view if its container is no longer present.
    fn apply_container_list(&mut self, res: Result<Vec<Container>, String>) {
        match res {
            Ok(list) => {
                self.containers = list;
                self.error = None;
                // Drop a stale logs view if its container is gone now.
                let drop_logs = match &self.logs_for {
                    Some(id) => !self.containers.iter().any(|c| &c.id == id),
                    None => false,
                };
                if drop_logs {
                    self.logs_for = None;
                    self.logs_text.clear();
                    self.logs_loading = false;
                }
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Toggle the inline logs region for `id`, fetching `docker logs
    /// --tail 200 <id>` off the render path when opening.
    fn toggle_logs(&mut self, id: String, cx: &mut Context<Self>) {
        if self.logs_for.as_deref() == Some(id.as_str()) {
            self.logs_for = None;
            self.logs_text.clear();
            self.logs_loading = false;
            cx.notify();
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.logs_for = Some(id.clone());
        self.logs_text.clear();
        self.logs_loading = true;
        cx.notify();

        let fetch_id = id.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    exec_blocking(
                        &session,
                        &[
                            "logs".to_string(),
                            "--tail".to_string(),
                            "200".to_string(),
                            fetch_id,
                        ],
                    )
                    .map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Ignore a stale fetch if the user switched away meanwhile.
                if this.logs_for.as_deref() != Some(id.as_str()) {
                    return;
                }
                this.logs_loading = false;
                match res {
                    Ok((_exit, out)) => this.logs_text = out,
                    Err(e) => this.logs_text = format!("logs error: {e}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Switch resource tabs, lazily fetching images the first time the
    /// Images tab is opened for a session.
    fn select_tab(&mut self, tab: DockerTab, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        if tab == DockerTab::Images && !self.images_loaded && !self.images_loading {
            self.load_images(cx);
        }
        cx.notify();
    }

    /// Fetch `docker images` off the render path and cache the rows.
    fn load_images(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        self.images_loading = true;
        cx.notify();
        let gen = self.generation;
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { list_images_blocking(&session).map_err(|e| e.to_string()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != gen {
                    return; // a newer selection superseded this fetch
                }
                this.images_loading = false;
                match res {
                    Ok(list) => {
                        this.images = list;
                        this.images_loaded = true;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A small icon button that runs `on_click` against the View.
    fn icon_btn(
        &self,
        cx: &mut Context<Self>,
        key: String,
        glyph: &'static str,
        color: Hsla,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(SharedString::from(key))
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
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| on_click(this, cx)),
            )
            .child(ui::icon(glyph, px(14.0), color))
    }

    /// Containers / Images segmented toggle, shown once connected.
    fn tab_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .gap(t.sp1)
            .px(t.sp3)
            .pt(t.sp2)
            .child(self.tab_pill(cx, "Containers", DockerTab::Containers))
            .child(self.tab_pill(cx, "Images", DockerTab::Images))
    }

    /// One pill in the [`Self::tab_toggle`] segmented control.
    fn tab_pill(&self, cx: &mut Context<Self>, label: &'static str, tab: DockerTab) -> impl IntoElement {
        let t = &self.theme;
        let active = self.tab == tab;
        div()
            .id(SharedString::from(format!("dtab-{label}")))
            .px(t.sp3)
            .py(t.sp1)
            .rounded(t.radius_sm)
            .text_size(t.fs_ui)
            .cursor_pointer()
            .when(active, |d| d.bg(t.accent_dim).text_color(t.ink))
            .when(!active, |d| d.text_color(t.muted).hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| this.select_tab(tab, cx)),
            )
            .child(label)
    }

    /// One container: running dot + name, image/ports meta, and the action
    /// buttons (lifecycle + logs + remove, or an inline remove confirmation).
    fn container_row(&self, cx: &mut Context<Self>, c: &Container) -> impl IntoElement {
        let t = &self.theme;
        let dot = if c.is_running() { t.pos } else { t.muted };
        let running = c.is_running();
        let confirming = self.confirm_remove.as_deref() == Some(c.id.as_str());
        let logs_open = self.logs_for.as_deref() == Some(c.id.as_str());
        let logs_color = if logs_open { t.accent } else { t.muted };
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
            .when(confirming, |d| {
                d.child(
                    div()
                        .text_size(t.fs_sm)
                        .text_color(t.muted)
                        .child("Remove?"),
                )
                .child(self.icon_btn(cx, format!("dok-{}", c.id), "check", t.neg, {
                    let id = c.id.clone();
                    move |this, cx| this.remove_container(id.clone(), cx)
                }))
                .child(self.icon_btn(cx, format!("dno-{}", c.id), "close", t.muted, {
                    move |this, cx| {
                        this.confirm_remove = None;
                        cx.notify();
                    }
                }))
            })
            .when(!confirming, |d| {
                d.when(running, |d| {
                    d.child(self.icon_btn(cx, format!("dstop-{}", c.id), "pause", t.warn, {
                        let id = c.id.clone();
                        move |this, cx| this.container_op(CtrOp::Stop, id.clone(), cx)
                    }))
                    .child(self.icon_btn(cx, format!("drst-{}", c.id), "redo-2", t.info, {
                        let id = c.id.clone();
                        move |this, cx| this.container_op(CtrOp::Restart, id.clone(), cx)
                    }))
                })
                .when(!running, |d| {
                    d.child(self.icon_btn(cx, format!("dstart-{}", c.id), "play", t.pos, {
                        let id = c.id.clone();
                        move |this, cx| this.container_op(CtrOp::Start, id.clone(), cx)
                    }))
                    // Remove is offered only for stopped containers, matching the
                    // web panel; a running container must be stopped first.
                    .child(self.icon_btn(cx, format!("dtrash-{}", c.id), "delete", t.neg, {
                        let id = c.id.clone();
                        move |this, cx| {
                            this.confirm_remove = Some(id.clone());
                            cx.notify();
                        }
                    }))
                })
                .child(self.icon_btn(cx, format!("dlog-{}", c.id), "scroll-text", logs_color, {
                    let id = c.id.clone();
                    move |this, cx| this.toggle_logs(id.clone(), cx)
                }))
            })
    }

    /// Inline, scrollable `docker logs --tail 200` output for `c_id`.
    fn logs_area(&self, c_id: &str) -> impl IntoElement {
        let t = &self.theme;
        let mut body = v_flex().w_full().py(t.sp1);
        if self.logs_loading {
            body = body.child(
                div()
                    .px(t.sp3)
                    .py(px(2.0))
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child("Loading logs…"),
            );
        } else if self.logs_text.trim().is_empty() {
            body = body.child(
                div()
                    .px(t.sp3)
                    .py(px(2.0))
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child("(no output)"),
            );
        } else {
            for line in self.logs_text.lines() {
                body = body.child(
                    div()
                        .w_full()
                        .px(t.sp3)
                        .py(px(1.0))
                        .font_family(t.mono.clone())
                        .text_size(t.fs_sm)
                        .text_color(t.ink_2)
                        .child(line.to_string()),
                );
            }
        }
        div()
            .id(SharedString::from(format!("dlogsbox-{c_id}")))
            .w_full()
            .max_h(px(240.0))
            .overflow_y_scroll()
            .bg(t.surface)
            .border_t_1()
            .border_b_1()
            .border_color(t.line)
            .child(body)
    }

    /// One image: hard-drive glyph + repository, then tag / size / created.
    fn image_row(&self, img: &DockerImage) -> impl IntoElement {
        let t = &self.theme;
        let repo = if img.repository.is_empty() {
            "<none>".to_string()
        } else {
            img.repository.clone()
        };
        let tag = if img.tag.is_empty() {
            "<none>".to_string()
        } else {
            img.tag.clone()
        };
        let mut sub = h_flex()
            .gap(t.sp2)
            .font_family(t.mono.clone())
            .text_size(t.fs_sm)
            .child(div().text_color(t.accent).child(tag))
            .child(div().text_color(t.muted).child(img.size.clone()));
        if !img.created.is_empty() {
            sub = sub.child(div().overflow_hidden().text_color(t.muted).child(img.created.clone()));
        }
        h_flex()
            .id(SharedString::from(format!("dimg-{}-{}-{}", img.repository, img.tag, img.id)))
            .items_center()
            .gap(t.sp2)
            .py(px(6.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .child(ui::icon("hard-drive", px(14.0), t.muted))
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
                            .child(repo),
                    )
                    .child(sub),
            )
    }
}

impl Render for DockerPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = &self.theme;
        let count = if self.session.is_some() {
            match self.tab {
                DockerTab::Containers => self.containers.len().to_string(),
                DockerTab::Images => {
                    if self.images_loaded {
                        self.images.len().to_string()
                    } else {
                        String::new()
                    }
                }
            }
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
            col = col.child(self.tab_toggle(cx));
            match self.tab {
                DockerTab::Containers => {
                    if self.containers.is_empty() {
                        col = col.child(ui::empty_state(t, "No containers"));
                    } else {
                        col = col.child(
                            ui::section_label(t, format!("CONTAINERS · {}", self.containers.len())),
                        );
                        for c in &self.containers {
                            col = col.child(self.container_row(cx, c));
                            if self.logs_for.as_deref() == Some(c.id.as_str()) {
                                col = col.child(self.logs_area(&c.id));
                            }
                        }
                    }
                }
                DockerTab::Images => {
                    if self.images_loading {
                        col = col.child(ui::empty_state(t, "Loading images…"));
                    } else if self.images.is_empty() {
                        col = col.child(ui::empty_state(t, "No images"));
                    } else {
                        col = col
                            .child(ui::section_label(t, format!("IMAGES · {}", self.images.len())));
                        for img in &self.images {
                            col = col.child(self.image_row(img));
                        }
                    }
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
