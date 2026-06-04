// Docker panel — container + image management over an SSH session.
//
// Flow: render a connection selector from the saved SSH configs
// (data::connections_raw). On pick, connect off the render path
// (data::connect_blocking) and list containers
// (pier_core::services::docker::list_containers_blocking) on the
// background executor, then cache the session + rows on the View and
// notify. Per-container start/stop/restart/remove run over the cached
// session and refresh the list; container logs (`docker logs --tail 200`)
// and `docker inspect` expand inline. A Containers/Images/Volumes/Networks/
// Projects toggle lists images, volumes, and networks, derives Compose
// projects from container labels (per-service start/stop/restart fans out
// over member ids), and a Run form creates a container via `docker run -d`.
// Every blocking call runs on cx.background_executor().

use gpui::prelude::*;
use gpui::{
    div, px, Context, FocusHandle, FontWeight, Hsla, KeyDownEvent, MouseButton, MouseDownEvent,
    SharedString, Window,
};
use gpui_component::{h_flex, v_flex};

use pier_core::services::docker::{
    exec_blocking, inspect_container_blocking, list_containers_blocking, list_images_blocking,
    list_networks_blocking, list_volumes_blocking, remove_blocking, remove_network_blocking,
    remove_volume_blocking, restart_blocking, run_container_blocking, start_blocking, stop_blocking,
    Container, DockerImage, DockerNetwork, DockerVolume, RunContainerOptions,
};
use pier_core::ssh::{SshConfig, SshSession};

use crate::data;
use crate::i18n;
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
    Volumes,
    Networks,
    /// Compose projects, derived from container labels (no fetch of its own).
    Projects,
}

/// A resource awaiting inline remove confirmation. Tagged by kind so the
/// confirm row dispatches the right `*_blocking` call; only one is pending at
/// a time and it's cleared whenever the tab changes.
#[derive(Clone, PartialEq)]
enum PendingRemove {
    Container(String),
    Volume(String),
    Network(String),
}

/// Which text field of the inline Run form currently captures keystrokes.
/// (Restart is a cycle button, not a text field, so it isn't represented here.)
#[derive(Clone, Copy, PartialEq, Default)]
enum RunField {
    #[default]
    Image,
    Name,
    Port,
    Env,
}

/// State for the inline `docker run -d` form. Mirrors the sftp `Edit` pattern:
/// one shared input focus, the active field captures keys. Single port / env
/// by design (no multi-row repeater).
#[derive(Default)]
struct RunForm {
    /// Image reference (required), e.g. `nginx:latest`.
    image: String,
    /// Optional `--name`.
    name: String,
    /// One `host:container` mapping (bare `container` lets docker pick the host).
    port: String,
    /// One `KEY=value` environment variable.
    env: String,
    /// Restart policy: `""` | `always` | `on-failure` | `unless-stopped`.
    restart: String,
    /// Which text field is focused for keyboard input.
    field: RunField,
    /// True while the `docker run` round-trip is in flight.
    submitting: bool,
}

impl RunForm {
    /// The buffer for the currently-focused field, for key accumulation.
    fn active_buf(&mut self) -> &mut String {
        match self.field {
            RunField::Image => &mut self.image,
            RunField::Name => &mut self.name,
            RunField::Port => &mut self.port,
            RunField::Env => &mut self.env,
        }
    }
}

/// Restart-policy values the Run form cycles through (first = none).
const RESTART_CYCLE: [&str; 4] = ["", "always", "on-failure", "unless-stopped"];

/// One Compose service within a project: its containers and their run state.
struct ComposeService {
    name: String,
    /// `(container id, is_running)` for each member.
    members: Vec<(String, bool)>,
}

/// A Compose project derived from container labels.
struct ComposeProject {
    name: String,
    services: Vec<ComposeService>,
    running: usize,
    total: usize,
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
    /// Volume rows from `docker volume ls`, fetched lazily on first Volumes view.
    volumes: Vec<DockerVolume>,
    /// True once volumes have been fetched for the current session.
    volumes_loaded: bool,
    /// True while the `docker volume ls` round-trip is in flight.
    volumes_loading: bool,
    /// Network rows from `docker network ls`, fetched lazily on first view.
    networks: Vec<DockerNetwork>,
    /// True once networks have been fetched for the current session.
    networks_loaded: bool,
    /// True while the `docker network ls` round-trip is in flight.
    networks_loading: bool,
    /// Resource (container/volume/network) awaiting a remove confirmation.
    confirm_remove: Option<PendingRemove>,
    /// Container id whose logs are expanded inline, if any.
    logs_for: Option<String>,
    /// Captured `docker logs` output for [`Self::logs_for`].
    logs_text: String,
    /// True while a `docker logs` round-trip is in flight.
    logs_loading: bool,
    /// Container id whose `docker inspect` JSON is expanded inline, if any.
    inspect_for: Option<String>,
    /// Captured `docker inspect` output for [`Self::inspect_for`].
    inspect_text: String,
    /// True while a `docker inspect` round-trip is in flight.
    inspect_loading: bool,
    /// Inline `docker run` form, present while open.
    run_form: Option<RunForm>,
    /// Focus handle shared by the Run form's text fields.
    run_focus: FocusHandle,
    /// One-line failure from connect or listing, shown in `t.neg`.
    error: Option<String>,
    /// Bumped per connect so a stale background result (from a host the user
    /// has since switched away from) can't overwrite a newer selection.
    generation: u64,
}

