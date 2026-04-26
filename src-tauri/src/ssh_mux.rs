//! Pier-X owned OpenSSH ControlMaster multiplexer.
//!
//! What this gives the user: when they type `ssh user@host` in a
//! Pier-X terminal tab, the first connection authenticates normally
//! (password / key / agent), and every subsequent ssh to the SAME
//! `user@host:port` within the persist window is a free ride —
//! OpenSSH multiplexes through a unix socket back to the still-alive
//! master process. No re-authentication, no second password prompt.
//!
//! Why this lives here and not in `~/.ssh/config`: an earlier Pier-X
//! version wrote `Include /tmp/pier-ssh-config` into the user's
//! global `~/.ssh/config`, which leaked the behaviour to every
//! ssh client on the machine (system Terminal, Warp, IDEs) and
//! survived uninstall. This module owns its own ssh_config and
//! injects it via a PATH wrapper, so:
//!
//!   * Pier-X-launched ssh → goes through wrapper → mux on
//!   * Anything else      → unchanged, no global pollution
//!
//! Layout:
//! ```text
//!   <cache>/ssh-mux/
//!     config        # auto-generated; ControlMaster auto, ControlPath, ControlPersist
//!     bin/ssh       # shell wrapper that exec's /usr/bin/ssh -F <config> "$@"
//!     settings.json # { enabled, persist_seconds }
//!   /tmp/com.pier-x.ssh/         # owner=user, mode=0700
//!     cm-<hash>     # ControlPath sockets (short path for sun_path limit)
//! ```
//!
//! The socket path is intentionally NOT under the cache dir on
//! macOS — `~/Library/Caches/com.kkape.pier-x/...` plus a 40-char
//! `%C` hash trips over the 104-byte sun_path ceiling for some
//! usernames. `/tmp/com.pier-x.ssh/` is bounded and short.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Settings persisted to `<cache>/ssh-mux/settings.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MuxSettings {
    /// Master switch. When false the wrapper still exists but the
    /// generated config has `ControlMaster no` so no mux happens.
    /// Useful for security-sensitive users who want every ssh
    /// invocation to re-authenticate.
    pub enabled: bool,
    /// `ControlPersist` value in seconds — how long an idle master
    /// stays alive after the last client disconnects. OpenSSH's own
    /// docs use 600s as the canonical example; we default to the
    /// same.
    pub persist_seconds: u32,
}

impl Default for MuxSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            persist_seconds: 600,
        }
    }
}

/// Process-global state. Initialised once at app startup via
/// [`init`]; subsequent reads are lock-free until a setting changes.
struct State {
    /// Where the auto-generated `ssh_config` lives (`<cache>/ssh-mux/config`).
    config_path: PathBuf,
    /// Where the `ssh` wrapper shim lives (`<cache>/ssh-mux/bin`).
    wrapper_dir: PathBuf,
    /// Where master sockets live (short path, bounded for sun_path).
    socket_dir: PathBuf,
    /// `<cache>/ssh-mux/settings.json`.
    settings_path: PathBuf,
    /// Currently effective settings.
    settings: Mutex<MuxSettings>,
}

static STATE: OnceLock<State> = OnceLock::new();

/// Initialise the mux directories and write the wrapper + config.
/// Idempotent — safe to call multiple times. On any I/O error,
/// returns it to the caller; the app should still come up (mux is
/// a convenience, not a load-bearing feature) but log loudly.
pub fn init(cache_dir: &Path) -> std::io::Result<()> {
    let mux_root = cache_dir.join("ssh-mux");
    let wrapper_dir = mux_root.join("bin");
    let config_path = mux_root.join("config");
    let settings_path = mux_root.join("settings.json");
    let socket_dir = PathBuf::from("/tmp/com.pier-x.ssh");

    fs::create_dir_all(&wrapper_dir)?;
    fs::create_dir_all(&socket_dir)?;
    set_dir_mode_0700(&socket_dir)?;

    let settings = read_settings(&settings_path).unwrap_or_default();

    write_config(&config_path, &socket_dir, &settings)?;
    write_wrapper(&wrapper_dir, &config_path, &socket_dir)?;

    let _ = STATE.set(State {
        config_path,
        wrapper_dir,
        socket_dir,
        settings_path,
        settings: Mutex::new(settings),
    });

    Ok(())
}

/// Wrapper directory to prepend to the PATH of any locally-spawned
/// PTY shell. `None` if [`init`] hasn't run (which would mean we
/// failed to create the cache dir on startup — the app falls back
/// to "no mux", same as before this module existed).
pub fn wrapper_dir() -> Option<&'static Path> {
    STATE.get().map(|s| s.wrapper_dir.as_path())
}

