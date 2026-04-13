//! PTY process backend — platform-split behind a trait.
//!
//! Any concrete `Pty` owns a child process and exposes a byte-oriented,
//! non-blocking read/write surface. Rules:
//!
//! * `write` sends bytes directly to the master end of the pty. It
//!   reports the number of bytes accepted by the OS. Short writes are
//!   possible but rare in practice.
//! * `read` is non-blocking. An empty `Ok(Vec::new())` means "no data
//!   right now" (either the OS had nothing to hand us, or the child is
//!   still running but idle). A `Ok(data)` with a non-empty vec is a
//!   chunk of output. An `Err` is a real I/O error.
//! * `resize` updates the child's window size so interactive apps like
//!   `vim` and `htop` reflow.
//! * Dropping the implementation reaps the child. Implementations MUST
//!   NOT block the caller indefinitely on Drop — the upstream Pier
//!   pattern is `SIGTERM` → wait briefly → `SIGKILL` → final `WNOHANG`.

use std::io;

/// Errors that can arise from the terminal subsystem.
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    /// An underlying I/O syscall failed (forkpty, read, write, ioctl, …).
    #[error("terminal I/O: {0}")]
    Io(#[from] io::Error),

    /// This target does not yet have a PTY backend. Currently: Windows.
    /// M2b lands the ConPTY implementation.
    #[error("terminal backend not implemented on this platform yet")]
    Unsupported,
}

/// A pseudo-terminal that owns a running child process.
///
/// The trait is object-safe so callers can hold `Box<dyn Pty>` without
/// caring which platform implementation is underneath.
pub trait Pty: Send {
    /// Non-blocking read of whatever the child has emitted since the
    /// last call. Returns `Ok(Vec::new())` if nothing is available.
    fn read(&mut self) -> Result<Vec<u8>, TerminalError>;

    /// Write bytes to the master end of the pty. Returns the number of
    /// bytes actually written, which may be less than `data.len()` on
    /// short writes.
    fn write(&mut self, data: &[u8]) -> Result<usize, TerminalError>;

    /// Tell the child its window is now `cols × rows` cells.
    fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TerminalError>;

    /// Current grid size, as last set by `resize` (or at spawn time).
    fn size(&self) -> (u16, u16);
}

// ─────────────────────────────────────────────────────────
// Unix implementation — forkpty(3)
// ─────────────────────────────────────────────────────────

#[cfg(unix)]
pub use unix::UnixPty;

#[cfg(unix)]
mod unix {
    use super::{Pty, TerminalError};
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    /// `forkpty`-backed PTY. Spawns a child, stores the master fd in
    /// non-blocking mode, reaps on drop.
    pub struct UnixPty {
        master_fd: OwnedFd,
        child_pid: libc::pid_t,
        cols: u16,
        rows: u16,
    }

    impl UnixPty {
        /// Spawn `program` with `args` inside a new PTY sized to
        /// `cols × rows`. The child process's working directory is
        /// `$HOME` if that env var is set, otherwise unchanged.
        ///
        /// `TERM` is forced to `xterm-256color` and the locale is
        /// pinned to `en_US.UTF-8` inside the child so the emulator
        /// gets a predictable byte stream regardless of how Pier-X
        /// itself was launched.
        pub fn spawn(
            cols: u16,
            rows: u16,
            program: &str,
            args: &[&str],
        ) -> Result<Self, TerminalError> {
            let mut master_fd: libc::c_int = 0;
            let mut win_size = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };

            // SAFETY: forkpty is an async-signal-safe POSIX call; we
            // pass valid pointers or NULL per the man page. After the
            // fork, the child branch only uses async-signal-safe libc
            // calls (putenv, chdir, execvp, _exit).
            let child_pid = unsafe {
                libc::forkpty(
                    &mut master_fd,
                    std::ptr::null_mut(), // slave name — we don't need it
                    std::ptr::null_mut(), // termios — inherit defaults
                    &mut win_size,
                )
            };

            if child_pid < 0 {
                return Err(TerminalError::Io(io::Error::last_os_error()));
            }

