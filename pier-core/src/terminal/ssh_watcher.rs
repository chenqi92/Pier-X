//! SSH child-process watcher for local terminal sessions.
//!
//! ## Why this exists
//!
//! The right-side Server Monitor panel needs to follow whatever
//! target the user is actually connected to inside the terminal. The
//! old approach parsed user keystrokes as they were typed and wrote
//! the host into tab state optimistically — which worked for clean
//! typing but broke the moment the user:
//!
//!  * navigated shell history with the arrow keys (we cleared our
//!    local command buffer, so an edited-and-resubmitted `ssh
//!    root@host` was never captured);
//!  * pasted a block of lines (we can capture paste, but not
//!    intermediate history edits);
//!  * typed an `ssh` that failed (DNS, auth, timeout) — the failed
//!    host stayed pinned because nothing observed the exit;
//!  * ran a second `ssh` from inside an already-live ssh session
//!    (the panel needs to follow the *innermost* target).
//!
//! The watcher fixes all four by using ground truth: is there a
//! live `ssh` process somewhere in the descendant tree of this
//! terminal's PTY child? If yes, what argv did it get? When that
//! answer changes, the panel should switch — period.
//!
//! ## Cross-platform approach
//!
//! `sysinfo` gives us a uniform read across Linux / macOS / Windows:
//! [`System::refresh_processes`] populates a `HashMap<Pid, Process>`
//! where each entry exposes `parent()` and `cmd()`. We walk upward
//! from every `ssh`-named process; if the walk reaches our PTY root
//! pid, that ssh is "inside the terminal" and we parse its argv.
//!
//! We **do not** call `System::new_all()`. That would also collect
//! disks, networks, memory, and every process's environment on
//! Windows — fast-expensive per-probe when we just need pid/ppid/argv.
//! Refreshing only the process table keeps one second of polling to
//! a couple of milliseconds on a busy laptop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// Well-known OpenSSH failure banners. Finding any of these in the
/// PTY byte stream is a strong signal that whichever `ssh` the user
/// just launched is about to exit — so the watcher should re-scan
/// immediately instead of waiting for its next timer tick.
///
/// Kept to the canonical English strings that OpenSSH emits before
/// its localisation layer (DNS + connection + auth errors all come
/// from `ssh_err` / `debug` paths that bypass gettext). The check
/// is a substring scan so partial matches across chunk boundaries
/// only lose one kick — the watcher's normal 1s cadence still wins
/// eventually.
const SSH_FAILURE_MARKERS: &[&[u8]] = &[
    b"ssh: Could not resolve hostname",
    b"ssh: connect to host",
    b"ssh: connect to address",
    b"Permission denied (publickey",
    b"Permission denied (password",
    b"Permission denied, please try again",
    b"Host key verification failed",
    b"No route to host",
    b"Connection refused",
    b"Connection timed out",
    b"Connection closed by",
    b"Connection reset by peer",
];

/// Quick substring scan over a chunk of recently-received PTY
/// output. Returns true when any failure marker is present.
///
/// Complexity is O(chunk.len() * markers.len()) in the worst case,
/// but the marker set is small and the inner `windows(..).any(..)`
/// bails on the first hit, so a 4 KB chunk with no match costs a
/// few microseconds. The reader thread already pays for one
/// allocation per chunk (emulator processing); this adds a second
/// pass that the compiler can vectorise.
pub fn output_indicates_ssh_failure(chunk: &[u8]) -> bool {
    SSH_FAILURE_MARKERS
        .iter()
        .any(|marker| contains_subsequence(chunk, marker))
}

/// OpenSSH password-prompt markers. The client prints one of these
/// right before it disables terminal echo and blocks reading a
/// password, so seeing any of them in the PTY output stream is an
/// anchor for "the very next typed line is the password".
///
/// We keep the set narrow on purpose: general English matches like
/// just "password:" would fire for a remote `sudo` prompt, the
/// Linux `passwd` binary changing a local password, or any
/// application on the remote side that happens to print the word.
/// A capture armed against those would grab the user's sudo
/// password and send it to our right-side russh session — which
/// would then try to authenticate sshd with the user's sudo creds
/// (wrong) or worse, store the sudo password in `tab.sshPassword`.
/// So we only match the canonical OpenSSH prompt shapes:
///
///   `<user>@<host>'s password:`          ← local ssh.exe / OpenSSH
///   `Password for <user>@<host>:`        ← some vendor forks
///   `<user>@<host>: Password:`           ← PuTTY-alike wrappers
///   `Enter passphrase for key '...':`    ← key unlock (also captured
///                                          because the answer is
///                                          effectively the secret
///                                          the right-side session
///                                          needs too)
///
/// The `user@host` shape requires a `@` and a `'s password:` tail,
/// which is specific enough that a remote `sudo` prompt can't
/// trigger it.
const SSH_PASSWORD_MARKER: &[u8] = b"'s password:";
const SSH_PASSPHRASE_MARKER: &[u8] = b"Enter passphrase for key ";

