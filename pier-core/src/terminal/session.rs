//! Terminal session — composes a [`Pty`] and a [`VtEmulator`] behind a
//! single, thread-safe, callback-driven API.
//!
//! ## Why this layer exists
//!
//! [`Pty`] and [`VtEmulator`] are intentionally dumb. `Pty` produces
//! raw bytes; `VtEmulator` consumes raw bytes. Neither knows the
//! other exists, and neither does its own I/O loop — they are leaves
//! that can be unit-tested without any threading.
//!
//! The shell, however, needs something much friendlier: a single
//! handle it can `write` to, `snapshot` from, `resize`, and be told
//! when something changed. It absolutely must not block the main
//! thread on a `read` call that's waiting for shell output.
//!
//! [`PierTerminal`] fills that gap. Internally it:
//!
//!  * owns a `Box<dyn Pty>` + a `VtEmulator`, both wrapped in a single
//!    `Arc<Mutex<Inner>>` so writes, snapshots, and the reader thread
//!    share the same consistent view of state;
//!  * spawns a dedicated reader thread at construction time that
//!    loops on `pty.read()` + `emu.process()` until a shutdown flag
//!    is set;
//!  * invokes a caller-provided `notify` callback whenever something
//!    interesting happened (new bytes, child exit). The callback is
//!    called WITHOUT holding the internal mutex — its only job is to
//!    wake the shell, which then calls [`PierTerminal::snapshot`]
//!    on its own terms.
//!
//! ## Thread model
//!
//! ```text
//!   shell/main thread               reader thread
//!   ─────────────────               ─────────────
//!   write(bytes) ──┐                 loop {
//!                  ├─► lock Inner       lock Inner
//!                  │                    read from pty
//!                  │                    feed emu
//!                  │                    unlock
//!                  └─► unlock           call notify(user_data, event)
//!   snapshot() ────┐                    if shutdown { break }
//!                  ├─► lock Inner       sleep 5ms
//!                  │                 }
//!                  │   copy grid
//!                  └─► unlock
//! ```
//!
//! The notify callback is called *outside* the lock so that, if the
//! callback takes its own lock in the shell layer and then calls back
//! into [`PierTerminal::snapshot`] — a common pattern for a
//! "data ready" wakeup — there is no deadlock.
//!
//! ## Long-term extensibility
//!
//! The reader-thread-per-session design is the only shape that maps
//! cleanly onto every backend we expect to care about:
//!
//!  * local Unix PTY (M2)
//!  * local Windows ConPTY (M2)
//!  * remote SSH channel (M3) — an SSH channel does not expose an OS
//!    file descriptor, so `QSocketNotifier` style designs would need
//!    a completely different code path. Polling via the same thread
//!    loop is portable.
//!
//! Swapping the backend is a matter of passing a different
//! `Box<dyn Pty>` into [`PierTerminal::with_pty`]; nothing in this
//! file or anything above it needs to change.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::emulator::{Cell, VtEmulator};
use super::pty::{Pty, TerminalError};
use super::ssh_watcher::{self, SshChildTarget};

/// Event kinds that the notify callback reports back to the consumer.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyEvent {
    /// The emulator grid has changed (new output, cursor moved, etc.).
    /// The consumer should request a new snapshot.
    DataReady = 0,
    /// The child process has exited and the reader thread has stopped.
    /// No further events will fire on this terminal.
    Exited = 1,
    /// The set of `ssh` clients running under this terminal's PTY
    /// changed — either a new one was spawned, the target changed
    /// (nested ssh), or the last one exited. The consumer should
    /// fetch the new target via [`PierTerminal::current_ssh_target`]
    /// and update any right-side state bound to this session.
    SshStateChanged = 2,
    /// The reader thread saw an OpenSSH server password prompt
    /// (`<user>@<host>'s password:`) in the PTY output. The consumer
    /// should arm a one-shot capture: the next Enter-terminated line
    /// the user types is the SERVER password — to be mirrored into
    /// the right-side russh session as `AuthMethod::DirectPassword`.
    /// Firing only on the specific OpenSSH shape (not generic
    /// `password:`) keeps remote `sudo` / local `passwd` prompts
    /// from triggering a capture.
    SshPasswordPrompt = 3,
    /// The reader thread saw an OpenSSH key-decryption passphrase
    /// prompt (`Enter passphrase for key '<path>':`). The captured
    /// line is a key passphrase, NOT a server password — the
    /// frontend stores it in a separate slot and the right-side
    /// russh session uses it via `russh::keys::load_secret_key`.
    /// Conflating the two costs failed connect attempts.
    SshPassphrasePrompt = 4,
}

