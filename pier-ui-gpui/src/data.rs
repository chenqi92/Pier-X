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
use std::sync::Arc;
use std::time::SystemTime;

use futures::channel::{mpsc, oneshot};
use pier_core::connections::ConnectionStore;
pub use pier_core::services::git::{BlameLine, CommitInfo, ConfigEntry, RemoteInfo, StashEntry, TagInfo};
use pier_core::services::git::{FileStatus, GitClient};
use pier_core::services::host_health::{self, HealthStatus, HostHealthTarget};
use pier_core::services::local_monitor;
pub use pier_core::ssh::service_detector::{DetectedService, ServiceStatus};
// `HostKeyPromptCb` is the async callback the verifier consults on unknown /
// changed keys; the dialog/shell import the request + decision types straight
// from `pier_core::ssh`, so they're not re-exported here.
use pier_core::ssh::{
    AuthMethod, HostKeyDecision, HostKeyPromptCb, HostKeyPromptRequest, HostKeyVerifier, SshConfig,
    SshSession,
};

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
    /// TCP reachability of the host: `None` until probed, then `Some(true)`
    /// reachable / `Some(false)` not. Filled in asynchronously by the shell's
    /// background health probe (see `Shell::probe_connections_async`).
    pub online: Option<bool>,
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

/// Open a blocking SSH session to `cfg` with the escape-hatch verifier that
/// accepts any host key and logs its fingerprint. Used by the side-session
/// probes (Test-connection + the read-only panels: docker / firewall / sftp /
/// db / webserver / search / software) which have no overlay to prompt
/// through — the interactive terminal path uses [`connect_blocking_prompt`]
/// instead, which routes unknown / changed keys to the user. Run OFF the
/// render path: this blocks on the network. Returns a display error on failure.
pub fn connect_blocking(cfg: &SshConfig) -> Result<SshSession, String> {
    SshSession::connect_blocking(cfg, HostKeyVerifier::accept_all_log_fingerprint())
        .map_err(|e| e.to_string())
}

/// A pending host-key decision handed from a background connect task to the
/// shell: the [`HostKeyPromptRequest`] to display plus the oneshot the connect
/// task is blocked on. The shell shows [`crate::dialogs::HostKeyDialog`] and
/// sends the user's [`HostKeyDecision`] back through the sender; **dropping the
/// sender resolves to `Reject`**, so a dismissed dialog (scrim click, Esc, app
/// exit) never leaves the connect task hung.
pub type HostKeyPrompt = (HostKeyPromptRequest, oneshot::Sender<HostKeyDecision>);

/// Open a blocking SSH session to `cfg`, routing unknown / changed host keys
/// through `prompt_tx` for interactive confirmation. Backed by the real
/// `~/.ssh/known_hosts` verifier ([`HostKeyVerifier::default`]): already-known
/// and previously-accepted hosts connect silently, and only a first-contact
/// (unknown) key or a changed key sends a [`HostKeyPrompt`] and blocks until
/// the user — or a dropped channel — decides.
///
/// MUST run off the render path: it blocks on both the network and the user's
/// decision. The block happens on the caller's (background) thread; the prompt
/// callback ships the request to the shell over `prompt_tx` and parks on a
/// oneshot, leaving the UI thread free to paint the dialog and collect the
/// click. A send failure (shell gone) or a dropped decision sender both resolve
/// to [`HostKeyDecision::Reject`]. Returns a display error on failure (an
/// implicit/explicit rejection surfaces as the verifier's typed error string).
pub fn connect_blocking_prompt(
    cfg: &SshConfig,
    prompt_tx: mpsc::UnboundedSender<HostKeyPrompt>,
) -> Result<SshSession, String> {
    let cb: HostKeyPromptCb = Arc::new(move |req: HostKeyPromptRequest| {
        let tx = prompt_tx.clone();
        Box::pin(async move {
            let (decision_tx, decision_rx) = oneshot::channel();
            // Shell gone before we could ask → fail safe (reject).
            if tx.unbounded_send((req, decision_tx)).is_err() {
                return HostKeyDecision::Reject;
            }
            // Park here (on the background connect thread, never the UI thread)
            // until the user decides; a dropped sender — dismissed dialog or app
            // exit — resolves to Reject so this task is never orphaned.
            decision_rx.await.unwrap_or(HostKeyDecision::Reject)
        })
    });
    SshSession::connect_blocking(cfg, HostKeyVerifier::default().with_prompt(cb))
        .map_err(|e| e.to_string())
}

