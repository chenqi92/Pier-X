//! Generic remote-package install / update / probe over SSH.
//!
//! Replaces the per-service "is this binary installed?" + "auto-install
//! it" patterns that were starting to duplicate across `sqlite_remote`
//! and the upcoming Software panel. Centralises:
//!
//!   * `/etc/os-release` parsing and distro-id → package-manager mapping
//!   * `command -v <bin> && <bin> --version` style presence/version probe
//!   * `systemctl is-active <unit>` lookup
//!   * `apt-get install -y` / `dnf install -y` / ... command synthesis
//!     with a `sudo -n ` prefix when the session isn't already root
//!   * Streaming stdout+stderr through a per-line callback so the UI
//!     can render progress live instead of waiting for a 30s blob
//!
//! Adding a new piece of software is data-only: append a
//! `PackageDescriptor` to the registry. The execution path here doesn't
//! special-case any single tool.

use serde::{Deserialize, Serialize};

use crate::ssh::error::{Result, SshError};
use crate::ssh::SshSession;

// ── Types ───────────────────────────────────────────────────────────

/// One row in the registry — describes how to detect and install one
/// piece of software across the package managers we support.
#[derive(Debug, Clone)]
pub struct PackageDescriptor {
    /// Stable identifier exposed to the frontend (e.g. `"sqlite3"`).
    pub id: &'static str,
    /// Human label shown in the UI (e.g. `"SQLite"`).
    pub display_name: &'static str,
    /// Shell command that prints the version of the installed binary.
    /// Convention: `command -v <bin> >/dev/null 2>&1 && <bin> --version`.
    /// Exit 0 + non-empty stdout → installed; anything else → missing.
    pub probe_command: &'static str,
    /// Per package-manager package list. The first matching entry wins;
    /// distro_id falls through to `ID_LIKE` if it's not directly listed.
    pub install_packages: &'static [(PackageManager, &'static [&'static str])],
    /// Optional systemd unit name(s) per distro family. `None` means
    /// this software has no service to enable. The keys mirror
    /// `PackageManager` because service naming is package-manager-
    /// adjacent (`redis-server` on debian, `redis` on rhel/fedora).
    pub service_units: &'static [(PackageManager, &'static str)],
    /// Filesystem directories that count as "user data" for this
    /// package. Surfaced to the uninstall dialog behind a red
    /// checkbox + name-typed confirmation so docker images / postgres
    /// clusters are never wiped by accident. Empty for stateless
    /// software (jq, curl, …).
    pub data_dirs: &'static [&'static str],
    /// Free-form note shown in the panel (e.g. "发行版仓库版本可能滞后").
    pub notes: Option<&'static str>,
    /// `true` when this software's daemon supports `systemctl reload`
    /// without a downtime restart (nginx, apache, …). The Software
    /// panel uses this to show a "Reload (no downtime)" entry in the
    /// row's service menu in addition to start / stop / restart.
    pub supports_reload: bool,
}

/// Canonical package-manager IDs. Stable strings exposed to the UI via
/// `as_str` — keep them short and lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    /// Debian / Ubuntu / Mint / Raspbian / Pop / Elementary / Kali.
    Apt,
    /// Fedora / RHEL / CentOS / Rocky / Alma / OL / Amazon Linux.
    Dnf,
    /// Older RHEL-family hosts that don't have `dnf`. Mostly a
    /// fallback inside the dnf install command.
    Yum,
    /// Alpine.
    Apk,
    /// Arch / Manjaro / EndeavourOS.
    Pacman,
    /// openSUSE / SLES / SLED.
    Zypper,
}

impl PackageManager {
    /// Stable lowercase id for serialization to the UI / event payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            PackageManager::Apt => "apt",
            PackageManager::Dnf => "dnf",
            PackageManager::Yum => "yum",
            PackageManager::Apk => "apk",
            PackageManager::Pacman => "pacman",
            PackageManager::Zypper => "zypper",
        }
    }
}

/// Result of a probe — populated in one round trip per package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageStatus {
    /// Stable package id (e.g. `"sqlite3"`).
    pub id: String,
    /// `true` when the binary is on PATH and exits 0 from `--version`.
    pub installed: bool,
    /// Parsed version string when the probe succeeded; `None` when the
    /// probe couldn't extract a recognisable version token.
    pub version: Option<String>,
    /// `Some(true)` / `Some(false)` only when the descriptor declared a
    /// service unit and the systemctl probe ran. `None` for software
    /// without a service or when systemctl is missing.
    pub service_active: Option<bool>,
}

/// Snapshot of the host's package-manager environment, used to drive
/// the panel header ("Ubuntu 24.04 · apt") and to gate install buttons
/// when no manager is detected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostPackageEnv {
    /// `ID=` from `/etc/os-release` lowercased (e.g. `ubuntu`).
    pub distro_id: String,
    /// `PRETTY_NAME=` from `/etc/os-release` (e.g. `Ubuntu 24.04 LTS`).
    pub distro_pretty: String,
    /// `None` when `distro_id` isn't in the supported list — the panel
    /// disables install buttons in that case.
    pub package_manager: Option<PackageManager>,
    /// `true` when `id -u` reported `0`. Drives whether commands get
    /// the `sudo -n ` prefix.
    pub is_root: bool,
}

/// Outcome of an install / update attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum InstallStatus {
    /// Binary is now on PATH (either it already was, or the install
    /// command succeeded).
    Installed,
    /// We could not match the host's distro to any package manager.
    UnsupportedDistro,
    /// `sudo -n` reported that a password is required.
    SudoRequiresPassword,
    /// The package manager exited non-zero and a follow-up probe still
    /// can't see the binary.
    PackageManagerFailed,
}

/// Per-call options for [`uninstall`]. The frontend's uninstall dialog
/// maps each checkbox onto one of these flags. apk + pacman ignore
/// flags they don't natively support (apk has no autoremove, pacman's
/// `-Rns` already implies the equivalents, etc.) — see
/// `build_uninstall_command`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UninstallOptions {
    /// `apt purge` instead of `apt remove`; `pacman -n` flag set; for
    /// other managers no-op (their default remove already drops
    /// configs, or they have no config concept).
    pub purge_config: bool,
    /// Append `apt-get autoremove -y` / `dnf autoremove -y` after the
    /// remove succeeds; switch pacman to `-s`; switch zypper to
    /// `--clean-deps`. Silently ignored on apk.
    pub autoremove: bool,
    /// `rm -rf` every entry in the descriptor's `data_dirs` after the
    /// package manager has finished. Empty descriptor `data_dirs` =
    /// no-op even when set.
    pub remove_data_dirs: bool,
}

/// One of the systemctl verbs the Software panel exposes for a row's
/// service. `Reload` is only offered when the descriptor's
/// `supports_reload` is `true` — most services we ship would
/// effectively restart on `reload` so we hide the option to keep the
/// menu meaningful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceAction {
    /// `systemctl start <unit>`.
    Start,
    /// `systemctl stop <unit>`.
    Stop,
    /// `systemctl restart <unit>` — drops connections; the default
    /// when the user wants their config change to take effect.
    Restart,
    /// `systemctl reload <unit>` — only offered for descriptors with
    /// `supports_reload = true` (currently nginx).
    Reload,
}

impl ServiceAction {
    /// Lowercase verb passed straight to `systemctl`. Stable across
    /// all systemd versions we target.
    pub fn as_systemctl_verb(self) -> &'static str {
        match self {
            ServiceAction::Start => "start",
            ServiceAction::Stop => "stop",
            ServiceAction::Restart => "restart",
            ServiceAction::Reload => "reload",
        }
    }
}

/// Outcome class for [`service_action`]. Mirrors the install /
/// uninstall outcome shape so the frontend can reuse a single
/// "describe outcome" formatting helper.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum ServiceActionStatus {
    /// systemctl exited 0 and the post-action `is-active` agrees with
    /// the requested verb (`start` / `restart` / `reload` → active;
    /// `stop` → inactive).
    Ok,
    /// `sudo -n` reported that a password is required.
    SudoRequiresPassword,
    /// Anything else: systemctl exited non-zero, or the post-probe
    /// disagrees with the requested verb.
    Failed,
}

/// Structured result of a service action. `service_active_after`
/// matches the post-action `systemctl is-active` ground truth so the
/// panel can flip its dot without doing a full re-probe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceActionReport {
    /// Echoes the descriptor id so the frontend can correlate event
    /// streams to rows.
    pub package_id: String,
    /// Outcome class — see [`ServiceActionStatus`].
    pub status: ServiceActionStatus,
    /// Verb that was attempted (`"start"` / `"stop"` / `"restart"`
    /// / `"reload"`).
    pub action: String,
    /// Service unit name we drove (e.g. `"redis-server"` on debian,
    /// `"redis"` on rhel-family). Empty when the descriptor has no
    /// service unit for this distro family — shouldn't happen in
    /// practice because the UI gates the menu on `has_service`.
    pub unit: String,
    /// Exact command that ran on the remote (sudo + sh -c …).
    pub command: String,
    /// Exit code from the systemctl invocation.
    pub exit_code: i32,
    /// Last ~60 lines of merged stdout+stderr.
    pub output_tail: String,
    /// `systemctl is-active` re-probe after the action. `true` =
    /// active. `false` for any other value (`inactive` / `failed` /
    /// `activating` / probe error). The frontend uses this directly
    /// for the row's service-active dot.
    pub service_active_after: bool,
}