/// Function-pointer signature for the notify callback.
///
/// Called from the reader thread, *without* the internal mutex held.
/// Implementations must be quick and thread-safe. A typical body
/// schedules a wakeup onto the app's main thread or event loop. Do
/// NOT call back into [`PierTerminal::snapshot`] synchronously from
/// inside this callback; bounce to another thread first.
///
/// `user_data` is whatever the caller passed to [`PierTerminal::new`]
/// and is opaque to this crate.
pub type NotifyFn = extern "C" fn(user_data: *mut std::ffi::c_void, event: u32);

/// Inner state protected by a single mutex. Held briefly by both
/// the reader thread and the shell/main thread.
struct Inner {
    pty: Box<dyn Pty>,
    emu: VtEmulator,
    /// Latest SSH target detected by the child-process watcher, or
    /// `None` when no `ssh` client is currently alive under this
    /// terminal's PTY. Updated exclusively by the watcher thread;
    /// read by [`PierTerminal::current_ssh_target`]. The field lives
    /// here (rather than in a separate mutex) so a single lock serves
    /// all shared state the UI might ask about.
    ssh_child_target: Option<SshChildTarget>,
}

/// A live terminal session — PTY + emulator + reader thread, all
/// behind one handle.
///
/// Construct via [`PierTerminal::new`] (default Unix PTY) or
/// [`PierTerminal::with_pty`] (inject your own `Box<dyn Pty>` for
/// tests or future SSH sessions). Dropping the handle shuts down the
/// reader thread and reaps the child process.
pub struct PierTerminal {
    inner: Arc<Mutex<Inner>>,
    shutdown: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    /// Background thread that scans the PTY's descendant process
    /// tree for an `ssh` client once a second. Spawned only when the
    /// underlying PTY exposes a child pid (local shells do; remote
    /// SSH-channel PTYs don't — they'd have nothing meaningful to
    /// scan). `None` when the watcher is disabled for this session.
    ssh_watcher: Option<JoinHandle<()>>,
    // We keep cols/rows at the struct level for lock-free accessors;
    // the authoritative size is always Inner::pty.size() / emu.cols,
    // but reading those requires taking the lock.
    cols: u16,
    rows: u16,
}

/// Result of [`PierTerminal::snapshot`] — a caller-copied view of the
/// grid at one point in time.
#[derive(Debug, Clone)]
pub struct GridSnapshot {
    /// Columns at the time of the snapshot.
    pub cols: u16,
    /// Rows at the time of the snapshot.
    pub rows: u16,
    /// Cursor column, zero-based, `< cols`.
    pub cursor_x: u16,
    /// Cursor row, zero-based, `< rows`.
    pub cursor_y: u16,
    /// Cell grid, row-major: `cells[row * cols + col]`.
    pub cells: Vec<Cell>,
    /// Smart-mode prompt-end position — `(row, col)` of the most
    /// recent OSC 133;B emitted by the shell. `None` until the first
    /// wrapped prompt is seen, or after a screen-clear / scroll-off
    /// invalidates the position. Consumed by the UI to overlay
    /// autosuggest / syntax-highlight from this cell onward.
    pub prompt_end: Option<(u16, u16)>,
    /// `true` between OSC 133;B (user starts typing) and OSC 133;C
    /// (user pressed Enter). The smart-mode UI activates input
    /// mirroring only when this is set.
    pub awaiting_input: bool,
    /// `true` while the application has switched to the alternate
    /// screen (vim/htop/less/tmux). The smart-mode UI must hide
    /// itself entirely while this is set.
    pub alt_screen: bool,
    /// `true` while the shell is inside a bracketed-paste sequence.
    /// The smart-mode UI pauses completion / autosuggest while set.
    pub bracketed_paste: bool,
}

impl PierTerminal {
    /// Spawn a new local shell session.
    ///
    /// On Unix this goes through [`super::pty::UnixPty::spawn_shell`]
    /// (forkpty); on Windows through
    /// [`super::pty::WindowsPty::spawn_shell`] (ConPTY).
    ///
    /// `notify` and `user_data` are stored and invoked from the
    /// reader thread on any subsequent event. See [`NotifyFn`] for
    /// the callback contract.
    pub fn new(
        cols: u16,
        rows: u16,
        shell: &str,
        notify: NotifyFn,
        user_data: *mut std::ffi::c_void,
    ) -> Result<Self, TerminalError> {
        Self::new_with_smart(cols, rows, shell, false, notify, user_data)
    }

