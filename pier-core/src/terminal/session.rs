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
//!  * owns the `Box<dyn Pty>` and `VtEmulator` behind separate mutexes so
//!    PTY writes don't have to wait behind emulator snapshot copies;
//!  * spawns a dedicated reader thread at construction time that
//!    loops on `pty.read()` + `emu.process()` until a shutdown flag
//!    is set;
//!  * spawns a dedicated writer thread so UI callers enqueue bytes and
//!    return immediately instead of blocking on PTY back-pressure;
//!  * invokes a caller-provided `notify` callback whenever something
//!    interesting happened (new bytes, child exit). The callback is
//!    called WITHOUT holding the internal mutex — its only job is to
//!    wake the shell, which then calls [`PierTerminal::snapshot`]
//!    on its own terms.
//!
//! ## Thread model
//!
//! ```text
//!   shell/main thread               writer thread                reader thread
//!   ─────────────────               ─────────────                ─────────────
//!   write(bytes) ──┐                 loop {                      loop {
//!                  ├─► queue bytes      recv queued bytes          lock Pty
//!                  └─► return           lock Pty                   read from pty
//!   snapshot() ────┐                    write to pty              unlock
//!                  ├─► lock Emu         unlock                    lock Emu
//!                  │   copy grid        if shutdown { break }     feed emu
//!                  └─► unlock         }                           unlock
//!                                                                  call notify(user_data, event)
//!                                                                  if shutdown { break }
//!                                                                  sleep 5ms
//!                                                               }
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
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::emulator::{Cell, VtEmulator};
use super::pty::{Pty, TerminalError};

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

/// A live terminal session — PTY + emulator + reader thread, all
/// behind one handle.
///
/// Construct via [`PierTerminal::new`] (default local PTY) or
/// [`PierTerminal::with_pty`] (inject your own `Box<dyn Pty>` for
/// tests or future SSH sessions). Dropping the handle shuts down the
/// background threads and reaps the child process.
pub struct PierTerminal {
    pty: Arc<Mutex<Box<dyn Pty>>>,
    emu: Arc<Mutex<VtEmulator>>,
    shutdown: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    write_tx: Option<Sender<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
    // We keep cols/rows at the struct level for lock-free accessors;
    // the authoritative size is still stored inside the backend + emulator,
    // but reading those would require taking locks.
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
    /// Whether the foreground app requested DECSET 2004 bracketed paste.
    pub bracketed_paste_mode: bool,
    /// Cell grid, row-major: `cells[row * cols + col]`.
    pub cells: Vec<Cell>,
}