/// Outcome of an uninstall attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum UninstallStatus {
    /// The package manager removed the package and a follow-up probe
    /// confirms the binary is no longer on PATH.
    Uninstalled,
    /// We could not match the host's distro to any package manager.
    UnsupportedDistro,
    /// `sudo -n` reported that a password is required.
    SudoRequiresPassword,
    /// The package manager exited non-zero, or a post-removal probe
    /// still finds the binary on PATH.
    PackageManagerFailed,
    /// Pre-probe says the package isn't installed — nothing to do.
    /// We still surface this as a "successful" no-op so the UI can
    /// drop the row's "installed" badge.
    NotInstalled,
}

/// Structured result of an uninstall attempt. Mirrors [`InstallReport`]
/// in shape so the frontend can reuse a single outcome card layout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UninstallReport {
    /// Echoes the descriptor id so the frontend can correlate event
    /// streams to rows.
    pub package_id: String,
    /// Outcome class — see [`UninstallStatus`].
    pub status: UninstallStatus,
    /// `ID=` from `/etc/os-release` (empty when probe failed).
    pub distro_id: String,
    /// Lowercase package-manager label or empty on UnsupportedDistro.
    pub package_manager: String,
    /// Exact command that was run on the remote.
    pub command: String,
    /// Exit code reported by the uninstall command. `0` for the
    /// `NotInstalled` no-op fast path.
    pub exit_code: i32,
    /// Last ~60 lines of merged stdout+stderr.
    pub output_tail: String,
    /// True iff `remove_data_dirs` was requested AND the package
    /// manager succeeded AND data dirs were declared for the
    /// descriptor — i.e. `rm -rf` actually ran. False otherwise (so
    /// the panel's "data wiped" badge never lies).
    pub data_dirs_removed: bool,
}

/// Structured result of an install / update. Always populated — only
/// SSH-level failures surface as `Err`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallReport {
    /// Echoes the descriptor id so the frontend can correlate event
    /// streams to rows.
    pub package_id: String,
    /// Outcome class — see [`InstallStatus`].
    pub status: InstallStatus,
    /// `ID=` from `/etc/os-release` (empty when probe failed).
    pub distro_id: String,
    /// Lowercase package-manager label (`"apt"`, `"dnf"`, …) or empty
    /// when the distro wasn't supported.
    pub package_manager: String,
    /// Exact command that was run on the remote (with `sudo -n` /
    /// `DEBIAN_FRONTEND=...` prefixes already substituted).
    pub command: String,
    /// Exit code reported by the install/update command.
    pub exit_code: i32,
    /// Last ~60 lines of merged stdout+stderr — the UI also gets the
    /// streamed lines, but this serves as a single-shot summary if the
    /// caller didn't subscribe to events.
    pub output_tail: String,
    /// Version string from a post-install `--version` probe. `None`
    /// when the package manager failed or no version token matched.
    pub installed_version: Option<String>,
    /// `Some(true)` when the descriptor has a service and the post-
    /// install `systemctl enable --now` succeeded.
    pub service_active: Option<bool>,
}

// ── Registry ────────────────────────────────────────────────────────

/// v1 software list. Order here is the order rendered in the panel.
pub fn registry() -> &'static [PackageDescriptor] {
    REGISTRY
}

const REGISTRY: &[PackageDescriptor] = &[
    PackageDescriptor {
        id: "sqlite3",
        display_name: "SQLite",
        probe_command: "command -v sqlite3 >/dev/null 2>&1 && sqlite3 --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["sqlite3"]),
            (PackageManager::Dnf, &["sqlite"]),
            (PackageManager::Yum, &["sqlite"]),
            (PackageManager::Apk, &["sqlite"]),
            (PackageManager::Pacman, &["sqlite"]),
            (PackageManager::Zypper, &["sqlite3"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
    },
    PackageDescriptor {
        id: "docker",
        display_name: "Docker Engine",
        probe_command: "command -v docker >/dev/null 2>&1 && docker --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["docker.io"]),
            (PackageManager::Dnf, &["docker"]),
            (PackageManager::Yum, &["docker"]),
            (PackageManager::Apk, &["docker"]),
            (PackageManager::Pacman, &["docker"]),
            (PackageManager::Zypper, &["docker"]),
        ],
        service_units: &[
            (PackageManager::Apt, "docker"),
            (PackageManager::Dnf, "docker"),
            (PackageManager::Yum, "docker"),
            (PackageManager::Apk, "docker"),
            (PackageManager::Pacman, "docker"),
            (PackageManager::Zypper, "docker"),
        ],
        data_dirs: &["/var/lib/docker", "/var/lib/containerd"],
        notes: Some("发行版仓库的 Docker 版本可能旧；如需最新版后续走 v2 的官方脚本通道。"),
        supports_reload: false,
    },
    PackageDescriptor {
        id: "compose",
        display_name: "Docker Compose",
        probe_command: "command -v docker >/dev/null 2>&1 && docker compose version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["docker-compose-v2"]),
            (PackageManager::Dnf, &["docker-compose-plugin"]),
            (PackageManager::Yum, &["docker-compose-plugin"]),
            (PackageManager::Apk, &["docker-cli-compose"]),
            (PackageManager::Pacman, &["docker-compose"]),
            (PackageManager::Zypper, &["docker-compose"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
    },
    PackageDescriptor {
        id: "redis",
        display_name: "Redis",
        probe_command: "command -v redis-server >/dev/null 2>&1 && redis-server --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["redis-server"]),
            (PackageManager::Dnf, &["redis"]),
            (PackageManager::Yum, &["redis"]),
            (PackageManager::Apk, &["redis"]),
            (PackageManager::Pacman, &["redis"]),
            (PackageManager::Zypper, &["redis"]),
        ],
        service_units: &[
            (PackageManager::Apt, "redis-server"),
            (PackageManager::Dnf, "redis"),
            (PackageManager::Yum, "redis"),
            (PackageManager::Apk, "redis"),
            (PackageManager::Pacman, "redis"),
            (PackageManager::Zypper, "redis"),
        ],
        data_dirs: &["/var/lib/redis"],
        notes: None,
        supports_reload: false,
    },
    PackageDescriptor {
        id: "postgres",
        display_name: "PostgreSQL",
        probe_command: "command -v psql >/dev/null 2>&1 && psql --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["postgresql"]),
            (PackageManager::Dnf, &["postgresql-server"]),
            (PackageManager::Yum, &["postgresql-server"]),
            (PackageManager::Apk, &["postgresql"]),
            (PackageManager::Pacman, &["postgresql"]),
            (PackageManager::Zypper, &["postgresql-server"]),
        ],
        service_units: &[
            (PackageManager::Apt, "postgresql"),
            (PackageManager::Dnf, "postgresql"),
            (PackageManager::Yum, "postgresql"),
            (PackageManager::Apk, "postgresql"),
            (PackageManager::Pacman, "postgresql"),
            (PackageManager::Zypper, "postgresql"),
        ],
        data_dirs: &["/var/lib/postgresql", "/var/lib/pgsql"],
        notes: None,
        supports_reload: false,
    },
    PackageDescriptor {
        id: "mariadb",
        display_name: "MySQL / MariaDB",
        probe_command: "command -v mysql >/dev/null 2>&1 && mysql --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["mariadb-server"]),
            (PackageManager::Dnf, &["mariadb-server"]),
            (PackageManager::Yum, &["mariadb-server"]),
            (PackageManager::Apk, &["mariadb"]),
            (PackageManager::Pacman, &["mariadb"]),
            (PackageManager::Zypper, &["mariadb"]),
        ],
        service_units: &[
            (PackageManager::Apt, "mariadb"),
            (PackageManager::Dnf, "mariadb"),
            (PackageManager::Yum, "mariadb"),
            (PackageManager::Apk, "mariadb"),
            (PackageManager::Pacman, "mariadb"),
            (PackageManager::Zypper, "mariadb"),
        ],
        data_dirs: &["/var/lib/mysql"],
        notes: Some("默认装 MariaDB（与 MySQL 协议兼容，发行版仓库的标准选择）。"),
        supports_reload: false,
    },
    PackageDescriptor {
        id: "nginx",
        display_name: "nginx",
        probe_command: "command -v nginx >/dev/null 2>&1 && nginx -v 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["nginx"]),
            (PackageManager::Dnf, &["nginx"]),
            (PackageManager::Yum, &["nginx"]),
            (PackageManager::Apk, &["nginx"]),
            (PackageManager::Pacman, &["nginx"]),
            (PackageManager::Zypper, &["nginx"]),
        ],
        service_units: &[
            (PackageManager::Apt, "nginx"),
            (PackageManager::Dnf, "nginx"),
            (PackageManager::Yum, "nginx"),
            (PackageManager::Apk, "nginx"),
            (PackageManager::Pacman, "nginx"),
            (PackageManager::Zypper, "nginx"),
        ],
        // /etc/nginx is config (purge handles it); /var/log/nginx is logs
        // not user data. nginx is the rare service with nothing in the
        // dataset bucket.
        data_dirs: &[],
        notes: None,
        // nginx reloads its config without dropping connections — the
        // panel surfaces this as a separate service action so users
        // don't reach for "restart" out of habit.
        supports_reload: true,
    },
    PackageDescriptor {
        id: "jq",
        display_name: "jq",
        probe_command: "command -v jq >/dev/null 2>&1 && jq --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["jq"]),
            (PackageManager::Dnf, &["jq"]),
            (PackageManager::Yum, &["jq"]),
            (PackageManager::Apk, &["jq"]),
            (PackageManager::Pacman, &["jq"]),
            (PackageManager::Zypper, &["jq"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
    },
    PackageDescriptor {
        id: "curl",
        display_name: "curl",
        probe_command: "command -v curl >/dev/null 2>&1 && curl --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["curl"]),
            (PackageManager::Dnf, &["curl"]),
            (PackageManager::Yum, &["curl"]),
            (PackageManager::Apk, &["curl"]),
            (PackageManager::Pacman, &["curl"]),
            (PackageManager::Zypper, &["curl"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
    },
];

/// Look up a descriptor by id. `None` means "not in registry".
pub fn descriptor(id: &str) -> Option<&'static PackageDescriptor> {
    REGISTRY.iter().find(|d| d.id == id)
}

// ── Distro / package-manager detection ──────────────────────────────

/// Read `/etc/os-release` and return `(ID, PRETTY_NAME)`. Falls back to
/// `(ID_LIKE first token, "")` when `ID` is missing. Both fields are
/// empty when the file isn't readable.
pub async fn read_os_release(session: &SshSession) -> (String, String) {
    let Ok((code, stdout)) = session
        .exec_command("cat /etc/os-release 2>/dev/null")
        .await
    else {
        return (String::new(), String::new());
    };
    if code != 0 {
        return (String::new(), String::new());
    }
    let mut id = String::new();
    let mut id_like = String::new();
    let mut pretty = String::new();
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("ID=") {
            id = strip_os_release_quotes(rest).to_lowercase();
        } else if let Some(rest) = line.strip_prefix("ID_LIKE=") {
            id_like = strip_os_release_quotes(rest).to_lowercase();
        } else if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            pretty = strip_os_release_quotes(rest).to_string();
        }
    }
    let id = if !id.is_empty() {
        id
    } else {
        id_like
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string()
    };
    (id, pretty)
}