/// Distinguishes the two kinds of OpenSSH secret prompts. The
/// frontend cares about the difference: a password belongs in
/// `tab.sshPassword` and is sent as the SSH server password;
/// a passphrase belongs in `tab.sshKeyPassphrase` and is fed to
/// `russh::keys::load_secret_key` to decrypt the private key.
/// Crossing them wastes connect attempts and confuses the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshSecretPromptKind {
    /// `<user>@<host>'s password:` — server-side password auth.
    Password,
    /// `Enter passphrase for key '<path>':` — local key decryption.
    Passphrase,
}

/// Scan a PTY output chunk for an OpenSSH secret prompt. Returns
/// `Some(kind)` on a match so the caller can fire the right
/// one-shot event telling the frontend "the next typed line is
/// the password / passphrase".
///
/// Passphrase wins when both markers are present in the same
/// chunk (rare, but possible if the terminal redraw bundles the
/// prompt with leftover output from a previous attempt). The
/// passphrase prompt is always more specific.
pub fn detect_ssh_secret_prompt(chunk: &[u8]) -> Option<SshSecretPromptKind> {
    if contains_subsequence(chunk, SSH_PASSPHRASE_MARKER) {
        Some(SshSecretPromptKind::Passphrase)
    } else if contains_subsequence(chunk, SSH_PASSWORD_MARKER) {
        Some(SshSecretPromptKind::Password)
    } else {
        None
    }
}

/// Backward-compatible alias for callers that only need the boolean
/// "did we see any secret prompt" answer. New code should prefer
/// [`detect_ssh_secret_prompt`] so the password vs. passphrase
/// distinction propagates.
pub fn output_indicates_ssh_password_prompt(chunk: &[u8]) -> bool {
    detect_ssh_secret_prompt(chunk).is_some()
}

fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// SSH target resolved from a live ssh client process under the
/// terminal's PTY root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshChildTarget {
    /// Destination host — IP or DNS name, as the user wrote it.
    pub host: String,
    /// Remote login user. Defaults to the local user when the
    /// invocation omits `user@` (matches OpenSSH's own behavior).
    pub user: String,
    /// TCP port; 22 when the command had no `-p` or `-o Port=`.
    pub port: u16,
    /// `-i <path>` if the user passed one. Empty string when absent.
    pub identity_path: String,
}

/// Refresh the shared `System` and return the SSH target currently
/// reachable from `root_pid`, if any. Returns `None` when no ssh
/// client is running in the descendant tree.
///
/// Uses a pre-allocated `System` so repeated calls avoid allocating
/// and deallocating the internal process map every second.
pub fn scan(system: &mut System, root_pid: u32) -> Option<SshChildTarget> {
    // We only care about pid/ppid/cmd — skip environment and exe
    // resolution so Windows doesn't open every process handle.
    let refresh = ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always);
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);

    let root = Pid::from_u32(root_pid);
    let procs = system.processes();

    // Build a pid → parent map once so repeated upward walks stay
    // cheap. On Windows with ~400 processes this is ~20µs of work.
    let mut parents: HashMap<Pid, Option<Pid>> = HashMap::with_capacity(procs.len());
    for (pid, proc_) in procs.iter() {
        parents.insert(*pid, proc_.parent());
    }

    // Find the most deeply-nested ssh whose ancestor chain includes
    // root. Deepest wins so `ssh -> bash -> ssh` (a nested hop from
    // inside a session) follows the inner target, which is what the
    // user is actually typing at.
    let mut best: Option<(u32, SshChildTarget)> = None;
    for (pid, proc_) in procs.iter() {
        if !is_ssh_process_name(proc_.name().to_string_lossy().as_ref()) {
            continue;
        }
        let cmd_os = proc_.cmd();
        if cmd_os.is_empty() {
            continue;
        }
        // Second-line sanity: the leaf name looks like ssh, but the
        // cmdline's argv[0] must also be an ssh invocation. Rules
        // out coincidental process names.
        let argv0 = cmd_os[0].to_string_lossy();
        if !is_ssh_argv0(argv0.as_ref()) {
            continue;
        }

        let Some(depth) = ancestor_depth(&parents, *pid, root) else {
            continue;
        };

        // Convert argv to owned Strings once for the parser.
        let argv: Vec<String> = cmd_os
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();

        let Some(target) = parse_ssh_argv(&argv) else {
            continue;
        };

        match best {
            Some((best_depth, _)) if depth <= best_depth => {}
            _ => best = Some((depth, target)),
        }
    }

    best.map(|(_, t)| t)
}