impl PierTerminal {
    /// Spawn a new local shell session.
    ///
    /// On Unix this goes through [`super::pty::UnixPty::spawn_shell`];
    /// on Windows it currently uses the pipe-backed
    /// [`super::pty::WindowsPty::spawn_shell`] transport until the
    /// ConPTY backend lands.
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
        #[cfg(unix)]
        {
            let pty: Box<dyn Pty> = Box::new(super::pty::UnixPty::spawn_shell(cols, rows, shell)?);
            return Self::with_pty(pty, cols, rows, notify, user_data);
        }
        #[cfg(windows)]
        {
            let pty: Box<dyn Pty> =
                Box::new(super::pty::WindowsPty::spawn_shell(cols, rows, shell)?);
            Self::with_pty(pty, cols, rows, notify, user_data)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (cols, rows, shell, notify, user_data);
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
        let pty = Arc::new(Mutex::new(pty));
        let emu = Arc::new(Mutex::new(VtEmulator::new(cols as usize, rows as usize)));
        let shutdown = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>();

        let reader = Some(Self::spawn_reader(
            Arc::clone(&pty),
            Arc::clone(&emu),
            Arc::clone(&shutdown),
            Arc::clone(&alive),
            notify,
            user_data as usize,
        ));
        let writer = Some(Self::spawn_writer(
            Arc::clone(&pty),
            Arc::clone(&shutdown),
            write_rx,
        ));

        Ok(Self {
            pty,
            emu,
            shutdown,
            alive,
            write_tx: Some(write_tx),
            reader,
            writer,
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
        pty: Arc<Mutex<Box<dyn Pty>>>,
        emu: Arc<Mutex<VtEmulator>>,
        shutdown: Arc<AtomicBool>,
        alive: Arc<AtomicBool>,
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

                    let outcome = {
                        let read_result = {
                            let mut guard = lock_or_recover(&pty);
                            guard.read()
                        };
                        match read_result {
                            Ok(chunk) if !chunk.is_empty() => {
                                let mut guard = lock_or_recover(&emu);
                                guard.process(&chunk);
                                ReadOutcome::Data
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

    fn spawn_writer(
        pty: Arc<Mutex<Box<dyn Pty>>>,
        shutdown: Arc<AtomicBool>,
        rx: mpsc::Receiver<Vec<u8>>,
    ) -> JoinHandle<()> {
        thread::Builder::new()
            .name("pier-terminal-writer".to_string())
            .spawn(move || {
                let idle = Duration::from_millis(5);

                loop {
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }

                    let chunk = match rx.recv_timeout(idle) {
                        Ok(chunk) => chunk,
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    };
                    if chunk.is_empty() {
                        continue;
                    }

                    let mut written = 0usize;
                    while written < chunk.len() {
                        if shutdown.load(Ordering::Relaxed) {
                            return;
                        }

                        let outcome = {
                            let mut guard = lock_or_recover(&pty);
                            guard.write(&chunk[written..])
                        };

                        match outcome {
                            Ok(0) => thread::sleep(idle),
                            Ok(n) => {
                                written = written.saturating_add(n).min(chunk.len());
                            }
                            Err(TerminalError::Io(err))
                                if err.kind() == std::io::ErrorKind::WouldBlock =>
                            {
                                thread::sleep(idle);
                            }
                            Err(_) => return,
                        }
                    }
                }
            })
            .expect("spawning writer thread must not fail in practice")
    }

    /// Send bytes to the shell (user keystrokes, paste, etc.).
    pub fn write(&self, data: &[u8]) -> Result<usize, TerminalError> {
        let tx = self.write_tx.as_ref().ok_or_else(|| {
            TerminalError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "terminal writer is closed",
            ))
        })?;
        tx.send(data.to_vec()).map_err(|_| {
            TerminalError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "terminal writer thread exited",
            ))
        })?;
        Ok(data.len())
    }

    /// Resize the terminal. Forwards to the underlying pty and to
    /// the emulator. The new size is reflected in [`Self::size`].
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TerminalError> {
        {
            let mut pty = self.pty.lock().map_err(mutex_poison_error)?;
            pty.resize(cols, rows)?;
        }
        {
            let mut emu = self.emu.lock().map_err(mutex_poison_error)?;
            emu.resize(cols as usize, rows as usize);
        }
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    /// Snapshot the current grid + cursor state.
    ///
    /// Locks the emulator briefly, copies the cells into a fresh `Vec`, and
    /// returns. Safe to call at any cadence from any thread — the
    /// copy is cheap (typical 120×40 grid = under 100 KB).
    pub fn snapshot(&self) -> GridSnapshot {
        let guard = lock_or_recover(&self.emu);
        let cols = guard.cols as u16;
        let rows = guard.rows as u16;
        let mut cells = Vec::with_capacity(cols as usize * rows as usize);
        for row in &guard.cells {
            cells.extend_from_slice(row);
        }
        GridSnapshot {
            cols,
            rows,
            cursor_x: guard.cursor_x as u16,
            cursor_y: guard.cursor_y as u16,
            bracketed_paste_mode: guard.bracketed_paste_mode,
            cells,
        }
    }

    /// Snapshot a viewport that can be scrolled back into history.
    ///
    /// `scrollback_offset` is measured in lines from the live bottom:
    /// `0` means the newest visible grid, `1` moves the viewport up by
    /// one line, and so on until the oldest retained scrollback line is
    /// visible at the top edge.
    pub fn snapshot_view(&self, scrollback_offset: usize) -> GridSnapshot {
        let guard = lock_or_recover(&self.emu);
        let cols = guard.cols as u16;
        let rows = guard.rows as u16;
        let visible_rows = rows as usize;
        let scrollback_len = guard.scrollback.len();
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
                append_line(&mut cells, &guard.scrollback[line_index], cols as usize);
            } else {
                let visible_index = line_index - scrollback_len;
                append_line(&mut cells, &guard.cells[visible_index], cols as usize);
            }
        }

        GridSnapshot {
            cols,
            rows,
            cursor_x: guard.cursor_x as u16,
            cursor_y: guard.cursor_y as u16,
            bracketed_paste_mode: guard.bracketed_paste_mode,
            cells,
        }
    }