/// Strip surrounding `"..."` or `'...'` from `/etc/os-release` values.
fn strip_os_release_quotes(value: &str) -> &str {
    value
        .trim()
        .trim_start_matches('"')
        .trim_end_matches('"')
        .trim_start_matches('\'')
        .trim_end_matches('\'')
}

/// Map an `/etc/os-release` `ID=` to the package manager we drive.
pub fn pick_package_manager(distro_id: &str) -> Option<PackageManager> {
    match distro_id {
        "debian" | "ubuntu" | "linuxmint" | "raspbian" | "pop" | "elementary" | "kali" => {
            Some(PackageManager::Apt)
        }
        "fedora" => Some(PackageManager::Dnf),
        "rhel" | "centos" | "rocky" | "almalinux" | "ol" | "amzn" => Some(PackageManager::Dnf),
        "alpine" => Some(PackageManager::Apk),
        "arch" | "manjaro" | "endeavouros" => Some(PackageManager::Pacman),
        "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "sles" | "sled" => {
            Some(PackageManager::Zypper)
        }
        _ => None,
    }
}

/// `id -u` reports `0` for root. Treat any failure as "not root" so we
/// err on the side of using `sudo`.
pub async fn is_root(session: &SshSession) -> bool {
    let Ok((code, stdout)) = session.exec_command("id -u").await else {
        return false;
    };
    code == 0 && stdout.trim() == "0"
}

// ── Public API: probe / install / update ────────────────────────────

/// Probe the host environment in one shot — distro + manager + sudo
/// state. The panel uses this for the header and to disable the
/// install column on unsupported distros.
pub async fn probe_host_env(session: &SshSession) -> HostPackageEnv {
    let (distro_id, distro_pretty) = read_os_release(session).await;
    let package_manager = pick_package_manager(&distro_id);
    let is_root = is_root(session).await;
    HostPackageEnv {
        distro_id,
        distro_pretty,
        package_manager,
        is_root,
    }
}

/// Probe one descriptor — `installed?`, `version`, `service active?`.
pub async fn probe_status(session: &SshSession, id: &str) -> Option<PackageStatus> {
    let descriptor = descriptor(id)?;
    let (installed, version) = match session.exec_command(descriptor.probe_command).await {
        Ok((0, stdout)) => {
            let v = parse_version(&stdout);
            (true, v)
        }
        _ => (false, None),
    };
    // Service unit name depends on distro family; only run the
    // systemctl probe when (a) the binary is actually installed and
    // (b) the descriptor declares a service unit. Awaiting the inner
    // future inside a struct chain doesn't compose cleanly, so spell
    // out the resolution.
    let service_active: Option<bool> = if installed && !descriptor.service_units.is_empty() {
        let env = probe_host_env(session).await;
        match env
            .package_manager
            .and_then(|pm| descriptor_service_unit(descriptor, pm))
        {
            Some(unit) => Some(systemctl_is_active(session, unit).await),
            None => None,
        }
    } else {
        None
    };
    Some(PackageStatus {
        id: descriptor.id.to_string(),
        installed,
        version,
        service_active,
    })
}

/// Probe the entire registry. Runs probes sequentially — they're all
/// `command -v` style one-liners that finish in <50ms each, and the
/// SSH channel is single-threaded per-session anyway.
pub async fn probe_all(session: &SshSession) -> Vec<PackageStatus> {
    let mut out = Vec::with_capacity(REGISTRY.len());
    for descriptor in REGISTRY {
        if let Some(s) = probe_status(session, descriptor.id).await {
            out.push(s);
        }
    }
    out
}

/// Install a single package. Streams every output line through
/// `on_line`. Always returns a structured report — only an SSH-level
/// failure surfaces as `Err`.
///
/// `version` pins to a specific package-manager version when `Some`.
/// pacman silently ignores it (Arch repos only carry the latest).
pub async fn install<F>(
    session: &SshSession,
    id: &str,
    enable_service: bool,
    version: Option<&str>,
    on_line: F,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    run_install_or_update(session, id, false, enable_service, version, on_line).await
}

/// Update (re-install / upgrade) a single package. Only meaningful for
/// already-installed packages — for missing ones it falls through to
/// the install command (most package managers' install is idempotent).
///
/// `version` pins to a specific package-manager version when `Some`.
/// pacman silently ignores it (Arch repos only carry the latest).
pub async fn update<F>(
    session: &SshSession,
    id: &str,
    enable_service: bool,
    version: Option<&str>,
    on_line: F,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    run_install_or_update(session, id, true, enable_service, version, on_line).await
}

/// List the package-manager-visible versions for a descriptor on the
/// remote host, freshest first. The frontend caches this for 5
/// minutes per host+package — [`available_versions`] always re-runs
/// the command.
///
/// Per-manager dispatch:
/// * apt → `apt-cache madison`
/// * dnf / yum → `list available --showduplicates`
/// * apk → `apk version -a` (filters to descriptor's package row)
/// * pacman → empty Vec (Arch repos don't carry historical versions —
///   the panel hides the dropdown)
/// * zypper → `zypper search -s`, parsed with awk on `|` columns
///
/// Returns an empty Vec on unsupported distro / unknown package.
pub async fn available_versions(
    session: &SshSession,
    id: &str,
) -> Result<Vec<String>> {
    let descriptor = descriptor(id).ok_or_else(|| {
        SshError::InvalidConfig(format!("unknown package id: {id}"))
    })?;
    let env = probe_host_env(session).await;
    let Some(manager) = env.package_manager else {
        return Ok(Vec::new());
    };
    let Some(packages) = packages_for(descriptor, manager) else {
        return Ok(Vec::new());
    };
    let pkg = match packages.first().copied() {
        Some(p) if !p.is_empty() => p,
        _ => return Ok(Vec::new()),
    };
    let Some(cmd) = build_versions_command(manager, pkg) else {
        return Ok(Vec::new());
    };
    let (_, stdout) = session.exec_command(&cmd).await?;
    Ok(parse_versions_output(&stdout))
}

/// Blocking wrapper for [`probe_host_env`].
pub fn probe_host_env_blocking(session: &SshSession) -> HostPackageEnv {
    crate::ssh::runtime::shared().block_on(probe_host_env(session))
}

/// Blocking wrapper for [`probe_all`].
pub fn probe_all_blocking(session: &SshSession) -> Vec<PackageStatus> {
    crate::ssh::runtime::shared().block_on(probe_all(session))
}

/// Blocking wrapper for [`install`]. Tauri commands using
/// `spawn_blocking` call this directly so they can use a `FnMut(&str)`
/// from the synchronous closure body without re-entering an async
/// context.
pub fn install_blocking<F>(
    session: &SshSession,
    id: &str,
    enable_service: bool,
    version: Option<&str>,
    on_line: F,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    crate::ssh::runtime::shared().block_on(install(session, id, enable_service, version, on_line))
}

/// Blocking wrapper for [`update`].
pub fn update_blocking<F>(
    session: &SshSession,
    id: &str,
    enable_service: bool,
    version: Option<&str>,
    on_line: F,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    crate::ssh::runtime::shared().block_on(update(session, id, enable_service, version, on_line))
}

/// Blocking wrapper for [`available_versions`]. Tauri commands using
/// `spawn_blocking` invoke it from the sync closure body.
pub fn available_versions_blocking(
    session: &SshSession,
    id: &str,
) -> Result<Vec<String>> {
    crate::ssh::runtime::shared().block_on(available_versions(session, id))
}

