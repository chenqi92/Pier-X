//! Right panel — Pier-aligned mode container + vertical icon sidebar.
//!
//! Mirrors `Pier/PierApp/Sources/Views/RightPanel/RightPanelView.swift`.
//! Postgres remains part of the standard right-panel flow; SQLite stays wired
//! for follow-up work but is not exposed in the default sidebar yet.
//!
//! Modes pulled from existing views:
//!   - Git    → [`crate::views::git::GitView`]
//!   - DBs    → [`crate::views::database::DatabaseView`] (one struct, 4 kinds)
//!
//! Phase-1 placeholders (visual + descriptive, no backend yet):
//!   - Markdown / Monitor / SFTP / Docker / Logs

use std::rc::Rc;

use gpui::{div, prelude::*, px, App, IntoElement, Pixels, SharedString, WeakEntity, Window};
use gpui_component::{input::InputState, scroll::ScrollableElement, Icon as UiIcon, IconName};
use pier_core::services::server_monitor::ServerSnapshot;
use rust_i18n::t;

use crate::app::layout::{DockerTab, RightContext, RightMode, RIGHT_ICON_BAR_W};
use crate::app::PierApp;
use crate::components::{
    data_cell_row, text, Button, ButtonSize, Card, DataCell, DataTone, HeaderSize, IconButton,
    IconButtonSize, IconButtonVariant, InlineInput, InlineInputTone, InspectorSection, PageHeader,
    PropertyRow, SectionLabel, Separator, StatusKind, StatusPill,
};
use crate::theme::{
    heights::{BUTTON_MD_H, ICON_SM, ROW_MD_H, TAB_GLYPH},
    radius::{RADIUS_MD, RADIUS_SM, RADIUS_XS},
    spacing::{SP_0_5, SP_1, SP_1_5, SP_2, SP_3, SP_4},
    theme,
    typography::{SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};
use crate::widgets::{SegmentedControl, SegmentedItem};
use std::path::PathBuf;

use gpui::Entity;

use crate::app::ssh_session::{
    ConnectStatus, DockerActionKind, DockerInspectState, DockerPanelSnapshot, DockerStatus,
    DockerTargetKind, LogLine, LogLineKind, LogsStatus, MonitorStatus, PendingDockerAction,
    ServiceProbeStatus, ServiceTunnelState, SshSessionState, TunnelStatus,
};
use crate::views::database::DatabaseView;
use crate::views::git::GitView;
use crate::views::markdown::MarkdownView;
use crate::views::sftp_browser::{
    DropPathsHandler as SftpDropPaths, GoUpHandler as SftpGoUp,
    HeaderActionHandler as SftpHeaderAction, NavigateHandler as SftpNavigate,
    RowActionHandler as SftpRowAction, SftpBrowser,
};

pub type ModeSelector = Rc<dyn Fn(&RightMode, &mut Window, &mut App) + 'static>;
pub type DockerRefreshHandler = Rc<dyn Fn(&(), &mut Window, &mut App) + 'static>;
pub type DockerActionHandler = Rc<dyn Fn(&DockerActionRequest, &mut Window, &mut App) + 'static>;
pub type DockerTabHandler = Rc<dyn Fn(&DockerTab, &mut Window, &mut App) + 'static>;
pub type LogsActionHandler = Rc<dyn Fn(&LogsAction, &mut Window, &mut App) + 'static>;

const MODE_RAIL_BUTTON_W: Pixels = px(34.0);
const MODE_RAIL_INDICATOR_W: Pixels = px(2.0);
const MODE_RAIL_INDICATOR_H: Pixels = px(16.0);

#[derive(Clone, Debug)]
pub struct DockerActionRequest {
    pub kind: DockerActionKind,
    pub target_id: String,
    pub target_label: String,
    pub target_kind: DockerTargetKind,
}

#[derive(Clone, Debug)]
pub enum LogsAction {
    RunCurrent,
    Stop,
    Clear,
    Preset { command: String },
}

#[derive(IntoElement)]
pub struct RightPanel {
    active_mode: RightMode,
    /// Most recently opened `.md` file (set by the file tree, consumed by
    /// the Markdown mode). `None` shows the empty-state card.
    current_markdown: Option<PathBuf>,
    active_session: Option<Entity<SshSessionState>>,
    logs_command_input: Entity<InputState>,
    /// Weak back-reference to `PierApp`, used by `DatabaseView` (and
    /// any future mode) to read saved DB connections / session state
    /// and to call `schedule_db_*` / `refresh_db_connections`.
    pier_app: WeakEntity<PierApp>,
    sftp_navigate: SftpNavigate,
    sftp_go_up: SftpGoUp,
    sftp_mkdir: SftpHeaderAction,
    sftp_upload: SftpHeaderAction,
    sftp_row_action: SftpRowAction,
    sftp_drop_paths: SftpDropPaths,
    docker_refresh: DockerRefreshHandler,
    docker_action: DockerActionHandler,
    /// Selected Docker sub-view — drives the segmented control.
    docker_tab: DockerTab,
    /// Click handler invoked when the user picks a new segment.
    docker_tab_handler: DockerTabHandler,
    logs_action: LogsActionHandler,
    on_select_mode: ModeSelector,
    /// Current right-panel width (post-clamp). SFTP uses it to decide
    /// whether to reveal the mtime / permissions columns.
    panel_width: Pixels,
}

impl RightPanel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        active_mode: RightMode,
        current_markdown: Option<PathBuf>,
        active_session: Option<Entity<SshSessionState>>,
        logs_command_input: Entity<InputState>,
        pier_app: WeakEntity<PierApp>,
        sftp_navigate: SftpNavigate,
        sftp_go_up: SftpGoUp,
        sftp_mkdir: SftpHeaderAction,
        sftp_upload: SftpHeaderAction,
        sftp_row_action: SftpRowAction,
        sftp_drop_paths: SftpDropPaths,
        docker_refresh: DockerRefreshHandler,
        docker_action: DockerActionHandler,
        docker_tab: DockerTab,
        docker_tab_handler: DockerTabHandler,
        logs_action: LogsActionHandler,
        on_select_mode: ModeSelector,
        panel_width: Pixels,
    ) -> Self {
        Self {
            active_mode,
            current_markdown,
            active_session,
            logs_command_input,
            pier_app,
            sftp_navigate,
            sftp_go_up,
            sftp_mkdir,
            sftp_upload,
            sftp_row_action,
            sftp_drop_paths,
            docker_refresh,
            docker_action,
            docker_tab,
            docker_tab_handler,
            logs_action,
            on_select_mode,
            panel_width,
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
            logs_command_input,
            pier_app,
            sftp_navigate,
            sftp_go_up,
            sftp_mkdir,
            sftp_upload,
            sftp_row_action,
            sftp_drop_paths,
            docker_refresh,
            docker_action,
            docker_tab,
            docker_tab_handler,
            logs_action,
            on_select_mode,
            panel_width,
        } = self;
        let available_modes = available_right_modes(active_session.as_ref(), cx);

        // SFTP content sits to the left of the 36 px vertical icon bar
        // plus a 1 px separator — subtract those so the browser sees its
        // real column budget rather than the outer panel width.
        let sftp_content_width = (panel_width - RIGHT_ICON_BAR_W - px(1.0)).max(px(0.0));

        let body = render_mode_body(
            &t,
            active_mode,
            current_markdown,
            active_session.clone(),
            logs_command_input,
            pier_app,
            sftp_navigate,
            sftp_go_up,
            sftp_mkdir,
            sftp_upload,
            sftp_row_action,
            sftp_drop_paths,
            docker_refresh,
            docker_action,
            docker_tab,
            docker_tab_handler,
            logs_action,
            on_select_mode.clone(),
            sftp_content_width,
            cx,
        );

        div()
            .w_full()
            .h_full()
            .flex()
            .flex_row()
            .bg(t.color.bg_surface)
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
    logs_command_input: Entity<InputState>,
    pier_app: WeakEntity<PierApp>,
    sftp_navigate: SftpNavigate,
    sftp_go_up: SftpGoUp,
    sftp_mkdir: SftpHeaderAction,
    sftp_upload: SftpHeaderAction,
    sftp_row_action: SftpRowAction,
    sftp_drop_paths: SftpDropPaths,
    docker_refresh: DockerRefreshHandler,
    docker_action: DockerActionHandler,
    docker_tab: DockerTab,
    docker_tab_handler: DockerTabHandler,
    logs_action: LogsActionHandler,
    on_select_mode: ModeSelector,
    sftp_content_width: Pixels,
    cx: &mut App,
) -> gpui::AnyElement {
    let shell_header = mode_uses_shell_header(mode)
        .then(|| mode_page_header(mode, active_session.as_ref(), cx).into_any_element());
    let remote_overview = remote_overview(active_session.as_ref(), cx);

    let content: gpui::AnyElement = match mode {
        RightMode::Markdown => MarkdownView::new(current_markdown).into_any_element(),
        RightMode::Monitor => monitor_view(t, active_session.as_ref(), cx).into_any_element(),
        RightMode::Sftp => SftpBrowser::new(
            active_session.clone(),
            sftp_navigate,
            sftp_go_up,
            sftp_mkdir,
            sftp_upload,
            sftp_row_action,
            sftp_drop_paths,
            sftp_content_width,
        )
        .into_any_element(),
        RightMode::Docker => docker_view(
            t,
            active_session.as_ref(),
            docker_tab,
            docker_refresh,
            docker_action,
            docker_tab_handler,
            pier_app.clone(),
            cx,
        )
        .into_any_element(),
        RightMode::Git => GitView::new(pier_app.clone()).into_any_element(),
        RightMode::Mysql | RightMode::Postgres | RightMode::Redis | RightMode::Sqlite => {
            DatabaseView::new(pier_app.clone(), mode.db_kind().expect("db mode")).into_any_element()
        }
        RightMode::Logs => logs_view(
            t,
            active_session.as_ref(),
            logs_command_input,
            logs_action,
            cx,
        )
        .into_any_element(),
    };

    let mut panel = div().w_full().h_full().flex().flex_col();
    if let Some(header) = shell_header {
        panel = panel.child(header);
    }
    // Remote context strip. Only rendered for Remote modes — Local
    // modes (Markdown / Git / SQLite) would show nothing meaningful
    // since the panel reads from disk. The two previous strips
    // (services + ssh) are collapsed into a single compact row so
    // the docker-row / sftp-row content below them gets back the
    // 60-80px of vertical space they were eating.
    if matches!(mode.context(), RightContext::Remote) {
        if let Some(overview) = remote_overview.as_ref() {
            if let Some(strip) =
                render_remote_context_strip(t, mode, overview, on_select_mode.clone())
            {
                panel = panel.child(strip);
            }
        }
    }

    panel
        .child(if matches!(mode, RightMode::Sftp) {
            div()
                .flex_1()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .overflow_hidden()
                .child(content)
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .overflow_y_scrollbar()
                .overflow_x_hidden()
                .child(div().w_full().min_w(px(0.0)).child(content))
                .into_any_element()
        })
        .into_any_element()
}