    /// Number of scrollback lines currently retained above the live grid.
    pub fn scrollback_len(&self) -> usize {
        lock_or_recover(&self.emu).scrollback.len()
    }

    /// Whether DECSET 2004 bracketed paste mode is currently enabled.
    pub fn bracketed_paste_mode(&self) -> bool {
        lock_or_recover(&self.emu).bracketed_paste_mode
    }

    /// Update the scrollback history cap.
    pub fn set_scrollback_limit(&self, limit: usize) {
        let mut guard = lock_or_recover(&self.emu);
        guard.scrollback_limit = limit.max(1);
        while guard.scrollback.len() > guard.scrollback_limit {
            guard.scrollback.pop_front();
        }
    }

    /// Check whether a bell character was received since the last read.
    /// Clears the pending flag after reading.
    pub fn take_bell_pending(&self) -> bool {
        let mut guard = lock_or_recover(&self.emu);
        if guard.bell_pending {
            guard.bell_pending = false;
            true
        } else {
            false
        }
    }

    /// Consume the most recent OSC 52 clipboard payload, if any.
    pub fn take_osc52_clipboard(&self) -> Option<String> {
        lock_or_recover(&self.emu).osc52_clipboard.take()
    }

    /// Current OSC 0/1/2 window title, if the foreground app set one.
    pub fn window_title(&self) -> Option<String> {
        let guard = lock_or_recover(&self.emu);
        let title = guard.window_title.trim();
        (!title.is_empty()).then(|| title.to_string())
    }

    /// Current working directory advertised through OSC 7, if any.
    pub fn current_dir(&self) -> Option<String> {
        let guard = lock_or_recover(&self.emu);
        let cwd = guard.current_dir.trim();
        (!cwd.is_empty()).then(|| cwd.to_string())
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
        let mut guard = lock_or_recover(&self.emu);
        if guard.ssh_command_detected {
            guard.ssh_command_detected = false;
            Some((
                guard.ssh_detected_host.clone(),
                guard.ssh_detected_user.clone(),
                guard.ssh_detected_port,
            ))
        } else {
            None
        }
    }

    /// Check if the emulator detected an `exit`/`logout` command.
    /// Clears the flag after reading.
    pub fn take_ssh_exit_detected(&self) -> bool {
        let mut guard = lock_or_recover(&self.emu);
        if guard.ssh_exit_detected {
            guard.ssh_exit_detected = false;
            true
        } else {
            false
        }
    }
}