/// Saved connections from the default store, or empty if none/unreadable.
/// Online state is unknown here (no probe), so it's reported as `None`
/// (unprobed) — [`probe_connections`] fills it in later.
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
            online: None,
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

/// TCP-probe every saved connection and report which are reachable, as
/// `(saved_connection_index, online)` pairs where `online` is true only for
/// [`HealthStatus::Online`]. One target per saved connection (host + port; a
/// 0 port resolves to 22 inside the probe). Probes run in parallel on the
/// shared runtime but the call blocks, so run it off the render path. Indices
/// line up with [`load_connections`] (same store, same order).
pub fn probe_connections(timeout_ms: u32) -> Vec<(usize, bool)> {
    let targets: Vec<HostHealthTarget> = connections_raw()
        .iter()
        .enumerate()
        .map(|(i, c)| HostHealthTarget {
            saved_connection_index: i,
            host: c.host.clone(),
            port: c.port,
        })
        .collect();
    if targets.is_empty() {
        return Vec::new();
    }
    host_health::probe_many_blocking(targets, timeout_ms)
        .into_iter()
        .map(|r| (r.saved_connection_index, r.status == HealthStatus::Online))
        .collect()
}

/// Detect the well-known services (mysql / redis / postgresql / docker) on the
/// host behind `session`, reusing its already-authenticated connection. Runs a
/// handful of SSH execs and blocks, so call it off the render path.
pub fn detect_services(session: &SshSession) -> Vec<DetectedService> {
    pier_core::ssh::service_detector::detect_all_blocking(session)
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

fn sftp_bookmarks_path() -> Option<PathBuf> {
    pier_core::paths::config_dir().map(|d| d.join("pier-x-gpui-sftp-bookmarks.conf"))
}

/// SFTP directory bookmarks for `host_key` (`"user@host:port"`). The file holds
/// one `host_key\tpath` line per bookmark across every host; this returns just
/// the paths saved for the given host, in file order.
pub fn load_sftp_bookmarks(host_key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(p) = sftp_bookmarks_path() else {
        return out;
    };
    if let Ok(text) = std::fs::read_to_string(&p) {
        for line in text.lines() {
            if let Some((k, path)) = line.split_once('\t') {
                if k == host_key && !path.is_empty() {
                    out.push(path.to_string());
                }
            }
        }
    }
    out
}

/// Replace `host_key`'s bookmark set with `paths`, preserving every other
/// host's lines (best-effort; ignores IO errors).
pub fn save_sftp_bookmarks(host_key: &str, paths: &[String]) {
    let Some(p) = sftp_bookmarks_path() else {
        return;
    };
    // Carry over other hosts' lines untouched, then append this host's set.
    let mut lines: Vec<String> = Vec::new();
    if let Ok(text) = std::fs::read_to_string(&p) {
        for line in text.lines() {
            let other_host = match line.split_once('\t') {
                Some((k, _)) => k != host_key,
                None => false,
            };
            if other_host {
                lines.push(line.to_string());
            }
        }
    }
    for path in paths {
        lines.push(format!("{host_key}\t{path}"));
    }
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&p, lines.join("\n"));
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

/// One submodule entry from `git submodule status`. `status` is the human
/// label derived from the leading porcelain char (' ' ok, '-' uninitialized,
/// '+' modified, 'U' conflict).
pub struct SubmoduleInfo {
    pub path: String,
    pub short_hash: String,
    pub status: String,
}

// ── Git: tags / remotes / config / diff / blame / submodules ─────────
//
// Thin wrappers over the GitClient methods of the same name, plus the
// subprocess `git submodule …` for the one area GitClient doesn't cover.
// List reads swallow errors to an empty Vec (the panel just shows
// nothing); mutations surface the error string for the panel to display.

/// Tags for the repo, empty if not a repo.
pub fn git_tags(repo: &Path) -> Vec<TagInfo> {
    GitClient::open(&repo.to_string_lossy())
        .and_then(|c| c.tag_list())
        .unwrap_or_default()
}

/// Create a tag. Empty `message` → lightweight tag, else annotated.
pub fn git_tag_create(repo: &Path, name: &str, message: &str) -> Result<String, String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .tag_create(name, message)
        .map_err(|e| e.to_string())
}

