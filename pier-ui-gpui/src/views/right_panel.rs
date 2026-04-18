//! Right panel — 10 mode container + vertical icon sidebar.
//!
//! Mirrors `Pier/PierApp/Sources/Views/RightPanel/RightPanelView.swift`.
//! Pier-X adds Postgres + SQLite to Pier's 8 modes (per "功能只能多不能少").
//!
//! Modes pulled from existing views:
//!   - Git    → [`crate::views::git::GitView`]
//!   - DBs    → [`crate::views::database::DatabaseView`] (one struct, 4 kinds)
//!
//! Phase-1 placeholders (visual + descriptive, no backend yet):
//!   - Markdown / Monitor / SFTP / Docker / Logs

use std::rc::Rc;

use gpui::{div, prelude::*, px, App, IntoElement, SharedString, Window};
use gpui_component::{scroll::ScrollableElement, Icon as UiIcon, IconName};
use pier_core::services::server_monitor::ServerSnapshot;

use crate::app::layout::{RightContext, RightMode, RIGHT_ICON_BAR_W};
use crate::components::{text, Button, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_2, SP_3, SP_4},
    theme,
    typography::{SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};
use std::path::PathBuf;

use gpui::Entity;

use crate::app::ssh_session::{
    ConnectStatus, DockerActionKind, DockerInspectState, DockerPanelSnapshot, DockerStatus,
    MonitorStatus, PendingDockerAction, ServiceProbeStatus, ServiceTunnelState, SshSessionState,
    TunnelStatus,
};
use crate::views::database::DatabaseView;
use crate::views::git::GitView;
use crate::views::markdown::MarkdownView;
use crate::views::sftp_browser::{
    GoUpHandler as SftpGoUp, NavigateHandler as SftpNavigate, SftpBrowser,
};

pub type ModeSelector = Rc<dyn Fn(&RightMode, &mut Window, &mut App) + 'static>;
pub type DockerRefreshHandler = Rc<dyn Fn(&(), &mut Window, &mut App) + 'static>;
pub type DockerActionHandler = Rc<dyn Fn(&DockerActionRequest, &mut Window, &mut App) + 'static>;

#[derive(Clone, Debug)]
pub struct DockerActionRequest {
    pub kind: DockerActionKind,
    pub target_id: String,
    pub target_label: String,
}

#[derive(IntoElement)]
pub struct RightPanel {
    active_mode: RightMode,
    /// Most recently opened `.md` file (set by the file tree, consumed by
    /// the Markdown mode). `None` shows the empty-state card.
    current_markdown: Option<PathBuf>,
    active_session: Option<Entity<SshSessionState>>,
    sftp_navigate: SftpNavigate,
    sftp_go_up: SftpGoUp,
    docker_refresh: DockerRefreshHandler,
    docker_action: DockerActionHandler,
    on_select_mode: ModeSelector,
}

impl RightPanel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        active_mode: RightMode,
        current_markdown: Option<PathBuf>,
        active_session: Option<Entity<SshSessionState>>,
        sftp_navigate: SftpNavigate,
        sftp_go_up: SftpGoUp,
        docker_refresh: DockerRefreshHandler,
        docker_action: DockerActionHandler,
        on_select_mode: ModeSelector,
    ) -> Self {
        Self {
            active_mode,
            current_markdown,
            active_session,
            sftp_navigate,
            sftp_go_up,
            docker_refresh,
            docker_action,
            on_select_mode,
        }
    }
}

impl RenderOnce for RightPanel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx).clone();
        let RightPanel {
            active_mode,
            current_markdown,
            active_session,
            sftp_navigate,
            sftp_go_up,
            docker_refresh,
            docker_action,
            on_select_mode,
        } = self;
        let available_modes = available_right_modes(active_session.as_ref(), cx);

        let body = render_mode_body(
            &t,
            active_mode,
            current_markdown,
            active_session.clone(),
            sftp_navigate,
            sftp_go_up,
            docker_refresh,
            docker_action,
            on_select_mode.clone(),
            cx,
        );

        div()
            .h_full()
            .flex()
            .flex_row()
            .bg(t.color.bg_panel)
            .border_l_1()
            .border_color(t.color.border_subtle)
            // Content
            .child(div().flex_1().min_w(px(0.0)).child(body))
            // Vertical separator
            .child(div().w(px(1.0)).h_full().bg(t.color.border_subtle))
            // Icon sidebar (far right)
            .child(render_icon_sidebar(
                &t,
                active_mode,
                &available_modes,
                on_select_mode,
            ))
    }
}

#[allow(clippy::too_many_arguments)]
fn render_mode_body(
    t: &crate::theme::Theme,
    mode: RightMode,
    current_markdown: Option<PathBuf>,
    active_session: Option<Entity<SshSessionState>>,
    sftp_navigate: SftpNavigate,
    sftp_go_up: SftpGoUp,
    docker_refresh: DockerRefreshHandler,
    docker_action: DockerActionHandler,
    on_select_mode: ModeSelector,
    cx: &mut App,
) -> gpui::AnyElement {
    let status = mode_status_bar(t, mode, active_session.as_ref(), cx);
    let remote_overview = remote_overview(active_session.as_ref(), cx);

    let content: gpui::AnyElement = match mode {
        RightMode::Markdown => MarkdownView::new(current_markdown).into_any_element(),
        RightMode::Monitor => monitor_view(t, active_session.as_ref(), cx).into_any_element(),
        RightMode::Sftp => {
            SftpBrowser::new(active_session.clone(), sftp_navigate, sftp_go_up).into_any_element()
        }
        RightMode::Docker => docker_view(
            t,
            active_session.as_ref(),
            docker_refresh,
            docker_action,
            cx,
        )
        .into_any_element(),
        RightMode::Git => GitView::new().into_any_element(),
        RightMode::Mysql | RightMode::Postgres | RightMode::Redis | RightMode::Sqlite => {
            DatabaseView::new(mode.db_kind().expect("db mode")).into_any_element()
        }
        RightMode::Logs => placeholder(
            "Logs",
            "Tail remote log files via SSH exec.",
            "Multi-file watcher + level filtering + JSON formatting land in Phase 4.",
        )
        .into_any_element(),
    };

    let mut panel = div().h_full().flex().flex_col().child(status);
    if let Some(overview) = remote_overview.as_ref() {
        panel = panel
            .child(render_services_strip(
                t,
                mode,
                overview,
                on_select_mode.clone(),
            ))
            .child(render_ssh_strip(t, overview));
    }

    panel
        .child(if matches!(mode, RightMode::Sftp) {
            div()
                .flex_1()
                .min_h(px(0.0))
                .child(content)
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scrollbar()
                .child(div().w_full().child(content))
                .into_any_element()
        })
        .into_any_element()
}

