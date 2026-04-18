//! SSH session backing the right-panel SFTP browser (and, eventually, the
//! Docker / Logs / DB modes that need an `exec` channel into the same host).
//!
//! Each connection the user clicks in the Servers list spawns one of these.
//! The session is **lazy-connected**: nothing happens until the right panel
//! asks for a directory listing. The actual SSH handshake + SFTP listing runs
//! on a GPUI background executor; this state object only tracks the latest
//! request and applies results back on the UI thread.
//!
//! Auth coverage:
//!   - `AuthMethod::Agent`            — handled by russh
//!   - `AuthMethod::DirectPassword`   — handled by russh
//!   - `AuthMethod::KeychainPassword` — pier-core looks up the OS keychain
//!   - `AuthMethod::PublicKeyFile`    — pier-core reads file + opt. passphrase

use std::path::PathBuf;

use pier_core::services::docker::{
    inspect_container_blocking, list_containers_blocking, list_images_blocking,
    list_networks_blocking, list_volumes_blocking, restart_blocking, start_blocking, stop_blocking,
    Container as DockerContainer, DockerImage, DockerNetwork, DockerVolume,
};
use pier_core::services::server_monitor::ServerSnapshot;
use pier_core::ssh::{
    DetectedService, ExecEvent, ExecStream, HostKeyVerifier, SftpClient, SshConfig, SshSession,
    Tunnel, EXIT_UNKNOWN,
};

use crate::app::layout::{RightContext, RightMode};

/// Where the SFTP root lands on first connect — `~` is the SSH spec's
/// shorthand for "user's home directory" and SFTP servers honour it.
pub const DEFAULT_REMOTE_ROOT: &str = ".";