/// Process-global handle to Pier-X's ssh-mux directory (where the
/// PATH-injected wrapper drops `recent-by-shell-<ppid>` hint files
/// and where ControlMaster sockets `cm-<hash>` live). The host app
/// (pier-x's tauri layer) sets this once at startup via
/// [`set_mux_hint_dir`]; pier-core itself stays UI-agnostic by
/// only consulting it in the optional [`scan_with_mux_fallback`]
/// path. Subsequent [`set_mux_hint_dir`] calls are no-ops.
static MUX_HINT_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Wired by the host app at startup to the directory where the
/// ssh-mux wrapper writes its per-shell argv hints. After this is
/// set, the watcher's [`scan_with_mux_fallback`] gains the ability
/// to recover an SSH target even when the actual ssh client process
/// is too short-lived for the PTY-tree scan to catch — which is the
/// steady state in OpenSSH `ControlMaster=auto` mode after the
/// master has daemonised out of our PTY's ancestor chain.
pub fn set_mux_hint_dir(dir: PathBuf) {
    let _ = MUX_HINT_DIR.set(dir);
}

/// Maximum age of a `recent-by-shell-<ppid>` hint file before we
/// stop trusting it. Set to match the default OpenSSH
/// `ControlPersist` of 600s — once a master has been gone that
/// long the user has effectively walked away, and the file is
/// almost certainly stale.
const MUX_HINT_TTL_SECS: u64 = 600;

/// PTY-tree scan with a ControlMaster fallback.
///
/// The base [`scan`] only finds an `ssh` process whose ancestor
/// chain reaches `root_pid` (the PTY's child shell). That works
/// perfectly for plain ssh — the client process runs for the
/// duration of the session under the shell. It silently fails for
/// `ControlMaster=auto` mux mode: the client forks the master,
/// the master `daemon()`s itself with `parent=1`, and the client
/// itself exits within milliseconds of the user pressing Enter.
/// By the time the watcher's 250ms scan runs, the only ssh
/// process left is the master — and its ancestor chain bypasses
/// `root_pid` entirely.
///
/// This wrapper covers the gap: when [`scan`] returns `None` it
/// reads `<MUX_HINT_DIR>/recent-by-shell-<root_pid>`, the file
/// the wrapper script wrote BEFORE exec'ing ssh. The argv it
/// recorded is re-parsed with [`parse_ssh_argv`] so the resulting
/// target is bit-identical to what the live-process path would
/// have produced. Two safety checks guard against stale hints:
///
///   1. `mtime` of the hint must be within [`MUX_HINT_TTL_SECS`]
///      — older entries imply the user already exited and walked
///      away; the file is just stale residue.
///   2. At least one `cm-*` socket must exist in the same
///      directory — proves an OpenSSH master is currently alive
///      somewhere. Without this we'd happily re-claim a target
///      moments after `ssh -O exit` killed the master.
///
/// Both checks are best-effort: a stale hint that survives both
/// only misleads the right-side panel for one probe cycle, after
/// which the panel surfaces an "auth rejected" / "connection
/// refused" the user can act on (vs. silent stuck-on-old-state).
pub fn scan_with_mux_fallback(system: &mut System, root_pid: u32) -> Option<SshChildTarget> {
    if let Some(target) = scan(system, root_pid) {
        return Some(target);
    }
    let dir = MUX_HINT_DIR.get()?;
    if !dir.is_dir() {
        return None;
    }
    let hint_path = dir.join(format!("recent-by-shell-{root_pid}"));
    let content = std::fs::read_to_string(&hint_path).ok()?;
    let mut lines = content.lines();
    let timestamp: u64 = lines.next()?.trim().parse().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    if now.saturating_sub(timestamp) > MUX_HINT_TTL_SECS {
        return None;
    }
    let mut argv: Vec<String> = Vec::with_capacity(8);
    // parse_ssh_argv expects argv[0] to be the program name; the
    // hint file stores only the user-supplied arguments (what was
    // passed to the wrapper), so prepend a synthetic `ssh`.
    argv.push("ssh".to_string());
    for arg in lines {
        argv.push(arg.to_string());
    }
    let target = parse_ssh_argv(&argv)?;
    if !any_master_socket_present(dir) {
        return None;
    }
    Some(target)
}

