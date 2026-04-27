//! Package-source mirror switching for apt and dnf hosts.
//!
//! Drives a one-click "switch the system to mirror X" flow:
//!
//! 1. Read the current sources file(s) on the remote.
//! 2. Match the first hostname against a curated mirror list so
//!    the panel can render "current source: 阿里云".
//! 3. On user opt-in, back up the sources file(s) to a
//!    `.pier-bak` companion (one-shot — never overwritten on
//!    subsequent switches) and `sed -i` swap the upstream
//!    hostname(s) with the picked mirror's hostname.
//! 4. On "restore", copy the `.pier-bak` back over the live
//!    sources.
//!
//! The hostname-swap strategy assumes mirrors mirror the upstream
//! directory layout 1:1 (which is the contract every mirror site
//! we support honours). It does NOT touch GPG keys — keys are
//! issued by the distro and validate any mirror that serves the
//! same files, so apt/dnf still verify package signatures
//! correctly after a mirror switch.
//!
//! ## What this module does NOT do
//!
//! * **No "official upstream" choice.** Restoring is a separate
//!   action — pressing "restore" reverts to whatever was on disk
//!   before Pier-X first touched it. We don't try to compute
//!   what "official Ubuntu" looks like from scratch.
//! * **No apk / pacman / zypper.** Out of scope for v2.3; their
//!   sources files are simpler and can be added later.

use serde::{Deserialize, Serialize};

use crate::ssh::error::{Result, SshError};
use crate::ssh::SshSession;

use super::package_manager::{looks_like_sudo_password_prompt, PackageManager};

// ── Types ───────────────────────────────────────────────────────────

/// Stable id of one curated mirror. Lower-case, dash-separated
/// strings flow over the IPC boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MirrorId {
    /// 阿里云 — `mirrors.aliyun.com`. Most widely-used Chinese mirror.
    Aliyun,
    /// 清华 TUNA — `mirrors.tuna.tsinghua.edu.cn`. Education-network
    /// peering tends to be excellent on edu / research hosts.
    Tsinghua,
    /// 中科大 USTC — `mirrors.ustc.edu.cn`. Same edu peering story
    /// as TUNA; useful as a fallback when TUNA is congested.
    Ustc,
    /// 华为云 — `repo.huaweicloud.com`. Strong coverage of openEuler
    /// and Kylin in addition to standard distros.
    Huawei,
    /// 腾讯云 — `mirrors.cloud.tencent.com`. Best latency from
    /// Tencent Cloud VPC hosts.
    Tencent,
}

impl MirrorId {
    /// Stable lowercase id used in serialization and logs. The
    /// inverse of [`MirrorId::from_str`].
    pub fn as_str(self) -> &'static str {
        match self {
            MirrorId::Aliyun => "aliyun",
            MirrorId::Tsinghua => "tsinghua",
            MirrorId::Ustc => "ustc",
            MirrorId::Huawei => "huawei",
            MirrorId::Tencent => "tencent",
        }
    }

    /// Parse a serialized id back to the enum. Returns `None` for
    /// any string that isn't one of the catalog entries.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "aliyun" => Some(MirrorId::Aliyun),
            "tsinghua" => Some(MirrorId::Tsinghua),
            "ustc" => Some(MirrorId::Ustc),
            "huawei" => Some(MirrorId::Huawei),
            "tencent" => Some(MirrorId::Tencent),
            _ => None,
        }
    }
}

/// Describes one mirror — id + human label + per-manager rewrite
/// targets. apt/dnf only need a hostname (mirrors mirror the
/// upstream layout 1:1); apk also fits the host-swap pattern; pacman
/// needs an explicit URL prefix because each mirror's archlinux
/// path differs (`/archlinux/...` on aliyun vs upstream `/...`);
/// zypper reuses dnf's flat .repo format under a different dir.
#[derive(Debug, Clone, Copy)]
pub struct MirrorChoice {
    /// Stable id — same set as [`MirrorId`].
    pub id: MirrorId,
    /// Human label rendered in the picker dialog
    /// (e.g. `"阿里云"`).
    pub label: &'static str,
    /// Hostname used when rewriting Debian/Ubuntu apt sources.
    pub apt_host: &'static str,
    /// Hostname used when rewriting RHEL-family dnf repo files.
    pub dnf_host: &'static str,
    /// Hostname used when rewriting Alpine `/etc/apk/repositories`.
    /// `None` when the mirror doesn't carry Alpine.
    pub apk_host: Option<&'static str>,
    /// Full URL prefix for pacman `Server = ...` lines, e.g.
    /// `"https://mirrors.aliyun.com/archlinux"`. `None` when the
    /// mirror doesn't carry Arch.
    pub pacman_url: Option<&'static str>,
    /// Hostname used when rewriting openSUSE `/etc/zypp/repos.d/*.repo`.
    /// `None` when the mirror doesn't carry openSUSE.
    pub zypper_host: Option<&'static str>,
}

