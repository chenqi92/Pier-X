//! C ABI wrapper around [`crate::terminal::PierTerminal`].
//!
//! ## Handle model
//!
//! The consumer gets an opaque `*mut PierTerminal` from
//! [`pier_terminal_new`]. Every other function in this module takes
//! that same pointer and is a no-op / error if the pointer is null.
//! The only way to release the handle is [`pier_terminal_free`],
//! which joins the reader thread and reaps the child before
//! returning. Double-free is undefined behavior (standard C ABI
//! contract); the C++ wrapper stores the handle inside a
//! `std::unique_ptr` with a custom deleter to enforce this.
//!
//! ## Notify callback
//!
//! [`pier_terminal_new`] takes a function pointer `notify` and an
//! opaque `user_data`. When the reader thread has fresh output or the
//! child has exited, it invokes `notify(user_data, event)` from the
//! reader thread — NOT the UI thread. The implementation must be
//! thread-safe and quick. The canonical body is a single
//! `QMetaObject::invokeMethod(Qt::QueuedConnection, ...)` that wakes
//! a slot on the Qt main thread; the slot then calls
//! [`pier_terminal_snapshot`] on its own terms. Do not call back into
//! this module synchronously from the callback — while the current
//! implementation releases its mutex before invoking the callback,
//! depending on that is fragile.
//!
//! ## Snapshot model
//!
//! Callers allocate a `PierCell` buffer large enough for `cols*rows`
//! and pass it to [`pier_terminal_snapshot`]. The function fills
//! [`PierGridInfo`] with the current dimensions + cursor, and memcpys
//! the grid cells into the buffer. If the caller's buffer is too
//! small the function returns `-2` without touching the buffer. This
//! shape means zero allocation in Rust, zero lifetime dance, and a
//! single cache-friendly copy for rendering.
//!
//! ## Error codes
//!
//! | value | meaning                                      |
//! |------:|----------------------------------------------|
//! |  `0`  | success                                      |
//! | `-1`  | null handle / null out pointer               |
//! | `-2`  | buffer too small                             |
//! | `-3`  | underlying I/O error (write / resize failed) |
//! | `-4`  | platform does not support this backend yet   |

#![allow(clippy::missing_safety_doc)]
// Every function in this module is `unsafe extern "C"` and its
// Safety section is spelled out in the per-function doc comment
// and/or in pier_terminal.h, rather than a clippy-mandated
// boilerplate `# Safety` line.

use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::ptr;

use crate::terminal::{Cell, Color, NotifyFn, PierTerminal, Pty, TerminalError};

// ─────────────────────────────────────────────────────────
// Thread-local last-error channel for the SSH async path.
//
// The C++ wrapper calls `pier_terminal_new_ssh` from a dedicated
// std::thread so it can block without freezing the Qt event loop.
// When that call returns NULL we want to surface a human-readable
// reason (authentication failure, DNS lookup failed, host key
// mismatch, ...) to the UI. Having the FFI function stuff the
// message into a thread-local cell and a separate reader function
// retrieve it is the standard OpenSSL / libcurl pattern: it's
// cheap, doesn't churn the signature, and is safe because the C++
// side always reads the error on the same thread that made the
// call that produced it.
// ─────────────────────────────────────────────────────────