/// Delete a tag.
pub fn git_tag_delete(repo: &Path, name: &str) -> Result<String, String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .tag_delete(name)
        .map_err(|e| e.to_string())
}

/// Remotes (name + fetch/push URL) for the repo, empty if not a repo.
pub fn git_remotes(repo: &Path) -> Vec<RemoteInfo> {
    GitClient::open(&repo.to_string_lossy())
        .and_then(|c| c.remote_list())
        .unwrap_or_default()
}

/// Add a remote.
pub fn git_remote_add(repo: &Path, name: &str, url: &str) -> Result<String, String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .remote_add(name, url)
        .map_err(|e| e.to_string())
}

/// Remove a remote.
pub fn git_remote_remove(repo: &Path, name: &str) -> Result<String, String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .remote_remove(name)
        .map_err(|e| e.to_string())
}

/// Merged local+global git config entries, empty if not a repo.
pub fn git_config_list(repo: &Path) -> Vec<ConfigEntry> {
    GitClient::open(&repo.to_string_lossy())
        .and_then(|c| c.config_list())
        .unwrap_or_default()
}

/// Set a config value (`global` → `--global`, else repo-local).
pub fn git_config_set(repo: &Path, key: &str, value: &str, global: bool) -> Result<String, String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .config_set(key, value, global)
        .map_err(|e| e.to_string())
}

/// Unset a config value (`global` → `--global`, else repo-local).
pub fn git_config_unset(repo: &Path, key: &str, global: bool) -> Result<String, String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .config_unset(key, global)
        .map_err(|e| e.to_string())
}

/// Unified diff for one file. `staged` → index-vs-HEAD, else worktree-vs-index.
pub fn git_diff(repo: &Path, path: &str, staged: bool) -> Result<String, String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .diff(path, staged)
        .map_err(|e| e.to_string())
}

/// Pseudo-diff for an untracked file (its full content as additions).
pub fn git_diff_untracked(repo: &Path, path: &str) -> Result<String, String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .diff_untracked(path)
        .map_err(|e| e.to_string())
}

/// Blame annotation for a file, or the error (e.g. binary / unknown path).
pub fn git_blame(repo: &Path, path: &str) -> Result<Vec<BlameLine>, String> {
    GitClient::open(&repo.to_string_lossy())
        .map_err(|e| e.to_string())?
        .blame(path)
        .map_err(|e| e.to_string())
}

/// Submodules via `git submodule status --recursive`, empty if none/not a repo.
pub fn git_submodules(repo: &Path) -> Vec<SubmoduleInfo> {
    let Ok(out) = run_git(repo, &["submodule", "status", "--recursive"]) else {
        return Vec::new();
    };
    out.lines()
        .filter_map(|line| {
            if line.trim().is_empty() {
                return None;
            }
            let symbol = line.chars().next()?;
            let rest = line.get(1..)?.trim();
            let mut parts = rest.split_whitespace();
            let hash = parts.next()?.to_string();
            let path = parts.next()?.to_string();
            let status = match symbol {
                '-' => "uninitialized",
                '+' => "modified",
                'U' => "conflict",
                _ => "ok",
            };
            Some(SubmoduleInfo {
                short_hash: hash.chars().take(7).collect(),
                path,
                status: status.to_string(),
            })
        })
        .collect()
}

/// `git submodule init` (local).
pub fn git_submodule_init(repo: &Path) -> Result<String, String> {
    run_git(repo, &["submodule", "init"])
}

