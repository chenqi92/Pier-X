// Read-only data sources for the shell.
//
// Wraps pier-core (git status, saved connections) plus a std::fs directory
// listing for the local Files tree, so shell.rs renders from plain owned
// structs and never touches the backend directly. All calls here are
// synchronous and cheap (single git invocation / one read_dir / one JSON
// load); the shell calls them on construction and on demand, not per frame.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use pier_core::connections::ConnectionStore;
pub use pier_core::services::git::{CommitInfo, StashEntry};
use pier_core::services::git::{FileStatus, GitClient};
use pier_core::services::local_monitor;
use pier_core::ssh::{AuthMethod, HostKeyVerifier, SshConfig, SshSession};

/// One entry in the Files sidebar.
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    /// Relative modification age, e.g. "2h", "3d".
    pub age: String,
    /// File size in bytes; `None` for directories.
    pub size: Option<u64>,
}

/// Which authentication method a saved connection uses, for the
/// sidebar's auth badge.
#[derive(Clone, Copy, PartialEq)]
pub enum AuthKind {
    Password,
    Key,
    Agent,
}

/// One saved SSH connection in the Servers sidebar.
pub struct ConnRow {
    pub name: String,
    pub addr: String,
    /// Host and user kept separately so the sidebar can filter on them.
    pub host: String,
    pub user: String,
    pub auth: AuthKind,
    pub online: bool,
}

/// A single changed file, carrying its porcelain status for mark/colour.
pub struct GitChange {
    pub status: FileStatus,
    pub path: String,
}

/// The current repository's status, as the Git panel renders it.
pub struct GitData {
    pub branch: String,
    pub tracking: String,
    pub ahead: i32,
    pub behind: i32,
    pub staged: Vec<GitChange>,
    pub unstaged: Vec<GitChange>,
}