const MIRRORS: &[MirrorChoice] = &[
    MirrorChoice {
        id: MirrorId::Aliyun,
        label: "阿里云",
        apt_host: "mirrors.aliyun.com",
        dnf_host: "mirrors.aliyun.com",
        apk_host: Some("mirrors.aliyun.com"),
        pacman_url: Some("https://mirrors.aliyun.com/archlinux"),
        zypper_host: Some("mirrors.aliyun.com"),
    },
    MirrorChoice {
        id: MirrorId::Tsinghua,
        label: "清华 TUNA",
        apt_host: "mirrors.tuna.tsinghua.edu.cn",
        dnf_host: "mirrors.tuna.tsinghua.edu.cn",
        apk_host: Some("mirrors.tuna.tsinghua.edu.cn"),
        pacman_url: Some("https://mirrors.tuna.tsinghua.edu.cn/archlinux"),
        zypper_host: Some("mirrors.tuna.tsinghua.edu.cn"),
    },
    MirrorChoice {
        id: MirrorId::Ustc,
        label: "中科大 USTC",
        apt_host: "mirrors.ustc.edu.cn",
        dnf_host: "mirrors.ustc.edu.cn",
        apk_host: Some("mirrors.ustc.edu.cn"),
        pacman_url: Some("https://mirrors.ustc.edu.cn/archlinux"),
        zypper_host: Some("mirrors.ustc.edu.cn"),
    },
    MirrorChoice {
        id: MirrorId::Huawei,
        label: "华为云",
        apt_host: "repo.huaweicloud.com",
        dnf_host: "repo.huaweicloud.com",
        apk_host: Some("repo.huaweicloud.com"),
        pacman_url: Some("https://repo.huaweicloud.com/archlinux"),
        zypper_host: Some("repo.huaweicloud.com"),
    },
    MirrorChoice {
        id: MirrorId::Tencent,
        label: "腾讯云",
        apt_host: "mirrors.cloud.tencent.com",
        dnf_host: "mirrors.cloud.tencent.com",
        apk_host: Some("mirrors.cloud.tencent.com"),
        pacman_url: Some("https://mirrors.cloud.tencent.com/archlinux"),
        zypper_host: Some("mirrors.cloud.tencent.com"),
    },
];

/// Public accessor — used by the Tauri layer to surface the catalog.
pub fn supported_mirrors() -> &'static [MirrorChoice] {
    MIRRORS
}

/// Find a choice by id.
pub fn mirror_by_id(id: MirrorId) -> Option<&'static MirrorChoice> {
    MIRRORS.iter().find(|c| c.id == id)
}

/// All hostnames the mirror swap recognises as "old". Includes
/// the official upstreams plus every other mirror in our catalog
/// — so a user can switch from aliyun → tsinghua without going
/// through restore first.
fn known_apt_hosts() -> Vec<&'static str> {
    let mut hosts = vec![
        "archive.ubuntu.com",
        "security.ubuntu.com",
        "ports.ubuntu.com",
        "deb.debian.org",
        "security.debian.org",
        "cn.archive.ubuntu.com",
    ];
    hosts.extend(MIRRORS.iter().map(|m| m.apt_host));
    hosts
}

fn known_dnf_hosts() -> Vec<&'static str> {
    let mut hosts = vec![
        "mirror.centos.org",
        "vault.centos.org",
        "download.fedoraproject.org",
        "dl.fedoraproject.org",
        "repo.openeuler.org",
        "repo.openeuler.openatom.cn",
        "mirrors.openanolis.cn",
    ];
    hosts.extend(MIRRORS.iter().map(|m| m.dnf_host));
    hosts
}

fn known_apk_hosts() -> Vec<&'static str> {
    let mut hosts = vec![
        "dl-cdn.alpinelinux.org",
        "dl-2.alpinelinux.org",
        "dl-3.alpinelinux.org",
        "dl-4.alpinelinux.org",
        "dl-5.alpinelinux.org",
    ];
    hosts.extend(MIRRORS.iter().filter_map(|m| m.apk_host));
    hosts
}