/// Directory holding ControlMaster sockets and the wrapper's
/// per-shell hint files (`recent-by-shell-<ppid>`). The pier-core
/// SSH watcher reads this dir to recover the active target when
/// the actual ssh client process is too short-lived to catch via
/// the PTY-tree scan — which is the steady state in
/// `ControlMaster=auto` mode after the master has daemonised.
pub fn socket_dir() -> Option<&'static Path> {
    STATE.get().map(|s| s.socket_dir.as_path())
}

/// The on-disk path of the auto-generated ssh config. Used by
/// `forget_target` and `shutdown_all_masters` so they hit the same
/// `ControlPath` template the wrapper uses.
#[allow(dead_code)]
pub fn config_path() -> Option<&'static Path> {
    STATE.get().map(|s| s.config_path.as_path())
}

/// Read current settings. Returns the default when uninitialised.
pub fn settings() -> MuxSettings {
    match STATE.get() {
        Some(s) => s.settings.lock().map(|g| g.clone()).unwrap_or_default(),
        None => MuxSettings::default(),
    }
}

/// Update settings, persist to disk, and rewrite the config so the
/// new ControlPersist takes effect on the NEXT ssh invocation
/// (already-running masters keep their old TTL until they exit).
pub fn set_settings(new_settings: MuxSettings) -> std::io::Result<()> {
    let state = match STATE.get() {
        Some(s) => s,
        None => {
            return Err(std::io::Error::other("ssh_mux not initialised"));
        }
    };

    write_config(&state.config_path, &state.socket_dir, &new_settings)?;
    write_settings(&state.settings_path, &new_settings)?;
    if let Ok(mut guard) = state.settings.lock() {
        *guard = new_settings;
    }
    Ok(())
}