fn mode_uses_shell_header(mode: RightMode) -> bool {
    !matches!(
        mode,
        RightMode::Markdown
            | RightMode::Sftp
            | RightMode::Git
            | RightMode::Mysql
            | RightMode::Postgres
            | RightMode::Redis
            | RightMode::Sqlite
    )
}

#[derive(Clone)]
struct RemoteOverview {
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

/// Single compact strip that merges the old "Services" row and
/// "SSH" row. Drops the `服务` / `SSH` label columns (pure chrome),
/// drops the "MySQL 8.0.45-0ubuntu0.24.04.1" version string from
/// each chip, and hides the "无隧道" pill when it's the default
/// zero-tunnel state — only a live tunnel or an error raises a pill.
/// The ssh command string is faded out on the far right so the
/// user sees the connection detail without it competing with the
/// service chips for attention.
fn render_remote_context_strip(
    t: &crate::theme::Theme,
    active_mode: RightMode,
    overview: &RemoteOverview,
    on_select: ModeSelector,
) -> Option<gpui::AnyElement> {
    let error = overview
        .service_probe_error
        .clone()
        .or_else(|| overview.last_error.clone());
    let should_show_detecting =
        matches!(overview.service_probe_status, ServiceProbeStatus::Probing)
            && overview.services.is_empty();
    if overview.services.is_empty()
        && overview.tunnels.is_empty()
        && error.is_none()
        && !should_show_detecting
    {
        return None;
    }

    let mut row = div()
        .px(SP_3)
        .py(SP_1)
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .min_w(px(0.0))
        .overflow_hidden()
        .gap(SP_2)
        .border_b_1()
        .border_color(t.color.border_subtle);

    // Service chips — compact, name-only. Tunnel port appended only
    // when the tunnel is actually live (`service_button` handles).
    let services = if should_show_detecting {
        Some(
            StatusPill::new(t!("App.LeftPanel.Services.detecting"), StatusKind::Info)
                .into_any_element(),
        )
    } else if overview.services.is_empty() {
        None
    } else {
        let mut pills = div().flex().flex_row().flex_wrap().gap(SP_1_5);
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
        Some(pills.into_any_element())
    };
    if let Some(services) = services {
        row = row.child(services);
    }

    // Live tunnel pills — only pop in when there's actually a tunnel
    // the user cares about; the "无隧道" warning was noise for the
    // 95% case where no forwarding is configured.
    if !overview.tunnels.is_empty() {
        let mut tunnel_row = div().flex().flex_row().flex_wrap().gap(SP_1_5);
        for tunnel in &overview.tunnels {
            tunnel_row = tunnel_row.child(tunnel_chip(tunnel));
        }
        row = row.child(tunnel_row);
    }

    if let Some(err) = error {
        row = row.child(
            div()
                .w_full()
                .text_size(SIZE_SMALL)
                .text_color(t.color.status_error)
                .child(err),
        );
    }
    Some(row.into_any_element())
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
    // Keep the chip label tight — just the service name. A full
    // version string like "MySQL 8.0.45-0ubuntu0.24.04.1" eats the
    // entire strip width on a typical right-panel column. Version
    // is surfaced in the Monitor / Docker inspector views where the
    // user has deliberately asked for detail.
    let mut label = mode.label().to_string();
    if let Some(local_port) = tunnel
        .filter(|tunnel| matches!(tunnel.status, TunnelStatus::Active))
        .and_then(|tunnel| tunnel.local_port)
    {
        label.push_str(&format!(" · {local_port}"));
    }

    let mut chip = div()
        .id(gpui::ElementId::Name(id))
        .min_h(crate::theme::heights::BUTTON_SM_H)
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
                .child(tunnel.last_error.clone().unwrap_or_else(|| {
                    SharedString::from(t!("App.RightPanel.tunnel_failed").to_string())
                })),
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
        (TunnelStatus::Opening, _) => t!(
            "App.RightPanel.tunnel_opening",
            service = service_label.as_ref(),
            port = tunnel.remote_port
        )
        .into(),
        (TunnelStatus::Failed, _) => t!(
            "App.RightPanel.tunnel_error",
            service = service_label.as_ref(),
            port = tunnel.remote_port
        )
        .into(),
        (TunnelStatus::Active, None) => format!("{service_label} -> {}", tunnel.remote_port).into(),
    };
    StatusPill::new(label, tunnel_status_kind(tunnel.status)).into_any_element()
}

