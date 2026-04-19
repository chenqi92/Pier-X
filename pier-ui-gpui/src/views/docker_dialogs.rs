//! Docker-panel modal dialogs:
//!
//! - [`open_docker_inspect_dialog`] — full-window reactive modal that
//!   renders the current container's `docker inspect` JSON. The bottom-
//!   of-panel inspect strip is retired for containers; only volumes
//!   keep their inline expansion (see `docker_volume_expanded_block`
//!   in `right_panel.rs`).
//!
//! - [`open_docker_run_dialog`] — form modal triggered by the Play
//!   button on an image row. Walks the user through the common
//!   `docker run` flags and, on OK, builds a [`DockerRunSpec`] and
//!   dispatches it via `PierApp::schedule_docker_run`.
//!
//! Both dialogs follow the shape used elsewhere in the app
//! (sftp_dialogs / database_form): pre-create `InputState` entities
//! outside the `open_dialog` closure so the bindings survive the
//! re-renders gpui_component's Dialog triggers internally.

use gpui::{
    div, prelude::*, px, App, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    WeakEntity, Window,
};
use gpui_component::{
    input::{Input, InputState},
    scroll::ScrollableElement,
    switch::Switch,
    WindowExt as _,
};
use pier_core::services::docker::DockerRunSpec;
use rust_i18n::t;

use crate::app::ssh_session::SshSessionState;
use crate::app::PierApp;
use crate::components::{
    text, FormField, FormSection, Separator, SettingRow, StatusKind, StatusPill,
};
use crate::theme::{
    spacing::{SP_1_5, SP_2, SP_3},
    theme,
    typography::SIZE_MONO_SMALL,
};

// ═══════════════════════════════════════════════════════════════════
// Inspect dialog
// ═══════════════════════════════════════════════════════════════════

const INSPECT_DIALOG_W: f32 = 880.0;
const INSPECT_DIALOG_H: f32 = 560.0;

/// Reactive body for the container-inspect dialog. Reads the active
/// session's `docker_inspect` on each render so the dialog updates
/// in place when the backend's `docker inspect` returns.
pub struct DockerInspectDialog {
    app: WeakEntity<PierApp>,
    target_id: String,
    target_label: SharedString,
}

impl DockerInspectDialog {
    pub fn new(app: WeakEntity<PierApp>, target_id: String, target_label: SharedString) -> Self {
        Self {
            app,
            target_id,
            target_label,
        }
    }

    fn current(&self, cx: &App) -> Option<(String, bool)> {
        let app_entity = self.app.upgrade()?;
        let pier = app_entity.read(cx);
        let session: &Entity<SshSessionState> = pier.active_session_ref()?;
        let session = session.read(cx);
        let output = session
            .docker_inspect
            .as_ref()
            .filter(|s| s.target_id == self.target_id)
            .map(|s| s.output.clone());
        let loading = session
            .docker_pending_action
            .as_ref()
            .map(|a| a.target_id == self.target_id)
            .unwrap_or(false);
        output
            .map(|o| (o, loading))
            .or_else(|| Some((String::new(), loading)))
    }
}

impl Render for DockerInspectDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let (output, loading) = self.current(cx).unwrap_or_default();

        let body: gpui::AnyElement = if loading && output.is_empty() {
            div()
                .p(SP_3)
                .child(text::caption(t!("App.Common.Status.loading")).secondary())
                .into_any_element()
        } else if output.is_empty() {
            div()
                .p(SP_3)
                .child(text::caption(t!("App.RightPanel.Docker.idle_hint")).secondary())
                .into_any_element()
        } else {
            div()
                .px(SP_3)
                .py(SP_2)
                .overflow_hidden()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_secondary)
                .child(output)
                .into_any_element()
        };

        div()
            .w(px(INSPECT_DIALOG_W))
            .h(px(INSPECT_DIALOG_H))
            .flex()
            .flex_col()
            .bg(t.color.bg_surface)
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(SP_2)
                    .px(SP_3)
                    .py(SP_2)
                    .child(StatusPill::new(self.target_label.clone(), StatusKind::Info))
                    .child(
                        div().flex_1().min_w(px(0.0)).overflow_hidden().child(
                            text::caption(SharedString::from(self.target_id.clone()))
                                .secondary()
                                .truncate(),
                        ),
                    ),
            )
            .child(Separator::horizontal())
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .overflow_x_hidden()
                    .child(body),
            )
    }
}