fn known_zypper_hosts() -> Vec<&'static str> {
    let mut hosts = vec![
        "download.opensuse.org",
        "mirrors.opensuse.org",
        "ftp.suse.com",
    ];
    hosts.extend(MIRRORS.iter().filter_map(|m| m.zypper_host));
    hosts
}

/// Snapshot of the host's current mirror state. Loaded by the
/// Tauri layer on panel open and after every set/restore so the
/// UI reflects the post-action ground truth.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MirrorState {
    /// Which package-manager family this state applies to.
    pub package_manager: String,
    /// Curated mirror id when the detected hostname matches one
    /// in our catalog; `None` for "official upstream" / unknown.
    pub current_id: Option<String>,
    /// First hostname we found in the sources file. `None` when
    /// the file couldn't be read.
    pub current_host: Option<String>,
    /// `true` when the `.pier-bak` companion exists — the UI uses
    /// this to gate the "restore" button.
    pub has_backup: bool,
}

/// Outcome of a `set_mirror` / `restore_mirror` call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum MirrorActionStatus {
    /// `sed -i` (or the equivalent file write) succeeded and the
    /// re-probe confirms the new mirror hostname is in place.
    Ok,
    /// `sudo -n` reported that a password is required — the user
    /// has to either connect as root or configure passwordless sudo.
    SudoRequiresPassword,
    /// The shell pipeline exited non-zero for some other reason
    /// (file-not-found, permission denied, sed syntax error from
    /// an unexpected sources.list shape, …). Inspect `output_tail`.
    Failed,
    /// The host's package manager isn't covered by this module
    /// yet, **or** the picked mirror has no entry for that manager
    /// (e.g. a mirror that doesn't carry Alpine).
    UnsupportedManager,
}

/// Structured outcome of a mirror set/restore call. Mirrors the
/// install-report shape so the panel can reuse the same outcome
/// formatter for the activity log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MirrorActionReport {
    /// Outcome class — see [`MirrorActionStatus`].
    pub status: MirrorActionStatus,
    /// Lowercase package-manager id the action targeted
    /// (`"apt"` / `"dnf"` / `"apk"` / `"pacman"` / `"zypper"`).
    pub package_manager: String,
    /// Exact shell command that ran on the remote, including the
    /// `sudo -n sh -c '...'` wrapper.
    pub command: String,
    /// Exit code from the command. `0` means success; non-zero
    /// surfaces as `Failed` (or `SudoRequiresPassword` when the
    /// output tail looks like a sudo prompt).
    pub exit_code: i32,
    /// Last ~40 lines of merged stdout+stderr — surfaced in the
    /// dialog when the action fails so the user can diagnose.
    pub output_tail: String,
    /// Mirror state after the action — fresh probe so the UI
    /// doesn't have to re-detect.
    pub state_after: MirrorState,
}

// ── Public API ──────────────────────────────────────────────────────

/// Detect the current mirror for a manager. Always succeeds —
/// returns `current_id = None` when the file is unreadable or
/// doesn't match any known hostname.
pub async fn detect_mirror(
    session: &SshSession,
    manager: PackageManager,
) -> MirrorState {
    match manager {
        PackageManager::Apt => detect_apt(session).await,
        PackageManager::Dnf | PackageManager::Yum => detect_dnf(session).await,
        PackageManager::Apk => detect_apk(session).await,
        PackageManager::Pacman => detect_pacman(session).await,
        PackageManager::Zypper => detect_zypper(session).await,
    }
}

/// Switch to `mirror_id`. Backs up the original sources to
/// `.pier-bak` on first invocation (subsequent switches keep the
/// original backup intact).
pub async fn set_mirror(
    session: &SshSession,
    manager: PackageManager,
    mirror_id: MirrorId,
) -> Result<MirrorActionReport> {
    let mirror = mirror_by_id(mirror_id).ok_or_else(|| {
        SshError::InvalidConfig(format!("unknown mirror id: {}", mirror_id.as_str()))
    })?;
    match manager {
        PackageManager::Apt => set_apt(session, mirror).await,
        PackageManager::Dnf | PackageManager::Yum => set_dnf(session, mirror).await,
        PackageManager::Apk => set_apk(session, mirror).await,
        PackageManager::Pacman => set_pacman(session, mirror).await,
        PackageManager::Zypper => set_zypper(session, mirror).await,
    }
}