/// Build the PageHeader for the given right-panel mode.
///
/// Only the lightweight inspector-style modes (Monitor / Docker / Logs)
/// use the outer shell header. Modes that already ship their own local
/// control/header row (Markdown / SFTP / Git / DBs) own that space
/// themselves; rendering *both* was the main source of the stacked,
/// repetitive chrome in the right pane.
///
/// Remote shell headers intentionally stay single-line and do not repeat
/// the endpoint string: the terminal tab already shows `user@host`, and
/// the service strip below the header already communicates the remote
/// context.
fn mode_page_header(
    mode: RightMode,
    active_session: Option<&Entity<SshSessionState>>,
    cx: &App,
) -> PageHeader {
    match mode.context() {
        RightContext::Local => PageHeader::new(mode.label())
            .size(HeaderSize::Page)
            .eyebrow(t!("App.Common.local")),
        RightContext::Remote => {
            let eyebrow = mode.label();
            let (title, status_pill) = match active_session {
                Some(session_entity) => {
                    let session = session_entity.read(cx);
                    let (label, kind) = remote_status_pill(&session.status);
                    (
                        SharedString::from(session.config.name.clone()),
                        Some(StatusPill::new(label, kind)),
                    )
                }
                None => (
                    SharedString::from(t!("App.RightPanel.no_session").to_string()),
                    Some(StatusPill::new(
                        t!("App.RightPanel.no_session").to_string(),
                        StatusKind::Warning,
                    )),
                ),
            };
            let mut header = PageHeader::new(title)
                .size(HeaderSize::Page)
                .eyebrow(eyebrow);
            if let Some(pill) = status_pill {
                header = header.status(pill);
            }
            header
        }
    }
}

/// Legacy helper kept around during the transition — identical to the
/// old inline strip but now only used by call sites not yet migrated.
/// Remove once every mode body composes via `mode_page_header`.
#[allow(dead_code)]
fn mode_status_bar(
    t: &crate::theme::Theme,
    mode: RightMode,
    active_session: Option<&Entity<SshSessionState>>,
    cx: &App,
) -> impl IntoElement {
    let (context_label, kind, endpoint) = match mode.context() {
        RightContext::Local => (t!("App.Common.local").into(), StatusKind::Success, None),
        RightContext::Remote => active_session
            .map(|session_entity| {
                let session = session_entity.read(cx);
                let (label, kind) = remote_status_pill(&session.status);
                (label, kind, Some(remote_endpoint_label(&session.config)))
            })
            .unwrap_or_else(|| {
                (
                    t!("App.RightPanel.no_session").into(),
                    StatusKind::Warning,
                    None,
                )
            }),
    };

    let mut row = div()
        .h(ROW_MD_H)
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
        ConnectStatus::Idle => (t!("App.Common.Status.idle").into(), StatusKind::Warning),
        ConnectStatus::Connecting => (t!("App.Common.Status.connecting").into(), StatusKind::Info),
        ConnectStatus::Refreshing => (t!("App.Common.Status.loading").into(), StatusKind::Info),
        ConnectStatus::Connected => (
            t!("App.Common.Status.connected").into(),
            StatusKind::Success,
        ),
        ConnectStatus::Failed => (t!("App.Common.Status.error").into(), StatusKind::Error),
    }
}

fn remote_endpoint_label(config: &pier_core::ssh::SshConfig) -> SharedString {
    if config.port == 22 {
        format!("{}@{}", config.user, config.host).into()
    } else {
        format!("{}@{}:{}", config.user, config.host, config.port).into()
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
            t!("App.RightPanel.Modes.monitor"),
            t!("App.RightPanel.no_active_ssh_session"),
            t!("App.RightPanel.Monitor.placeholder"),
        )
        .into_any_element();
    };

    // Flush inspector grammar: no outer padding, no gap between
    // sections — each `InspectorSection` supplies its own title bar +
    // hairline, and a `Separator::horizontal()` between sections keeps
    // the panel reading as one connected 1px grid (see the reference
    // trading app's right column). The PageHeader rendered above us by
    // `mode_page_header` already carries session name / endpoint /
    // connection status, so we never re-state them here.
    let (status, snapshot, error) = {
        let session = session_entity.read(cx);
        (
            session.monitor_status.clone(),
            session.monitor_snapshot.clone(),
            session.monitor_error.clone().map(SharedString::from),
        )
    };

    let mut col = div().w_full().flex().flex_col().bg(t.color.bg_surface);

    if let Some(err) = error {
        col = col
            .child(
                InspectorSection::new(t!("App.RightPanel.Monitor.probe_error"))
                    .icon(IconName::TriangleAlert)
                    .child(
                        div()
                            .px(SP_3)
                            .py(SP_2)
                            .child(text::caption(err).secondary()),
                    ),
            )
            .child(Separator::horizontal());
    }

    let Some(snapshot) = snapshot else {
        let empty_label = match status {
            MonitorStatus::Loading => t!("App.RightPanel.Monitor.collecting"),
            MonitorStatus::Failed => t!("App.RightPanel.Monitor.failed_before_sample"),
            MonitorStatus::Idle => t!("App.RightPanel.Monitor.idle_hint"),
            MonitorStatus::Ready => t!("App.RightPanel.Monitor.waiting_first_sample"),
        };
        return col
            .child(
                InspectorSection::new(t!("App.Common.status")).child(
                    div()
                        .px(SP_3)
                        .py(SP_2)
                        .child(text::caption(empty_label).secondary()),
                ),
            )
            .into_any_element();
    };

    // Primary stats row: CPU / Memory / Disk — the three resources a
    // user checks first. `data_cell_row` renders equal-width cells
    // separated by 1px subtle rules, so the row reads as a unit rather
    // than three floating tiles. Each cell pulls its bar color from
    // the tone so disk transitions warning→error without a color
    // literal at the call site.
    let primary = data_cell_row(vec![
        DataCell::new("CPU", percentage_label(snapshot.cpu_pct))
            .secondary(t!(
                "App.RightPanel.Monitor.load_tail",
                load_5 = compact_label(snapshot.load_5).as_ref(),
                load_15 = compact_label(snapshot.load_15).as_ref()
            ))
            .tone(DataTone::Accent)
            .when_some(percent_ratio(snapshot.cpu_pct), DataCell::bar),
        DataCell::new(
            t!("App.RightPanel.Monitor.memory"),
            memory_primary(&snapshot),
        )
        .secondary(memory_secondary(&snapshot))
        .when_some(memory_ratio(&snapshot), DataCell::bar),
        DataCell::new(t!("App.RightPanel.Monitor.disk"), disk_primary(&snapshot))
            .secondary(disk_secondary(&snapshot))
            .tone(disk_tone(snapshot.disk_use_pct))
            .when_some(percent_ratio(snapshot.disk_use_pct), DataCell::bar),
    ]);

    // Secondary stats row: Swap / Uptime — less critical, no bar.
    let secondary = data_cell_row(vec![
        DataCell::new(t!("App.RightPanel.Monitor.swap"), swap_primary(&snapshot))
            .secondary(swap_secondary(&snapshot))
            .when_some(swap_ratio(&snapshot), DataCell::bar)
            .tone(DataTone::Warning),
        DataCell::new(
            t!("App.RightPanel.Monitor.uptime"),
            if snapshot.uptime.is_empty() {
                SharedString::from("—")
            } else {
                SharedString::from(snapshot.uptime.clone())
            },
        )
        .secondary(t!(
            "App.RightPanel.Monitor.root_free",
            avail = empty_dash(&snapshot.disk_avail),
            total = empty_dash(&snapshot.disk_total)
        ))
        .mono(),
    ]);

    col.child(
        InspectorSection::new(t!("App.RightPanel.Modes.monitor"))
            .icon(IconName::ChartPie)
            .child(primary)
            .child(Separator::horizontal())
            .child(secondary),
    )
    .child(Separator::horizontal())
    .child(
        InspectorSection::new(t!("App.RightPanel.Monitor.load"))
            .icon(IconName::ChartPie)
            .child(
                PropertyRow::new("monitor-load-1", t!("App.RightPanel.Monitor.load_1m"))
                    .value(text::mono(compact_label(snapshot.load_1))),
            )
            .child(
                PropertyRow::new("monitor-load-5", t!("App.RightPanel.Monitor.load_5m"))
                    .value(text::mono(compact_label(snapshot.load_5))),
            )
            .child(
                PropertyRow::new("monitor-load-15", t!("App.RightPanel.Monitor.load_15m"))
                    .value(text::mono(compact_label(snapshot.load_15))),
            ),
    )
    .into_any_element()
}