thread_local! {
    /// The most recent error message left by `pier_terminal_new_ssh`
    /// on this thread. Reset to `None` on every successful call.
    static LAST_SSH_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Helper: stash a message into the thread-local error slot.
/// Any interior NUL bytes are replaced with '?' so the CString
/// constructor can never fail and lose the error information.
fn set_last_ssh_error(msg: impl Into<String>) {
    let cleaned: String = msg
        .into()
        .chars()
        .map(|c| if c == '\0' { '?' } else { c })
        .collect();
    let cstring = CString::new(cleaned).expect("nul bytes already scrubbed");
    LAST_SSH_ERROR.with(|slot| {
        *slot.borrow_mut() = Some(cstring);
    });
}

/// Helper: clear the thread-local error slot. Called from the
/// success path so a stale error from a previous attempt can't
/// linger and confuse the caller.
fn clear_last_ssh_error() {
    LAST_SSH_ERROR.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

/// Plain-data cell struct mirroring the Rust [`Cell`] but with a
/// fixed, stable layout suitable for memcpy across the FFI boundary.
///
/// Fields are ordered and sized so the overall struct fits neatly in
/// 16 bytes on every target — padding + packing are deliberate so
/// pier_terminal.h can declare the same layout without uintptr
/// trickery.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PierCell {
    /// Unicode codepoint. Empty / cleared cells use U+0020.
    pub ch: u32,
    /// Foreground color kind: `0` = default, `1` = indexed palette
    /// (value in `fg_r`), `2` = RGB (`fg_r`/`fg_g`/`fg_b`).
    pub fg_kind: u8,
    /// Foreground red channel or palette index (see `fg_kind`).
    pub fg_r: u8,
    /// Foreground green channel (only meaningful when `fg_kind == 2`).
    pub fg_g: u8,
    /// Foreground blue channel (only meaningful when `fg_kind == 2`).
    pub fg_b: u8,
    /// Background color kind, same encoding as `fg_kind`.
    pub bg_kind: u8,
    /// Background red channel or palette index.
    pub bg_r: u8,
    /// Background green channel.
    pub bg_g: u8,
    /// Background blue channel.
    pub bg_b: u8,
    /// Bit 0 = bold, bit 1 = underline, bit 2 = reverse.
    pub attrs: u8,
    /// Padding so the total size is 16 bytes on every target.
    _padding: [u8; 3],
}

impl PierCell {
    const ATTR_BOLD: u8 = 0b0000_0001;
    const ATTR_UNDERLINE: u8 = 0b0000_0010;
    const ATTR_REVERSE: u8 = 0b0000_0100;

    fn from_cell(c: &Cell) -> Self {
        let (fg_kind, fg_r, fg_g, fg_b) = encode_color(c.fg);
        let (bg_kind, bg_r, bg_g, bg_b) = encode_color(c.bg);
        let mut attrs = 0u8;
        if c.bold {
            attrs |= Self::ATTR_BOLD;
        }
        if c.underline {
            attrs |= Self::ATTR_UNDERLINE;
        }
        if c.reverse {
            attrs |= Self::ATTR_REVERSE;
        }
        Self {
            ch: c.ch as u32,
            fg_kind,
            fg_r,
            fg_g,
            fg_b,
            bg_kind,
            bg_r,
            bg_g,
            bg_b,
            attrs,
            _padding: [0; 3],
        }
    }
}

fn encode_color(c: Color) -> (u8, u8, u8, u8) {
    match c {
        Color::Default => (0, 0, 0, 0),
        Color::Indexed(i) => (1, i, 0, 0),
        Color::Rgb(r, g, b) => (2, r, g, b),
    }
}

/// Grid metadata returned from [`pier_terminal_snapshot`].
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PierGridInfo {
    /// Columns at the time of the snapshot.
    pub cols: u16,
    /// Rows at the time of the snapshot.
    pub rows: u16,
    /// Cursor column, zero-based, `< cols`.
    pub cursor_x: u16,
    /// Cursor row, zero-based, `< rows`.
    pub cursor_y: u16,
    /// `1` if the backing process is still running, `0` if it has
    /// exited. Updated before the final `Exited` notify event fires.
    pub alive: u8,
    /// Padding so the overall struct is 16 bytes.
    _padding: [u8; 7],
}

// ─────────────────────────────────────────────────────────
// Functions
// ─────────────────────────────────────────────────────────

/// Spawn a new local terminal session.
///
/// `shell` must be a NUL-terminated UTF-8 path (e.g. `/bin/zsh`).
/// Returns `NULL` on failure — the caller can check
/// [`pier_terminal_last_error`] (TODO, lands with the first protocol
/// module that needs it). On success, the returned pointer must
/// eventually be passed to [`pier_terminal_free`].
///
/// The `notify` callback is invoked from the reader thread on any
/// `DataReady` or `Exited` event. See the module-level documentation
/// for the callback contract. `notify` must not be null; pass a
/// no-op function if you don't care about wakeups.
///
/// # Safety
///
/// `shell` must be a valid NUL-terminated C string. `notify` must be
/// a valid function pointer. `user_data` is opaque and is not
/// dereferenced by pier-core.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_new(
    cols: u16,
    rows: u16,
    shell: *const c_char,
    notify: Option<NotifyFn>,
    user_data: *mut c_void,
) -> *mut PierTerminal {
    if shell.is_null() {
        return ptr::null_mut();
    }
    let Some(notify) = notify else {
        return ptr::null_mut();
    };
    // SAFETY: caller guarantees a NUL-terminated UTF-8 C string.
    let shell_str = match unsafe { std::ffi::CStr::from_ptr(shell) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    match PierTerminal::new(cols, rows, shell_str, notify, user_data) {
        Ok(t) => Box::into_raw(Box::new(t)),
        Err(e) => {
            log::warn!("pier_terminal_new failed: {e}");
            ptr::null_mut()
        }
    }
}

/// Spawn a new terminal session backed by a remote SSH shell
/// instead of a local PTY.
///
/// This constructor exists so that every call site above the
/// opaque `*mut PierTerminal` handle — the C++ `PierTerminalSession`
/// wrapper, the `PierTerminalGrid` renderer, the QML keyboard
/// routing, all of M2b's infrastructure — gets to treat a remote
/// shell as indistinguishable from a local one. The backend is
/// swapped behind the M2 [`crate::terminal::Pty`] trait by handing
/// an [`crate::ssh::SshChannelPty`] into
/// [`PierTerminal::with_pty`] instead of the default `UnixPty`.
///
/// ## Inputs
///
/// * `host`, `user`, `password` — NUL-terminated UTF-8 C strings.
///   The password is used only for the duration of authentication
///   and is not stored anywhere by pier-core after the handshake.
///   Pass an empty password if your server allows it (e.g. key
///   authentication via the ssh agent — but note that variant is
///   not yet wired in M3b; use M3c's full-config constructor).
/// * `port` — TCP port, usually 22.
/// * `cols`, `rows` — initial grid size; the remote PTY is
///   requested at this size via the SSH `pty-req` channel message.
/// * `notify`, `user_data` — same contract as
///   [`pier_terminal_new`]. The notify fn is invoked on the
///   reader thread, not from inside this constructor.
///
/// ## Blocking behavior
///
/// The call is **synchronous and blocking** on the calling thread
/// for the full TCP connect + SSH handshake + authentication +
/// `channel_open_session` + `request_pty` + `request_shell`
/// sequence. On a LAN this is typically under 300 ms; across
/// long-haul links it can be several seconds. Callers that care
/// about UI responsiveness should call this on a worker thread.
/// M3c will introduce an async variant with progress callbacks.
///
/// Returns `NULL` on any failure (config invalid, DNS lookup
/// failed, TCP refused, auth rejected, channel open refused).
/// Details are logged at `log::warn!` level via the `log` crate;
/// M3c exposes a structured `pier_last_error` API.
///
/// # Safety
///
/// Every `*const c_char` must be a valid NUL-terminated UTF-8 C
/// string. `notify` must be a valid function pointer. `user_data`
/// is opaque and not dereferenced by pier-core.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_new_ssh(
    cols: u16,
    rows: u16,
    host: *const c_char,
    port: u16,
    user: *const c_char,
    password: *const c_char,
    notify: Option<NotifyFn>,
    user_data: *mut c_void,
) -> *mut PierTerminal {
    use crate::ssh::AuthMethod;

    if host.is_null() || user.is_null() || password.is_null() {
        set_last_ssh_error("null argument passed to pier_terminal_new_ssh");
        return ptr::null_mut();
    }
    let Some(notify) = notify else {
        set_last_ssh_error("notify callback must not be null");
        return ptr::null_mut();
    };

    // SAFETY: caller contract — all three pointers are valid
    // NUL-terminated UTF-8 C strings.
    let host_str = match unsafe { std::ffi::CStr::from_ptr(host) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_ssh_error("host is not valid UTF-8");
            return ptr::null_mut();
        }
    };
    let user_str = match unsafe { std::ffi::CStr::from_ptr(user) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_ssh_error("user is not valid UTF-8");
            return ptr::null_mut();
        }
    };
    let password_str = match unsafe { std::ffi::CStr::from_ptr(password) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_ssh_error("password is not valid UTF-8");
            return ptr::null_mut();
        }
    };

    if host_str.is_empty() || user_str.is_empty() {
        set_last_ssh_error("host and user must not be empty");
        return ptr::null_mut();
    }

    let auth = AuthMethod::InMemoryPassword {
        password: password_str,
    };
    new_ssh_with_auth(cols, rows, host_str, port, user_str, auth, notify, user_data)
}