/// The process working directory (the repo we were launched in).
pub fn current_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Directory entries, directories first then files, each case-insensitively
/// sorted. Hidden entries are kept (the web sidebar shows `.git`, `.github`).
pub fn list_dir(path: &Path) -> Vec<FileEntry> {
    let mut entries: Vec<FileEntry> = Vec::new();
    if let Ok(read) = fs::read_dir(path) {
        for e in read.flatten() {
            let md = e.metadata().ok();
            let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let age = md
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(rel_age)
                .unwrap_or_default();
            let size = if is_dir {
                None
            } else {
                md.as_ref().map(|m| m.len())
            };
            entries.push(FileEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir,
                age,
                size,
            });
        }
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

/// Raw saved SSH configs (for panels that need to connect, not just display).
pub fn connections_raw() -> Vec<SshConfig> {
    ConnectionStore::load_default()
        .map(|s| s.connections)
        .unwrap_or_default()
}

/// Append a connection to the default store and persist it.
pub fn add_connection(cfg: SshConfig) -> Result<(), String> {
    let mut store = ConnectionStore::load_default().unwrap_or_default();
    store.add(cfg);
    store.save_default().map_err(|e| e.to_string())
}

/// Open a blocking SSH session to `cfg`. Trust-on-first-use host-key policy
/// (logs the fingerprint) — fine for the spike. Run OFF the render path: this
/// blocks on the network. Returns a display error string on failure.
pub fn connect_blocking(cfg: &SshConfig) -> Result<SshSession, String> {
    SshSession::connect_blocking(cfg, HostKeyVerifier::accept_all_log_fingerprint())
        .map_err(|e| e.to_string())
}

/// Saved connections from the default store, or empty if none/unreadable.
/// Online state is unknown here (no probe), so it's reported false.
pub fn load_connections() -> Vec<ConnRow> {
    let Ok(store) = ConnectionStore::load_default() else {
        return Vec::new();
    };
    store
        .connections
        .iter()
        .map(|c| ConnRow {
            name: c.name.clone(),
            addr: format!("{}@{}:{}", c.user, c.host, c.port),
            host: c.host.clone(),
            user: c.user.clone(),
            auth: auth_kind(&c.auth),
            online: false,
        })
        .collect()
}

/// Collapse the full [`AuthMethod`] enum into the three badge buckets
/// the sidebar distinguishes.
fn auth_kind(auth: &AuthMethod) -> AuthKind {
    match auth {
        AuthMethod::PublicKeyFile { .. } => AuthKind::Key,
        AuthMethod::Agent | AuthMethod::Auto | AuthMethod::AutoChain { .. } => AuthKind::Agent,
        AuthMethod::KeychainPassword { .. } | AuthMethod::DirectPassword { .. } => {
            AuthKind::Password
        }
    }
}

/// Remove the saved connection at `index` and persist. Out-of-range
/// indices are a no-op (idempotent).
pub fn remove_connection(index: usize) -> Result<(), String> {
    let mut store = ConnectionStore::load_default().map_err(|e| e.to_string())?;
    store.remove(index);
    store.save_default().map_err(|e| e.to_string())
}

/// Replace the saved connection at `index` with `cfg` and persist.
pub fn update_connection(index: usize, cfg: SshConfig) -> Result<(), String> {
    let mut store = ConnectionStore::load_default().map_err(|e| e.to_string())?;
    if index >= store.connections.len() {
        return Err("connection index out of range".to_string());
    }
    store.connections[index] = cfg;
    store.save_default().map_err(|e| e.to_string())
}

fn favorites_path() -> Option<PathBuf> {
    pier_core::paths::config_dir().map(|d| d.join("pier-x-gpui-favorites.conf"))
}

/// Favorite connection names, persisted locally (one name per line).
pub fn load_favorites() -> HashSet<String> {
    let mut set = HashSet::new();
    let Some(p) = favorites_path() else {
        return set;
    };
    if let Ok(text) = std::fs::read_to_string(&p) {
        for line in text.lines() {
            let l = line.trim();
            if !l.is_empty() {
                set.insert(l.to_string());
            }
        }
    }
    set
}

/// Persist the favorite-connection name set (best-effort).
pub fn save_favorites(favs: &HashSet<String>) {
    let Some(p) = favorites_path() else {
        return;
    };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = favs.iter().cloned().collect::<Vec<_>>().join("\n");
    let _ = std::fs::write(&p, body);
}

/// Git status for the repo at `path`, or `None` if it isn't a repo.
pub fn git_status(path: &Path) -> Option<GitData> {
    let client = GitClient::open(&path.to_string_lossy()).ok()?;
    let branch = client.branch_info().ok()?;
    let changes = client.status().ok()?;
    let (mut staged, mut unstaged) = (Vec::new(), Vec::new());
    for c in changes {
        let change = GitChange {
            status: c.status,
            path: c.path,
        };
        if c.staged {
            staged.push(change);
        } else {
            unstaged.push(change);
        }
    }
    Some(GitData {
        branch: branch.name,
        tracking: branch.tracking,
        ahead: branch.ahead,
        behind: branch.behind,
        staged,
        unstaged,
    })
}

/// One mounted filesystem's usage, for the Monitor disk table.
pub struct DiskInfo {
    pub mount: String,
    /// Pre-humanized used / total (e.g. "12 GB").
    pub used: String,
    pub total: String,
    pub use_pct: f64,
}

/// One process row for the Monitor top-process tables.
pub struct ProcInfo {
    pub name: String,
    /// Pre-formatted percentages (e.g. "12.3").
    pub cpu: String,
    pub mem: String,
}

/// Live local-host metrics for the Monitor panel.
pub struct MonStat {
    pub cpu_pct: f64,
    pub cpu_count: u32,
    pub mem_used_mb: f64,
    pub mem_total_mb: f64,
    pub swap_used_mb: f64,
    pub swap_total_mb: f64,
    pub proc_count: u32,
    pub uptime: String,
    pub os_label: String,
    /// 1/5/15-minute load average, or None on platforms without it (Windows).
    pub load: Option<(f64, f64, f64)>,
    /// Per-mount disk usage.
    pub disks: Vec<DiskInfo>,
    /// Aggregate network rate in bytes/sec; negative while warming up.
    pub net_rx_bps: f64,
    pub net_tx_bps: f64,
    /// Top processes by CPU and by memory.
    pub top_cpu: Vec<ProcInfo>,
    pub top_mem: Vec<ProcInfo>,
}

/// Sample the local host once. Backed by `sysinfo`, so CPU/mem/uptime are
/// real on every platform; load average is Unix-only.
pub fn monitor_snapshot() -> MonStat {
    let s = local_monitor::collect_snapshot(true);
    let load = if s.load_1 < 0.0 {
        None
    } else {
        Some((s.load_1, s.load_5, s.load_15))
    };
    let disks = s
        .disks
        .iter()
        .map(|d| DiskInfo {
            mount: d.mountpoint.clone(),
            used: d.used.clone(),
            total: d.total.clone(),
            use_pct: d.use_pct.max(0.0),
        })
        .collect();
    let top_cpu = s
        .top_processes
        .iter()
        .take(6)
        .map(|p| ProcInfo {
            name: p.command.clone(),
            cpu: p.cpu_pct.clone(),
            mem: p.mem_pct.clone(),
        })
        .collect();
    let top_mem = s
        .top_processes_mem
        .iter()
        .take(6)
        .map(|p| ProcInfo {
            name: p.command.clone(),
            cpu: p.cpu_pct.clone(),
            mem: p.mem_pct.clone(),
        })
        .collect();
    MonStat {
        cpu_pct: s.cpu_pct.max(0.0),
        cpu_count: s.cpu_count,
        mem_used_mb: s.mem_used_mb,
        mem_total_mb: s.mem_total_mb,
        swap_used_mb: s.swap_used_mb,
        swap_total_mb: s.swap_total_mb,
        proc_count: s.proc_count,
        uptime: s.uptime,
        os_label: s.os_label,
        load,
        disks,
        net_rx_bps: s.net_rx_bps,
        net_tx_bps: s.net_tx_bps,
        top_cpu,
        top_mem,
    }
}

/// Recent commits (newest first) for the repo at `path`, empty if not a repo.
pub fn git_log(path: &Path, limit: usize) -> Vec<CommitInfo> {
    GitClient::open(&path.to_string_lossy())
        .and_then(|c| c.log(limit))
        .unwrap_or_default()
}

/// Local branch names for the repo at `path`.
pub fn git_branches(path: &Path) -> Vec<String> {
    GitClient::open(&path.to_string_lossy())
        .and_then(|c| c.branch_list())
        .unwrap_or_default()
}

/// Stash entries for the repo at `path`.
pub fn git_stash(path: &Path) -> Vec<StashEntry> {
    GitClient::open(&path.to_string_lossy())
        .and_then(|c| c.stash_list())
        .unwrap_or_default()
}

/// Stage a single path. Fast local op — safe to call from a click handler.
pub fn git_stage(repo: &Path, file: &str) -> Result<(), String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .stage(&[file.to_string()])
        .map_err(|e| e.to_string())
}