/// Translate disk-use percent → inspector tone. Keeps the 75/90
/// thresholds in one place so the DataCell value *and* its bar share
/// the same warn/error signal.
fn disk_tone(pct: f64) -> DataTone {
    if pct >= 90.0 {
        DataTone::Negative
    } else if pct >= 75.0 {
        DataTone::Warning
    } else {
        DataTone::Default
    }
}

fn docker_view(
    t: &crate::theme::Theme,
    active_session: Option<&Entity<SshSessionState>>,
    docker_tab: DockerTab,
    on_refresh: DockerRefreshHandler,
    on_action: DockerActionHandler,
    on_tab: DockerTabHandler,
    pier_app: WeakEntity<PierApp>,
    cx: &App,
) -> gpui::AnyElement {
    let Some(session_entity) = active_session else {
        return placeholder(
            t!("App.RightPanel.Modes.docker"),
            t!("App.RightPanel.no_active_ssh_session"),
            t!("App.RightPanel.Docker.placeholder"),
        )
        .into_any_element();
    };

    let (status, snapshot, error, pending, action_error, inspect) = {
        let session = session_entity.read(cx);
        (
            session.docker_status.clone(),
            session.docker_snapshot.clone(),
            session.docker_error.clone().map(SharedString::from),
            session.docker_pending_action.clone(),
            session.docker_action_error.clone().map(SharedString::from),
            session.docker_inspect.clone(),
        )
    };
    let has_snapshot = snapshot.is_some();

    // Flush Pier grammar: inventory chip strip, segmented tab bar,
    // then a flat run of rows — no nested rounded cards. The outer
    // PageHeader already labels this panel "Docker / host / status";
    // we don't repeat that title here.
    let refresh_btn = IconButton::new("docker-refresh", IconName::RefreshCw)
        .size(IconButtonSize::Sm)
        .variant(IconButtonVariant::Filled)
        .disabled(pending.is_some())
        .on_click(move |_, window, app| on_refresh(&(), window, app));

    // Top strip: one-line status + inventory as a single caption,
    // plus refresh on the right. Matches the Pier Swift reference,
    // where chips are typographic — compact text separated by `·` —
    // instead of 5 pill islands that eat 60px of vertical space.
    let status_pill = StatusPill::new(
        docker_status_label(&status, has_snapshot),
        docker_status_kind(&status),
    );
    let inventory_caption: Option<SharedString> = snapshot.as_ref().map(|snapshot_ref| {
        let running = snapshot_ref
            .containers
            .iter()
            .filter(|container| container.is_running())
            .count();
        format!(
            "{} · {} · {} · {}",
            t!(
                "App.RightPanel.Docker.Inventory.running",
                running = running,
                total = snapshot_ref.containers.len()
            ),
            t!(
                "App.RightPanel.Docker.Inventory.images",
                count = snapshot_ref.images.len()
            ),
            t!(
                "App.RightPanel.Docker.Inventory.volumes",
                count = snapshot_ref.volumes.len()
            ),
            t!(
                "App.RightPanel.Docker.Inventory.networks",
                count = snapshot_ref.networks.len()
            ),
        )
        .into()
    });
    let mut top_strip = div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_1_5)
        .child(status_pill);
    if let Some(caption) = inventory_caption {
        top_strip = top_strip.child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(text::caption(caption).secondary().truncate()),
        );
    } else {
        top_strip = top_strip.child(div().flex_1().min_w(px(0.0)));
    }
    top_strip = top_strip.child(div().flex_none().child(refresh_btn));

    let mut col = div()
        .w_full()
        .flex()
        .flex_col()
        .bg(t.color.bg_surface)
        .child(top_strip)
        .child(Separator::horizontal());

    if let Some(err) = error {
        col = col
            .child(docker_error_strip(
                t,
                t!("App.RightPanel.Docker.refresh_error"),
                err,
            ))
            .child(Separator::horizontal());
    }
    if let Some(err) = action_error {
        col = col
            .child(docker_error_strip(
                t,
                t!("App.RightPanel.Docker.action_error"),
                err,
            ))
            .child(Separator::horizontal());
    }

    let Some(snapshot) = snapshot else {
        let empty_label = match status {
            DockerStatus::Loading => t!("App.RightPanel.Docker.collecting"),
            DockerStatus::Failed => t!("App.RightPanel.Docker.failed_before_sample"),
            DockerStatus::Idle => t!("App.RightPanel.Docker.idle_hint"),
            DockerStatus::Ready => t!("App.RightPanel.Docker.waiting_first_sample"),
        };
        return col
            .child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .child(text::caption(empty_label).secondary()),
            )
            .into_any_element();
    };

    // Segmented tab switcher lives in its own padded strip so the
    // pill outline has breathing room on both sides.
    col = col
        .child(
            div()
                .w_full()
                .px(SP_3)
                .py(SP_1_5)
                .child(docker_tab_bar(docker_tab, on_tab)),
        )
        .child(Separator::horizontal());

    col = col.child(match docker_tab {
        DockerTab::Containers => docker_containers_body(
            t,
            &snapshot,
            pending.as_ref(),
            on_action.clone(),
            pier_app.clone(),
        )
        .into_any_element(),
        DockerTab::Images => docker_images_body(
            t,
            &snapshot,
            pending.as_ref(),
            on_action.clone(),
            pier_app.clone(),
        )
        .into_any_element(),
        DockerTab::Volumes => docker_volumes_body(
            t,
            &snapshot,
            pending.as_ref(),
            inspect.as_ref(),
            on_action.clone(),
        )
        .into_any_element(),
    });

    // Container inspects open as a modal dialog (see
    // `views::docker_dialogs::open_docker_inspect_dialog`), not at
    // the panel bottom. Volume inspects live inline under their row
    // (see `docker_volume_expanded_block`). So this function has no
    // bottom-of-panel inspect rendering — the inspect state still
    // flows through `session.docker_inspect`, the dialog + volume
    // block read from it.
    let _ = inspect; // keep variable live for future routing.

    col.into_any_element()
}

/// One-line error strip — replaces the old standalone error Card.
/// Keeps the flush grammar: icon + title on the left, message on
/// the right. Padded like every other row so the separator above
/// and below it line up with the rest of the panel.
fn docker_error_strip(
    t: &crate::theme::Theme,
    title: impl Into<SharedString>,
    body: SharedString,
) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_1_5)
        .child(
            div()
                .flex_none()
                .text_color(t.color.status_error)
                .child(UiIcon::new(IconName::TriangleAlert).size(ICON_SM)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(SP_0_5)
                .child(text::caption(title).secondary())
                .child(text::caption(body).truncate()),
        )
}

