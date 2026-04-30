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
use tokio_util::sync::CancellationToken;

use crate::ssh::error::{Result, SshError};
use crate::ssh::session::CANCELLED_EXIT_CODE;
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
    /// Optional vendor-supplied install script (v2). When `Some`, the
    /// panel offers a second install path next to the default apt /
    /// dnf / … one — typically used to pick up a version much newer
    /// than the distro repos carry (Docker is the canonical example).
    /// `None` = only the default package-manager path is offered.
    pub vendor_script: Option<VendorScriptDescriptor>,
    /// Short binary name used by the details probe (`command -v {name}`
    /// → resolved path → `readlink -f`). Empty = derive from the first
    /// install package on the matching manager (works for jq/curl/git
    /// where binary == package name).
    pub binary_name: &'static str,
    /// Common config-file / config-dir paths we surface in the details
    /// pane. Each path is `test -e`'d at probe time so we never show a
    /// stale entry. Empty for stateless software (jq, curl, …).
    pub config_paths: &'static [&'static str],
    /// Default ports this software listens on when running with stock
    /// config (e.g. nginx → 80/443, postgres → 5432). Surfaced in the
    /// details pane as "default" alongside an `ss -ltn` probe of which
    /// of those are actually open. Empty for non-network software.
    pub default_ports: &'static [u16],
    /// Major-version variants (OpenJDK 8/11/17/21, Python 3.10/11/12).
    /// Empty for single-version software — install/update use the
    /// descriptor's `install_packages` directly. When non-empty the UI
    /// shows a variant picker and the install command uses the picked
    /// variant's `install_packages` instead of the descriptor's.
    pub version_variants: &'static [VersionVariant],
    /// Coarse-grained app-store category. Drives the panel's
    /// section grouping. Stable strings — UI maps them to localized
    /// labels via the i18n table. Empty string = "其它".
    pub category: &'static str,
}

/// One installable major version of a multi-version software (e.g.
/// `openjdk-21` for Java). The frontend shows a cascading picker on
/// descriptors that declare any. Variants are mutually exclusive at
/// install time (the UI surfaces a single picker, not checkboxes).
#[derive(Debug, Clone, Copy)]
pub struct VersionVariant {
    /// Stable variant id passed across the IPC boundary
    /// (e.g. `"openjdk-21"`). Lowercase, dash-separated.
    pub key: &'static str,
    /// Human label rendered in the picker (e.g. `"OpenJDK 21"`).
    pub label: &'static str,
    /// Per package-manager package list — same shape as the
    /// descriptor's `install_packages`. The frontend uses the
    /// resolved-manager row to display "via apt: openjdk-21-jdk".
    pub install_packages: &'static [(PackageManager, &'static [&'static str])],
    /// Optional override for the descriptor's `probe_command`. Use
    /// when the variant has its own canonical binary path
    /// (e.g. `/usr/lib/jvm/java-21-openjdk-amd64/bin/java -version`).
    /// `None` = use descriptor's probe (which may detect any variant).
    pub probe_command: Option<&'static str>,
}

/// Static description of an "official upstream installer" we know how
/// to invoke via the v2 vendor-script channel.
///
/// **The URL is hard-coded in the registry** — the frontend never
/// passes a URL into the install command, and there is no way for a
/// user to point the channel at an arbitrary script. Adding a new
/// vendor source requires landing a registry change. This is the
/// security boundary for the channel.
#[derive(Debug, Clone, Copy)]
pub struct VendorScriptDescriptor {
    /// Short label rendered in the install dropdown and the post-run
    /// "via {label}" log line (e.g. `"Docker 官方脚本"`).
    pub label: &'static str,
    /// Fully-qualified `https://` URL of the installer script. **Must**
    /// be a static literal so the URL set is auditable from the
    /// registry alone.
    ///
    /// Used as the fallback when [`urls`] is empty or doesn't list
    /// the host's package manager. Distros where the script needs a
    /// per-family URL (NodeSource ships separate `deb.nodesource.com`
    /// vs `rpm.nodesource.com`) populate `urls` with the matrix.
    pub url: &'static str,
    /// Optional per-package-manager URL override. When set, the
    /// install path picks the URL whose `PackageManager` matches the
    /// host's resolved manager and falls back to `url` otherwise.
    /// Empty = always use `url`.
    pub urls: &'static [(PackageManager, &'static str)],
    /// Multi-step pre-install setup snippets keyed by package manager.
    /// When the matching entry is set, [`run_install_via_script`]
    /// runs **this snippet first** (under `sudo -n sh -c '...'`),
    /// then falls through to the normal package-manager install path
    /// (`apt-get install <descriptor.install_packages>`) instead of
    /// the curl→sh path.
    ///
    /// Use this for upstream sources whose setup is more than one
    /// command — e.g. PostgreSQL's pgdg apt repo: install GPG key,
    /// write `/etc/apt/sources.list.d/pgdg.list`, `apt-get update`.
    /// `urls` and `setup_scripts` are mutually exclusive at runtime;
    /// when both are populated for the resolved manager, setup wins.
    pub setup_scripts: &'static [(PackageManager, &'static str)],
    /// Reverse of [`setup_scripts`] — undoes the upstream source
    /// install so the host falls back to distro packages. Run when
    /// the user ticks "also remove upstream source" in the uninstall
    /// dialog. The dialog only surfaces that checkbox when the
    /// resolved manager has an entry here (otherwise: no-op).
    ///
    /// For PostgreSQL pgdg: removes
    /// `/etc/apt/sources.list.d/pgdg.list` + the imported GPG key
    /// (apt) or `pgdg-redhat-repo` package (dnf/yum), then
    /// `apt-get update` / `dnf clean all` to refresh metadata.
    pub cleanup_scripts: &'static [(PackageManager, &'static str)],
    /// Most upstream installers (get.docker.com, NodeSource, etc.)
    /// expect to be run as root because they `apt-get install` /
    /// `systemctl enable` themselves. When `true` and the session
    /// isn't root, we prefix the script invocation with `sudo -n `.
    pub run_as_root: bool,
    /// Free-form risk-disclosure text rendered in the confirmation
    /// dialog (e.g. "由 Docker, Inc. 维护…").
    pub notes: &'static str,
    /// `true` when installing via this script will conflict with the
    /// distro-package version of the same software (`docker.io` vs
    /// the upstream `docker-ce` package, classically). The default
    /// apt path stays available, but the dialog warns the user to
    /// uninstall the distro package first.
    pub conflicts_with_apt: bool,
}

/// Canonical package-manager IDs. Stable strings exposed to the UI via
/// `as_str` — keep them short and lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// Details surfaced in the per-row "expand" pane. Loaded lazily — the
/// row only fetches this when the user clicks the disclosure, so the
/// panel's first paint stays at one round-trip per package (the
/// existing [`probe_status`]) and we never block the panel header on
/// `apt-cache policy` / `dpkg -L`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageDetail {
    /// Echoes the descriptor id.
    pub package_id: String,
    /// Re-probed installed flag. Independent of any prior cached
    /// `PackageStatus` — the user may have apt-removed between the
    /// first probe and the click that opens the details pane.
    pub installed: bool,
    /// Resolved binary paths from `command -v {bin}` + `readlink -f`.
    /// Single entry for the common case; multiple when the descriptor
    /// has aliases (none today, but the field is `Vec` for forward
    /// compatibility with multi-binary packages like jdk → java/javac).
    pub install_paths: Vec<String>,
    /// Config paths from the descriptor that exist on the remote
    /// (filtered through `test -e`). Stale entries never reach the UI.
    pub config_paths: Vec<String>,
    /// Default ports declared on the descriptor — surfaced as "default"
    /// in the UI even when nothing is actually listening.
    pub default_ports: Vec<u16>,
    /// Subset of `default_ports` that an `ss -ltn` probe found bound on
    /// the host. Empty when `ss` isn't installed or the probe failed —
    /// distinguish from "nothing listening" via the side-channel
    /// `listen_probe_ok`.
    pub listening_ports: Vec<u16>,
    /// `true` when the `ss -ltn` probe ran successfully (regardless of
    /// whether anything was listening). `false` = the probe failed and
    /// `listening_ports` is unreliable; the UI hides the "live ports"
    /// row in that case.
    pub listen_probe_ok: bool,
    /// Service unit name resolved against the host's package manager,
    /// or `None` when the descriptor has no service.
    pub service_unit: Option<String>,
    /// Latest installable version from the package manager's "candidate"
    /// query (apt-cache policy / dnf info / apk policy / pacman -Si /
    /// zypper info). `None` on unsupported distro or when the query
    /// returned nothing parseable.
    pub latest_version: Option<String>,
    /// Currently-installed version (re-probed). Same value the panel
    /// header shows after probe_all; surfaced here so a stale row
    /// snapshot can't disagree with the details pane.
    pub installed_version: Option<String>,
    /// Per-variant install state when the descriptor declares
    /// `version_variants`. Empty for single-version software.
    pub variants: Vec<PackageVariantStatus>,
}

/// Per-variant install state for descriptors with `version_variants`.
/// One entry per declared variant, same order as the descriptor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageVariantStatus {
    /// Same as `VersionVariant::key`.
    pub key: String,
    /// Same as `VersionVariant::label`.
    pub label: String,
    /// `true` when this specific variant's probe succeeded.
    pub installed: bool,
    /// Version reported by the variant's probe; `None` when not
    /// installed or the probe didn't yield a version token.
    pub installed_version: Option<String>,
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
    /// The caller-supplied [`CancellationToken`] fired while the package
    /// manager was still running. We bail out without re-probing — the
    /// remote process may still be alive (apt/dpkg lock, half-staged
    /// packages); see PRODUCT-SPEC §5.11 v2 for the user-facing
    /// caveat.
    Cancelled,
    /// Vendor-script path: download of the installer (`curl -fsSL …`)
    /// failed — typically a DNS / network / TLS error or HTTP 404.
    /// Distinct from `PackageManagerFailed` so the UI can say "网络下
    /// 载失败" instead of pointing the finger at apt.
    VendorScriptDownloadFailed,
    /// Vendor-script path: download succeeded but executing the
    /// script exited non-zero, **and** the post-run probe still can't
    /// see the binary on PATH. Mirrors `PackageManagerFailed` but on
    /// the script channel.
    VendorScriptFailed,
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
    /// Run the descriptor's `vendor_script.cleanup_scripts` snippet
    /// for the host's manager after the uninstall succeeds. Drops
    /// upstream sources files/repos (e.g. PostgreSQL pgdg). No-op
    /// when the descriptor has no cleanup snippet for the manager.
    #[serde(default)]
    pub remove_upstream_source: bool,
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
    /// The caller-supplied [`CancellationToken`] fired while the
    /// package manager was still running. Same caveat as
    /// [`InstallStatus::Cancelled`].
    Cancelled,
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
    /// `Some(label)` when the install ran via the v2 vendor-script
    /// channel. The label is the same string the user picked from the
    /// install dropdown (e.g. `"Docker 官方脚本"`); the frontend uses
    /// it to append a `via {label} ({url})` line to the activity log
    /// once the report arrives. `None` for the default apt / dnf / …
    /// path.
    pub vendor_script: Option<VendorScriptUsedView>,
    /// Stale / unreachable third-party repos detected during install.
    /// Populated by [`detect_broken_repo_warnings`] from the merged
    /// install output. **Empty when nothing was flagged**, so the
    /// frontend can `len() > 0` to decide whether to render the
    /// "host has stale APT/DNF sources" advisory banner. Skipped on
    /// the wire when empty so legacy clients keep parsing the report.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repo_warnings: Vec<String>,
}

/// Echo-back of the vendor script that produced an install — kept on
/// the report so the frontend can render `via {label} ({url})` after
/// the run completes without re-reading the registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VendorScriptUsedView {
    /// Same string as the descriptor's `label` (e.g. `"Docker 官方脚本"`).
    pub label: String,
    /// Same string as the descriptor's `url` (e.g. `"https://get.docker.com"`).
    pub url: String,
}

// ── Registry ────────────────────────────────────────────────────────