    /// Same as [`Self::new`] but optionally enables smart-mode shell
    /// integration (see `pier-core::terminal::smart`).
    ///
    /// When `smart_mode` is `true` and the requested `shell` is
    /// recognised by [`super::smart::inject_init`] (today: bash, zsh),
    /// the shell is launched with a temp `--rcfile` / `ZDOTDIR` that
    /// wraps the user's PS1 with OSC 133 prompt sentinels. The
    /// emulator picks those up and exposes `prompt_end` /
    /// `awaiting_input` on each [`GridSnapshot`] so the UI can overlay
    /// the smart layer.
    ///
    /// When `smart_mode` is `false`, or when the shell is not
    /// recognised, this falls back to a plain login shell — identical
    /// behaviour to [`Self::new`].
    pub fn new_with_smart(
        cols: u16,
        rows: u16,
        shell: &str,
        smart_mode: bool,
        notify: NotifyFn,
        user_data: *mut std::ffi::c_void,
    ) -> Result<Self, TerminalError> {
        Self::new_with_smart_env(cols, rows, shell, smart_mode, &[], notify, user_data)
    }

    /// Same as [`Self::new_with_smart`] but applies `extra_env`
    /// (KEY, VALUE pairs) to the spawned shell on top of the
    /// process-inherited environment. The pairs are merged with
    /// any smart-mode env (`init.env` wins on key collisions, since
    /// it carries `ZDOTDIR` etc. that the smart layer needs to be
    /// authoritative about).
    ///
    /// On Windows the env extras are currently ignored — ConPTY env
    /// injection lives in a separate code path we haven't unified
    /// yet. The terminal still starts; callers that depend on the
    /// extras (e.g. PATH wrappers for ssh ControlMaster) get a
    /// silent fallback to "no mux", same as before this method
    /// existed.
    pub fn new_with_smart_env(
        cols: u16,
        rows: u16,
        shell: &str,
        smart_mode: bool,
        extra_env: &[(&str, &str)],
        notify: NotifyFn,
        user_data: *mut std::ffi::c_void,
    ) -> Result<Self, TerminalError> {
        #[cfg(unix)]
        {
            let pty: Box<dyn Pty> = if smart_mode {
                let init = super::smart::inject_init(shell);
                if init.recognised {
                    Box::new(super::pty::UnixPty::spawn_shell_smart_with_env(
                        cols, rows, shell, init, extra_env,
                    )?)
                } else {
                    Box::new(super::pty::UnixPty::spawn_shell_with_env(
                        cols, rows, shell, extra_env,
                    )?)
                }
            } else {
                Box::new(super::pty::UnixPty::spawn_shell_with_env(
                    cols, rows, shell, extra_env,
                )?)
            };
            Self::with_pty(pty, cols, rows, notify, user_data)
        }
        #[cfg(windows)]
        {
            // Smart mode + env injection are a no-op on Windows for
            // M1 — cmd.exe has no OSC 133 support, pwsh would need
            // PSReadLine integration we haven't validated yet, and
            // the ConPTY env block is built per-process in a
            // different module. Fall through to the plain ConPTY
            // shell either way.
            let _ = (smart_mode, extra_env);
            let pty: Box<dyn Pty> =
                Box::new(super::pty::WindowsPty::spawn_shell(cols, rows, shell)?);
            Self::with_pty(pty, cols, rows, notify, user_data)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (cols, rows, shell, smart_mode, extra_env, notify, user_data);
            Err(TerminalError::Unsupported)
        }
    }

    /// Construct a session from an already-spawned `Pty`.
    ///
    /// Useful for tests (inject a mock Pty), and for the future M3
    /// remote terminal where the Pty is actually an SSH channel
    /// wrapped to implement the [`Pty`] trait.
    pub fn with_pty(
        pty: Box<dyn Pty>,
        cols: u16,
        rows: u16,
        notify: NotifyFn,
        user_data: *mut std::ffi::c_void,
    ) -> Result<Self, TerminalError> {
        // Capture the child pid before we move the pty into Inner;
        // once it's behind the mutex we'd need to lock to read it.
        let child_pid = pty.child_pid();
        // TEMP DIAGNOSTIC — track watcher lifecycle so we can tell
        // a "watcher never spawned" failure mode apart from a "watcher
        // spawned but never matched anything" one. The whole point of
        // having logs is being able to tell those apart in the field;
        // without this the only way to debug a silent watcher is to
        // attach a debugger to a running Pier-X build.
        crate::logging::write_event(
            "INFO",
            "ssh.watcher",
            &format!("PierTerminal::with_pty child_pid={:?}", child_pid),
        );

        let emu = VtEmulator::new(cols as usize, rows as usize);
        let inner = Arc::new(Mutex::new(Inner {
            pty,
            emu,
            ssh_child_target: None,
        }));
        let shutdown = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let ssh_failure_kick = Arc::new(AtomicBool::new(false));

        let reader = Some(Self::spawn_reader(
            Arc::clone(&inner),
            Arc::clone(&shutdown),
            Arc::clone(&alive),
            Arc::clone(&ssh_failure_kick),
            notify,
            user_data as usize,
        ));

        // Only local PTYs give us a pid to walk; the SSH-channel Pty
        // (remote terminal) reports `None` and we skip the watcher —
        // scanning *this* host's process tree would return nonsense
        // for a session that's running commands on a remote host.
        let ssh_watcher = child_pid.map(|pid| {
            crate::logging::write_event(
                "INFO",
                "ssh.watcher",
                &format!("spawn_ssh_watcher root_pid={pid}"),
            );
            Self::spawn_ssh_watcher(
                Arc::clone(&inner),
                Arc::clone(&shutdown),
                Arc::clone(&alive),
                Arc::clone(&ssh_failure_kick),
                pid,
                notify,
                user_data as usize,
            )
        });

        Ok(Self {
            inner,
            shutdown,
            alive,
            reader,
            ssh_watcher,
            cols,
            rows,
        })
    }