fn logs_view(
    t: &crate::theme::Theme,
    active_session: Option<&Entity<SshSessionState>>,
    logs_command_input: Entity<InputState>,
    on_action: LogsActionHandler,
    cx: &App,
) -> gpui::AnyElement {
    let Some(session_entity) = active_session else {
        return placeholder(
            t!("App.RightPanel.Modes.logs"),
            t!("App.RightPanel.no_active_ssh_session"),
            t!("App.RightPanel.Logs.placeholder"),
        )
        .into_any_element();
    };

    const MAX_VISIBLE_LOG_LINES: usize = 200;

    let (status, visible_lines, retained_count, hidden_count, error, command, exit_code) = {
        let session = session_entity.read(cx);
        let retained_count = session.logs_lines.len();
        let visible_start = retained_count.saturating_sub(MAX_VISIBLE_LOG_LINES);
        (
            session.logs_status.clone(),
            session.logs_lines[visible_start..].to_vec(),
            retained_count,
            visible_start,
            session.logs_error.clone().map(SharedString::from),
            session.logs_command.clone().map(SharedString::from),
            session.logs_exit_code,
        )
    };

    let run_action = LogsAction::RunCurrent;
    let stop_action = LogsAction::Stop;
    let clear_action = LogsAction::Clear;
    let on_run = on_action.clone();
    let on_stop = on_action.clone();
    let on_clear = on_action.clone();

    // Command section actions: Run (primary sm) + Stop / Clear as
    // quiet icon buttons. Status + exit pills live inside the section
    // body as a single eyebrow row so the header stays single-line on
    // narrow panels.
    let command_actions = div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .child(
            Button::primary("logs-run", t!("App.Common.run"))
                .size(ButtonSize::Sm)
                .leading_icon(IconName::Play)
                .on_click(move |_, window, app| on_run(&run_action, window, app)),
        )
        .child(
            IconButton::new("logs-stop", IconName::Square)
                .size(IconButtonSize::Sm)
                .on_click(move |_, window, app| on_stop(&stop_action, window, app)),
        )
        .child(
            IconButton::new("logs-clear", IconName::Delete)
                .size(IconButtonSize::Sm)
                .on_click(move |_, window, app| on_clear(&clear_action, window, app)),
        );

    let mut status_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .px(SP_3)
        .py(SP_1_5)
        .child(StatusPill::new(
            logs_status_label(&status),
            logs_status_kind(&status),
        ));
    if let Some(exit_code) = exit_code {
        status_row = status_row.child(StatusPill::new(
            logs_exit_label(exit_code),
            if exit_code == 0 {
                StatusKind::Success
            } else {
                StatusKind::Warning
            },
        ));
    }

    // Command input + preset row — lives flush inside the section,
    // no surrounding card. Preset buttons downgraded to `Sm` so a
    // 4-wide row fits on a typical 380px panel without wrapping.
    let command_body = div()
        .flex()
        .flex_col()
        .gap(SP_1_5)
        .px(SP_3)
        .pb(SP_2)
        .child(
            InlineInput::new(&logs_command_input)
                .tone(InlineInputTone::Inset)
                .mono(),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(SP_1)
                .child(logs_preset_button(
                    "logs-preset-journal",
                    "Journal",
                    "journalctl -f -n 200 --no-pager",
                    on_action.clone(),
                ))
                .child(logs_preset_button(
                    "logs-preset-syslog",
                    "Syslog",
                    "tail -n 200 -F /var/log/syslog",
                    on_action.clone(),
                ))
                .child(logs_preset_button(
                    "logs-preset-messages",
                    "Messages",
                    "tail -n 200 -F /var/log/messages",
                    on_action.clone(),
                ))
                .child(logs_preset_button(
                    "logs-preset-app",
                    "App.log",
                    "tail -n 200 -F ~/app.log",
                    on_action,
                )),
        );

    let mut col = div()
        .w_full()
        .flex()
        .flex_col()
        .bg(t.color.bg_surface)
        .child(
            InspectorSection::new(t!("App.RightPanel.Logs.command"))
                .icon(IconName::SquareTerminal)
                .actions(command_actions)
                .child(status_row)
                .child(command_body),
        )
        .child(Separator::horizontal());

    if let Some(err) = error {
        col = col
            .child(
                InspectorSection::new(t!("App.RightPanel.Logs.stream_error"))
                    .icon(IconName::TriangleAlert)
                    .child(div().px(SP_3).py(SP_2).child(text::body(err).secondary())),
            )
            .child(Separator::horizontal());
    }

    // Output section. The "active command" echo that used to be its
    // own card is now the section eyebrow — one less stacked card on
    // every refresh. When the stream is live but silent we still want
    // a subtle status line so users know the process is running.
    let retained_pill = if retained_count == 0 {
        StatusPill::new(
            match status {
                LogsStatus::Idle => t!("App.RightPanel.Logs.idle_hint"),
                LogsStatus::Starting => t!("App.RightPanel.Logs.starting_stream"),
                LogsStatus::Live => t!("App.RightPanel.Logs.waiting_first_line"),
                LogsStatus::Stopped => t!("App.RightPanel.Logs.stream_stopped"),
                LogsStatus::Failed => t!("App.RightPanel.Logs.failed_before_line"),
            },
            logs_status_kind(&status),
        )
    } else {
        StatusPill::new(
            t!("App.RightPanel.Logs.retained_lines", count = retained_count),
            StatusKind::Info,
        )
    };

    let mut output_section = InspectorSection::new(t!("App.RightPanel.Logs.output"))
        .icon(IconName::GalleryVerticalEnd)
        .actions(retained_pill);
    if let Some(cmd) = command {
        output_section = output_section.eyebrow(cmd);
    }

    if retained_count == 0 {
        col.child(output_section).into_any_element()
    } else {
        for line in visible_lines.into_iter().rev() {
            output_section = output_section.child(log_line_row(t, line));
        }
        if hidden_count > 0 {
            output_section = output_section.child(
                div().px(SP_3).py(SP_1_5).child(
                    text::caption(t!(
                        "App.RightPanel.Logs.hidden_older_lines",
                        count = hidden_count
                    ))
                    .secondary(),
                ),
            );
        }
        col.child(output_section).into_any_element()
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
        t!("App.RightPanel.Monitor.memory_unavailable").into()
    } else {
        t!(
            "App.RightPanel.Monitor.memory_free_of_total",
            free = format!("{:.0}", snapshot.mem_free_mb.max(0.0)),
            total = format!("{:.0}", snapshot.mem_total_mb)
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
        t!("App.RightPanel.Monitor.disk_unavailable").into()
    } else {
        t!(
            "App.RightPanel.Monitor.disk_free",
            avail = empty_dash(&snapshot.disk_avail),
            pct = percentage_label(snapshot.disk_use_pct).as_ref()
        )
        .into()
    }
}

fn swap_primary(snapshot: &ServerSnapshot) -> SharedString {
    if snapshot.swap_total_mb <= 0.0 {
        t!("App.RightPanel.Monitor.not_available").into()
    } else {
        format!("{:.0} MB used", snapshot.swap_used_mb.max(0.0)).into()
    }
}

fn swap_secondary(snapshot: &ServerSnapshot) -> SharedString {
    if snapshot.swap_total_mb <= 0.0 {
        t!("App.RightPanel.Monitor.swap_unavailable").into()
    } else {
        format!("{:.0} MB total", snapshot.swap_total_mb).into()
    }
}

fn empty_dash(value: &str) -> &str {
    if value.is_empty() {
        "—"
    } else {
        value
    }
}

/// Segmented control that drives the Docker sub-view. Matches the
/// Swift reference: a pill-shaped track with the active segment
/// raised. Fires `on_tab(DockerTab::…)` on click — PierApp's
/// `set_docker_tab` persists the choice and `cx.notify()` repaints.
fn docker_tab_bar(selected: DockerTab, on_tab: DockerTabHandler) -> impl IntoElement {
    let items = DockerTab::ALL.iter().copied().map(|tab| {
        let handler = on_tab.clone();
        SegmentedItem::new(
            gpui::ElementId::Name(format!("docker-tab-{}", tab.id()).into()),
            tab.label(),
            tab == selected,
            move |_ev, window, app| handler(&tab, window, app),
        )
    });
    SegmentedControl::new().items(items)
}

/// Flush container list — no outer card, rows stack directly against
/// the tab-bar separator above them with 1px hairlines in between.
/// Matches the Pier Swift reference (容器 tab).
fn docker_containers_body(
    t: &crate::theme::Theme,
    snapshot: &DockerPanelSnapshot,
    pending: Option<&PendingDockerAction>,
    on_action: DockerActionHandler,
    pier_app: WeakEntity<PierApp>,
) -> gpui::AnyElement {
    if snapshot.containers.is_empty() {
        return div()
            .px(SP_3)
            .py(SP_2)
            .child(text::caption(t!("App.RightPanel.Docker.no_containers")).secondary())
            .into_any_element();
    }
    let mut col = div().w_full().flex().flex_col();
    for container in &snapshot.containers {
        col = col.child(docker_container_row(
            t,
            container,
            pending,
            on_action.clone(),
            pier_app.clone(),
        ));
    }
    col.into_any_element()
}

fn docker_container_row(
    t: &crate::theme::Theme,
    container: &pier_core::services::docker::Container,
    pending: Option<&PendingDockerAction>,
    on_action: DockerActionHandler,
    pier_app: WeakEntity<PierApp>,
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

    let mut actions = div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_0_5);
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
        // Order matches Pier reference: primary (Start/Stop) — Restart
        // — Inspect (quiet) — Delete (destructive, rightmost). The red
        // trash-icon sitting on the far right reads as "permanent" and
        // matches the macOS Finder destructive-action convention.
        actions = actions
            .child(docker_action_icon(
                primary_kind,
                DockerTargetKind::Container,
                &container.id,
                &target_label,
                on_action.clone(),
            ))
            .child(docker_action_icon(
                DockerActionKind::Restart,
                DockerTargetKind::Container,
                &container.id,
                &target_label,
                on_action.clone(),
            ))
            .child(docker_container_inspect_icon(
                &container.id,
                &target_label,
                on_action.clone(),
                pier_app.clone(),
            ))
            .child(docker_action_icon(
                DockerActionKind::Delete,
                DockerTargetKind::Container,
                &container.id,
                &target_label,
                on_action,
            ));
    }

    // Flush row: no rounded box, no panel fill — just a padded
    // flex-row with a 1px bottom hairline. Hover reveals a very
    // subtle surface so the clickable actions feel anchored.
    div()
        .id(gpui::ElementId::Name(
            format!("docker-container-row-{}", short_docker_id(&container.id)).into(),
        ))
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_1_5)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .overflow_hidden()
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex_none()
                .w(px(6.0))
                .h(px(6.0))
                .rounded(px(3.0))
                .bg(docker_container_state_color(t, &container.state)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .overflow_hidden()
                .child(
                    div()
                        .truncate()
                        .text_size(SIZE_SMALL)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_primary)
                        .child(target_label.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(format!(
                            "{} · {}",
                            empty_dash(&container.image),
                            short_docker_id(&container.id)
                        )),
                ),
        )
        .child(div().flex_none().child(StatusPill::new(
            state_label,
            docker_container_state_kind(&container.state),
        )))
        .child(actions)
}