/// Uninstall a single package. Streams every output line through
/// `on_line`. Always returns a structured report — only an SSH-level
/// failure surfaces as `Err`.
///
/// Sequence executed remotely (assembled into one `sh -c '…'` so the
/// streaming output stays on one channel):
///
/// 1. `systemctl disable --now <unit>` when the descriptor declares a
///    service (best-effort, suppressed on hosts without systemd).
/// 2. The package manager's remove command, with `purge_config` /
///    `autoremove` flags applied per the matrix in
///    [`build_uninstall_command`].
/// 3. `rm -rf <data_dirs>` when `remove_data_dirs` was requested and
///    the descriptor has any. This step is `&&`-chained to the
///    remove step so a failed package removal never wipes user data.
pub async fn uninstall<F>(
    session: &SshSession,
    id: &str,
    opts: &UninstallOptions,
    on_line: F,
) -> Result<UninstallReport>
where
    F: FnMut(&str),
{
    run_uninstall(session, id, opts, on_line).await
}

/// Blocking wrapper for [`uninstall`].
pub fn uninstall_blocking<F>(
    session: &SshSession,
    id: &str,
    opts: &UninstallOptions,
    on_line: F,
) -> Result<UninstallReport>
where
    F: FnMut(&str),
{
    crate::ssh::runtime::shared().block_on(uninstall(session, id, opts, on_line))
}

/// Drive a `systemctl <verb> <unit>` for one descriptor's service.
/// Streams stdout / stderr through `on_line` for live UI feedback and
/// always returns a structured report (only SSH-level failures
/// surface as `Err`).
///
/// The descriptor's `service_units` matrix picks the unit name per
/// package manager (e.g. `redis-server` on apt, `redis` on dnf). When
/// the descriptor has no service for the resolved manager we still
/// return `Ok` with [`ServiceActionStatus::Failed`] and an empty
/// `unit` — the panel gates the menu on `has_service` so this is a
/// belt-and-suspenders path, not a UX one.
pub async fn service_action<F>(
    session: &SshSession,
    descriptor: &PackageDescriptor,
    action: ServiceAction,
    on_line: F,
) -> Result<ServiceActionReport>
where
    F: FnMut(&str),
{
    run_service_action(session, descriptor, action, on_line).await
}

/// Blocking wrapper for [`service_action`]. Tauri commands using
/// `spawn_blocking` call this directly so they can pass a synchronous
/// `FnMut(&str)` for the streaming callback.
pub fn service_action_blocking<F>(
    session: &SshSession,
    descriptor: &PackageDescriptor,
    action: ServiceAction,
    on_line: F,
) -> Result<ServiceActionReport>
where
    F: FnMut(&str),
{
    crate::ssh::runtime::shared()
        .block_on(service_action(session, descriptor, action, on_line))
}

/// Pull the most recent `lines` rows of `journalctl -u <unit>` output
/// for one descriptor's service. One-shot — no streaming. Returns the
/// list of lines in the order journalctl printed them (oldest →
/// newest with `--no-pager`).
///
/// The frontend uses this to populate a "View logs" dialog with a
/// refresh button; a true follow-style `-f` tail is intentionally
/// out of scope (cancel semantics + multi-host fan-out push it to a
/// later milestone — the existing Log panel handles real-time tail).
pub async fn journalctl_tail(
    session: &SshSession,
    descriptor: &PackageDescriptor,
    lines: usize,
) -> Result<Vec<String>> {
    run_journalctl_tail(session, descriptor, lines).await
}

/// Blocking wrapper for [`journalctl_tail`].
pub fn journalctl_tail_blocking(
    session: &SshSession,
    descriptor: &PackageDescriptor,
    lines: usize,
) -> Result<Vec<String>> {
    crate::ssh::runtime::shared().block_on(journalctl_tail(session, descriptor, lines))
}

// ── Internals ───────────────────────────────────────────────────────

/// Common install / update path. `is_update` switches the apt/dnf
/// command from "install" to "install --only-upgrade" / equivalent.
/// `version`, when set, pins the package-manager invocation to that
/// version (formatted per manager — e.g. `pkg=ver` for apt/apk/zypper,
/// `pkg-ver` for dnf/yum). pacman silently ignores it.
async fn run_install_or_update<F>(
    session: &SshSession,
    id: &str,
    is_update: bool,
    enable_service: bool,
    version: Option<&str>,
    mut on_line: F,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    let descriptor = descriptor(id).ok_or_else(|| {
        SshError::InvalidConfig(format!("unknown package id: {id}"))
    })?;

    let env = probe_host_env(session).await;

    let Some(manager) = env.package_manager else {
        return Ok(InstallReport {
            package_id: id.to_string(),
            status: InstallStatus::UnsupportedDistro,
            distro_id: env.distro_id,
            package_manager: String::new(),
            command: String::new(),
            exit_code: 0,
            output_tail: String::new(),
            installed_version: None,
            service_active: None,
        });
    };

    let Some(packages) = packages_for(descriptor, manager) else {
        return Ok(InstallReport {
            package_id: id.to_string(),
            status: InstallStatus::UnsupportedDistro,
            distro_id: env.distro_id,
            package_manager: manager.as_str().to_string(),
            command: String::new(),
            exit_code: 0,
            output_tail: String::new(),
            installed_version: None,
            service_active: None,
        });
    };

    let install_inner = build_install_command(manager, packages, is_update, version);
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let command = format!(
        "{prefix}sh -c {} 2>&1",
        shell_single_quote(&install_inner)
    );

    let mut tail_lines: Vec<String> = Vec::new();
    let (exit_code, _full) = session
        .exec_command_streaming(&command, |line| {
            on_line(line);
            tail_lines.push(line.to_string());
            if tail_lines.len() > 80 {
                tail_lines.drain(0..tail_lines.len() - 60);
            }
        })
        .await?;
    let output_tail = tail_lines.join("\n");

    if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        return Ok(InstallReport {
            package_id: id.to_string(),
            status: InstallStatus::SudoRequiresPassword,
            distro_id: env.distro_id,
            package_manager: manager.as_str().to_string(),
            command,
            exit_code,
            output_tail,
            installed_version: None,
            service_active: None,
        });
    }

    // Re-probe so the report reflects the post-install reality even if
    // the manager exited 0 but the binary still isn't on PATH (rare —
    // happens with broken alternative-versions / multilib packages).
    let post = probe_status(session, id).await;
    let installed_version = post.as_ref().and_then(|s| s.version.clone());
    let was_installed_after = post.as_ref().map(|s| s.installed).unwrap_or(false);

    let status = if was_installed_after {
        InstallStatus::Installed
    } else {
        InstallStatus::PackageManagerFailed
    };

    // Best-effort service enable + start. We don't fail the install if
    // this step trips; just record the resulting service_active state.
    let service_active = if was_installed_after && enable_service {
        if let Some(unit) = descriptor_service_unit(descriptor, manager) {
            let svc_cmd = format!(
                "{prefix}sh -c {} 2>&1",
                shell_single_quote(&format!(
                    "systemctl enable --now {unit} || true"
                ))
            );
            let _ = session
                .exec_command_streaming(&svc_cmd, &mut on_line)
                .await;
            Some(systemctl_is_active(session, unit).await)
        } else {
            None
        }
    } else if was_installed_after {
        // No enable requested, but report whether the service happens
        // to be running already (e.g. distro auto-started it post-install).
        if let Some(unit) = descriptor_service_unit(descriptor, manager) {
            Some(systemctl_is_active(session, unit).await)
        } else {
            None
        }
    } else {
        None
    };

    Ok(InstallReport {
        package_id: id.to_string(),
        status,
        distro_id: env.distro_id,
        package_manager: manager.as_str().to_string(),
        command,
        exit_code,
        output_tail,
        installed_version,
        service_active,
    })
}