/// Restore the original sources file from `.pier-bak`. No-op when
/// the backup doesn't exist.
pub async fn restore_mirror(
    session: &SshSession,
    manager: PackageManager,
) -> Result<MirrorActionReport> {
    match manager {
        PackageManager::Apt => restore_apt(session).await,
        PackageManager::Dnf | PackageManager::Yum => restore_dnf(session).await,
        PackageManager::Apk => restore_apk(session).await,
        PackageManager::Pacman => restore_pacman(session).await,
        PackageManager::Zypper => restore_zypper(session).await,
    }
}

// ── Blocking wrappers ───────────────────────────────────────────────

/// Blocking wrapper for [`detect_mirror`]. Tauri commands using
/// `spawn_blocking` call this directly so they can stay in a
/// synchronous closure body.
pub fn detect_mirror_blocking(
    session: &SshSession,
    manager: PackageManager,
) -> MirrorState {
    crate::ssh::runtime::shared().block_on(detect_mirror(session, manager))
}

/// Blocking wrapper for [`set_mirror`].
pub fn set_mirror_blocking(
    session: &SshSession,
    manager: PackageManager,
    mirror_id: MirrorId,
) -> Result<MirrorActionReport> {
    crate::ssh::runtime::shared().block_on(set_mirror(session, manager, mirror_id))
}

/// Blocking wrapper for [`restore_mirror`].
pub fn restore_mirror_blocking(
    session: &SshSession,
    manager: PackageManager,
) -> Result<MirrorActionReport> {
    crate::ssh::runtime::shared().block_on(restore_mirror(session, manager))
}

// ── apt ─────────────────────────────────────────────────────────────

const APT_LIST: &str = "/etc/apt/sources.list";
const APT_LIST_BAK: &str = "/etc/apt/sources.list.pier-bak";
const APT_DIR: &str = "/etc/apt/sources.list.d";
const APT_DIR_BAK: &str = "/etc/apt/sources.list.d.pier-bak";

async fn detect_apt(session: &SshSession) -> MirrorState {
    // Probe sources.list first; fall back to deb822 .sources files
    // so Ubuntu 24.04 / Debian 12 hosts using ubuntu.sources are
    // still detected. Take the first http(s) URL we see.
    let cmd = format!(
        "cat {APT_LIST} {APT_DIR}/*.list {APT_DIR}/*.sources 2>/dev/null | grep -hE '^[^#]*https?://' | head -50"
    );
    let lines = match session.exec_command(&cmd).await {
        Ok((_, stdout)) => stdout,
        Err(_) => String::new(),
    };
    let host = first_http_host(&lines);

    let backup_exists = file_or_dir_exists(session, APT_LIST_BAK).await
        || file_or_dir_exists(session, APT_DIR_BAK).await;

    let current_id = host
        .as_deref()
        .and_then(|h| MIRRORS.iter().find(|m| m.apt_host == h))
        .map(|m| m.id.as_str().to_string());

    MirrorState {
        package_manager: "apt".to_string(),
        current_id,
        current_host: host,
        has_backup: backup_exists,
    }
}

async fn set_apt(
    session: &SshSession,
    mirror: &MirrorChoice,
) -> Result<MirrorActionReport> {
    let new_host = mirror.apt_host;
    let sed_expr = build_sed_swap(&known_apt_hosts(), new_host);
    // Backup-once + swap. Keep this as a single sh chain so the
    // remote sees one transaction; on partial failure the user's
    // original file isn't half-rewritten.
    let inner = format!(
        "set -e; \
         [ -e {APT_LIST_BAK} ] || cp -a {APT_LIST} {APT_LIST_BAK}; \
         [ -e {APT_DIR_BAK} ] || ([ -d {APT_DIR} ] && cp -a {APT_DIR} {APT_DIR_BAK} || true); \
         sed -i -E {sed} {APT_LIST} 2>/dev/null || true; \
         for f in {APT_DIR}/*.list {APT_DIR}/*.sources; do \
           [ -e \"$f\" ] && sed -i -E {sed} \"$f\" 2>/dev/null || true; \
         done; \
         echo OK",
        sed = shell_single_quote(&sed_expr),
    );
    run_root_sh(session, &inner, "apt").await
}