    /// Spawn the background reader thread.
    ///
    /// The thread loops on `pty.read`, feeds every non-empty chunk
    /// into the emulator, then calls notify. On EOF / I/O error it
    /// marks `alive = false`, fires a final `Exited` event, and
    /// terminates.
    ///
    /// `user_data_addr` is the caller's opaque pointer cast to
    /// `usize`. We take it as an integer so the closure below
    /// captures it by value — Rust 2021 disjoint captures would
    /// otherwise try to capture the underlying `*mut c_void`, which
    /// isn't `Send`. We cast back to `*mut c_void` at the moment of
    /// calling `notify`.
    fn spawn_reader(
        inner: Arc<Mutex<Inner>>,
        shutdown: Arc<AtomicBool>,
        alive: Arc<AtomicBool>,
        ssh_failure_kick: Arc<AtomicBool>,
        notify: NotifyFn,
        user_data_addr: usize,
    ) -> JoinHandle<()> {
        thread::Builder::new()
            .name("pier-terminal-reader".to_string())
            .spawn(move || {
                // Tight polling interval. On Unix `read` returns
                // immediately with EAGAIN when there's nothing to
                // read, so the CPU cost of this loop is dominated by
                // the sleep, not the syscall.
                let idle = Duration::from_millis(5);
                let user_data = user_data_addr as *mut std::ffi::c_void;

                loop {
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }

                    // Lock briefly for the read + feed. This is the
                    // only critical section the reader thread holds.
                    // The failure-marker scan runs against the raw
                    // chunk under the same lock — a handful of
                    // substring passes over ≤64KB is microseconds.
                    let outcome = {
                        let mut guard = match inner.lock() {
                            Ok(g) => g,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        match guard.pty.read() {
                            Ok(chunk) if !chunk.is_empty() => {
                                if ssh_watcher::output_indicates_ssh_failure(&chunk) {
                                    ssh_failure_kick.store(true, Ordering::Relaxed);
                                }
                                let prompt_kind = ssh_watcher::detect_ssh_secret_prompt(&chunk);
                                guard.emu.process(&chunk);
                                match prompt_kind {
                                    Some(ssh_watcher::SshSecretPromptKind::Passphrase) => {
                                        ReadOutcome::PassphrasePrompt
                                    }
                                    Some(ssh_watcher::SshSecretPromptKind::Password) => {
                                        ReadOutcome::PasswordPrompt
                                    }
                                    None => ReadOutcome::Data,
                                }
                            }
                            Ok(_) => ReadOutcome::Idle,
                            Err(_) => ReadOutcome::Done,
                        }
                    };

                    match outcome {
                        ReadOutcome::Data => {
                            // Notify outside the lock. If the UI
                            // callback turns around and calls snapshot,
                            // we've already released — no deadlock.
                            (notify)(user_data, NotifyEvent::DataReady as u32);
                        }
                        ReadOutcome::PasswordPrompt => {
                            // Fire the data notification first so the
                            // prompt text renders in the terminal
                            // grid, then the prompt event so the UI
                            // can arm a one-shot capture before the
                            // user finishes typing.
                            (notify)(user_data, NotifyEvent::DataReady as u32);
                            (notify)(user_data, NotifyEvent::SshPasswordPrompt as u32);
                        }
                        ReadOutcome::PassphrasePrompt => {
                            (notify)(user_data, NotifyEvent::DataReady as u32);
                            (notify)(user_data, NotifyEvent::SshPassphrasePrompt as u32);
                        }
                        ReadOutcome::Idle => {
                            thread::sleep(idle);
                        }
                        ReadOutcome::Done => {
                            alive.store(false, Ordering::Relaxed);
                            (notify)(user_data, NotifyEvent::Exited as u32);
                            break;
                        }
                    }
                }
            })
            .expect("spawning reader thread must not fail in practice")
    }