/// Spawn a new SSH-backed terminal session whose password lives
/// in the OS keychain rather than crossing the FFI boundary.
///
/// This is the M3c2 entry point: the dialog (or sidebar
/// reconnect path) hands us a `credential_id` that was previously
/// stored via [`pier_credential_set`]. The Rust SSH layer looks
/// up the password from the OS keychain at handshake time via
/// [`crate::credentials::get`] and discards it before returning.
/// **No plaintext password ever crosses this FFI in either
/// direction.**
///
/// Identical handle semantics to [`pier_terminal_new_ssh`]: the
/// returned opaque `*mut PierTerminal` is consumed by the same
/// _write / _resize / _snapshot / _free functions.
///
/// # Safety
///
/// `host`, `user`, `credential_id` must be valid NUL-terminated
/// UTF-8 C strings. `notify` must be a valid function pointer.
/// `user_data` is opaque.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_new_ssh_credential(
    cols: u16,
    rows: u16,
    host: *const c_char,
    port: u16,
    user: *const c_char,
    credential_id: *const c_char,
    notify: Option<NotifyFn>,
    user_data: *mut c_void,
) -> *mut PierTerminal {
    use crate::ssh::AuthMethod;

    if host.is_null() || user.is_null() || credential_id.is_null() {
        set_last_ssh_error("null argument passed to pier_terminal_new_ssh_credential");
        return ptr::null_mut();
    }
    let Some(notify) = notify else {
        set_last_ssh_error("notify callback must not be null");
        return ptr::null_mut();
    };

    // SAFETY: caller contract — all three pointers are valid
    // NUL-terminated UTF-8 C strings.
    let host_str = match unsafe { std::ffi::CStr::from_ptr(host) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_ssh_error("host is not valid UTF-8");
            return ptr::null_mut();
        }
    };
    let user_str = match unsafe { std::ffi::CStr::from_ptr(user) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_ssh_error("user is not valid UTF-8");
            return ptr::null_mut();
        }
    };
    let cred_str = match unsafe { std::ffi::CStr::from_ptr(credential_id) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_ssh_error("credential_id is not valid UTF-8");
            return ptr::null_mut();
        }
    };

    if host_str.is_empty() || user_str.is_empty() || cred_str.is_empty() {
        set_last_ssh_error("host, user and credential_id must not be empty");
        return ptr::null_mut();
    }

    let auth = AuthMethod::KeychainPassword {
        credential_id: cred_str,
    };
    new_ssh_with_auth(cols, rows, host_str, port, user_str, auth, notify, user_data)
}

/// Spawn a new SSH-backed terminal session that authenticates
/// via the system SSH agent.
///
/// On Unix, the agent is located via `$SSH_AUTH_SOCK`. On
/// Windows, Pageant's named pipe is used. No secret ever
/// crosses this FFI — the agent itself holds the private keys
/// and signs challenges without ever handing them to the client.
///
/// Identical handle semantics to [`pier_terminal_new_ssh`].
///
/// # Safety
///
/// `host`, `user` must be valid NUL-terminated UTF-8 C strings.
/// `notify` must be a valid function pointer. `user_data` is
/// opaque.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_new_ssh_agent(
    cols: u16,
    rows: u16,
    host: *const c_char,
    port: u16,
    user: *const c_char,
    notify: Option<NotifyFn>,
    user_data: *mut c_void,
) -> *mut PierTerminal {
    use crate::ssh::AuthMethod;

    if host.is_null() || user.is_null() {
        set_last_ssh_error("null argument passed to pier_terminal_new_ssh_agent");
        return ptr::null_mut();
    }
    let Some(notify) = notify else {
        set_last_ssh_error("notify callback must not be null");
        return ptr::null_mut();
    };

    // SAFETY: caller contract — NUL-terminated UTF-8.
    let host_str = match unsafe { std::ffi::CStr::from_ptr(host) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_ssh_error("host is not valid UTF-8");
            return ptr::null_mut();
        }
    };
    let user_str = match unsafe { std::ffi::CStr::from_ptr(user) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_ssh_error("user is not valid UTF-8");
            return ptr::null_mut();
        }
    };

    if host_str.is_empty() || user_str.is_empty() {
        set_last_ssh_error("host and user must not be empty");
        return ptr::null_mut();
    }

    new_ssh_with_auth(
        cols, rows, host_str, port, user_str,
        AuthMethod::Agent,
        notify, user_data,
    )
}