#[derive(Clone)]
struct RemoteOverview {
    ssh_command: SharedString,
    services: Vec<pier_core::ssh::DetectedService>,
    service_probe_status: ServiceProbeStatus,
    service_probe_error: Option<SharedString>,
    last_error: Option<SharedString>,
    tunnels: Vec<TunnelOverview>,
}

#[derive(Clone)]
struct TunnelOverview {
    service_name: SharedString,
    remote_port: u16,
    local_port: Option<u16>,
    status: TunnelStatus,
    last_error: Option<SharedString>,
}

fn available_right_modes(
    active_session: Option<&Entity<SshSessionState>>,
    cx: &App,
) -> Vec<RightMode> {
    active_session
        .map(|session| session.read(cx).available_modes())
        .unwrap_or_else(|| RightMode::LOCAL_ONLY.into_iter().collect())
}

fn remote_overview(
    active_session: Option<&Entity<SshSessionState>>,
    cx: &App,
) -> Option<RemoteOverview> {
    active_session.map(|session_entity| {
        let session = session_entity.read(cx);
        RemoteOverview {
            ssh_command: remote_ssh_command(&session.config),
            services: session.services.clone(),
            service_probe_status: session.service_probe_status.clone(),
            service_probe_error: session.service_probe_error.clone().map(SharedString::from),
            last_error: session.last_error.clone().map(SharedString::from),
            tunnels: session.tunnels.iter().map(tunnel_overview).collect(),
        }
    })
}

fn tunnel_overview(tunnel: &ServiceTunnelState) -> TunnelOverview {
    TunnelOverview {
        service_name: tunnel.service_name.clone().into(),
        remote_port: tunnel.remote_port,
        local_port: tunnel.local_port,
        status: tunnel.status,
        last_error: tunnel.last_error.clone().map(SharedString::from),
    }
}

fn render_services_strip(
    t: &crate::theme::Theme,
    active_mode: RightMode,
    overview: &RemoteOverview,
    on_select: ModeSelector,
) -> impl IntoElement {
    let mut row = div()
        .px(SP_3)
        .py(SP_2)
        .flex()
        .flex_row()
        .flex_wrap()
        .items_start()
        .gap(SP_2)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(remote_strip_label(t, "Services"));

    let services = if matches!(overview.service_probe_status, ServiceProbeStatus::Probing)
        && overview.services.is_empty()
    {
        div()
            .flex_1()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .child(StatusPill::new("detecting", StatusKind::Info))
            .child(
                div()
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child("probing mysql, postgres, redis and docker"),
            )
            .into_any_element()
    } else if overview.services.is_empty() {
        div()
            .flex_1()
            .text_size(SIZE_SMALL)
            .text_color(t.color.text_tertiary)
            .child("no supported remote services detected")
            .into_any_element()
    } else {
        let mut pills = div().flex_1().flex().flex_row().flex_wrap().gap(SP_2);
        for service in &overview.services {
            if let Some(mode) = RightMode::from_service_name(&service.name) {
                let tunnel = overview
                    .tunnels
                    .iter()
                    .find(|tunnel| tunnel.service_name.as_ref() == service.name.as_str());
                pills = pills.child(service_button(
                    t,
                    mode,
                    service,
                    tunnel,
                    mode == active_mode,
                    on_select.clone(),
                ));
            }
        }
        pills.into_any_element()
    };

    row = row.child(services);
    if let Some(err) = overview.service_probe_error.clone() {
        row = row.child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.status_error)
                .child(err),
        );
    }
    row
}

fn render_ssh_strip(t: &crate::theme::Theme, overview: &RemoteOverview) -> impl IntoElement {
    let mut row = div()
        .px(SP_3)
        .py(SP_2)
        .flex()
        .flex_row()
        .flex_wrap()
        .items_start()
        .gap(SP_2)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(remote_strip_label(t, "SSH"))
        .child(
            div()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_secondary)
                .child(overview.ssh_command.clone()),
        );

    if overview.tunnels.is_empty() {
        row = row.child(StatusPill::new("no tunnels", StatusKind::Warning));
    } else {
        let mut tunnel_row = div().flex().flex_row().flex_wrap().gap(SP_2);
        for tunnel in &overview.tunnels {
            tunnel_row = tunnel_row.child(tunnel_chip(tunnel));
        }
        row = row.child(tunnel_row);
    }

    if let Some(err) = overview.last_error.clone() {
        row = row.child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.status_error)
                .child(err),
        );
    }
    row
}