/// Unstage a single path.
pub fn git_unstage(repo: &Path, file: &str) -> Result<(), String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .unstage(&[file.to_string()])
        .map_err(|e| e.to_string())
}

/// Discard worktree changes to a single path (destructive).
pub fn git_discard(repo: &Path, file: &str) -> Result<(), String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .discard(&[file.to_string()])
        .map_err(|e| e.to_string())
}

/// Commit staged changes with `message`; returns the new commit hash.
pub fn git_commit(repo: &Path, message: &str) -> Result<String, String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .commit(message)
        .map_err(|e| e.to_string())
}

/// `git push` for the repo at `path` (network — run off the render path).
pub fn git_push(path: &Path) -> Result<String, String> {
    GitClient::open(&path.to_string_lossy())
        .map_err(|e| e.to_string())?
        .push()
        .map_err(|e| e.to_string())
}

/// `git pull` for the repo at `path` (network — run off the render path).
pub fn git_pull(path: &Path) -> Result<String, String> {
    GitClient::open(&path.to_string_lossy())
        .map_err(|e| e.to_string())?
        .pull()
        .map_err(|e| e.to_string())
}

/// A `git` subprocess rooted at `repo`, with the console window
/// suppressed on Windows so a GUI launch never flashes a `cmd`
/// window (mirrors pier-core's `configure_background_command`,
/// which is crate-private). The GitClient wrappers above already
/// hide it; these direct calls cover the few subcommands GitClient
/// doesn't expose (numstat / fetch / rebase).
fn git_command(repo: &Path, args: &[&str]) -> Command {
    let mut c = Command::new("git");
    c.current_dir(repo);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Per-file added/deleted line counts, merging the worktree
/// (`git diff --numstat`) and index (`--cached`) views keyed by
/// repo-root-relative path. Binary files (numstat `-`) count as
/// zero. Best-effort: a non-repo or git failure yields an empty
/// map, so the Git panel just omits the inline `+N -N`.
pub fn git_numstat(repo: &Path) -> HashMap<String, (u32, u32)> {
    let mut out: HashMap<String, (u32, u32)> = HashMap::new();
    for cached in [false, true] {
        let mut args = vec!["diff", "--numstat"];
        if cached {
            args.push("--cached");
        }
        let Ok(output) = git_command(repo, &args).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let mut parts = line.splitn(3, '\t');
            let (Some(add), Some(del), Some(path)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let entry = out.entry(path.to_string()).or_insert((0, 0));
            entry.0 += add.parse::<u32>().unwrap_or(0);
            entry.1 += del.parse::<u32>().unwrap_or(0);
        }
    }
    out
}

/// Run a `git` subcommand, returning trimmed stdout on success or
/// stderr (falling back to stdout) on failure. Console-suppressed.
fn run_git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_command(repo, args)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

/// `git fetch --prune` for the repo (network — run off the render path).
pub fn git_fetch(repo: &Path) -> Result<String, String> {
    run_git(repo, &["fetch", "--prune"])
}

/// `git rebase` onto the current branch's configured upstream (run
/// off the render path). Conflicts/errors come back as the Err string.
pub fn git_rebase(repo: &Path) -> Result<String, String> {
    run_git(repo, &["rebase"])
}

/// Switch the working tree to `branch` (local, fast). Fails when the
/// worktree has conflicting changes — error surfaced to the caller.
pub fn git_checkout(repo: &Path, branch: &str) -> Result<String, String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .checkout_branch(branch)
        .map_err(|e| e.to_string())
}