/// `git submodule update --init --recursive` (clones — network, run off render).
pub fn git_submodule_update(repo: &Path) -> Result<String, String> {
    run_git(repo, &["submodule", "update", "--init", "--recursive"])
}

/// `git submodule sync --recursive` (local).
pub fn git_submodule_sync(repo: &Path) -> Result<String, String> {
    run_git(repo, &["submodule", "sync", "--recursive"])
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
    /// Interface language code ("en" / "zh"); see `crate::i18n`.
    pub lang: String,
    /// Terminal smart-mode: OSC 133 shell integration + completion / syntax /
    /// autosuggest overlays. Opt-in (PRODUCT-SPEC §4.2.1), off by default.
    pub smart_mode: bool,
}

/// Process-global terminal smart-mode flag, mirroring how `i18n` holds the
/// interface language: loaded from `UiState` at startup, flipped from Settings,
/// and read by `TerminalView` when a shell is created.
static SMART_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether terminal smart-mode is currently enabled.
pub fn smart_mode() -> bool {
    SMART_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Set the process-global smart-mode flag (does not persist on its own —
/// callers also write `UiState`).
pub fn set_smart_mode(on: bool) {
    SMART_MODE.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Global smart-mode command-history ring (most-recent-first, deduped, capped).
/// Cross-tab by design — a command typed in one terminal is suggestible in
/// another, matching the Tauri frontend's global ring.
static HISTORY_RING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
const HISTORY_CAP: usize = 500;

/// Record a freshly-submitted command in the history ring.
pub fn history_push(cmd: &str) {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return;
    }
    if let Ok(mut ring) = HISTORY_RING.lock() {
        if ring.first().map(String::as_str) == Some(cmd) {
            return;
        }
        ring.retain(|c| c != cmd);
        ring.insert(0, cmd.to_string());
        ring.truncate(HISTORY_CAP);
    }
}

/// The fish-style autosuggest suffix: the newest history entry that starts with
/// `prefix` (and is strictly longer), with `prefix` stripped. `None` when the
/// prefix is empty or nothing matches.
pub fn history_suggest(prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        return None;
    }
    let ring = HISTORY_RING.lock().ok()?;
    ring.iter()
        .find(|c| c.len() > prefix.len() && c.starts_with(prefix))
        .map(|c| c[prefix.len()..].to_string())
}

/// Process-global bundled command library (subcommand / option descriptions),
/// built once — `Library::bundled` parses the embedded packs.
fn completion_library() -> &'static pier_core::terminal::library::Library {
    static LIB: std::sync::OnceLock<pier_core::terminal::library::Library> =
        std::sync::OnceLock::new();
    LIB.get_or_init(pier_core::terminal::library::Library::bundled)
}

/// Lists a directory on the SSH host via `ls` so file completion in an SSH
/// terminal queries the *remote* filesystem. Paths are normalised to POSIX
/// because a Windows client's `PathBuf` join yields `\`.
struct RemoteDirReader {
    session: SshSession,
}