/// Common uninstall path. Wraps service-disable + remove +
/// (optionally) autoremove + (optionally) `rm -rf <data_dirs>` into
/// one streamed remote shell invocation.
async fn run_uninstall<F>(
    session: &SshSession,
    id: &str,
    opts: &UninstallOptions,
    mut on_line: F,
) -> Result<UninstallReport>
where
    F: FnMut(&str),
{
    let descriptor = descriptor(id).ok_or_else(|| {
        SshError::InvalidConfig(format!("unknown package id: {id}"))
    })?;

    let env = probe_host_env(session).await;

    let Some(manager) = env.package_manager else {
        return Ok(UninstallReport {
            package_id: id.to_string(),
            status: UninstallStatus::UnsupportedDistro,
            distro_id: env.distro_id,
            package_manager: String::new(),
            command: String::new(),
            exit_code: 0,
            output_tail: String::new(),
            data_dirs_removed: false,
        });
    };

    let Some(packages) = packages_for(descriptor, manager) else {
        return Ok(UninstallReport {
            package_id: id.to_string(),
            status: UninstallStatus::UnsupportedDistro,
            distro_id: env.distro_id,
            package_manager: manager.as_str().to_string(),
            command: String::new(),
            exit_code: 0,
            output_tail: String::new(),
            data_dirs_removed: false,
        });
    };

    // Fast no-op: probe before doing anything destructive. The user
    // may have manually removed the package since the last probe and
    // this skips an apt round-trip + a misleading "remove failed"
    // when there's literally nothing to remove.
    let pre = probe_status(session, id).await;
    if !pre.as_ref().map(|s| s.installed).unwrap_or(false) {
        return Ok(UninstallReport {
            package_id: id.to_string(),
            status: UninstallStatus::NotInstalled,
            distro_id: env.distro_id,
            package_manager: manager.as_str().to_string(),
            command: String::new(),
            exit_code: 0,
            output_tail: String::new(),
            data_dirs_removed: false,
        });
    }

    let service_unit = descriptor_service_unit(descriptor, manager);
    let inner = build_uninstall_command(manager, packages, descriptor.data_dirs, opts, service_unit);
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let command = format!("{prefix}sh -c {} 2>&1", shell_single_quote(&inner));

    let mut tail_lines: Vec<String> = Vec::new();
    let (exit_code, _full) = session
        .exec_command_streaming(&command, |line| {
            on_line(line);
            tail_lines.push(line.to_string());
            if tail_lines.len() > 80 {
                tail_lines.drain(0..tail_lines.len() - 60);
            }
        })
        .await?;
    let output_tail = tail_lines.join("\n");

    if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        return Ok(UninstallReport {
            package_id: id.to_string(),
            status: UninstallStatus::SudoRequiresPassword,
            distro_id: env.distro_id,
            package_manager: manager.as_str().to_string(),
            command,
            exit_code,
            output_tail,
            data_dirs_removed: false,
        });
    }

    // Re-probe to confirm. Some package managers exit 0 on a remove
    // that didn't actually unhook the binary (held packages,
    // alternatives slots) — the post-probe is the ground truth.
    let post = probe_status(session, id).await;
    let still_installed = post.as_ref().map(|s| s.installed).unwrap_or(false);
    let status = if !still_installed {
        UninstallStatus::Uninstalled
    } else {
        UninstallStatus::PackageManagerFailed
    };

    let data_dirs_removed = !still_installed
        && opts.remove_data_dirs
        && !descriptor.data_dirs.is_empty();

    Ok(UninstallReport {
        package_id: id.to_string(),
        status,
        distro_id: env.distro_id,
        package_manager: manager.as_str().to_string(),
        command,
        exit_code,
        output_tail,
        data_dirs_removed,
    })
}

/// Common service-action path. Resolves the unit, runs
/// `systemctl <verb> <unit>` (with `sudo -n` when non-root), streams
/// the output, then re-probes `is-active` to decide the final
/// status. The post-probe is the source of truth — a manager that
/// exits 0 but leaves the unit `failed` (e.g. dependency cycle, port
/// collision) should still surface as `Failed`.
async fn run_service_action<F>(
    session: &SshSession,
    descriptor: &PackageDescriptor,
    action: ServiceAction,
    mut on_line: F,
) -> Result<ServiceActionReport>
where
    F: FnMut(&str),
{
    let env = probe_host_env(session).await;
    let unit = env
        .package_manager
        .and_then(|pm| descriptor_service_unit(descriptor, pm));
    let Some(unit) = unit else {
        return Ok(ServiceActionReport {
            package_id: descriptor.id.to_string(),
            status: ServiceActionStatus::Failed,
            action: action.as_systemctl_verb().to_string(),
            unit: String::new(),
            command: String::new(),
            exit_code: 0,
            output_tail: String::new(),
            service_active_after: false,
        });
    };

    let command = build_systemctl_command(action, unit, env.is_root);

    let mut tail_lines: Vec<String> = Vec::new();
    let (exit_code, _full) = session
        .exec_command_streaming(&command, |line| {
            on_line(line);
            tail_lines.push(line.to_string());
            if tail_lines.len() > 80 {
                tail_lines.drain(0..tail_lines.len() - 60);
            }
        })
        .await?;
    let output_tail = tail_lines.join("\n");

    if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        return Ok(ServiceActionReport {
            package_id: descriptor.id.to_string(),
            status: ServiceActionStatus::SudoRequiresPassword,
            action: action.as_systemctl_verb().to_string(),
            unit: unit.to_string(),
            command,
            exit_code,
            output_tail,
            service_active_after: false,
        });
    }

    let active_after = systemctl_is_active(session, unit).await;
    let expected_active = !matches!(action, ServiceAction::Stop);
    let succeeded = exit_code == 0 && active_after == expected_active;
    let status = if succeeded {
        ServiceActionStatus::Ok
    } else {
        ServiceActionStatus::Failed
    };

    Ok(ServiceActionReport {
        package_id: descriptor.id.to_string(),
        status,
        action: action.as_systemctl_verb().to_string(),
        unit: unit.to_string(),
        command,
        exit_code,
        output_tail,
        service_active_after: active_after,
    })
}

/// Common journalctl-tail path. We merge stdout+stderr (`2>&1`) so
/// hosts with `journalctl` warnings (missing unit, permission denied)
/// still produce something for the UI to show.
async fn run_journalctl_tail(
    session: &SshSession,
    descriptor: &PackageDescriptor,
    lines: usize,
) -> Result<Vec<String>> {
    let env = probe_host_env(session).await;
    let unit = env
        .package_manager
        .and_then(|pm| descriptor_service_unit(descriptor, pm));
    let Some(unit) = unit else {
        return Ok(Vec::new());
    };
    let command = build_journalctl_command(unit, lines, env.is_root);
    let (_code, stdout) = session.exec_command(&command).await?;
    Ok(stdout
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect())
}

/// Synthesise the `systemctl <verb> <unit>` command, with `sudo -n`
/// when non-root and `2>&1` so stderr lines reach the streaming
/// callback. The unit is single-quoted in case it ever contains
/// shell metacharacters (today they don't, but the matrix is data).
fn build_systemctl_command(
    action: ServiceAction,
    unit: &str,
    is_root: bool,
) -> String {
    let prefix = if is_root { "" } else { "sudo -n " };
    let verb = action.as_systemctl_verb();
    format!(
        "{prefix}systemctl {verb} {} 2>&1",
        shell_single_quote(unit)
    )
}

/// Synthesise the `journalctl -u <unit> -n <lines>` command. We pin
/// `--no-pager` so the channel doesn't end up in `less` waiting for
/// keypresses, and `2>&1` so "no entries" / permission warnings flow
/// alongside the entries themselves.
fn build_journalctl_command(unit: &str, lines: usize, is_root: bool) -> String {
    let prefix = if is_root { "" } else { "sudo -n " };
    format!(
        "{prefix}journalctl -u {} -n {} --no-pager 2>&1",
        shell_single_quote(unit),
        lines,
    )
}

/// Pick the package list for a manager, respecting registry order.
fn packages_for(
    descriptor: &PackageDescriptor,
    manager: PackageManager,
) -> Option<&'static [&'static str]> {
    descriptor
        .install_packages
        .iter()
        .find_map(|(m, pkgs)| (*m == manager).then_some(*pkgs))
}

/// Pick the service unit name for a manager.
fn descriptor_service_unit(
    descriptor: &PackageDescriptor,
    manager: PackageManager,
) -> Option<&'static str> {
    descriptor
        .service_units
        .iter()
        .find_map(|(m, unit)| (*m == manager).then_some(*unit))
}

/// Rewrite `packages` with the manager's version-pin syntax when
/// `version` is set. Pacman returns the unmodified list because Arch
/// repos only carry the latest. Whitespace in the version string is
/// stripped to keep the resulting shell argv clean.
fn format_packages_with_version(
    manager: PackageManager,
    packages: &[&str],
    version: Option<&str>,
) -> String {
    let Some(v) = version else {
        return packages.join(" ");
    };
    let v = v.trim();
    if v.is_empty() {
        return packages.join(" ");
    }
    match manager {
        PackageManager::Pacman => packages.join(" "),
        PackageManager::Apt | PackageManager::Apk | PackageManager::Zypper => packages
            .iter()
            .map(|p| format!("{p}={v}"))
            .collect::<Vec<_>>()
            .join(" "),
        PackageManager::Dnf | PackageManager::Yum => packages
            .iter()
            .map(|p| format!("{p}-{v}"))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Build the per-manager "list available versions" remote command.
/// Returns `None` for managers that can't enumerate historical
/// versions (currently pacman).
///
/// All commands suppress stderr and pipe through `awk` so the parsed
/// stdout is one version-per-line, freshest first when the manager
/// orders that way (`apt-cache madison`, `dnf list --showduplicates`).
fn build_versions_command(manager: PackageManager, package: &str) -> Option<String> {
    let pkg = shell_single_quote(package);
    match manager {
        PackageManager::Apt => Some(format!(
            "apt-cache madison {pkg} 2>/dev/null | awk '{{print $3}}'"
        )),
        PackageManager::Dnf => Some(format!(
            "dnf list available {pkg} --showduplicates -q 2>/dev/null | awk 'NR>1{{print $2}}'"
        )),
        PackageManager::Yum => Some(format!(
            "yum list available {pkg} --showduplicates -q 2>/dev/null | awk 'NR>1{{print $2}}'"
        )),
        PackageManager::Apk => Some(format!(
            "apk version -a 2>/dev/null | awk '$1=={pkg}{{print $3}}'"
        )),
        // pacman has no historical-version listing in the standard
        // repos. Returning None tells the frontend to hide the dropdown.
        PackageManager::Pacman => None,
        PackageManager::Zypper => Some(format!(
            "zypper search -s {pkg} 2>/dev/null | awk -F'|' 'NR>2 && /^v/{{gsub(/ /,\"\",$4); print $4}}' | sort -u"
        )),
    }
}

/// Parse the stdout of a `build_versions_command` invocation: split
/// on newlines, trim, drop empties, dedup while preserving first-seen
/// order so the manager's natural "freshest first" ordering survives.
fn parse_versions_output(stdout: &str) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let v = line.trim();
        if v.is_empty() {
            continue;
        }
        if seen.insert(v.to_string()) {
            out.push(v.to_string());
        }
    }
    out
}