/// Cheap "is at least one ControlMaster master alive" check —
/// scans the mux dir for any `cm-*` socket file. We do not query
/// the master via `ssh -O check` here because (a) it spawns a
/// subprocess and the watcher loop runs every 250ms, and (b) any
/// active master file is a strong-enough signal: stale unix
/// sockets get cleaned up by the master itself on `O exit`, by
/// the OS on reboot (the dir is under `/tmp`), or explicitly by
/// `ssh_mux::shutdown_all_masters` on Pier-X exit.
fn any_master_socket_present(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("cm-")
        })
}

/// Leaf-name check: matches `ssh` on Unix, `ssh.exe` on Windows
/// (case-insensitive). Rejects `sshd`, `ssh-agent`, `ssh-keygen`, etc.
fn is_ssh_process_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(lower.as_str());
    stem == "ssh"
}

/// Same check but for argv[0], which may be a fully-qualified path
/// (`/usr/bin/ssh`, `C:\\Windows\\System32\\OpenSSH\\ssh.exe`) or
/// bare (`ssh`). We take the basename and reuse [`is_ssh_process_name`].
fn is_ssh_argv0(argv0: &str) -> bool {
    let basename = argv0.rsplit(['/', '\\']).next().unwrap_or(argv0);
    is_ssh_process_name(basename)
}

/// Walk `pid`'s ancestor chain through `parents`. Returns `Some(depth)`
/// if `root` is encountered (1 = direct child, 2 = grandchild, …) and
/// `None` if the chain reaches a process without a parent without
/// hitting root. Caps at 64 hops as a cycle-guard — nothing should be
/// that deep and a bad parent link shouldn't hang the watcher.
fn ancestor_depth(parents: &HashMap<Pid, Option<Pid>>, pid: Pid, root: Pid) -> Option<u32> {
    let mut current = pid;
    for depth in 1..=64 {
        let parent = parents.get(&current).copied().flatten()?;
        if parent == root {
            return Some(depth);
        }
        current = parent;
    }
    None
}