/// Spawn a new SSH-backed terminal session that authenticates
/// via a private key file rather than a password.
///
/// `private_key_path` is the on-disk location of the OpenSSH-
/// format private key (e.g. `~/.ssh/id_ed25519`).
/// `passphrase_credential_id` may be NULL if the key is
/// unencrypted; otherwise it must reference a previously stored
/// keychain entry containing the passphrase. The Rust SSH layer
/// looks the passphrase up at handshake time via
/// [`crate::credentials::get`] and discards it immediately —
/// **plaintext passphrases never cross this FFI in either
/// direction.**
///
/// Identical handle semantics to [`pier_terminal_new_ssh`]: the
/// returned opaque `*mut PierTerminal` is consumed by the same
/// _write, _resize, _snapshot, _free functions.
///
/// # Safety
///
/// `host`, `user`, `private_key_path` must be valid NUL-terminated
/// UTF-8 C strings. `passphrase_credential_id` must be either
/// null or a valid NUL-terminated UTF-8 C string. `notify` must
/// be a valid function pointer. `user_data` is opaque.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_new_ssh_key(
    cols: u16,
    rows: u16,
    host: *const c_char,
    port: u16,
    user: *const c_char,
    private_key_path: *const c_char,
    passphrase_credential_id: *const c_char,
    notify: Option<NotifyFn>,
    user_data: *mut c_void,
) -> *mut PierTerminal {
    use crate::ssh::AuthMethod;

    if host.is_null() || user.is_null() || private_key_path.is_null() {
        set_last_ssh_error("null argument passed to pier_terminal_new_ssh_key");
        return ptr::null_mut();
    }
    let Some(notify) = notify else {
        set_last_ssh_error("notify callback must not be null");
        return ptr::null_mut();
    };

    // SAFETY: caller contract — pointers are valid NUL-terminated
    // UTF-8 C strings (passphrase_credential_id may be null,
    // checked separately below).
    let host_str = match unsafe { std::ffi::CStr::from_ptr(host) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_ssh_error("host is not valid UTF-8");
            return ptr::null_mut();
        }
    };
    let user_str = match unsafe { std::ffi::CStr::from_ptr(user) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_ssh_error("user is not valid UTF-8");
            return ptr::null_mut();
        }
    };
    let key_path_str = match unsafe { std::ffi::CStr::from_ptr(private_key_path) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_ssh_error("private_key_path is not valid UTF-8");
            return ptr::null_mut();
        }
    };

    if host_str.is_empty() || user_str.is_empty() || key_path_str.is_empty() {
        set_last_ssh_error("host, user and private_key_path must not be empty");
        return ptr::null_mut();
    }

    // Optional passphrase credential id. Null → unencrypted key.
    let passphrase_id_opt: Option<String> = if passphrase_credential_id.is_null() {
        None
    } else {
        match unsafe { std::ffi::CStr::from_ptr(passphrase_credential_id) }.to_str() {
            Ok("") => None,
            Ok(s) => Some(s.to_string()),
            Err(_) => {
                set_last_ssh_error("passphrase_credential_id is not valid UTF-8");
                return ptr::null_mut();
            }
        }
    };

    let auth = AuthMethod::PublicKeyFile {
        private_key_path: key_path_str,
        passphrase_credential_id: passphrase_id_opt,
    };
    new_ssh_with_auth(cols, rows, host_str, port, user_str, auth, notify, user_data)
}

/// Shared body of all three SSH constructors. Builds an `SshConfig`
/// with whichever auth method the caller picked, runs the
/// blocking handshake on the current thread, and either returns
/// the opaque handle or records a typed error in the
/// thread-local last-error slot.
#[allow(clippy::too_many_arguments)]
fn new_ssh_with_auth(
    cols: u16,
    rows: u16,
    host: String,
    port: u16,
    user: String,
    auth: crate::ssh::AuthMethod,
    notify: NotifyFn,
    user_data: *mut c_void,
) -> *mut PierTerminal {
    use crate::ssh::{HostKeyVerifier, SshConfig, SshSession};

    let mut config = SshConfig::new(&host, &host, &user);
    config.port = port;
    config.auth = auth;

    let session = match SshSession::connect_blocking(&config, HostKeyVerifier::default()) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("{e}");
            log::warn!("pier_terminal_new_ssh* connect failed: {msg}");
            set_last_ssh_error(format!("connect failed: {msg}"));
            return ptr::null_mut();
        }
    };

    let ssh_pty = match session.open_shell_channel_blocking(cols, rows) {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("{e}");
            log::warn!("pier_terminal_new_ssh* open_shell_channel failed: {msg}");
            set_last_ssh_error(format!("open shell channel failed: {msg}"));
            return ptr::null_mut();
        }
    };

    let boxed: Box<dyn Pty> = Box::new(ssh_pty);
    match PierTerminal::with_pty(boxed, cols, rows, notify, user_data) {
        Ok(t) => {
            // Clear the slot so a subsequent call on this thread
            // that happens not to stash a new error won't return
            // a stale message from a previous attempt.
            clear_last_ssh_error();
            Box::into_raw(Box::new(t))
        }
        Err(e) => {
            let msg = format!("{e}");
            log::warn!("pier_terminal_new_ssh* with_pty failed: {msg}");
            set_last_ssh_error(format!("with_pty failed: {msg}"));
            ptr::null_mut()
        }
    }
}