/// The repo's configured git identity `(user.name, user.email)` as git
/// resolves it (local + global merged); empty strings when unset.
///
/// Best-effort and deliberately error-swallowing: a missing key, a
/// non-repo, or a failed `git` all collapse to an empty string, since the
/// Settings panel only displays the identity and has no error surface.
pub fn git_identity(repo: &Path) -> (String, String) {
    let name = run_git(repo, &["config", "user.name"]).unwrap_or_default();
    let email = run_git(repo, &["config", "user.email"]).unwrap_or_default();
    (name, email)
}

/// Persisted shell layout/state, restored on launch. Terminals aren't restored
/// (PTYs are live), but tool/panel/theme/widths/cwd are.
pub struct UiState {
    pub active_tool: usize,
    pub right_collapsed: bool,
    pub show_servers: bool,
    pub dark: bool,
    pub sidebar_w: Option<f32>,
    pub right_w: Option<f32>,
    pub cwd: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            active_tool: 1, // Git
            right_collapsed: false,
            show_servers: false,
            dark: true,
            sidebar_w: None,
            right_w: None,
            cwd: String::new(),
        }
    }
}

fn ui_state_path() -> Option<std::path::PathBuf> {
    pier_core::paths::config_dir().map(|d| d.join("pier-x-gpui-ui.conf"))
}

/// Load persisted UI state, or defaults if absent/unreadable.
pub fn load_ui_state() -> UiState {
    let mut s = UiState::default();
    let Some(p) = ui_state_path() else { return s };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return s;
    };
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k {
            "active_tool" => {
                if let Ok(n) = v.parse() {
                    s.active_tool = n;
                }
            }
            "right_collapsed" => s.right_collapsed = v == "true",
            "show_servers" => s.show_servers = v == "true",
            "dark" => s.dark = v == "true",
            "sidebar_w" => s.sidebar_w = v.parse().ok(),
            "right_w" => s.right_w = v.parse().ok(),
            "cwd" => s.cwd = v.to_string(),
            _ => {}
        }
    }
    s
}