pub fn open_docker_inspect_dialog(
    window: &mut Window,
    cx: &mut App,
    app: WeakEntity<PierApp>,
    target_id: String,
    target_label: impl Into<SharedString>,
) {
    let target_label: SharedString = target_label.into();
    let entity =
        cx.new(|_| DockerInspectDialog::new(app.clone(), target_id.clone(), target_label.clone()));
    let title: SharedString =
        format!("{} · {}", t!("App.RightPanel.Docker.inspect"), target_label).into();
    window.open_dialog(cx, move |dialog, _w, _app| {
        dialog
            .title(title.clone())
            .w(px(INSPECT_DIALOG_W))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(entity.clone())
    });
}

// ═══════════════════════════════════════════════════════════════════
// Run dialog
// ═══════════════════════════════════════════════════════════════════

const RUN_DIALOG_W: f32 = 620.0;
const RUN_DIALOG_H: f32 = 680.0;

/// Per-dialog state bag. Holds one `InputState` entity per form
/// field + booleans for the toggle rows, all persistent across the
/// dialog's internal re-renders.
pub struct DockerRunDialog {
    image: SharedString,
    name: Entity<InputState>,
    command: Entity<InputState>,
    ports: Entity<InputState>,
    volumes: Entity<InputState>,
    env: Entity<InputState>,
    network: Entity<InputState>,
    restart: Entity<InputState>,
    workdir: Entity<InputState>,
    user: Entity<InputState>,
    hostname: Entity<InputState>,
    entrypoint: Entity<InputState>,
    detached: bool,
    auto_remove: bool,
    privileged: bool,
    read_only: bool,
    interactive_tty: bool,
}

impl DockerRunDialog {
    fn read_spec(&self, cx: &App) -> DockerRunSpec {
        DockerRunSpec {
            image: self.image.to_string(),
            name: read_trim(&self.name, cx),
            ports: read_lines(&self.ports, cx),
            volumes: read_lines(&self.volumes, cx),
            env: read_lines(&self.env, cx),
            network: read_trim(&self.network, cx),
            restart: read_trim(&self.restart, cx),
            workdir: read_trim(&self.workdir, cx),
            user: read_trim(&self.user, cx),
            hostname: read_trim(&self.hostname, cx),
            entrypoint: read_trim(&self.entrypoint, cx),
            command: read_trim(&self.command, cx),
            detached: self.detached,
            auto_remove: self.auto_remove,
            privileged: self.privileged,
            read_only: self.read_only,
            interactive_tty: self.interactive_tty,
        }
    }
}