impl pier_core::terminal::completions::DirReader for RemoteDirReader {
    fn list(&self, dir: &std::path::Path) -> Vec<pier_core::terminal::completions::DirReadEntry> {
        let path = dir.to_string_lossy().replace('\\', "/");
        // -1 one per line, -A dotfiles (no . / ..), -p trailing slash on dirs.
        let cmd = format!("ls -1Ap -- '{}'", path.replace('\'', "'\\''"));
        match self.session.exec_command_blocking(&cmd) {
            Ok((0, out)) => out
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| pier_core::terminal::completions::DirReadEntry {
                    name: l.trim_end_matches('/').to_string(),
                    is_dir: l.ends_with('/'),
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// Tab-completion candidates for `line` at byte offset `cursor`, using the
/// shell's last-known `cwd` (OSC 7) for file completion and the bundled library
/// for subcommand / option descriptions. `locale` selects the description
/// language (e.g. "en" / "zh-CN").
///
/// With a `session` (SSH terminal) file / directory rows come from the remote
/// host via [`RemoteDirReader`]; command-position binaries still resolve against
/// the **local** PATH (the completer's command branch isn't reader-driven), so
/// those are dropped — the remaining builtin / subcommand / option / remote-file
/// rows are valid for the remote shell.
pub fn terminal_complete(
    line: &str,
    cursor: usize,
    cwd: Option<&str>,
    locale: &str,
    session: Option<SshSession>,
) -> Vec<pier_core::terminal::completions::Completion> {
    use pier_core::terminal::completions::CompletionKind;
    let cwd_path = cwd.map(std::path::Path::new);
    let lib = completion_library();
    match session {
        Some(s) => {
            let reader = RemoteDirReader { session: s };
            let mut rows = pier_core::terminal::completions::complete_with_library_using(
                line, cursor, cwd_path, lib, locale, &reader,
            );
            rows.retain(|r| !matches!(r.kind, CompletionKind::Binary));
            rows
        }
        None => pier_core::terminal::completions::complete_with_library(
            line, cursor, cwd_path, lib, locale,
        ),
    }
}

/// Man-page / `--help` summary for `cmd`, for the smart-mode help popover. With
/// a `session` (SSH) the text is fetched from the **remote** host and parsed by
/// pier-core; locally it uses pier-core's process-spawning lookup. Returns
/// `None` for an invalid name or when nothing usable was found.
pub fn terminal_man(
    session: Option<SshSession>,
    cmd: &str,
) -> Option<pier_core::terminal::man::ManSynopsis> {
    let name = cmd.trim();
    if name.is_empty()
        || name.contains([' ', '\t', '|', ';', '&', '<', '>', '`', '$', '\n', '\'', '"'])
    {
        return None;
    }
    let Some(s) = session else {
        return pier_core::terminal::man::man_synopsis(name).ok();
    };
    let meaningful = |m: &pier_core::terminal::man::ManSynopsis| {
        !(m.synopsis.is_empty() && m.description.is_empty() && m.options.is_empty())
    };
    // Remote man, overstriking stripped via `col -b`, output bounded.
    let man_cmd =
        format!("LANG=C LC_ALL=C man -P cat -- {name} 2>/dev/null | col -b | head -c 16000");
    if let Ok((0, out)) = s.exec_command_blocking(&man_cmd) {
        if !out.trim().is_empty() {
            let parsed = pier_core::terminal::man::parse_rendered(&out, "man");
            if meaningful(&parsed) {
                return Some(parsed);
            }
        }
    }
    // Fallback: remote `<cmd> --help`.
    let help_cmd = format!("{name} --help 2>&1 | head -c 16000");
    if let Ok((_, out)) = s.exec_command_blocking(&help_cmd) {
        if !out.trim().is_empty() {
            let parsed = pier_core::terminal::man::parse_rendered(&out, "help");
            if meaningful(&parsed) {
                return Some(parsed);
            }
        }
    }
    None
}

/// Whether `name` resolves to a runnable command (shell builtin or on PATH) on
/// the terminal's host — drives smart-mode typo highlighting. Remote check via
/// `command -v` over the session; local via pier-core's PATH scan.
pub fn terminal_command_exists(session: Option<SshSession>, name: &str) -> bool {
    match session {
        Some(s) => {
            let cmd = format!("command -v -- '{}'", name.replace('\'', "'\\''"));
            matches!(s.exec_command_blocking(&cmd), Ok((0, _)))
        }
        None => !matches!(
            pier_core::terminal::validate::validate_command(name),
            pier_core::terminal::validate::CommandKind::Missing
        ),
    }
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
            lang: String::from("en"),
            smart_mode: false,
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
            "lang" => s.lang = v.to_string(),
            "smart_mode" => s.smart_mode = v == "true",
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
        "active_tool={}\nright_collapsed={}\nshow_servers={}\ndark={}\nsidebar_w={}\nright_w={}\ncwd={}\nlang={}\nsmart_mode={}\n",
        s.active_tool,
        s.right_collapsed,
        s.show_servers,
        s.dark,
        opt(s.sidebar_w),
        opt(s.right_w),
        s.cwd,
        s.lang,
        s.smart_mode,
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