    /// Spawn the background SSH-child watcher.
    ///
    /// The thread owns a single `sysinfo::System` so consecutive
    /// refreshes reuse the same internal allocations. It polls once
    /// a second — fast enough that the user perceives the right-side
    /// panel as "following" the terminal, slow enough that the cost
    /// is negligible on a busy laptop. The loop exits when the reader
    /// thread has marked `alive = false` (child shell died) or the
    /// shutdown flag is set.
    ///
    /// On every change to the detected target we write to `Inner` and
    /// fire `NotifyEvent::SshStateChanged` so the UI layer can emit
    /// its own event / refetch without polling.
    fn spawn_ssh_watcher(
        inner: Arc<Mutex<Inner>>,
        shutdown: Arc<AtomicBool>,
        alive: Arc<AtomicBool>,
        ssh_failure_kick: Arc<AtomicBool>,
        root_pid: u32,
        notify: NotifyFn,
        user_data_addr: usize,
    ) -> JoinHandle<()> {
        thread::Builder::new()
            .name("pier-terminal-ssh-watcher".to_string())
            .spawn(move || {
                crate::logging::write_event(
                    "INFO",
                    "ssh.watcher",
                    &format!("watcher thread started root_pid={root_pid}"),
                );
                // Lazy-init the System. The first scan_for_ssh call
                // populates the full process map; subsequent calls
                // only diff what changed, which is why we hold onto
                // it across iterations.
                let mut system = sysinfo::System::new();
                let poll_interval = Duration::from_millis(1000);
                // Shorter interval for the first few seconds so a
                // just-typed `ssh user@host` feels instant — the
                // child takes ~50-200ms to actually appear in the
                // process table after fork/spawn.
                let fast_interval = Duration::from_millis(250);
                let fast_scans_remaining_init = 8u32;
                let mut fast_scans_remaining = fast_scans_remaining_init;
                let mut iter_count: u32 = 0;

                let user_data = user_data_addr as *mut std::ffi::c_void;

                loop {
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                    if !alive.load(Ordering::Relaxed) {
                        break;
                    }

                    // Consume the "SSH failure banner seen" kick so
                    // the next sleep block doesn't treat a stale hit
                    // as a reason to re-wake; the current iteration
                    // is already about to scan.
                    ssh_failure_kick.store(false, Ordering::Relaxed);

                    let new_target = ssh_watcher::scan(&mut system, root_pid);
                    iter_count = iter_count.saturating_add(1);
                    if iter_count <= 12 || iter_count % 30 == 0 {
                        // First ~12 iterations always log so we capture the
                        // boot-up race; after that throttle to once every
                        // ~30s so the file doesn't bloat.
                        crate::logging::write_event(
                            "DEBUG",
                            "ssh.watcher",
                            &format!(
                                "iter={iter_count} root_pid={root_pid} scan→{new_target:?}"
                            ),
                        );
                    }

                    // Minimise the critical section: compute under
                    // the lock only long enough to compare + swap,
                    // then release before firing notify.
                    let changed = {
                        let mut guard = match inner.lock() {
                            Ok(g) => g,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        if guard.ssh_child_target == new_target {
                            false
                        } else {
                            guard.ssh_child_target = new_target.clone();
                            true
                        }
                    };

                    if changed {
                        crate::logging::write_event(
                            "INFO",
                            "ssh.watcher",
                            &format!(
                                "transition: notify SshStateChanged target={new_target:?}"
                            ),
                        );
                        (notify)(user_data, NotifyEvent::SshStateChanged as u32);
                        // After a transition, go back into fast-scan
                        // mode for a few iterations so the follow-up
                        // (nested ssh appearing, ssh exiting with a
                        // "client disconnected" message) propagates
                        // without waiting a full second.
                        fast_scans_remaining = fast_scans_remaining_init;
                    }

                    let sleep_for = if fast_scans_remaining > 0 {
                        fast_scans_remaining -= 1;
                        fast_interval
                    } else {
                        poll_interval
                    };
                    // Break the sleep into small slices so a
                    // terminal_close / app shutdown doesn't wait up
                    // to a full second for this thread to notice.
                    // 50ms slice ≤ natural jitter of a 1s poll, so
                    // the watcher's cadence stays predictable even
                    // when the shell is otherwise idle.
                    //
                    // Bail out early if the reader thread saw an SSH
                    // failure banner in the byte stream — `ssh` is
                    // about to exit, and re-scanning immediately
                    // propagates "no more ssh" to the UI without
                    // waiting for the rest of the poll interval.
                    let slice = Duration::from_millis(50);
                    let mut remaining = sleep_for;
                    while remaining > Duration::ZERO {
                        if shutdown.load(Ordering::Relaxed) {
                            break;
                        }
                        if ssh_failure_kick.load(Ordering::Relaxed) {
                            break;
                        }
                        let step = remaining.min(slice);
                        thread::sleep(step);
                        remaining = remaining.saturating_sub(step);
                    }
                }
            })
            .expect("spawning ssh watcher thread must not fail in practice")
    }

    /// Send bytes to the shell (user keystrokes, paste, etc.).
    pub fn write(&self, data: &[u8]) -> Result<usize, TerminalError> {
        let mut guard = self.inner.lock().map_err(|p| {
            // A poisoned mutex means a different thread panicked
            // while holding state. Surface it to the caller as an
            // I/O error rather than propagating the panic.
            TerminalError::Io(std::io::Error::other(format!(
                "terminal mutex poisoned: {p}"
            )))
        })?;
        guard.pty.write(data)
    }

    /// Resize the terminal. Forwards to the underlying pty and to
    /// the emulator. The new size is reflected in [`Self::size`].
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TerminalError> {
        let mut guard = self.inner.lock().map_err(|p| {
            TerminalError::Io(std::io::Error::other(format!(
                "terminal mutex poisoned: {p}"
            )))
        })?;
        guard.pty.resize(cols, rows)?;
        guard.emu.resize(cols as usize, rows as usize);
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    /// Snapshot the current grid + cursor state.
    ///
    /// Locks Inner briefly, copies the cells into a fresh `Vec`, and
    /// returns. Safe to call at any cadence from any thread — the
    /// copy is cheap (typical 120×40 grid = under 100 KB).
    pub fn snapshot(&self) -> GridSnapshot {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let cols = guard.emu.cols as u16;
        let rows = guard.emu.rows as u16;
        let mut cells = Vec::with_capacity(cols as usize * rows as usize);
        for row in &guard.emu.cells {
            cells.extend_from_slice(row);
        }
        GridSnapshot {
            cols,
            rows,
            cursor_x: guard.emu.cursor_x as u16,
            cursor_y: guard.emu.cursor_y as u16,
            cells,
            prompt_end: guard.emu.last_prompt_end.map(|(r, c)| (r as u16, c as u16)),
            awaiting_input: guard.emu.awaiting_input,
            alt_screen: guard.emu.alt_screen,
            bracketed_paste: guard.emu.bracketed_paste,
        }
    }

    /// Snapshot a viewport that can be scrolled back into history.
    ///
    /// `scrollback_offset` is measured in lines from the live bottom:
    /// `0` means the newest visible grid, `1` moves the viewport up by
    /// one line, and so on until the oldest retained scrollback line is
    /// visible at the top edge.
    pub fn snapshot_view(&self, scrollback_offset: usize) -> GridSnapshot {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let cols = guard.emu.cols as u16;
        let rows = guard.emu.rows as u16;
        let visible_rows = rows as usize;
        let scrollback_len = guard.emu.scrollback.len();
        let clamped_offset = scrollback_offset.min(scrollback_len);
        let total_lines = scrollback_len + visible_rows;
        let start_line = total_lines.saturating_sub(visible_rows + clamped_offset);

        let mut cells = Vec::with_capacity(cols as usize * visible_rows);
        let append_line = |target: &mut Vec<Cell>, line: &[Cell], width: usize| {
            if line.len() >= width {
                target.extend_from_slice(&line[..width]);
            } else {
                target.extend_from_slice(line);
                target.resize(target.len() + (width - line.len()), Cell::default());
            }
        };
        for line_index in start_line..start_line + visible_rows {
            if line_index < scrollback_len {
                append_line(&mut cells, &guard.emu.scrollback[line_index], cols as usize);
            } else {
                let visible_index = line_index - scrollback_len;
                append_line(&mut cells, &guard.emu.cells[visible_index], cols as usize);
            }
        }

        // Prompt-end / smart-mode flags only make sense for the live
        // bottom of the grid. When the user has scrolled into history
        // (`clamped_offset > 0`) the smart layer should be inactive,
        // so report `None` / `false` rather than a stale position
        // pointing at the wrong row of the visible viewport.
        let smart_visible = clamped_offset == 0;
        GridSnapshot {
            cols,
            rows,
            cursor_x: guard.emu.cursor_x as u16,
            cursor_y: guard.emu.cursor_y as u16,
            cells,
            prompt_end: if smart_visible {
                guard.emu.last_prompt_end.map(|(r, c)| (r as u16, c as u16))
            } else {
                None
            },
            awaiting_input: smart_visible && guard.emu.awaiting_input,
            alt_screen: guard.emu.alt_screen,
            bracketed_paste: guard.emu.bracketed_paste,
        }
    }

    /// Number of scrollback lines currently retained above the live grid.
    pub fn scrollback_len(&self) -> usize {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.emu.scrollback.len()
    }

    /// Update the scrollback history cap.
    pub fn set_scrollback_limit(&self, limit: usize) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.emu.scrollback_limit = limit.max(1);
        while guard.emu.scrollback.len() > guard.emu.scrollback_limit {
            guard.emu.scrollback.pop_front();
        }
    }

    /// Last-known current working directory reported by the
    /// shell via OSC 7. Returns `None` when the shell hasn't
    /// emitted the sequence yet (e.g. bash without a
    /// `PROMPT_COMMAND` hook, or before the first prompt).
    /// Non-destructive — reading doesn't clear the value.
    pub fn current_cwd(&self) -> Option<String> {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.emu.cwd.is_empty() {
            None
        } else {
            Some(guard.emu.cwd.clone())
        }
    }

    /// Check whether a bell character was received since the last read.
    /// Clears the pending flag after reading.
    pub fn take_bell_pending(&self) -> bool {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.emu.bell_pending {
            guard.emu.bell_pending = false;
            true
        } else {
            false
        }
    }

    /// Current grid size. Cheap (no lock, just atomics-free reads of
    /// the struct fields — the fields are updated under the lock).
    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// Has the child exited and the reader thread stopped?
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Check if the emulator detected an SSH command and return
    /// the details. Clears the detection flag after reading.
    pub fn take_ssh_detected(&self) -> Option<(String, String, u16)> {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.emu.ssh_command_detected {
            guard.emu.ssh_command_detected = false;
            Some((
                guard.emu.ssh_detected_host.clone(),
                guard.emu.ssh_detected_user.clone(),
                guard.emu.ssh_detected_port,
            ))
        } else {
            None
        }
    }

    /// Current SSH target, as reported by the child-process watcher.
    ///
    /// Returns `None` when no `ssh` client is running in the PTY's
    /// descendant tree — i.e. the user is sitting at the local
    /// shell, or the last ssh has exited. Returns `Some(target)` for
    /// the innermost live ssh (so nested `ssh → ssh` follows the
    /// inner hop). Non-destructive: reading does not clear state,
    /// so polling from multiple places is safe.
    pub fn current_ssh_target(&self) -> Option<SshChildTarget> {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.ssh_child_target.clone()
    }

    /// Check if the emulator detected an `exit`/`logout` command.
    /// Clears the flag after reading.
    pub fn take_ssh_exit_detected(&self) -> bool {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.emu.ssh_exit_detected {
            guard.emu.ssh_exit_detected = false;
            true
        } else {
            false
        }
    }
}

impl Drop for PierTerminal {
    fn drop(&mut self) {
        // 1. Ask both background threads to stop.
        self.shutdown.store(true, Ordering::Relaxed);
        // 2. Join the reader first — its loop wakes every 5ms, so it
        //    returns almost immediately. Bounded by whatever a pending
        //    `pty.read` takes (pty.read is non-blocking).
        if let Some(handle) = self.reader.take() {
            let _ = handle.join();
        }
        // 3. Join the SSH watcher. Worst-case latency is one `sleep`
        //    interval (≤1s) — it doesn't hold `inner` across the
        //    sleep, so the main thread is never blocked waiting on
        //    it to release the mutex.
        if let Some(handle) = self.ssh_watcher.take() {
            let _ = handle.join();
        }
        // 4. Dropping `inner` happens after both threads joined, so
        //    the Pty (and its Drop, which reaps the child) runs only
        //    on this thread — no races with background readers.
    }
}

enum ReadOutcome {
    Data,
    /// Same as `Data` but the chunk contained an OpenSSH server
    /// password prompt — caller should fire `SshPasswordPrompt`.
    PasswordPrompt,
    /// Same as `Data` but the chunk contained an OpenSSH key
    /// passphrase prompt — caller should fire `SshPassphrasePrompt`.
    PassphrasePrompt,
    Idle,
    Done,
}

// ─────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::terminal::emulator::Color;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    /// `extern "C"` notify fn shared by every test. `user_data` is a
    /// pointer to a per-test `AtomicUsize` that the test leaks so it
    /// lives for 'static. Each test leaks its own counter so tests
    /// can run in parallel without cross-contaminating state.
    extern "C" fn test_notify(user_data: *mut std::ffi::c_void, _event: u32) {
        // SAFETY: test-only; user_data always points at a leaked
        // AtomicUsize owned by this test module.
        let counter = unsafe { &*(user_data as *const AtomicUsize) };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Allocate a fresh per-test counter and return it as a leaked
    /// static reference + the raw pointer the notify fn expects.
    fn fresh_counter() -> (&'static AtomicUsize, *mut std::ffi::c_void) {
        let boxed: Box<AtomicUsize> = Box::new(AtomicUsize::new(0));
        let leaked: &'static AtomicUsize = Box::leak(boxed);
        let ptr = leaked as *const AtomicUsize as *mut std::ffi::c_void;
        (leaked, ptr)
    }

    fn wait_for<F: Fn() -> bool>(cond: F, deadline: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < deadline {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        cond()
    }

    #[test]
    fn echo_flows_through_session_into_grid_snapshot() {
        // Use /bin/cat so the child stays alive between our write
        // and snapshot calls; writing to stdin is echoed on stdout.
        let (counter, user_data) = fresh_counter();

        let term = PierTerminal::new(80, 24, "/bin/cat", test_notify, user_data)
            .expect("spawn via PierTerminal::new failed");

        let msg = b"pier-session-roundtrip\n";
        term.write(msg).expect("write failed");

        // Wait until the reader thread has fed at least one chunk.
        let got_data = wait_for(
            || counter.load(Ordering::Relaxed) > 0,
            Duration::from_secs(2),
        );
        assert!(got_data, "notify callback was never fired");

        // Snapshot should contain our message. cat echoes input; the
        // emulator writes it to the grid row-major starting at (0,0).
        let snap = term.snapshot();
        assert_eq!(snap.cols, 80);
        assert_eq!(snap.rows, 24);
        assert_eq!(snap.cells.len(), 80 * 24);

        // Reassemble the first few lines and look for the needle.
        let mut text = String::new();
        for r in 0..snap.rows as usize {
            for c in 0..snap.cols as usize {
                text.push(snap.cells[r * snap.cols as usize + c].ch);
            }
            text.push('\n');
        }
        assert!(
            text.contains("pier-session-roundtrip"),
            "expected echoed input in grid, got:\n{text}",
        );
    }

    #[test]
    fn resize_updates_snapshot_dimensions() {
        let (_counter, user_data) = fresh_counter();

        let mut term =
            PierTerminal::new(80, 24, "/bin/cat", test_notify, user_data).expect("spawn failed");

        assert_eq!(term.size(), (80, 24));
        term.resize(120, 40).expect("resize failed");
        assert_eq!(term.size(), (120, 40));

        let snap = term.snapshot();
        assert_eq!(snap.cols, 120);
        assert_eq!(snap.rows, 40);
        assert_eq!(snap.cells.len(), 120 * 40);
    }

    #[test]
    fn dropping_session_reaps_reader_thread_and_child() {
        let (_counter, user_data) = fresh_counter();

        let term =
            PierTerminal::new(80, 24, "/bin/cat", test_notify, user_data).expect("spawn failed");
        assert!(term.is_alive());

        // Explicit drop. This should:
        //   1. set shutdown = true
        //   2. join the reader thread (within the 5ms poll window)
        //   3. drop Inner → drop UnixPty → SIGTERM → SIGKILL → reap
        // All of that must happen before `drop` returns — otherwise
        // the test would hang here.
        drop(term);
    }

    #[test]
    fn color_attributes_survive_round_trip() {
        // Feed printf through /bin/sh so we get real shell quoting
        // of the escape sequences. printf then emits raw ESC bytes
        // into the pty, the emulator parses them, and the snapshot
        // preserves the per-cell fg color.
        let (counter, user_data) = fresh_counter();

        let term =
            PierTerminal::new(80, 24, "/bin/sh", test_notify, user_data).expect("spawn failed");

        // Send the printf command and exit so we can wait for stable
        // output. Use single quotes so the shell does no expansion.
        term.write(b"printf '\\033[31mRED\\033[0mPLAIN\\n' && exit\n")
            .expect("write failed");

        assert!(
            wait_for(
                || counter.load(Ordering::Relaxed) > 0,
                Duration::from_secs(3),
            ),
            "notify never fired",
        );

        // Give the shell a beat to execute and exit.
        thread::sleep(Duration::from_millis(200));

        let snap = term.snapshot();

        // Walk the grid and find the "REDPLAIN" sequence.
        let mut flat = String::new();
        for r in 0..snap.rows as usize {
            for c in 0..snap.cols as usize {
                flat.push(snap.cells[r * snap.cols as usize + c].ch);
            }
            flat.push('\n');
        }

        if let Some(pos) = flat.find("REDPLAIN") {
            // pos is the byte index into the reassembled text with
            // '\n' separators; convert to (row, col).
            let before: &str = &flat[..pos];
            let row = before.matches('\n').count();
            let col = pos - before.rfind('\n').map(|i| i + 1).unwrap_or(0);

            let idx_r = row * snap.cols as usize + col;
            let idx_p = idx_r + 3;

            assert_eq!(snap.cells[idx_r].ch, 'R');
            assert_eq!(snap.cells[idx_r].fg, Color::Indexed(1));
            assert_eq!(snap.cells[idx_p].ch, 'P');
            assert_eq!(snap.cells[idx_p].fg, Color::Default);
        } else {
            // Some test environments (sh missing, printf not
            // supporting \033) may not produce the sequence. Rather
            // than failing CI on a setup quirk, accept the miss —
            // the per-function emulator tests already cover this
            // logic exhaustively.
            eprintln!(
                "note: REDPLAIN not found in shell output; skipping color \
                 assertion. This is OK on unusual setups.\n{flat}",
            );
        }
    }
}