/// Software list rendered in the panel. Built lazily — the first
/// call merges built-in REGISTRY with user-extras (when src-tauri
/// has set [`set_user_extras_path`]); subsequent calls return the
/// memoized slice.
pub fn registry() -> &'static [PackageDescriptor] {
    merged_registry()
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
        vendor_script: None,
        binary_name: "sqlite3",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "database",
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
        supports_reload: false,
        notes: Some("发行版仓库的 Docker 版本可能旧；可选择 Docker 官方脚本安装最新稳定版。"),
        vendor_script: Some(VendorScriptDescriptor {
            label: "Docker 官方脚本",
            url: "https://get.docker.com",
            urls: &[],
            setup_scripts: &[],
            cleanup_scripts: &[],
            run_as_root: true,
            notes: "由 Docker, Inc. 维护，会自动添加上游 apt/yum 仓库并安装 docker-ce 最新稳定版。Pier-X 不会校验脚本签名，请确认网络通路可信后再继续。",
            conflicts_with_apt: true,
        }),
        binary_name: "docker",
        config_paths: &["/etc/docker/daemon.json"],
        default_ports: &[],
        version_variants: &[],
        category: "container",
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
        vendor_script: None,
        binary_name: "docker",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "container",
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
        vendor_script: None,
        binary_name: "redis-server",
        config_paths: &["/etc/redis/redis.conf", "/etc/redis.conf"],
        default_ports: &[6379],
        version_variants: &[],
        category: "database",
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
        notes: Some("发行版仓库的 PostgreSQL 通常滞后；可选 PostgreSQL 官方源安装最新主线版本。"),
        supports_reload: false,
        vendor_script: Some(VendorScriptDescriptor {
            label: "PostgreSQL 官方源",
            // url is a fallback marker only — setup_scripts handles
            // the real action below. Keep the URL pointing at the
            // canonical setup-doc page so the audit trail still has
            // a meaningful "where this came from" record.
            url: "https://www.postgresql.org/download/",
            urls: &[],
            // Multi-step setup: import the GPG key, write the
            // sources file, refresh metadata. After this snippet
            // returns 0, run_install_via_script falls through to
            // the descriptor's install_packages with the new repo
            // in place.
            setup_scripts: &[
                (
                    PackageManager::Apt,
                    "set -e; \
                     install -d -m 0755 /usr/share/postgresql-common/pgdg; \
                     curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc \
                       -o /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc; \
                     codename=$(. /etc/os-release && echo \"$VERSION_CODENAME\"); \
                     [ -n \"$codename\" ] || codename=$(lsb_release -cs 2>/dev/null); \
                     [ -n \"$codename\" ] || { echo 'cannot detect ubuntu/debian codename'; exit 1; }; \
                     echo \"deb [signed-by=/usr/share/postgresql-common/pgdg/apt.postgresql.org.asc] \
                       https://apt.postgresql.org/pub/repos/apt $codename-pgdg main\" \
                       > /etc/apt/sources.list.d/pgdg.list; \
                     apt-get update -qq",
                ),
                (
                    PackageManager::Dnf,
                    "set -e; \
                     ver=$(rpm -E %rhel 2>/dev/null); \
                     [ -n \"$ver\" ] || { echo 'cannot detect rhel version (rpm -E %rhel)'; exit 1; }; \
                     dnf install -y \"https://download.postgresql.org/pub/repos/yum/reporpms/EL-${ver}-x86_64/pgdg-redhat-repo-latest.noarch.rpm\"; \
                     dnf -qy module disable postgresql || true",
                ),
                (
                    PackageManager::Yum,
                    "set -e; \
                     ver=$(rpm -E %rhel 2>/dev/null); \
                     [ -n \"$ver\" ] || { echo 'cannot detect rhel version'; exit 1; }; \
                     yum install -y \"https://download.postgresql.org/pub/repos/yum/reporpms/EL-${ver}-x86_64/pgdg-redhat-repo-latest.noarch.rpm\"",
                ),
            ],
            // Reverse the setup. Each line is best-effort so a
            // partially-set-up host (e.g. user removed the .list
            // by hand) doesn't cause cleanup to bail out half-way.
            cleanup_scripts: &[
                (
                    PackageManager::Apt,
                    "rm -f /etc/apt/sources.list.d/pgdg.list \
                       /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc \
                       2>/dev/null || true; \
                     apt-get update -qq 2>/dev/null || true; \
                     echo 'pgdg apt source removed'",
                ),
                (
                    PackageManager::Dnf,
                    "dnf remove -y pgdg-redhat-repo 2>/dev/null || true; \
                     dnf clean all 2>/dev/null || true; \
                     echo 'pgdg dnf source removed'",
                ),
                (
                    PackageManager::Yum,
                    "yum remove -y pgdg-redhat-repo 2>/dev/null || true; \
                     yum clean all 2>/dev/null || true; \
                     echo 'pgdg yum source removed'",
                ),
            ],
            run_as_root: true,
            notes: "由 PostgreSQL Global Development Group 维护。会写入 /etc/apt/sources.list.d/pgdg.list（apt）或安装 pgdg 源 RPM（dnf/yum），然后从官方源安装 postgresql。pgdg 不覆盖 pacman / apk / zypper（这些发行版的仓库通常已经较新）。Pier-X 不会校验脚本签名，请确认网络通路可信后再继续。",
            conflicts_with_apt: true,
        }),
        binary_name: "psql",
        config_paths: &[
            "/etc/postgresql",
            "/var/lib/pgsql/data/postgresql.conf",
        ],
        default_ports: &[5432],
        // pgdg ships parallel `postgresql-16` / `postgresql-17` /
        // etc. packages — pick a variant only after using the
        // PostgreSQL 官方源 channel; on a stock distro repo these
        // package names won't resolve and the install will fail
        // cleanly. The dialog labels this constraint.
        version_variants: &[
            VersionVariant {
                key: "pg-15",
                label: "PostgreSQL 15 (pgdg)",
                install_packages: &[
                    (PackageManager::Apt, &["postgresql-15"]),
                    (PackageManager::Dnf, &["postgresql15-server"]),
                    (PackageManager::Yum, &["postgresql15-server"]),
                ],
                probe_command: None,
            },
            VersionVariant {
                key: "pg-16",
                label: "PostgreSQL 16 (pgdg)",
                install_packages: &[
                    (PackageManager::Apt, &["postgresql-16"]),
                    (PackageManager::Dnf, &["postgresql16-server"]),
                    (PackageManager::Yum, &["postgresql16-server"]),
                ],
                probe_command: None,
            },
            VersionVariant {
                key: "pg-17",
                label: "PostgreSQL 17 (pgdg)",
                install_packages: &[
                    (PackageManager::Apt, &["postgresql-17"]),
                    (PackageManager::Dnf, &["postgresql17-server"]),
                    (PackageManager::Yum, &["postgresql17-server"]),
                ],
                probe_command: None,
            },
            VersionVariant {
                key: "pg-18",
                label: "PostgreSQL 18 (pgdg)",
                install_packages: &[
                    (PackageManager::Apt, &["postgresql-18"]),
                    (PackageManager::Dnf, &["postgresql18-server"]),
                    (PackageManager::Yum, &["postgresql18-server"]),
                ],
                probe_command: None,
            },
        ],
        category: "database",
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
        vendor_script: None,
        binary_name: "mysql",
        config_paths: &[
            "/etc/mysql/my.cnf",
            "/etc/my.cnf",
            "/etc/mysql/mariadb.conf.d",
        ],
        default_ports: &[3306],
        version_variants: &[],
        category: "database",
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
        vendor_script: None,
        binary_name: "nginx",
        config_paths: &[
            "/etc/nginx/nginx.conf",
            "/etc/nginx/conf.d",
            "/etc/nginx/sites-enabled",
        ],
        default_ports: &[80, 443],
        version_variants: &[],
        category: "web",
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
        vendor_script: None,
        binary_name: "jq",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "text",
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
        vendor_script: None,
        binary_name: "curl",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "network",
    },
    // ── Runtimes & build tools ─────────────────────────────────────
    PackageDescriptor {
        id: "java",
        display_name: "Java (OpenJDK)",
        // Generic probe — `java -version` writes to stderr; redirect
        // for version capture. The descriptor's probe finds *any*
        // installed JDK; per-variant detail probes use the variant's
        // override when present.
        probe_command: "command -v java >/dev/null 2>&1 && java -version 2>&1",
        // Default install packages — used only when no variant is
        // selected. Each manager's default JDK package picks a recent
        // long-term-supported release.
        install_packages: &[
            (PackageManager::Apt, &["default-jdk"]),
            (PackageManager::Dnf, &["java-latest-openjdk-devel"]),
            (PackageManager::Yum, &["java-21-openjdk-devel"]),
            (PackageManager::Apk, &["openjdk21"]),
            (PackageManager::Pacman, &["jdk-openjdk"]),
            (PackageManager::Zypper, &["java-21-openjdk-devel"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: Some("可在版本下拉里选择 OpenJDK 8 / 11 / 17 / 21；默认按发行版仓库的当前推荐版本安装。"),
        supports_reload: false,
        vendor_script: None,
        binary_name: "java",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[
            VersionVariant {
                key: "openjdk-8",
                label: "OpenJDK 8",
                install_packages: &[
                    (PackageManager::Apt, &["openjdk-8-jdk"]),
                    (PackageManager::Dnf, &["java-1.8.0-openjdk-devel"]),
                    (PackageManager::Yum, &["java-1.8.0-openjdk-devel"]),
                    (PackageManager::Apk, &["openjdk8"]),
                    (PackageManager::Pacman, &["jdk8-openjdk"]),
                    (PackageManager::Zypper, &["java-1_8_0-openjdk-devel"]),
                ],
                probe_command: None,
            },
            VersionVariant {
                key: "openjdk-11",
                label: "OpenJDK 11",
                install_packages: &[
                    (PackageManager::Apt, &["openjdk-11-jdk"]),
                    (PackageManager::Dnf, &["java-11-openjdk-devel"]),
                    (PackageManager::Yum, &["java-11-openjdk-devel"]),
                    (PackageManager::Apk, &["openjdk11"]),
                    (PackageManager::Pacman, &["jdk11-openjdk"]),
                    (PackageManager::Zypper, &["java-11-openjdk-devel"]),
                ],
                probe_command: None,
            },
            VersionVariant {
                key: "openjdk-17",
                label: "OpenJDK 17",
                install_packages: &[
                    (PackageManager::Apt, &["openjdk-17-jdk"]),
                    (PackageManager::Dnf, &["java-17-openjdk-devel"]),
                    (PackageManager::Yum, &["java-17-openjdk-devel"]),
                    (PackageManager::Apk, &["openjdk17"]),
                    (PackageManager::Pacman, &["jdk17-openjdk"]),
                    (PackageManager::Zypper, &["java-17-openjdk-devel"]),
                ],
                probe_command: None,
            },
            VersionVariant {
                key: "openjdk-21",
                label: "OpenJDK 21",
                install_packages: &[
                    (PackageManager::Apt, &["openjdk-21-jdk"]),
                    (PackageManager::Dnf, &["java-21-openjdk-devel"]),
                    (PackageManager::Yum, &["java-21-openjdk-devel"]),
                    (PackageManager::Apk, &["openjdk21"]),
                    (PackageManager::Pacman, &["jdk21-openjdk"]),
                    (PackageManager::Zypper, &["java-21-openjdk-devel"]),
                ],
                probe_command: None,
            },
        ],
        category: "runtime",
    },
    PackageDescriptor {
        id: "maven",
        display_name: "Apache Maven",
        probe_command: "command -v mvn >/dev/null 2>&1 && mvn -v 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["maven"]),
            (PackageManager::Dnf, &["maven"]),
            (PackageManager::Yum, &["maven"]),
            (PackageManager::Apk, &["maven"]),
            (PackageManager::Pacman, &["maven"]),
            (PackageManager::Zypper, &["maven"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "mvn",
        config_paths: &["/etc/maven/settings.xml"],
        default_ports: &[],
        version_variants: &[],
        category: "dev",
    },
    PackageDescriptor {
        id: "node",
        display_name: "Node.js",
        probe_command: "command -v node >/dev/null 2>&1 && node --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["nodejs"]),
            (PackageManager::Dnf, &["nodejs"]),
            (PackageManager::Yum, &["nodejs"]),
            (PackageManager::Apk, &["nodejs"]),
            (PackageManager::Pacman, &["nodejs"]),
            (PackageManager::Zypper, &["nodejs"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: Some("发行版仓库的 Node 版本通常滞后；可选 NodeSource 官方脚本安装最新 LTS 主线版本。"),
        supports_reload: false,
        vendor_script: Some(VendorScriptDescriptor {
            label: "NodeSource LTS",
            // Fallback URL — used when the host's manager isn't in
            // the per-family `urls` matrix below. NodeSource doesn't
            // ship a unified entrypoint, so we keep apt's URL here
            // and let RHEL-family route through `urls`.
            url: "https://deb.nodesource.com/setup_lts.x",
            urls: &[
                (PackageManager::Apt, "https://deb.nodesource.com/setup_lts.x"),
                (PackageManager::Dnf, "https://rpm.nodesource.com/setup_lts.x"),
                (PackageManager::Yum, "https://rpm.nodesource.com/setup_lts.x"),
            ],
            setup_scripts: &[],
            cleanup_scripts: &[],
            run_as_root: true,
            notes: "由 NodeSource 维护，会添加上游 apt / yum 仓库并安装最新 LTS 版 Node.js。Pier-X 不会校验脚本签名，请确认网络通路可信后再继续。",
            conflicts_with_apt: true,
        }),
        binary_name: "node",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "runtime",
    },
    PackageDescriptor {
        id: "python3",
        display_name: "Python 3",
        probe_command: "command -v python3 >/dev/null 2>&1 && python3 --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["python3"]),
            (PackageManager::Dnf, &["python3"]),
            (PackageManager::Yum, &["python3"]),
            (PackageManager::Apk, &["python3"]),
            (PackageManager::Pacman, &["python"]),
            (PackageManager::Zypper, &["python3"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "python3",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "runtime",
    },
    PackageDescriptor {
        id: "go",
        display_name: "Go",
        probe_command: "command -v go >/dev/null 2>&1 && go version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["golang-go"]),
            (PackageManager::Dnf, &["golang"]),
            (PackageManager::Yum, &["golang"]),
            (PackageManager::Apk, &["go"]),
            (PackageManager::Pacman, &["go"]),
            (PackageManager::Zypper, &["go"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "go",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "runtime",
    },
    PackageDescriptor {
        id: "git",
        display_name: "Git",
        probe_command: "command -v git >/dev/null 2>&1 && git --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["git"]),
            (PackageManager::Dnf, &["git"]),
            (PackageManager::Yum, &["git"]),
            (PackageManager::Apk, &["git"]),
            (PackageManager::Pacman, &["git"]),
            (PackageManager::Zypper, &["git"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "git",
        config_paths: &["/etc/gitconfig"],
        default_ports: &[],
        version_variants: &[],
        category: "dev",
    },
    PackageDescriptor {
        id: "htop",
        display_name: "htop",
        probe_command: "command -v htop >/dev/null 2>&1 && htop --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["htop"]),
            (PackageManager::Dnf, &["htop"]),
            (PackageManager::Yum, &["htop"]),
            (PackageManager::Apk, &["htop"]),
            (PackageManager::Pacman, &["htop"]),
            (PackageManager::Zypper, &["htop"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "htop",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "system",
    },
    PackageDescriptor {
        id: "wget",
        display_name: "wget",
        probe_command: "command -v wget >/dev/null 2>&1 && wget --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["wget"]),
            (PackageManager::Dnf, &["wget"]),
            (PackageManager::Yum, &["wget"]),
            (PackageManager::Apk, &["wget"]),
            (PackageManager::Pacman, &["wget"]),
            (PackageManager::Zypper, &["wget"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "wget",
        config_paths: &["/etc/wgetrc"],
        default_ports: &[],
        version_variants: &[],
        category: "network",
    },
    PackageDescriptor {
        id: "vim",
        display_name: "Vim",
        probe_command: "command -v vim >/dev/null 2>&1 && vim --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["vim"]),
            (PackageManager::Dnf, &["vim-enhanced"]),
            (PackageManager::Yum, &["vim-enhanced"]),
            (PackageManager::Apk, &["vim"]),
            (PackageManager::Pacman, &["vim"]),
            (PackageManager::Zypper, &["vim"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "vim",
        config_paths: &["/etc/vim/vimrc", "/etc/vimrc"],
        default_ports: &[],
        version_variants: &[],
        category: "editor",
    },
    PackageDescriptor {
        id: "tmux",
        display_name: "tmux",
        probe_command: "command -v tmux >/dev/null 2>&1 && tmux -V 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["tmux"]),
            (PackageManager::Dnf, &["tmux"]),
            (PackageManager::Yum, &["tmux"]),
            (PackageManager::Apk, &["tmux"]),
            (PackageManager::Pacman, &["tmux"]),
            (PackageManager::Zypper, &["tmux"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "tmux",
        config_paths: &["/etc/tmux.conf"],
        default_ports: &[],
        version_variants: &[],
        category: "terminal",
    },
    // ── App-store v2.2: editor / dev / network / system / text ────
    PackageDescriptor {
        id: "nano",
        display_name: "GNU nano",
        probe_command: "command -v nano >/dev/null 2>&1 && nano --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["nano"]),
            (PackageManager::Dnf, &["nano"]),
            (PackageManager::Yum, &["nano"]),
            (PackageManager::Apk, &["nano"]),
            (PackageManager::Pacman, &["nano"]),
            (PackageManager::Zypper, &["nano"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "nano",
        config_paths: &["/etc/nanorc"],
        default_ports: &[],
        version_variants: &[],
        category: "editor",
    },
    PackageDescriptor {
        id: "neovim",
        display_name: "Neovim",
        probe_command: "command -v nvim >/dev/null 2>&1 && nvim --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["neovim"]),
            (PackageManager::Dnf, &["neovim"]),
            (PackageManager::Yum, &["neovim"]),
            (PackageManager::Apk, &["neovim"]),
            (PackageManager::Pacman, &["neovim"]),
            (PackageManager::Zypper, &["neovim"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "nvim",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "editor",
    },
    PackageDescriptor {
        id: "rust",
        display_name: "Rust (rustc)",
        // The distro `rustc` is usually old; the row's vendor-script
        // path covers the rustup channel for users who need a recent
        // toolchain. Probe matches whichever flavour is installed.
        probe_command: "command -v rustc >/dev/null 2>&1 && rustc --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["rustc"]),
            (PackageManager::Dnf, &["rust"]),
            (PackageManager::Yum, &["rust"]),
            (PackageManager::Apk, &["rust"]),
            (PackageManager::Pacman, &["rust"]),
            (PackageManager::Zypper, &["rust"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: Some("发行版仓库的 Rust 通常滞后。需要新版可选 rustup 官方脚本（按用户安装到 ~/.cargo）。"),
        supports_reload: false,
        vendor_script: Some(VendorScriptDescriptor {
            label: "rustup 官方脚本",
            url: "https://sh.rustup.rs",
            urls: &[],
            setup_scripts: &[],
            cleanup_scripts: &[],
            run_as_root: false,
            notes: "由 Rust 官方维护，安装到当前用户的 ~/.cargo（不写系统目录）。Pier-X 不会校验脚本签名，请确认网络通路可信后再继续。",
            conflicts_with_apt: false,
        }),
        binary_name: "rustc",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "runtime",
    },
    PackageDescriptor {
        id: "php",
        display_name: "PHP CLI",
        probe_command: "command -v php >/dev/null 2>&1 && php --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["php-cli"]),
            (PackageManager::Dnf, &["php-cli"]),
            (PackageManager::Yum, &["php-cli"]),
            (PackageManager::Apk, &["php83-cli"]),
            (PackageManager::Pacman, &["php"]),
            (PackageManager::Zypper, &["php8-cli"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "php",
        config_paths: &["/etc/php"],
        default_ports: &[],
        version_variants: &[],
        category: "runtime",
    },
    PackageDescriptor {
        id: "gcc",
        display_name: "GCC (C compiler)",
        probe_command: "command -v gcc >/dev/null 2>&1 && gcc --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["gcc"]),
            (PackageManager::Dnf, &["gcc"]),
            (PackageManager::Yum, &["gcc"]),
            (PackageManager::Apk, &["gcc"]),
            (PackageManager::Pacman, &["gcc"]),
            (PackageManager::Zypper, &["gcc"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "gcc",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "dev",
    },
    PackageDescriptor {
        id: "make",
        display_name: "GNU make",
        probe_command: "command -v make >/dev/null 2>&1 && make --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["make"]),
            (PackageManager::Dnf, &["make"]),
            (PackageManager::Yum, &["make"]),
            (PackageManager::Apk, &["make"]),
            (PackageManager::Pacman, &["make"]),
            (PackageManager::Zypper, &["make"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "make",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "dev",
    },
    PackageDescriptor {
        id: "cmake",
        display_name: "CMake",
        probe_command: "command -v cmake >/dev/null 2>&1 && cmake --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["cmake"]),
            (PackageManager::Dnf, &["cmake"]),
            (PackageManager::Yum, &["cmake"]),
            (PackageManager::Apk, &["cmake"]),
            (PackageManager::Pacman, &["cmake"]),
            (PackageManager::Zypper, &["cmake"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "cmake",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "dev",
    },
    PackageDescriptor {
        id: "openssl",
        display_name: "OpenSSL CLI",
        probe_command: "command -v openssl >/dev/null 2>&1 && openssl version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["openssl"]),
            (PackageManager::Dnf, &["openssl"]),
            (PackageManager::Yum, &["openssl"]),
            (PackageManager::Apk, &["openssl"]),
            (PackageManager::Pacman, &["openssl"]),
            (PackageManager::Zypper, &["openssl"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "openssl",
        config_paths: &["/etc/ssl/openssl.cnf", "/etc/pki/tls/openssl.cnf"],
        default_ports: &[],
        version_variants: &[],
        category: "network",
    },
    PackageDescriptor {
        id: "fail2ban",
        display_name: "Fail2Ban",
        probe_command: "command -v fail2ban-server >/dev/null 2>&1 && fail2ban-server -V 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["fail2ban"]),
            (PackageManager::Dnf, &["fail2ban"]),
            (PackageManager::Yum, &["fail2ban"]),
            (PackageManager::Apk, &["fail2ban"]),
            (PackageManager::Pacman, &["fail2ban"]),
            (PackageManager::Zypper, &["fail2ban"]),
        ],
        service_units: &[
            (PackageManager::Apt, "fail2ban"),
            (PackageManager::Dnf, "fail2ban"),
            (PackageManager::Yum, "fail2ban"),
            (PackageManager::Apk, "fail2ban"),
            (PackageManager::Pacman, "fail2ban"),
            (PackageManager::Zypper, "fail2ban"),
        ],
        data_dirs: &["/var/lib/fail2ban"],
        notes: None,
        supports_reload: true,
        vendor_script: None,
        binary_name: "fail2ban-server",
        config_paths: &["/etc/fail2ban/jail.local", "/etc/fail2ban/jail.conf"],
        default_ports: &[],
        version_variants: &[],
        category: "system",
    },
    PackageDescriptor {
        id: "zsh",
        display_name: "Zsh",
        probe_command: "command -v zsh >/dev/null 2>&1 && zsh --version 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["zsh"]),
            (PackageManager::Dnf, &["zsh"]),
            (PackageManager::Yum, &["zsh"]),
            (PackageManager::Apk, &["zsh"]),
            (PackageManager::Pacman, &["zsh"]),
            (PackageManager::Zypper, &["zsh"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "zsh",
        config_paths: &["/etc/zsh/zshrc", "/etc/zshrc"],
        default_ports: &[],
        version_variants: &[],
        category: "terminal",
    },
    PackageDescriptor {
        id: "ca-certificates",
        display_name: "CA Certificates",
        // No `--version` for ca-certificates; check via dpkg/rpm/etc.
        // We use update-ca-certificates / update-ca-trust as the
        // canonical "is this present" probe across families.
        probe_command:
            "command -v update-ca-certificates >/dev/null 2>&1 || command -v update-ca-trust >/dev/null 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["ca-certificates"]),
            (PackageManager::Dnf, &["ca-certificates"]),
            (PackageManager::Yum, &["ca-certificates"]),
            (PackageManager::Apk, &["ca-certificates"]),
            (PackageManager::Pacman, &["ca-certificates"]),
            (PackageManager::Zypper, &["ca-certificates"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: Some("根证书包。一些容器基底镜像未默认装；连 https 一直报证书错误时检查这里。"),
        supports_reload: false,
        vendor_script: None,
        binary_name: "",
        config_paths: &["/etc/ssl/certs", "/etc/pki/tls/certs"],
        default_ports: &[],
        version_variants: &[],
        category: "system",
    },
    PackageDescriptor {
        id: "rsync",
        display_name: "rsync",
        probe_command: "command -v rsync >/dev/null 2>&1 && rsync --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["rsync"]),
            (PackageManager::Dnf, &["rsync"]),
            (PackageManager::Yum, &["rsync"]),
            (PackageManager::Apk, &["rsync"]),
            (PackageManager::Pacman, &["rsync"]),
            (PackageManager::Zypper, &["rsync"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "rsync",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "network",
    },
    PackageDescriptor {
        id: "unzip",
        display_name: "unzip",
        probe_command: "command -v unzip >/dev/null 2>&1 && unzip -v 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["unzip"]),
            (PackageManager::Dnf, &["unzip"]),
            (PackageManager::Yum, &["unzip"]),
            (PackageManager::Apk, &["unzip"]),
            (PackageManager::Pacman, &["unzip"]),
            (PackageManager::Zypper, &["unzip"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "unzip",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "system",
    },
    PackageDescriptor {
        id: "ripgrep",
        display_name: "ripgrep",
        probe_command: "command -v rg >/dev/null 2>&1 && rg --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["ripgrep"]),
            (PackageManager::Dnf, &["ripgrep"]),
            (PackageManager::Yum, &["ripgrep"]),
            (PackageManager::Apk, &["ripgrep"]),
            (PackageManager::Pacman, &["ripgrep"]),
            (PackageManager::Zypper, &["ripgrep"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "rg",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "text",
    },
    // ── App-store v2.4: cross-distro common utilities ──────────────
    PackageDescriptor {
        id: "gradle",
        display_name: "Gradle",
        probe_command: "command -v gradle >/dev/null 2>&1 && gradle --version 2>&1 | head -3",
        install_packages: &[
            (PackageManager::Apt, &["gradle"]),
            (PackageManager::Dnf, &["gradle"]),
            (PackageManager::Yum, &["gradle"]),
            (PackageManager::Apk, &["gradle"]),
            (PackageManager::Pacman, &["gradle"]),
            (PackageManager::Zypper, &["gradle"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "gradle",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "dev",
    },
    PackageDescriptor {
        id: "ansible",
        display_name: "Ansible",
        probe_command: "command -v ansible >/dev/null 2>&1 && ansible --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["ansible"]),
            (PackageManager::Dnf, &["ansible"]),
            (PackageManager::Yum, &["ansible"]),
            (PackageManager::Apk, &["ansible"]),
            (PackageManager::Pacman, &["ansible"]),
            (PackageManager::Zypper, &["ansible"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "ansible",
        config_paths: &["/etc/ansible/ansible.cfg"],
        default_ports: &[],
        version_variants: &[],
        category: "system",
    },
    PackageDescriptor {
        id: "screen",
        display_name: "GNU Screen",
        probe_command: "command -v screen >/dev/null 2>&1 && screen --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["screen"]),
            (PackageManager::Dnf, &["screen"]),
            (PackageManager::Yum, &["screen"]),
            (PackageManager::Apk, &["screen"]),
            (PackageManager::Pacman, &["screen"]),
            (PackageManager::Zypper, &["screen"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "screen",
        config_paths: &["/etc/screenrc"],
        default_ports: &[],
        version_variants: &[],
        category: "terminal",
    },
    PackageDescriptor {
        id: "less",
        display_name: "less",
        probe_command: "command -v less >/dev/null 2>&1 && less --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["less"]),
            (PackageManager::Dnf, &["less"]),
            (PackageManager::Yum, &["less"]),
            (PackageManager::Apk, &["less"]),
            (PackageManager::Pacman, &["less"]),
            (PackageManager::Zypper, &["less"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "less",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "system",
    },
    PackageDescriptor {
        id: "lsof",
        display_name: "lsof",
        probe_command: "command -v lsof >/dev/null 2>&1 && lsof -v 2>&1 | head -2",
        install_packages: &[
            (PackageManager::Apt, &["lsof"]),
            (PackageManager::Dnf, &["lsof"]),
            (PackageManager::Yum, &["lsof"]),
            (PackageManager::Apk, &["lsof"]),
            (PackageManager::Pacman, &["lsof"]),
            (PackageManager::Zypper, &["lsof"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "lsof",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "system",
    },
    PackageDescriptor {
        id: "strace",
        display_name: "strace",
        probe_command: "command -v strace >/dev/null 2>&1 && strace -V 2>&1",
        install_packages: &[
            (PackageManager::Apt, &["strace"]),
            (PackageManager::Dnf, &["strace"]),
            (PackageManager::Yum, &["strace"]),
            (PackageManager::Apk, &["strace"]),
            (PackageManager::Pacman, &["strace"]),
            (PackageManager::Zypper, &["strace"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "strace",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "dev",
    },
    PackageDescriptor {
        id: "net-tools",
        display_name: "net-tools (ifconfig/netstat)",
        // No `--version` — probe via the canonical binary's presence;
        // version comes back empty which the panel handles gracefully.
        probe_command: "command -v ifconfig >/dev/null 2>&1 && ifconfig --version 2>&1 | head -1",
        install_packages: &[
            (PackageManager::Apt, &["net-tools"]),
            (PackageManager::Dnf, &["net-tools"]),
            (PackageManager::Yum, &["net-tools"]),
            (PackageManager::Apk, &["net-tools"]),
            (PackageManager::Pacman, &["net-tools"]),
            (PackageManager::Zypper, &["net-tools"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: Some("提供 ifconfig/netstat/route 等老牌命令；现代发行版已默认改用 ip/ss。"),
        supports_reload: false,
        vendor_script: None,
        binary_name: "ifconfig",
        config_paths: &[],
        default_ports: &[],
        version_variants: &[],
        category: "network",
    },
    PackageDescriptor {
        id: "bash-completion",
        display_name: "bash-completion",
        // bash-completion is a sourced script set — `command -v` is
        // pointless. Probe the canonical install marker file.
        probe_command:
            "[ -e /usr/share/bash-completion/bash_completion ] && echo bash-completion installed",
        install_packages: &[
            (PackageManager::Apt, &["bash-completion"]),
            (PackageManager::Dnf, &["bash-completion"]),
            (PackageManager::Yum, &["bash-completion"]),
            (PackageManager::Apk, &["bash-completion"]),
            (PackageManager::Pacman, &["bash-completion"]),
            (PackageManager::Zypper, &["bash-completion"]),
        ],
        service_units: &[],
        data_dirs: &[],
        notes: None,
        supports_reload: false,
        vendor_script: None,
        binary_name: "",
        config_paths: &["/etc/bash_completion.d"],
        default_ports: &[],
        version_variants: &[],
        category: "system",
    },
];

/// Look up a descriptor by id. `None` means "not in registry".
pub fn descriptor(id: &str) -> Option<&'static PackageDescriptor> {
    merged_registry().iter().find(|d| d.id == id)
}

// ── Curated bundles (v2.6) ─────────────────────────────────────────
//
// Pre-baked groups of registry ids that go together. The panel
// renders these as one-click cards above the per-package list so a
// fresh host can come up with "DevOps basics" or "Java dev" in a
// single confirmation.
//
// Bundles point at descriptor ids — order in `package_ids` is the
// order the panel installs them. We don't resolve the ids here so a
// user-extras entry can be referenced by a bundle (the merged
// registry resolves it at render time; missing ids are skipped with
// a log line).

/// One curated bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoftwareBundle {
    /// Stable id (`"devops"`, `"java-dev"`, …).
    pub id: &'static str,
    /// Human label rendered on the bundle card.
    pub display_name: &'static str,
    /// One-liner description shown beneath the label.
    pub description: &'static str,
    /// Descriptor ids this bundle pulls in. Install order = this
    /// order; a missing id (e.g. dropped by a future registry edit)
    /// is silently skipped at install time.
    pub package_ids: &'static [&'static str],
}

const BUNDLES: &[SoftwareBundle] = &[
    SoftwareBundle {
        id: "devops",
        display_name: "DevOps 基础",
        description: "git / curl / jq / vim / htop / tmux / ripgrep — 服务器维护常用工具",
        package_ids: &["git", "curl", "jq", "vim", "htop", "tmux", "ripgrep"],
    },
    SoftwareBundle {
        id: "java-dev",
        display_name: "Java 开发",
        description: "OpenJDK + Maven + Gradle + Git — Java 服务端开发起步",
        package_ids: &["java", "maven", "gradle", "git"],
    },
    SoftwareBundle {
        id: "container-ops",
        display_name: "容器运维",
        description: "Docker + Compose + git + curl — 跑容器化应用最小集合",
        package_ids: &["docker", "compose", "git", "curl"],
    },
    SoftwareBundle {
        id: "lamp",
        display_name: "Web 服务（LNMP）",
        description: "nginx + MariaDB + PHP — 经典 Linux Web 栈（M 用 MariaDB 替代 MySQL）",
        package_ids: &["nginx", "mariadb", "php"],
    },
    SoftwareBundle {
        id: "diagnostics",
        display_name: "系统诊断",
        description: "lsof + strace + net-tools + less — 排查线上问题常用",
        package_ids: &["lsof", "strace", "net-tools", "less"],
    },
    SoftwareBundle {
        id: "python-dev",
        display_name: "Python 开发",
        description: "Python 3 + git + curl + vim + tmux + ripgrep — 后端 / 数据脚本工作机",
        package_ids: &["python3", "git", "curl", "vim", "tmux", "ripgrep"],
    },
    SoftwareBundle {
        id: "node-dev",
        display_name: "Node.js 开发",
        description: "Node.js + git + curl + vim + tmux — JS 服务端工作机",
        package_ids: &["node", "git", "curl", "vim", "tmux"],
    },
    SoftwareBundle {
        id: "web-admin",
        display_name: "Web 管理员",
        description: "nginx + fail2ban + openssl + curl — 起一个对外 HTTPS 站点的最小集合",
        package_ids: &["nginx", "fail2ban", "openssl", "curl"],
    },
    SoftwareBundle {
        id: "monitoring",
        display_name: "运维诊断扩展",
        description: "htop + lsof + strace + net-tools + tcpdump 替代品 less + ripgrep — 完整的 troubleshooting 套件",
        package_ids: &["htop", "lsof", "strace", "net-tools", "less", "ripgrep"],
    },
];

/// Public bundle catalog. Built-in BUNDLES merged with user-extras
/// bundles (see `software-extras.json`'s `bundles` field). Order:
/// built-in first, then user extras in declaration order.
pub fn bundles() -> &'static [SoftwareBundle] {
    merged_bundles()
}

// ── Database metrics (v2.13) ───────────────────────────────────────
//
// Light-weight "is the daemon alive + how busy is it" probes for
// the three DBs we orchestrate. Returns a few numeric fields the
// panel renders as inline mini-stats. Probes are best-effort —
// permission failures / unreachable daemons surface as `None`,
// not errors.

/// One snapshot of database metrics. Field semantics depend on
/// the engine — see comments. `None` = the probe couldn't read
/// that metric (auth missing / wrong daemon state / no support).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbMetrics {
    /// `"postgres"` / `"mariadb"` / `"redis"`.
    pub kind: String,
    /// Active client connections / sessions.
    pub connections: Option<u32>,
    /// Resident memory in MiB (from `ps -o rss`).
    pub memory_mib: Option<u32>,
    /// Engine-specific extra. For postgres: backend count by state;
    /// for redis: total commands processed; for mysql: queries
    /// per second (Questions delta — but we only have one snapshot
    /// so it's the cumulative count). Free-form string the panel
    /// renders verbatim.
    pub extra: Option<String>,
    /// `true` when the probe ran without auth/connectivity errors.
    /// `false` = the numbers above are unreliable (UI shows "—").
    pub probe_ok: bool,
}

/// Probe PostgreSQL metrics via `psql -tAc`. Runs as the postgres
/// system user via `sudo` (same auth path as `postgres_create_user`).
pub async fn postgres_metrics(session: &SshSession) -> Result<DbMetrics> {
    let env = probe_host_env(session).await;
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let inner = "su - postgres -c 'psql -tAF\"|\" -c \"\
      SELECT \
        (SELECT count(*) FROM pg_stat_activity), \
        (SELECT pg_size_pretty(sum(pg_database_size(datname))) FROM pg_database)\"' 2>&1";
    let cmd = format!("{prefix}sh -c {} 2>&1", shell_single_quote(inner));
    let (code, stdout) = session.exec_command(&cmd).await?;
    if code != 0 {
        return Ok(DbMetrics {
            kind: "postgres".to_string(),
            connections: None,
            memory_mib: None,
            extra: None,
            probe_ok: false,
        });
    }
    let line = stdout.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let parts: Vec<&str> = line.split('|').collect();
    let connections = parts.first().and_then(|s| s.trim().parse().ok());
    let total_size = parts.get(1).map(|s| s.trim().to_string());
    let memory_mib = read_process_rss_mib(session, "postgres").await;
    Ok(DbMetrics {
        kind: "postgres".to_string(),
        connections,
        memory_mib,
        extra: total_size.map(|s| format!("data: {s}")),
        probe_ok: true,
    })
}

/// Probe MySQL/MariaDB metrics via `SHOW STATUS`. Uses
/// `auth_socket` first (sudo mysql); accepts an optional root
/// password for distros where root requires one.
pub async fn mysql_metrics(
    session: &SshSession,
    root_password: Option<&str>,
) -> Result<DbMetrics> {
    let env = probe_host_env(session).await;
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let pwd_env = match root_password {
        Some(p) if !p.is_empty() => format!("MYSQL_PWD={} ", shell_single_quote(p)),
        _ => String::new(),
    };
    let sql = "SHOW STATUS WHERE Variable_name IN ('Threads_connected','Questions','Uptime');";
    let inner = format!(
        "{pwd_env}mysql -u root -B -e {} 2>&1",
        shell_single_quote(sql),
    );
    let cmd = format!("{prefix}sh -c {} 2>&1", shell_single_quote(&inner));
    let (code, stdout) = session.exec_command(&cmd).await?;
    if code != 0 {
        return Ok(DbMetrics {
            kind: "mariadb".to_string(),
            connections: None,
            memory_mib: None,
            extra: None,
            probe_ok: false,
        });
    }
    // Two-column output, tab-separated, header line first.
    let mut conns: Option<u32> = None;
    let mut questions: Option<u64> = None;
    for line in stdout.lines().skip(1) {
        let mut it = line.split('\t');
        if let (Some(name), Some(val)) = (it.next(), it.next()) {
            let v = val.trim();
            match name.trim() {
                "Threads_connected" => conns = v.parse().ok(),
                "Questions" => questions = v.parse().ok(),
                _ => {}
            }
        }
    }
    let memory_mib = read_process_rss_mib(session, "mysqld").await
        .or(read_process_rss_mib(session, "mariadbd").await);
    Ok(DbMetrics {
        kind: "mariadb".to_string(),
        connections: conns,
        memory_mib,
        extra: questions.map(|q| format!("queries: {q}")),
        probe_ok: true,
    })
}

/// Probe Redis metrics via `redis-cli INFO clients` + `INFO memory`.
pub async fn redis_metrics(session: &SshSession) -> Result<DbMetrics> {
    let env = probe_host_env(session).await;
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let inner = "redis-cli INFO clients 2>&1; redis-cli INFO memory 2>&1; redis-cli INFO stats 2>&1";
    let cmd = format!("{prefix}sh -c {} 2>&1", shell_single_quote(inner));
    let (code, stdout) = session.exec_command(&cmd).await?;
    if code != 0 || stdout.contains("Could not connect") {
        return Ok(DbMetrics {
            kind: "redis".to_string(),
            connections: None,
            memory_mib: None,
            extra: None,
            probe_ok: false,
        });
    }
    let mut conns: Option<u32> = None;
    let mut mem_human: Option<String> = None;
    let mut commands: Option<u64> = None;
    for line in stdout.lines() {
        let line = line.trim().trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("connected_clients:") {
            conns = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("used_memory_human:") {
            mem_human = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("total_commands_processed:") {
            commands = rest.trim().parse().ok();
        }
    }
    let memory_mib = read_process_rss_mib(session, "redis-server").await;
    let extra = match (mem_human, commands) {
        (Some(m), Some(c)) => Some(format!("used: {m} · cmds: {c}")),
        (Some(m), None) => Some(format!("used: {m}")),
        (None, Some(c)) => Some(format!("cmds: {c}")),
        _ => None,
    };
    Ok(DbMetrics {
        kind: "redis".to_string(),
        connections: conns,
        memory_mib,
        extra,
        probe_ok: true,
    })
}

/// Helper — read the resident memory of a process by name in MiB.
/// `ps` prints "rss" in KiB; convert to MiB. Returns `None` when
/// no matching process is running.
async fn read_process_rss_mib(session: &SshSession, name: &str) -> Option<u32> {
    let cmd = format!(
        "ps -C {} -o rss= 2>/dev/null | awk '{{s+=$1}} END {{print s}}'",
        shell_single_quote(name),
    );
    match session.exec_command(&cmd).await {
        Ok((_, stdout)) => stdout
            .trim()
            .parse::<u64>()
            .ok()
            .and_then(|kib| (kib > 0).then_some((kib / 1024) as u32)),
        Err(_) => None,
    }
}

/// Blocking wrappers.
pub fn postgres_metrics_blocking(session: &SshSession) -> Result<DbMetrics> {
    crate::ssh::runtime::shared().block_on(postgres_metrics(session))
}
/// Blocking wrapper for [`mysql_metrics`].
pub fn mysql_metrics_blocking(
    session: &SshSession,
    root_password: Option<&str>,
) -> Result<DbMetrics> {
    crate::ssh::runtime::shared().block_on(mysql_metrics(session, root_password))
}
/// Blocking wrapper for [`redis_metrics`].
pub fn redis_metrics_blocking(session: &SshSession) -> Result<DbMetrics> {
    crate::ssh::runtime::shared().block_on(redis_metrics(session))
}

// ── Cross-host package clone (v2.12) ───────────────────────────────
//
// List all explicitly-installed (not pulled in as a transitive
// dependency) packages on a host. Per-manager:
//   * apt → `apt-mark showmanual`
//   * dnf/yum → `dnf history userinstalled` (RHEL 8+) or fallback
//   * apk → `apk info -e $(cat /etc/apk/world)` is impractical;
//     use `cat /etc/apk/world` directly (it's the user-pinned set)
//   * pacman → `pacman -Qe`
//   * zypper → `zypper search -i --installed-only -t package` parsed
//
// The frontend cross-references this list with the registry to
// surface only entries we know how to install on the target host.

/// List packages the user has explicitly installed (not auto deps).
/// Returns raw package names — no descriptor lookup yet, the
/// frontend filters / displays.
pub async fn list_user_installed(session: &SshSession) -> Result<Vec<String>> {
    let env = probe_host_env(session).await;
    let Some(manager) = env.package_manager else {
        return Ok(Vec::new());
    };
    let cmd = match manager {
        PackageManager::Apt => "apt-mark showmanual 2>/dev/null".to_string(),
        PackageManager::Dnf => {
            // `dnf history userinstalled` on RHEL 8+; fall back
            // to dnf list installed otherwise.
            "dnf history userinstalled 2>/dev/null | tail -n +2 || \
             dnf list installed 2>/dev/null | awk 'NR>1 {print $1}' | sed 's/\\..*//'"
                .to_string()
        }
        PackageManager::Yum => {
            "yum list installed 2>/dev/null | awk 'NR>1 {print $1}' | sed 's/\\..*//'"
                .to_string()
        }
        PackageManager::Apk => "cat /etc/apk/world 2>/dev/null".to_string(),
        PackageManager::Pacman => "pacman -Qeq 2>/dev/null".to_string(),
        PackageManager::Zypper => {
            "zypper search -i --installed-only -t package 2>/dev/null \
             | awk -F'|' 'NR>4 {gsub(/ /, \"\", $2); print $2}'"
                .to_string()
        }
    };
    let (_code, stdout) = session.exec_command(&cmd).await?;
    let mut out: Vec<String> = stdout
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    Ok(out)
}

/// Blocking wrapper for [`list_user_installed`].
pub fn list_user_installed_blocking(session: &SshSession) -> Result<Vec<String>> {
    crate::ssh::runtime::shared().block_on(list_user_installed(session))
}

/// Resolve an arbitrary package-manager name back to a registry
/// descriptor id when possible. The reverse map is per-manager —
/// `redis-server` (apt) and `redis` (dnf) both resolve to the
/// `redis` descriptor.
///
/// Returns the descriptor id when one matches; `None` when the
/// name isn't anywhere in the registry's `install_packages` matrix.
pub fn resolve_descriptor_for_package(
    package_name: &str,
    manager: PackageManager,
) -> Option<&'static str> {
    for d in registry() {
        if let Some(pkgs) = packages_for(d, manager) {
            if pkgs.iter().any(|p| *p == package_name) {
                return Some(d.id);
            }
        }
        // Also check version variants — useful for Java OpenJDK 21.
        for v in d.version_variants {
            if let Some(pkgs) = v
                .install_packages
                .iter()
                .find_map(|(m, ps)| (*m == manager).then_some(*ps))
            {
                if pkgs.iter().any(|p| *p == package_name) {
                    return Some(d.id);
                }
            }
        }
    }
    None
}

// ── Docker Compose templates (v2.11) ───────────────────────────────
//
// Curated single-file `docker-compose.yml` snippets the user can
// stamp out in one click after installing Docker. Each template
// gets a unique stack id so multiple stacks can coexist in
// `~/pier-x-stacks/<stack-id>/docker-compose.yml`.
//
// Security: templates are static literals — no user input flows
// into the YAML. Defaults pick safe-but-changeable passwords;
// the dialog warns the user to change them before exposing the
// stack to the internet.

/// One Compose stack template.
#[derive(Debug, Clone, Copy)]
pub struct ComposeTemplate {
    /// Stable id (also the directory name under `~/pier-x-stacks`).
    pub id: &'static str,
    /// Human label.
    pub display_name: &'static str,
    /// One-line description.
    pub description: &'static str,
    /// Verbatim `docker-compose.yml` text. Single-quoted on the
    /// remote when written to disk.
    pub yaml: &'static str,
    /// Default ports the stack publishes — surfaced in the dialog
    /// as a heads-up before the user clicks apply.
    pub published_ports: &'static [u16],
}

const COMPOSE_TEMPLATES: &[ComposeTemplate] = &[
    ComposeTemplate {
        id: "postgres",
        display_name: "PostgreSQL 17",
        description: "PostgreSQL 17 with persistent volume; default password = piertest. Change it!",
        yaml: r#"services:
  postgres:
    image: postgres:17
    restart: unless-stopped
    environment:
      POSTGRES_USER: piertest
      POSTGRES_PASSWORD: piertest
      POSTGRES_DB: app
    ports:
      - "5432:5432"
    volumes:
      - pg-data:/var/lib/postgresql/data

volumes:
  pg-data:
"#,
        published_ports: &[5432],
    },
    ComposeTemplate {
        id: "redis",
        display_name: "Redis 7",
        description: "Redis 7 with appendonly persistence; no password by default.",
        yaml: r#"services:
  redis:
    image: redis:7-alpine
    restart: unless-stopped
    command: redis-server --appendonly yes
    ports:
      - "6379:6379"
    volumes:
      - redis-data:/data

volumes:
  redis-data:
"#,
        published_ports: &[6379],
    },
    ComposeTemplate {
        id: "nginx",
        display_name: "nginx (static site)",
        description: "nginx serving /srv/www. Drop static files into ./www on the host.",
        yaml: r#"services:
  nginx:
    image: nginx:alpine
    restart: unless-stopped
    ports:
      - "80:80"
    volumes:
      - ./www:/usr/share/nginx/html:ro
"#,
        published_ports: &[80],
    },
    ComposeTemplate {
        id: "grafana",
        display_name: "Grafana + Prometheus",
        description: "Monitoring stack. Grafana on :3000 (admin/admin), Prometheus on :9090.",
        yaml: r#"services:
  prometheus:
    image: prom/prometheus:latest
    restart: unless-stopped
    ports:
      - "9090:9090"
    volumes:
      - prom-data:/prometheus
  grafana:
    image: grafana/grafana:latest
    restart: unless-stopped
    depends_on: [prometheus]
    ports:
      - "3000:3000"
    environment:
      GF_SECURITY_ADMIN_USER: admin
      GF_SECURITY_ADMIN_PASSWORD: admin
    volumes:
      - grafana-data:/var/lib/grafana

volumes:
  prom-data:
  grafana-data:
"#,
        published_ports: &[3000, 9090],
    },
    ComposeTemplate {
        id: "registry",
        display_name: "Docker Registry",
        description: "Local Docker image registry on :5000 with persistent storage.",
        yaml: r#"services:
  registry:
    image: registry:2
    restart: unless-stopped
    ports:
      - "5000:5000"
    volumes:
      - registry-data:/var/lib/registry

volumes:
  registry-data:
"#,
        published_ports: &[5000],
    },
    ComposeTemplate {
        id: "elasticsearch",
        display_name: "Elasticsearch + Kibana",
        description: "ES 8 single-node + Kibana on :5601. Disables xpack security for local dev.",
        yaml: r#"services:
  elasticsearch:
    image: docker.elastic.co/elasticsearch/elasticsearch:8.13.0
    restart: unless-stopped
    environment:
      discovery.type: single-node
      xpack.security.enabled: "false"
      ES_JAVA_OPTS: "-Xms512m -Xmx512m"
    ports:
      - "9200:9200"
    volumes:
      - es-data:/usr/share/elasticsearch/data
  kibana:
    image: docker.elastic.co/kibana/kibana:8.13.0
    restart: unless-stopped
    depends_on: [elasticsearch]
    ports:
      - "5601:5601"
    environment:
      ELASTICSEARCH_HOSTS: "http://elasticsearch:9200"

volumes:
  es-data:
"#,
        published_ports: &[5601, 9200],
    },
];

/// Public accessor for the catalog.
pub fn compose_templates() -> &'static [ComposeTemplate] {
    COMPOSE_TEMPLATES
}

/// Look up a compose template by id.
pub fn compose_template_by_id(id: &str) -> Option<&'static ComposeTemplate> {
    COMPOSE_TEMPLATES.iter().find(|t| t.id == id)
}

/// Apply a compose template to a remote host: write the YAML to
/// `~/pier-x-stacks/<id>/docker-compose.yml` and run `docker
/// compose up -d`. Returns the same report shape as the install
/// path so the panel reuses the outcome formatter.
pub async fn compose_apply(
    session: &SshSession,
    template_id: &str,
) -> Result<PostgresActionReport> {
    let tmpl = compose_template_by_id(template_id).ok_or_else(|| {
        SshError::InvalidConfig(format!("unknown compose template: {template_id}"))
    })?;
    compose_apply_inline(session, tmpl.id, tmpl.yaml).await
}

/// Same as [`compose_apply`] but takes the YAML directly — used by the
/// user-uploaded template path where the catalog ID isn't part of the
/// built-in `COMPOSE_TEMPLATES` table.
pub async fn compose_apply_inline(
    session: &SshSession,
    id: &str,
    yaml: &str,
) -> Result<PostgresActionReport> {
    if id.is_empty() {
        return Err(SshError::InvalidConfig(
            "compose_apply_inline: empty id".to_string(),
        ));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(SshError::InvalidConfig(format!(
            "compose_apply_inline: id must be [a-zA-Z0-9_-]: {id:?}"
        )));
    }
    let env = probe_host_env(session).await;
    let prefix = if env.is_root { "" } else { "sudo -n " };
    // Use a heredoc so we don't have to escape $ / quotes inside
    // the YAML. The marker is constant so the heredoc body is
    // verbatim from the supplied yaml.
    let heredoc = "PIERX_COMPOSE_EOF";
    let inner = format!(
        "set -e; \
         dir=\"$HOME/pier-x-stacks/{id}\"; \
         mkdir -p \"$dir\"; \
         cat > \"$dir/docker-compose.yml\" <<'{heredoc}'\n{yaml}{heredoc}\n; \
         cd \"$dir\"; \
         docker compose up -d 2>&1",
    );
    let command = format!("{prefix}sh -c {} 2>&1", shell_single_quote(&inner));
    let (exit_code, stdout) = session.exec_command(&command).await?;
    let output_tail = stdout
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let status = if exit_code == 0 {
        "ok"
    } else if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        "sudo-requires-password"
    } else {
        "failed"
    };
    Ok(PostgresActionReport {
        status: status.to_string(),
        command,
        exit_code,
        output_tail,
    })
}

/// Tear down a previously-applied template via `docker compose
/// down`. Doesn't delete the YAML file — re-run `compose_apply`
/// or rm the directory manually to fully remove. Accepts any
/// stack id matching `[a-zA-Z0-9_-]+` so user-uploaded templates
/// can be torn down the same way as built-ins.
pub async fn compose_down(
    session: &SshSession,
    template_id: &str,
) -> Result<PostgresActionReport> {
    if template_id.is_empty() {
        return Err(SshError::InvalidConfig(
            "compose_down: empty template_id".to_string(),
        ));
    }
    if !template_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(SshError::InvalidConfig(format!(
            "compose_down: id must be [a-zA-Z0-9_-]: {template_id:?}"
        )));
    }
    let env = probe_host_env(session).await;
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let inner = format!(
        "set -e; \
         dir=\"$HOME/pier-x-stacks/{id}\"; \
         [ -e \"$dir/docker-compose.yml\" ] || {{ echo 'stack not found'; exit 1; }}; \
         cd \"$dir\"; \
         docker compose down 2>&1",
        id = template_id,
    );
    let command = format!("{prefix}sh -c {} 2>&1", shell_single_quote(&inner));
    let (exit_code, stdout) = session.exec_command(&command).await?;
    let output_tail = stdout
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let status = if exit_code == 0 {
        "ok"
    } else if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        "sudo-requires-password"
    } else {
        "failed"
    };
    Ok(PostgresActionReport {
        status: status.to_string(),
        command,
        exit_code,
        output_tail,
    })
}

/// Blocking wrappers.
pub fn compose_apply_blocking(
    session: &SshSession,
    template_id: &str,
) -> Result<PostgresActionReport> {
    crate::ssh::runtime::shared().block_on(compose_apply(session, template_id))
}

/// Blocking wrapper for [`compose_apply_inline`].
pub fn compose_apply_inline_blocking(
    session: &SshSession,
    id: &str,
    yaml: &str,
) -> Result<PostgresActionReport> {
    crate::ssh::runtime::shared().block_on(compose_apply_inline(session, id, yaml))
}

/// Blocking wrapper for [`compose_down`].
pub fn compose_down_blocking(
    session: &SshSession,
    template_id: &str,
) -> Result<PostgresActionReport> {
    crate::ssh::runtime::shared().block_on(compose_down(session, template_id))
}

// ── Co-install recommendations (v2.10) ─────────────────────────────
//
// Static "X is commonly installed alongside Y" map. After a
// successful install, the panel checks this list and surfaces
// any unmet recommendations as a chip strip below the row.
//
// Curation rules:
//   * Suggest only descriptors already in the registry — pointing
//     users at things we can install with one click.
//   * Keep lists short (3-5 items). Long lists become noise.
//   * Don't recommend the same id back at itself or its
//     dependencies (compose for docker — that's already linked
//     through the bundle catalog, separately).

/// Topologically sort a bundle's package ids so anchors install
/// before their co-install companions.
///
/// Semantics: an edge `anchor → companion` exists iff
/// `co_install_suggestions(anchor)` lists `companion` AND both are
/// in the input. Anchors install first, companions follow.
///
/// Concretely this means a bundle that mixes `docker` and
/// `compose` in arbitrary user order always gets reordered so
/// docker installs first — the apt resolver doesn't strictly need
/// it (compose is its own package), but the result is much closer
/// to what a user reads top-to-bottom in install logs.
///
/// Stability: items not connected by any edge keep their original
/// input position (Kahn's algorithm with input-order tie-break).
/// Cycles — which the static `co_install_suggestions` map should
/// never produce, but defensive code is cheap — fall back to
/// appending in input order so we never lose a package id.
///
/// This function operates on the static recommendation map only.
/// It does NOT consult the host's actual installed-package
/// dependency graph (apt-cache depends/ rdepends, etc.); that's
/// per-host work and lives downstream of the Tauri command layer.
pub fn topo_sort_bundle(ids: &[&str]) -> Vec<String> {
    use std::collections::{HashMap, HashSet};

    let id_set: HashSet<&str> = ids.iter().copied().collect();

    // Pre-seed indegree with every input id so the find() loop
    // below sees a value even for nodes that have no predecessors.
    let mut indegree: HashMap<&str, usize> =
        ids.iter().map(|&s| (s, 0_usize)).collect();
    let mut succs: HashMap<&str, Vec<&str>> = HashMap::new();

    for &id in ids {
        for &companion in co_install_suggestions(id) {
            if companion != id && id_set.contains(companion) {
                // Skip duplicate edges that would over-count
                // indegree if the same companion appears twice in
                // the static list (it shouldn't, but defensive).
                let entry = succs.entry(id).or_default();
                if !entry.contains(&companion) {
                    entry.push(companion);
                    *indegree.entry(companion).or_insert(0) += 1;
                }
            }
        }
    }

    let mut output: Vec<String> = Vec::with_capacity(ids.len());
    let mut placed: HashSet<&str> = HashSet::new();

    loop {
        // Pick the FIRST unplaced node in input order whose
        // indegree has dropped to zero. This is Kahn's with
        // explicit input-order priority — equivalent to a stable
        // topological sort.
        let candidate = ids
            .iter()
            .copied()
            .find(|id| !placed.contains(id) && indegree.get(id).copied() == Some(0));
        let Some(id) = candidate else { break };
        output.push(id.to_string());
        placed.insert(id);
        if let Some(children) = succs.get(id) {
            for &child in children {
                if let Some(d) = indegree.get_mut(child) {
                    if *d > 0 {
                        *d -= 1;
                    }
                }
            }
        }
    }

    // Cycle fallback: a true cycle would leave some nodes
    // perpetually with indegree > 0 and the loop above exits with
    // them unplaced. Append in input order so the bundle still
    // installs every requested package, just without the
    // ordering guarantee for the cyclic subset.
    for &id in ids {
        if !placed.contains(id) {
            output.push(id.to_string());
        }
    }
    output
}

/// Look up what to suggest installing alongside `installed_id`.
/// Returns descriptor ids in display order; empty when the id has
/// no curated recommendations.
pub fn co_install_suggestions(installed_id: &str) -> &'static [&'static str] {
    match installed_id {
        // Server daemons
        "nginx" => &["fail2ban", "openssl", "curl", "rsync"],
        "redis" => &["openssl", "curl"],
        "postgres" => &["openssl", "curl", "rsync"],
        "mariadb" => &["openssl", "curl", "rsync"],
        // Runtimes
        "java" => &["maven", "gradle", "git"],
        "node" => &["git", "vim", "curl"],
        "python3" => &["git", "vim", "curl", "ripgrep"],
        "go" => &["git", "gcc", "make"],
        "rust" => &["git", "gcc", "make"],
        "php" => &["git", "curl", "openssl"],
        // Container ecosystem
        "docker" => &["compose", "git", "curl"],
        // Utility families that pull each other in
        "vim" => &["tmux", "ripgrep"],
        "tmux" => &["vim", "htop"],
        "git" => &["vim", "curl"],
        // Diagnostic tools naturally bundle
        "htop" => &["lsof", "strace", "net-tools"],
        _ => &[],
    }
}

// ── User-extras catalog ─────────────────────────────────────────────
//
// On startup, src-tauri calls [`set_user_extras_path`] with a path
// to a JSON file (typically the app config dir). The first call to
// [`registry`] / [`merged_registry`] / [`descriptor`] reads + parses
// that file; entries that pass validation are leaked into static
// memory and appended to the catalog the panel renders.
//
// Failures (file missing, JSON parse error, validation error) are
// logged but never surface as a panel error — the user keeps the
// built-in catalog regardless.

/// Path injected by src-tauri at startup. Set once; later calls
/// silently no-op so a misbehaving caller can't swap the file
/// after the catalog has already been built.
static USER_EXTRAS_PATH: std::sync::OnceLock<std::path::PathBuf> =
    std::sync::OnceLock::new();

/// Memoized merged catalog: REGISTRY + parsed user extras. Built
/// lazily on first access. Subsequent edits to the extras file are
/// not observed — restart the app to pick up changes.
static MERGED_REGISTRY: std::sync::OnceLock<&'static [PackageDescriptor]> =
    std::sync::OnceLock::new();

/// Set the path to the user-extras JSON file. Idempotent — only
/// the first call wins. Errors when called after the catalog is
/// already built (the panel queried `registry()` before init).
pub fn set_user_extras_path(path: std::path::PathBuf) -> std::result::Result<(), &'static str> {
    USER_EXTRAS_PATH
        .set(path)
        .map_err(|_| "user_extras_path already set")
}

/// Return the path src-tauri set (if any). Used by tests + the
/// `software_user_extras_path` Tauri command so the UI can show
/// "your extras live here" in the panel.
pub fn user_extras_path() -> Option<&'static std::path::Path> {
    USER_EXTRAS_PATH.get().map(|p| p.as_path())
}

/// Memoized merged bundle catalog: built-in BUNDLES + user extras.
/// Same lazy + Box::leak pattern as MERGED_REGISTRY.
static MERGED_BUNDLES: std::sync::OnceLock<&'static [SoftwareBundle]> =
    std::sync::OnceLock::new();

fn ensure_user_extras_loaded() -> &'static UserExtrasParsed {
    static CACHED: std::sync::OnceLock<UserExtrasParsed> = std::sync::OnceLock::new();
    CACHED.get_or_init(|| {
        let Some(path) = USER_EXTRAS_PATH.get() else {
            return UserExtrasParsed::default();
        };
        match load_user_extras(path) {
            Ok(e) => UserExtrasParsed {
                packages: e.packages,
                bundles: e.bundles,
            },
            Err(e) => {
                eprintln!(
                    "pier-core: failed to load user extras at {}: {e}",
                    path.display()
                );
                UserExtrasParsed::default()
            }
        }
    })
}

#[derive(Default)]
struct UserExtrasParsed {
    packages: Vec<PackageDescriptor>,
    bundles: Vec<SoftwareBundle>,
}

fn merged_registry() -> &'static [PackageDescriptor] {
    MERGED_REGISTRY.get_or_init(|| {
        let extras = ensure_user_extras_loaded();
        let mut v: Vec<PackageDescriptor> = REGISTRY.iter().cloned().collect();
        let built_in_ids: std::collections::HashSet<&str> =
            REGISTRY.iter().map(|d| d.id).collect();
        for e in &extras.packages {
            if built_in_ids.contains(e.id) {
                eprintln!(
                    "pier-core: user extra '{}' skipped (id collides with built-in)",
                    e.id
                );
                continue;
            }
            v.push(e.clone());
        }
        Box::leak(v.into_boxed_slice())
    })
}

fn merged_bundles() -> &'static [SoftwareBundle] {
    MERGED_BUNDLES.get_or_init(|| {
        let extras = ensure_user_extras_loaded();
        let mut v: Vec<SoftwareBundle> = BUNDLES.iter().copied().collect();
        let built_in_ids: std::collections::HashSet<&str> =
            BUNDLES.iter().map(|b| b.id).collect();
        for b in &extras.bundles {
            if built_in_ids.contains(b.id) {
                eprintln!(
                    "pier-core: user bundle '{}' skipped (id collides with built-in)",
                    b.id
                );
                continue;
            }
            v.push(*b);
        }
        Box::leak(v.into_boxed_slice())
    })
}

/// JSON schema mirror — what the user writes in software-extras.json.
/// Keep the field set narrow on purpose: every field that surfaces
/// in the panel must be settable, but advanced fields (vendor_script,
/// version_variants) stay out of the user-facing surface for now —
/// they're security-sensitive and would need their own validation.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserPackageJson {
    id: String,
    display_name: String,
    probe_command: String,
    install_packages: std::collections::HashMap<String, Vec<String>>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    binary_name: Option<String>,
    #[serde(default)]
    config_paths: Vec<String>,
    #[serde(default)]
    default_ports: Vec<u16>,
    #[serde(default)]
    data_dirs: Vec<String>,
    #[serde(default)]
    service_units: std::collections::HashMap<String, String>,
    #[serde(default)]
    supports_reload: bool,
    #[serde(default)]
    category: Option<String>,
}

/// Parsed contents of `software-extras.json`. The schema accepts both
/// the legacy `[<package>...]` array and a richer wrapper object so
/// users can ship custom bundles alongside their custom packages.
#[derive(Debug, Default)]
struct UserExtras {
    packages: Vec<PackageDescriptor>,
    bundles: Vec<SoftwareBundle>,
}

/// Wrapper-form schema. Legacy file shapes (a top-level array) get
/// promoted into this struct's `packages` field with `bundles` empty.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserExtrasWrapper {
    #[serde(default)]
    packages: Vec<UserPackageJson>,
    #[serde(default)]
    bundles: Vec<UserBundleJson>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserBundleJson {
    id: String,
    display_name: String,
    #[serde(default)]
    description: String,
    package_ids: Vec<String>,
}

/// Two-shape JSON deserialiser. Accepts either:
///   * `[ {package}, {package}, ... ]`               (legacy)
///   * `{ "packages": [...], "bundles": [...] }`     (wrapper)
fn parse_user_extras_bytes(
    bytes: &[u8],
) -> std::result::Result<UserExtrasWrapper, String> {
    // Probe the first non-whitespace byte: `[` → legacy, `{` → wrapper.
    let first = bytes.iter().copied().find(|b| !b.is_ascii_whitespace());
    if first == Some(b'[') {
        let pkgs: Vec<UserPackageJson> =
            serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        Ok(UserExtrasWrapper {
            packages: pkgs,
            bundles: Vec::new(),
        })
    } else {
        serde_json::from_slice::<UserExtrasWrapper>(bytes).map_err(|e| e.to_string())
    }
}

fn load_user_extras(
    path: &std::path::Path,
) -> std::result::Result<UserExtras, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let wrapper = parse_user_extras_bytes(&bytes)?;
    let mut packages: Vec<PackageDescriptor> = Vec::with_capacity(wrapper.packages.len());
    for raw in wrapper.packages {
        match validate_and_leak(raw) {
            Ok(d) => packages.push(d),
            Err(e) => eprintln!("pier-core: skipping user extra package: {e}"),
        }
    }
    let mut bundles: Vec<SoftwareBundle> = Vec::with_capacity(wrapper.bundles.len());
    for raw in wrapper.bundles {
        match validate_and_leak_bundle(raw) {
            Ok(b) => bundles.push(b),
            Err(e) => eprintln!("pier-core: skipping user extra bundle: {e}"),
        }
    }
    Ok(UserExtras { packages, bundles })
}

fn validate_and_leak_bundle(
    raw: UserBundleJson,
) -> std::result::Result<SoftwareBundle, String> {
    if raw.id.trim().is_empty() {
        return Err("bundle missing id".into());
    }
    if raw.display_name.trim().is_empty() {
        return Err(format!("bundle '{}' missing displayName", raw.id));
    }
    if raw.package_ids.is_empty() {
        return Err(format!("bundle '{}' has empty packageIds", raw.id));
    }
    let leaked_ids: Vec<&'static str> = raw.package_ids.into_iter().map(leak_str).collect();
    Ok(SoftwareBundle {
        id: leak_str(raw.id),
        display_name: leak_str(raw.display_name),
        description: leak_str(raw.description),
        package_ids: leak_slice(leaked_ids),
    })
}

fn validate_and_leak(raw: UserPackageJson) -> std::result::Result<PackageDescriptor, String> {
    if raw.id.trim().is_empty() {
        return Err("entry missing id".into());
    }
    if raw.display_name.trim().is_empty() {
        return Err(format!("entry '{}' missing displayName", raw.id));
    }
    if raw.probe_command.trim().is_empty() {
        return Err(format!("entry '{}' missing probeCommand", raw.id));
    }
    if raw.install_packages.is_empty() {
        return Err(format!(
            "entry '{}' has empty installPackages",
            raw.id
        ));
    }
    let mut install: Vec<(PackageManager, &'static [&'static str])> =
        Vec::with_capacity(raw.install_packages.len());
    for (manager_id, pkgs) in raw.install_packages {
        if pkgs.is_empty() {
            return Err(format!(
                "entry '{}' has empty package list for manager '{}'",
                raw.id, manager_id
            ));
        }
        let manager = parse_manager_id(&manager_id).ok_or_else(|| {
            format!(
                "entry '{}' references unknown manager '{}'",
                raw.id, manager_id
            )
        })?;
        let leaked: Vec<&'static str> = pkgs.into_iter().map(leak_str).collect();
        install.push((manager, leak_slice(leaked)));
    }
    let install_packages: &'static [(PackageManager, &'static [&'static str])] =
        leak_slice(install);

    let mut services: Vec<(PackageManager, &'static str)> = Vec::new();
    for (manager_id, unit) in raw.service_units {
        let manager = parse_manager_id(&manager_id).ok_or_else(|| {
            format!(
                "entry '{}' service_units references unknown manager '{}'",
                raw.id, manager_id
            )
        })?;
        services.push((manager, leak_str(unit)));
    }
    let service_units: &'static [(PackageManager, &'static str)] = leak_slice(services);

    let data_dirs: Vec<&'static str> = raw.data_dirs.into_iter().map(leak_str).collect();
    let config_paths: Vec<&'static str> = raw.config_paths.into_iter().map(leak_str).collect();
    let default_ports: Vec<u16> = raw.default_ports;

    let category = raw.category.unwrap_or_default();
    let binary_name = raw.binary_name.unwrap_or_default();

    Ok(PackageDescriptor {
        id: leak_str(raw.id),
        display_name: leak_str(raw.display_name),
        probe_command: leak_str(raw.probe_command),
        install_packages,
        service_units,
        data_dirs: leak_slice(data_dirs),
        notes: raw.notes.map(leak_str),
        supports_reload: raw.supports_reload,
        // User extras can't declare vendor scripts — those need a
        // separate audit path (would land Pier-X-supplied curl|sh
        // execution behind user-controlled URLs).
        vendor_script: None,
        binary_name: leak_str(binary_name),
        config_paths: leak_slice(config_paths),
        default_ports: leak_slice(default_ports),
        version_variants: &[],
        category: leak_str(category),
    })
}

fn parse_manager_id(s: &str) -> Option<PackageManager> {
    match s.to_ascii_lowercase().as_str() {
        "apt" => Some(PackageManager::Apt),
        "dnf" => Some(PackageManager::Dnf),
        "yum" => Some(PackageManager::Yum),
        "apk" => Some(PackageManager::Apk),
        "pacman" => Some(PackageManager::Pacman),
        "zypper" => Some(PackageManager::Zypper),
        _ => None,
    }
}

fn leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn leak_slice<T>(v: Vec<T>) -> &'static [T] {
    Box::leak(v.into_boxed_slice())
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
///
/// Distro coverage notes:
/// * apt: every Debian/Ubuntu derivative we've seen in the wild,
///   including Chinese-localized Deepin and Linx (deepin/linx).
/// * dnf: RHEL family + the openEuler / Anolis / OpenCloudOS / Kylin /
///   UOS line which are technically RHEL-clone-adjacent but ship `dnf`
///   as the default. Amazon Linux 2023 / Oracle Linux likewise.
/// * yum is intentionally NOT a top-level pick — `dnf` provides a
///   `yum` shim on every distro that still calls itself "yum"-based,
///   and our install command falls back to yum syntax when needed.
pub fn pick_package_manager(distro_id: &str) -> Option<PackageManager> {
    match distro_id {
        // Debian-family
        "debian" | "ubuntu" | "linuxmint" | "raspbian" | "pop" | "elementary" | "kali"
        | "deepin" | "linx" | "uos" => Some(PackageManager::Apt),
        // RHEL-family + Chinese RHEL clones (openEuler, Kylin server,
        // OpenCloudOS, TencentOS, Anolis, Asianux). All ship dnf as
        // the default; the few that still default to yum (CentOS 7,
        // RHEL 7) get auto-aliased by our install command.
        "fedora" | "rhel" | "centos" | "rocky" | "almalinux" | "ol" | "amzn"
        | "openeuler" | "kylin" | "anolis" | "opencloudos" | "tencentos"
        | "asianux" | "circlelinux" => Some(PackageManager::Dnf),
        // Alpine
        "alpine" => Some(PackageManager::Apk),
        // Arch-family
        "arch" | "manjaro" | "endeavouros" | "garuda" | "artix" => Some(PackageManager::Pacman),
        // SUSE-family
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
///
/// `cancel` is the cancellation token forwarded into
/// [`SshSession::exec_command_streaming`]. When it fires mid-run the
/// returned report has [`InstallStatus::Cancelled`] and the post-install
/// probe / service-enable steps are skipped. `None` keeps the legacy
/// behaviour.
pub async fn install<F>(
    session: &SshSession,
    id: &str,
    enable_service: bool,
    version: Option<&str>,
    variant_key: Option<&str>,
    on_line: F,
    cancel: Option<CancellationToken>,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    run_install_or_update(
        session,
        id,
        false,
        enable_service,
        version,
        variant_key,
        on_line,
        cancel,
    )
    .await
}

/// Update (re-install / upgrade) a single package. Only meaningful for
/// already-installed packages — for missing ones it falls through to
/// the install command (most package managers' install is idempotent).
///
/// `version` pins to a specific package-manager version when `Some`.
/// pacman silently ignores it (Arch repos only carry the latest).
///
/// `cancel` semantics match [`install`].
pub async fn update<F>(
    session: &SshSession,
    id: &str,
    enable_service: bool,
    version: Option<&str>,
    variant_key: Option<&str>,
    on_line: F,
    cancel: Option<CancellationToken>,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    run_install_or_update(
        session,
        id,
        true,
        enable_service,
        version,
        variant_key,
        on_line,
        cancel,
    )
    .await
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
    variant_key: Option<&str>,
) -> Result<Vec<String>> {
    let descriptor = descriptor(id).ok_or_else(|| {
        SshError::InvalidConfig(format!("unknown package id: {id}"))
    })?;
    let env = probe_host_env(session).await;
    let Some(manager) = env.package_manager else {
        return Ok(Vec::new());
    };
    let Some(packages) = packages_for_with_variant(descriptor, manager, variant_key) else {
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

/// Install via the descriptor's vendor-supplied script (v2). The
/// `descriptor` argument is borrowed from [`registry`] — there is no
/// way to pass an arbitrary URL through this function.
///
/// Sequence:
///
/// 1. `mktemp`-equivalent path: `/tmp/pier-x-installer-{id}.sh`. The
///    `id` is the descriptor id, NOT user input — so the path is
///    statically known per package and can't be injected into.
/// 2. `curl -fsSL '<url>' -o '<path>'` (streamed; `-f` makes HTTP 4xx
///    a non-zero exit, `-S` keeps error messages, `-L` follows
///    redirects).
/// 3. `stat` for non-empty file size — defends against a 200-with-empty
///    body or a transparent proxy returning a placeholder.
/// 4. `sh '<path>'` (NOT `bash -c "$(curl ...)"` and NOT `curl … | sh`)
///    — splitting download from execution makes the audit trail
///    obvious and allows step 3's size-check to short-circuit.
/// 5. `rm -f '<path>'` always runs (cleanup), inside an `sh -c` chain
///    using `trap` so it fires even on failure.
/// 6. Re-probe to confirm the binary is on PATH.
///
/// We do **not** verify GPG signatures or sha256 hashes. The dialog
/// surfaces this to the user explicitly; v3 is where signature
/// verification lands once we have a vendor public-key registry.
pub async fn install_via_script<F>(
    session: &SshSession,
    id: &str,
    enable_service: bool,
    variant_key: Option<&str>,
    on_line: F,
    cancel: Option<CancellationToken>,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    run_install_via_script(session, id, enable_service, variant_key, on_line, cancel).await
}

/// Blocking wrapper for [`install_via_script`]. Accepts the same
/// `CancellationToken` plumbing as [`install_blocking`] — the Tauri
/// command registers `install_id → token` before kicking the
/// `spawn_blocking` task and a Cancel click triggers the token to bail
/// out of the curl / sh exec channels.
pub fn install_via_script_blocking<F>(
    session: &SshSession,
    id: &str,
    enable_service: bool,
    variant_key: Option<&str>,
    on_line: F,
    cancel: Option<CancellationToken>,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    crate::ssh::runtime::shared().block_on(install_via_script(
        session,
        id,
        enable_service,
        variant_key,
        on_line,
        cancel,
    ))
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
    variant_key: Option<&str>,
    on_line: F,
    cancel: Option<CancellationToken>,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    crate::ssh::runtime::shared().block_on(install(
        session,
        id,
        enable_service,
        version,
        variant_key,
        on_line,
        cancel,
    ))
}

/// Blocking wrapper for [`update`].
pub fn update_blocking<F>(
    session: &SshSession,
    id: &str,
    enable_service: bool,
    version: Option<&str>,
    variant_key: Option<&str>,
    on_line: F,
    cancel: Option<CancellationToken>,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    crate::ssh::runtime::shared().block_on(update(
        session,
        id,
        enable_service,
        version,
        variant_key,
        on_line,
        cancel,
    ))
}

/// Blocking wrapper for [`available_versions`]. Tauri commands using
/// `spawn_blocking` invoke it from the sync closure body.
pub fn available_versions_blocking(
    session: &SshSession,
    id: &str,
    variant_key: Option<&str>,
) -> Result<Vec<String>> {
    crate::ssh::runtime::shared().block_on(available_versions(session, id, variant_key))
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
    cancel: Option<CancellationToken>,
) -> Result<UninstallReport>
where
    F: FnMut(&str),
{
    run_uninstall(session, id, opts, on_line, cancel).await
}

/// Blocking wrapper for [`uninstall`].
pub fn uninstall_blocking<F>(
    session: &SshSession,
    id: &str,
    opts: &UninstallOptions,
    on_line: F,
    cancel: Option<CancellationToken>,
) -> Result<UninstallReport>
where
    F: FnMut(&str),
{
    crate::ssh::runtime::shared().block_on(uninstall(session, id, opts, on_line, cancel))
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

// ── Service-level orchestration (v2.8) ─────────────────────────────
//
// Software-specific post-install helpers — the kind of thing users
// run after `apt install postgresql` to actually make the daemon
// useful. Currently only PostgreSQL has these; other DB packages
// (MySQL/MariaDB/Redis) can plug in here later.

/// Result of a [`postgres_create_user`] / [`postgres_create_db`] /
/// [`postgres_open_remote`] action. Mirrors install-report shape so
/// the panel can reuse the same outcome formatter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PostgresActionReport {
    /// `"ok"` / `"sudo-requires-password"` / `"failed"`.
    pub status: String,
    /// Exact shell command that ran on the remote.
    pub command: String,
    /// Exit code from the remote shell.
    pub exit_code: i32,
    /// Last ~40 lines of merged stdout+stderr.
    pub output_tail: String,
}

/// Run a one-liner `psql` command as the `postgres` system user.
/// Used by every PostgreSQL helper below — keeps the sudo +
/// `-u postgres` boilerplate in one place.
async fn run_pg_psql(session: &SshSession, sql: &str) -> Result<PostgresActionReport> {
    let env = probe_host_env(session).await;
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let inner = format!(
        "su - postgres -c {} 2>&1",
        shell_single_quote(&format!("psql -tAc {}", shell_single_quote(sql))),
    );
    let command = format!("{prefix}sh -c {} 2>&1", shell_single_quote(&inner));
    let (exit_code, stdout) = session.exec_command(&command).await?;
    let output_tail = stdout
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let status = if exit_code == 0 {
        "ok"
    } else if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        "sudo-requires-password"
    } else {
        "failed"
    };
    Ok(PostgresActionReport {
        status: status.to_string(),
        command,
        exit_code,
        output_tail,
    })
}

/// Create a PostgreSQL role with a password. Idempotent: skips
/// when a role with that name already exists. The username and
/// password are interpolated into a `DO $$ ... $$` block so the
/// PL/pgSQL block handles the "already exists" branch — avoids a
/// failure-to-create error when re-run.
pub async fn postgres_create_user(
    session: &SshSession,
    username: &str,
    password: &str,
    is_superuser: bool,
) -> Result<PostgresActionReport> {
    if username.trim().is_empty() {
        return Err(SshError::InvalidConfig("username empty".into()));
    }
    let escaped_user = pg_quote_ident(username);
    let escaped_pass = pg_quote_literal(password);
    let extra = if is_superuser { "SUPERUSER" } else { "LOGIN" };
    let sql = format!(
        "DO $$ BEGIN \
           IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = lower('{u_lit}')) THEN \
             EXECUTE format('CREATE ROLE %I WITH {extra} PASSWORD %L', '{u}', '{p_inner}'); \
           ELSE \
             EXECUTE format('ALTER ROLE %I WITH {extra} PASSWORD %L', '{u}', '{p_inner}'); \
           END IF; \
         END $$;",
        u_lit = pg_quote_literal_inner(username),
        u = username.replace('\'', "''"),
        p_inner = password.replace('\'', "''"),
        extra = extra,
    );
    // Suppress the gigantic SQL from the report. We still send it
    // verbatim; the report's `command` shows the wrapper.
    let _ = (escaped_user, escaped_pass);
    run_pg_psql(session, &sql).await
}

/// Create a database owned by `owner`. Idempotent — skips when
/// the database exists.
pub async fn postgres_create_db(
    session: &SshSession,
    db_name: &str,
    owner: &str,
) -> Result<PostgresActionReport> {
    if db_name.trim().is_empty() || owner.trim().is_empty() {
        return Err(SshError::InvalidConfig("db_name / owner empty".into()));
    }
    let sql = format!(
        "SELECT 'CREATE DATABASE \"{db}\" OWNER \"{ow}\"' \
         WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = '{db_lit}')\\gexec",
        db = db_name.replace('"', "\"\""),
        ow = owner.replace('"', "\"\""),
        db_lit = db_name.replace('\'', "''"),
    );
    run_pg_psql(session, &sql).await
}

/// Allow remote TCP connections by:
///   1. Setting `listen_addresses = '*'` in `postgresql.conf`.
///   2. Appending `host all all 0.0.0.0/0 md5` to `pg_hba.conf`
///      if no equivalent line is already present.
///   3. Reloading the server (`pg_ctl reload`) — this is enough
///      for hba changes; listen_addresses needs a restart, which
///      we report so the user can opt in.
///
/// Path discovery is per-distro: we ask postgres for SHOW
/// hba_file / config_file via psql so we don't hardcode
/// `/etc/postgresql/X/main/...`.
pub async fn postgres_open_remote(
    session: &SshSession,
) -> Result<PostgresActionReport> {
    let env = probe_host_env(session).await;
    let prefix = if env.is_root { "" } else { "sudo -n " };
    // Sequence:
    //   1. discover paths via psql (-tA strips formatting)
    //   2. append/replace listen_addresses
    //   3. ensure md5 host line in pg_hba.conf
    //   4. systemctl reload (best-effort)
    let inner = "set -e; \
      conf=$(su - postgres -c 'psql -tAc \"SHOW config_file\"' 2>&1 | tail -1); \
      hba=$(su - postgres -c 'psql -tAc \"SHOW hba_file\"' 2>&1 | tail -1); \
      [ -n \"$conf\" ] && [ -n \"$hba\" ] || { echo 'cannot discover postgres paths'; exit 1; }; \
      if grep -qE \"^[[:space:]]*listen_addresses\" \"$conf\"; then \
        sed -i -E \"s|^[[:space:]]*listen_addresses.*|listen_addresses = '*'|\" \"$conf\"; \
      else \
        echo \"listen_addresses = '*'\" >> \"$conf\"; \
      fi; \
      if ! grep -qE \"^host[[:space:]]+all[[:space:]]+all[[:space:]]+0\\.0\\.0\\.0/0\" \"$hba\"; then \
        echo 'host all all 0.0.0.0/0 md5' >> \"$hba\"; \
      fi; \
      systemctl reload postgresql 2>&1 || systemctl restart postgresql 2>&1 || true; \
      echo OK";
    let command = format!("{prefix}sh -c {} 2>&1", shell_single_quote(inner));
    let (exit_code, stdout) = session.exec_command(&command).await?;
    let output_tail = stdout
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let status = if exit_code == 0 {
        "ok"
    } else if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        "sudo-requires-password"
    } else {
        "failed"
    };
    Ok(PostgresActionReport {
        status: status.to_string(),
        command,
        exit_code,
        output_tail,
    })
}

/// Blocking wrappers.
pub fn postgres_create_user_blocking(
    session: &SshSession,
    username: &str,
    password: &str,
    is_superuser: bool,
) -> Result<PostgresActionReport> {
    crate::ssh::runtime::shared().block_on(postgres_create_user(
        session,
        username,
        password,
        is_superuser,
    ))
}

/// Blocking wrapper for [`postgres_create_db`].
pub fn postgres_create_db_blocking(
    session: &SshSession,
    db_name: &str,
    owner: &str,
) -> Result<PostgresActionReport> {
    crate::ssh::runtime::shared()
        .block_on(postgres_create_db(session, db_name, owner))
}

/// Blocking wrapper for [`postgres_open_remote`].
pub fn postgres_open_remote_blocking(session: &SshSession) -> Result<PostgresActionReport> {
    crate::ssh::runtime::shared().block_on(postgres_open_remote(session))
}

// ── MySQL / MariaDB service-level orchestration (v2.9) ───────────

/// Run a one-liner `mysql` command. Tries with no password first
/// (fresh installs on Ubuntu use `auth_socket` for root, so
/// `sudo mysql` works without a password). If `root_password` is
/// set we pass it via `MYSQL_PWD` so it doesn't leak into ps.
async fn run_mysql(
    session: &SshSession,
    sql: &str,
    root_password: Option<&str>,
) -> Result<PostgresActionReport> {
    let env = probe_host_env(session).await;
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let pwd_env = match root_password {
        Some(p) if !p.is_empty() => format!(
            "MYSQL_PWD={} ",
            shell_single_quote(p)
        ),
        _ => String::new(),
    };
    let inner = format!(
        "{pwd_env}mysql -u root -e {} 2>&1",
        shell_single_quote(sql),
    );
    let command = format!("{prefix}sh -c {} 2>&1", shell_single_quote(&inner));
    let (exit_code, stdout) = session.exec_command(&command).await?;
    let output_tail = stdout
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let status = if exit_code == 0 {
        "ok"
    } else if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        "sudo-requires-password"
    } else {
        "failed"
    };
    Ok(PostgresActionReport {
        status: status.to_string(),
        command,
        exit_code,
        output_tail,
    })
}

/// Create a MySQL user with full DB privileges. Idempotent —
/// `CREATE USER IF NOT EXISTS` + `ALTER USER` sequence resets the
/// password every time so re-running fixes a forgotten password.
pub async fn mysql_create_user(
    session: &SshSession,
    username: &str,
    password: &str,
    db_name: &str,
    root_password: Option<&str>,
) -> Result<PostgresActionReport> {
    if username.trim().is_empty() {
        return Err(SshError::InvalidConfig("username empty".into()));
    }
    let safe_user = username.replace('\'', "");
    let safe_pass = password.replace('\'', "");
    let safe_db = db_name.replace('`', "");
    // Allow connections from anywhere (`%`); GRANT is db-scoped
    // when db_name is non-empty, otherwise global.
    let grant_target = if safe_db.is_empty() {
        "*.*".to_string()
    } else {
        format!("`{safe_db}`.*")
    };
    let sql = format!(
        "CREATE USER IF NOT EXISTS '{safe_user}'@'%' IDENTIFIED BY '{safe_pass}'; \
         ALTER USER '{safe_user}'@'%' IDENTIFIED BY '{safe_pass}'; \
         GRANT ALL PRIVILEGES ON {grant_target} TO '{safe_user}'@'%'; \
         FLUSH PRIVILEGES;"
    );
    run_mysql(session, &sql, root_password).await
}

/// Create a MySQL database. Idempotent via `CREATE DATABASE IF NOT EXISTS`.
pub async fn mysql_create_db(
    session: &SshSession,
    db_name: &str,
    root_password: Option<&str>,
) -> Result<PostgresActionReport> {
    if db_name.trim().is_empty() {
        return Err(SshError::InvalidConfig("db_name empty".into()));
    }
    let safe_db = db_name.replace('`', "");
    let sql = format!(
        "CREATE DATABASE IF NOT EXISTS `{safe_db}` \
         CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;"
    );
    run_mysql(session, &sql, root_password).await
}

/// Allow remote TCP connections to MySQL/MariaDB by setting
/// `bind-address = 0.0.0.0` in the daemon config and restarting.
/// Walks every `mysqld.cnf` / `my.cnf` fragment that ships across
/// the apt/dnf packaging variants. Best-effort restart at the end.
pub async fn mysql_open_remote(
    session: &SshSession,
) -> Result<PostgresActionReport> {
    let env = probe_host_env(session).await;
    let prefix = if env.is_root { "" } else { "sudo -n " };
    // Targets:
    //   apt MariaDB  → /etc/mysql/mariadb.conf.d/50-server.cnf
    //   apt MySQL    → /etc/mysql/mysql.conf.d/mysqld.cnf
    //   dnf/yum      → /etc/my.cnf or /etc/my.cnf.d/*.cnf
    //
    // We sed over every file matching the pattern; absent files
    // fail with "no such file" but `|| true` keeps the chain
    // moving. `bind-address` rewrite lands on whichever file
    // already declared it.
    let inner = "set -e; \
      changed=0; \
      for f in /etc/mysql/mariadb.conf.d/*.cnf /etc/mysql/mysql.conf.d/*.cnf /etc/my.cnf /etc/my.cnf.d/*.cnf; do \
        [ -e \"$f\" ] || continue; \
        if grep -qE '^[[:space:]]*bind-address' \"$f\"; then \
          sed -i -E 's|^[[:space:]]*bind-address.*|bind-address = 0.0.0.0|' \"$f\"; \
          changed=1; \
        fi; \
      done; \
      if [ \"$changed\" -eq 0 ]; then \
        for f in /etc/mysql/mariadb.conf.d/50-server.cnf /etc/mysql/mysql.conf.d/mysqld.cnf /etc/my.cnf; do \
          if [ -e \"$f\" ]; then \
            printf '\\n[mysqld]\\nbind-address = 0.0.0.0\\n' >> \"$f\"; \
            changed=1; break; \
          fi; \
        done; \
      fi; \
      systemctl restart mariadb 2>&1 || systemctl restart mysql 2>&1 || systemctl restart mysqld 2>&1 || true; \
      [ \"$changed\" -eq 1 ] && echo OK || { echo 'no mysql config file found'; exit 1; }";
    let command = format!("{prefix}sh -c {} 2>&1", shell_single_quote(inner));
    let (exit_code, stdout) = session.exec_command(&command).await?;
    let output_tail = stdout
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let status = if exit_code == 0 {
        "ok"
    } else if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        "sudo-requires-password"
    } else {
        "failed"
    };
    Ok(PostgresActionReport {
        status: status.to_string(),
        command,
        exit_code,
        output_tail,
    })
}

/// Blocking wrappers for the three MySQL helpers.
pub fn mysql_create_user_blocking(
    session: &SshSession,
    username: &str,
    password: &str,
    db_name: &str,
    root_password: Option<&str>,
) -> Result<PostgresActionReport> {
    crate::ssh::runtime::shared().block_on(mysql_create_user(
        session,
        username,
        password,
        db_name,
        root_password,
    ))
}

/// Blocking wrapper for [`mysql_create_db`].
pub fn mysql_create_db_blocking(
    session: &SshSession,
    db_name: &str,
    root_password: Option<&str>,
) -> Result<PostgresActionReport> {
    crate::ssh::runtime::shared().block_on(mysql_create_db(session, db_name, root_password))
}

/// Blocking wrapper for [`mysql_open_remote`].
pub fn mysql_open_remote_blocking(session: &SshSession) -> Result<PostgresActionReport> {
    crate::ssh::runtime::shared().block_on(mysql_open_remote(session))
}

// ── Redis service-level orchestration (v2.9) ────────────────────

/// Set Redis `requirepass`. Walks the standard config locations
/// (`/etc/redis/redis.conf` for apt, `/etc/redis.conf` for dnf),
/// rewrites the `requirepass` line in place (or appends one when
/// none exists), then restarts the service.
pub async fn redis_set_password(
    session: &SshSession,
    password: &str,
) -> Result<PostgresActionReport> {
    if password.trim().is_empty() {
        return Err(SshError::InvalidConfig("password empty".into()));
    }
    let env = probe_host_env(session).await;
    let prefix = if env.is_root { "" } else { "sudo -n " };
    // The password is single-quoted into the sed expression after
    // we shell-escape it; defence-in-depth against passwords with
    // `'` or `&` (the latter is sed's match-back reference).
    let escaped_for_sed = password.replace('&', "\\&").replace('/', "\\/");
    let inner = format!(
        "set -e; \
         conf=''; \
         for f in /etc/redis/redis.conf /etc/redis.conf; do \
           [ -e \"$f\" ] && conf=\"$f\" && break; \
         done; \
         [ -n \"$conf\" ] || {{ echo 'redis.conf not found'; exit 1; }}; \
         if grep -qE '^[[:space:]]*requirepass[[:space:]]' \"$conf\"; then \
           sed -i -E 's/^[[:space:]]*requirepass[[:space:]].*/requirepass {esc}/' \"$conf\"; \
         else \
           printf '\\nrequirepass {esc}\\n' >> \"$conf\"; \
         fi; \
         systemctl restart redis-server 2>&1 || systemctl restart redis 2>&1 || true; \
         echo OK",
        esc = escaped_for_sed,
    );
    let command = format!("{prefix}sh -c {} 2>&1", shell_single_quote(&inner));
    let (exit_code, stdout) = session.exec_command(&command).await?;
    let output_tail = stdout
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let status = if exit_code == 0 {
        "ok"
    } else if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        "sudo-requires-password"
    } else {
        "failed"
    };
    Ok(PostgresActionReport {
        status: status.to_string(),
        command,
        exit_code,
        output_tail,
    })
}

/// Allow remote Redis connections by:
///   1. Replacing `bind 127.0.0.1` with `bind 0.0.0.0`.
///   2. Setting `protected-mode no` (Redis refuses external
///      connections in protected-mode unless a password is set
///      AND we're using the password — for the simple case we
///      drop protected-mode; the user should also call
///      [`redis_set_password`] for a sane setup).
pub async fn redis_open_remote(
    session: &SshSession,
) -> Result<PostgresActionReport> {
    let env = probe_host_env(session).await;
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let inner = "set -e; \
      conf=''; \
      for f in /etc/redis/redis.conf /etc/redis.conf; do \
        [ -e \"$f\" ] && conf=\"$f\" && break; \
      done; \
      [ -n \"$conf\" ] || { echo 'redis.conf not found'; exit 1; }; \
      sed -i -E 's/^[[:space:]]*bind[[:space:]].*/bind 0.0.0.0/' \"$conf\"; \
      if grep -qE '^[[:space:]]*protected-mode[[:space:]]' \"$conf\"; then \
        sed -i -E 's/^[[:space:]]*protected-mode[[:space:]].*/protected-mode no/' \"$conf\"; \
      else \
        printf '\\nprotected-mode no\\n' >> \"$conf\"; \
      fi; \
      systemctl restart redis-server 2>&1 || systemctl restart redis 2>&1 || true; \
      echo OK";
    let command = format!("{prefix}sh -c {} 2>&1", shell_single_quote(inner));
    let (exit_code, stdout) = session.exec_command(&command).await?;
    let output_tail = stdout
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let status = if exit_code == 0 {
        "ok"
    } else if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        "sudo-requires-password"
    } else {
        "failed"
    };
    Ok(PostgresActionReport {
        status: status.to_string(),
        command,
        exit_code,
        output_tail,
    })
}

/// Blocking wrapper for [`redis_set_password`].
pub fn redis_set_password_blocking(
    session: &SshSession,
    password: &str,
) -> Result<PostgresActionReport> {
    crate::ssh::runtime::shared().block_on(redis_set_password(session, password))
}

/// Blocking wrapper for [`redis_open_remote`].
pub fn redis_open_remote_blocking(session: &SshSession) -> Result<PostgresActionReport> {
    crate::ssh::runtime::shared().block_on(redis_open_remote(session))
}

/// Quote an SQL identifier (`"..."`) — escaping internal `"`.
fn pg_quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Quote an SQL literal (`'...'`) — escaping internal `'`.
fn pg_quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Same as [`pg_quote_literal`] but returns just the inner content
/// (no surrounding quotes) — used when the surrounding quotes are
/// added by the caller's format string.
fn pg_quote_literal_inner(s: &str) -> String {
    s.replace('\'', "''")
}

/// One row in a system-wide package search result. Returns the
/// raw package name + a one-line summary; the panel renders these
/// underneath the registry section as "搜索系统仓库".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// Package name as the manager reports it (e.g. `"redis-server"`).
    pub name: String,
    /// One-line summary the manager prints alongside the name.
    /// Empty string when the manager's output didn't include one.
    pub summary: String,
}

/// Search the host's package manager catalog for `query`. Maps to:
///   * apt → `apt-cache search`
///   * dnf → `dnf search -q`
///   * yum → `yum search -q`
///   * apk → `apk search -d`
///   * pacman → `pacman -Ss`
///   * zypper → `zypper search`
///
/// Returns up to `limit` hits parsed from the manager's stdout.
/// `query` is single-quoted before interpolation so spaces /
/// shell metacharacters can't break the command.
pub async fn search_remote(
    session: &SshSession,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    let env = probe_host_env(session).await;
    let Some(manager) = env.package_manager else {
        return Ok(Vec::new());
    };
    let q = shell_single_quote(query);
    let inner = match manager {
        PackageManager::Apt => format!("apt-cache search {q} 2>/dev/null"),
        PackageManager::Dnf => format!("dnf search {q} -q 2>/dev/null"),
        PackageManager::Yum => format!("yum search {q} -q 2>/dev/null"),
        PackageManager::Apk => format!("apk search -d {q} 2>/dev/null"),
        PackageManager::Pacman => format!("pacman -Ss {q} 2>/dev/null"),
        PackageManager::Zypper => format!("zypper --non-interactive search {q} 2>/dev/null"),
    };
    let cmd = format!("sh -c {} 2>&1 | head -{}", shell_single_quote(&inner), limit * 4);
    let (_code, stdout) = session.exec_command(&cmd).await?;
    Ok(parse_search_output(manager, &stdout, limit))
}

/// Blocking wrapper for [`search_remote`].
pub fn search_remote_blocking(
    session: &SshSession,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    crate::ssh::runtime::shared().block_on(search_remote(session, query, limit))
}

/// Install a package not in the registry — i.e. one the user found
/// via [`search_remote`]. Same code path as [`install`] but the
/// caller supplies the package name directly instead of a
/// descriptor id.
///
/// We deliberately don't fold this into [`install`] because the
/// descriptor lookup carries metadata (config_paths / data_dirs /
/// service_units) the registry needs; ad-hoc installs have none of
/// that, and pretending they do would mean writing fake registry
/// entries.
pub async fn install_arbitrary<F>(
    session: &SshSession,
    package_name: &str,
    on_line: F,
    cancel: Option<CancellationToken>,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    run_install_arbitrary(session, package_name, on_line, cancel).await
}

/// Blocking wrapper for [`install_arbitrary`].
pub fn install_arbitrary_blocking<F>(
    session: &SshSession,
    package_name: &str,
    on_line: F,
    cancel: Option<CancellationToken>,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    crate::ssh::runtime::shared().block_on(install_arbitrary(session, package_name, on_line, cancel))
}

/// Synthesise the install command **without running it** so the
/// frontend can offer a "copy to clipboard" path for users who prefer
/// to vet the command before pasting it into their own shell. Honours
/// the same `version_pin` / `variant_key` semantics as [`install`].
///
/// Returns:
///   * the `sudo -n sh -c '...'` wrapper that pier-core would have
///     run, with each interpolation already shell-escaped.
///   * the inner command (without the sh -c wrapper) for users who
///     want to copy just the meaningful bits.
///
/// Errors only on SSH-level failure — unsupported distro / unknown
/// id surface as descriptive `Err` strings the panel can show.
pub async fn install_command_preview(
    session: &SshSession,
    id: &str,
    version: Option<&str>,
    variant_key: Option<&str>,
    is_update: bool,
) -> Result<InstallCommandPreview> {
    let descriptor = descriptor(id).ok_or_else(|| {
        SshError::InvalidConfig(format!("unknown package id: {id}"))
    })?;
    let env = probe_host_env(session).await;
    let manager = env.package_manager.ok_or_else(|| {
        SshError::InvalidConfig(format!(
            "host distro '{}' has no detected package manager",
            env.distro_id
        ))
    })?;
    let packages = packages_for_with_variant(descriptor, manager, variant_key)
        .ok_or_else(|| {
            SshError::InvalidConfig(format!(
                "{id} has no install packages for {}",
                manager.as_str()
            ))
        })?;
    let inner = build_install_command(manager, packages, is_update, version);
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let outer = format!(
        "{prefix}sh -c {} 2>&1",
        shell_single_quote(&inner)
    );
    Ok(InstallCommandPreview {
        package_id: id.to_string(),
        package_manager: manager.as_str().to_string(),
        is_root: env.is_root,
        inner_command: inner,
        wrapped_command: outer,
    })
}

/// Blocking wrapper for [`install_command_preview`].
pub fn install_command_preview_blocking(
    session: &SshSession,
    id: &str,
    version: Option<&str>,
    variant_key: Option<&str>,
    is_update: bool,
) -> Result<InstallCommandPreview> {
    crate::ssh::runtime::shared().block_on(install_command_preview(
        session,
        id,
        version,
        variant_key,
        is_update,
    ))
}

/// Result of [`install_command_preview`]. Both forms are returned so
/// the UI can choose: the wrapped form pastes into any shell as-is;
/// the inner form is what the user actually sees doing work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstallCommandPreview {
    /// Echoes the descriptor id.
    pub package_id: String,
    /// Resolved manager (`apt` / `dnf` / …).
    pub package_manager: String,
    /// Whether the host runs as root — drives whether the wrapper
    /// has the `sudo -n` prefix.
    pub is_root: bool,
    /// Just the package-manager command (e.g.
    /// `"apt-get update -qq && apt-get install -y nginx"`).
    pub inner_command: String,
    /// Full `sudo -n sh -c '...' 2>&1` wrapping that pier-core would
    /// have run. Paste-able into any shell.
    pub wrapped_command: String,
}

/// Lazy "expand a row" probe — returns install paths, existing config
/// files, default + listening ports, candidate (latest) version, and
/// per-variant install state. Only invoked from the panel when the
/// user clicks the row's disclosure, so the slow apt-cache / dnf info
/// queries never block the first paint.
///
/// Cost: roughly 4-5 extra SSH commands per call (binary probe, config
/// existence test, `ss -ltn`, candidate-version, plus one per variant).
/// All run sequentially on the existing session — no extra connections.
pub async fn probe_details(session: &SshSession, id: &str) -> Result<PackageDetail> {
    let descriptor = descriptor(id).ok_or_else(|| {
        SshError::InvalidConfig(format!("unknown package id: {id}"))
    })?;
    let env = probe_host_env(session).await;
    let pm = env.package_manager;

    let status = probe_status(session, id).await;
    let installed = status.as_ref().map(|s| s.installed).unwrap_or(false);
    let installed_version = status.as_ref().and_then(|s| s.version.clone());

    // Resolved binary path. Only meaningful when installed; skip the
    // round-trip otherwise.
    let bin = if descriptor.binary_name.is_empty() {
        descriptor.id
    } else {
        descriptor.binary_name
    };
    let install_paths = if installed && !bin.is_empty() {
        let inner = format!(
            "p=$(command -v {} 2>/dev/null); if [ -n \"$p\" ]; then readlink -f \"$p\" 2>/dev/null || echo \"$p\"; fi",
            shell_single_quote(bin)
        );
        let cmd = format!("sh -c {} 2>/dev/null", shell_single_quote(&inner));
        match session.exec_command(&cmd).await {
            Ok((_, stdout)) => stdout
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    // Existence check on each declared config path. We batch into one
    // shell invocation so /tmp/sh round-trips don't multiply per path.
    let config_paths = if !descriptor.config_paths.is_empty() {
        let inner = descriptor
            .config_paths
            .iter()
            .map(|p| {
                let qp = shell_single_quote(p);
                format!("[ -e {qp} ] && echo {qp}")
            })
            .collect::<Vec<_>>()
            .join("; ");
        let cmd = format!("sh -c {} 2>/dev/null || true", shell_single_quote(&inner));
        match session.exec_command(&cmd).await {
            Ok((_, stdout)) => stdout
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    // Listening-port probe — only run when the descriptor declares
    // defaults. Filter `ss -ltn` output to the declared port set so the
    // UI doesn't fan out into "everything listening on the host".
    let (listening_ports, listen_probe_ok) = if !descriptor.default_ports.is_empty() {
        match session.exec_command("ss -ltn 2>/dev/null").await {
            Ok((0, stdout)) if !stdout.trim().is_empty() => {
                let mut ports: Vec<u16> = Vec::new();
                for line in stdout.lines().skip(1) {
                    for tok in line.split_whitespace() {
                        if let Some(idx) = tok.rfind(':') {
                            if let Ok(p) = tok[idx + 1..].parse::<u16>() {
                                if descriptor.default_ports.contains(&p) && !ports.contains(&p) {
                                    ports.push(p);
                                }
                            }
                        }
                    }
                }
                (ports, true)
            }
            _ => (Vec::new(), false),
        }
    } else {
        (Vec::new(), true)
    };

    // Candidate version — always queries the descriptor's default
    // package list; per-variant queries belong to the variants block
    // below.
    let latest_version = if let Some(manager) = pm {
        if let Some(packages) = packages_for(descriptor, manager) {
            if let Some(pkg) = packages.first().copied() {
                if let Some(cmd) = build_candidate_version_command(manager, pkg) {
                    match session.exec_command(&cmd).await {
                        Ok((_, stdout)) => stdout
                            .lines()
                            .map(|l| l.trim())
                            .find(|l| !l.is_empty())
                            .map(|s| s.to_string()),
                        Err(_) => None,
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Per-variant probe — one rpm/dpkg/apk-info call per declared
    // variant. Empty for single-version software.
    let variants = if descriptor.version_variants.is_empty() {
        Vec::new()
    } else {
        let mut out = Vec::with_capacity(descriptor.version_variants.len());
        for v in descriptor.version_variants {
            let installed_variant = if let Some(manager) = pm {
                if let Some(pkgs) = packages_for_with_variant(descriptor, manager, Some(v.key)) {
                    if let Some(pkg) = pkgs.first().copied() {
                        let cmd = build_pkg_installed_check(manager, pkg);
                        session
                            .exec_command(&cmd)
                            .await
                            .ok()
                            .map(|(c, _)| c == 0)
                            .unwrap_or(false)
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };
            out.push(PackageVariantStatus {
                key: v.key.to_string(),
                label: v.label.to_string(),
                installed: installed_variant,
                installed_version: None,
            });
        }
        out
    };

    let service_unit = pm
        .and_then(|m| descriptor_service_unit(descriptor, m))
        .map(|s| s.to_string());

    Ok(PackageDetail {
        package_id: id.to_string(),
        installed,
        install_paths,
        config_paths,
        default_ports: descriptor.default_ports.to_vec(),
        listening_ports,
        listen_probe_ok,
        service_unit,
        latest_version,
        installed_version,
        variants,
    })
}

/// Blocking wrapper for [`probe_details`].
pub fn probe_details_blocking(session: &SshSession, id: &str) -> Result<PackageDetail> {
    crate::ssh::runtime::shared().block_on(probe_details(session, id))
}

// ── Internals ───────────────────────────────────────────────────────

/// Vendor-script install path. Owns the `curl → stat → sh → rm`
/// pipeline; see [`install_via_script`] for the sequence rationale.
/// Two-phase upstream-source install path. Used when a vendor_script
/// declares `setup_scripts` for the host's manager (PostgreSQL pgdg
/// is the canonical case). Runs the setup snippet, then drives the
/// descriptor's standard install path so the post-setup repo's
/// packages get pulled in. The returned report carries the
/// `vendor_script` view so the activity log credits the upstream
/// source instead of the default apt/dnf path.
async fn run_setup_then_install<F>(
    session: &SshSession,
    id: &str,
    setup_inner: &str,
    enable_service: bool,
    version: Option<&str>,
    variant_key: Option<&str>,
    mut on_line: F,
    cancel: Option<CancellationToken>,
    env: HostPackageEnv,
    used_view: VendorScriptUsedView,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let setup_command = format!(
        "{prefix}sh -c {} 2>&1",
        shell_single_quote(setup_inner)
    );

    let mut tail_lines: Vec<String> = Vec::new();
    let push_tail = |line: &str, tail: &mut Vec<String>| {
        tail.push(line.to_string());
        if tail.len() > 80 {
            tail.drain(0..tail.len() - 60);
        }
    };

    let setup_exit = match session
        .exec_command_streaming(
            &setup_command,
            |line| {
                on_line(line);
                push_tail(line, &mut tail_lines);
            },
            cancel.clone(),
        )
        .await
    {
        Ok((code, _)) => code,
        Err(e) => return Err(e),
    };
    let setup_tail = tail_lines.join("\n");

    if setup_exit == CANCELLED_EXIT_CODE
        || cancel.as_ref().is_some_and(|t| t.is_cancelled())
    {
        let repo_warnings = detect_broken_repo_warnings(&setup_tail);
        return Ok(InstallReport {
            package_id: id.to_string(),
            status: InstallStatus::Cancelled,
            distro_id: env.distro_id,
            package_manager: env
                .package_manager
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            command: setup_command,
            exit_code: setup_exit,
            output_tail: setup_tail,
            installed_version: None,
            service_active: None,
            vendor_script: Some(used_view),
            repo_warnings,
        });
    }

    if !env.is_root && looks_like_sudo_password_prompt(&setup_tail) {
        let repo_warnings = detect_broken_repo_warnings(&setup_tail);
        return Ok(InstallReport {
            package_id: id.to_string(),
            status: InstallStatus::SudoRequiresPassword,
            distro_id: env.distro_id,
            package_manager: env
                .package_manager
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            command: setup_command,
            exit_code: setup_exit,
            output_tail: setup_tail,
            installed_version: None,
            service_active: None,
            vendor_script: Some(used_view),
            repo_warnings,
        });
    }

    if setup_exit != 0 {
        let repo_warnings = detect_broken_repo_warnings(&setup_tail);
        return Ok(InstallReport {
            package_id: id.to_string(),
            // Reuse VendorScriptFailed since the setup step is
            // morally an extended version of "the upstream installer
            // didn't work" — same UI affordance.
            status: InstallStatus::VendorScriptFailed,
            distro_id: env.distro_id,
            package_manager: env
                .package_manager
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            command: setup_command,
            exit_code: setup_exit,
            output_tail: setup_tail,
            installed_version: None,
            service_active: None,
            vendor_script: Some(used_view),
            repo_warnings,
        });
    }

    // Setup succeeded — fall through to the normal install path.
    // The descriptor's install_packages (or the picked variant's)
    // resolve against the freshly-added upstream source.
    let install_report = run_install_or_update(
        session,
        id,
        false,
        enable_service,
        version,
        variant_key,
        on_line,
        cancel,
    )
    .await?;
    // Tag the report with the vendor view so the UI says "via
    // PostgreSQL 官方源" instead of "via apt".
    Ok(InstallReport {
        vendor_script: Some(used_view),
        ..install_report
    })
}

/// Ad-hoc install path — same shell synthesis as
/// [`run_install_or_update`] but with a caller-supplied package
/// name (no descriptor lookup, no service-unit handling, no
/// version pin / variant).
async fn run_install_arbitrary<F>(
    session: &SshSession,
    package_name: &str,
    mut on_line: F,
    cancel: Option<CancellationToken>,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    if package_name.trim().is_empty() {
        return Err(SshError::InvalidConfig("empty package name".into()));
    }
    let env = probe_host_env(session).await;
    let Some(manager) = env.package_manager else {
        return Ok(InstallReport {
            package_id: package_name.to_string(),
            status: InstallStatus::UnsupportedDistro,
            distro_id: env.distro_id,
            package_manager: String::new(),
            command: String::new(),
            exit_code: 0,
            output_tail: String::new(),
            installed_version: None,
            service_active: None,
            vendor_script: None,
            repo_warnings: Vec::new(),
        });
    };

    // Each package_name is a single token — apt/dnf/etc. don't
    // need it shell-quoted (their package names are
    // alphanumeric+dash). But quote anyway for defence-in-depth
    // since this string may originate in a search result the user
    // could have manipulated.
    let install_inner = build_install_command(manager, &[package_name], false, None);
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let command = format!(
        "{prefix}sh -c {} 2>&1",
        shell_single_quote(&install_inner)
    );

    let mut tail_lines: Vec<String> = Vec::new();
    let (exit_code, _full) = session
        .exec_command_streaming(
            &command,
            |line| {
                on_line(line);
                tail_lines.push(line.to_string());
                if tail_lines.len() > 80 {
                    tail_lines.drain(0..tail_lines.len() - 60);
                }
            },
            cancel.clone(),
        )
        .await?;
    let output_tail = tail_lines.join("\n");

    if exit_code == CANCELLED_EXIT_CODE
        || cancel.as_ref().is_some_and(|t| t.is_cancelled())
    {
        let repo_warnings = detect_broken_repo_warnings(&output_tail);
        return Ok(InstallReport {
            package_id: package_name.to_string(),
            status: InstallStatus::Cancelled,
            distro_id: env.distro_id,
            package_manager: manager.as_str().to_string(),
            command,
            exit_code,
            output_tail,
            installed_version: None,
            service_active: None,
            vendor_script: None,
            repo_warnings,
        });
    }

    if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        let repo_warnings = detect_broken_repo_warnings(&output_tail);
        return Ok(InstallReport {
            package_id: package_name.to_string(),
            status: InstallStatus::SudoRequiresPassword,
            distro_id: env.distro_id,
            package_manager: manager.as_str().to_string(),
            command,
            exit_code,
            output_tail,
            installed_version: None,
            service_active: None,
            vendor_script: None,
            repo_warnings,
        });
    }

    // No descriptor → no probe to confirm install. Trust the
    // package manager's exit code.
    let status = if exit_code == 0 {
        InstallStatus::Installed
    } else {
        InstallStatus::PackageManagerFailed
    };
    let repo_warnings = detect_broken_repo_warnings(&output_tail);
    Ok(InstallReport {
        package_id: package_name.to_string(),
        status,
        distro_id: env.distro_id,
        package_manager: manager.as_str().to_string(),
        command,
        exit_code,
        output_tail,
        installed_version: None,
        service_active: None,
        vendor_script: None,
        repo_warnings,
    })
}

/// Per-manager search-result parser. apt's "name - summary" format
/// and dnf's "name.arch : summary" format need separate handling;
/// pacman/zypper output is two-line per result. apk uses
/// "name-version description" on one line.
fn parse_search_output(manager: PackageManager, raw: &str, limit: usize) -> Vec<SearchHit> {
    let mut out: Vec<SearchHit> = Vec::new();
    match manager {
        PackageManager::Apt => {
            for line in raw.lines() {
                if let Some((name, summary)) = line.split_once(" - ") {
                    let name = name.trim();
                    let summary = summary.trim();
                    if !name.is_empty() {
                        out.push(SearchHit {
                            name: name.to_string(),
                            summary: summary.to_string(),
                        });
                    }
                }
                if out.len() >= limit {
                    break;
                }
            }
        }
        PackageManager::Dnf | PackageManager::Yum => {
            for line in raw.lines() {
                // Format: "name.arch : summary"
                let s = line.trim();
                if s.starts_with("===")
                    || s.starts_with("Last metadata")
                    || s.is_empty()
                {
                    continue;
                }
                if let Some((left, summary)) = s.split_once(" : ") {
                    let name = left.split('.').next().unwrap_or(left).trim();
                    if !name.is_empty() {
                        out.push(SearchHit {
                            name: name.to_string(),
                            summary: summary.trim().to_string(),
                        });
                    }
                }
                if out.len() >= limit {
                    break;
                }
            }
        }
        PackageManager::Apk => {
            for line in raw.lines() {
                let s = line.trim();
                if s.is_empty() {
                    continue;
                }
                // "name-version description" — split on first space.
                if let Some((name_ver, summary)) = s.split_once(' ') {
                    // Trim version tail off `name-1.2.3-r0` → `name`.
                    let name = name_ver
                        .rsplitn(3, '-')
                        .last()
                        .unwrap_or(name_ver)
                        .to_string();
                    out.push(SearchHit {
                        name,
                        summary: summary.trim().to_string(),
                    });
                }
                if out.len() >= limit {
                    break;
                }
            }
        }
        PackageManager::Pacman => {
            // Two-line repeating format:
            //   "repo/name version [installed]"
            //   "    Description text"
            let mut iter = raw.lines();
            while let Some(head) = iter.next() {
                if head.starts_with("    ") || head.is_empty() {
                    continue;
                }
                let summary = iter.next().unwrap_or("").trim();
                let name = head.split_whitespace().next().unwrap_or(head);
                let name = name.split('/').nth(1).unwrap_or(name);
                if !name.is_empty() {
                    out.push(SearchHit {
                        name: name.to_string(),
                        summary: summary.to_string(),
                    });
                }
                if out.len() >= limit {
                    break;
                }
            }
        }
        PackageManager::Zypper => {
            // `|`-separated columns: "S | Name | Type | Version | Arch | Repository"
            for line in raw.lines() {
                if !line.contains('|') || line.starts_with("---") {
                    continue;
                }
                let cols: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
                if cols.len() < 2 {
                    continue;
                }
                let name = cols.get(1).copied().unwrap_or("");
                if name.is_empty() || name == "Name" {
                    continue;
                }
                out.push(SearchHit {
                    name: name.to_string(),
                    summary: cols.get(5).copied().unwrap_or("").to_string(),
                });
                if out.len() >= limit {
                    break;
                }
            }
        }
    }
    out
}

async fn run_install_via_script<F>(
    session: &SshSession,
    id: &str,
    enable_service: bool,
    variant_key: Option<&str>,
    mut on_line: F,
    cancel: Option<CancellationToken>,
) -> Result<InstallReport>
where
    F: FnMut(&str),
{
    let descriptor = descriptor(id).ok_or_else(|| {
        SshError::InvalidConfig(format!("unknown package id: {id}"))
    })?;
    let Some(vendor) = descriptor.vendor_script else {
        return Err(SshError::InvalidConfig(format!(
            "package {id} has no vendor_script"
        )));
    };

    let env = probe_host_env(session).await;
    // Pick the per-manager URL when the descriptor declares a
    // matrix; fall back to the generic `url` so existing entries
    // (Docker → get.docker.com, rustup → sh.rustup.rs) keep their
    // behaviour without an `urls` row.
    let chosen_url = env
        .package_manager
        .and_then(|pm| {
            vendor
                .urls
                .iter()
                .find_map(|(m, u)| (*m == pm).then_some(*u))
        })
        .unwrap_or(vendor.url);
    let used_view = VendorScriptUsedView {
        label: vendor.label.to_string(),
        url: chosen_url.to_string(),
    };

    // ── setup_scripts branch ─────────────────────────────────────
    //
    // PostgreSQL-style upstream sources do their setup in multiple
    // shell steps (GPG key + sources.list + apt-get update); the
    // download-and-execute pattern doesn't fit. When the descriptor
    // declares a setup_script for the resolved manager, run that
    // snippet first under sudo, then fall through to the regular
    // package-manager install path so the new repo's packages get
    // pulled in. The returned report is tagged with the same
    // `vendor_script` view so the activity log says "via PostgreSQL
    // 官方源" instead of pointing at the default apt path.
    if let Some(manager) = env.package_manager {
        if let Some(setup) = vendor
            .setup_scripts
            .iter()
            .find_map(|(m, s)| (*m == manager).then_some(*s))
        {
            return run_setup_then_install(
                session,
                id,
                setup,
                enable_service,
                None, // no version pin from this entrypoint
                variant_key,
                on_line,
                cancel,
                env,
                used_view,
            )
            .await;
        }
    }

    // Helper: build a Cancelled report from any state we have in hand.
    // Used by both the post-download and post-execute cancel checks so
    // we never run later steps (sh exec / re-probe / service-enable)
    // after the user has bailed.
    let cancelled_report = |command: String, exit_code: i32, output_tail: String| {
        let repo_warnings = detect_broken_repo_warnings(&output_tail);
        InstallReport {
            package_id: id.to_string(),
            status: InstallStatus::Cancelled,
            distro_id: env.distro_id.clone(),
            package_manager: env
                .package_manager
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            command,
            exit_code,
            output_tail,
            installed_version: None,
            service_active: None,
            vendor_script: Some(used_view.clone()),
            repo_warnings,
        }
    };

    // --- Step 1+2: download ---
    let script_path = format!("/tmp/pier-x-installer-{}.sh", descriptor.id);
    let download_inner = build_vendor_download_command(chosen_url, &script_path);
    // Download itself doesn't need root — `/tmp` is world-writable. We
    // only escalate for the script execution step. Keeping these as two
    // separate exec invocations means a download failure surfaces with a
    // clean exit code instead of being shadowed by sudo's behavior.
    let download_command = format!(
        "sh -c {} 2>&1",
        shell_single_quote(&download_inner)
    );

    let mut tail_lines: Vec<String> = Vec::new();
    let push_tail = |line: &str, tail: &mut Vec<String>| {
        tail.push(line.to_string());
        if tail.len() > 80 {
            tail.drain(0..tail.len() - 60);
        }
    };
    let dl_exit = match session
        .exec_command_streaming(
            &download_command,
            |line| {
                on_line(line);
                push_tail(line, &mut tail_lines);
            },
            cancel.clone(),
        )
        .await
    {
        Ok((code, _)) => code,
        Err(e) => return Err(e),
    };

    // Cancel check must precede the download-failure branch — a
    // cancelled curl can return any exit code (including the
    // CANCELLED_EXIT_CODE sentinel from `exec_command_streaming` or
    // a vanilla curl error if SIGHUP arrived mid-handshake). Either way
    // the user's intent was Cancel, not "download failed".
    if dl_exit == CANCELLED_EXIT_CODE
        || cancel.as_ref().is_some_and(|t| t.is_cancelled())
    {
        cleanup_vendor_temp(session, &script_path).await;
        return Ok(cancelled_report(download_command, dl_exit, tail_lines.join("\n")));
    }

    if dl_exit != 0 {
        // `cleanup_vendor_temp` is best-effort; ignore its result.
        cleanup_vendor_temp(session, &script_path).await;
        let output_tail = tail_lines.join("\n");
        let repo_warnings = detect_broken_repo_warnings(&output_tail);
        return Ok(InstallReport {
            package_id: id.to_string(),
            status: InstallStatus::VendorScriptDownloadFailed,
            distro_id: env.distro_id,
            package_manager: env
                .package_manager
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            command: download_command,
            exit_code: dl_exit,
            output_tail,
            installed_version: None,
            service_active: None,
            vendor_script: Some(used_view),
            repo_warnings,
        });
    }

    // --- Step 3+4+5: size-check + execute + cleanup, single sh ---
    let exec_inner = build_vendor_exec_command(&script_path);
    let prefix = if vendor.run_as_root && !env.is_root {
        "sudo -n "
    } else {
        ""
    };
    let exec_command = format!(
        "{prefix}sh -c {} 2>&1",
        shell_single_quote(&exec_inner)
    );

    let exec_exit = match session
        .exec_command_streaming(
            &exec_command,
            |line| {
                on_line(line);
                push_tail(line, &mut tail_lines);
            },
            cancel.clone(),
        )
        .await
    {
        Ok((code, _)) => code,
        Err(e) => {
            cleanup_vendor_temp(session, &script_path).await;
            return Err(e);
        }
    };
    let output_tail = tail_lines.join("\n");

    // Same priority as the apt path: cancellation wins over both
    // sudo-prompt detection and the post-probe / service-enable steps.
    if exec_exit == CANCELLED_EXIT_CODE
        || cancel.as_ref().is_some_and(|t| t.is_cancelled())
    {
        cleanup_vendor_temp(session, &script_path).await;
        return Ok(cancelled_report(exec_command, exec_exit, output_tail));
    }

    if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        cleanup_vendor_temp(session, &script_path).await;
        let repo_warnings = detect_broken_repo_warnings(&output_tail);
        return Ok(InstallReport {
            package_id: id.to_string(),
            status: InstallStatus::SudoRequiresPassword,
            distro_id: env.distro_id,
            package_manager: env
                .package_manager
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            command: exec_command,
            exit_code: exec_exit,
            output_tail,
            installed_version: None,
            service_active: None,
            vendor_script: Some(used_view),
            repo_warnings,
        });
    }

    // --- Step 6: re-probe ---
    let post = probe_status(session, id).await;
    let installed_version = post.as_ref().and_then(|s| s.version.clone());
    let was_installed_after = post.as_ref().map(|s| s.installed).unwrap_or(false);

    let status = if was_installed_after {
        InstallStatus::Installed
    } else {
        InstallStatus::VendorScriptFailed
    };

    // Vendor scripts (get.docker.com) typically already enable+start
    // their service. Mirror the apt-path's enable-service logic so the
    // checkbox keeps working for the script path too.
    let service_active = if was_installed_after && enable_service {
        if let Some(unit) = env
            .package_manager
            .and_then(|pm| descriptor_service_unit(descriptor, pm))
        {
            let svc_inner = format!("systemctl enable --now {unit} || true");
            let svc_cmd =
                format!("{prefix}sh -c {} 2>&1", shell_single_quote(&svc_inner));
            let _ = session
                .exec_command_streaming(&svc_cmd, &mut on_line, None)
                .await;
            Some(systemctl_is_active(session, unit).await)
        } else {
            None
        }
    } else if was_installed_after {
        if let Some(unit) = env
            .package_manager
            .and_then(|pm| descriptor_service_unit(descriptor, pm))
        {
            Some(systemctl_is_active(session, unit).await)
        } else {
            None
        }
    } else {
        None
    };

    let repo_warnings = detect_broken_repo_warnings(&output_tail);
    Ok(InstallReport {
        package_id: id.to_string(),
        status,
        distro_id: env.distro_id,
        package_manager: env
            .package_manager
            .map(|m| m.as_str().to_string())
            .unwrap_or_default(),
        command: exec_command,
        exit_code: exec_exit,
        output_tail,
        installed_version,
        service_active,
        vendor_script: Some(used_view),
        repo_warnings,
    })
}

/// `curl -fsSL '<url>' -o '<path>'`. URL is single-quoted so even if
/// a future registry edit introduces quotes / `$` the shell can't
/// interpret them — same defense as `shell_single_quote` elsewhere.
/// Path is single-quoted for symmetry; in practice it's always
/// `/tmp/pier-x-installer-{id}.sh` with a static id.
fn build_vendor_download_command(url: &str, path: &str) -> String {
    format!(
        "curl -fsSL {url} -o {path}",
        url = shell_single_quote(url),
        path = shell_single_quote(path),
    )
}

/// Size-check + execute + cleanup. The `trap` ensures the temp file
/// is removed regardless of the script's exit code, so a failed
/// install doesn't leave detritus in `/tmp`.
fn build_vendor_exec_command(path: &str) -> String {
    let qpath = shell_single_quote(path);
    format!(
        "trap 'rm -f {qpath}' EXIT; \
         if [ ! -s {qpath} ]; then \
         echo 'pier-x: downloaded installer is empty, aborting' >&2; \
         exit 64; \
         fi; \
         sh {qpath}"
    )
}

/// Best-effort `rm -f` for vendor-script tmp files. Used both in the
/// success-fast-path's exec command (via `trap`) and in error branches
/// that bail before `trap` ever runs.
async fn cleanup_vendor_temp(session: &SshSession, path: &str) {
    let cmd = format!("rm -f {} 2>/dev/null || true", shell_single_quote(path));
    let _ = session.exec_command(&cmd).await;
}

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
    variant_key: Option<&str>,
    mut on_line: F,
    cancel: Option<CancellationToken>,
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
            vendor_script: None,
            repo_warnings: Vec::new(),
        });
    };

    let Some(packages) = packages_for_with_variant(descriptor, manager, variant_key) else {
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
            vendor_script: None,
            repo_warnings: Vec::new(),
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
        .exec_command_streaming(
            &command,
            |line| {
                on_line(line);
                tail_lines.push(line.to_string());
                if tail_lines.len() > 80 {
                    tail_lines.drain(0..tail_lines.len() - 60);
                }
            },
            cancel.clone(),
        )
        .await?;
    let output_tail = tail_lines.join("\n");

    // Cancellation has to be checked BEFORE the "looks like sudo" /
    // post-probe / service-enable branches: those would fire fresh
    // commands on the same session and can mask the user's bail-out
    // intent. The CANCELLED_EXIT_CODE check covers both the case where
    // we tripped the select! arm ourselves and the rare case where the
    // remote happened to surface the same code.
    if exit_code == CANCELLED_EXIT_CODE
        || cancel.as_ref().is_some_and(|t| t.is_cancelled())
    {
        let repo_warnings = detect_broken_repo_warnings(&output_tail);
        return Ok(InstallReport {
            package_id: id.to_string(),
            status: InstallStatus::Cancelled,
            distro_id: env.distro_id,
            package_manager: manager.as_str().to_string(),
            command,
            exit_code,
            output_tail,
            installed_version: None,
            service_active: None,
            vendor_script: None,
            repo_warnings,
        });
    }

    if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        let repo_warnings = detect_broken_repo_warnings(&output_tail);
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
            vendor_script: None,
            repo_warnings,
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
                .exec_command_streaming(&svc_cmd, &mut on_line, cancel.clone())
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

    let repo_warnings = detect_broken_repo_warnings(&output_tail);
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
        vendor_script: None,
        repo_warnings,
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
    cancel: Option<CancellationToken>,
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
    let cleanup_script = descriptor.vendor_script.and_then(|v| {
        v.cleanup_scripts
            .iter()
            .find_map(|(m, s)| (*m == manager).then_some(*s))
    });
    let inner = build_uninstall_command_inner(
        manager,
        packages,
        descriptor.data_dirs,
        opts,
        service_unit,
        cleanup_script,
    );
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let command = format!("{prefix}sh -c {} 2>&1", shell_single_quote(&inner));

    let mut tail_lines: Vec<String> = Vec::new();
    let (exit_code, _full) = session
        .exec_command_streaming(
            &command,
            |line| {
                on_line(line);
                tail_lines.push(line.to_string());
                if tail_lines.len() > 80 {
                    tail_lines.drain(0..tail_lines.len() - 60);
                }
            },
            cancel.clone(),
        )
        .await?;
    let output_tail = tail_lines.join("\n");

    // Same fast-path-out as the install side: cancel beats every other
    // post-action branch, including the post-removal probe.
    if exit_code == CANCELLED_EXIT_CODE
        || cancel.as_ref().is_some_and(|t| t.is_cancelled())
    {
        return Ok(UninstallReport {
            package_id: id.to_string(),
            status: UninstallStatus::Cancelled,
            distro_id: env.distro_id,
            package_manager: manager.as_str().to_string(),
            command,
            exit_code,
            output_tail,
            data_dirs_removed: false,
        });
    }

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
        .exec_command_streaming(
            &command,
            |line| {
                on_line(line);
                tail_lines.push(line.to_string());
                if tail_lines.len() > 80 {
                    tail_lines.drain(0..tail_lines.len() - 60);
                }
            },
            None,
        )
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

/// Pick the package list, honouring an optional variant override.
/// Falls back to the descriptor's defaults when `variant_key` is `None`
/// or unknown — the latter case shouldn't happen in practice (the
/// frontend hands back the same key that came from the registry) but
/// we don't error so a stale registry never blocks an install.
fn packages_for_with_variant(
    descriptor: &PackageDescriptor,
    manager: PackageManager,
    variant_key: Option<&str>,
) -> Option<&'static [&'static str]> {
    if let Some(key) = variant_key {
        if let Some(variant) = descriptor.version_variants.iter().find(|v| v.key == key) {
            return variant
                .install_packages
                .iter()
                .find_map(|(m, pkgs)| (*m == manager).then_some(*pkgs));
        }
    }
    packages_for(descriptor, manager)
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
///
/// ## Tolerance to broken third-party repos
///
/// A common failure mode in the wild: the host has some
/// `/etc/apt/sources.list.d/<vendor>.list` line pointing at a repo that
/// the upstream has since taken down (Docker dropping Ubuntu focal,
/// PPAs going dormant, internal mirrors moving). With a strict
/// `apt-get update && apt-get install` chain, that one stale line
/// returns exit 100 and **every** install attempt fails — even for
/// packages that live in the perfectly-healthy main archive.
///
/// We deliberately decouple the steps so the same scenario degrades
/// gracefully without touching the host's source list:
///
/// * **apt** — `update; install` (sequential, not `&&`). The refresh
///   stderr still streams to the UI so the user sees *which* repo
///   broke; install proceeds against the existing
///   `/var/lib/apt/lists/` cache. If install genuinely needs a fresh
///   index for a package not in cache, it will surface its own
///   "Unable to locate package" error — which is more actionable than
///   "exit 100".
/// * **dnf / yum** — `--setopt=skip_if_unavailable=True` makes a
///   single broken repo a per-repo warning instead of a global abort,
///   matching apt's new behaviour.
/// * **apk / pacman / zypper** — these don't have a separate
///   pre-install refresh step in our flow (apk's cache is per-call,
///   pacman/zypper read the existing DB), so the broken-third-party
///   pattern doesn't apply. Left unchanged.
fn build_install_command(
    manager: PackageManager,
    packages: &[&str],
    is_update: bool,
    version: Option<&str>,
) -> String {
    let pkgs = format_packages_with_version(manager, packages, version);
    match (manager, is_update) {
        (PackageManager::Apt, false) => format!(
            "DEBIAN_FRONTEND=noninteractive apt-get update -qq; \
             DEBIAN_FRONTEND=noninteractive apt-get install -y {pkgs}"
        ),
        (PackageManager::Apt, true) => format!(
            "DEBIAN_FRONTEND=noninteractive apt-get update -qq; \
             DEBIAN_FRONTEND=noninteractive apt-get install -y --only-upgrade {pkgs}"
        ),
        (PackageManager::Dnf, false) => {
            format!("dnf install -y --setopt=skip_if_unavailable=True {pkgs}")
        }
        (PackageManager::Dnf, true) => {
            format!("dnf upgrade -y --setopt=skip_if_unavailable=True {pkgs}")
        }
        (PackageManager::Yum, false) => {
            format!("yum install -y --setopt=skip_if_unavailable=True {pkgs}")
        }
        (PackageManager::Yum, true) => {
            format!("yum update -y --setopt=skip_if_unavailable=True {pkgs}")
        }
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

/// Scan a chunk of merged install output for "broken third-party repo"
/// patterns that we proactively skipped past. Returns the offending
/// repo URLs / hostnames, deduped and trimmed for the UI banner.
///
/// Pattern coverage (kept lenient on purpose — the same human-readable
/// wording shows up across distros and locales):
///
///  * apt: `不再含有 Release` / `no longer has a Release file`
///         `Failed to fetch ... 404`
///  * dnf/yum: `Failed to download metadata for repo '<id>'`
///             `Cannot download repomd.xml`
///             `skipping unavailable repo '<id>'`
///  * zypper: `Repository '<id>' is invalid`
///
/// Always returns warnings keyed by the repository URL or repo id —
/// good enough for the banner; the user can still scroll through the
/// full output for the verbatim apt/dnf/zypper diagnostic.
pub fn detect_broken_repo_warnings(output: &str) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();
    let mut push_unique = |s: String| {
        if !s.trim().is_empty() && !warnings.contains(&s) {
            warnings.push(s);
        }
    };

    for raw in output.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        // ── apt ──────────────────────────────────────────────────
        // Localised forms: en, zh-CN. Both flag the same scenario:
        // a previously-cached repo dropped its Release file upstream.
        let apt_release_marker = line.contains("no longer has a Release file")
            || line.contains("不再含有 Release")
            || line.contains("不再包含 Release");
        let apt_404_marker = line.starts_with("E:") && line.contains("Failed to fetch")
            || line.contains("404 Not Found")
            || line.contains("404  Not Found");
        if apt_release_marker || apt_404_marker {
            // Pull the URL out of `仓库 "<url>" 不再含有 Release 文件` /
            // `Repository '<url>' no longer has a Release file`.
            // Fall back to the whole line trimmed of the leading
            // diagnostic prefix when no quoted URL is present.
            let url = extract_quoted_url(line)
                .or_else(|| extract_url_token(line))
                .unwrap_or_else(|| line.trim_start_matches("E: ").to_string());
            push_unique(format!("apt: {url}"));
            continue;
        }

        // ── dnf / yum ────────────────────────────────────────────
        if line.contains("Failed to download metadata for repo")
            || line.contains("Cannot download repomd.xml")
            || line.contains("skipping unavailable repo")
            || line.contains("Errors during downloading metadata")
        {
            let id = extract_quoted_url(line).unwrap_or_else(|| line.to_string());
            push_unique(format!("dnf/yum: {id}"));
            continue;
        }

        // ── zypper ───────────────────────────────────────────────
        if line.contains("Repository") && line.contains("is invalid") {
            let id = extract_quoted_url(line).unwrap_or_else(|| line.to_string());
            push_unique(format!("zypper: {id}"));
            continue;
        }
    }
    warnings
}

/// Pull the first single- or double-quoted token out of `line`. Used
/// by [`detect_broken_repo_warnings`] to extract a repo URL or repo id
/// from diagnostic lines like `仓库 "https://download.docker.com/..."
/// 不再含有 Release 文件`.
fn extract_quoted_url(line: &str) -> Option<String> {
    for (open, close) in [('"', '"'), ('\'', '\''), ('“', '”')] {
        if let Some(i) = line.find(open) {
            if let Some(j) = line[i + open.len_utf8()..].find(close) {
                let start = i + open.len_utf8();
                let end = start + j;
                let inner = line[start..end].trim();
                if !inner.is_empty() {
                    return Some(inner.to_string());
                }
            }
        }
    }
    None
}

/// Pull the first `http[s]://` token out of `line` when no quotes
/// surrounded the URL (some apt locales emit `Failed to fetch
/// http://example/...`).
fn extract_url_token(line: &str) -> Option<String> {
    let i = line.find("http").or_else(|| line.find("https"))?;
    let tail = &line[i..];
    let end = tail
        .find(|c: char| c.is_whitespace())
        .unwrap_or(tail.len());
    let url = tail[..end].trim_end_matches(|c: char| matches!(c, ',' | '.' | ';' | ')' | ']'));
    if url.starts_with("http") && url.len() > "http://".len() {
        Some(url.to_string())
    } else {
        None
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
/// Test-only convenience wrapper around [`build_uninstall_command_inner`]
/// without the `cleanup_script` arg. Production callers always go
/// through the inner form so the cleanup chain is plumbed through;
/// the wrapper keeps the existing test fixtures readable.
#[cfg(test)]
fn build_uninstall_command(
    manager: PackageManager,
    packages: &[&str],
    data_dirs: &[&str],
    opts: &UninstallOptions,
    service_unit: Option<&str>,
) -> String {
    build_uninstall_command_inner(manager, packages, data_dirs, opts, service_unit, None)
}

/// Internal: same as [`build_uninstall_command`] but threads through
/// the descriptor's `vendor_script.cleanup_scripts` snippet so the
/// runtime call site can tack it onto the chain. Tests stay on the
/// public 5-arg form to keep their fixtures readable.
fn build_uninstall_command_inner(
    manager: PackageManager,
    packages: &[&str],
    data_dirs: &[&str],
    opts: &UninstallOptions,
    service_unit: Option<&str>,
    cleanup_script: Option<&str>,
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
    // Upstream-source cleanup is best-effort: chain with `; ` so
    // even if the package-manager remove step exited non-zero (e.g.
    // dependency held back) we still try to drop the pgdg.list / repo
    // package the user asked us to remove. We gate this on the user
    // option; the descriptor's data is consulted at the call site.
    if opts.remove_upstream_source {
        if let Some(snippet) = cleanup_script {
            chain.push_str("; ");
            chain.push_str(snippet);
        }
    }

    format!("{service_step}{chain}")
}

/// Build the per-manager "what is the latest installable version of
/// `package`" remote command. Returns the candidate / "Version" line
/// from the package manager's metadata cache. Distinct from
/// [`build_versions_command`], which enumerates every visible version —
/// here we just want the single most-recent line for the row's
/// "最新版" hint.
fn build_candidate_version_command(manager: PackageManager, package: &str) -> Option<String> {
    let pkg = shell_single_quote(package);
    Some(match manager {
        PackageManager::Apt => format!(
            "apt-cache policy {pkg} 2>/dev/null | awk '/Candidate:/ {{print $2; exit}}'"
        ),
        PackageManager::Dnf => format!(
            "dnf info --available {pkg} -q 2>/dev/null | awk -F': *' '/^Version/ {{print $2; exit}}'"
        ),
        PackageManager::Yum => format!(
            "yum info {pkg} -q 2>/dev/null | awk -F': *' '/^Version/ {{print $2; exit}}'"
        ),
        PackageManager::Apk => format!(
            "apk policy {pkg} 2>/dev/null | awk 'NR==2 {{gsub(/[ \\t]+/, \"\"); print; exit}}'"
        ),
        PackageManager::Pacman => format!(
            "pacman -Si {pkg} 2>/dev/null | awk -F': *' '/^Version/ {{print $2; exit}}'"
        ),
        PackageManager::Zypper => format!(
            "zypper info {pkg} 2>/dev/null | awk -F': *' '/^Version/ {{print $2; exit}}'"
        ),
    })
}

/// Per-manager "is `package` installed" check. Used by the variants
/// probe so we can distinguish "Java is installed" (descriptor's
/// generic probe) from "OpenJDK 17 specifically is installed". Always
/// exits 0 or 1; we treat exit 0 as "yes".
fn build_pkg_installed_check(manager: PackageManager, package: &str) -> String {
    let qpkg = shell_single_quote(package);
    match manager {
        PackageManager::Apt => format!(
            "dpkg -l {qpkg} 2>/dev/null | awk 'BEGIN {{f=1}} /^ii/ {{f=0}} END {{exit f}}'"
        ),
        PackageManager::Dnf | PackageManager::Yum | PackageManager::Zypper => {
            format!("rpm -q {qpkg} >/dev/null 2>&1")
        }
        PackageManager::Apk => format!("apk info -e {qpkg} >/dev/null 2>&1"),
        PackageManager::Pacman => format!("pacman -Q {qpkg} >/dev/null 2>&1"),
    }
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
    fn pick_package_manager_chinese_rhel_clones_route_to_dnf() {
        // openEuler / Kylin / Anolis / OpenCloudOS / TencentOS are all
        // RHEL-clone-adjacent and ship dnf. Coverage here is the
        // load-bearing test for the v2.2 distro expansion.
        for id in ["openeuler", "kylin", "anolis", "opencloudos", "tencentos"] {
            assert_eq!(
                pick_package_manager(id),
                Some(PackageManager::Dnf),
                "expected {id} to route to dnf",
            );
        }
    }

    #[test]
    fn pick_package_manager_deepin_uos_route_to_apt() {
        // Deepin (`deepin`) and UOS (`uos`) are Debian-derived even
        // though their docs read like RHEL clones; same matrix as
        // Ubuntu for install commands.
        assert_eq!(pick_package_manager("deepin"), Some(PackageManager::Apt));
        assert_eq!(pick_package_manager("uos"), Some(PackageManager::Apt));
    }

    #[test]
    fn pick_package_manager_unknown_returns_none() {
        assert!(pick_package_manager("solaris").is_none());
        assert!(pick_package_manager("").is_none());
    }

    // ── User-extras parsing ─────────────────────────────────────

    #[test]
    fn user_extras_minimal_entry_parses() {
        let json = r#"[{
            "id": "redis-stack",
            "displayName": "Redis Stack",
            "probeCommand": "command -v redis-stack-server >/dev/null 2>&1 && redis-stack-server --version 2>&1",
            "installPackages": { "apt": ["redis-stack-server"] }
        }]"#;
        let parsed: Vec<UserPackageJson> = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.len(), 1);
        let leaked = validate_and_leak(parsed.into_iter().next().unwrap()).unwrap();
        assert_eq!(leaked.id, "redis-stack");
        assert_eq!(leaked.display_name, "Redis Stack");
        // Manager set should be exactly the one apt entry.
        assert_eq!(leaked.install_packages.len(), 1);
        assert_eq!(leaked.install_packages[0].0, PackageManager::Apt);
        assert_eq!(leaked.install_packages[0].1, &["redis-stack-server"]);
        // Defaults
        assert_eq!(leaked.category, "");
        assert!(leaked.config_paths.is_empty());
        assert!(leaked.version_variants.is_empty());
        assert!(leaked.vendor_script.is_none());
    }

    #[test]
    fn user_extras_full_entry_parses() {
        let json = r#"{
            "id": "fail2ban-pro",
            "displayName": "Fail2Ban Pro",
            "category": "system",
            "binaryName": "fail2ban-pro",
            "probeCommand": "command -v fail2ban-pro >/dev/null && fail2ban-pro -V",
            "installPackages": {
                "apt": ["fail2ban-pro"],
                "dnf": ["fail2ban-pro"]
            },
            "serviceUnits": { "apt": "fail2ban-pro" },
            "supportsReload": true,
            "configPaths": ["/etc/fail2ban-pro/jail.conf"],
            "defaultPorts": [],
            "dataDirs": ["/var/lib/fail2ban-pro"],
            "notes": "third-party fork"
        }"#;
        let raw: UserPackageJson = serde_json::from_str(json).unwrap();
        let leaked = validate_and_leak(raw).unwrap();
        assert_eq!(leaked.category, "system");
        assert!(leaked.supports_reload);
        assert_eq!(leaked.service_units.len(), 1);
        assert_eq!(leaked.config_paths, &["/etc/fail2ban-pro/jail.conf"]);
        assert_eq!(leaked.data_dirs, &["/var/lib/fail2ban-pro"]);
        assert_eq!(leaked.notes, Some("third-party fork"));
    }

    #[test]
    fn user_extras_missing_id_rejected() {
        let raw = UserPackageJson {
            id: String::new(),
            display_name: "X".into(),
            probe_command: "true".into(),
            install_packages: [("apt".into(), vec!["x".into()])].into_iter().collect(),
            notes: None,
            binary_name: None,
            config_paths: vec![],
            default_ports: vec![],
            data_dirs: vec![],
            service_units: Default::default(),
            supports_reload: false,
            category: None,
        };
        assert!(validate_and_leak(raw).is_err());
    }

    #[test]
    fn user_extras_unknown_manager_rejected() {
        let raw = UserPackageJson {
            id: "x".into(),
            display_name: "X".into(),
            probe_command: "true".into(),
            install_packages: [("brew".into(), vec!["x".into()])].into_iter().collect(),
            notes: None,
            binary_name: None,
            config_paths: vec![],
            default_ports: vec![],
            data_dirs: vec![],
            service_units: Default::default(),
            supports_reload: false,
            category: None,
        };
        let err = validate_and_leak(raw).unwrap_err();
        assert!(err.contains("brew"));
    }

    #[test]
    fn user_extras_empty_package_list_rejected() {
        let raw = UserPackageJson {
            id: "x".into(),
            display_name: "X".into(),
            probe_command: "true".into(),
            install_packages: [("apt".into(), vec![])].into_iter().collect(),
            notes: None,
            binary_name: None,
            config_paths: vec![],
            default_ports: vec![],
            data_dirs: vec![],
            service_units: Default::default(),
            supports_reload: false,
            category: None,
        };
        assert!(validate_and_leak(raw).is_err());
    }

    #[test]
    fn build_uninstall_appends_cleanup_when_requested() {
        let opts = UninstallOptions {
            purge_config: false,
            autoremove: false,
            remove_data_dirs: false,
            remove_upstream_source: true,
        };
        let s = build_uninstall_command_inner(
            PackageManager::Apt,
            &["postgresql"],
            &[],
            &opts,
            None,
            Some("rm -f /etc/apt/sources.list.d/pgdg.list"),
        );
        assert!(s.contains("apt-get remove -y postgresql"));
        assert!(s.contains("rm -f /etc/apt/sources.list.d/pgdg.list"));
        // Chained with `;` so a failed remove doesn't stop the cleanup.
        assert!(s.contains("postgresql; rm -f"));
    }

    #[test]
    fn build_uninstall_skips_cleanup_when_option_unset() {
        let opts = UninstallOptions::default();
        let s = build_uninstall_command_inner(
            PackageManager::Apt,
            &["postgresql"],
            &[],
            &opts,
            None,
            Some("rm -f /etc/apt/sources.list.d/pgdg.list"),
        );
        assert!(!s.contains("pgdg.list"));
    }

    #[test]
    fn postgres_cleanup_scripts_cover_apt_and_dnf() {
        let postgres = descriptor("postgres").expect("postgres in registry");
        let vendor = postgres.vendor_script.expect("postgres vendor_script");
        let managers: std::collections::HashSet<PackageManager> =
            vendor.cleanup_scripts.iter().map(|(m, _)| *m).collect();
        assert!(managers.contains(&PackageManager::Apt));
        assert!(managers.contains(&PackageManager::Dnf));
        let apt = vendor
            .cleanup_scripts
            .iter()
            .find(|(m, _)| *m == PackageManager::Apt)
            .map(|(_, s)| *s)
            .unwrap();
        assert!(apt.contains("/etc/apt/sources.list.d/pgdg.list"));
    }

    #[test]
    fn postgres_variants_only_cover_pgdg_managers() {
        // pgdg ships parallel postgresql-N packages for apt / dnf /
        // yum only. The variants intentionally do NOT declare
        // packages for apk / pacman / zypper — picking a variant
        // there would surface as a clean install failure (which is
        // fine; the dialog tells the user pgdg only covers those
        // three families).
        let postgres = descriptor("postgres").expect("postgres in registry");
        assert!(!postgres.version_variants.is_empty());
        for v in postgres.version_variants {
            for m in [PackageManager::Apt, PackageManager::Dnf, PackageManager::Yum] {
                assert!(
                    v.install_packages.iter().any(|(mm, _)| *mm == m),
                    "postgres variant {} missing install_packages for {:?}",
                    v.key,
                    m
                );
            }
            for m in [
                PackageManager::Apk,
                PackageManager::Pacman,
                PackageManager::Zypper,
            ] {
                assert!(
                    !v.install_packages.iter().any(|(mm, _)| *mm == m),
                    "postgres variant {} unexpectedly has install_packages for {:?}",
                    v.key,
                    m,
                );
            }
        }
    }

    #[test]
    fn postgres_setup_scripts_cover_apt_and_dnf() {
        let postgres = descriptor("postgres").expect("postgres in registry");
        let vendor = postgres.vendor_script.expect("postgres vendor_script");
        let managers: std::collections::HashSet<PackageManager> = vendor
            .setup_scripts
            .iter()
            .map(|(m, _)| *m)
            .collect();
        assert!(
            managers.contains(&PackageManager::Apt),
            "postgres setup_scripts missing apt entry"
        );
        assert!(
            managers.contains(&PackageManager::Dnf),
            "postgres setup_scripts missing dnf entry"
        );
        // The apt snippet should write the canonical pgdg list path.
        let apt_script = vendor
            .setup_scripts
            .iter()
            .find(|(m, _)| *m == PackageManager::Apt)
            .map(|(_, s)| *s)
            .unwrap();
        assert!(apt_script.contains("/etc/apt/sources.list.d/pgdg.list"));
        assert!(apt_script.contains("apt.postgresql.org"));
    }

    #[test]
    fn co_install_suggestions_only_reference_built_in_ids() {
        // Suggesting an id that doesn't exist would surface a dead
        // chip in the UI. Catch typos at test time.
        let ids: std::collections::HashSet<&str> =
            REGISTRY.iter().map(|d| d.id).collect();
        for d in REGISTRY {
            for sugg in co_install_suggestions(d.id) {
                assert!(
                    ids.contains(sugg),
                    "co_install_suggestions for {} references unknown id {}",
                    d.id,
                    sugg,
                );
            }
        }
    }

    #[test]
    fn co_install_suggestions_unknown_returns_empty() {
        assert!(co_install_suggestions("not-real").is_empty());
        assert!(co_install_suggestions("").is_empty());
    }

    #[test]
    fn topo_sort_pushes_anchor_before_companion() {
        // co_install_suggestions("docker") includes "compose" and
        // "git". A user-supplied bundle in the wrong order should
        // come back with docker first, compose/git after.
        let sorted = topo_sort_bundle(&["compose", "docker", "git"]);
        let pos = |id: &str| sorted.iter().position(|s| s == id).unwrap();
        assert!(pos("docker") < pos("compose"));
        assert!(pos("docker") < pos("git"));
    }

    #[test]
    fn topo_sort_preserves_input_order_when_no_edges() {
        // None of these ids share a co-install edge — order should
        // come out as input.
        let sorted = topo_sort_bundle(&["sqlite3", "ripgrep", "lsof"]);
        assert_eq!(sorted, vec!["sqlite3", "ripgrep", "lsof"]);
    }

    #[test]
    fn topo_sort_keeps_unrelated_anchors_in_input_order() {
        // Two independent anchors with companions. The two
        // anchor→companion edges should both be respected, and
        // the two anchors should keep their input order relative
        // to each other.
        let sorted = topo_sort_bundle(&["nginx", "compose", "docker", "fail2ban"]);
        let pos = |id: &str| sorted.iter().position(|s| s == id).unwrap();
        assert!(pos("nginx") < pos("fail2ban"));
        assert!(pos("docker") < pos("compose"));
        // nginx came before docker in the input — that ordering
        // should survive since neither has an edge to the other.
        assert!(pos("nginx") < pos("docker"));
    }

    #[test]
    fn topo_sort_returns_every_input_even_under_cycle() {
        // Build a synthetic cycle by abusing two ids that DO point
        // at each other (vim ↔ tmux in the static map). Both end
        // up in the cycle fallback — but the function still
        // returns every input id exactly once.
        let sorted = topo_sort_bundle(&["vim", "tmux"]);
        assert_eq!(sorted.len(), 2);
        let mut s = sorted.clone();
        s.sort();
        assert_eq!(s, vec!["tmux".to_string(), "vim".to_string()]);
    }

    #[test]
    fn topo_sort_handles_empty_input() {
        assert!(topo_sort_bundle(&[]).is_empty());
    }

    #[test]
    fn topo_sort_passes_through_unknown_ids() {
        // Unknown ids have no co-install entry — they should
        // survive intact in input order.
        let sorted = topo_sort_bundle(&["docker", "unknown-pkg", "compose"]);
        assert!(sorted.iter().any(|s| s == "unknown-pkg"));
        assert_eq!(sorted.len(), 3);
        let pos = |id: &str| sorted.iter().position(|s| s == id).unwrap();
        assert!(pos("docker") < pos("compose"));
    }

    #[test]
    fn bundles_have_unique_ids() {
        let mut seen = std::collections::HashSet::new();
        for b in bundles() {
            assert!(seen.insert(b.id), "duplicate bundle id: {}", b.id);
        }
    }

    #[test]
    fn bundles_only_reference_built_in_descriptors() {
        // User extras can be referenced too in production, but the
        // built-in bundles should only point at the static REGISTRY
        // so a fresh install with no extras file still wires up.
        let ids: std::collections::HashSet<&str> =
            REGISTRY.iter().map(|d| d.id).collect();
        for b in bundles() {
            for pkg in b.package_ids {
                assert!(
                    ids.contains(pkg),
                    "bundle {} references unknown id {}",
                    b.id,
                    pkg,
                );
            }
        }
    }

    #[test]
    fn parse_search_output_apt() {
        let raw = "redis-server - Persistent key-value database\nredis-tools - Persistent key-value database (client tools)\n";
        let hits = parse_search_output(PackageManager::Apt, raw, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "redis-server");
        assert_eq!(hits[0].summary, "Persistent key-value database");
        assert_eq!(hits[1].name, "redis-tools");
    }

    #[test]
    fn parse_search_output_apt_respects_limit() {
        let raw = "a - 1\nb - 2\nc - 3\nd - 4\n";
        let hits = parse_search_output(PackageManager::Apt, raw, 2);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn parse_search_output_dnf_strips_arch_and_skips_headers() {
        let raw = "Last metadata expiration check ...\n=== Name & Summary ===\nredis.x86_64 : Fast key-value store\nredis-debuginfo.x86_64 : Debuginfo for redis\n";
        let hits = parse_search_output(PackageManager::Dnf, raw, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "redis");
        assert_eq!(hits[0].summary, "Fast key-value store");
    }

    #[test]
    fn parse_search_output_pacman_two_line_format() {
        let raw = "extra/redis 7.4.0-1\n    A persistent key-value database\nextra/redis-cli 1.0-1\n    CLI for redis\n";
        let hits = parse_search_output(PackageManager::Pacman, raw, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "redis");
        assert_eq!(hits[0].summary, "A persistent key-value database");
    }

    #[test]
    fn parse_search_output_apk_strips_version_tail() {
        let raw = "redis-7.0.15-r0 Persistent key-value db\n";
        let hits = parse_search_output(PackageManager::Apk, raw, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "redis");
    }

    #[test]
    fn parse_user_extras_legacy_array() {
        let json = br#"[{
            "id": "tool",
            "displayName": "Tool",
            "probeCommand": "true",
            "installPackages": { "apt": ["tool"] }
        }]"#;
        let parsed = parse_user_extras_bytes(json).unwrap();
        assert_eq!(parsed.packages.len(), 1);
        assert_eq!(parsed.bundles.len(), 0);
    }

    #[test]
    fn parse_user_extras_wrapper_object() {
        let json = br#"{
            "packages": [{
                "id": "tool",
                "displayName": "Tool",
                "probeCommand": "true",
                "installPackages": { "apt": ["tool"] }
            }],
            "bundles": [{
                "id": "my-stack",
                "displayName": "My Stack",
                "description": "personal favourites",
                "packageIds": ["tool", "git"]
            }]
        }"#;
        let parsed = parse_user_extras_bytes(json).unwrap();
        assert_eq!(parsed.packages.len(), 1);
        assert_eq!(parsed.bundles.len(), 1);
        assert_eq!(parsed.bundles[0].id, "my-stack");
        assert_eq!(parsed.bundles[0].package_ids.len(), 2);
    }

    #[test]
    fn parse_user_extras_wrapper_packages_optional() {
        let json = br#"{
            "bundles": [{
                "id": "b1",
                "displayName": "B1",
                "packageIds": ["git"]
            }]
        }"#;
        let parsed = parse_user_extras_bytes(json).unwrap();
        assert!(parsed.packages.is_empty());
        assert_eq!(parsed.bundles.len(), 1);
    }

    #[test]
    fn validate_bundle_rejects_empty_package_ids() {
        let raw = UserBundleJson {
            id: "x".into(),
            display_name: "X".into(),
            description: String::new(),
            package_ids: vec![],
        };
        assert!(validate_and_leak_bundle(raw).is_err());
    }

    #[test]
    fn validate_bundle_rejects_missing_id() {
        let raw = UserBundleJson {
            id: String::new(),
            display_name: "X".into(),
            description: String::new(),
            package_ids: vec!["git".into()],
        };
        assert!(validate_and_leak_bundle(raw).is_err());
    }

    #[test]
    fn parse_manager_id_known_lowercase() {
        assert_eq!(parse_manager_id("apt"), Some(PackageManager::Apt));
        assert_eq!(parse_manager_id("APT"), Some(PackageManager::Apt));
        assert_eq!(parse_manager_id("dnf"), Some(PackageManager::Dnf));
        assert_eq!(parse_manager_id("brew"), None);
        assert_eq!(parse_manager_id(""), None);
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
        assert!(cmd.starts_with("dnf install -y"));
        assert!(cmd.contains("docker-27.5.1-1.fc40"));
    }

    #[test]
    fn build_install_command_yum_version_pin_uses_dash() {
        let cmd = build_install_command(PackageManager::Yum, &["redis"], false, Some("7.2.4-1"));
        assert!(cmd.starts_with("yum install -y"));
        assert!(cmd.contains("redis-7.2.4-1"));
    }

    #[test]
    fn build_install_command_apt_decouples_update_and_install() {
        // Regression: a stale third-party repo (Docker pulling Ubuntu
        // focal, dormant PPA, etc.) must not gate every install. We
        // run update + install sequentially with `;` so a non-zero
        // update exit lets install still try against the cached
        // package list.
        let cmd = build_install_command(PackageManager::Apt, &["redis-server"], false, None);
        assert!(cmd.contains("apt-get update"));
        assert!(cmd.contains("apt-get install -y redis-server"));
        assert!(
            !cmd.contains("&&"),
            "apt install chain must not gate install on update success: {cmd}"
        );
    }

    #[test]
    fn build_install_command_dnf_skips_unavailable_repo() {
        // Mirror of the apt fix on the rpm side. With
        // `skip_if_unavailable=True`, a single unreachable repo in
        // /etc/yum.repos.d turns into a per-repo warning instead of
        // a global abort.
        let cmd = build_install_command(PackageManager::Dnf, &["redis"], false, None);
        assert!(cmd.contains("--setopt=skip_if_unavailable=True"));
    }

    #[test]
    fn detect_broken_repo_warnings_picks_up_apt_release_loss() {
        let out = "命中:1 https://archive.ubuntu.com/ubuntu focal InRelease\n\
                   忽略:2 https://download.docker.com/linux/ubuntu focal InRelease\n\
                   错误:3 仓库 \"https://download.docker.com/linux/ubuntu focal Release\" 不再含有 Release 文件\n\
                   E: 仓库 \"https://download.docker.com/linux/ubuntu focal Release\" 不再含有 Release 文件";
        let warnings = detect_broken_repo_warnings(out);
        assert!(!warnings.is_empty());
        assert!(
            warnings.iter().any(|w| w.contains("download.docker.com")),
            "expected docker repo to be flagged: {warnings:?}"
        );
    }

    #[test]
    fn detect_broken_repo_warnings_picks_up_apt_404() {
        let out =
            "E: Failed to fetch https://example.invalid/repo/dists/focal/InRelease 404 Not Found";
        let warnings = detect_broken_repo_warnings(out);
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.contains("example.invalid")));
    }

    #[test]
    fn detect_broken_repo_warnings_picks_up_dnf_repo_metadata() {
        let out = "Failed to download metadata for repo 'docker-ce-stable': \
                   Cannot download repomd.xml: All mirrors were tried";
        let warnings = detect_broken_repo_warnings(out);
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.starts_with("dnf/yum:")));
    }

    #[test]
    fn detect_broken_repo_warnings_returns_empty_on_clean_output() {
        let out = "Reading package lists... Done\n\
                   Building dependency tree... Done\n\
                   The following NEW packages will be installed:\n  redis-server";
        assert!(detect_broken_repo_warnings(out).is_empty());
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
    fn supports_reload_set_only_for_zero_downtime_daemons() {
        // Reload semantics are software-specific — most daemons we
        // ship would effectively restart on `reload`. The whitelist
        // is the small set of daemons that genuinely support
        // zero-downtime reload (nginx config, fail2ban jail rules).
        const RELOAD_OK: &[&str] = &["nginx", "fail2ban"];
        for d in registry() {
            let expected = RELOAD_OK.contains(&d.id);
            assert_eq!(
                d.supports_reload, expected,
                "{} supports_reload should be {}",
                d.id, expected,
            );
        }
    }

    // ── Vendor-script command builder ─────────────────────

    #[test]
    fn docker_descriptor_advertises_official_script() {
        let d = descriptor("docker").expect("docker in registry");
        let v = d.vendor_script.expect("docker has vendor_script");
        assert_eq!(v.url, "https://get.docker.com");
        assert!(v.run_as_root);
        assert!(v.conflicts_with_apt);
    }

    #[test]
    fn vendor_script_whitelist() {
        // Vendor scripts run third-party shell with sudo (or as the
        // user). Whitelist the set so a typo doesn't accidentally
        // wire one onto an unaudited entry.
        const SCRIPT_OK: &[&str] = &["docker", "rust", "node", "postgres"];
        for d in registry() {
            if SCRIPT_OK.contains(&d.id) {
                assert!(
                    d.vendor_script.is_some(),
                    "{} must have vendor_script",
                    d.id,
                );
            } else {
                assert!(
                    d.vendor_script.is_none(),
                    "{} should not have a vendor_script",
                    d.id,
                );
            }
        }
    }

    #[test]
    fn install_status_cancelled_serializes_with_kebab_kind() {
        // The `kind` discriminant is what crosses the IPC seam to the
        // Tauri view layer — if this changes, the frontend dispatch on
        // `report.status` silently misses the cancelled branch.
        let json = serde_json::to_string(&InstallStatus::Cancelled).unwrap();
        assert_eq!(json, r#"{"kind":"cancelled"}"#);
    }

    #[test]
    fn uninstall_status_cancelled_serializes_with_kebab_kind() {
        let json = serde_json::to_string(&UninstallStatus::Cancelled).unwrap();
        assert_eq!(json, r#"{"kind":"cancelled"}"#);
    }

    #[test]
    fn vendor_download_command_quotes_url_and_path() {
        let cmd = build_vendor_download_command(
            "https://get.docker.com",
            "/tmp/pier-x-installer-docker.sh",
        );
        assert!(cmd.contains("curl -fsSL "));
        assert!(cmd.contains("'https://get.docker.com'"));
        assert!(cmd.contains("-o '/tmp/pier-x-installer-docker.sh'"));
    }

    #[test]
    fn vendor_download_command_escapes_quotes_in_url() {
        // Defense-in-depth: even though the registry URLs are static
        // literals, make sure our quoter handles a hostile value
        // without breaking out of the single-quote — internal `'`
        // gets rewritten as the canonical `'\''` close-escape-reopen
        // sequence, and the `-o <path>` argument stays positionally
        // separate from the URL.
        let cmd = build_vendor_download_command(
            "https://evil.example/x';rm -rf /;'",
            "/tmp/x.sh",
        );
        // Embedded quotes were escaped via the close-escape-reopen
        // dance — the literal `'\''` token has to appear at least
        // once.
        assert!(cmd.contains("'\\''"));
        // The `-o` flag must remain a separate argument: the URL's
        // closing quote followed by whitespace then `-o '/tmp/x.sh'`.
        assert!(cmd.contains("' -o '/tmp/x.sh'"));
        // And the curl invocation prefix must still parse cleanly.
        assert!(cmd.starts_with("curl -fsSL '"));
    }

    #[test]
    fn vendor_exec_command_traps_cleanup_and_size_checks() {
        let cmd = build_vendor_exec_command("/tmp/pier-x-installer-docker.sh");
        assert!(cmd.contains("trap 'rm -f"));
        assert!(cmd.contains("EXIT"));
        // Size-check defends against a 200-with-empty-body proxy.
        assert!(cmd.contains("[ ! -s "));
        // Plain `sh path` — never `bash -c "$(curl ...)"` and never a
        // pipe to sh.
        assert!(cmd.contains("sh '/tmp/pier-x-installer-docker.sh'"));
        assert!(!cmd.contains("| sh"));
        assert!(!cmd.contains("$(curl"));
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