impl DockerPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
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
            volumes: Vec::new(),
            volumes_loaded: false,
            volumes_loading: false,
            networks: Vec::new(),
            networks_loaded: false,
            networks_loading: false,
            confirm_remove: None,
            logs_for: None,
            logs_text: String::new(),
            logs_loading: false,
            inspect_for: None,
            inspect_text: String::new(),
            inspect_loading: false,
            run_form: None,
            run_focus: cx.focus_handle(),
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
        self.volumes.clear();
        self.volumes_loaded = false;
        self.volumes_loading = false;
        self.networks.clear();
        self.networks_loaded = false;
        self.networks_loading = false;
        self.confirm_remove = None;
        self.logs_for = None;
        self.logs_text.clear();
        self.logs_loading = false;
        self.inspect_for = None;
        self.inspect_text.clear();
        self.inspect_loading = false;
        self.run_form = None;
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
                // Drop a stale logs / inspect view if its container is gone now.
                let drop_logs = match &self.logs_for {
                    Some(id) => !self.containers.iter().any(|c| &c.id == id),
                    None => false,
                };
                if drop_logs {
                    self.logs_for = None;
                    self.logs_text.clear();
                    self.logs_loading = false;
                }
                let drop_inspect = match &self.inspect_for {
                    Some(id) => !self.containers.iter().any(|c| &c.id == id),
                    None => false,
                };
                if drop_inspect {
                    self.inspect_for = None;
                    self.inspect_text.clear();
                    self.inspect_loading = false;
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

    /// Switch resource tabs, lazily fetching the list the first time a
    /// fetch-backed tab (Images / Volumes / Networks) is opened for a session.
    /// Projects is derived from containers, so it needs no fetch.
    fn select_tab(&mut self, tab: DockerTab, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        // A pending confirm belongs to the tab it was opened on; drop it so it
        // can't match a row on the tab we're switching to.
        self.confirm_remove = None;
        match tab {
            DockerTab::Images if !self.images_loaded && !self.images_loading => {
                self.load_images(cx)
            }
            DockerTab::Volumes if !self.volumes_loaded && !self.volumes_loading => {
                self.load_volumes(cx)
            }
            DockerTab::Networks if !self.networks_loaded && !self.networks_loading => {
                self.load_networks(cx)
            }
            _ => {}
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

    /// Fetch `docker volume ls` off the render path and cache the rows.
    fn load_volumes(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        self.volumes_loading = true;
        cx.notify();
        let gen = self.generation;
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { list_volumes_blocking(&session).map_err(|e| e.to_string()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != gen {
                    return; // a newer selection superseded this fetch
                }
                this.volumes_loading = false;
                match res {
                    Ok(list) => {
                        this.volumes = list;
                        this.volumes_loaded = true;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Fetch `docker network ls` off the render path and cache the rows.
    fn load_networks(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        self.networks_loading = true;
        cx.notify();
        let gen = self.generation;
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { list_networks_blocking(&session).map_err(|e| e.to_string()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != gen {
                    return; // a newer selection superseded this fetch
                }
                this.networks_loading = false;
                match res {
                    Ok(list) => {
                        this.networks = list;
                        this.networks_loaded = true;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Remove the named volume (`docker volume rm`), then re-list volumes.
    /// Triggered from the inline confirm row.
    fn remove_volume(&mut self, name: String, cx: &mut Context<Self>) {
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
                    remove_volume_blocking(&session, &name).map_err(|e| e.to_string())?;
                    list_volumes_blocking(&session).map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != gen {
                    return; // the session changed under us; drop this result
                }
                match res {
                    Ok(list) => {
                        this.volumes = list;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Remove the named network (`docker network rm`), then re-list networks.
    /// Triggered from the inline confirm row.
    fn remove_network(&mut self, name: String, cx: &mut Context<Self>) {
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
                    remove_network_blocking(&session, &name).map_err(|e| e.to_string())?;
                    list_networks_blocking(&session).map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != gen {
                    return; // the session changed under us; drop this result
                }
                match res {
                    Ok(list) => {
                        this.networks = list;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Run `op` over every container id in `ids` sequentially, then refresh the
    /// container list once. Backs the Compose Projects per-service buttons; the
    /// caller pre-filters out no-ops (e.g. Start only passes stopped members),
    /// so an empty `ids` is a no-op.
    fn service_op(&mut self, op: CtrOp, ids: Vec<String>, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if ids.is_empty() {
            return;
        }
        let gen = self.generation;
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    for id in &ids {
                        match op {
                            CtrOp::Start => start_blocking(&session, id),
                            CtrOp::Stop => stop_blocking(&session, id),
                            CtrOp::Restart => restart_blocking(&session, id),
                        }
                        .map_err(|e| e.to_string())?;
                    }
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

    /// Toggle the inline inspect region for `id`, fetching `docker inspect <id>`
    /// (raw JSON) off the render path when opening. Mirrors [`Self::toggle_logs`].
    fn toggle_inspect(&mut self, id: String, cx: &mut Context<Self>) {
        if self.inspect_for.as_deref() == Some(id.as_str()) {
            self.inspect_for = None;
            self.inspect_text.clear();
            self.inspect_loading = false;
            cx.notify();
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.inspect_for = Some(id.clone());
        self.inspect_text.clear();
        self.inspect_loading = true;
        cx.notify();

        let fetch_id = id.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    inspect_container_blocking(&session, &fetch_id).map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Ignore a stale fetch if the user switched away meanwhile.
                if this.inspect_for.as_deref() != Some(id.as_str()) {
                    return;
                }
                this.inspect_loading = false;
                match res {
                    Ok(out) => this.inspect_text = out,
                    Err(e) => this.inspect_text = format!("inspect error: {e}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Open / close the inline Run form. Opening focuses the image field.
    fn toggle_run_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.run_form.is_some() {
            self.run_form = None;
        } else {
            self.run_form = Some(RunForm::default());
            window.focus(&self.run_focus, cx);
        }
        cx.notify();
    }

    /// Advance the Run form's restart policy to the next value in the cycle.
    fn cycle_restart(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.run_form {
            let next = RESTART_CYCLE
                .iter()
                .position(|r| *r == form.restart)
                .map(|i| (i + 1) % RESTART_CYCLE.len())
                .unwrap_or(0);
            form.restart = RESTART_CYCLE[next].to_string();
            cx.notify();
        }
    }

    /// Feed a keystroke into the focused Run-form field. Enter submits, Escape
    /// closes, Backspace pops, printable characters append. Mirrors sftp's
    /// `on_input_key`.
    fn on_run_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "enter" => {
                self.submit_run(cx);
                return;
            }
            "escape" => {
                self.run_form = None;
                cx.notify();
                return;
            }
            "backspace" => {
                if let Some(form) = &mut self.run_form {
                    if form.active_buf().pop().is_some() {
                        cx.notify();
                    }
                }
                return;
            }
            _ => {}
        }
        let m = &ks.modifiers;
        if m.control || m.alt || m.platform {
            return;
        }
        if let Some(kc) = &ks.key_char {
            if kc.is_empty() || kc.chars().any(|c| c.is_control()) {
                return;
            }
            if let Some(form) = &mut self.run_form {
                form.active_buf().push_str(kc);
                cx.notify();
            }
        }
    }

    /// Build `RunContainerOptions` from the form and run `docker run -d` off the
    /// render path. On success, close the form, switch to Containers, and show
    /// the refreshed list.
    fn submit_run(&mut self, cx: &mut Context<Self>) {
        // Copy every field out before any `self` mutation so the shared borrow
        // of `run_form` ends here.
        let (image, name, ports, env, restart) = {
            let Some(form) = &self.run_form else {
                return;
            };
            if form.submitting {
                return;
            }
            (
                form.image.trim().to_string(),
                form.name.trim().to_string(),
                parse_port(&form.port).into_iter().collect::<Vec<_>>(),
                parse_env(&form.env).into_iter().collect::<Vec<_>>(),
                form.restart.clone(),
            )
        };
        if image.is_empty() {
            self.error = Some(i18n::t("docker.image_required").to_string());
            cx.notify();
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        let opts = RunContainerOptions {
            image,
            name,
            ports,
            env,
            volumes: Vec::new(),
            restart,
            command: String::new(),
        };
        if let Some(form) = &mut self.run_form {
            form.submitting = true;
        }
        self.error = None;
        cx.notify();
        let gen = self.generation;
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    run_container_blocking(&session, &opts).map_err(|e| e.to_string())?;
                    list_containers_blocking(&session, true).map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != gen {
                    return; // the session changed under us; drop this result
                }
                match res {
                    Ok(list) => {
                        this.containers = list;
                        this.error = None;
                        this.run_form = None;
                        this.tab = DockerTab::Containers;
                    }
                    Err(e) => {
                        this.error = Some(e);
                        if let Some(form) = &mut this.run_form {
                            form.submitting = false;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Group the cached containers into Compose projects by their
    /// `com.docker.compose.project` / `.service` labels. Pure derivation from
    /// `self.containers` (no IO); containers without a project label are
    /// omitted. Projects are sorted by name.
    fn compose_projects(&self) -> Vec<ComposeProject> {
        let mut projects: Vec<ComposeProject> = Vec::new();
        for c in &self.containers {
            let Some(project) = label_value(&c.labels, "com.docker.compose.project") else {
                continue;
            };
            if project.is_empty() {
                continue;
            }
            let service = match label_value(&c.labels, "com.docker.compose.service") {
                Some(s) if !s.is_empty() => s,
                _ => "(no service)",
            };
            let running = c.is_running();
            // Index-based find-or-insert (avoids the iter_mut().find() borrow).
            let pi = match projects.iter().position(|p| p.name == project) {
                Some(i) => i,
                None => {
                    projects.push(ComposeProject {
                        name: project.to_string(),
                        services: Vec::new(),
                        running: 0,
                        total: 0,
                    });
                    projects.len() - 1
                }
            };
            let p = &mut projects[pi];
            p.total += 1;
            if running {
                p.running += 1;
            }
            let si = match p.services.iter().position(|s| s.name == service) {
                Some(i) => i,
                None => {
                    p.services.push(ComposeService {
                        name: service.to_string(),
                        members: Vec::new(),
                    });
                    p.services.len() - 1
                }
            };
            p.services[si].members.push((c.id.clone(), running));
        }
        projects.sort_by(|a, b| a.name.cmp(&b.name));
        projects
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

    /// Resource tabs (Containers / Images / Volumes / Networks / Projects) plus
    /// the Run button, shown once connected. Wraps to a second line on narrow
    /// panels so every tab stays reachable.
    fn tab_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .flex_wrap()
            .items_center()
            .gap(t.sp1)
            .px(t.sp3)
            .pt(t.sp2)
            .child(self.tab_pill(cx, "containers", i18n::t("docker.tab_containers"), DockerTab::Containers))
            .child(self.tab_pill(cx, "images", i18n::t("docker.tab_images"), DockerTab::Images))
            .child(self.tab_pill(cx, "volumes", i18n::t("docker.tab_volumes"), DockerTab::Volumes))
            .child(self.tab_pill(cx, "networks", i18n::t("docker.tab_networks"), DockerTab::Networks))
            .child(self.tab_pill(cx, "projects", i18n::t("docker.tab_projects"), DockerTab::Projects))
            .child(self.run_btn(cx))
    }

    /// One pill in the [`Self::tab_toggle`] segmented control. `key` is the
    /// stable element id; `label` is the localized display text.
    fn tab_pill(&self, cx: &mut Context<Self>, key: &'static str, label: SharedString, tab: DockerTab) -> impl IntoElement {
        let t = &self.theme;
        let active = self.tab == tab;
        div()
            .id(SharedString::from(format!("dtab-{key}")))
            .px(t.sp2)
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

    /// The "+ Run" button that toggles the inline run form, highlighted while
    /// the form is open.
    fn run_btn(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let open = self.run_form.is_some();
        h_flex()
            .id("docker-run-btn")
            .items_center()
            .gap(px(3.0))
            .px(t.sp2)
            .py(t.sp1)
            .rounded(t.radius_sm)
            .text_size(t.fs_ui)
            .cursor_pointer()
            .when(open, |d| d.bg(t.accent_dim).text_color(t.ink))
            .when(!open, |d| d.text_color(t.muted).hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| this.toggle_run_form(window, cx)),
            )
            .child(ui::icon("plus", px(12.0), if open { t.accent } else { t.muted }))
            .child(i18n::t("docker.run"))
    }

    /// One container: running dot + name, image/ports meta, and the action
    /// buttons (lifecycle + logs + remove, or an inline remove confirmation).
    fn container_row(&self, cx: &mut Context<Self>, c: &Container) -> impl IntoElement {
        let t = &self.theme;
        let dot = if c.is_running() { t.pos } else { t.muted };
        let running = c.is_running();
        let confirming = self.confirm_remove == Some(PendingRemove::Container(c.id.clone()));
        let logs_open = self.logs_for.as_deref() == Some(c.id.as_str());
        let logs_color = if logs_open { t.accent } else { t.muted };
        let inspect_open = self.inspect_for.as_deref() == Some(c.id.as_str());
        let inspect_color = if inspect_open { t.accent } else { t.muted };
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
                        .child(i18n::t("docker.confirm_remove")),
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
                            this.confirm_remove = Some(PendingRemove::Container(id.clone()));
                            cx.notify();
                        }
                    }))
                })
                .child(self.icon_btn(cx, format!("dlog-{}", c.id), "scroll-text", logs_color, {
                    let id = c.id.clone();
                    move |this, cx| this.toggle_logs(id.clone(), cx)
                }))
                .child(self.icon_btn(cx, format!("dinsp-{}", c.id), "inspector", inspect_color, {
                    let id = c.id.clone();
                    move |this, cx| this.toggle_inspect(id.clone(), cx)
                }))
            })
    }

    /// Inline, scrollable monospace text region (shared by logs + inspect).
    /// `key` must be unique per row so the scroll state tracks correctly.
    fn mono_area(
        &self,
        key: String,
        loading: bool,
        text: &str,
        loading_msg: SharedString,
    ) -> impl IntoElement {
        let t = &self.theme;
        let mut body = v_flex().w_full().py(t.sp1);
        if loading {
            body = body.child(
                div()
                    .px(t.sp3)
                    .py(px(2.0))
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child(loading_msg),
            );
        } else if text.trim().is_empty() {
            body = body.child(
                div()
                    .px(t.sp3)
                    .py(px(2.0))
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child(i18n::t("docker.no_output")),
            );
        } else {
            for line in text.lines() {
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
            .id(SharedString::from(key))
            .w_full()
            .max_h(px(240.0))
            .overflow_y_scroll()
            .bg(t.surface)
            .border_t_1()
            .border_b_1()
            .border_color(t.line)
            .child(body)
    }

    /// Inline, scrollable `docker logs --tail 200` output for `c_id`.
    fn logs_area(&self, c_id: &str) -> impl IntoElement {
        self.mono_area(
            format!("dlogsbox-{c_id}"),
            self.logs_loading,
            &self.logs_text,
            i18n::t("docker.loading_logs"),
        )
    }

    /// Inline, scrollable `docker inspect` JSON for `c_id`.
    fn inspect_area(&self, c_id: &str) -> impl IntoElement {
        self.mono_area(
            format!("dinspbox-{c_id}"),
            self.inspect_loading,
            &self.inspect_text,
            i18n::t("docker.loading_inspect"),
        )
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

    /// One volume: database glyph + name, driver / mountpoint meta, and a
    /// delete button (or an inline remove confirmation).
    fn volume_row(&self, cx: &mut Context<Self>, v: &DockerVolume) -> impl IntoElement {
        let t = &self.theme;
        let confirming = self.confirm_remove == Some(PendingRemove::Volume(v.name.clone()));
        let mut meta = h_flex()
            .gap(t.sp2)
            .font_family(t.mono.clone())
            .text_size(t.fs_sm)
            .text_color(t.muted)
            .child(div().child(v.driver.clone()));
        if !v.mountpoint.is_empty() {
            meta = meta.child(div().overflow_hidden().child(v.mountpoint.clone()));
        }
        h_flex()
            .id(SharedString::from(format!("dvol-{}", v.name)))
            .items_center()
            .gap(t.sp2)
            .py(px(6.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .child(ui::icon("database", px(14.0), t.muted))
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
                            .child(v.name.clone()),
                    )
                    .child(meta),
            )
            .when(confirming, |d| {
                d.child(div().text_size(t.fs_sm).text_color(t.muted).child(i18n::t("docker.confirm_remove")))
                    .child(self.icon_btn(cx, format!("dvol-ok-{}", v.name), "check", t.neg, {
                        let name = v.name.clone();
                        move |this, cx| this.remove_volume(name.clone(), cx)
                    }))
                    .child(self.icon_btn(cx, format!("dvol-no-{}", v.name), "close", t.muted, {
                        move |this, cx| {
                            this.confirm_remove = None;
                            cx.notify();
                        }
                    }))
            })
            .when(!confirming, |d| {
                d.child(self.icon_btn(cx, format!("dvol-rm-{}", v.name), "delete", t.neg, {
                    let name = v.name.clone();
                    move |this, cx| {
                        this.confirm_remove = Some(PendingRemove::Volume(name.clone()));
                        cx.notify();
                    }
                }))
            })
    }

    /// One network: network glyph + name, driver / scope meta, and a delete
    /// button (or an inline remove confirmation). The predefined `bridge` /
    /// `host` / `none` networks can't be removed, so they show no delete.
    fn network_row(&self, cx: &mut Context<Self>, n: &DockerNetwork) -> impl IntoElement {
        let t = &self.theme;
        let confirming = self.confirm_remove == Some(PendingRemove::Network(n.name.clone()));
        let removable = !matches!(n.name.as_str(), "bridge" | "host" | "none");
        let mut meta = h_flex()
            .gap(t.sp2)
            .font_family(t.mono.clone())
            .text_size(t.fs_sm)
            .text_color(t.muted)
            .child(div().child(n.driver.clone()));
        if !n.scope.is_empty() {
            meta = meta.child(div().child(n.scope.clone()));
        }
        h_flex()
            .id(SharedString::from(format!("dnet-{}", n.id)))
            .items_center()
            .gap(t.sp2)
            .py(px(6.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .child(ui::icon("network", px(14.0), t.muted))
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
                            .child(n.name.clone()),
                    )
                    .child(meta),
            )
            .when(confirming, |d| {
                d.child(div().text_size(t.fs_sm).text_color(t.muted).child(i18n::t("docker.confirm_remove")))
                    .child(self.icon_btn(cx, format!("dnet-ok-{}", n.id), "check", t.neg, {
                        let name = n.name.clone();
                        move |this, cx| this.remove_network(name.clone(), cx)
                    }))
                    .child(self.icon_btn(cx, format!("dnet-no-{}", n.id), "close", t.muted, {
                        move |this, cx| {
                            this.confirm_remove = None;
                            cx.notify();
                        }
                    }))
            })
            .when(!confirming && removable, |d| {
                d.child(self.icon_btn(cx, format!("dnet-rm-{}", n.id), "delete", t.neg, {
                    let name = n.name.clone();
                    move |this, cx| {
                        this.confirm_remove = Some(PendingRemove::Network(name.clone()));
                        cx.notify();
                    }
                }))
            })
    }

    /// One Compose service row: status dot + service name, running/total meta,
    /// and Start / Stop / Restart that fan out over the service's containers.
    /// Each button pre-filters no-ops (Start skips already-running members,
    /// Stop skips stopped ones).
    fn service_row(&self, cx: &mut Context<Self>, project: &str, svc: &ComposeService) -> impl IntoElement {
        let t = &self.theme;
        let total = svc.members.len();
        let running = svc.members.iter().filter(|(_, r)| *r).count();
        let dot = if running > 0 { t.pos } else { t.muted };
        let start_ids: Vec<String> = svc
            .members
            .iter()
            .filter(|(_, r)| !*r)
            .map(|(id, _)| id.clone())
            .collect();
        let stop_ids: Vec<String> = svc
            .members
            .iter()
            .filter(|(_, r)| *r)
            .map(|(id, _)| id.clone())
            .collect();
        let all_ids: Vec<String> = svc.members.iter().map(|(id, _)| id.clone()).collect();
        let key = format!("{project}-{}", svc.name);
        h_flex()
            .id(SharedString::from(format!("dsvc-{key}")))
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
                            .child(svc.name.clone()),
                    )
                    .child(
                        div()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(i18n::tf("docker.running_count", &[&running.to_string(), &total.to_string()])),
                    ),
            )
            .child(self.icon_btn(cx, format!("dsvc-start-{key}"), "play", t.pos, {
                let ids = start_ids;
                move |this, cx| this.service_op(CtrOp::Start, ids.clone(), cx)
            }))
            .child(self.icon_btn(cx, format!("dsvc-stop-{key}"), "pause", t.warn, {
                let ids = stop_ids;
                move |this, cx| this.service_op(CtrOp::Stop, ids.clone(), cx)
            }))
            .child(self.icon_btn(cx, format!("dsvc-rst-{key}"), "redo-2", t.info, {
                let ids = all_ids;
                move |this, cx| this.service_op(CtrOp::Restart, ids.clone(), cx)
            }))
    }

    /// The inline `docker run -d` form (rendered below the tabs while open).
    fn run_form_view(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let t = &self.theme;
        let form = self.run_form.as_ref()?;
        let run_label: SharedString = if form.submitting {
            i18n::t("docker.running")
        } else {
            i18n::t("docker.run")
        };
        let restart_label: SharedString = if form.restart.is_empty() {
            i18n::t("docker.restart_none")
        } else {
            form.restart.clone().into()
        };
        Some(
            v_flex()
                .mx(t.sp3)
                .my(t.sp2)
                .py(t.sp2)
                .gap(px(2.0))
                .rounded(t.radius_md)
                .bg(t.surface)
                .border_1()
                .border_color(t.line_2)
                .child(
                    h_flex()
                        .items_center()
                        .px(t.sp3)
                        .pb(t.sp1)
                        .child(
                            div()
                                .flex_1()
                                .text_size(t.fs_ui)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.ink)
                                .child(i18n::t("docker.run_container")),
                        )
                        .child(self.icon_btn(cx, "drun-close".to_string(), "close", t.muted, {
                            move |this, cx| {
                                this.run_form = None;
                                cx.notify();
                            }
                        })),
                )
                .child(self.run_field(cx, RunField::Image, "image", i18n::t("docker.run_image"), form.image.clone(), SharedString::from("nginx:latest")))
                .child(self.run_field(cx, RunField::Name, "name", i18n::t("docker.run_name"), form.name.clone(), i18n::t("docker.run_optional")))
                .child(self.run_field(cx, RunField::Port, "port", i18n::t("docker.run_port"), form.port.clone(), SharedString::from("8080:80")))
                .child(self.run_field(cx, RunField::Env, "env", i18n::t("docker.run_env"), form.env.clone(), SharedString::from("KEY=value")))
                .child(
                    self.run_field_row(i18n::t("docker.run_restart")).child(
                        div()
                            .id("drun-restart")
                            .h(px(22.0))
                            .px(t.sp2)
                            .flex()
                            .items_center()
                            .rounded(t.radius_sm)
                            .bg(t.panel_2)
                            .border_1()
                            .border_color(t.line)
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.ink_2)
                            .cursor_pointer()
                            .hover(|s| s.border_color(t.line_3))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _w, cx| this.cycle_restart(cx)),
                            )
                            .child(restart_label),
                    ),
                )
                .child(
                    h_flex().px(t.sp3).pt(t.sp1).child(
                        div()
                            .id("drun-submit")
                            .px(t.sp3)
                            .py(px(4.0))
                            .rounded(t.radius_sm)
                            .bg(t.accent)
                            .text_color(t.accent_ink)
                            .text_size(t.fs_ui)
                            .cursor_pointer()
                            .hover(|s| s.bg(t.accent_hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _w, cx| this.submit_run(cx)),
                            )
                            .child(run_label),
                    ),
                ),
        )
    }

    /// A label + control row in the Run form (the control is supplied by the
    /// caller). `label` is the localized display text.
    fn run_field_row(&self, label: SharedString) -> gpui::Div {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .py(px(2.0))
            .child(
                div()
                    .w(px(52.0))
                    .flex_none()
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(label),
            )
    }

    /// One text field in the Run form: the focused field is an editable input;
    /// the others are clickable cells that activate (focus) themselves. `key` is
    /// the stable element id; `label` and `placeholder` are localized display
    /// text.
    fn run_field(
        &self,
        cx: &mut Context<Self>,
        field: RunField,
        key: &'static str,
        label: SharedString,
        value: String,
        placeholder: SharedString,
    ) -> impl IntoElement {
        let t = &self.theme;
        let active = self.run_form.as_ref().map(|f| f.field) == Some(field);
        let empty = value.is_empty();
        let cell = if active {
            div()
                .track_focus(&self.run_focus)
                .key_context("DockerRunInput")
                .on_key_down(cx.listener(Self::on_run_key))
                .w_full()
                .h(px(22.0))
                .px(t.sp2)
                .flex()
                .items_center()
                .rounded(t.radius_sm)
                .bg(t.panel_2)
                .border_1()
                .border_color(t.accent)
                .font_family(t.mono.clone())
                .text_size(t.fs_sm)
                .when(empty, |d| d.text_color(t.dim).child(placeholder))
                .when(!empty, |d| d.text_color(t.ink).child(value))
                .into_any_element()
        } else {
            div()
                .id(SharedString::from(format!("drun-{key}")))
                .w_full()
                .h(px(22.0))
                .px(t.sp2)
                .flex()
                .items_center()
                .rounded(t.radius_sm)
                .bg(t.panel_2)
                .border_1()
                .border_color(t.line)
                .font_family(t.mono.clone())
                .text_size(t.fs_sm)
                .cursor_pointer()
                .hover(|s| s.border_color(t.line_3))
                .when(empty, |d| d.text_color(t.dim).child(placeholder))
                .when(!empty, |d| d.text_color(t.ink).child(value))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        if let Some(form) = &mut this.run_form {
                            form.field = field;
                        }
                        window.focus(&this.run_focus, cx);
                        cx.notify();
                    }),
                )
                .into_any_element()
        };
        self.run_field_row(label)
            .child(div().flex_1().min_w(px(0.0)).child(cell))
    }
}

impl Render for DockerPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = &self.theme;
        // Compose projects are derived from the loaded containers; compute once
        // for both the header count and the Projects body.
        let projects = if self.session.is_some() && self.tab == DockerTab::Projects {
            self.compose_projects()
        } else {
            Vec::new()
        };
        let count = if self.session.is_some() {
            match self.tab {
                DockerTab::Containers => self.containers.len().to_string(),
                DockerTab::Images => maybe_count(self.images_loaded, self.images.len()),
                DockerTab::Volumes => maybe_count(self.volumes_loaded, self.volumes.len()),
                DockerTab::Networks => maybe_count(self.networks_loaded, self.networks.len()),
                DockerTab::Projects => projects.len().to_string(),
            }
        } else {
            String::new()
        };

        let mut col = v_flex()
            .flex_1()
            .min_h(px(0.0))
            .child(ui::panel_header(t, "container", i18n::t("tool.docker"), count));

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
            col = col.child(ui::empty_state(t, i18n::t("panel.connecting")));
        } else if self.session.is_some() {
            col = col.child(self.tab_toggle(cx));
            if let Some(form) = self.run_form_view(cx) {
                col = col.child(form);
            }
            match self.tab {
                DockerTab::Containers => {
                    if self.containers.is_empty() {
                        col = col.child(ui::empty_state(t, i18n::t("panel.no_containers")));
                    } else {
                        col = col.child(
                            ui::section_label(t, i18n::tf("docker.containers_count", &[&self.containers.len().to_string()])),
                        );
                        for c in &self.containers {
                            col = col.child(self.container_row(cx, c));
                            if self.logs_for.as_deref() == Some(c.id.as_str()) {
                                col = col.child(self.logs_area(&c.id));
                            }
                            if self.inspect_for.as_deref() == Some(c.id.as_str()) {
                                col = col.child(self.inspect_area(&c.id));
                            }
                        }
                    }
                }
                DockerTab::Images => {
                    if self.images_loading {
                        col = col.child(ui::empty_state(t, i18n::t("panel.loading_images")));
                    } else if self.images.is_empty() {
                        col = col.child(ui::empty_state(t, i18n::t("panel.no_images")));
                    } else {
                        col = col
                            .child(ui::section_label(t, i18n::tf("docker.images_count", &[&self.images.len().to_string()])));
                        for img in &self.images {
                            col = col.child(self.image_row(img));
                        }
                    }
                }
                DockerTab::Volumes => {
                    if self.volumes_loading {
                        col = col.child(ui::empty_state(t, i18n::t("panel.loading_volumes")));
                    } else if self.volumes.is_empty() {
                        col = col.child(ui::empty_state(t, i18n::t("panel.no_volumes")));
                    } else {
                        col = col.child(
                            ui::section_label(t, i18n::tf("docker.volumes_count", &[&self.volumes.len().to_string()])),
                        );
                        for v in &self.volumes {
                            col = col.child(self.volume_row(cx, v));
                        }
                    }
                }
                DockerTab::Networks => {
                    if self.networks_loading {
                        col = col.child(ui::empty_state(t, i18n::t("panel.loading_networks")));
                    } else if self.networks.is_empty() {
                        col = col.child(ui::empty_state(t, i18n::t("panel.no_networks")));
                    } else {
                        col = col.child(
                            ui::section_label(t, i18n::tf("docker.networks_count", &[&self.networks.len().to_string()])),
                        );
                        for n in &self.networks {
                            col = col.child(self.network_row(cx, n));
                        }
                    }
                }
                DockerTab::Projects => {
                    if projects.is_empty() {
                        col = col.child(ui::empty_state(t, i18n::t("panel.no_compose")));
                    } else {
                        for p in &projects {
                            col = col.child(ui::section_label(
                                t,
                                i18n::tf("docker.project_up", &[&p.name, &p.running.to_string(), &p.total.to_string()]),
                            ));
                            for svc in &p.services {
                                col = col.child(self.service_row(cx, &p.name, svc));
                            }
                        }
                    }
                }
            }
        } else if self.conns.is_empty() {
            col = col.child(ui::empty_state(t, i18n::t("side.no_saved_connections")));
        } else {
            col = col.child(ui::section_label(t, i18n::tf("side.servers_count", &[&self.conns.len().to_string()])));
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

/// Count string for a lazily-loaded tab: blank until the first fetch lands so
/// the header doesn't show a misleading `0` before any data is in.
fn maybe_count(loaded: bool, n: usize) -> String {
    if loaded {
        n.to_string()
    } else {
        String::new()
    }
}

/// Look up `key` in a comma-separated `k=v,k2=v2` docker label string (the
/// shape `docker ps --format '{{json .}}'` emits in `Labels`). Returns the
/// first match, trimmed. Naive split on `,` / `=` — Compose project/service
/// names contain neither, which is all this is used for.
fn label_value<'a>(labels: &'a str, key: &str) -> Option<&'a str> {
    labels.split(',').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        if k.trim() == key {
            Some(v.trim())
        } else {
            None
        }
    })
}

/// Split a `host:container` (or bare `container`) port string into the
/// `(host, container)` pair `RunContainerOptions` wants. Empty input → `None`;
/// a bare port leaves the host empty so docker assigns one.
fn parse_port(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    match s.split_once(':') {
        Some((h, g)) => Some((h.trim().to_string(), g.trim().to_string())),
        None => Some((String::new(), s.to_string())),
    }
}

/// Split a `KEY=value` env string into a pair. Empty input or a missing `=`
/// (or empty key) → `None`.
fn parse_env(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (k, v) = s.split_once('=')?;
    let k = k.trim();
    if k.is_empty() {
        return None;
    }
    Some((k.to_string(), v.trim().to_string()))
}