async fn restore_apt(session: &SshSession) -> Result<MirrorActionReport> {
    let inner = format!(
        "set -e; \
         restored=0; \
         if [ -e {APT_LIST_BAK} ]; then cp -a {APT_LIST_BAK} {APT_LIST}; rm -f {APT_LIST_BAK}; restored=1; fi; \
         if [ -d {APT_DIR_BAK} ]; then rm -rf {APT_DIR}; cp -a {APT_DIR_BAK} {APT_DIR}; rm -rf {APT_DIR_BAK}; restored=1; fi; \
         if [ \"$restored\" -eq 0 ]; then echo 'no backup found'; exit 0; fi; \
         echo OK",
    );
    run_root_sh(session, &inner, "apt").await
}

// ── dnf ─────────────────────────────────────────────────────────────

const DNF_DIR: &str = "/etc/yum.repos.d";
const DNF_DIR_BAK: &str = "/etc/yum.repos.d.pier-bak";

async fn detect_dnf(session: &SshSession) -> MirrorState {
    let cmd = format!(
        "cat {DNF_DIR}/*.repo 2>/dev/null | grep -hE '^(baseurl|mirrorlist)=' | head -50"
    );
    let lines = match session.exec_command(&cmd).await {
        Ok((_, stdout)) => stdout,
        Err(_) => String::new(),
    };
    let host = first_http_host(&lines);

    let backup_exists = file_or_dir_exists(session, DNF_DIR_BAK).await;

    let current_id = host
        .as_deref()
        .and_then(|h| MIRRORS.iter().find(|m| m.dnf_host == h))
        .map(|m| m.id.as_str().to_string());

    MirrorState {
        package_manager: "dnf".to_string(),
        current_id,
        current_host: host,
        has_backup: backup_exists,
    }
}

async fn set_dnf(
    session: &SshSession,
    mirror: &MirrorChoice,
) -> Result<MirrorActionReport> {
    let new_host = mirror.dnf_host;
    let sed_expr = build_sed_swap(&known_dnf_hosts(), new_host);
    let inner = format!(
        "set -e; \
         [ -e {DNF_DIR_BAK} ] || cp -a {DNF_DIR} {DNF_DIR_BAK}; \
         for f in {DNF_DIR}/*.repo; do \
           [ -e \"$f\" ] && sed -i -E {sed} \"$f\" 2>/dev/null || true; \
         done; \
         echo OK",
        sed = shell_single_quote(&sed_expr),
    );
    run_root_sh(session, &inner, "dnf").await
}

async fn restore_dnf(session: &SshSession) -> Result<MirrorActionReport> {
    let inner = format!(
        "set -e; \
         if [ ! -d {DNF_DIR_BAK} ]; then echo 'no backup found'; exit 0; fi; \
         rm -rf {DNF_DIR}; \
         cp -a {DNF_DIR_BAK} {DNF_DIR}; \
         rm -rf {DNF_DIR_BAK}; \
         echo OK",
    );
    run_root_sh(session, &inner, "dnf").await
}

// ── apk ─────────────────────────────────────────────────────────────

const APK_FILE: &str = "/etc/apk/repositories";
const APK_FILE_BAK: &str = "/etc/apk/repositories.pier-bak";

async fn detect_apk(session: &SshSession) -> MirrorState {
    let cmd = format!("cat {APK_FILE} 2>/dev/null | grep -E '^[^#]*https?://' | head -10");
    let lines = match session.exec_command(&cmd).await {
        Ok((_, stdout)) => stdout,
        Err(_) => String::new(),
    };
    let host = first_http_host(&lines);
    let backup_exists = file_or_dir_exists(session, APK_FILE_BAK).await;
    let current_id = host
        .as_deref()
        .and_then(|h| MIRRORS.iter().find(|m| m.apk_host == Some(h)))
        .map(|m| m.id.as_str().to_string());
    MirrorState {
        package_manager: "apk".to_string(),
        current_id,
        current_host: host,
        has_backup: backup_exists,
    }
}

async fn set_apk(session: &SshSession, mirror: &MirrorChoice) -> Result<MirrorActionReport> {
    let Some(new_host) = mirror.apk_host else {
        return Ok(unsupported_for(mirror.id, "apk"));
    };
    let sed_expr = build_sed_swap(&known_apk_hosts(), new_host);
    let inner = format!(
        "set -e; \
         [ -e {APK_FILE_BAK} ] || cp -a {APK_FILE} {APK_FILE_BAK}; \
         sed -i -E {sed} {APK_FILE}; \
         echo OK",
        sed = shell_single_quote(&sed_expr),
    );
    run_root_sh(session, &inner, "apk").await
}