fn remote_strip_label(t: &crate::theme::Theme, label: &'static str) -> impl IntoElement {
    div()
        .w(px(52.0))
        .pt(px(2.0))
        .text_size(SIZE_CAPTION)
        .font_weight(WEIGHT_MEDIUM)
        .text_color(t.color.text_tertiary)
        .child(label)
}

fn service_button(
    t: &crate::theme::Theme,
    mode: RightMode,
    service: &pier_core::ssh::DetectedService,
    tunnel: Option<&TunnelOverview>,
    is_active: bool,
    on_select: ModeSelector,
) -> impl IntoElement {
    let id: SharedString = format!("service-{}", service.name).into();
    let mut label = mode.label().to_string();
    if !service.version.is_empty() && service.version != "unknown" {
        label.push(' ');
        label.push_str(&service.version);
    }
    if let Some(local_port) = tunnel
        .filter(|tunnel| matches!(tunnel.status, TunnelStatus::Active))
        .and_then(|tunnel| tunnel.local_port)
    {
        label.push_str(&format!(" · {local_port}"));
    }

    let mut chip = div()
        .id(gpui::ElementId::Name(id))
        .min_h(px(22.0))
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(if is_active {
            t.color.accent_muted
        } else {
            t.color.border_subtle
        })
        .bg(if is_active {
            t.color.accent_subtle
        } else {
            t.color.bg_surface
        })
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(t.color.bg_hover)
                .border_color(t.color.border_default)
        })
        .on_click(move |_, w, app| on_select(&mode, w, app))
        .child(
            div()
                .w(px(6.0))
                .h(px(6.0))
                .rounded(px(3.0))
                .bg(service_status_color(t, service.status)),
        )
        .child(
            div()
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(if is_active {
                    t.color.accent
                } else {
                    t.color.text_secondary
                })
                .child(label),
        );

    if let Some(tunnel) = tunnel.filter(|tunnel| matches!(tunnel.status, TunnelStatus::Failed)) {
        chip = chip.child(
            div()
                .text_size(SIZE_CAPTION)
                .text_color(t.color.status_error)
                .child(
                    tunnel
                        .last_error
                        .clone()
                        .unwrap_or_else(|| "tunnel failed".into()),
                ),
        );
    }
    chip
}

fn tunnel_chip(tunnel: &TunnelOverview) -> impl IntoElement {
    let service_label = service_display_name(tunnel.service_name.as_ref());
    let label: SharedString = match (tunnel.status, tunnel.local_port) {
        (TunnelStatus::Active, Some(local_port)) => format!(
            "{service_label} localhost:{local_port} -> {}",
            tunnel.remote_port
        )
        .into(),
        (TunnelStatus::Opening, _) => {
            format!("{service_label} opening -> {}", tunnel.remote_port).into()
        }
        (TunnelStatus::Failed, _) => {
            format!("{service_label} error -> {}", tunnel.remote_port).into()
        }
        (TunnelStatus::Active, None) => format!("{service_label} -> {}", tunnel.remote_port).into(),
    };
    StatusPill::new(label, tunnel_status_kind(tunnel.status)).into_any_element()
}

fn mode_status_bar(
    t: &crate::theme::Theme,
    mode: RightMode,
    active_session: Option<&Entity<SshSessionState>>,
    cx: &App,
) -> impl IntoElement {
    let (context_label, kind, endpoint) = match mode.context() {
        RightContext::Local => ("local".into(), StatusKind::Success, None),
        RightContext::Remote => active_session
            .map(|session_entity| {
                let session = session_entity.read(cx);
                let (label, kind) = remote_status_pill(&session.status);
                (label, kind, Some(remote_endpoint_label(&session.config)))
            })
            .unwrap_or_else(|| ("no session".into(), StatusKind::Warning, None)),
    };

    let mut row = div()
        .h(px(28.0))
        .px(SP_3)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(
            div()
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child(mode.label()),
        )
        .child(StatusPill::new(context_label, kind));

    if let Some(endpoint) = endpoint {
        row = row.child(
            div()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_tertiary)
                .child(endpoint),
        );
    }

    row
}

fn remote_status_pill(status: &ConnectStatus) -> (SharedString, StatusKind) {
    match status {
        ConnectStatus::Idle => ("idle".into(), StatusKind::Warning),
        ConnectStatus::Connecting => ("connecting".into(), StatusKind::Info),
        ConnectStatus::Refreshing => ("loading".into(), StatusKind::Info),
        ConnectStatus::Connected => ("connected".into(), StatusKind::Success),
        ConnectStatus::Failed => ("error".into(), StatusKind::Error),
    }
}

fn remote_endpoint_label(config: &pier_core::ssh::SshConfig) -> SharedString {
    if config.port == 22 {
        format!("{}@{}", config.user, config.host).into()
    } else {
        format!("{}@{}:{}", config.user, config.host, config.port).into()
    }
}

fn remote_ssh_command(config: &pier_core::ssh::SshConfig) -> SharedString {
    if config.port == 22 {
        format!("ssh {}@{}", config.user, config.host).into()
    } else {
        format!("ssh {}@{} -p {}", config.user, config.host, config.port).into()
    }
}

fn service_status_color(
    t: &crate::theme::Theme,
    status: pier_core::ssh::ServiceStatus,
) -> gpui::Hsla {
    match status {
        pier_core::ssh::ServiceStatus::Running => t.color.status_success.into(),
        pier_core::ssh::ServiceStatus::Stopped => t.color.status_warning.into(),
        pier_core::ssh::ServiceStatus::Installed => t.color.status_info.into(),
    }
}

fn tunnel_status_kind(status: TunnelStatus) -> StatusKind {
    match status {
        TunnelStatus::Opening => StatusKind::Info,
        TunnelStatus::Active => StatusKind::Success,
        TunnelStatus::Failed => StatusKind::Error,
    }
}