/// Return the most recent SSH error message left by
/// [`pier_terminal_new_ssh`] on *this thread*, or `NULL` if the
/// last call on this thread succeeded (or no such call has ever
/// been made on this thread).
///
/// The returned pointer is owned by pier-core's thread-local
/// storage and is valid until the next call to
/// [`pier_terminal_new_ssh`] on the same thread. Callers that
/// need to retain the message should copy it to their own buffer
/// immediately.
///
/// ## Threading
///
/// Because the storage is thread-local, the caller **must** read
/// the error from the same OS thread that made the failing
/// [`pier_terminal_new_ssh`] call. The C++ wrapper pattern —
/// spawn `std::thread`, call `pier_terminal_new_ssh`, read
/// `pier_terminal_last_ssh_error` in the same closure, then
/// `QMetaObject::invokeMethod` to hand the already-copied
/// message to the main thread — satisfies this automatically.
///
/// # Safety
///
/// Always safe to call. Returns a `*const c_char` that is either
/// null or points at a NUL-terminated UTF-8 string owned by
/// thread-local storage. The pointer must not be freed by the
/// caller and must not be retained across another call to
/// [`pier_terminal_new_ssh`] on the same thread.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_last_ssh_error() -> *const c_char {
    LAST_SSH_ERROR.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|cs| cs.as_ptr())
            .unwrap_or(ptr::null())
    })
}

/// Send bytes to the shell (keystrokes, paste, ...).
/// Returns the number of bytes accepted or a negative error code.
///
/// # Safety
///
/// `t` must be either null or a valid handle returned by
/// [`pier_terminal_new`] that has not yet been freed. `data` must
/// point at `len` readable bytes, or be null when `len == 0`.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_write(
    t: *mut PierTerminal,
    data: *const u8,
    len: usize,
) -> i64 {
    if t.is_null() {
        return -1;
    }
    if len > 0 && data.is_null() {
        return -1;
    }
    // SAFETY: non-null handle invariant checked; caller promises
    // the handle is still live.
    let term = unsafe { &*t };
    // SAFETY: non-null + len readable bytes per the contract above.
    let slice = if len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(data, len) }
    };
    match term.write(slice) {
        Ok(n) => n as i64,
        Err(_) => -3,
    }
}

/// Tell the terminal its visible area is now `cols × rows` cells.
///
/// # Safety
///
/// `t` must be either null or a valid, not-yet-freed handle.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_resize(
    t: *mut PierTerminal,
    cols: u16,
    rows: u16,
) -> i32 {
    if t.is_null() {
        return -1;
    }
    // SAFETY: non-null handle invariant checked.
    let term = unsafe { &mut *t };
    match term.resize(cols, rows) {
        Ok(()) => 0,
        Err(TerminalError::Unsupported) => -4,
        Err(_) => -3,
    }
}

/// Copy the current grid into the caller-provided buffer.
///
/// `out_info` receives cols/rows/cursor info + alive flag.
/// `out_cells` must have capacity for at least `out_cells_capacity`
/// [`PierCell`] entries, and must be at least `info.cols * info.rows`
/// in size after the call. If the buffer is too small, returns `-2`
/// and leaves the buffer untouched. `out_info` is still populated in
/// that case so the caller can allocate a larger buffer and retry.
///
/// # Safety
///
/// `t` must be null or a live handle. `out_info` and `out_cells`
/// must be non-null and point at writable memory of at least
/// `sizeof(PierGridInfo)` and `out_cells_capacity * sizeof(PierCell)`
/// bytes respectively.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_snapshot(
    t: *mut PierTerminal,
    out_info: *mut PierGridInfo,
    out_cells: *mut PierCell,
    out_cells_capacity: usize,
) -> i32 {
    if t.is_null() || out_info.is_null() {
        return -1;
    }
    // SAFETY: non-null handle, live by contract.
    let term = unsafe { &*t };
    let snap = term.snapshot();
    let needed = snap.cols as usize * snap.rows as usize;

    let info = PierGridInfo {
        cols: snap.cols,
        rows: snap.rows,
        cursor_x: snap.cursor_x,
        cursor_y: snap.cursor_y,
        alive: if term.is_alive() { 1 } else { 0 },
        _padding: [0; 7],
    };
    // SAFETY: caller contract: out_info is writable.
    unsafe { ptr::write(out_info, info) };

    if out_cells.is_null() {
        // Caller wanted just the metadata to size their buffer.
        return if out_cells_capacity == 0 { 0 } else { -1 };
    }
    if out_cells_capacity < needed {
        return -2;
    }

    // Convert and memcpy. We go cell-by-cell because PierCell is
    // a different layout from Cell. For a typical 120×40 grid this
    // is ~4800 iterations of trivial arithmetic — sub-microsecond.
    for (i, cell) in snap.cells.iter().enumerate() {
        let pc = PierCell::from_cell(cell);
        // SAFETY: i is bounded by needed ≤ out_cells_capacity,
        // and out_cells points at out_cells_capacity writable cells.
        unsafe { ptr::write(out_cells.add(i), pc) };
    }
    0
}