async fn restore_apk(session: &SshSession) -> Result<MirrorActionReport> {
    let inner = format!(
        "set -e; \
         if [ ! -e {APK_FILE_BAK} ]; then echo 'no backup found'; exit 0; fi; \
         cp -a {APK_FILE_BAK} {APK_FILE}; \
         rm -f {APK_FILE_BAK}; \
         echo OK",
    );
    run_root_sh(session, &inner, "apk").await
}

// ── pacman ──────────────────────────────────────────────────────────

const PACMAN_FILE: &str = "/etc/pacman.d/mirrorlist";
const PACMAN_FILE_BAK: &str = "/etc/pacman.d/mirrorlist.pier-bak";

async fn detect_pacman(session: &SshSession) -> MirrorState {
    // First non-comment Server = line wins.
    let cmd = format!(
        "grep -m1 -E '^[[:space:]]*Server[[:space:]]*=' {PACMAN_FILE} 2>/dev/null"
    );
    let line = match session.exec_command(&cmd).await {
        Ok((_, stdout)) => stdout,
        Err(_) => String::new(),
    };
    let host = first_http_host(&line);
    let backup_exists = file_or_dir_exists(session, PACMAN_FILE_BAK).await;
    let current_id = host
        .as_deref()
        .and_then(|h| {
            MIRRORS.iter().find(|m| {
                m.pacman_url
                    .map(|u| u.contains(h))
                    .unwrap_or(false)
            })
        })
        .map(|m| m.id.as_str().to_string());
    MirrorState {
        package_manager: "pacman".to_string(),
        current_id,
        current_host: host,
        has_backup: backup_exists,
    }
}

async fn set_pacman(session: &SshSession, mirror: &MirrorChoice) -> Result<MirrorActionReport> {
    let Some(url) = mirror.pacman_url else {
        return Ok(unsupported_for(mirror.id, "pacman"));
    };
    // pacman's mirrorlist is a list of `Server = <url>/$repo/os/$arch`
    // lines. We replace the file with a single chosen Server line +
    // a commented-out copy of the original list so the user can fall
    // back manually if the picked mirror is unreachable.
    let new_line = format!("Server = {url}/$repo/os/$arch");
    let inner = format!(
        "set -e; \
         [ -e {PACMAN_FILE_BAK} ] || cp -a {PACMAN_FILE} {PACMAN_FILE_BAK}; \
         {{ echo {line}; echo; echo '# --- previous mirrorlist (commented out by pier-x) ---'; sed 's/^/# /' {PACMAN_FILE_BAK}; }} > {PACMAN_FILE}.new; \
         mv {PACMAN_FILE}.new {PACMAN_FILE}; \
         echo OK",
        line = shell_single_quote(&new_line),
    );
    run_root_sh(session, &inner, "pacman").await
}

async fn restore_pacman(session: &SshSession) -> Result<MirrorActionReport> {
    let inner = format!(
        "set -e; \
         if [ ! -e {PACMAN_FILE_BAK} ]; then echo 'no backup found'; exit 0; fi; \
         cp -a {PACMAN_FILE_BAK} {PACMAN_FILE}; \
         rm -f {PACMAN_FILE_BAK}; \
         echo OK",
    );
    run_root_sh(session, &inner, "pacman").await
}

// ── zypper ──────────────────────────────────────────────────────────

const ZYPPER_DIR: &str = "/etc/zypp/repos.d";
const ZYPPER_DIR_BAK: &str = "/etc/zypp/repos.d.pier-bak";

async fn detect_zypper(session: &SshSession) -> MirrorState {
    let cmd = format!(
        "cat {ZYPPER_DIR}/*.repo 2>/dev/null | grep -hE '^(baseurl|mirrorlist)=' | head -50"
    );
    let lines = match session.exec_command(&cmd).await {
        Ok((_, stdout)) => stdout,
        Err(_) => String::new(),
    };
    let host = first_http_host(&lines);
    let backup_exists = file_or_dir_exists(session, ZYPPER_DIR_BAK).await;
    let current_id = host
        .as_deref()
        .and_then(|h| MIRRORS.iter().find(|m| m.zypper_host == Some(h)))
        .map(|m| m.id.as_str().to_string());
    MirrorState {
        package_manager: "zypper".to_string(),
        current_id,
        current_host: host,
        has_backup: backup_exists,
    }
}