fn service_display_name(service_name: &str) -> SharedString {
    RightMode::from_service_name(service_name)
        .map(RightMode::label)
        .unwrap_or_else(|| service_name.to_string().into())
}

fn monitor_view(
    t: &crate::theme::Theme,
    active_session: Option<&Entity<SshSessionState>>,
    cx: &App,
) -> impl IntoElement {
    let Some(session_entity) = active_session else {
        return placeholder(
            "Monitor",
            "No active SSH session.",
            "Open a saved connection from the left panel to start remote monitoring.",
        )
        .into_any_element();
    };

    let (endpoint, status, snapshot, error) = {
        let session = session_entity.read(cx);
        (
            remote_endpoint_label(&session.config),
            session.monitor_status.clone(),
            session.monitor_snapshot.clone(),
            session.monitor_error.clone().map(SharedString::from),
        )
    };

    let mut col = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(SP_3)
        .p(SP_4)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_3)
                .child(text::h2("Remote Monitor"))
                .child(StatusPill::new(monitor_status_label(&status), monitor_status_kind(&status))),
        )
        .child(
            Card::new()
                .padding(SP_3)
                .child(SectionLabel::new("Target"))
                .child(text::mono(endpoint.clone()))
                .child(
                    text::body(
                        "5 second polling over the active SSH session. CPU / memory / disk are live; network / GPU / process charts land in a follow-up slice.",
                    )
                    .secondary(),
                ),
        );

    if let Some(err) = error {
        col = col.child(
            Card::new()
                .padding(SP_3)
                .child(SectionLabel::new("Probe Error"))
                .child(text::body(err).secondary()),
        );
    }

    let Some(snapshot) = snapshot else {
        let empty_label = match status {
            MonitorStatus::Loading => "collecting remote metrics...",
            MonitorStatus::Failed => "last probe failed before any sample was stored",
            MonitorStatus::Idle => "monitor starts when this panel is active",
            MonitorStatus::Ready => "waiting for first sample",
        };
        return col
            .child(
                Card::new()
                    .padding(SP_3)
                    .child(SectionLabel::new("Status"))
                    .child(text::body(empty_label).secondary()),
            )
            .into_any_element();
    };

    let mut grid = div().flex().flex_row().flex_wrap().gap(SP_3);
    grid = grid.child(monitor_meter_card(
        t,
        "CPU",
        percentage_label(snapshot.cpu_pct),
        load_label(snapshot.load_1, snapshot.load_5, snapshot.load_15),
        percent_ratio(snapshot.cpu_pct),
        t.color.accent,
    ));
    grid = grid.child(monitor_meter_card(
        t,
        "Memory",
        memory_primary(&snapshot),
        memory_secondary(&snapshot),
        memory_ratio(&snapshot),
        t.color.status_info,
    ));
    grid = grid.child(monitor_meter_card(
        t,
        "Disk /",
        disk_primary(&snapshot),
        disk_secondary(&snapshot),
        percent_ratio(snapshot.disk_use_pct),
        disk_color(t, snapshot.disk_use_pct),
    ));
    grid = grid.child(monitor_meter_card(
        t,
        "Swap",
        swap_primary(&snapshot),
        swap_secondary(&snapshot),
        swap_ratio(&snapshot),
        t.color.status_warning,
    ));
    grid = grid.child(monitor_detail_card(
        "Load",
        compact_label(snapshot.load_1),
        format!(
            "5m {} · 15m {}",
            compact_label(snapshot.load_5),
            compact_label(snapshot.load_15)
        )
        .into(),
    ));
    grid = grid.child(monitor_detail_card(
        "Uptime",
        if snapshot.uptime.is_empty() {
            "—".into()
        } else {
            snapshot.uptime.clone().into()
        },
        format!(
            "root free {} of {}",
            empty_dash(&snapshot.disk_avail),
            empty_dash(&snapshot.disk_total)
        )
        .into(),
    ));

    col.child(grid).into_any_element()
}