/// Try to close the master for `(host, port, user)` if one exists.
/// Sends `ssh -O exit` through the same config the wrapper uses, so
/// the ControlPath template resolves to the right socket. Returns
/// Ok regardless of whether a socket was actually present — "no
/// master to forget" is a no-op, not an error.
pub fn forget_target(host: &str, port: u16, user: &str) -> std::io::Result<()> {
    let state = match STATE.get() {
        Some(s) => s,
        None => return Ok(()),
    };
    let target = format!("{user}@{host}");
    let port_str = port.to_string();
    let status = Command::new("ssh")
        .arg("-F")
        .arg(&state.config_path)
        .arg("-p")
        .arg(&port_str)
        .arg("-O")
        .arg("exit")
        .arg(&target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    // `ssh -O exit` returns non-zero when there was no master to
    // close — that's not an error from the user's POV (they wanted
    // it gone, it's gone). Only propagate spawn failures.
    match status {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Walk the socket directory and `ssh -O exit` every master found.
/// Called from the Tauri RunEvent::Exit handler so the user
/// quitting Pier-X actually shuts down the mux daemons (matches the
/// "GUI app boundary" the user expects, instead of leaving rogue
/// processes after uninstall).
///
/// Best-effort: each socket is processed independently and the
/// function returns the count of successful exits. A socket whose
/// filename doesn't decode back to `user@host:port` is removed
/// outright (stale leftover from a previous run).
pub fn shutdown_all_masters() -> usize {
    let state = match STATE.get() {
        Some(s) => s,
        None => return 0,
    };
    let entries = match fs::read_dir(&state.socket_dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut closed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        // We don't decode our own %C-hashed filenames back to a
        // target — they're SHA-1 of (thishost, host, port, user)
        // by design. Instead, ask ssh to close via the path
        // directly: `-S <path> -O exit any@any` works because the
        // master socket carries the user/host info itself.
        let path_str = match path.to_str() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let status = Command::new("ssh")
            .arg("-S")
            .arg(&path_str)
            .arg("-O")
            .arg("exit")
            // The host/user args are required by ssh's parser but
            // ignored when -S points at a live socket.
            .arg("placeholder")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if status.map(|s| s.success()).unwrap_or(false) {
            closed += 1;
        }
        // Either way, scrub the socket file so a relaunch starts
        // clean. ssh leaves the file behind on `exit`.
        let _ = fs::remove_file(&path);
    }
    closed
}

// ───────────────────────── helpers ─────────────────────────

fn write_config(
    config_path: &Path,
    socket_dir: &Path,
    settings: &MuxSettings,
) -> std::io::Result<()> {
    // OpenSSH's `%C` token is a SHA-1 hex of (l,h,p,r) — bounded
    // length, no shell-meta chars, safe in any path. The leading
    // `cm-` prefix lets us scrub the directory by glob without
    // accidentally killing unrelated dotfiles a user might drop
    // into /tmp/com.pier-x.ssh.
    let socket_template = socket_dir.join("cm-%C");
    let cm_directive = if settings.enabled {
        "auto"
    } else {
        "no"
    };

    let mut body = String::new();
    body.push_str("# Pier-X — auto-generated SSH config for terminal-side ControlMaster.\n");
    body.push_str("# This file is owned by Pier-X. Do not edit manually; it is\n");
    body.push_str("# overwritten on every app launch and on every settings change.\n");
    body.push_str("Host *\n");
    body.push_str(&format!("    ControlMaster {cm_directive}\n"));
    body.push_str(&format!(
        "    ControlPath {}\n",
        socket_template.display()
    ));
    body.push_str(&format!(
        "    ControlPersist {}\n",
        settings.persist_seconds
    ));

    write_atomic(config_path, body.as_bytes())
}

fn write_wrapper(
    wrapper_dir: &Path,
    config_path: &Path,
    socket_dir: &Path,
) -> std::io::Result<()> {
    let wrapper_path = wrapper_dir.join("ssh");
    // POSIX shell, not bash — the system /bin/sh exists everywhere
    // we ship. `exec` replaces the wrapper process so $? and
    // signals (Ctrl-C in interactive ssh) propagate normally.
    //
    // Using an absolute /usr/bin/ssh prevents recursion when the
    // wrapper happens to find itself first on the inherited PATH.
    //
    // The wrapper additionally records the invocation's argv to
    // `<socket_dir>/recent-by-shell-$PPID` BEFORE exec'ing ssh.
    // The pier-core watcher reads this hint to recover the active
    // target whenever the PTY-tree scan returns nothing — which is
    // exactly what happens in OpenSSH `ControlMaster=auto` mode:
    // the ssh client forks the master, the master daemonises out
    // of the PTY ancestor chain (parent becomes init), and the
    // client itself exits within milliseconds. By the time the
    // watcher's 250ms scan runs, there is nothing left under the
    // shell PID to find. The hint file (keyed by `$PPID` = the
    // shell that invoked us = the watcher's `root_pid`) is the
    // bridge.
    //
    // The `printf '%s\n' "$@"` form puts each argv element on its
    // own line so the watcher can faithfully re-tokenise without
    // having to undo shell quoting; multi-arg / quoted commands
    // (`ssh user@host 'long cmd'`) round-trip correctly.
    let body = format!(
        "#!/bin/sh\n\
         # Pier-X SSH wrapper — auto-generated; regenerated on every\n\
         # app launch. Do not edit.\n\
         #\n\
         # Step 1: drop a hint for the pier-core SSH watcher so it can\n\
         # follow ControlMaster mux sessions even after the ssh client\n\
         # daemonises out of our PTY ancestor chain.\n\
         hint_dir={socket_dir}\n\
         {{\n\
         \tdate +%s\n\
         \tprintf '%s\\n' \"$@\"\n\
         }} > \"$hint_dir/recent-by-shell-$PPID\" 2>/dev/null || true\n\
         #\n\
         # Step 2: exec the real ssh with our auto-generated config.\n\
         exec /usr/bin/ssh -F {config} \"$@\"\n",
        socket_dir = shell_escape(socket_dir.to_string_lossy().as_ref()),
        config = shell_escape(config_path.to_string_lossy().as_ref()),
    );
    write_atomic(&wrapper_path, body.as_bytes())?;
    set_executable(&wrapper_path)?;
    Ok(())
}

fn write_settings(path: &Path, settings: &MuxSettings) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|e| std::io::Error::other(format!("settings serialize: {e}")))?;
    write_atomic(path, &json)
}

fn read_settings(path: &Path) -> Option<MuxSettings> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_atomic(target: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = target.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, target)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_dir_mode_0700(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o700);
    fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn set_dir_mode_0700(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn shell_escape(s: &str) -> String {
    // Single-quote, escape any embedded single-quote. Sufficient for
    // a path written by us (no newlines, no NULs); we still bother
    // because some users have `Application Support` in their cache
    // path on macOS.
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

/// Tiny helper for callers that want to inject the wrapper into a
/// child PTY's PATH. Returns `<wrapper_dir>:<existing PATH>` when
/// initialised, else just the existing PATH unchanged.
pub fn prepended_path(existing: &str) -> String {
    match wrapper_dir() {
        Some(w) => format!("{}:{}", w.display(), existing),
        None => existing.to_string(),
    }
}

/// Currently configured persist window, exposed for diagnostics.
#[allow(dead_code)]
pub fn persist_duration() -> Duration {
    Duration::from_secs(settings().persist_seconds.into())
}