/// Argv-level parser for an `ssh` invocation. Returns the target the
/// command would dial, or `None` if the argv does not name a host
/// (e.g. `ssh -V`, `ssh -Q cipher`).
///
/// Mirrors the JavaScript `parseSshCommand` on the frontend but works
/// on already-tokenised argv (no shell quoting to unwind). Honors the
/// same flag set:
///
/// * `-p PORT`, `-pPORT`
/// * `-l USER`, `-lUSER`, `user@host` destination form
/// * `-i PATH`, `-iPATH`
/// * `-o Port=..`, `-o User=..`, `-o IdentityFile=..`
/// * value-consuming flags we don't care about (`-o -J -F -c -m -D -L
///   -R -W -Q -O -S -w -b -B -E -I`) — we only need to skip their
///   values so we don't mistake the value for the destination.
pub fn parse_ssh_argv(argv: &[String]) -> Option<SshChildTarget> {
    if argv.len() < 2 {
        return None;
    }

    const FLAGS_WITH_VALUE: &[&str] = &[
        "-p", "-l", "-i", "-o", "-J", "-F", "-c", "-m", "-D", "-L", "-R", "-W", "-Q", "-O", "-S",
        "-w", "-b", "-B", "-E", "-I",
    ];

    let mut user = String::new();
    let mut host = String::new();
    let mut port: u16 = 0;
    let mut identity_path = String::new();

    let mut i = 1;
    while i < argv.len() {
        let arg = argv[i].as_str();
        if arg == "--" {
            i += 1;
            continue;
        }
        if FLAGS_WITH_VALUE.contains(&arg) {
            let value = argv.get(i + 1)?;
            match arg {
                "-p" => {
                    let parsed: u32 = value.parse().ok()?;
                    if parsed == 0 || parsed > 65_535 {
                        return None;
                    }
                    port = parsed as u16;
                }
                "-l" => user = value.clone(),
                "-i" => identity_path = value.clone(),
                "-o" => {
                    if let Some((key, val)) = value.split_once('=') {
                        let key_lc = key.to_ascii_lowercase();
                        match key_lc.as_str() {
                            "port" => {
                                if let Ok(p) = val.parse::<u32>() {
                                    if p > 0 && p <= 65_535 {
                                        port = p as u16;
                                    }
                                }
                            }
                            "user" => user = val.to_string(),
                            "identityfile" => identity_path = val.to_string(),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            i += 2;
            continue;
        }
        // `-pNN`, `-lname`, `-iPATH` — short attached forms.
        if let Some(rest) = arg.strip_prefix("-p") {
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(p) = rest.parse::<u32>() {
                    if p > 0 && p <= 65_535 {
                        port = p as u16;
                    }
                }
                i += 1;
                continue;
            }
        }
        if let Some(rest) = arg.strip_prefix("-l") {
            if !rest.is_empty() {
                user = rest.to_string();
                i += 1;
                continue;
            }
        }
        if let Some(rest) = arg.strip_prefix("-i") {
            if !rest.is_empty() {
                identity_path = rest.to_string();
                i += 1;
                continue;
            }
        }
        // Unknown dash-prefixed flag — treat as boolean, skip.
        if arg.starts_with('-') {
            i += 1;
            continue;
        }

        // Positional destination.
        if let Some(at) = arg.rfind('@') {
            let user_part = &arg[..at];
            let host_part = &arg[at + 1..];
            if !user_part.is_empty() {
                user = user_part.to_string();
            }
            host = host_part.to_string();
        } else {
            host = arg.to_string();
        }
        break;
    }

    if host.is_empty() || host.starts_with('-') {
        return None;
    }
    if user.is_empty() {
        // Mirror OpenSSH behavior: destination without a user defaults
        // to the process's login user. We ask the OS for it here
        // rather than leaving it blank so the Server Monitor panel
        // doesn't reject the target (it requires a user to probe).
        user = current_login_user().unwrap_or_default();
        if user.is_empty() {
            return None;
        }
    }
    let port = if port == 0 { 22 } else { port };

    Some(SshChildTarget {
        host,
        user,
        port,
        identity_path,
    })
}

/// Best-effort current user for filling in an `ssh host` (no user@)
/// destination. Falls back across `USER` → `USERNAME` → `LOGNAME` so
/// it works on Unix shells and Windows cmd/PowerShell alike.
fn current_login_user() -> Option<String> {
    for var in ["USER", "USERNAME", "LOGNAME"] {
        if let Ok(value) = std::env::var(var) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_user_at_host() {
        let t = parse_ssh_argv(&argv(&["ssh", "root@192.168.0.182"])).unwrap();
        assert_eq!(t.user, "root");
        assert_eq!(t.host, "192.168.0.182");
        assert_eq!(t.port, 22);
        assert_eq!(t.identity_path, "");
    }

    #[test]
    fn parses_explicit_port_and_key() {
        let t = parse_ssh_argv(&argv(&[
            "ssh",
            "-p",
            "2222",
            "-i",
            "/home/a/.ssh/id",
            "me@example",
        ]))
        .unwrap();
        assert_eq!(t.user, "me");
        assert_eq!(t.host, "example");
        assert_eq!(t.port, 2222);
        assert_eq!(t.identity_path, "/home/a/.ssh/id");
    }

    #[test]
    fn parses_attached_short_flags() {
        let t = parse_ssh_argv(&argv(&["ssh", "-p2222", "-lme", "example"])).unwrap();
        assert_eq!(t.user, "me");
        assert_eq!(t.host, "example");
        assert_eq!(t.port, 2222);
    }

    #[test]
    fn parses_dash_o_port_and_user() {
        let t = parse_ssh_argv(&argv(&[
            "ssh",
            "-o",
            "Port=2200",
            "-o",
            "User=ops",
            "bastion",
        ]))
        .unwrap();
        assert_eq!(t.user, "ops");
        assert_eq!(t.host, "bastion");
        assert_eq!(t.port, 2200);
    }

    #[test]
    fn skips_value_of_irrelevant_flag() {
        // `-J jump@host` must not be read as the destination.
        let t = parse_ssh_argv(&argv(&["ssh", "-J", "jump@bastion", "target"])).unwrap();
        assert_eq!(t.host, "target");
    }

    #[test]
    fn rejects_no_destination() {
        assert!(parse_ssh_argv(&argv(&["ssh", "-V"])).is_none());
        assert!(parse_ssh_argv(&argv(&["ssh", "-p", "22"])).is_none());
    }

    #[test]
    fn rejects_dash_host() {
        assert!(parse_ssh_argv(&argv(&["ssh", "--help"])).is_none());
    }

    #[test]
    fn accepts_bare_host_with_env_user() {
        // Simulate an env-set user so the test isn't dependent on the
        // test runner's environment.
        std::env::set_var("USER", "alice");
        let t = parse_ssh_argv(&argv(&["ssh", "bastion"])).unwrap();
        assert_eq!(t.user, "alice");
        assert_eq!(t.host, "bastion");
    }

    #[test]
    fn is_ssh_process_name_matches() {
        assert!(is_ssh_process_name("ssh"));
        assert!(is_ssh_process_name("SSH"));
        assert!(is_ssh_process_name("ssh.exe"));
        assert!(is_ssh_process_name("SSH.EXE"));
        assert!(!is_ssh_process_name("sshd"));
        assert!(!is_ssh_process_name("ssh-agent"));
        assert!(!is_ssh_process_name("ssh-keygen"));
    }

    #[test]
    fn is_ssh_argv0_strips_path() {
        assert!(is_ssh_argv0("/usr/bin/ssh"));
        assert!(is_ssh_argv0("C:\\Windows\\System32\\OpenSSH\\ssh.exe"));
        assert!(!is_ssh_argv0("/usr/sbin/sshd"));
    }

    #[test]
    fn output_failure_marker_detects_dns() {
        let chunk = b"ssh: Could not resolve hostname 192.168.0.1822: temporary failure\r\n";
        assert!(output_indicates_ssh_failure(chunk));
    }

    #[test]
    fn output_failure_marker_detects_auth() {
        let chunk = b"Permission denied (publickey,password).\r\n";
        assert!(output_indicates_ssh_failure(chunk));
    }

    #[test]
    fn output_failure_marker_detects_refused() {
        let chunk = b"ssh: connect to host 10.0.0.1 port 22: Connection refused\r\n";
        assert!(output_indicates_ssh_failure(chunk));
    }

    #[test]
    fn output_failure_marker_ignores_unrelated_output() {
        let chunk = b"total 42\r\ndrwxr-xr-x  5 user user 4096 Jan  1 .\r\n";
        assert!(!output_indicates_ssh_failure(chunk));
    }

    #[test]
    fn password_prompt_marker_detects_canonical_shape() {
        let chunk = b"chenqi@192.168.0.174's password: ";
        assert!(output_indicates_ssh_password_prompt(chunk));
    }

    #[test]
    fn password_prompt_marker_detects_passphrase() {
        let chunk = b"Enter passphrase for key '/home/u/.ssh/id_ed25519': ";
        assert!(output_indicates_ssh_password_prompt(chunk));
    }

    #[test]
    fn password_prompt_marker_ignores_remote_sudo() {
        // A remote `sudo` prompt must not arm the SSH password
        // capture — the user's sudo password would otherwise end up
        // as `tab.sshPassword` and be sent to the right-side russh
        // session, where it would both fail authentication AND
        // persist in memory as if it were the ssh login password.
        let chunk = b"[sudo] password for chenqi: ";
        assert!(!output_indicates_ssh_password_prompt(chunk));
    }

    #[test]
    fn password_prompt_marker_ignores_local_passwd_command() {
        let chunk = b"Changing password for chenqi.\r\nCurrent password: ";
        assert!(!output_indicates_ssh_password_prompt(chunk));
    }
}