            if child_pid == 0 {
                // ── Child ──────────────────────────────────────────
                //
                // IMPORTANT: every CString used via putenv must live
                // until execvp runs. putenv stores the pointer, not
                // the contents. Keeping these bindings alive in this
                // scope (which ends only when execvp replaces the
                // process image) is enough.

                let term = std::ffi::CString::new("TERM=xterm-256color").unwrap();
                let lang = std::ffi::CString::new("LANG=en_US.UTF-8").unwrap();
                let lc_all = std::ffi::CString::new("LC_ALL=en_US.UTF-8").unwrap();
                // SAFETY: putenv(3) accepts a string of the form KEY=VALUE.
                // All three pointers are valid for the entire child lifetime.
                unsafe {
                    libc::putenv(term.as_ptr() as *mut _);
                    libc::putenv(lang.as_ptr() as *mut _);
                    libc::putenv(lc_all.as_ptr() as *mut _);
                }

                // Change to the user's home directory so new shells
                // don't inherit Pier-X's cwd (which is typically the
                // bundle directory, surprising the user).
                if let Ok(home) = std::env::var("HOME") {
                    if let Ok(home_c) = std::ffi::CString::new(home) {
                        // SAFETY: chdir with a valid NUL-terminated path.
                        unsafe {
                            libc::chdir(home_c.as_ptr());
                        }
                    }
                }

                // Build argv for execvp. argv[0] is program itself per
                // POSIX convention; the caller's args follow; the list
                // is terminated with a NULL pointer.
                let program_c = match std::ffi::CString::new(program) {
                    Ok(c) => c,
                    Err(_) => unsafe { libc::_exit(1) },
                };
                let args_c: Vec<std::ffi::CString> = std::iter::once(program.to_string())
                    .chain(args.iter().map(|s| s.to_string()))
                    .map(|s| {
                        std::ffi::CString::new(s)
                            .unwrap_or_else(|_| std::ffi::CString::new("").unwrap())
                    })
                    .collect();
                let args_ptrs: Vec<*const libc::c_char> = args_c
                    .iter()
                    .map(|s| s.as_ptr())
                    .chain(std::iter::once(std::ptr::null()))
                    .collect();

                // SAFETY: execvp takes NUL-terminated argv; on success
                // it never returns. On failure we _exit(1).
                unsafe {
                    libc::execvp(program_c.as_ptr(), args_ptrs.as_ptr());
                    libc::_exit(1);
                }
            }

            // ── Parent ─────────────────────────────────────────────
            //
            // Flip the master fd to non-blocking so our `read` can
            // return `WouldBlock` instead of hanging the UI thread.
            // SAFETY: master_fd is a live fd we just got from forkpty.
            unsafe {
                let flags = libc::fcntl(master_fd, libc::F_GETFL);
                if flags >= 0 {
                    libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                }
            }

            Ok(Self {
                // SAFETY: master_fd is owned by us from this point on.
                master_fd: unsafe { OwnedFd::from_raw_fd(master_fd) },
                child_pid,
                cols,
                rows,
            })
        }

        /// Convenience: spawn a login shell.
        pub fn spawn_shell(cols: u16, rows: u16, shell: &str) -> Result<Self, TerminalError> {
            Self::spawn(cols, rows, shell, &["-l"])
        }