/// Returns `1` if the underlying child process is still running.
///
/// # Safety
///
/// `t` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_is_alive(t: *const PierTerminal) -> i32 {
    if t.is_null() {
        return 0;
    }
    // SAFETY: non-null handle, live by contract.
    let term = unsafe { &*t };
    if term.is_alive() {
        1
    } else {
        0
    }
}

/// Destroy a terminal session. Joins the reader thread and reaps
/// the child before returning. Safe to call with null.
///
/// # Safety
///
/// `t`, if non-null, must have been returned by
/// [`pier_terminal_new`] and not yet freed. After this call the
/// pointer is invalid.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_free(t: *mut PierTerminal) {
    if t.is_null() {
        return;
    }
    // SAFETY: caller contract — the handle was produced by
    // Box::into_raw in pier_terminal_new and has not yet been
    // freed. Box::from_raw reclaims ownership and its Drop runs
    // the full shutdown sequence.
    drop(unsafe { Box::from_raw(t) });
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    extern "C" fn test_notify(user_data: *mut c_void, _event: u32) {
        // SAFETY: tests always pass a leaked AtomicUsize.
        let counter = unsafe { &*(user_data as *const AtomicUsize) };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn wait_for<F: Fn() -> bool>(f: F, limit: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        f()
    }

    #[test]
    fn ffi_roundtrip_spawn_write_snapshot_free() {
        let counter: &'static AtomicUsize = Box::leak(Box::new(AtomicUsize::new(0)));
        let user_data = counter as *const AtomicUsize as *mut c_void;

        let shell = CString::new("/bin/cat").unwrap();
        // SAFETY: shell is a valid NUL-terminated C string,
        // test_notify is a valid function pointer, user_data
        // is the leaked counter.
        let t = unsafe {
            pier_terminal_new(80, 24, shell.as_ptr(), Some(test_notify), user_data)
        };
        assert!(!t.is_null(), "spawn should succeed on Unix");

        let msg = b"ffi-roundtrip\n";
        // SAFETY: t is a live handle, msg is a valid slice.
        let n = unsafe { pier_terminal_write(t, msg.as_ptr(), msg.len()) };
        assert_eq!(n, msg.len() as i64);

        assert!(
            wait_for(|| counter.load(Ordering::Relaxed) > 0, Duration::from_secs(2)),
            "notify callback should have fired after cat echoed our input",
        );

        // Snapshot with a sized buffer.
        let mut info = PierGridInfo {
            cols: 0,
            rows: 0,
            cursor_x: 0,
            cursor_y: 0,
            alive: 0,
            _padding: [0; 7],
        };
        let mut cells = vec![
            PierCell {
                ch: 0,
                fg_kind: 0,
                fg_r: 0,
                fg_g: 0,
                fg_b: 0,
                bg_kind: 0,
                bg_r: 0,
                bg_g: 0,
                bg_b: 0,
                attrs: 0,
                _padding: [0; 3],
            };
            80 * 24
        ];
        // SAFETY: info + cells are both caller-owned writable memory
        // sized for cols*rows = 80*24.
        let rc = unsafe {
            pier_terminal_snapshot(t, &mut info, cells.as_mut_ptr(), cells.len())
        };
        assert_eq!(rc, 0);
        assert_eq!(info.cols, 80);
        assert_eq!(info.rows, 24);
        assert_eq!(info.alive, 1);

        // Walk the grid and look for our needle.
        let mut text = String::new();
        for r in 0..info.rows as usize {
            for c in 0..info.cols as usize {
                let ch = char::from_u32(cells[r * info.cols as usize + c].ch).unwrap_or(' ');
                text.push(ch);
            }
            text.push('\n');
        }
        assert!(
            text.contains("ffi-roundtrip"),
            "grid did not contain echoed text:\n{text}",
        );

        // Clean up.
        // SAFETY: handle is still live and was produced by
        // pier_terminal_new in this test.
        unsafe { pier_terminal_free(t) };
    }

    #[test]
    fn snapshot_rejects_undersized_buffer() {
        let counter: &'static AtomicUsize = Box::leak(Box::new(AtomicUsize::new(0)));
        let user_data = counter as *const AtomicUsize as *mut c_void;
        let shell = CString::new("/bin/cat").unwrap();
        // SAFETY: see above test.
        let t = unsafe {
            pier_terminal_new(80, 24, shell.as_ptr(), Some(test_notify), user_data)
        };
        assert!(!t.is_null());

        let mut info = PierGridInfo {
            cols: 0,
            rows: 0,
            cursor_x: 0,
            cursor_y: 0,
            alive: 0,
            _padding: [0; 7],
        };
        // Deliberately too small.
        let mut cells = vec![
            PierCell {
                ch: 0,
                fg_kind: 0,
                fg_r: 0,
                fg_g: 0,
                fg_b: 0,
                bg_kind: 0,
                bg_r: 0,
                bg_g: 0,
                bg_b: 0,
                attrs: 0,
                _padding: [0; 3],
            };
            10
        ];
        // SAFETY: info + cells are writable.
        let rc = unsafe {
            pier_terminal_snapshot(t, &mut info, cells.as_mut_ptr(), cells.len())
        };
        assert_eq!(rc, -2);
        // info should still be populated so the caller can retry.
        assert_eq!(info.cols, 80);
        assert_eq!(info.rows, 24);

        // SAFETY: still-live handle.
        unsafe { pier_terminal_free(t) };
    }

    #[test]
    fn null_handle_is_safe_everywhere() {
        // SAFETY: all these take null; each function is defined to
        // return an error code without touching memory.
        unsafe {
            assert_eq!(pier_terminal_write(ptr::null_mut(), ptr::null(), 0), -1);
            assert_eq!(pier_terminal_resize(ptr::null_mut(), 80, 24), -1);
            assert_eq!(pier_terminal_is_alive(ptr::null()), 0);
            pier_terminal_free(ptr::null_mut()); // no-op
            let mut info = PierGridInfo {
                cols: 0,
                rows: 0,
                cursor_x: 0,
                cursor_y: 0,
                alive: 0,
                _padding: [0; 7],
            };
            assert_eq!(
                pier_terminal_snapshot(ptr::null_mut(), &mut info, ptr::null_mut(), 0),
                -1,
            );
        }
    }

    // ── pier_terminal_new_ssh ─────────────────────────────

    #[test]
    fn new_ssh_rejects_null_strings() {
        let host = CString::new("example.com").unwrap();
        let user = CString::new("root").unwrap();
        let pass = CString::new("").unwrap();

        // SAFETY: every call has at least one null argument,
        // which the function is defined to reject without
        // touching memory.
        unsafe {
            assert!(
                pier_terminal_new_ssh(
                    80, 24,
                    ptr::null(),
                    22,
                    user.as_ptr(),
                    pass.as_ptr(),
                    Some(test_notify),
                    ptr::null_mut(),
                )
                .is_null(),
                "null host must return NULL",
            );
            assert!(
                pier_terminal_new_ssh(
                    80, 24,
                    host.as_ptr(),
                    22,
                    ptr::null(),
                    pass.as_ptr(),
                    Some(test_notify),
                    ptr::null_mut(),
                )
                .is_null(),
                "null user must return NULL",
            );
            assert!(
                pier_terminal_new_ssh(
                    80, 24,
                    host.as_ptr(),
                    22,
                    user.as_ptr(),
                    ptr::null(),
                    Some(test_notify),
                    ptr::null_mut(),
                )
                .is_null(),
                "null password must return NULL",
            );
            assert!(
                pier_terminal_new_ssh(
                    80, 24,
                    host.as_ptr(),
                    22,
                    user.as_ptr(),
                    pass.as_ptr(),
                    None,
                    ptr::null_mut(),
                )
                .is_null(),
                "null notify fn must return NULL",
            );
        }
    }

    // ── pier_terminal_last_ssh_error ──────────────────────

    /// Helper: read the thread-local error slot as an owned String
    /// (None if null). Safe to call after any FFI call on the same
    /// thread.
    fn read_last_ssh_error() -> Option<String> {
        // SAFETY: returned pointer is owned by pier-core's
        // thread-local storage and is valid until another
        // new_ssh call on this thread. Copying it to a String
        // before that happens is sound.
        let ptr = unsafe { pier_terminal_last_ssh_error() };
        if ptr.is_null() {
            None
        } else {
            Some(
                unsafe { std::ffi::CStr::from_ptr(ptr) }
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }

    #[test]
    fn last_ssh_error_starts_null_and_populates_on_null_input() {
        // A fresh thread must see no error until a failing call
        // lands. We can't rely on test isolation because rustc
        // runs tests on a thread pool — but we CAN force-clear
        // via a successful-shaped call that never reaches the
        // network (null host → immediate reject). The null-host
        // path writes "null argument passed to ..." to the slot.
        // SAFETY: every arg matches the NULL-is-OK contract for
        // the reject path.
        let user = CString::new("root").unwrap();
        let pass = CString::new("x").unwrap();
        let handle = unsafe {
            pier_terminal_new_ssh(
                80, 24,
                ptr::null(),
                22,
                user.as_ptr(),
                pass.as_ptr(),
                Some(test_notify),
                ptr::null_mut(),
            )
        };
        assert!(handle.is_null());

        let msg = read_last_ssh_error().expect("error slot must be populated");
        assert!(
            msg.contains("null argument"),
            "expected 'null argument' in error, got {msg:?}",
        );
    }

    #[test]
    fn last_ssh_error_reports_unreachable_host_reason() {
        // TEST-NET-1 — unroutable — produces a typed error from
        // the Rust session layer which should flow through to
        // the thread-local slot as a useful string.
        let host = CString::new("192.0.2.1").unwrap();
        let user = CString::new("root").unwrap();
        let pass = CString::new("x").unwrap();

        // SAFETY: all strings non-null + NUL-terminated,
        // notify is valid.
        let handle = unsafe {
            pier_terminal_new_ssh(
                80, 24,
                host.as_ptr(),
                22,
                user.as_ptr(),
                pass.as_ptr(),
                Some(test_notify),
                ptr::null_mut(),
            )
        };
        assert!(handle.is_null(), "unreachable host must return NULL");

        let msg = read_last_ssh_error().expect("error slot must be populated");
        assert!(
            msg.contains("connect failed"),
            "expected connect-failed prefix, got {msg:?}",
        );
    }

    // ── pier_terminal_new_ssh_agent ───────────────────────

    #[test]
    fn new_ssh_agent_rejects_null_strings() {
        let host = CString::new("example.com").unwrap();
        let user = CString::new("root").unwrap();

        // SAFETY: each call has at least one null; the function
        // rejects without touching memory.
        unsafe {
            assert!(
                pier_terminal_new_ssh_agent(
                    80, 24,
                    ptr::null(),
                    22,
                    user.as_ptr(),
                    Some(test_notify),
                    ptr::null_mut(),
                )
                .is_null(),
                "null host must return NULL",
            );
            assert!(
                pier_terminal_new_ssh_agent(
                    80, 24,
                    host.as_ptr(),
                    22,
                    ptr::null(),
                    Some(test_notify),
                    ptr::null_mut(),
                )
                .is_null(),
                "null user must return NULL",
            );
            assert!(
                pier_terminal_new_ssh_agent(
                    80, 24,
                    host.as_ptr(),
                    22,
                    user.as_ptr(),
                    None,
                    ptr::null_mut(),
                )
                .is_null(),
                "null notify must return NULL",
            );
        }

        // The null-input error message must contain something
        // recognizable so the C++ side can forward it.
        let msg = read_last_ssh_error().unwrap_or_default();
        assert!(
            msg.contains("null argument") || msg.contains("null"),
            "expected null arg error, got {msg:?}",
        );
    }

    // ── pier_terminal_new_ssh_key ─────────────────────────

    #[test]
    fn new_ssh_key_rejects_null_strings() {
        let host = CString::new("example.com").unwrap();
        let user = CString::new("root").unwrap();
        let key  = CString::new("/tmp/nonexistent.key").unwrap();

        // SAFETY: every call has at least one null string
        // (host / user / key path / notify), all of which the
        // function rejects without touching memory.
        unsafe {
            assert!(
                pier_terminal_new_ssh_key(
                    80, 24,
                    ptr::null(),
                    22,
                    user.as_ptr(),
                    key.as_ptr(),
                    ptr::null(),
                    Some(test_notify),
                    ptr::null_mut(),
                )
                .is_null(),
                "null host must return NULL",
            );
            assert!(
                pier_terminal_new_ssh_key(
                    80, 24,
                    host.as_ptr(),
                    22,
                    ptr::null(),
                    key.as_ptr(),
                    ptr::null(),
                    Some(test_notify),
                    ptr::null_mut(),
                )
                .is_null(),
                "null user must return NULL",
            );
            assert!(
                pier_terminal_new_ssh_key(
                    80, 24,
                    host.as_ptr(),
                    22,
                    user.as_ptr(),
                    ptr::null(),
                    ptr::null(),
                    Some(test_notify),
                    ptr::null_mut(),
                )
                .is_null(),
                "null key path must return NULL",
            );
            assert!(
                pier_terminal_new_ssh_key(
                    80, 24,
                    host.as_ptr(),
                    22,
                    user.as_ptr(),
                    key.as_ptr(),
                    ptr::null(),
                    None,
                    ptr::null_mut(),
                )
                .is_null(),
                "null notify must return NULL",
            );
        }
    }

    #[test]
    fn new_ssh_key_passphrase_id_may_be_null() {
        // The passphrase credential id slot accepts null and
        // empty string interchangeably ("no passphrase"). Both
        // forms must reach the SshSession layer; here we only
        // assert that NEITHER trips the early null-arg
        // validation. The actual connect attempt then fails on
        // the unroutable host below — which is what we want,
        // it proves the validation passed.
        let host = CString::new("192.0.2.1").unwrap();
        let user = CString::new("root").unwrap();
        let key  = CString::new("/tmp/nonexistent-pier-x-key.key").unwrap();
        let empty = CString::new("").unwrap();

        for passphrase_ptr in [ptr::null(), empty.as_ptr()] {
            // SAFETY: all string pointers are valid
            // NUL-terminated strings or null in the case
            // explicitly handled by the function.
            let handle = unsafe {
                pier_terminal_new_ssh_key(
                    80, 24,
                    host.as_ptr(),
                    22,
                    user.as_ptr(),
                    key.as_ptr(),
                    passphrase_ptr,
                    Some(test_notify),
                    ptr::null_mut(),
                )
            };
            // Connect must fail (key file is missing) but it
            // must fail with a real error, not a validation
            // reject. Either way the handle is null.
            assert!(handle.is_null());

            // The error message should mention the key file or
            // the connect, NOT the validation paths.
            let msg = read_last_ssh_error()
                .unwrap_or_else(|| String::from("(none)"));
            assert!(
                !msg.contains("must not be empty")
                    && !msg.contains("not valid UTF-8"),
                "unexpected validation error: {msg:?}",
            );
        }
    }

    #[test]
    fn new_ssh_fails_fast_on_unreachable_host() {
        // RFC 5737 TEST-NET-1 — guaranteed unroutable. Matches
        // the session-layer test, but exercised here through the
        // C ABI surface to prove the blocking connect + error
        // mapping work from the caller's perspective without
        // leaking resources on failure.
        let host = CString::new("192.0.2.1").unwrap();
        let user = CString::new("root").unwrap();
        let pass = CString::new("ignored").unwrap();
        let counter: &'static AtomicUsize = Box::leak(Box::new(AtomicUsize::new(0)));
        let user_data = counter as *const AtomicUsize as *mut c_void;

        let start = std::time::Instant::now();
        // SAFETY: all strings are non-null and NUL-terminated;
        // notify is a valid fn pointer; user_data points at a
        // leaked AtomicUsize that lives for 'static.
        let handle = unsafe {
            pier_terminal_new_ssh(
                80, 24,
                host.as_ptr(),
                22,
                user.as_ptr(),
                pass.as_ptr(),
                Some(test_notify),
                user_data,
            )
        };
        let elapsed = start.elapsed();

        assert!(
            handle.is_null(),
            "unroutable host must return NULL, got handle",
        );
        // Must fail in under ~15s (russh default + slop). If this
        // times out it means error mapping is wrong or the
        // blocking call leaked the runtime.
        assert!(
            elapsed < Duration::from_secs(15),
            "SSH connect should fail fast on unroutable host, took {elapsed:?}",
        );
        // And the callback must NOT have fired — we never got far
        // enough to install the reader thread.
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }
}