fn docker_view(
    t: &crate::theme::Theme,
    active_session: Option<&Entity<SshSessionState>>,
    on_refresh: DockerRefreshHandler,
    on_action: DockerActionHandler,
    cx: &App,
) -> gpui::AnyElement {
    let Some(session_entity) = active_session else {
        return placeholder(
            "Docker",
            "No active SSH session.",
            "Open a saved connection from the left panel to inspect remote containers.",
        )
        .into_any_element();
    };

    let (endpoint, status, snapshot, error, pending, action_error, inspect) = {
        let session = session_entity.read(cx);
        (
            remote_endpoint_label(&session.config),
            session.docker_status.clone(),
            session.docker_snapshot.clone(),
            session.docker_error.clone().map(SharedString::from),
            session.docker_pending_action.clone(),
            session.docker_action_error.clone().map(SharedString::from),
            session.docker_inspect.clone(),
        )
    };
    let has_snapshot = snapshot.is_some();

    let mut header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_3)
        .child(text::h2("Docker"))
        .child(StatusPill::new(
            docker_status_label(&status, has_snapshot),
            docker_status_kind(&status),
        ));
    if let Some(action) = pending.as_ref() {
        header = header.child(StatusPill::new(
            docker_pending_label(action),
            StatusKind::Info,
        ));
    }

    let refresh_control = if pending.is_some() {
        StatusPill::new("busy", StatusKind::Warning).into_any_element()
    } else {
        Button::ghost("docker-refresh", "Refresh")
            .on_click(move |_, window, app| on_refresh(&(), window, app))
            .into_any_element()
    };

    let mut col = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(SP_3)
        .p(SP_4)
        .child(header)
        .child(
            Card::new().padding(SP_3).child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_start()
                    .gap(SP_3)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(SP_1)
                            .child(SectionLabel::new("Target"))
                            .child(text::mono(endpoint.clone()))
                            .child(
                                text::body(
                                    "Inventory and container actions run over the active SSH session using the remote docker CLI.",
                                )
                                .secondary(),
                            ),
                    )
                    .child(refresh_control),
            ),
        );

    if let Some(err) = error {
        col = col.child(
            Card::new()
                .padding(SP_3)
                .child(SectionLabel::new("Refresh Error"))
                .child(text::body(err).secondary()),
        );
    }
    if let Some(err) = action_error {
        col = col.child(
            Card::new()
                .padding(SP_3)
                .child(SectionLabel::new("Action Error"))
                .child(text::body(err).secondary()),
        );
    }

    let Some(snapshot) = snapshot else {
        let empty_label = match status {
            DockerStatus::Loading => "collecting container, image, volume and network inventory...",
            DockerStatus::Failed => "docker probe failed before any inventory was stored",
            DockerStatus::Idle => "docker inventory starts when this panel is active",
            DockerStatus::Ready => "waiting for first docker inventory sample",
        };
        return col
            .child(
                Card::new()
                    .padding(SP_3)
                    .child(SectionLabel::new("Status"))
                    .child(text::body(empty_label).secondary()),
            )
            .into_any_element();
    };

    col = col
        .child(docker_summary_card(&snapshot))
        .child(docker_containers_card(
            t,
            &snapshot,
            pending.as_ref(),
            on_action.clone(),
        ))
        .child(docker_images_card(t, &snapshot))
        .child(docker_storage_card(t, &snapshot));

    if let Some(inspect_state) = inspect.as_ref() {
        col = col.child(docker_inspect_card(t, inspect_state));
    }

    col.into_any_element()
}

fn monitor_meter_card(
    t: &crate::theme::Theme,
    title: &'static str,
    primary: SharedString,
    secondary: SharedString,
    ratio: Option<f32>,
    fill: gpui::Rgba,
) -> impl IntoElement {
    let bar_w = 148.0;
    div().w(px(176.0)).child(
        Card::new()
            .padding(SP_3)
            .child(SectionLabel::new(title))
            .child(text::body(primary))
            .child(text::body(secondary).secondary())
            .child(div().pt(SP_2).child(monitor_bar(t, bar_w, ratio, fill))),
    )
}

fn monitor_detail_card(
    title: &'static str,
    primary: SharedString,
    secondary: SharedString,
) -> impl IntoElement {
    div().w(px(176.0)).child(
        Card::new()
            .padding(SP_3)
            .child(SectionLabel::new(title))
            .child(text::body(primary))
            .child(text::body(secondary).secondary()),
    )
}

fn monitor_bar(
    t: &crate::theme::Theme,
    width: f32,
    ratio: Option<f32>,
    fill: gpui::Rgba,
) -> impl IntoElement {
    div()
        .w(px(width))
        .h(px(6.0))
        .rounded(px(3.0))
        .bg(t.color.bg_panel)
        .child(
            div()
                .w(px(width * ratio.unwrap_or(0.0).clamp(0.0, 1.0)))
                .h_full()
                .rounded(px(3.0))
                .bg(fill),
        )
}

fn monitor_status_label(status: &MonitorStatus) -> SharedString {
    match status {
        MonitorStatus::Idle => "idle".into(),
        MonitorStatus::Loading => "polling".into(),
        MonitorStatus::Ready => "live 5s".into(),
        MonitorStatus::Failed => "stale".into(),
    }
}

fn monitor_status_kind(status: &MonitorStatus) -> StatusKind {
    match status {
        MonitorStatus::Idle => StatusKind::Warning,
        MonitorStatus::Loading => StatusKind::Info,
        MonitorStatus::Ready => StatusKind::Success,
        MonitorStatus::Failed => StatusKind::Error,
    }
}

fn percentage_label(value: f64) -> SharedString {
    if value < 0.0 {
        "—".into()
    } else {
        format!("{value:.1}%").into()
    }
}

fn compact_label(value: f64) -> SharedString {
    if value < 0.0 {
        "—".into()
    } else {
        format!("{value:.2}").into()
    }
}

fn percent_ratio(value: f64) -> Option<f32> {
    (value >= 0.0).then_some((value / 100.0) as f32)
}

fn memory_ratio(snapshot: &ServerSnapshot) -> Option<f32> {
    if snapshot.mem_total_mb <= 0.0 || snapshot.mem_used_mb < 0.0 {
        None
    } else {
        Some((snapshot.mem_used_mb / snapshot.mem_total_mb) as f32)
    }
}

fn swap_ratio(snapshot: &ServerSnapshot) -> Option<f32> {
    if snapshot.swap_total_mb <= 0.0 || snapshot.swap_used_mb < 0.0 {
        None
    } else {
        Some((snapshot.swap_used_mb / snapshot.swap_total_mb) as f32)
    }
}

fn memory_primary(snapshot: &ServerSnapshot) -> SharedString {
    if snapshot.mem_total_mb <= 0.0 || snapshot.mem_used_mb < 0.0 {
        "—".into()
    } else {
        format!("{:.0} MB used", snapshot.mem_used_mb).into()
    }
}

fn memory_secondary(snapshot: &ServerSnapshot) -> SharedString {
    if snapshot.mem_total_mb <= 0.0 {
        "memory counters unavailable".into()
    } else {
        format!(
            "{:.0} MB free of {:.0} MB",
            snapshot.mem_free_mb.max(0.0),
            snapshot.mem_total_mb
        )
        .into()
    }
}

