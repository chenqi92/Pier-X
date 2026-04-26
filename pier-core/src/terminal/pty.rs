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

    /// This target has no PTY backend. Unix uses `forkpty(3)` and
    /// Windows uses ConPTY; anything else (WASM, other unixes we
    /// haven't compiled for) falls into this variant.
    #[error("terminal backend not implemented on this platform")]
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

    /// OS-level process id of the direct child this PTY spawned, if
    /// the backend can expose one. Used by the SSH watcher to scan
    /// the descendant tree for an `ssh` client the user may have
    /// launched inside the terminal. Default `None` so alternate
    /// backends (remote SSH channel, tests) don't need to lie about
    /// a pid.
    fn child_pid(&self) -> Option<u32> {
        None
    }
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
        /// Smart-mode init artefacts (temp rcfile / ZDOTDIR), kept
        /// alive so the temp dir under them is removed only after the
        /// shell exits. `None` for plain (non-smart) sessions.
        _smart_init: Option<crate::terminal::smart::SmartShellInit>,
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
            Self::spawn_with_env(cols, rows, program, args, &[])
        }

        /// Like [`Self::spawn`] but applies `extra_env` (KEY, VALUE
        /// pairs) in the child branch before `execvp`. Used by the
        /// smart-mode launcher (`smart.rs`) to inject `ZDOTDIR` /
        /// `PIERX_REAL_ZDOTDIR` without polluting the parent process.
        pub fn spawn_with_env(
            cols: u16,
            rows: u16,
            program: &str,
            args: &[&str],
            extra_env: &[(&str, &str)],
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

                // Smart-mode env overrides. Built before execvp; the
                // CStrings live in this scope which only ends when
                // execvp replaces the process image — putenv keeps
                // its pointers valid for that long.
                let extra_env_strings: Vec<std::ffi::CString> = extra_env
                    .iter()
                    .filter_map(|(k, v)| std::ffi::CString::new(format!("{}={}", k, v)).ok())
                    .collect();
                for entry in &extra_env_strings {
                    // SAFETY: same contract as the TERM/LANG putenvs above.
                    unsafe {
                        libc::putenv(entry.as_ptr() as *mut _);
                    }
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
            // return `WouldBlock` instead of hanging the caller.
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
                _smart_init: None,
            })
        }

        /// Convenience: spawn a login shell.
        pub fn spawn_shell(cols: u16, rows: u16, shell: &str) -> Result<Self, TerminalError> {
            Self::spawn(cols, rows, shell, &["-l"])
        }

        /// Like [`Self::spawn_shell`] but applies `extra_env` to the
        /// child before `execvp`. Used by callers that need to
        /// inject overrides (e.g. a PATH prefix carrying an `ssh`
        /// wrapper for ControlMaster) without polluting the parent
        /// process's environment.
        pub fn spawn_shell_with_env(
            cols: u16,
            rows: u16,
            shell: &str,
            extra_env: &[(&str, &str)],
        ) -> Result<Self, TerminalError> {
            Self::spawn_with_env(cols, rows, shell, &["-l"], extra_env)
        }

        /// Spawn an interactive shell with smart-mode init applied.
        ///
        /// The caller owns the [`SmartShellInit`]; this method forwards
        /// `init.args` and `init.env` to `forkpty`/`execvp`, then
        /// stashes the init in the returned PTY so the temp directory
        /// it carries is removed only when the PTY (and child shell)
        /// goes away. If `init.recognised` is false the caller should
        /// fall back to [`Self::spawn_shell`] instead — there's nothing
        /// for this method to do.
        pub fn spawn_shell_smart(
            cols: u16,
            rows: u16,
            shell: &str,
            init: crate::terminal::smart::SmartShellInit,
        ) -> Result<Self, TerminalError> {
            Self::spawn_shell_smart_with_env(cols, rows, shell, init, &[])
        }

        /// Like [`Self::spawn_shell_smart`] but additionally layers
        /// `extra_env` on top — `init.env` wins on duplicate keys
        /// because the smart layer's overrides (ZDOTDIR etc.) are
        /// authoritative. Used by callers that need to add extras
        /// (e.g. a PATH prefix for an `ssh` wrapper) without losing
        /// the smart-mode init.
        pub fn spawn_shell_smart_with_env(
            cols: u16,
            rows: u16,
            shell: &str,
            init: crate::terminal::smart::SmartShellInit,
            extra_env: &[(&str, &str)],
        ) -> Result<Self, TerminalError> {
            let args_owned: Vec<&str> = init.args.iter().map(String::as_str).collect();
            // Merge: extras first, then smart's env overrides any
            // colliding key. We keep ownership in this Vec so the
            // &str slice we hand to spawn_with_env is valid for the
            // duration of the call.
            let mut merged: Vec<(String, String)> = extra_env
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect();
            for (k, v) in &init.env {
                if let Some(slot) = merged.iter_mut().find(|(mk, _)| mk == k) {
                    slot.1 = v.clone();
                } else {
                    merged.push((k.clone(), v.clone()));
                }
            }
            let merged_refs: Vec<(&str, &str)> = merged
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let mut pty = Self::spawn_with_env(cols, rows, shell, &args_owned, &merged_refs)?;
            pty._smart_init = Some(init);
            Ok(pty)
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

        fn child_pid(&self) -> Option<u32> {
            if self.child_pid > 0 {
                Some(self.child_pid as u32)
            } else {
                None
            }
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
// Windows implementation — ConPTY-backed shell transport
// ─────────────────────────────────────────────────────────

#[cfg(windows)]
pub use windows_impl::WindowsPty;

#[cfg(windows)]
mod windows_impl {
    use super::{Pty, TerminalError};
    use std::ffi::{c_void, OsStr};
    use std::fs::File;
    use std::io::{self, Read, Write};
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::sync::mpsc::{self, Receiver, TryRecvError};
    use std::thread::{self, JoinHandle};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT};
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Console::{
        ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
        TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
        CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };

    const READ_CHUNK_SIZE: usize = 8192;
    const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;

    /// Minimal Windows terminal transport.
    ///
    /// Uses the native Windows pseudoconsole (ConPTY) so interactive
    /// shells get proper line editing, cursor movement, ANSI output,
    /// and resize semantics instead of the degraded pipe-only mode.
    pub struct WindowsPty {
        process: Option<OwnedHandle>,
        /// PID captured from `PROCESS_INFORMATION.dwProcessId` at spawn
        /// time. We stash it eagerly (instead of calling `GetProcessId`
        /// on demand) because the SSH watcher polls every second and
        /// we'd rather not syscall just to re-read a value that never
        /// changes for the lifetime of this session.
        child_pid: u32,
        pseudoconsole: Option<PseudoConsole>,
        stdin: Option<File>,
        rx: Receiver<Vec<u8>>,
        reader_threads: Vec<JoinHandle<()>>,
        cols: u16,
        rows: u16,
    }

    impl WindowsPty {
        /// Spawn `program` inside a Windows pseudoconsole and forward
        /// its VT stream through the shared [`Pty`] trait.
        pub fn spawn(
            cols: u16,
            rows: u16,
            program: &str,
            args: &[&str],
        ) -> Result<Self, TerminalError> {
            let (input_read_side, input_write_side) = create_pipe_pair()?;
            let (output_read_side, output_write_side) = create_pipe_pair()?;

            let size = COORD {
                X: cols as i16,
                Y: rows as i16,
            };
            let mut hpc: HPCON = 0;
            let hr = unsafe {
                CreatePseudoConsole(size, input_read_side, output_write_side, 0, &mut hpc)
            };
            if hr < 0 {
                unsafe {
                    CloseHandle(input_read_side);
                    CloseHandle(output_write_side);
                    CloseHandle(input_write_side);
                    CloseHandle(output_read_side);
                }
                return Err(hresult_error("CreatePseudoConsole", hr));
            }

            let pseudoconsole = PseudoConsole(hpc);
            let attr_list = ProcThreadAttributeList::new(hpc)?;

            let mut startup_info: STARTUPINFOEXW = unsafe { zeroed() };
            startup_info.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup_info.StartupInfo.dwFlags |= STARTF_USESTDHANDLES;
            startup_info.StartupInfo.hStdInput = std::ptr::null_mut();
            startup_info.StartupInfo.hStdOutput = std::ptr::null_mut();
            startup_info.StartupInfo.hStdError = std::ptr::null_mut();
            startup_info.lpAttributeList = attr_list.as_ptr();

            let command_line = build_command_line(program, args);
            let mut command_line_wide = wide_null(&command_line);
            let current_dir = std::env::var_os("USERPROFILE")
                .filter(|dir| !dir.is_empty())
                .map(|dir| wide_null_os(dir.as_os_str()));
            let env_block = env_block_for_child(program);

            let mut process_info: PROCESS_INFORMATION = unsafe { zeroed() };
            let create_ok = unsafe {
                CreateProcessW(
                    std::ptr::null(),
                    command_line_wide.as_mut_ptr(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                    EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                    env_block
                        .as_ref()
                        .map(|block| block.as_ptr() as *mut c_void)
                        .unwrap_or(std::ptr::null_mut()),
                    current_dir
                        .as_ref()
                        .map(|dir| dir.as_ptr())
                        .unwrap_or(std::ptr::null()),
                    &startup_info.StartupInfo,
                    &mut process_info,
                )
            };
            unsafe {
                CloseHandle(input_read_side);
                CloseHandle(output_write_side);
            }
            if create_ok == 0 {
                unsafe {
                    CloseHandle(input_write_side);
                    CloseHandle(output_read_side);
                }
                return Err(TerminalError::Io(io::Error::last_os_error()));
            }

            unsafe {
                CloseHandle(process_info.hThread);
            }

            let stdin = unsafe { File::from_raw_handle(input_write_side) };
            let output = unsafe { File::from_raw_handle(output_read_side) };
            let process = unsafe { OwnedHandle::from_raw_handle(process_info.hProcess) };
            let child_pid = process_info.dwProcessId;

            let (tx, rx) = mpsc::channel();
            let reader_threads = vec![spawn_pipe_reader("pier-terminal-conpty", output, tx)];

            Ok(Self {
                process: Some(process),
                child_pid,
                pseudoconsole: Some(pseudoconsole),
                stdin: Some(stdin),
                rx,
                reader_threads,
                cols,
                rows,
            })
        }

        /// Spawn an interactive shell.
        ///
        /// PowerShell gets a profile-less interactive launch so local
        /// Pier-X sessions don't inherit line editor hooks or prompt
        /// scripts that assume a native Win32 console host.
        /// `cmd.exe` gets `/Q /K` so it stays interactive and avoids
        /// echoing every line back twice.
        pub fn spawn_shell(cols: u16, rows: u16, shell: &str) -> Result<Self, TerminalError> {
            let leaf = shell
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or(shell)
                .to_ascii_lowercase();
            let args: &[&str] = match leaf.as_str() {
                "powershell.exe" | "powershell" | "pwsh.exe" | "pwsh" => {
                    &["-NoLogo", "-NoExit", "-NoProfile"]
                }
                "cmd.exe" | "cmd" => &["/D", "/Q", "/K"],
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
            let stdin = self.stdin.as_mut().ok_or_else(|| {
                TerminalError::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "terminal stdin is closed",
                ))
            })?;
            stdin.write_all(data).map_err(TerminalError::Io)?;
            stdin.flush().map_err(TerminalError::Io)?;
            Ok(data.len())
        }
        fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TerminalError> {
            let hpc = self
                .pseudoconsole
                .as_ref()
                .map(|console| console.0)
                .ok_or_else(|| {
                    TerminalError::Io(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "pseudoconsole is closed",
                    ))
                })?;
            let size = COORD {
                X: cols as i16,
                Y: rows as i16,
            };
            let hr = unsafe { ResizePseudoConsole(hpc, size) };
            if hr < 0 {
                return Err(hresult_error("ResizePseudoConsole", hr));
            }
            self.cols = cols;
            self.rows = rows;
            Ok(())
        }
        fn size(&self) -> (u16, u16) {
            (self.cols, self.rows)
        }
        fn child_pid(&self) -> Option<u32> {
            if self.child_pid > 0 {
                Some(self.child_pid)
            } else {
                None
            }
        }
    }

    impl Drop for WindowsPty {
        fn drop(&mut self) {
            if let Some(mut stdin) = self.stdin.take() {
                let _ = stdin.flush();
                drop(stdin);
            }
            if let Some(console) = self.pseudoconsole.take() {
                drop(console);
            }
            if let Some(process) = self.process.as_ref() {
                let handle = process.as_raw_handle() as HANDLE;
                let wait = unsafe { WaitForSingleObject(handle, 250) };
                if wait == WAIT_TIMEOUT {
                    unsafe {
                        TerminateProcess(handle, 1);
                    }
                    let _ = unsafe { WaitForSingleObject(handle, 2000) };
                }
            }
            self.process.take();
            for handle in self.reader_threads.drain(..) {
                let _ = handle.join();
            }
        }
    }

    struct PseudoConsole(HPCON);

    impl Drop for PseudoConsole {
        fn drop(&mut self) {
            unsafe {
                ClosePseudoConsole(self.0);
            }
        }
    }

    struct ProcThreadAttributeList {
        _storage: Box<[usize]>,
        ptr: LPPROC_THREAD_ATTRIBUTE_LIST,
    }

    impl ProcThreadAttributeList {
        fn new(hpc: HPCON) -> Result<Self, TerminalError> {
            let mut bytes_required = 0usize;
            unsafe {
                InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes_required);
            }
            if bytes_required == 0 {
                return Err(TerminalError::Io(io::Error::last_os_error()));
            }

            let words = bytes_required.div_ceil(size_of::<usize>());
            let mut storage = vec![0usize; words].into_boxed_slice();
            let ptr = storage.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
            let init_ok =
                unsafe { InitializeProcThreadAttributeList(ptr, 1, 0, &mut bytes_required) };
            if init_ok == 0 {
                return Err(TerminalError::Io(io::Error::last_os_error()));
            }

            let update_ok = unsafe {
                UpdateProcThreadAttribute(
                    ptr,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                    hpc as *mut c_void,
                    size_of::<HPCON>(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if update_ok == 0 {
                unsafe {
                    DeleteProcThreadAttributeList(ptr);
                }
                return Err(TerminalError::Io(io::Error::last_os_error()));
            }

            Ok(Self {
                _storage: storage,
                ptr,
            })
        }

        fn as_ptr(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
            self.ptr
        }
    }

    impl Drop for ProcThreadAttributeList {
        fn drop(&mut self) {
            unsafe {
                DeleteProcThreadAttributeList(self.ptr);
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

    fn create_pipe_pair() -> Result<(HANDLE, HANDLE), TerminalError> {
        let mut read_side: HANDLE = std::ptr::null_mut();
        let mut write_side: HANDLE = std::ptr::null_mut();
        let attrs = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        let ok = unsafe { CreatePipe(&mut read_side, &mut write_side, &attrs, 0) };
        if ok == 0 {
            Err(TerminalError::Io(io::Error::last_os_error()))
        } else {
            Ok((read_side, write_side))
        }
    }

    fn build_command_line(program: &str, args: &[&str]) -> String {
        let mut parts = Vec::with_capacity(args.len() + 1);
        parts.push(quote_windows_arg(program));
        for arg in args {
            parts.push(quote_windows_arg(arg));
        }
        parts.join(" ")
    }

    fn quote_windows_arg(arg: &str) -> String {
        if !arg.contains([' ', '\t', '"']) {
            return arg.to_string();
        }

        let mut out = String::from("\"");
        let mut backslashes = 0usize;
        for ch in arg.chars() {
            match ch {
                '\\' => backslashes += 1,
                '"' => {
                    out.push_str(&"\\".repeat(backslashes * 2 + 1));
                    out.push('"');
                    backslashes = 0;
                }
                _ => {
                    out.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                    out.push(ch);
                }
            }
        }
        out.push_str(&"\\".repeat(backslashes * 2));
        out.push('"');
        out
    }

    fn wide_null(value: &str) -> Vec<u16> {
        wide_null_os(OsStr::new(value))
    }

    fn wide_null_os(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn hresult_error(op: &str, hr: i32) -> TerminalError {
        TerminalError::Io(io::Error::other(format!("{op} failed: HRESULT 0x{hr:08X}")))
    }

    fn env_block_for_child(program: &str) -> Option<Vec<u16>> {
        let leaf = program
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(program)
            .to_ascii_lowercase();
        if leaf != "powershell.exe" && leaf != "powershell" && leaf != "pwsh.exe" && leaf != "pwsh"
        {
            return None;
        }

        let mut vars: Vec<(String, String)> = std::env::vars_os()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect();

        let mut replaced = false;
        for (key, value) in vars.iter_mut() {
            if key.eq_ignore_ascii_case("PSREADLINE_VTINPUT") {
                *value = "0".to_string();
                replaced = true;
                break;
            }
        }
        if !replaced {
            vars.push(("PSREADLINE_VTINPUT".to_string(), "0".to_string()));
        }

        vars.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));

        let mut block = Vec::new();
        for (key, value) in vars {
            block.extend(OsStr::new(&(key + "=" + &value)).encode_wide());
            block.push(0);
        }
        block.push(0);
        Some(block)
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

    fn drain_until_contains(pty: &mut dyn Pty, deadline: Duration, needle: &str) -> String {
        let start = Instant::now();
        let mut out = Vec::new();
        while start.elapsed() < deadline {
            match pty.read() {
                Ok(chunk) if !chunk.is_empty() => {
                    out.extend_from_slice(&chunk);
                    if String::from_utf8_lossy(&out).contains(needle) {
                        break;
                    }
                }
                Ok(_) => thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn spawn_captures_cmd_output_through_conpty() {
        let mut pty = WindowsPty::spawn(80, 24, "cmd.exe", &["/D", "/C", "echo hello-pier"])
            .expect("spawn failed");

        let out = drain_until(&mut pty, Duration::from_secs(5));
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("hello-pier"),
            "expected hello-pier in output, got {s:?}",
        );
    }

    #[test]
    fn spawn_interactive_powershell_shell_accepts_commands() {
        let mut pty =
            WindowsPty::spawn_shell(80, 24, "powershell.exe").expect("spawn shell failed");

        thread::sleep(Duration::from_millis(250));
        let _ = drain_until(&mut pty, Duration::from_millis(500));

        pty.write(b"Write-Output 'hello-pier'\r\nexit\r\n")
            .expect("write failed");

        let out = drain_until(&mut pty, Duration::from_secs(5));
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("hello-pier"),
            "expected hello-pier in output, got {s:?}",
        );
    }

    #[test]
    fn interactive_powershell_backspace_edits_the_pending_line() {
        let mut pty =
            WindowsPty::spawn_shell(80, 24, "powershell.exe").expect("spawn shell failed");

        let prompt = drain_until_contains(&mut pty, Duration::from_secs(10), "PS ");
        assert!(
            prompt.contains("PS "),
            "expected an interactive PowerShell prompt before typing, got {prompt:?}",
        );

        pty.write(b"echo abcd\x7f\x7fXY\r\nexit\r\n")
            .expect("write failed");

        let out = drain_until(&mut pty, Duration::from_secs(5));
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("abXY"),
            "expected edited command/output to contain abXY, got {s:?}",
        );
        assert!(
            !s.contains("abcdXY"),
            "expected backspace to delete characters before execution, got {s:?}",
        );
    }
}