        /// The child process id. Exposed for tests and diagnostics.
        pub fn child_pid(&self) -> libc::pid_t {
            self.child_pid
        }
    }

    impl Pty for UnixPty {
        fn read(&mut self) -> Result<Vec<u8>, TerminalError> {
            let mut buf = vec![0u8; 65_536];
            // SAFETY: fd is owned + live, buf is valid for buf.len() bytes.
            let n = unsafe {
                libc::read(
                    self.master_fd.as_raw_fd(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n > 0 {
                buf.truncate(n as usize);
                Ok(buf)
            } else if n == 0 {
                // EOF — child closed its end of the pty.
                Ok(Vec::new())
            } else {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::WouldBlock {
                    Ok(Vec::new())
                } else {
                    Err(TerminalError::Io(err))
                }
            }
        }

        fn write(&mut self, data: &[u8]) -> Result<usize, TerminalError> {
            // SAFETY: fd is owned + live, data is a valid byte slice.
            let n = unsafe {
                libc::write(
                    self.master_fd.as_raw_fd(),
                    data.as_ptr() as *const libc::c_void,
                    data.len(),
                )
            };
            if n < 0 {
                Err(TerminalError::Io(io::Error::last_os_error()))
            } else {
                Ok(n as usize)
            }
        }

        fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TerminalError> {
            let win_size = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            // SAFETY: TIOCSWINSZ takes a struct winsize*; we pass our own.
            let result =
                unsafe { libc::ioctl(self.master_fd.as_raw_fd(), libc::TIOCSWINSZ, &win_size) };
            if result < 0 {
                Err(TerminalError::Io(io::Error::last_os_error()))
            } else {
                self.cols = cols;
                self.rows = rows;
                Ok(())
            }
        }

        fn size(&self) -> (u16, u16) {
            (self.cols, self.rows)
        }
    }

    impl Drop for UnixPty {
        fn drop(&mut self) {
            // Graceful → escalating shutdown. Never block the caller's
            // thread indefinitely: SIGTERM, up to 5×50ms of WNOHANG
            // polling, SIGKILL after 150ms, final WNOHANG. If the
            // child is still not reaped at this point the OS will
            // clean up the zombie when Pier-X itself exits — we
            // deliberately don't block the drop.
            unsafe {
                libc::kill(self.child_pid, libc::SIGTERM);
                let mut status: libc::c_int = 0;
                for attempt in 0..5 {
                    let waited = libc::waitpid(self.child_pid, &mut status, libc::WNOHANG);
                    if waited != 0 {
                        return;
                    }
                    if attempt == 2 {
                        libc::kill(self.child_pid, libc::SIGKILL);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                libc::waitpid(self.child_pid, &mut status, libc::WNOHANG);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────
// Windows implementation — pipe-backed shell transport
// ─────────────────────────────────────────────────────────

#[cfg(windows)]
pub use windows_impl::WindowsPty;

#[cfg(windows)]
mod windows_impl {
    use super::{Pty, TerminalError};
    use std::io::{self, Read, Write};
    use std::os::windows::process::CommandExt;
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::mpsc::{self, Receiver, TryRecvError};
    use std::thread::{self, JoinHandle};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const READ_CHUNK_SIZE: usize = 8192;

    /// Minimal Windows terminal transport.
    ///
    /// This is not a real pseudo console yet — it launches the shell
    /// with redirected stdin/stdout/stderr pipes and forwards bytes
    /// through the same [`Pty`] trait that Unix uses. That is enough
    /// for interactive PowerShell/cmd sessions while the ConPTY
    /// backend is still pending.
    pub struct WindowsPty {
        child: Child,
        stdin: ChildStdin,
        rx: Receiver<Vec<u8>>,
        reader_threads: Vec<JoinHandle<()>>,
        cols: u16,
        rows: u16,
    }

    impl WindowsPty {
        /// Spawn `program` with redirected stdio and background pipe
        /// readers that merge stdout + stderr into one byte stream.
        pub fn spawn(
            cols: u16,
            rows: u16,
            program: &str,
            args: &[&str],
        ) -> Result<Self, TerminalError> {
            let mut cmd = Command::new(program);
            cmd.args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .creation_flags(CREATE_NO_WINDOW)
                .env("TERM", "xterm-256color");

            if let Ok(home) = std::env::var("USERPROFILE") {
                if !home.is_empty() {
                    cmd.current_dir(home);
                }
            }

            let mut child = cmd.spawn().map_err(TerminalError::Io)?;
            let stdin = child.stdin.take().ok_or_else(|| {
                TerminalError::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "failed to capture child stdin",
                ))
            })?;
            let stdout = child.stdout.take().ok_or_else(|| {
                TerminalError::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "failed to capture child stdout",
                ))
            })?;
            let stderr = child.stderr.take().ok_or_else(|| {
                TerminalError::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "failed to capture child stderr",
                ))
            })?;

            let (tx, rx) = mpsc::channel();
            let mut reader_threads = Vec::with_capacity(2);
            reader_threads.push(spawn_pipe_reader("pier-terminal-stdout", stdout, tx.clone()));
            reader_threads.push(spawn_pipe_reader("pier-terminal-stderr", stderr, tx));

            Ok(Self {
                child,
                stdin,
                rx,
                reader_threads,
                cols,
                rows,
            })
        }

        /// Spawn an interactive shell.
        ///
        /// PowerShell gets `-NoLogo` to suppress the startup banner.
        /// `cmd.exe` gets `/Q /K` so it stays interactive and avoids
        /// echoing every line back twice.
        pub fn spawn_shell(cols: u16, rows: u16, shell: &str) -> Result<Self, TerminalError> {
            let leaf = shell
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or(shell)
                .to_ascii_lowercase();
            let args: &[&str] = match leaf.as_str() {
                "powershell.exe" | "powershell" | "pwsh.exe" | "pwsh" => &["-NoLogo"],
                "cmd.exe" | "cmd" => &["/Q", "/K"],
                _ => &[],
            };
            Self::spawn(cols, rows, shell, args)
        }
    }

    impl Pty for WindowsPty {
        fn read(&mut self) -> Result<Vec<u8>, TerminalError> {
            let mut out = Vec::new();
            loop {
                match self.rx.try_recv() {
                    Ok(chunk) => out.extend_from_slice(&chunk),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        if out.is_empty() {
                            return Err(TerminalError::Io(io::Error::new(
                                io::ErrorKind::BrokenPipe,
                                "terminal child exited",
                            )));
                        }
                        break;
                    }
                }
            }
            Ok(out)
        }
        fn write(&mut self, data: &[u8]) -> Result<usize, TerminalError> {
            self.stdin.write_all(data).map_err(TerminalError::Io)?;
            self.stdin.flush().map_err(TerminalError::Io)?;
            Ok(data.len())
        }
        fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TerminalError> {
            // Pipe-backed shells do not expose a real console buffer to
            // resize yet. Keep the logical size in sync so the UI and
            // emulator still agree on the viewport dimensions.
            self.cols = cols;
            self.rows = rows;
            Ok(())
        }
        fn size(&self) -> (u16, u16) {
            (self.cols, self.rows)
        }
    }

    impl Drop for WindowsPty {
        fn drop(&mut self) {
            let _ = self.stdin.flush();
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
            for handle in self.reader_threads.drain(..) {
                let _ = handle.join();
            }
        }
    }

    fn spawn_pipe_reader<R>(
        thread_name: &str,
        mut pipe: R,
        tx: mpsc::Sender<Vec<u8>>,
    ) -> JoinHandle<()>
    where
        R: Read + Send + 'static,
    {
        thread::Builder::new()
            .name(thread_name.to_string())
            .spawn(move || {
                let mut buf = [0u8; READ_CHUNK_SIZE];
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })
            .expect("spawning Windows pipe reader must not fail in practice")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::thread;
    use std::time::{Duration, Instant};

    /// Drain everything the child has produced within the given
    /// deadline, returning it as a single byte buffer. Useful in tests
    /// because the pty read is non-blocking — we can't assume the
    /// child has flushed by the first call.
    fn drain_until(pty: &mut dyn Pty, deadline: Duration) -> Vec<u8> {
        let start = Instant::now();
        let mut out = Vec::new();
        while start.elapsed() < deadline {
            match pty.read() {
                Ok(chunk) if !chunk.is_empty() => out.extend_from_slice(&chunk),
                Ok(_) => thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }
        out
    }

    #[test]
    fn spawn_echo_captures_output() {
        let mut pty =
            UnixPty::spawn(80, 24, "/bin/echo", &["hello-pier"]).expect("forkpty spawn failed");
        assert_eq!(pty.size(), (80, 24));
        let out = drain_until(&mut pty, Duration::from_secs(2));
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("hello-pier"),
            "expected 'hello-pier' in output, got {:?}",
            s
        );
    }

    #[test]
    fn resize_updates_size_accessor() {
        // /bin/cat with no args stays alive on stdin until we drop.
        let mut pty = UnixPty::spawn(80, 24, "/bin/cat", &[]).expect("forkpty spawn failed");
        assert_eq!(pty.size(), (80, 24));
        pty.resize(120, 40).expect("resize failed");
        assert_eq!(pty.size(), (120, 40));
        // Drop reaps cat via the SIGTERM → SIGKILL escalation.
    }

    #[test]
    fn write_roundtrips_through_cat() {
        // cat echoes its stdin back on stdout through the pty, which
        // gives us a simple loopback to prove write+read work.
        let mut pty = UnixPty::spawn(80, 24, "/bin/cat", &[]).expect("forkpty spawn failed");
        let msg = b"pier-x-roundtrip\n";
        let n = pty.write(msg).expect("write failed");
        assert_eq!(n, msg.len());
        let out = drain_until(&mut pty, Duration::from_secs(2));
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("pier-x-roundtrip"),
            "expected roundtrip text, got {:?}",
            s
        );
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::thread;
    use std::time::{Duration, Instant};

    fn drain_until(pty: &mut dyn Pty, deadline: Duration) -> Vec<u8> {
        let start = Instant::now();
        let mut out = Vec::new();
        while start.elapsed() < deadline {
            match pty.read() {
                Ok(chunk) if !chunk.is_empty() => out.extend_from_slice(&chunk),
                Ok(_) => thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }
        out
    }

    #[test]
    fn spawn_shell_roundtrips_powershell_commands() {
        let mut pty =
            WindowsPty::spawn_shell(80, 24, "powershell.exe").expect("spawn_shell failed");
        pty.write(b"Write-Output 'hello-pier'\r\nexit\r\n")
            .expect("write failed");

        let out = drain_until(&mut pty, Duration::from_secs(5));
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("hello-pier"),
            "expected hello-pier in output, got {:?}",
            s,
        );
    }
}