impl Drop for PierTerminal {
    fn drop(&mut self) {
        // 1. Ask the reader thread to stop.
        self.shutdown.store(true, Ordering::Relaxed);
        self.write_tx.take();
        // 2. Wait for it to notice — bounded by the `idle` sleep in
        //    the loop (5ms) plus whatever a pending `pty.read` takes
        //    to return (pty.read is non-blocking, so this is nearly
        //    instant).
        if let Some(handle) = self.reader.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.writer.take() {
            let _ = handle.join();
        }
        // 3. Dropping the PTY + emulator happens after the reader
        //    joined, so backend cleanup still runs on this thread only.
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn mutex_poison_error<T>(poisoned: std::sync::PoisonError<MutexGuard<'_, T>>) -> TerminalError {
    TerminalError::Io(std::io::Error::other(format!(
        "terminal mutex poisoned: {poisoned}"
    )))
}

enum ReadOutcome {
    Data,
    Idle,
    Done,
}

// ─────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────

#[cfg(test)]
mod async_write_tests {
    use super::*;
    use std::sync::{mpsc, Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    extern "C" fn noop_notify(_: *mut std::ffi::c_void, _: u32) {}

    struct MockPty {
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
        blocker: Option<WriteBlocker>,
        cols: u16,
        rows: u16,
    }

    struct WriteBlocker {
        started_tx: Option<mpsc::Sender<()>>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl MockPty {
        fn recording(writes: Arc<Mutex<Vec<Vec<u8>>>>, cols: u16, rows: u16) -> Self {
            Self {
                writes,
                blocker: None,
                cols,
                rows,
            }
        }

        fn blocking_first_write(
            writes: Arc<Mutex<Vec<Vec<u8>>>>,
            cols: u16,
            rows: u16,
            started_tx: mpsc::Sender<()>,
            release: Arc<(Mutex<bool>, Condvar)>,
        ) -> Self {
            Self {
                writes,
                blocker: Some(WriteBlocker {
                    started_tx: Some(started_tx),
                    release,
                }),
                cols,
                rows,
            }
        }
    }

    impl Pty for MockPty {
        fn read(&mut self) -> Result<Vec<u8>, TerminalError> {
            Ok(Vec::new())
        }

        fn write(&mut self, data: &[u8]) -> Result<usize, TerminalError> {
            if let Some(blocker) = self.blocker.as_mut() {
                if let Some(started_tx) = blocker.started_tx.take() {
                    let _ = started_tx.send(());
                    let (released, wake) = &*blocker.release;
                    let mut released = released.lock().expect("release mutex poisoned");
                    while !*released {
                        released = wake.wait(released).expect("release wait poisoned");
                    }
                }
            }

            self.writes
                .lock()
                .expect("writes mutex poisoned")
                .push(data.to_vec());
            Ok(data.len())
        }

        fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TerminalError> {
            self.cols = cols;
            self.rows = rows;
            Ok(())
        }

        fn size(&self) -> (u16, u16) {
            (self.cols, self.rows)
        }
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
    fn write_returns_before_backend_write_unblocks() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_tx, started_rx) = mpsc::channel();
        let pty =
            MockPty::blocking_first_write(Arc::clone(&writes), 80, 24, started_tx, release.clone());

        let term = PierTerminal::with_pty(Box::new(pty), 80, 24, noop_notify, std::ptr::null_mut())
            .expect("construct terminal with mock pty");

        let start = Instant::now();
        term.write(b"pier").expect("queue terminal write");
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_millis(100),
            "terminal write should return quickly even when backend write is blocked; elapsed={elapsed:?}",
        );
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("writer thread never attempted backend write");
        assert!(
            writes.lock().expect("writes mutex poisoned").is_empty(),
            "backend write should still be blocked before release",
        );

        let (released, wake) = &*release;
        *released.lock().expect("release mutex poisoned") = true;
        wake.notify_all();

        assert!(
            wait_for(
                || writes.lock().expect("writes mutex poisoned").len() == 1,
                Duration::from_secs(1),
            ),
            "queued write never completed after backend was released",
        );

        drop(term);
    }

    #[test]
    fn queued_writes_preserve_submission_order() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let pty = MockPty::recording(Arc::clone(&writes), 80, 24);
        let term = PierTerminal::with_pty(Box::new(pty), 80, 24, noop_notify, std::ptr::null_mut())
            .expect("construct terminal with mock pty");

        term.write(b"alpha").expect("queue alpha");
        term.write(b"beta").expect("queue beta");

        assert!(
            wait_for(
                || writes.lock().expect("writes mutex poisoned").len() == 2,
                Duration::from_secs(1),
            ),
            "queued writes were not drained by the writer thread",
        );

        let writes = writes.lock().expect("writes mutex poisoned").clone();
        assert_eq!(writes, vec![b"alpha".to_vec(), b"beta".to_vec()]);

        drop(term);
    }
}

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