/// Inspect button for a container row. Diverges from `docker_action_icon`
/// in one way: the click handler both **dispatches** the Inspect action
/// (so the backend populates `session.docker_inspect`) and **opens** a
/// modal dialog that reactively renders that state. Previously Inspect
/// output appeared in a section at the bottom of the panel; per Pier's
/// reference the container-inspect pane is a dedicated modal.
fn docker_container_inspect_icon(
    target_id: &str,
    target_label: &str,
    on_action: DockerActionHandler,
    pier_app: WeakEntity<PierApp>,
) -> IconButton {
    let request = DockerActionRequest {
        kind: DockerActionKind::Inspect,
        target_id: target_id.to_string(),
        target_label: target_label.to_string(),
        target_kind: DockerTargetKind::Container,
    };
    let target_id_owned = target_id.to_string();
    let target_label_owned: SharedString = target_label.to_string().into();
    IconButton::new(
        gpui::ElementId::Name(format!("docker-container-inspect-{target_id}").into()),
        IconName::Inspector,
    )
    .size(IconButtonSize::Xs)
    .variant(IconButtonVariant::Ghost)
    .on_click(move |_, window, app| {
        on_action(&request, window, app);
        crate::views::docker_dialogs::open_docker_inspect_dialog(
            window,
            app,
            pier_app.clone(),
            target_id_owned.clone(),
            target_label_owned.clone(),
        );
    })
}

fn docker_action_icon(
    kind: DockerActionKind,
    target_kind: DockerTargetKind,
    target_id: &str,
    target_label: &str,
    on_action: DockerActionHandler,
) -> IconButton {
    let request = DockerActionRequest {
        kind,
        target_id: target_id.to_string(),
        target_label: target_label.to_string(),
        target_kind,
    };
    // Color per action matches the Pier Swift reference:
    // - Stop / Delete → filled red (destructive, unmissable)
    // - Start         → filled accent blue (primary)
    // - Restart       → neutral gray filled (supportive)
    // - Inspect       → ghost (quiet, only on hover)
    let (icon, variant) = match kind {
        DockerActionKind::Start => (IconName::Play, IconButtonVariant::Primary),
        DockerActionKind::Stop => (IconName::Square, IconButtonVariant::Danger),
        DockerActionKind::Restart => (IconName::RefreshCw, IconButtonVariant::Filled),
        DockerActionKind::Inspect => (IconName::Inspector, IconButtonVariant::Ghost),
        DockerActionKind::Delete => (IconName::Delete, IconButtonVariant::Danger),
    };
    IconButton::new(
        gpui::ElementId::Name(format!("docker-{}-{target_id}", kind.label()).into()),
        icon,
    )
    .size(IconButtonSize::Xs)
    .variant(variant)
    .on_click(move |_, window, app| on_action(&request, window, app))
}

/// Flush list of image rows — each row icon + two-line label + size
/// chip + per-row actions. Matches the Pier 镜像 tab. Leading the tab
/// body is a Pull input (non-functional placeholder for now — the
/// ExecStream-backed pull command lands in a follow-up PR) that keeps
/// the visual parity with Pier while the backend is wired.
fn docker_images_body(
    t: &crate::theme::Theme,
    snapshot: &DockerPanelSnapshot,
    pending: Option<&PendingDockerAction>,
    on_action: DockerActionHandler,
    pier_app: WeakEntity<PierApp>,
) -> gpui::AnyElement {
    let mut col = div().w_full().flex().flex_col().child(docker_pull_strip(t));

    if snapshot.images.is_empty() {
        col = col.child(
            div()
                .px(SP_3)
                .py(SP_2)
                .child(text::caption(t!("App.RightPanel.Docker.no_images")).secondary()),
        );
        return col.into_any_element();
    }
    for image in &snapshot.images {
        col = col.child(docker_image_row(
            t,
            image,
            pending,
            on_action.clone(),
            pier_app.clone(),
        ));
    }
    col.into_any_element()
}

/// Pull strip: a disabled-look input + download IconButton. The
/// actual `docker pull` would hit ExecStream so it can report
/// progress — that backend hook is the next step; for now the UI
/// presents the affordance so panel parity with Pier is complete.
fn docker_pull_strip(t: &crate::theme::Theme) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_1_5)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .h(BUTTON_MD_H)
                .flex()
                .flex_row()
                .items_center()
                .px(SP_2)
                .rounded(RADIUS_SM)
                .border_1()
                .border_color(t.color.border_subtle)
                .bg(t.color.bg_panel)
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(
                    t!("App.RightPanel.Docker.pull_placeholder").to_string(),
                )),
        )
        .child(
            IconButton::new("docker-pull", IconName::ArrowDown)
                .size(IconButtonSize::Sm)
                .variant(IconButtonVariant::Primary)
                .disabled(true),
        )
}