fn disk_primary(snapshot: &ServerSnapshot) -> SharedString {
    if snapshot.disk_use_pct < 0.0 {
        "—".into()
    } else {
        format!("{} used", empty_dash(&snapshot.disk_used)).into()
    }
}

fn disk_secondary(snapshot: &ServerSnapshot) -> SharedString {
    if snapshot.disk_use_pct < 0.0 {
        "disk counters unavailable".into()
    } else {
        format!(
            "{} free · {}",
            empty_dash(&snapshot.disk_avail),
            percentage_label(snapshot.disk_use_pct)
        )
        .into()
    }
}

fn swap_primary(snapshot: &ServerSnapshot) -> SharedString {
    if snapshot.swap_total_mb <= 0.0 {
        "not available".into()
    } else {
        format!("{:.0} MB used", snapshot.swap_used_mb.max(0.0)).into()
    }
}

fn swap_secondary(snapshot: &ServerSnapshot) -> SharedString {
    if snapshot.swap_total_mb <= 0.0 {
        "host has no swap or probe unsupported".into()
    } else {
        format!("{:.0} MB total", snapshot.swap_total_mb).into()
    }
}

fn load_label(load_1: f64, load_5: f64, load_15: f64) -> SharedString {
    format!(
        "1m {} · 5m {} · 15m {}",
        compact_label(load_1),
        compact_label(load_5),
        compact_label(load_15)
    )
    .into()
}

fn disk_color(t: &crate::theme::Theme, disk_use_pct: f64) -> gpui::Rgba {
    if disk_use_pct >= 90.0 {
        t.color.status_error
    } else if disk_use_pct >= 75.0 {
        t.color.status_warning
    } else {
        t.color.status_success
    }
}

fn empty_dash(value: &str) -> &str {
    if value.is_empty() {
        "—"
    } else {
        value
    }
}

fn docker_summary_card(snapshot: &DockerPanelSnapshot) -> Card {
    let running = snapshot
        .containers
        .iter()
        .filter(|container| container.is_running())
        .count();

    Card::new()
        .padding(SP_3)
        .child(SectionLabel::new("Inventory"))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(SP_2)
                .child(StatusPill::new(
                    format!("{running}/{} running", snapshot.containers.len()),
                    if running > 0 {
                        StatusKind::Success
                    } else {
                        StatusKind::Warning
                    },
                ))
                .child(StatusPill::new(
                    format!("{} images", snapshot.images.len()),
                    StatusKind::Info,
                ))
                .child(StatusPill::new(
                    format!("{} volumes", snapshot.volumes.len()),
                    StatusKind::Info,
                ))
                .child(StatusPill::new(
                    format!("{} networks", snapshot.networks.len()),
                    StatusKind::Info,
                )),
        )
}

fn docker_containers_card(
    t: &crate::theme::Theme,
    snapshot: &DockerPanelSnapshot,
    pending: Option<&PendingDockerAction>,
    on_action: DockerActionHandler,
) -> Card {
    let mut card = Card::new()
        .padding(SP_3)
        .child(SectionLabel::new("Containers"))
        .child(text::body("Running and stopped containers from `docker ps --all`.").secondary());

    if snapshot.containers.is_empty() {
        return card.child(text::body("No containers detected.").secondary());
    }

    for container in &snapshot.containers {
        card = card.child(docker_container_row(
            t,
            container,
            pending,
            on_action.clone(),
        ));
    }
    card
}

fn docker_container_row(
    t: &crate::theme::Theme,
    container: &pier_core::services::docker::Container,
    pending: Option<&PendingDockerAction>,
    on_action: DockerActionHandler,
) -> impl IntoElement {
    let state_label = if container.state.is_empty() {
        empty_dash(&container.status).to_string()
    } else {
        container.state.clone()
    };
    let target_label = if container.names.is_empty() {
        short_docker_id(&container.id)
    } else {
        container.names.clone()
    };
    let pending_for_row = pending.filter(|action| action.target_id == container.id);

    let mut actions = div().flex().flex_row().flex_wrap().justify_end().gap(SP_1);
    if let Some(action) = pending_for_row {
        actions = actions.child(StatusPill::new(
            docker_pending_label(action),
            StatusKind::Info,
        ));
    } else if pending.is_none() {
        let primary_kind = if container.is_running() {
            DockerActionKind::Stop
        } else {
            DockerActionKind::Start
        };
        actions = actions
            .child(docker_action_button(
                primary_kind,
                &container.id,
                &target_label,
                on_action.clone(),
            ))
            .child(docker_action_button(
                DockerActionKind::Restart,
                &container.id,
                &target_label,
                on_action.clone(),
            ))
            .child(docker_action_button(
                DockerActionKind::Inspect,
                &container.id,
                &target_label,
                on_action,
            ));
    }

    div()
        .flex()
        .flex_row()
        .justify_between()
        .items_start()
        .gap(SP_3)
        .p(SP_2)
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(t.color.border_subtle)
        .bg(t.color.bg_panel)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(SP_1)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(SP_2)
                        .child(
                            div()
                                .w(px(6.0))
                                .h(px(6.0))
                                .rounded(px(3.0))
                                .bg(docker_container_state_color(t, &container.state)),
                        )
                        .child(
                            div()
                                .text_size(SIZE_SMALL)
                                .font_weight(WEIGHT_MEDIUM)
                                .text_color(t.color.text_primary)
                                .child(target_label.clone()),
                        )
                        .child(StatusPill::new(
                            state_label.clone(),
                            docker_container_state_kind(&container.state),
                        )),
                )
                .child(
                    div()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_secondary)
                        .child(format!("image: {}", empty_dash(&container.image))),
                )
                .child(
                    div()
                        .text_size(SIZE_SMALL)
                        .font_family(t.font_mono.clone())
                        .text_color(t.color.text_tertiary)
                        .child(format!(
                            "{} · ports {} · created {}",
                            short_docker_id(&container.id),
                            empty_dash(&container.ports),
                            empty_dash(&container.created)
                        )),
                ),
        )
        .child(actions)
}