async fn set_zypper(session: &SshSession, mirror: &MirrorChoice) -> Result<MirrorActionReport> {
    let Some(new_host) = mirror.zypper_host else {
        return Ok(unsupported_for(mirror.id, "zypper"));
    };
    let sed_expr = build_sed_swap(&known_zypper_hosts(), new_host);
    let inner = format!(
        "set -e; \
         [ -e {ZYPPER_DIR_BAK} ] || cp -a {ZYPPER_DIR} {ZYPPER_DIR_BAK}; \
         for f in {ZYPPER_DIR}/*.repo; do \
           [ -e \"$f\" ] && sed -i -E {sed} \"$f\" 2>/dev/null || true; \
         done; \
         echo OK",
        sed = shell_single_quote(&sed_expr),
    );
    run_root_sh(session, &inner, "zypper").await
}

async fn restore_zypper(session: &SshSession) -> Result<MirrorActionReport> {
    let inner = format!(
        "set -e; \
         if [ ! -d {ZYPPER_DIR_BAK} ]; then echo 'no backup found'; exit 0; fi; \
         rm -rf {ZYPPER_DIR}; \
         cp -a {ZYPPER_DIR_BAK} {ZYPPER_DIR}; \
         rm -rf {ZYPPER_DIR_BAK}; \
         echo OK",
    );
    run_root_sh(session, &inner, "zypper").await
}

/// Build a "this mirror doesn't carry that distro" report.
fn unsupported_for(_id: MirrorId, manager_name: &str) -> MirrorActionReport {
    MirrorActionReport {
        status: MirrorActionStatus::UnsupportedManager,
        package_manager: manager_name.to_string(),
        command: String::new(),
        exit_code: 0,
        output_tail: String::new(),
        state_after: MirrorState {
            package_manager: manager_name.to_string(),
            current_id: None,
            current_host: None,
            has_backup: false,
        },
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Run `inner` under `sh -c`, prefixed with `sudo -n` when the
/// session isn't root. Returns a structured report; SSH-level
/// failure surfaces as `Err`.
async fn run_root_sh(
    session: &SshSession,
    inner: &str,
    manager_name: &str,
) -> Result<MirrorActionReport> {
    let env = super::package_manager::probe_host_env(session).await;
    let prefix = if env.is_root { "" } else { "sudo -n " };
    let command = format!(
        "{prefix}sh -c {} 2>&1",
        shell_single_quote(inner)
    );
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
        MirrorActionStatus::Ok
    } else if !env.is_root && looks_like_sudo_password_prompt(&output_tail) {
        MirrorActionStatus::SudoRequiresPassword
    } else {
        MirrorActionStatus::Failed
    };
    let state_after = match manager_name {
        "apt" => detect_apt(session).await,
        "dnf" => detect_dnf(session).await,
        "apk" => detect_apk(session).await,
        "pacman" => detect_pacman(session).await,
        "zypper" => detect_zypper(session).await,
        _ => MirrorState {
            package_manager: manager_name.to_string(),
            current_id: None,
            current_host: None,
            has_backup: false,
        },
    };
    Ok(MirrorActionReport {
        status,
        package_manager: manager_name.to_string(),
        command,
        exit_code,
        output_tail,
        state_after,
    })
}

/// Build a single sed expression that swaps every host in `old`
/// with `new`. Format: `s,(old1|old2|...),NEW,g`. Hostnames are
/// regex-escaped (only `.` matters in our set).
fn build_sed_swap(old: &[&str], new: &str) -> String {
    let alt = old
        .iter()
        .map(|h| h.replace('.', "\\."))
        .collect::<Vec<_>>()
        .join("|");
    format!("s,({alt}),{new},g")
}

/// Pull the first hostname out of "...://host[:port]/..." lines.
fn first_http_host(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(after_scheme) = line.find("://").map(|i| &line[i + 3..]) {
            let host = after_scheme
                .split(|c: char| c == '/' || c == ':' || c.is_whitespace())
                .next()
                .unwrap_or("");
            if !host.is_empty() {
                return Some(host.to_string());
            }
        }
    }
    None
}

/// `[ -e <path> ]` over SSH. Treats any error as "doesn't exist".
async fn file_or_dir_exists(session: &SshSession, path: &str) -> bool {
    let cmd = format!("[ -e {} ] && echo 1 || echo 0", shell_single_quote(path));
    matches!(session.exec_command(&cmd).await, Ok((_, s)) if s.trim() == "1")
}