fn docker_image_row(
    t: &crate::theme::Theme,
    image: &pier_core::services::docker::DockerImage,
    pending: Option<&PendingDockerAction>,
    on_action: DockerActionHandler,
    pier_app: WeakEntity<PierApp>,
) -> impl IntoElement {
    let tag = if image.tag.is_empty() {
        "<none>".to_string()
    } else {
        image.tag.clone()
    };
    let repo = empty_dash(&image.repository).to_string();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_1_5)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .overflow_hidden()
        .hover(|s| s.bg(t.color.bg_hover))
        .child(
            div()
                .flex_none()
                .text_color(t.color.text_tertiary)
                .child(UiIcon::new(IconName::GalleryVerticalEnd).size(ICON_SM)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .overflow_hidden()
                .child(
                    div()
                        .truncate()
                        .text_size(SIZE_MONO_SMALL)
                        .font_family(t.font_mono.clone())
                        .text_color(t.color.text_primary)
                        .child(format!("{repo}:{tag}")),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(format!(
                            "{} · {}",
                            empty_dash(&image.size),
                            empty_dash(&image.created)
                        )),
                ),
        )
        .child({
            let image_ref: SharedString = format!("{repo}:{tag}").into();
            let pending_for_row = pending.filter(|action| action.target_id == image.id);
            if let Some(action) = pending_for_row {
                div()
                    .flex_none()
                    .child(StatusPill::new(
                        docker_pending_label(action),
                        StatusKind::Info,
                    ))
                    .into_any_element()
            } else if pending.is_none() {
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(SP_0_5)
                    .child(docker_image_run_icon(image_ref.clone(), pier_app.clone()))
                    .child(docker_action_icon(
                        DockerActionKind::Delete,
                        DockerTargetKind::Image,
                        &image.id,
                        image_ref.as_ref(),
                        on_action,
                    ))
                    .into_any_element()
            } else {
                div().flex_none().into_any_element()
            }
        })
}

/// Play-styled IconButton that opens the `docker run` config dialog
/// for the given image reference. Uses Primary (accent) variant so it
/// reads as "the main verb" next to the quieter red trash delete.
fn docker_image_run_icon(image_ref: SharedString, pier_app: WeakEntity<PierApp>) -> IconButton {
    let id_suffix = image_ref.replace([':', '/', '.'], "-");
    IconButton::new(
        gpui::ElementId::Name(format!("docker-image-run-{id_suffix}").into()),
        IconName::Play,
    )
    .size(IconButtonSize::Xs)
    .variant(IconButtonVariant::Primary)
    .on_click(move |_, window, app| {
        crate::views::docker_dialogs::open_docker_run_dialog(
            window,
            app,
            pier_app.clone(),
            image_ref.clone(),
        );
    })
}

/// Flush list of volume rows. Networks, which used to share this
/// section via `volumes_networks`, are dropped from the 存储卷 tab
/// to match the Pier reference (networks belong with a future
/// "Networks" tab, or an overview in the inventory strip above).
/// Top strip carries a `Cleanup unused` ghost button that will
/// eventually dispatch `docker volume prune`.
fn docker_volumes_body(
    t: &crate::theme::Theme,
    snapshot: &DockerPanelSnapshot,
    pending: Option<&PendingDockerAction>,
    inspect: Option<&DockerInspectState>,
    on_action: DockerActionHandler,
) -> gpui::AnyElement {
    let mut col = div().w_full().flex().flex_col().child(
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .px(SP_3)
            .py(SP_1_5)
            .border_b_1()
            .border_color(t.color.border_subtle)
            .child(div().flex_1().min_w(px(0.0)))
            .child(
                Button::secondary(
                    "docker-volume-cleanup",
                    t!("App.RightPanel.Docker.cleanup_volumes"),
                )
                .size(ButtonSize::Sm)
                .leading_icon(IconName::TriangleAlert)
                // No backend yet — the volume-prune call goes through
                // `collect_docker_snapshot` after `docker volume prune`
                // ships. Disable until then so the button still reads
                // as "present but not wired" instead of silently dying.
                .on_click(|_, _, _| {}),
            ),
    );
    if snapshot.volumes.is_empty() {
        col = col.child(
            div()
                .px(SP_3)
                .py(SP_2)
                .child(text::caption(t!("App.RightPanel.Docker.no_volumes")).secondary()),
        );
        return col.into_any_element();
    }
    for volume in &snapshot.volumes {
        let is_expanded = inspect.map(|i| i.target_id == volume.name).unwrap_or(false);
        let is_loading = pending
            .filter(|action| {
                action.target_id == volume.name
                    && matches!(action.kind, DockerActionKind::Inspect)
                    && matches!(action.target_kind, DockerTargetKind::Volume)
            })
            .is_some();
        col = col.child(docker_volume_row(
            t,
            volume,
            pending,
            is_expanded,
            on_action.clone(),
        ));
        if is_loading {
            col = col.child(docker_volume_loading_block(t, volume));
        } else if is_expanded {
            if let Some(inspect_state) = inspect {
                col = col.child(docker_volume_expanded_block(t, inspect_state));
            }
        }
    }
    col.into_any_element()
}

fn docker_volume_row(
    t: &crate::theme::Theme,
    volume: &pier_core::services::docker::DockerVolume,
    pending: Option<&PendingDockerAction>,
    is_expanded: bool,
    on_action: DockerActionHandler,
) -> impl IntoElement {
    let volume_name = empty_dash(&volume.name).to_string();
    let mountpoint = volume.mountpoint.clone();
    let row_id = gpui::ElementId::Name(
        format!("docker-volume-row-{}", short_docker_id(&volume.name)).into(),
    );
    // Tapping the row toggles the expansion. The handler dispatches
    // `Inspect`; `run_docker_action` folds instead when the target is
    // already expanded, so clicking twice is a natural toggle.
    let toggle_handler = {
        let on_action = on_action.clone();
        let target_id = volume.name.clone();
        let target_label = mountpoint.clone();
        move |_: &gpui::ClickEvent, window: &mut Window, app: &mut App| {
            let request = DockerActionRequest {
                kind: DockerActionKind::Inspect,
                target_id: target_id.clone(),
                target_label: target_label.clone(),
                target_kind: DockerTargetKind::Volume,
            };
            on_action(&request, window, app);
        }
    };

    div()
        .id(row_id)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_1_5)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .overflow_hidden()
        .cursor_pointer()
        .when(is_expanded, |el| el.bg(t.color.bg_panel))
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(toggle_handler)
        .child(
            div()
                .flex_none()
                .text_color(t.color.status_warning)
                .child(UiIcon::new(IconName::ChartPie).size(ICON_SM)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .overflow_hidden()
                .child(
                    div()
                        .truncate()
                        .text_size(SIZE_SMALL)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_primary)
                        .child(empty_dash(&volume.name).to_string()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(format!(
                            "{} · {}",
                            empty_dash(&volume.driver),
                            empty_dash(&volume.mountpoint)
                        )),
                ),
        )
        .child({
            let pending_for_row = pending.filter(|action| action.target_id == volume.name);
            if let Some(action) = pending_for_row {
                div()
                    .flex_none()
                    .child(StatusPill::new(
                        docker_pending_label(action),
                        StatusKind::Info,
                    ))
                    .into_any_element()
            } else if pending.is_none() {
                // The trash click must NOT bubble up to the row's
                // click handler — otherwise every delete would also
                // toggle the expansion state behind it. Wrap the
                // IconButton in an id'd div that swallows clicks via
                // GPUI's `.on_click(|ev, ...| ev.stop_propagation())`
                // pattern.
                let delete_wrapper_id = gpui::ElementId::Name(
                    format!(
                        "docker-volume-delete-wrap-{}",
                        short_docker_id(&volume.name)
                    )
                    .into(),
                );
                div()
                    .id(delete_wrapper_id)
                    .flex_none()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(docker_action_icon(
                        DockerActionKind::Delete,
                        DockerTargetKind::Volume,
                        &volume.name,
                        &volume_name,
                        on_action,
                    ))
                    .into_any_element()
            } else {
                div().flex_none().into_any_element()
            }
        })
}

/// Inline "loading" strip shown under a volume row while its
/// `ls -la` is in flight. Mirrors the spot where the output will
/// appear so the row doesn't jump when the data arrives.
fn docker_volume_loading_block(
    t: &crate::theme::Theme,
    volume: &pier_core::services::docker::DockerVolume,
) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(SP_0_5)
        .px(SP_3)
        .py(SP_2)
        .bg(t.color.bg_panel)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(
            div()
                .overflow_hidden()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_tertiary)
                .truncate()
                .child(SharedString::from(volume.mountpoint.clone())),
        )
        .child(text::caption(t!("App.Common.Status.loading")).secondary())
}