fn docker_action_button(
    kind: DockerActionKind,
    target_id: &str,
    target_label: &str,
    on_action: DockerActionHandler,
) -> Button {
    let request = DockerActionRequest {
        kind,
        target_id: target_id.to_string(),
        target_label: target_label.to_string(),
    };

    Button::ghost(
        gpui::ElementId::Name(format!("docker-{}-{target_id}", kind.label()).into()),
        docker_action_button_label(kind),
    )
    .on_click(move |_, window, app| on_action(&request, window, app))
}

fn docker_images_card(t: &crate::theme::Theme, snapshot: &DockerPanelSnapshot) -> Card {
    let mut card = Card::new().padding(SP_3).child(SectionLabel::new("Images"));

    if snapshot.images.is_empty() {
        return card.child(text::body("No images detected.").secondary());
    }

    for image in snapshot.images.iter().take(6) {
        let tag = if image.tag.is_empty() {
            "<none>"
        } else {
            image.tag.as_str()
        };
        card = card.child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .gap(SP_3)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(SIZE_MONO_SMALL)
                        .font_family(t.font_mono.clone())
                        .text_color(t.color.text_primary)
                        .child(format!("{}:{tag}", empty_dash(&image.repository))),
                )
                .child(
                    div()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(format!(
                            "{} · {}",
                            empty_dash(&image.size),
                            empty_dash(&image.created)
                        )),
                ),
        );
    }

    if snapshot.images.len() > 6 {
        card = card
            .child(text::body(format!("… +{} more images", snapshot.images.len() - 6)).secondary());
    }
    card
}

fn docker_storage_card(t: &crate::theme::Theme, snapshot: &DockerPanelSnapshot) -> Card {
    let mut card = Card::new()
        .padding(SP_3)
        .child(SectionLabel::new("Volumes & Networks"));

    card = card.child(text::body("Volumes").secondary());
    if snapshot.volumes.is_empty() {
        card = card.child(text::body("No volumes detected.").secondary());
    } else {
        for volume in snapshot.volumes.iter().take(4) {
            card = card.child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .gap(SP_3)
                    .child(
                        div()
                            .text_size(SIZE_SMALL)
                            .font_weight(WEIGHT_MEDIUM)
                            .child(empty_dash(&volume.name).to_string()),
                    )
                    .child(
                        div()
                            .text_size(SIZE_SMALL)
                            .text_color(t.color.text_tertiary)
                            .child(format!(
                                "{} · {}",
                                empty_dash(&volume.driver),
                                empty_dash(&volume.mountpoint)
                            )),
                    ),
            );
        }
    }

    card = card
        .child(div().pt(SP_2))
        .child(text::body("Networks").secondary());
    if snapshot.networks.is_empty() {
        card = card.child(text::body("No networks detected.").secondary());
    } else {
        for network in snapshot.networks.iter().take(4) {
            card = card.child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .gap(SP_3)
                    .child(
                        div()
                            .text_size(SIZE_SMALL)
                            .font_weight(WEIGHT_MEDIUM)
                            .child(empty_dash(&network.name).to_string()),
                    )
                    .child(
                        div()
                            .text_size(SIZE_SMALL)
                            .text_color(t.color.text_tertiary)
                            .child(format!(
                                "{} · {}",
                                empty_dash(&network.driver),
                                empty_dash(&network.scope)
                            )),
                    ),
            );
        }
    }

    card
}

fn docker_inspect_card(t: &crate::theme::Theme, inspect: &DockerInspectState) -> Card {
    Card::new()
        .padding(SP_3)
        .child(SectionLabel::new("Inspect"))
        .child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_secondary)
                .child(format!(
                    "{} ({})",
                    inspect.target_label,
                    short_docker_id(&inspect.target_id)
                )),
        )
        .child(
            div()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_secondary)
                .child(trim_panel_text(&inspect.output, 12_000)),
        )
}

fn docker_status_label(status: &DockerStatus, has_snapshot: bool) -> SharedString {
    match status {
        DockerStatus::Idle => "idle".into(),
        DockerStatus::Loading => "loading".into(),
        DockerStatus::Ready => "live".into(),
        DockerStatus::Failed if has_snapshot => "stale".into(),
        DockerStatus::Failed => "error".into(),
    }
}

fn docker_status_kind(status: &DockerStatus) -> StatusKind {
    match status {
        DockerStatus::Idle => StatusKind::Warning,
        DockerStatus::Loading => StatusKind::Info,
        DockerStatus::Ready => StatusKind::Success,
        DockerStatus::Failed => StatusKind::Error,
    }
}

fn docker_pending_label(action: &PendingDockerAction) -> SharedString {
    format!("{} {}", action.kind.label(), action.target_label).into()
}

fn docker_action_button_label(kind: DockerActionKind) -> &'static str {
    match kind {
        DockerActionKind::Start => "Start",
        DockerActionKind::Stop => "Stop",
        DockerActionKind::Restart => "Restart",
        DockerActionKind::Inspect => "Inspect",
    }
}

fn docker_container_state_kind(state: &str) -> StatusKind {
    if state.eq_ignore_ascii_case("running") {
        StatusKind::Success
    } else if state.eq_ignore_ascii_case("restarting") {
        StatusKind::Info
    } else {
        StatusKind::Warning
    }
}