#[derive(Clone, Debug)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConnectStatus {
    #[default]
    Idle,
    Connecting,
    Refreshing,
    Connected,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ServiceProbeStatus {
    #[default]
    Idle,
    Probing,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MonitorStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DockerStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LogsStatus {
    #[default]
    Idle,
    Starting,
    Live,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockerActionKind {
    Start,
    Stop,
    Restart,
    Inspect,
}

impl DockerActionKind {
    pub fn label(self) -> &'static str {
        match self {
            DockerActionKind::Start => "start",
            DockerActionKind::Stop => "stop",
            DockerActionKind::Restart => "restart",
            DockerActionKind::Inspect => "inspect",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelStatus {
    Opening,
    Active,
    Failed,
}

pub struct ServiceTunnelState {
    pub service_name: String,
    pub remote_port: u16,
    pub local_port: Option<u16>,
    pub status: TunnelStatus,
    pub last_error: Option<String>,
    handle: Option<Tunnel>,
    nonce: usize,
}

#[derive(Clone, Debug, Default)]
pub struct DockerPanelSnapshot {
    pub containers: Vec<DockerContainer>,
    pub images: Vec<DockerImage>,
    pub volumes: Vec<DockerVolume>,
    pub networks: Vec<DockerNetwork>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingDockerAction {
    pub kind: DockerActionKind,
    pub target_id: String,
    pub target_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerInspectState {
    pub target_id: String,
    pub target_label: String,
    pub output: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLineKind {
    Stdout,
    Stderr,
    Meta,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub kind: LogLineKind,
    pub text: String,
}

pub struct SshSessionState {
    pub config: SshConfig,
    /// Held to keep the underlying russh handle alive across SFTP ops.
    /// Boxed inside an Option so first-connect can populate it without
    /// re-allocating `self`.
    session: Option<SshSession>,
    sftp: Option<SftpClient>,
    pub cwd: PathBuf,
    pub entries: Vec<RemoteEntry>,
    pub status: ConnectStatus,
    pub last_error: Option<String>,
    pub services: Vec<DetectedService>,
    pub service_probe_status: ServiceProbeStatus,
    pub service_probe_error: Option<String>,
    pub tunnels: Vec<ServiceTunnelState>,
    pub monitor_snapshot: Option<ServerSnapshot>,
    pub monitor_status: MonitorStatus,
    pub monitor_error: Option<String>,
    pub docker_snapshot: Option<DockerPanelSnapshot>,
    pub docker_status: DockerStatus,
    pub docker_error: Option<String>,
    pub docker_inspect: Option<DockerInspectState>,
    pub docker_pending_action: Option<PendingDockerAction>,
    pub docker_action_error: Option<String>,
    pub logs_lines: Vec<LogLine>,
    pub logs_status: LogsStatus,
    pub logs_error: Option<String>,
    pub logs_command: Option<String>,
    pub logs_exit_code: Option<i32>,
    refresh_nonce: usize,
    bootstrap_nonce: usize,
    monitor_refresh_nonce: usize,
    docker_refresh_nonce: usize,
    docker_action_nonce: usize,
    logs_start_nonce: usize,
    /// Bumped per [`Self::begin_sftp_mutation`] so an in-flight
    /// rename / delete / upload / download / mkdir result that
    /// arrives after a newer mutation gets dropped on the floor.
    sftp_mutation_nonce: usize,
    logs_stream: Option<ExecStream>,
}

impl SshSessionState {
    pub fn new(config: SshConfig) -> Self {
        Self {
            config,
            session: None,
            sftp: None,
            cwd: PathBuf::from(DEFAULT_REMOTE_ROOT),
            entries: Vec::new(),
            status: ConnectStatus::Idle,
            last_error: None,
            services: Vec::new(),
            service_probe_status: ServiceProbeStatus::Idle,
            service_probe_error: None,
            tunnels: Vec::new(),
            monitor_snapshot: None,
            monitor_status: MonitorStatus::Idle,
            monitor_error: None,
            docker_snapshot: None,
            docker_status: DockerStatus::Idle,
            docker_error: None,
            docker_inspect: None,
            docker_pending_action: None,
            docker_action_error: None,
            logs_lines: Vec::new(),
            logs_status: LogsStatus::Idle,
            logs_error: None,
            logs_command: None,
            logs_exit_code: None,
            refresh_nonce: 0,
            bootstrap_nonce: 0,
            monitor_refresh_nonce: 0,
            docker_refresh_nonce: 0,
            docker_action_nonce: 0,
            logs_start_nonce: 0,
            sftp_mutation_nonce: 0,
            logs_stream: None,
        }
    }

    pub fn begin_refresh(&mut self, next_cwd: PathBuf) -> RefreshRequest {
        self.cwd = next_cwd;
        self.refresh_nonce = self.refresh_nonce.wrapping_add(1);
        self.status = if self.session.is_some() {
            ConnectStatus::Refreshing
        } else {
            ConnectStatus::Connecting
        };
        self.last_error = None;

        RefreshRequest {
            nonce: self.refresh_nonce,
            config: self.config.clone(),
            cwd: self.cwd.clone(),
            session: self.session.clone(),
            sftp: self.sftp.clone(),
        }
    }

    pub fn begin_bootstrap(&mut self) -> BootstrapRequest {
        self.bootstrap_nonce = self.bootstrap_nonce.wrapping_add(1);
        if self.session.is_none() {
            self.status = ConnectStatus::Connecting;
        }
        self.last_error = None;
        self.service_probe_status = ServiceProbeStatus::Probing;
        self.service_probe_error = None;

        BootstrapRequest {
            nonce: self.bootstrap_nonce,
            config: self.config.clone(),
            session: self.session.clone(),
        }
    }

    pub fn begin_monitor_refresh(&mut self) -> Option<MonitorRequest> {
        if matches!(self.monitor_status, MonitorStatus::Loading) {
            return None;
        }
        let session = self.session.clone()?;

        self.monitor_refresh_nonce = self.monitor_refresh_nonce.wrapping_add(1);
        self.monitor_status = MonitorStatus::Loading;
        self.monitor_error = None;

        Some(MonitorRequest {
            nonce: self.monitor_refresh_nonce,
            session,
        })
    }

    pub fn begin_docker_refresh(&mut self) -> Option<DockerRefreshRequest> {
        if matches!(self.docker_status, DockerStatus::Loading)
            || self.docker_pending_action.is_some()
        {
            return None;
        }
        let session = self.session.clone()?;

        self.docker_refresh_nonce = self.docker_refresh_nonce.wrapping_add(1);
        self.docker_status = DockerStatus::Loading;
        self.docker_error = None;

        Some(DockerRefreshRequest {
            nonce: self.docker_refresh_nonce,
            session,
        })
    }

    pub fn begin_docker_action(
        &mut self,
        kind: DockerActionKind,
        target_id: &str,
        target_label: &str,
    ) -> Option<DockerCommandRequest> {
        if self.docker_pending_action.is_some() {
            return None;
        }
        let session = self.session.clone()?;

        self.docker_action_nonce = self.docker_action_nonce.wrapping_add(1);
        self.docker_action_error = None;
        self.docker_pending_action = Some(PendingDockerAction {
            kind,
            target_id: target_id.to_string(),
            target_label: target_label.to_string(),
        });

        Some(DockerCommandRequest {
            nonce: self.docker_action_nonce,
            session,
            action: DockerCommand {
                kind,
                target_id: target_id.to_string(),
                target_label: target_label.to_string(),
            },
        })
    }

    pub fn begin_logs_start(&mut self, command: String) -> Option<LogsStartRequest> {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return None;
        }
        let session = self.session.clone()?;

        self.logs_start_nonce = self.logs_start_nonce.wrapping_add(1);
        self.logs_status = LogsStatus::Starting;
        self.logs_error = None;
        self.logs_exit_code = None;
        self.logs_command = Some(trimmed.to_string());
        self.logs_lines.clear();
        self.logs_stream = None;

        Some(LogsStartRequest {
            nonce: self.logs_start_nonce,
            session,
            command: trimmed.to_string(),
        })
    }

    pub fn apply_refresh_result(&mut self, result: RefreshResult) -> bool {
        if result.nonce != self.refresh_nonce {
            return false;
        }

        self.cwd = result.cwd;
        self.session = result.session;
        self.sftp = result.sftp;

        match result.outcome {
            Ok(entries) => {
                self.entries = entries;
                self.status = ConnectStatus::Connected;
                self.last_error = None;
            }
            Err(err) => {
                self.status = ConnectStatus::Failed;
                self.last_error = Some(err);
                self.entries.clear();
            }
        }

        true
    }

    /// Mint a request to apply one of the SFTP mutations
    /// (mkdir / rename / delete / upload / download). Returns
    /// `None` when no SFTP channel is open — callers should
    /// schedule a refresh first.
    #[allow(dead_code)] // Wired by `PierApp::schedule_sftp_mutation` in commit 2.
    pub fn begin_sftp_mutation(&mut self, kind: SftpMutationKind) -> Option<SftpMutationRequest> {
        let sftp = self.sftp.clone()?;
        self.sftp_mutation_nonce = self.sftp_mutation_nonce.wrapping_add(1);
        Some(SftpMutationRequest {
            nonce: self.sftp_mutation_nonce,
            sftp,
            kind,
        })
    }

    /// Apply a [`SftpMutationResult`]. Stale nonces are dropped
    /// (return `false`); on failure the verb-prefixed error is
    /// stored in `last_error` so the existing error card surfaces
    /// it. Returns `true` when the result was applied AND the
    /// underlying mutation succeeded — callers use that signal to
    /// schedule a follow-up refresh.
    #[allow(dead_code)] // Wired by `PierApp::schedule_sftp_mutation` in commit 2.
    pub fn apply_sftp_mutation_result(&mut self, result: SftpMutationResult) -> bool {
        if result.nonce != self.sftp_mutation_nonce {
            log::debug!(
                "ssh_session: dropping stale sftp mutation result (got {}, want {})",
                result.nonce,
                self.sftp_mutation_nonce
            );
            return false;
        }
        match result.outcome {
            Ok(()) => {
                self.last_error = None;
                true
            }
            Err(err) => {
                self.last_error = Some(format!("{}: {err}", result.verb));
                false
            }
        }
    }

    pub fn next_parent_target(&self) -> Option<PathBuf> {
        if let Some(parent) = self.cwd.parent() {
            let parent_buf = parent.to_path_buf();
            if !parent_buf.as_os_str().is_empty() {
                Some(parent_buf)
            } else {
                Some(PathBuf::from("/"))
            }
        } else {
            None
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(
            self.status,
            ConnectStatus::Connecting | ConnectStatus::Refreshing
        )
    }

    pub fn should_bootstrap(&self) -> bool {
        self.entries.is_empty() && self.last_error.is_none() && !self.is_loading()
    }

    pub fn apply_bootstrap_result(&mut self, result: BootstrapResult) -> bool {
        if result.nonce != self.bootstrap_nonce {
            return false;
        }

        self.session = result.session;
        match result.outcome {
            Ok(services) => {
                self.status = ConnectStatus::Connected;
                self.last_error = None;
                self.services = services;
                self.service_probe_status = ServiceProbeStatus::Ready;
                self.service_probe_error = None;
                self.tunnels.retain(|tunnel| {
                    self.services
                        .iter()
                        .any(|service| service.name == tunnel.service_name)
                });
            }
            Err(err) => {
                self.status = ConnectStatus::Failed;
                self.last_error = Some(err.clone());
                self.services.clear();
                self.service_probe_status = ServiceProbeStatus::Failed;
                self.service_probe_error = Some(err);
                self.tunnels.clear();
            }
        }

        true
    }

    pub fn apply_monitor_result(&mut self, result: MonitorResult) -> bool {
        if result.nonce != self.monitor_refresh_nonce {
            return false;
        }

        match result.outcome {
            Ok(snapshot) => {
                self.monitor_snapshot = Some(snapshot);
                self.monitor_status = MonitorStatus::Ready;
                self.monitor_error = None;
            }
            Err(err) => {
                self.monitor_status = MonitorStatus::Failed;
                self.monitor_error = Some(err);
            }
        }

        true
    }

    pub fn apply_docker_refresh_result(&mut self, result: DockerRefreshResult) -> bool {
        if result.nonce != self.docker_refresh_nonce {
            return false;
        }

        match result.outcome {
            Ok(snapshot) => {
                self.docker_snapshot = Some(snapshot);
                self.docker_status = DockerStatus::Ready;
                self.docker_error = None;
            }
            Err(err) => {
                self.docker_status = DockerStatus::Failed;
                self.docker_error = Some(err);
            }
        }

        true
    }

    pub fn apply_docker_command_result(&mut self, result: DockerCommandResult) -> bool {
        if result.nonce != self.docker_action_nonce {
            return false;
        }

        self.docker_pending_action = None;
        match result.outcome {
            Ok(output) => {
                self.docker_action_error = None;
                match output {
                    DockerCommandOutput::Snapshot(snapshot) => {
                        self.docker_snapshot = Some(snapshot);
                        self.docker_status = DockerStatus::Ready;
                        self.docker_error = None;
                    }
                    DockerCommandOutput::Inspect(inspect) => {
                        self.docker_inspect = Some(inspect);
                    }
                }
            }
            Err(err) => {
                self.docker_action_error = Some(format!(
                    "{} {} failed: {err}",
                    result.action.kind.label(),
                    result.action.target_label
                ));
            }
        }

        true
    }

    pub fn apply_logs_start_result(&mut self, result: LogsStartResult) -> bool {
        if result.nonce != self.logs_start_nonce {
            return false;
        }

        self.logs_command = Some(result.command.clone());
        match result.outcome {
            Ok(stream) => {
                self.logs_stream = Some(stream);
                self.logs_status = LogsStatus::Live;
                self.logs_error = None;
                self.logs_exit_code = None;
                self.push_log_line(
                    LogLineKind::Meta,
                    format!("stream started: {}", result.command),
                );
            }
            Err(err) => {
                self.logs_stream = None;
                self.logs_status = LogsStatus::Failed;
                self.logs_error = Some(err.clone());
                self.push_log_line(LogLineKind::Meta, format!("stream failed to start: {err}"));
            }
        }

        true
    }

    pub fn drain_logs_stream(&mut self) -> bool {
        let Some(stream) = self.logs_stream.as_ref() else {
            return false;
        };
        let events = stream.drain();
        let alive_after = stream.is_alive();

        if events.is_empty() && alive_after {
            return false;
        }

        let mut changed = false;
        let mut should_drop_stream = false;
        for event in events {
            changed = true;
            match event {
                ExecEvent::Stdout(line) => self.push_log_line(LogLineKind::Stdout, line),
                ExecEvent::Stderr(line) => self.push_log_line(LogLineKind::Stderr, line),
                ExecEvent::Exit(code) => {
                    self.logs_exit_code = Some(code);
                    self.logs_status = LogsStatus::Stopped;
                    should_drop_stream = true;
                    self.push_log_line(LogLineKind::Meta, logs_exit_summary(code));
                }
                ExecEvent::Error(err) => {
                    self.logs_error = Some(err.clone());
                    self.logs_status = LogsStatus::Failed;
                    should_drop_stream = true;
                    self.push_log_line(LogLineKind::Meta, format!("stream error: {err}"));
                }
            }
        }

        if !alive_after {
            should_drop_stream = true;
            if matches!(self.logs_status, LogsStatus::Live | LogsStatus::Starting) {
                self.logs_status = LogsStatus::Stopped;
                if self.logs_exit_code.is_none() {
                    self.logs_exit_code = Some(EXIT_UNKNOWN);
                }
            }
        }

        if should_drop_stream {
            self.logs_stream = None;
            changed = true;
        }

        changed
    }

    pub fn stop_logs(&mut self) -> bool {
        let Some(stream) = self.logs_stream.take() else {
            return false;
        };

        stream.stop();
        self.logs_status = LogsStatus::Stopped;
        self.logs_exit_code = None;
        self.logs_error = None;
        self.push_log_line(LogLineKind::Meta, "stream stopped".to_string());
        true
    }

    pub fn clear_logs(&mut self) -> bool {
        let had_any = !self.logs_lines.is_empty()
            || self.logs_error.is_some()
            || self.logs_exit_code.is_some();
        self.logs_lines.clear();
        self.logs_error = None;
        self.logs_exit_code = None;
        had_any
    }

    pub fn should_autostart_logs(&self) -> bool {
        matches!(self.logs_status, LogsStatus::Idle)
            && self.logs_lines.is_empty()
            && self.logs_stream.is_none()
    }

    pub fn available_modes(&self) -> Vec<RightMode> {
        RightMode::ALL
            .into_iter()
            .filter(|mode| self.supports_mode(*mode))
            .collect()
    }

    pub fn supports_mode(&self, mode: RightMode) -> bool {
        match mode.context() {
            RightContext::Local => true,
            RightContext::Remote => mode
                .required_service_name()
                .map(|service_name| self.has_detected_service(service_name))
                .unwrap_or(true),
        }
    }

    pub fn has_detected_service(&self, service_name: &str) -> bool {
        self.services
            .iter()
            .any(|service| service.name == service_name)
    }

    pub fn detected_service(&self, service_name: &str) -> Option<&DetectedService> {
        self.services
            .iter()
            .find(|service| service.name == service_name)
    }

    pub fn begin_tunnel(&mut self, service_name: &str) -> Option<TunnelRequest> {
        let session = self.session.clone()?;
        let (service_name_owned, remote_port) = {
            let service = self.detected_service(service_name)?;
            (service.name.clone(), service.port)
        };
        if remote_port == 0 {
            return None;
        }

        let tunnel_idx = if let Some(idx) = self
            .tunnels
            .iter()
            .position(|tunnel| tunnel.service_name == service_name_owned)
        {
            idx
        } else {
            self.tunnels.push(ServiceTunnelState {
                service_name: service_name_owned,
                remote_port,
                local_port: None,
                status: TunnelStatus::Opening,
                last_error: None,
                handle: None,
                nonce: 0,
            });
            self.tunnels.len() - 1
        };

        let tunnel = self.tunnels.get_mut(tunnel_idx)?;
        if tunnel.handle.as_ref().is_some_and(Tunnel::is_alive)
            && matches!(tunnel.status, TunnelStatus::Active)
        {
            return None;
        }
        if matches!(tunnel.status, TunnelStatus::Opening) {
            return None;
        }

        tunnel.handle = None;
        tunnel.local_port = None;
        tunnel.last_error = None;
        tunnel.remote_port = remote_port;
        tunnel.nonce = tunnel.nonce.wrapping_add(1);
        tunnel.status = TunnelStatus::Opening;

        Some(TunnelRequest {
            nonce: tunnel.nonce,
            session,
            service_name: tunnel.service_name.clone(),
            remote_port: tunnel.remote_port,
            preferred_local_port: preferred_local_port(tunnel.remote_port),
        })
    }

    pub fn apply_tunnel_result(&mut self, result: TunnelResult) -> bool {
        let Some(tunnel) = self
            .tunnels
            .iter_mut()
            .find(|tunnel| tunnel.service_name == result.service_name)
        else {
            return false;
        };
        if tunnel.nonce != result.nonce {
            return false;
        }

        match result.outcome {
            Ok(handle) => {
                tunnel.local_port = Some(handle.local_port());
                tunnel.status = TunnelStatus::Active;
                tunnel.last_error = None;
                tunnel.handle = Some(handle);
            }
            Err(err) => {
                tunnel.local_port = None;
                tunnel.status = TunnelStatus::Failed;
                tunnel.last_error = Some(err);
                tunnel.handle = None;
            }
        }

        true
    }
}

#[derive(Clone)]
pub struct RefreshRequest {
    nonce: usize,
    config: SshConfig,
    cwd: PathBuf,
    session: Option<SshSession>,
    sftp: Option<SftpClient>,
}

#[derive(Clone)]
pub struct BootstrapRequest {
    nonce: usize,
    config: SshConfig,
    session: Option<SshSession>,
}

pub struct RefreshResult {
    nonce: usize,
    cwd: PathBuf,
    session: Option<SshSession>,
    sftp: Option<SftpClient>,
    outcome: Result<Vec<RemoteEntry>, String>,
}

pub struct BootstrapResult {
    nonce: usize,
    session: Option<SshSession>,
    outcome: Result<Vec<DetectedService>, String>,
}

pub struct MonitorRequest {
    nonce: usize,
    session: SshSession,
}

pub struct MonitorResult {
    nonce: usize,
    outcome: Result<ServerSnapshot, String>,
}

pub struct DockerRefreshRequest {
    nonce: usize,
    session: SshSession,
}

pub struct DockerRefreshResult {
    nonce: usize,
    outcome: Result<DockerPanelSnapshot, String>,
}

pub struct DockerCommandRequest {
    nonce: usize,
    session: SshSession,
    action: DockerCommand,
}

pub struct DockerCommandResult {
    nonce: usize,
    action: DockerCommand,
    outcome: Result<DockerCommandOutput, String>,
}

pub struct LogsStartRequest {
    nonce: usize,
    session: SshSession,
    command: String,
}

pub struct LogsStartResult {
    nonce: usize,
    command: String,
    outcome: Result<ExecStream, String>,
}

pub struct TunnelRequest {
    nonce: usize,
    session: SshSession,
    service_name: String,
    remote_port: u16,
    preferred_local_port: u16,
}

pub struct TunnelResult {
    nonce: usize,
    service_name: String,
    outcome: Result<Tunnel, String>,
}

#[derive(Clone, Debug)]
struct DockerCommand {
    kind: DockerActionKind,
    target_id: String,
    target_label: String,
}

#[derive(Clone, Debug)]
enum DockerCommandOutput {
    Snapshot(DockerPanelSnapshot),
    Inspect(DockerInspectState),
}

// ─── SFTP file-operation mutations (P1-5) ────────────────────────────
//
// One enum + one run_*  fn  + one Request/Result pair instead of 5×
// (one per action). Keeps the schedule_/begin_/apply_ machinery to a
// single shape and makes it cheap to add future actions (e.g. chmod).

/// Which SFTP mutation to apply. Each variant carries the full set of
/// owned strings / paths the worker thread needs — must be `Send`.
///
/// Variants are referenced through dispatcher methods on PierApp in
/// commit 2 of the series; until then the enum is unused outside its
/// own `match` arms inside `run_sftp_mutation`.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum SftpMutationKind {
    /// `mkdir(path)`. Wraps [`SftpClient::create_dir_blocking`].
    Mkdir { path: String },
    /// `mv from -> to`. Wraps [`SftpClient::rename_blocking`].
    Rename { from: String, to: String },
    /// `rm path`. Wraps [`SftpClient::remove_file_blocking`].
    DeleteFile { path: String },
    /// `rmdir path` — server-enforced empty-only.
    /// Wraps [`SftpClient::remove_dir_blocking`].
    DeleteDir { path: String },
    /// Push local → remote. Wraps [`SftpClient::upload_from_blocking`].
    Upload { local: PathBuf, remote: String },
    /// Pull remote → local. Wraps [`SftpClient::download_to_blocking`].
    Download { remote: String, local: PathBuf },
}

impl SftpMutationKind {
    /// Short verb the UI prefixes onto the error message
    /// (`"rename: permission denied"`). Stable English string —
    /// commit 5 of this series swaps it for an i18n lookup.
    pub fn verb(&self) -> &'static str {
        match self {
            Self::Mkdir { .. } => "mkdir",
            Self::Rename { .. } => "rename",
            Self::DeleteFile { .. } | Self::DeleteDir { .. } => "delete",
            Self::Upload { .. } => "upload",
            Self::Download { .. } => "download",
        }
    }
}

/// Request payload for [`run_sftp_mutation`]. `sftp` is cloned out
/// of the live `SshSessionState` so the worker has its own handle.
pub struct SftpMutationRequest {
    pub(crate) nonce: usize,
    pub(crate) sftp: SftpClient,
    pub(crate) kind: SftpMutationKind,
}

/// Result of [`run_sftp_mutation`]. `verb` is echoed back so
/// `apply_sftp_mutation_result` can format the error consistently
/// without re-matching on `kind`.
pub struct SftpMutationResult {
    pub(crate) nonce: usize,
    pub(crate) verb: &'static str,
    pub(crate) outcome: Result<(), String>,
}

/// Background worker — runs the matching `_blocking` SFTP call and
/// stringifies any error. Designed to be invoked from
/// `cx.background_executor().spawn(async move { run_sftp_mutation(req) })`
/// so the UI thread never blocks on the SSH round-trip.
#[allow(dead_code)] // Wired by `PierApp::schedule_sftp_mutation` in commit 2.
pub fn run_sftp_mutation(request: SftpMutationRequest) -> SftpMutationResult {
    let verb = request.kind.verb();
    let outcome = match &request.kind {
        SftpMutationKind::Mkdir { path } => request
            .sftp
            .create_dir_blocking(path)
            .map_err(|e| e.to_string()),
        SftpMutationKind::Rename { from, to } => request
            .sftp
            .rename_blocking(from, to)
            .map_err(|e| e.to_string()),
        SftpMutationKind::DeleteFile { path } => request
            .sftp
            .remove_file_blocking(path)
            .map_err(|e| e.to_string()),
        SftpMutationKind::DeleteDir { path } => request
            .sftp
            .remove_dir_blocking(path)
            .map_err(|e| e.to_string()),
        SftpMutationKind::Upload { local, remote } => request
            .sftp
            .upload_from_blocking(local, remote)
            .map_err(|e| e.to_string()),
        SftpMutationKind::Download { remote, local } => request
            .sftp
            .download_to_blocking(remote, local)
            .map_err(|e| e.to_string()),
    };
    SftpMutationResult {
        nonce: request.nonce,
        verb,
        outcome,
    }
}

pub fn run_refresh(request: RefreshRequest) -> RefreshResult {
    let mut session = request.session;
    let mut sftp = request.sftp;
    let path_str = request
        .cwd
        .to_str()
        .map(str::to_string)
        .unwrap_or_else(|| DEFAULT_REMOTE_ROOT.to_string());

    let outcome = (|| -> Result<Vec<RemoteEntry>, String> {
        if session.is_none() {
            let verifier = HostKeyVerifier::default();
            session = Some(
                SshSession::connect_blocking(&request.config, verifier)
                    .map_err(|e| e.to_string())?,
            );
        }
        if sftp.is_none() {
            let live_session = session
                .as_ref()
                .ok_or_else(|| "SSH session unavailable after connect".to_string())?;
            sftp = Some(
                live_session
                    .open_sftp_blocking()
                    .map_err(|e| e.to_string())?,
            );
        }

        let remote_entries = sftp
            .as_ref()
            .ok_or_else(|| "SFTP session unavailable after connect".to_string())?
            .list_dir_blocking(&path_str)
            .map_err(|err| format!("list_dir({path_str}): {err}"))?;

        let mut entries = remote_entries
            .into_iter()
            .map(|e| RemoteEntry {
                name: e.name,
                path: e.path,
                is_dir: e.is_dir,
                is_link: e.is_link,
                size: e.size,
            })
            .filter(|e| !e.name.starts_with('.'))
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        Ok(entries)
    })();

    RefreshResult {
        nonce: request.nonce,
        cwd: request.cwd,
        session,
        sftp,
        outcome,
    }
}

pub fn run_bootstrap(request: BootstrapRequest) -> BootstrapResult {
    let mut session = request.session;
    let outcome = (|| -> Result<Vec<DetectedService>, String> {
        if session.is_none() {
            let verifier = HostKeyVerifier::default();
            session = Some(
                SshSession::connect_blocking(&request.config, verifier)
                    .map_err(|e| e.to_string())?,
            );
        }

        let live_session = session
            .as_ref()
            .ok_or_else(|| "SSH session unavailable after connect".to_string())?;
        Ok(pier_core::ssh::detect_all_blocking(live_session))
    })();

    BootstrapResult {
        nonce: request.nonce,
        session,
        outcome,
    }
}

pub fn run_monitor_refresh(request: MonitorRequest) -> MonitorResult {
    let outcome = pier_core::services::server_monitor::probe_blocking(&request.session)
        .map_err(|err| err.to_string());

    MonitorResult {
        nonce: request.nonce,
        outcome,
    }
}

pub fn run_docker_refresh(request: DockerRefreshRequest) -> DockerRefreshResult {
    let outcome = collect_docker_snapshot(&request.session).map_err(|err| err.to_string());

    DockerRefreshResult {
        nonce: request.nonce,
        outcome,
    }
}

pub fn run_docker_command(request: DockerCommandRequest) -> DockerCommandResult {
    let outcome = match request.action.kind {
        DockerActionKind::Start => start_blocking(&request.session, &request.action.target_id)
            .and_then(|_| collect_docker_snapshot(&request.session))
            .map(DockerCommandOutput::Snapshot),
        DockerActionKind::Stop => stop_blocking(&request.session, &request.action.target_id)
            .and_then(|_| collect_docker_snapshot(&request.session))
            .map(DockerCommandOutput::Snapshot),
        DockerActionKind::Restart => restart_blocking(&request.session, &request.action.target_id)
            .and_then(|_| collect_docker_snapshot(&request.session))
            .map(DockerCommandOutput::Snapshot),
        DockerActionKind::Inspect => {
            inspect_container_blocking(&request.session, &request.action.target_id).map(|output| {
                DockerCommandOutput::Inspect(DockerInspectState {
                    target_id: request.action.target_id.clone(),
                    target_label: request.action.target_label.clone(),
                    output,
                })
            })
        }
    }
    .map_err(|err| err.to_string());

    DockerCommandResult {
        nonce: request.nonce,
        action: request.action,
        outcome,
    }
}

pub fn run_logs_start(request: LogsStartRequest) -> LogsStartResult {
    let outcome = request
        .session
        .spawn_exec_stream_blocking(&request.command)
        .map_err(|err| err.to_string());

    LogsStartResult {
        nonce: request.nonce,
        command: request.command,
        outcome,
    }
}

pub fn run_tunnel(request: TunnelRequest) -> TunnelResult {
    let outcome = request
        .session
        .open_local_forward_blocking(
            request.preferred_local_port,
            "127.0.0.1",
            request.remote_port,
        )
        .or_else(|_| {
            request
                .session
                .open_local_forward_blocking(0, "127.0.0.1", request.remote_port)
        })
        .map_err(|err| err.to_string());

    TunnelResult {
        nonce: request.nonce,
        service_name: request.service_name,
        outcome,
    }
}

fn preferred_local_port(remote_port: u16) -> u16 {
    remote_port.saturating_add(10_000)
}

fn collect_docker_snapshot(
    session: &SshSession,
) -> pier_core::ssh::error::Result<DockerPanelSnapshot> {
    Ok(DockerPanelSnapshot {
        containers: list_containers_blocking(session, true)?,
        images: list_images_blocking(session)?,
        volumes: list_volumes_blocking(session)?,
        networks: list_networks_blocking(session)?,
    })
}

fn logs_exit_summary(code: i32) -> String {
    if code == EXIT_UNKNOWN {
        "stream closed without exit status".to_string()
    } else {
        format!("stream exited with code {code}")
    }
}

impl SshSessionState {
    fn push_log_line(&mut self, kind: LogLineKind, text: String) {
        const MAX_LOG_LINES: usize = 800;

        self.logs_lines.push(LogLine { kind, text });
        if self.logs_lines.len() > MAX_LOG_LINES {
            let overflow = self.logs_lines.len() - MAX_LOG_LINES;
            self.logs_lines.drain(..overflow);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pier_core::ssh::{AuthMethod, DetectedService, ServiceStatus, SshConfig};

    use super::{
        ConnectStatus, DockerActionKind, DockerCommand, DockerCommandOutput, DockerCommandResult,
        DockerPanelSnapshot, DockerRefreshResult, DockerStatus, LogsStartResult, LogsStatus,
        MonitorResult, MonitorStatus, PendingDockerAction, RefreshResult, ServiceProbeStatus,
        SshSessionState,
    };
    use crate::app::layout::RightMode;

    fn sample_config() -> SshConfig {
        SshConfig {
            name: "demo".into(),
            host: "example.com".into(),
            port: 22,
            user: "pier".into(),
            auth: AuthMethod::Agent,
            tags: Vec::new(),
            connect_timeout_secs: 5,
        }
    }

    #[test]
    fn stale_refresh_results_are_ignored() {
        let mut state = SshSessionState::new(sample_config());
        let _old = state.begin_refresh(PathBuf::from("/first"));
        let _new = state.begin_refresh(PathBuf::from("/second"));

        let applied = state.apply_refresh_result(RefreshResult {
            nonce: 1,
            cwd: PathBuf::from("/first"),
            session: None,
            sftp: None,
            outcome: Ok(Vec::new()),
        });

        assert!(!applied);
        assert!(matches!(state.status, ConnectStatus::Connecting));
        assert_eq!(state.cwd, PathBuf::from("/second"));
    }

    #[test]
    fn available_modes_hide_service_panels_until_detected() {
        let mut state = SshSessionState::new(sample_config());
        state.service_probe_status = ServiceProbeStatus::Ready;
        state.services = vec![DetectedService {
            name: "redis".into(),
            version: "7.2".into(),
            status: ServiceStatus::Running,
            port: 6379,
        }];

        let available = state.available_modes();
        assert!(available.contains(&RightMode::Markdown));
        assert!(available.contains(&RightMode::Monitor));
        assert!(available.contains(&RightMode::Redis));
        assert!(!available.contains(&RightMode::Mysql));
        assert!(!available.contains(&RightMode::Docker));
        assert!(!available.contains(&RightMode::Sqlite));
    }

    #[test]
    fn stale_monitor_results_are_ignored() {
        let mut state = SshSessionState::new(sample_config());
        state.monitor_status = MonitorStatus::Ready;
        state.monitor_refresh_nonce = 2;

        let applied = state.apply_monitor_result(MonitorResult {
            nonce: 1,
            outcome: Ok(Default::default()),
        });

        assert!(!applied);
        assert_eq!(state.monitor_status, MonitorStatus::Ready);
    }

    #[test]
    fn stale_docker_refresh_results_are_ignored() {
        let mut state = SshSessionState::new(sample_config());
        state.docker_status = DockerStatus::Ready;
        state.docker_refresh_nonce = 2;

        let applied = state.apply_docker_refresh_result(DockerRefreshResult {
            nonce: 1,
            outcome: Ok(DockerPanelSnapshot::default()),
        });

        assert!(!applied);
        assert_eq!(state.docker_status, DockerStatus::Ready);
    }

    #[test]
    fn docker_command_result_clears_pending_action() {
        let mut state = SshSessionState::new(sample_config());
        state.docker_action_nonce = 1;
        state.docker_pending_action = Some(PendingDockerAction {
            kind: DockerActionKind::Start,
            target_id: "abc123".into(),
            target_label: "web".into(),
        });

        let applied = state.apply_docker_command_result(DockerCommandResult {
            nonce: 1,
            action: DockerCommand {
                kind: DockerActionKind::Start,
                target_id: "abc123".into(),
                target_label: "web".into(),
            },
            outcome: Ok(DockerCommandOutput::Snapshot(DockerPanelSnapshot::default())),
        });

        assert!(applied);
        assert!(state.docker_pending_action.is_none());
        assert!(matches!(state.docker_status, DockerStatus::Ready));
        assert!(state.docker_snapshot.is_some());
    }

    #[test]
    fn stale_logs_start_results_are_ignored() {
        let mut state = SshSessionState::new(sample_config());
        state.logs_status = LogsStatus::Starting;
        state.logs_start_nonce = 2;

        let applied = state.apply_logs_start_result(LogsStartResult {
            nonce: 1,
            command: "journalctl -f -n 200 --no-pager".into(),
            outcome: Err("boom".into()),
        });

        assert!(!applied);
        assert_eq!(state.logs_status, LogsStatus::Starting);
    }

    #[test]
    fn clear_logs_resets_errors_and_exit_code() {
        let mut state = SshSessionState::new(sample_config());
        state.logs_status = LogsStatus::Stopped;
        state.logs_lines.push(super::LogLine {
            kind: super::LogLineKind::Meta,
            text: "stream stopped".into(),
        });
        state.logs_error = Some("boom".into());
        state.logs_exit_code = Some(1);

        assert!(state.clear_logs());
        assert!(state.logs_lines.is_empty());
        assert!(state.logs_error.is_none());
        assert!(state.logs_exit_code.is_none());
    }
}