/// Persist UI state (best-effort; ignores IO errors).
pub fn save_ui_state(s: &UiState) {
    let Some(p) = ui_state_path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let opt = |o: Option<f32>| o.map(|v| v.to_string()).unwrap_or_default();
    let body = format!(
        "active_tool={}\nright_collapsed={}\nshow_servers={}\ndark={}\nsidebar_w={}\nright_w={}\ncwd={}\n",
        s.active_tool,
        s.right_collapsed,
        s.show_servers,
        s.dark,
        opt(s.sidebar_w),
        opt(s.right_w),
        s.cwd,
    );
    let _ = std::fs::write(&p, body);
}

fn rel_age(t: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        86_400..=604_799 => format!("{}d", secs / 86_400),
        604_800..=2_591_999 => format!("{}w", secs / 604_800),
        _ => format!("{}mo", secs / 2_592_000),
    }
}

// ── Software panel — clipboard command builders ──────────────────────
//
// The GPUI Software panel is command-injection only: it never runs a
// privileged package-manager / systemctl command itself. Each row's action
// button copies the exact shell command to the clipboard so the user can
// paste it into a terminal. These helpers format those commands for the
// host's detected package manager (the same per-manager mapping pier-core's
// install/uninstall services use internally, in the plain user-typed form
// rather than the non-interactive automation wrapping).

/// A package lifecycle action whose command the Software panel copies to the
/// clipboard.
#[derive(Clone, Copy)]
pub enum PkgAction {
    Install,
    Update,
    Uninstall,
}

/// Format the paste-able package-manager command for `action` on `pkgs`,
/// matching the host's detected manager id (`apt` / `dnf` / `yum` / `apk` /
/// `pacman` / `zypper`). Prefixed with `sudo ` unless the remote session is
/// root. Returns `None` for an unrecognised manager id or an empty package
/// list.
pub fn pkg_command(
    manager: &str,
    action: PkgAction,
    pkgs: &[String],
    is_root: bool,
) -> Option<String> {
    if pkgs.is_empty() {
        return None;
    }
    let list = pkgs.join(" ");
    let body = match manager {
        "apt" => match action {
            PkgAction::Install => format!("apt install {list}"),
            PkgAction::Update => format!("apt install --only-upgrade {list}"),
            PkgAction::Uninstall => format!("apt remove {list}"),
        },
        "dnf" => match action {
            PkgAction::Install => format!("dnf install {list}"),
            PkgAction::Update => format!("dnf upgrade {list}"),
            PkgAction::Uninstall => format!("dnf remove {list}"),
        },
        "yum" => match action {
            PkgAction::Install => format!("yum install {list}"),
            PkgAction::Update => format!("yum update {list}"),
            PkgAction::Uninstall => format!("yum remove {list}"),
        },
        "apk" => match action {
            PkgAction::Install => format!("apk add {list}"),
            PkgAction::Update => format!("apk add --upgrade {list}"),
            PkgAction::Uninstall => format!("apk del {list}"),
        },
        "pacman" => match action {
            PkgAction::Install => format!("pacman -S {list}"),
            PkgAction::Update => format!("pacman -S {list}"),
            PkgAction::Uninstall => format!("pacman -R {list}"),
        },
        "zypper" => match action {
            PkgAction::Install => format!("zypper install {list}"),
            PkgAction::Update => format!("zypper update {list}"),
            PkgAction::Uninstall => format!("zypper remove {list}"),
        },
        _ => return None,
    };
    let prefix = if is_root { "" } else { "sudo " };
    Some(format!("{prefix}{body}"))
}

/// Format the paste-able `systemctl <verb> <unit>` command, prefixed with
/// `sudo ` unless the remote session is root.
pub fn systemctl_command(verb: &str, unit: &str, is_root: bool) -> String {
    let prefix = if is_root { "" } else { "sudo " };
    format!("{prefix}systemctl {verb} {unit}")
}