fn docker_container_state_color(t: &crate::theme::Theme, state: &str) -> gpui::Rgba {
    if state.eq_ignore_ascii_case("running") {
        t.color.status_success
    } else if state.eq_ignore_ascii_case("restarting") {
        t.color.status_info
    } else {
        t.color.status_warning
    }
}

fn short_docker_id(id: &str) -> String {
    id.chars().take(12).collect()
}

fn trim_panel_text(value: &str, max_chars: usize) -> SharedString {
    let total = value.chars().count();
    if total <= max_chars {
        return value.to_string().into();
    }

    let trimmed: String = value.chars().take(max_chars).collect();
    format!("{trimmed}\n… truncated from {total} chars").into()
}

fn placeholder(
    title: &'static str,
    headline: &'static str,
    body: &'static str,
) -> impl IntoElement {
    div()
        .p(SP_4)
        .flex()
        .flex_col()
        .gap(SP_2)
        .child(SectionLabel::new(title))
        .child(text::body(headline))
        .child(text::body(body).secondary())
}

fn render_icon_sidebar(
    t: &crate::theme::Theme,
    active_mode: RightMode,
    available_modes: &[RightMode],
    on_select: ModeSelector,
) -> impl IntoElement {
    let mut col = div()
        .w(RIGHT_ICON_BAR_W)
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .gap(SP_1)
        .py(SP_2)
        .bg(t.color.bg_panel);

    let local_modes: Vec<RightMode> = available_modes
        .iter()
        .copied()
        .filter(|mode| matches!(mode.context(), RightContext::Local))
        .collect();
    let remote_modes: Vec<RightMode> = available_modes
        .iter()
        .copied()
        .filter(|mode| matches!(mode.context(), RightContext::Remote))
        .collect();

    col = col.child(mode_group_marker(t, "L"));
    for mode in local_modes {
        col = col.child(mode_icon_button(
            t,
            mode,
            mode == active_mode,
            on_select.clone(),
        ));
    }
    if !remote_modes.is_empty() {
        col = col
            .child(
                div()
                    .w(px(18.0))
                    .h(px(1.0))
                    .my(SP_1)
                    .bg(t.color.border_subtle),
            )
            .child(mode_group_marker(t, "R"));
        for mode in remote_modes {
            col = col.child(mode_icon_button(
                t,
                mode,
                mode == active_mode,
                on_select.clone(),
            ));
        }
    }
    col
}

fn mode_group_marker(t: &crate::theme::Theme, label: &'static str) -> impl IntoElement {
    div()
        .w(px(18.0))
        .h(px(16.0))
        .flex()
        .items_center()
        .justify_center()
        .text_size(SIZE_CAPTION)
        .font_weight(WEIGHT_MEDIUM)
        .text_color(t.color.text_tertiary)
        .child(label)
}

#[cfg(test)]
mod tests {
    use pier_core::ssh::{AuthMethod, SshConfig};

    use super::{remote_endpoint_label, remote_status_pill};
    use crate::app::ssh_session::ConnectStatus;
    use crate::components::StatusKind;

    #[test]
    fn remote_status_pill_maps_transient_states_to_info() {
        assert_eq!(
            remote_status_pill(&ConnectStatus::Connecting),
            ("connecting".into(), StatusKind::Info)
        );
        assert_eq!(
            remote_status_pill(&ConnectStatus::Refreshing),
            ("loading".into(), StatusKind::Info)
        );
    }

    #[test]
    fn remote_endpoint_label_omits_default_port() {
        let default_port = SshConfig {
            name: "demo".into(),
            host: "example.com".into(),
            port: 22,
            user: "pier".into(),
            auth: AuthMethod::Agent,
            tags: Vec::new(),
            connect_timeout_secs: 5,
        };
        let custom_port = SshConfig {
            port: 2222,
            ..default_port.clone()
        };

        assert_eq!(remote_endpoint_label(&default_port), "pier@example.com");
        assert_eq!(remote_endpoint_label(&custom_port), "pier@example.com:2222");
    }
}

fn mode_icon_button(
    t: &crate::theme::Theme,
    mode: RightMode,
    is_active: bool,
    on_select: ModeSelector,
) -> impl IntoElement {
    let id_str: SharedString = format!("right-icon-{}", mode.id()).into();
    let icon = mode_icon(mode);

    let mut btn = div()
        .id(gpui::ElementId::Name(id_str))
        .w(px(28.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .cursor_pointer()
        .text_color(if is_active {
            t.color.accent
        } else {
            t.color.text_secondary
        })
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(move |_, w, app| on_select(&mode, w, app))
        .child(icon.size(px(16.0)).text_color(if is_active {
            t.color.accent
        } else {
            t.color.text_secondary
        }));

    if is_active {
        btn = btn
            .bg(t.color.accent_subtle)
            .border_1()
            .border_color(t.color.accent_muted);
    }
    btn
}

fn mode_icon(mode: RightMode) -> UiIcon {
    if let Some(asset) = mode.icon_asset() {
        return UiIcon::empty().path(asset);
    }
    let name = match mode {
        RightMode::Markdown => IconName::File,
        RightMode::Monitor => IconName::LayoutDashboard,
        // The remaining modes provide an asset_icon; this is just the
        // exhaustive arm so the match compiles.
        _ => IconName::Frame,
    };
    UiIcon::new(name)
}

// `Card` debug helper for the placeholder — silences an unused warning when
// the placeholder doesn't need additional metadata cards.
#[allow(dead_code)]
fn _hint_card(t: &crate::theme::Theme, label: &'static str) -> Card {
    Card::new().padding(SP_3).child(
        div()
            .text_size(SIZE_MONO_SMALL)
            .font_family(t.font_mono.clone())
            .text_color(t.color.text_tertiary)
            .child(label),
    )
}