impl Render for DockerRunDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let preview = self.read_spec(cx).to_command();

        div()
            .w(px(RUN_DIALOG_W))
            .h(px(RUN_DIALOG_H))
            .flex()
            .flex_col()
            .bg(t.color.bg_surface)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(SP_2)
                    .child(text::caption(t!("App.RightPanel.Docker.run_image")).secondary())
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(SIZE_MONO_SMALL)
                            .font_family(t.font_mono.clone())
                            .text_color(t.color.text_primary)
                            .truncate()
                            .child(self.image.clone()),
                    ),
            )
            .child(Separator::horizontal())
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .px(SP_3)
                    .py(SP_3)
                    .flex()
                    .flex_col()
                    .gap(SP_3)
                    .child(
                        FormSection::new(t!("App.RightPanel.Docker.RunDialog.launch"))
                            .description(t!("App.RightPanel.Docker.RunDialog.launch_body"))
                            .child(field(
                                t!("App.RightPanel.Docker.RunDialog.name"),
                                &self.name,
                                None,
                            ))
                            .child(field(
                                t!("App.RightPanel.Docker.RunDialog.entrypoint"),
                                &self.entrypoint,
                                None,
                            ))
                            .child(field(
                                t!("App.RightPanel.Docker.RunDialog.command"),
                                &self.command,
                                Some(t!("App.RightPanel.Docker.RunDialog.command_hint").into()),
                            )),
                    )
                    .child(
                        FormSection::new(t!("App.RightPanel.Docker.RunDialog.network_mounts"))
                            .child(field(
                                t!("App.RightPanel.Docker.RunDialog.ports"),
                                &self.ports,
                                Some(t!("App.RightPanel.Docker.RunDialog.ports_hint").into()),
                            ))
                            .child(field(
                                t!("App.RightPanel.Docker.RunDialog.volumes"),
                                &self.volumes,
                                Some(t!("App.RightPanel.Docker.RunDialog.volumes_hint").into()),
                            ))
                            .child(field(
                                t!("App.RightPanel.Docker.RunDialog.environment"),
                                &self.env,
                                Some(t!("App.RightPanel.Docker.RunDialog.environment_hint").into()),
                            ))
                            .child(field(
                                t!("App.RightPanel.Docker.RunDialog.network"),
                                &self.network,
                                None,
                            ))
                            .child(field(
                                t!("App.RightPanel.Docker.RunDialog.restart"),
                                &self.restart,
                                Some(t!("App.RightPanel.Docker.RunDialog.restart_hint").into()),
                            ))
                            .child(field(
                                t!("App.RightPanel.Docker.RunDialog.workdir"),
                                &self.workdir,
                                None,
                            ))
                            .child(field(
                                t!("App.RightPanel.Docker.RunDialog.user"),
                                &self.user,
                                None,
                            ))
                            .child(field(
                                t!("App.RightPanel.Docker.RunDialog.hostname"),
                                &self.hostname,
                                None,
                            )),
                    )
                    .child(
                        FormSection::new(t!("App.RightPanel.Docker.RunDialog.behavior"))
                            .description(t!("App.RightPanel.Docker.RunDialog.behavior_body"))
                            .child(toggle_row(
                                cx,
                                t!("App.RightPanel.Docker.RunDialog.detached"),
                                t!("App.RightPanel.Docker.RunDialog.detached_body"),
                                self.detached,
                                "docker-run-detached",
                                |this| this.detached = !this.detached,
                            ))
                            .child(toggle_row(
                                cx,
                                t!("App.RightPanel.Docker.RunDialog.auto_remove"),
                                t!("App.RightPanel.Docker.RunDialog.auto_remove_body"),
                                self.auto_remove,
                                "docker-run-rm",
                                |this| this.auto_remove = !this.auto_remove,
                            ))
                            .child(toggle_row(
                                cx,
                                t!("App.RightPanel.Docker.RunDialog.interactive_tty"),
                                t!("App.RightPanel.Docker.RunDialog.interactive_tty_body"),
                                self.interactive_tty,
                                "docker-run-it",
                                |this| this.interactive_tty = !this.interactive_tty,
                            ))
                            .child(toggle_row(
                                cx,
                                t!("App.RightPanel.Docker.RunDialog.privileged"),
                                t!("App.RightPanel.Docker.RunDialog.privileged_body"),
                                self.privileged,
                                "docker-run-priv",
                                |this| this.privileged = !this.privileged,
                            ))
                            .child(toggle_row(
                                cx,
                                t!("App.RightPanel.Docker.RunDialog.read_only"),
                                t!("App.RightPanel.Docker.RunDialog.read_only_body"),
                                self.read_only,
                                "docker-run-ro",
                                |this| this.read_only = !this.read_only,
                            )),
                    )
                    .child(
                        FormSection::new(t!("App.RightPanel.Docker.run_preview"))
                            .description(t!("App.RightPanel.Docker.RunDialog.preview_body"))
                            .child(
                                div()
                                    .w_full()
                                    .overflow_hidden()
                                    .text_size(SIZE_MONO_SMALL)
                                    .font_family(t.font_mono.clone())
                                    .text_color(t.color.text_secondary)
                                    .px(SP_2)
                                    .py(SP_1_5)
                                    .rounded(crate::theme::radius::RADIUS_SM)
                                    .bg(t.color.bg_canvas)
                                    .border_1()
                                    .border_color(t.color.border_subtle)
                                    .child(preview),
                            ),
                    ),
            )
    }
}