/// Inline file-listing block shown under the expanded volume row.
/// Pier ships this as `mountpoint path + ls -la output`. The panel
/// sits on `bg_panel` so it visually nests inside the row above it
/// without needing its own rounded container.
fn docker_volume_expanded_block(
    t: &crate::theme::Theme,
    inspect: &DockerInspectState,
) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(SP_1)
        .px(SP_3)
        .py(SP_2)
        .bg(t.color.bg_panel)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(
            div()
                .overflow_hidden()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_tertiary)
                .truncate()
                .child(SharedString::from(inspect.target_label.clone())),
        )
        .child(
            div()
                .overflow_hidden()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_secondary)
                .child(trim_panel_text(&inspect.output, 12_000)),
        )
}

fn docker_inspect_section(
    t: &crate::theme::Theme,
    inspect: &DockerInspectState,
) -> impl IntoElement {
    InspectorSection::new(t!("App.RightPanel.Docker.inspect"))
        .icon(IconName::Inspector)
        .eyebrow(format!(
            "{} ({})",
            inspect.target_label,
            short_docker_id(&inspect.target_id)
        ))
        .child(
            div()
                .px(SP_3)
                .py(SP_2)
                .overflow_hidden()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_secondary)
                .child(trim_panel_text(&inspect.output, 12_000)),
        )
}

fn docker_status_label(status: &DockerStatus, has_snapshot: bool) -> SharedString {
    match status {
        DockerStatus::Idle => t!("App.Common.Status.idle").into(),
        DockerStatus::Loading => t!("App.Common.Status.loading").into(),
        DockerStatus::Ready => t!("App.Common.Status.live").into(),
        DockerStatus::Failed if has_snapshot => t!("App.Common.Status.stale").into(),
        DockerStatus::Failed => t!("App.Common.Status.error").into(),
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
    t!(
        "App.RightPanel.Docker.pending",
        action = docker_action_button_label(action.kind).as_ref(),
        target = action.target_label.as_str()
    )
    .into()
}

fn docker_action_button_label(kind: DockerActionKind) -> SharedString {
    match kind {
        DockerActionKind::Start => t!("App.Common.start").into(),
        DockerActionKind::Stop => t!("App.Common.stop").into(),
        DockerActionKind::Restart => t!("App.RightPanel.Docker.restart").into(),
        DockerActionKind::Inspect => t!("App.RightPanel.Docker.inspect").into(),
        DockerActionKind::Delete => t!("App.RightPanel.Docker.delete").into(),
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
    t!(
        "App.RightPanel.truncated_chars",
        text = trimmed,
        total = total
    )
    .into()
}

fn logs_preset_button(
    id: &'static str,
    label: &'static str,
    command: &'static str,
    on_action: LogsActionHandler,
) -> Button {
    let action = LogsAction::Preset {
        command: command.to_string(),
    };

    // Sm (22px) so four presets fit in one row on a typical ~380px
    // panel without the old wrapped 28px row eating a whole extra
    // vertical band of chrome.
    Button::secondary(id, label)
        .size(ButtonSize::Sm)
        .on_click(move |_, window, app| on_action(&action, window, app))
}

fn logs_status_label(status: &LogsStatus) -> SharedString {
    match status {
        LogsStatus::Idle => t!("App.Common.Status.idle").into(),
        LogsStatus::Starting => t!("App.Common.Status.starting").into(),
        LogsStatus::Live => t!("App.Common.Status.live").into(),
        LogsStatus::Stopped => t!("App.Common.Status.stopped").into(),
        LogsStatus::Failed => t!("App.Common.Status.error").into(),
    }
}

fn logs_status_kind(status: &LogsStatus) -> StatusKind {
    match status {
        LogsStatus::Idle => StatusKind::Warning,
        LogsStatus::Starting => StatusKind::Info,
        LogsStatus::Live => StatusKind::Success,
        LogsStatus::Stopped => StatusKind::Warning,
        LogsStatus::Failed => StatusKind::Error,
    }
}

fn logs_exit_label(exit_code: i32) -> SharedString {
    if exit_code < 0 {
        "exit unknown".into()
    } else {
        format!("exit {exit_code}").into()
    }
}

fn log_line_row(t: &crate::theme::Theme, line: LogLine) -> impl IntoElement {
    let (label, kind, color) = match line.kind {
        LogLineKind::Stdout => ("OUT", StatusKind::Info, t.color.text_secondary),
        LogLineKind::Stderr => ("ERR", StatusKind::Error, t.color.status_error),
        LogLineKind::Meta => ("META", StatusKind::Warning, t.color.text_tertiary),
    };

    // Tight IDE-density row: 2px top/bottom padding, 1px hairline
    // between every row. The OUT/ERR/META pill anchors the left gutter
    // so the eye can scan kinds quickly without needing color.
    div()
        .id(("logs-line", line.id))
        .flex()
        .flex_row()
        .items_start()
        .gap(SP_2)
        .px(SP_3)
        .py(SP_0_5)
        .border_t_1()
        .border_color(t.color.border_subtle)
        .overflow_hidden()
        .child(div().flex_none().child(StatusPill::new(label, kind)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(color)
                .child(trim_panel_text(&line.text, 2_000)),
        )
}

fn placeholder(
    title: impl Into<SharedString>,
    headline: impl Into<SharedString>,
    body: impl Into<SharedString>,
) -> impl IntoElement {
    let title: SharedString = title.into();
    let headline: SharedString = headline.into();
    let body: SharedString = body.into();
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
    let mut rail = div()
        .w_full()
        .flex()
        .flex_col()
        .items_center()
        .gap(SP_1)
        .px(SP_1)
        .py(SP_2)
        .rounded(RADIUS_MD)
        .bg(t.color.bg_panel)
        .border_1()
        .border_color(t.color.border_subtle);

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

    for mode in local_modes {
        rail = rail.child(mode_icon_button(
            t,
            mode,
            mode == active_mode,
            on_select.clone(),
        ));
    }
    if !remote_modes.is_empty() {
        rail = rail.child(
            div()
                .w(px(18.0))
                .h(px(1.0))
                .my(SP_1)
                .bg(t.color.border_subtle),
        );
        for mode in remote_modes {
            rail = rail.child(mode_icon_button(
                t,
                mode,
                mode == active_mode,
                on_select.clone(),
            ));
        }
    }
    div()
        .w(RIGHT_ICON_BAR_W)
        .h_full()
        .px(SP_1)
        .py(SP_2)
        .bg(t.color.bg_surface)
        .child(rail)
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
        .w(MODE_RAIL_BUTTON_W)
        .h(BUTTON_MD_H)
        .px(SP_0_5)
        .flex()
        .flex_row()
        .items_center()
        .rounded(RADIUS_MD)
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(move |_, w, app| on_select(&mode, w, app))
        .child(
            div()
                .w(MODE_RAIL_INDICATOR_W)
                .h(MODE_RAIL_INDICATOR_H)
                .rounded(RADIUS_XS)
                .bg(if is_active {
                    t.color.accent
                } else {
                    gpui::Rgba::default()
                }),
        )
        .child(div().flex_1().flex().items_center().justify_center().child(
            icon.size(TAB_GLYPH).text_color(if is_active {
                t.color.accent
            } else {
                t.color.text_secondary
            }),
        ))
        .child(div().w(MODE_RAIL_INDICATOR_W));

    if is_active {
        btn = btn.bg(t.color.accent_subtle);
    }
    btn
}

fn mode_icon(mode: RightMode) -> UiIcon {
    UiIcon::empty().path(mode.icon_asset().unwrap_or("icons/file.svg"))
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