/// Synthesise the package-manager command for install or update. The
/// returned string is the *inner* command — wrap it with `sh -c '...'`
/// + optional `sudo -n` prefix at the call site.
///
/// When `version` is `Some`, each package atom is rewritten per the
/// manager's pin syntax: `pkg=ver` for apt/apk/zypper, `pkg-ver` for
/// dnf/yum. pacman ignores `version` because Arch repos don't carry
/// historical versions; the panel hides the dropdown there.
fn build_install_command(
    manager: PackageManager,
    packages: &[&str],
    is_update: bool,
    version: Option<&str>,
) -> String {
    let pkgs = format_packages_with_version(manager, packages, version);
    match (manager, is_update) {
        (PackageManager::Apt, false) => format!(
            "DEBIAN_FRONTEND=noninteractive apt-get update -qq \
             && DEBIAN_FRONTEND=noninteractive apt-get install -y {pkgs}"
        ),
        (PackageManager::Apt, true) => format!(
            "DEBIAN_FRONTEND=noninteractive apt-get update -qq \
             && DEBIAN_FRONTEND=noninteractive apt-get install -y --only-upgrade {pkgs}"
        ),
        (PackageManager::Dnf, false) => format!("dnf install -y {pkgs}"),
        (PackageManager::Dnf, true) => format!("dnf upgrade -y {pkgs}"),
        (PackageManager::Yum, false) => format!("yum install -y {pkgs}"),
        (PackageManager::Yum, true) => format!("yum update -y {pkgs}"),
        (PackageManager::Apk, false) => format!("apk add --no-cache {pkgs}"),
        (PackageManager::Apk, true) => format!("apk add --no-cache --upgrade {pkgs}"),
        (PackageManager::Pacman, false) => format!("pacman -S --noconfirm {pkgs}"),
        (PackageManager::Pacman, true) => {
            format!("pacman -Syu --noconfirm {pkgs}")
        }
        (PackageManager::Zypper, false) => {
            format!("zypper --non-interactive install {pkgs}")
        }
        (PackageManager::Zypper, true) => {
            format!("zypper --non-interactive update {pkgs}")
        }
    }
}

/// Synthesise the package-manager command for an uninstall, with
/// optional service-disable prefix and optional data-dir wipe
/// suffix. Returned string is the *inner* command — wrap with
/// `sh -c '...'` + optional `sudo -n` prefix at the call site.
///
/// The shape of the chain is important and worth narrating:
///
/// * Service step: `command -v systemctl >/dev/null 2>&1 &&
///   systemctl disable --now <unit>` followed by `; ` (best-effort —
///   alpine has no systemd, the unit may already be stopped, etc.).
/// * Remove step: `&&`-chained from the service step's outer `;` so
///   it always runs.
/// * Autoremove step (when requested + manager supports a separate
///   pass): `&&`-chained from remove so a failed remove doesn't
///   trigger autoremove.
/// * Data-dir step (when requested + descriptor declares any):
///   `&&`-chained at the end so a failed remove never wipes user
///   data. Each path is single-quoted.
///
/// pacman's flag matrix is unique: `-R`, `-Rs` (autoremove), `-Rn`
/// (purge), `-Rns` (both). zypper folds autoremove into
/// `--clean-deps`. apk silently ignores both flags. dnf and yum
/// each get their own `autoremove` follow-up command.
fn build_uninstall_command(
    manager: PackageManager,
    packages: &[&str],
    data_dirs: &[&str],
    opts: &UninstallOptions,
    service_unit: Option<&str>,
) -> String {
    let pkgs = packages.join(" ");

    let service_step = match service_unit {
        Some(unit) => format!(
            "(command -v systemctl >/dev/null 2>&1 && systemctl disable --now {unit} 2>&1) \
             || echo '(systemctl disable {unit}: skipped or failed; continuing)'; "
        ),
        None => String::new(),
    };

    let remove_step = match (manager, opts.purge_config, opts.autoremove) {
        (PackageManager::Apt, true, _) => {
            format!("DEBIAN_FRONTEND=noninteractive apt-get purge -y {pkgs}")
        }
        (PackageManager::Apt, false, _) => {
            format!("DEBIAN_FRONTEND=noninteractive apt-get remove -y {pkgs}")
        }
        (PackageManager::Dnf, _, _) => format!("dnf remove -y {pkgs}"),
        (PackageManager::Yum, _, _) => format!("yum remove -y {pkgs}"),
        (PackageManager::Apk, _, _) => format!("apk del {pkgs}"),
        (PackageManager::Pacman, true, true) => {
            format!("pacman -Rns --noconfirm {pkgs}")
        }
        (PackageManager::Pacman, true, false) => {
            format!("pacman -Rn --noconfirm {pkgs}")
        }
        (PackageManager::Pacman, false, true) => {
            format!("pacman -Rs --noconfirm {pkgs}")
        }
        (PackageManager::Pacman, false, false) => {
            format!("pacman -R --noconfirm {pkgs}")
        }
        (PackageManager::Zypper, _, true) => {
            format!("zypper --non-interactive remove --clean-deps {pkgs}")
        }
        (PackageManager::Zypper, _, false) => {
            format!("zypper --non-interactive remove {pkgs}")
        }
    };

    let autoremove_step = if opts.autoremove {
        match manager {
            PackageManager::Apt => {
                Some("DEBIAN_FRONTEND=noninteractive apt-get autoremove -y".to_string())
            }
            PackageManager::Dnf => Some("dnf autoremove -y".to_string()),
            PackageManager::Yum => Some("yum autoremove -y".to_string()),
            // pacman folded into the remove flags above; zypper
            // folded into `--clean-deps`; apk has no equivalent.
            _ => None,
        }
    } else {
        None
    };

    let data_step = if opts.remove_data_dirs && !data_dirs.is_empty() {
        let quoted: Vec<String> =
            data_dirs.iter().map(|d| shell_single_quote(d)).collect();
        Some(format!("rm -rf {}", quoted.join(" ")))
    } else {
        None
    };

    let mut chain = remove_step;
    if let Some(s) = autoremove_step {
        chain.push_str(" && ");
        chain.push_str(&s);
    }
    if let Some(s) = data_step {
        chain.push_str(" && ");
        chain.push_str(&s);
    }

    format!("{service_step}{chain}")
}

/// Heuristic: is this output from `sudo -n` bailing for a password?
pub fn looks_like_sudo_password_prompt(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("a password is required")
        || lower.contains("sudo: a terminal is required")
        || lower.contains("no tty present")
        || (lower.contains("sudo:") && lower.contains("password"))
}

/// `systemctl is-active <unit>` → bool. Treats anything that isn't an
/// `active` reply as `false`, which matches what an end user means by
/// "service is up".
async fn systemctl_is_active(session: &SshSession, unit: &str) -> bool {
    let cmd = format!(
        "systemctl is-active {} 2>/dev/null || true",
        shell_single_quote(unit)
    );
    match session.exec_command(&cmd).await {
        Ok((_, stdout)) => stdout.trim() == "active",
        Err(_) => false,
    }
}