fn field(
    label: impl Into<SharedString>,
    input: &Entity<InputState>,
    hint: Option<SharedString>,
) -> impl IntoElement {
    let mut field = FormField::new(label).child(Input::new(input));
    if let Some(hint) = hint {
        field = field.help(hint);
    }
    field
}

fn toggle_row(
    cx: &mut Context<DockerRunDialog>,
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    value: bool,
    id: &'static str,
    handler: fn(&mut DockerRunDialog),
) -> impl IntoElement {
    SettingRow::new(label)
        .description(description)
        .child(Switch::new(id).checked(value).on_click(cx.listener(
            move |this, _: &bool, _, cx| {
                handler(this);
                cx.notify();
            },
        )))
}

/// Entry point used by the image-row Play button.
pub fn open_docker_run_dialog(
    window: &mut Window,
    cx: &mut App,
    app: WeakEntity<PierApp>,
    image_ref: impl Into<SharedString>,
) {
    let image_ref: SharedString = image_ref.into();
    let name_placeholder: SharedString = t!("App.RightPanel.Docker.RunDialog.name_placeholder")
        .to_string()
        .into();
    let name = cx.new(|c| InputState::new(window, c).placeholder(name_placeholder));
    let command = cx.new(|c| InputState::new(window, c).placeholder(""));
    let ports = cx.new(|c| InputState::new(window, c).multi_line(true).placeholder(""));
    let volumes = cx.new(|c| InputState::new(window, c).multi_line(true).placeholder(""));
    let env = cx.new(|c| InputState::new(window, c).multi_line(true).placeholder(""));
    let network = cx.new(|c| InputState::new(window, c).placeholder("bridge"));
    let restart = cx.new(|c| InputState::new(window, c).placeholder("no"));
    let workdir = cx.new(|c| InputState::new(window, c).placeholder(""));
    let user = cx.new(|c| InputState::new(window, c).placeholder(""));
    let hostname = cx.new(|c| InputState::new(window, c).placeholder(""));
    let entrypoint = cx.new(|c| InputState::new(window, c).placeholder(""));

    let dialog_view = cx.new(|_| DockerRunDialog {
        image: image_ref.clone(),
        name,
        command,
        ports,
        volumes,
        env,
        network,
        restart,
        workdir,
        user,
        hostname,
        entrypoint,
        // Pier's default for one-click run: detached + --rm is what
        // users expect from a GUI ("start it, clean up after itself").
        detached: true,
        auto_remove: true,
        privileged: false,
        read_only: false,
        interactive_tty: false,
    });

    let title: SharedString = format!("{} · {}", t!("App.RightPanel.Docker.run"), image_ref).into();
    let dialog_for_ok = dialog_view.clone();
    window.open_dialog(cx, move |dialog, _w, _app_cx| {
        let app = app.clone();
        let view = dialog_view.clone();
        let ok_view = dialog_for_ok.clone();
        dialog
            .title(title.clone())
            .w(px(RUN_DIALOG_W))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .confirm()
            .button_props(
                gpui_component::dialog::DialogButtonProps::default()
                    .ok_text(SharedString::from(
                        t!("App.RightPanel.Docker.run").to_string(),
                    ))
                    .cancel_text(SharedString::from(t!("App.Common.cancel").to_string())),
            )
            .on_ok(move |_, _w, app_cx| {
                let spec = ok_view.read(app_cx).read_spec(app_cx);
                let _ = app.update(app_cx, |this, cx| this.schedule_docker_run(spec, cx));
                true
            })
            .child(view)
    });
}

fn read_trim(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string().trim().to_string()
}

fn read_lines(input: &Entity<InputState>, cx: &App) -> Vec<String> {
    input
        .read(cx)
        .value()
        .to_string()
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