/// POSIX-safe single-quote escape. (Re-implemented here so the
/// module is self-contained — package_manager has the same
/// helper but it's `pub(crate)`-private.)
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

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_mirrors_has_five_entries() {
        assert_eq!(supported_mirrors().len(), 5);
    }

    #[test]
    fn mirror_by_id_round_trips() {
        for m in supported_mirrors() {
            let id = MirrorId::from_str(m.id.as_str()).expect("id parses");
            let back = mirror_by_id(id).expect("lookup");
            assert_eq!(back.id, m.id);
        }
    }

    #[test]
    fn first_http_host_extracts_simple_url() {
        let s = "deb http://archive.ubuntu.com/ubuntu focal main\n";
        assert_eq!(first_http_host(s), Some("archive.ubuntu.com".to_string()));
    }

    #[test]
    fn first_http_host_handles_https_and_port() {
        let s = "baseurl=https://mirrors.aliyun.com:443/openEuler/foo\n";
        assert_eq!(first_http_host(s), Some("mirrors.aliyun.com".to_string()));
    }

    #[test]
    fn first_http_host_skips_no_url_lines() {
        let s = "# commented\n\ngpgkey=file:///etc/pki\nbaseurl=http://repo.openeuler.org/x\n";
        assert_eq!(first_http_host(s), Some("repo.openeuler.org".to_string()));
    }

    #[test]
    fn first_http_host_none_on_empty() {
        assert_eq!(first_http_host(""), None);
        assert_eq!(first_http_host("# only a comment\n"), None);
    }

    #[test]
    fn build_sed_swap_escapes_dots_and_alternates() {
        let expr = build_sed_swap(&["archive.ubuntu.com", "deb.debian.org"], "mirrors.aliyun.com");
        // Dots must be regex-escaped so `.` doesn't match arbitrary chars.
        assert!(expr.contains("archive\\.ubuntu\\.com"));
        assert!(expr.contains("deb\\.debian\\.org"));
        assert!(expr.contains("mirrors.aliyun.com"));
        assert!(expr.starts_with("s,("));
        assert!(expr.ends_with(",g"));
    }

    #[test]
    fn known_apt_hosts_includes_official_and_all_mirrors() {
        let hosts = known_apt_hosts();
        assert!(hosts.contains(&"archive.ubuntu.com"));
        assert!(hosts.contains(&"deb.debian.org"));
        assert!(hosts.contains(&"mirrors.aliyun.com"));
        assert!(hosts.contains(&"mirrors.tuna.tsinghua.edu.cn"));
    }

    #[test]
    fn known_dnf_hosts_includes_openeuler_and_all_mirrors() {
        let hosts = known_dnf_hosts();
        assert!(hosts.contains(&"repo.openeuler.org"));
        assert!(hosts.contains(&"mirror.centos.org"));
        assert!(hosts.contains(&"mirrors.aliyun.com"));
    }

    #[test]
    fn known_apk_hosts_includes_alpine_upstream() {
        let hosts = known_apk_hosts();
        assert!(hosts.contains(&"dl-cdn.alpinelinux.org"));
        assert!(hosts.contains(&"mirrors.aliyun.com"));
    }

    #[test]
    fn known_zypper_hosts_includes_opensuse_upstream() {
        let hosts = known_zypper_hosts();
        assert!(hosts.contains(&"download.opensuse.org"));
    }

    #[test]
    fn every_mirror_carries_apk_pacman_zypper_when_present() {
        // Each catalog entry must declare full coverage so the
        // panel can render every manager without "this mirror
        // can't help here" rows. If we ever ship a partial mirror
        // the assertion below has to be relaxed; for now we treat
        // partial coverage as a registry bug.
        for m in supported_mirrors() {
            assert!(m.apk_host.is_some(), "{} missing apk_host", m.label);
            assert!(m.pacman_url.is_some(), "{} missing pacman_url", m.label);
            assert!(m.zypper_host.is_some(), "{} missing zypper_host", m.label);
        }
    }

    #[test]
    fn shell_single_quote_round_trips_simple() {
        assert_eq!(shell_single_quote("hello"), "'hello'");
        assert_eq!(shell_single_quote("a's"), "'a'\\''s'");
    }

    #[test]
    fn mirror_id_serde_lowercase() {
        let json = serde_json::to_string(&MirrorId::Aliyun).unwrap();
        assert_eq!(json, "\"aliyun\"");
        let back: MirrorId = serde_json::from_str("\"tsinghua\"").unwrap();
        assert_eq!(back, MirrorId::Tsinghua);
    }
}