/// POSIX-safe single-quote escape so we can interpolate user-supplied
/// strings into `/bin/sh -c`.
fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Pull the version string out of probe output. We just take the first
/// dotted token on the first non-empty line — it's good enough for
/// `sqlite3 --version` (`3.46.1 ...`), `docker --version` (`Docker
/// version 27.5.1, build ...`), `nginx -v` (`nginx version: nginx/1.24.0`),
/// and `psql --version` (`psql (PostgreSQL) 16.4`). When it can't find
/// one we hand back `None` and the UI shows just "已安装".
pub fn parse_version(output: &str) -> Option<String> {
    for line in output.lines() {
        for token in line.split(|c: char| c.is_whitespace() || c == '/' || c == ',' || c == '(' || c == ')') {
            if token.contains('.') && token.chars().next()?.is_ascii_digit() {
                return Some(token.trim_end_matches('.').to_string());
            }
        }
    }
    None
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_v1_software() {
        let ids: Vec<&str> = registry().iter().map(|d| d.id).collect();
        for required in [
            "sqlite3", "docker", "compose", "redis", "postgres", "mariadb",
            "nginx", "jq", "curl",
        ] {
            assert!(ids.contains(&required), "registry missing {required}");
        }
    }

    #[test]
    fn registry_covers_every_manager_for_every_descriptor() {
        // If we add a manager but forget a row, the install button has
        // to fall back to UnsupportedDistro for that combo. Catch it
        // here so the panel never silently disables a button.
        for d in registry() {
            for m in [
                PackageManager::Apt,
                PackageManager::Dnf,
                PackageManager::Yum,
                PackageManager::Apk,
                PackageManager::Pacman,
                PackageManager::Zypper,
            ] {
                assert!(
                    packages_for(d, m).is_some(),
                    "{} has no install command for {:?}",
                    d.id,
                    m,
                );
            }
        }
    }

    #[test]
    fn pick_package_manager_known_distros() {
        assert_eq!(pick_package_manager("ubuntu"), Some(PackageManager::Apt));
        assert_eq!(pick_package_manager("debian"), Some(PackageManager::Apt));
        assert_eq!(pick_package_manager("alpine"), Some(PackageManager::Apk));
        assert_eq!(pick_package_manager("fedora"), Some(PackageManager::Dnf));
        assert_eq!(pick_package_manager("centos"), Some(PackageManager::Dnf));
        assert_eq!(pick_package_manager("arch"), Some(PackageManager::Pacman));
        assert_eq!(
            pick_package_manager("opensuse-leap"),
            Some(PackageManager::Zypper),
        );
    }

    #[test]
    fn pick_package_manager_unknown_returns_none() {
        assert!(pick_package_manager("solaris").is_none());
        assert!(pick_package_manager("").is_none());
    }

    #[test]
    fn build_install_command_apt_install_is_noninteractive() {
        let cmd = build_install_command(PackageManager::Apt, &["sqlite3"], false, None);
        assert!(cmd.contains("DEBIAN_FRONTEND=noninteractive"));
        assert!(cmd.contains("apt-get install -y sqlite3"));
    }

    #[test]
    fn build_install_command_apt_upgrade_uses_only_upgrade() {
        let cmd = build_install_command(PackageManager::Apt, &["redis-server"], true, None);
        assert!(cmd.contains("--only-upgrade"));
        assert!(cmd.contains("redis-server"));
    }

    #[test]
    fn build_install_command_alpine_uses_apk() {
        let cmd = build_install_command(PackageManager::Apk, &["sqlite"], false, None);
        assert_eq!(cmd, "apk add --no-cache sqlite");
    }

    // ── Version-pinned install commands ─────────────────────────

    #[test]
    fn build_install_command_apt_version_pin_uses_equals() {
        let cmd = build_install_command(
            PackageManager::Apt,
            &["docker.io"],
            false,
            Some("27.5.1-0ubuntu1"),
        );
        assert!(cmd.contains("apt-get install -y docker.io=27.5.1-0ubuntu1"));
    }

    #[test]
    fn build_install_command_dnf_version_pin_uses_dash() {
        let cmd =
            build_install_command(PackageManager::Dnf, &["docker"], false, Some("27.5.1-1.fc40"));
        assert!(cmd.contains("dnf install -y docker-27.5.1-1.fc40"));
    }

    #[test]
    fn build_install_command_yum_version_pin_uses_dash() {
        let cmd = build_install_command(PackageManager::Yum, &["redis"], false, Some("7.2.4-1"));
        assert!(cmd.contains("yum install -y redis-7.2.4-1"));
    }

    #[test]
    fn build_install_command_apk_version_pin_uses_equals() {
        let cmd =
            build_install_command(PackageManager::Apk, &["sqlite"], false, Some("3.46.1-r0"));
        assert_eq!(cmd, "apk add --no-cache sqlite=3.46.1-r0");
    }

    #[test]
    fn build_install_command_zypper_version_pin_uses_equals() {
        let cmd = build_install_command(
            PackageManager::Zypper,
            &["redis"],
            false,
            Some("7.0.4-1.1"),
        );
        assert!(cmd.contains("zypper --non-interactive install redis=7.0.4-1.1"));
    }

    #[test]
    fn build_install_command_pacman_ignores_version_pin() {
        // Arch's standard repos don't carry historical versions. The
        // panel hides the dropdown, but defence-in-depth: even if
        // `version=Some(...)` slips through, the command still runs.
        let cmd = build_install_command(
            PackageManager::Pacman,
            &["redis"],
            false,
            Some("7.2.4-1"),
        );
        assert!(cmd.contains("pacman -S --noconfirm redis"));
        assert!(!cmd.contains("7.2.4-1"));
    }

    #[test]
    fn build_install_command_apt_update_with_version_keeps_upgrade_flag() {
        let cmd = build_install_command(
            PackageManager::Apt,
            &["docker.io"],
            true,
            Some("27.5.1-0ubuntu1"),
        );
        assert!(cmd.contains("--only-upgrade"));
        assert!(cmd.contains("docker.io=27.5.1-0ubuntu1"));
    }

    #[test]
    fn build_install_command_blank_version_falls_back_to_unpinned() {
        let cmd =
            build_install_command(PackageManager::Apt, &["docker.io"], false, Some("   "));
        assert!(cmd.contains("apt-get install -y docker.io"));
        assert!(!cmd.contains("docker.io="));
    }

    // ── Versions probe builder + parser ─────────────────────────

    #[test]
    fn build_versions_command_per_manager() {
        assert!(
            build_versions_command(PackageManager::Apt, "docker.io")
                .unwrap()
                .contains("apt-cache madison")
        );
        assert!(
            build_versions_command(PackageManager::Dnf, "docker")
                .unwrap()
                .contains("dnf list available")
        );
        assert!(
            build_versions_command(PackageManager::Yum, "redis")
                .unwrap()
                .contains("yum list available")
        );
        assert!(
            build_versions_command(PackageManager::Apk, "sqlite")
                .unwrap()
                .contains("apk version -a")
        );
        assert!(
            build_versions_command(PackageManager::Zypper, "redis")
                .unwrap()
                .contains("zypper search -s")
        );
        // pacman has no historical-version query; surface as None so
        // the frontend can hide the dropdown.
        assert!(build_versions_command(PackageManager::Pacman, "redis").is_none());
    }

    #[test]
    fn build_versions_command_quotes_package_name() {
        // Defence-in-depth: even though descriptor ids come from a
        // hardcoded registry, the package name flows into a shell
        // command so single-quote it.
        let cmd = build_versions_command(PackageManager::Apt, "evil; rm -rf /").unwrap();
        assert!(cmd.contains("'evil; rm -rf /'"));
    }

    #[test]
    fn parse_versions_output_dedups_and_preserves_order() {
        let raw = "27.5.1-0ubuntu1\n27.5.1-0ubuntu1\n26.1.4-0ubuntu1\n   \n26.0.0-0ubuntu1\n";
        let parsed = parse_versions_output(raw);
        assert_eq!(
            parsed,
            vec![
                "27.5.1-0ubuntu1".to_string(),
                "26.1.4-0ubuntu1".to_string(),
                "26.0.0-0ubuntu1".to_string(),
            ],
        );
    }

    #[test]
    fn parse_versions_output_empty_returns_empty() {
        assert!(parse_versions_output("").is_empty());
        assert!(parse_versions_output("\n\n   \n").is_empty());
    }

    #[test]
    fn parse_version_handles_common_formats() {
        assert_eq!(
            parse_version("3.46.1 2024-08-13 ceb..."),
            Some("3.46.1".to_string()),
        );
        assert_eq!(
            parse_version("Docker version 27.5.1, build ..."),
            Some("27.5.1".to_string()),
        );
        assert_eq!(
            parse_version("nginx version: nginx/1.24.0"),
            Some("1.24.0".to_string()),
        );
        assert_eq!(
            parse_version("psql (PostgreSQL) 16.4"),
            Some("16.4".to_string()),
        );
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("garbage"), None);
    }

    #[test]
    fn strip_os_release_quotes_handles_double_and_single() {
        assert_eq!(strip_os_release_quotes("\"ubuntu\""), "ubuntu");
        assert_eq!(strip_os_release_quotes("'ubuntu'"), "ubuntu");
        assert_eq!(strip_os_release_quotes("ubuntu"), "ubuntu");
        assert_eq!(strip_os_release_quotes(" debian "), "debian");
    }

    #[test]
    fn shell_single_quote_escapes_internal_quotes() {
        assert_eq!(shell_single_quote("Tom's"), "'Tom'\\''s'");
        assert_eq!(shell_single_quote(""), "''");
    }

    #[test]
    fn looks_like_sudo_password_prompt_recognises_common_messages() {
        assert!(looks_like_sudo_password_prompt(
            "sudo: a password is required"
        ));
        assert!(looks_like_sudo_password_prompt(
            "sudo: a terminal is required to read the password"
        ));
        assert!(!looks_like_sudo_password_prompt(
            "E: Unable to locate package sqlite3"
        ));
    }

    #[test]
    fn descriptor_lookup_finds_known_id() {
        assert!(descriptor("docker").is_some());
        assert!(descriptor("nope").is_none());
    }

    #[test]
    fn package_manager_as_str_is_lowercase() {
        assert_eq!(PackageManager::Apt.as_str(), "apt");
        assert_eq!(PackageManager::Pacman.as_str(), "pacman");
    }

    // ── Uninstall command builder ────────────────────────────

    #[test]
    fn uninstall_apt_remove_vs_purge() {
        let plain = build_uninstall_command(
            PackageManager::Apt,
            &["redis-server"],
            &[],
            &UninstallOptions::default(),
            None,
        );
        assert!(plain.contains("apt-get remove -y redis-server"));
        assert!(!plain.contains("purge"));

        let purge = build_uninstall_command(
            PackageManager::Apt,
            &["redis-server"],
            &[],
            &UninstallOptions {
                purge_config: true,
                ..Default::default()
            },
            None,
        );
        assert!(purge.contains("apt-get purge -y redis-server"));
    }

    #[test]
    fn uninstall_apt_appends_autoremove_only_when_requested() {
        let with = build_uninstall_command(
            PackageManager::Apt,
            &["redis-server"],
            &[],
            &UninstallOptions {
                autoremove: true,
                ..Default::default()
            },
            None,
        );
        assert!(with.contains("apt-get autoremove -y"));

        let without = build_uninstall_command(
            PackageManager::Apt,
            &["redis-server"],
            &[],
            &UninstallOptions::default(),
            None,
        );
        assert!(!without.contains("autoremove"));
    }

    #[test]
    fn uninstall_dnf_yum_each_get_native_autoremove() {
        let dnf = build_uninstall_command(
            PackageManager::Dnf,
            &["redis"],
            &[],
            &UninstallOptions {
                autoremove: true,
                ..Default::default()
            },
            None,
        );
        assert!(dnf.contains("dnf remove -y redis"));
        assert!(dnf.contains("dnf autoremove -y"));

        let yum = build_uninstall_command(
            PackageManager::Yum,
            &["redis"],
            &[],
            &UninstallOptions {
                autoremove: true,
                ..Default::default()
            },
            None,
        );
        assert!(yum.contains("yum remove -y redis"));
        assert!(yum.contains("yum autoremove -y"));
    }

    #[test]
    fn uninstall_pacman_flag_matrix() {
        let none = build_uninstall_command(
            PackageManager::Pacman,
            &["redis"],
            &[],
            &UninstallOptions::default(),
            None,
        );
        assert!(none.contains("pacman -R --noconfirm redis"));

        let purge_only = build_uninstall_command(
            PackageManager::Pacman,
            &["redis"],
            &[],
            &UninstallOptions {
                purge_config: true,
                ..Default::default()
            },
            None,
        );
        assert!(purge_only.contains("pacman -Rn --noconfirm redis"));

        let auto_only = build_uninstall_command(
            PackageManager::Pacman,
            &["redis"],
            &[],
            &UninstallOptions {
                autoremove: true,
                ..Default::default()
            },
            None,
        );
        assert!(auto_only.contains("pacman -Rs --noconfirm redis"));

        let both = build_uninstall_command(
            PackageManager::Pacman,
            &["redis"],
            &[],
            &UninstallOptions {
                purge_config: true,
                autoremove: true,
                ..Default::default()
            },
            None,
        );
        assert!(both.contains("pacman -Rns --noconfirm redis"));
    }

    #[test]
    fn uninstall_zypper_clean_deps_when_autoremove() {
        let with = build_uninstall_command(
            PackageManager::Zypper,
            &["redis"],
            &[],
            &UninstallOptions {
                autoremove: true,
                ..Default::default()
            },
            None,
        );
        assert!(with.contains("--clean-deps"));

        let without = build_uninstall_command(
            PackageManager::Zypper,
            &["redis"],
            &[],
            &UninstallOptions::default(),
            None,
        );
        assert!(!without.contains("--clean-deps"));
    }

    #[test]
    fn uninstall_apk_ignores_unsupported_flags() {
        let s = build_uninstall_command(
            PackageManager::Apk,
            &["redis"],
            &[],
            &UninstallOptions {
                purge_config: true,
                autoremove: true,
                ..Default::default()
            },
            None,
        );
        assert_eq!(s, "apk del redis");
    }

    #[test]
    fn uninstall_data_dirs_only_when_requested_and_present() {
        let dirs = &["/var/lib/docker", "/var/lib/containerd"];

        let without = build_uninstall_command(
            PackageManager::Apt,
            &["docker.io"],
            dirs,
            &UninstallOptions::default(),
            None,
        );
        assert!(!without.contains("rm -rf"));

        let with = build_uninstall_command(
            PackageManager::Apt,
            &["docker.io"],
            dirs,
            &UninstallOptions {
                remove_data_dirs: true,
                ..Default::default()
            },
            None,
        );
        assert!(with.contains("rm -rf"));
        assert!(with.contains("/var/lib/docker"));
        assert!(with.contains("/var/lib/containerd"));

        // Empty data_dirs slice: flag is silently ignored.
        let with_empty = build_uninstall_command(
            PackageManager::Apt,
            &["htop"],
            &[],
            &UninstallOptions {
                remove_data_dirs: true,
                ..Default::default()
            },
            None,
        );
        assert!(!with_empty.contains("rm -rf"));
    }

    #[test]
    fn uninstall_service_step_is_best_effort_then_chain() {
        let s = build_uninstall_command(
            PackageManager::Apt,
            &["redis-server"],
            &["/var/lib/redis"],
            &UninstallOptions {
                autoremove: true,
                remove_data_dirs: true,
                ..Default::default()
            },
            Some("redis-server"),
        );
        let svc_pos = s.find("(command -v systemctl").expect("service step");
        let remove_pos = s.find("apt-get remove").expect("remove step");
        let auto_pos = s.find("apt-get autoremove").expect("autoremove step");
        let rm_pos = s.find("rm -rf").expect("data step");
        assert!(svc_pos < remove_pos);
        assert!(remove_pos < auto_pos);
        assert!(auto_pos < rm_pos);
        // Service step ends with `; ` so a failed disable doesn't
        // halt the chain. Subsequent steps `&&` so the data wipe
        // never runs after a failed remove.
        let between_svc_and_remove = &s[svc_pos..remove_pos];
        assert!(between_svc_and_remove.contains("; "));
        let between_remove_and_auto = &s[remove_pos..auto_pos];
        assert!(between_remove_and_auto.contains(" && "));
        let between_auto_and_rm = &s[auto_pos..rm_pos];
        assert!(between_auto_and_rm.contains(" && "));
    }

    // ── Service control + log builders ──────────────────────

    #[test]
    fn build_systemctl_command_root_omits_sudo() {
        let cmd = build_systemctl_command(ServiceAction::Restart, "redis-server", true);
        assert_eq!(cmd, "systemctl restart 'redis-server' 2>&1");
        assert!(!cmd.contains("sudo"));
    }

    #[test]
    fn build_systemctl_command_non_root_uses_sudo_n() {
        let cmd = build_systemctl_command(ServiceAction::Stop, "redis", false);
        assert!(cmd.starts_with("sudo -n systemctl stop "));
        assert!(cmd.contains("'redis'"));
        assert!(cmd.ends_with("2>&1"));
    }

    #[test]
    fn build_systemctl_command_quotes_unit() {
        // Defensive: even though no v1 unit has metacharacters, the
        // unit string is data — keep the escape in place.
        let cmd = build_systemctl_command(ServiceAction::Start, "weird unit", true);
        assert!(cmd.contains("'weird unit'"));
    }

    #[test]
    fn build_systemctl_command_each_action_emits_correct_verb() {
        for (action, verb) in [
            (ServiceAction::Start, "start"),
            (ServiceAction::Stop, "stop"),
            (ServiceAction::Restart, "restart"),
            (ServiceAction::Reload, "reload"),
        ] {
            let cmd = build_systemctl_command(action, "redis", true);
            assert!(
                cmd.contains(&format!("systemctl {verb} ")),
                "{action:?} → expected verb {verb} in {cmd}",
            );
        }
    }

    #[test]
    fn descriptor_service_unit_resolves_per_manager() {
        let redis = descriptor("redis").unwrap();
        // redis on apt is "redis-server"; on dnf / yum / apk / pacman / zypper it's "redis".
        assert_eq!(
            descriptor_service_unit(redis, PackageManager::Apt),
            Some("redis-server"),
        );
        assert_eq!(
            descriptor_service_unit(redis, PackageManager::Dnf),
            Some("redis"),
        );
        assert_eq!(
            descriptor_service_unit(redis, PackageManager::Pacman),
            Some("redis"),
        );

        // sqlite has no service.
        let sqlite = descriptor("sqlite3").unwrap();
        assert!(
            descriptor_service_unit(sqlite, PackageManager::Apt).is_none(),
        );
    }

    #[test]
    fn build_journalctl_command_root_no_sudo() {
        let cmd = build_journalctl_command("redis-server", 200, true);
        assert_eq!(
            cmd,
            "journalctl -u 'redis-server' -n 200 --no-pager 2>&1",
        );
    }

    #[test]
    fn build_journalctl_command_non_root_uses_sudo_n() {
        let cmd = build_journalctl_command("nginx", 50, false);
        assert!(cmd.starts_with("sudo -n journalctl -u 'nginx' -n 50 "));
        assert!(cmd.contains("--no-pager"));
        assert!(cmd.ends_with("2>&1"));
    }

    #[test]
    fn build_journalctl_command_includes_lines_argument() {
        let cmd = build_journalctl_command("redis", 1, true);
        assert!(cmd.contains("-n 1 "));
    }

    #[test]
    fn service_action_as_systemctl_verb_stable() {
        // These strings are wire-visible (report.action) — pin them so
        // a refactor doesn't silently break the panel's outcome strings.
        assert_eq!(ServiceAction::Start.as_systemctl_verb(), "start");
        assert_eq!(ServiceAction::Stop.as_systemctl_verb(), "stop");
        assert_eq!(ServiceAction::Restart.as_systemctl_verb(), "restart");
        assert_eq!(ServiceAction::Reload.as_systemctl_verb(), "reload");
    }

    #[test]
    fn supports_reload_set_only_for_nginx_in_v1_registry() {
        // Reload semantics are software-specific — most daemons we
        // ship would effectively restart on `reload`. nginx is the
        // one v1 entry that genuinely supports zero-downtime reload.
        for d in registry() {
            let expected = d.id == "nginx";
            assert_eq!(
                d.supports_reload, expected,
                "{} supports_reload should be {}",
                d.id, expected,
            );
        }
    }

    #[test]
    fn uninstall_no_service_step_for_serviceless() {
        let s = build_uninstall_command(
            PackageManager::Apt,
            &["htop"],
            &[],
            &UninstallOptions::default(),
            None,
        );
        assert!(!s.contains("systemctl"));
    }
}
