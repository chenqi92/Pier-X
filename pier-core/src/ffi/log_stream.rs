//! C ABI for the streaming log viewer (M5b).
//!
//! ## Handle model
//!
//! One opaque `*mut PierLogStream` per running remote command.
//! The handle owns both the [`crate::ssh::SshSession`] that
//! runs the exec and the [`crate::ssh::ExecStream`] that holds
//! the consumer end of the event channel. Dropping the handle
//! stops the remote process (best-effort channel close) and
//! tears down the SSH session.
//!
//! Field order is deliberate: `_stream` drops before
//! `_session` so the producer task sees the stop flag + the
//! channel closing **before** its parent session goes away.
//!
//! ## JSON drain pattern
//!
//! The C++ side polls [`pier_log_drain`] on a Qt timer. Each
//! call returns a heap-allocated UTF-8 JSON string of the
//! shape
//!
//! ```json
//! [{"kind":"stdout","text":"line 1"},
//!  {"kind":"stderr","text":"line 2"},
//!  {"kind":"exit","exit_code":0}]
//! ```
//!
//! …or `NULL` when there are no events to return. `NULL`
//! **does not** mean the stream has ended — `pier_log_is_alive`
//! is the source of truth for that.
//!
//! ## Auth kind discriminator
//!
//! Same table as every other session-based FFI (`pier_sftp.h`,
//! `pier_tunnel.h`, `pier_services.h`).

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use serde::Serialize;

use crate::ssh::{AuthMethod, ExecEvent, ExecStream, HostKeyVerifier, SshConfig, SshSession};

use super::sftp::{PIER_AUTH_AGENT, PIER_AUTH_CREDENTIAL, PIER_AUTH_KEY, PIER_AUTH_PASSWORD};

/// Opaque log-stream handle.
///
/// The session is kept alive alongside the stream because
/// dropping the session would tear down the channel the
/// producer task is currently reading from.
pub struct PierLogStream {
    _stream: ExecStream,
    _session: SshSession,
}

/// Wire-format for one event in the drain payload. Kept
/// private to the FFI crate — UI consumers only ever see JSON.
#[derive(Serialize)]
struct ExecEventDto<'a> {
    kind: &'static str,
    /// Only populated for stdout / stderr.
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
    /// Only populated for exit events.
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    /// Only populated for error events.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

impl<'a> From<&'a ExecEvent> for ExecEventDto<'a> {
    fn from(ev: &'a ExecEvent) -> Self {
        match ev {
            ExecEvent::Stdout(s) => ExecEventDto {
                kind: "stdout",
                text: Some(s.as_str()),
                exit_code: None,
                error: None,
            },
            ExecEvent::Stderr(s) => ExecEventDto {
                kind: "stderr",
                text: Some(s.as_str()),
                exit_code: None,
                error: None,
            },
            ExecEvent::Exit(code) => ExecEventDto {
                kind: "exit",
                text: None,
                exit_code: Some(*code),
                error: None,
            },
            ExecEvent::Error(e) => ExecEventDto {
                kind: "error",
                text: None,
                exit_code: None,
                error: Some(e.as_str()),
            },
        }
    }
}

/// Open an SSH connection, spawn `command` on the remote, and
/// return a handle to the resulting streaming output. Blocking
/// until the handshake + exec request complete.
///
/// Returns `NULL` on any failure (null arg, bad UTF-8, SSH
/// connect failed, auth rejected, exec refused).
///
/// # Safety
///
/// `host` / `user` / `command` must be valid NUL-terminated
/// UTF-8. `secret` and `extra` may be NULL per the auth-kind
/// table.
#[no_mangle]
pub unsafe extern "C" fn pier_log_open(
    host: *const c_char,
    port: u16,
    user: *const c_char,
    auth_kind: c_int,
    secret: *const c_char,
    extra: *const c_char,
    command: *const c_char,
) -> *mut PierLogStream {
    if host.is_null() || user.is_null() || command.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller contract — NUL-terminated UTF-8.
    let host_str = match unsafe { CStr::from_ptr(host) }.to_str() {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => return ptr::null_mut(),
    };
    let user_str = match unsafe { CStr::from_ptr(user) }.to_str() {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => return ptr::null_mut(),
    };
    let command_str = match unsafe { CStr::from_ptr(command) }.to_str() {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => return ptr::null_mut(),
    };

    let secret_str: Option<String> = if secret.is_null() {
        None
    } else {
        match unsafe { CStr::from_ptr(secret) }.to_str() {
            Ok(s) => Some(s.to_string()),
            Err(_) => return ptr::null_mut(),
        }
    };
    let extra_str: Option<String> = if extra.is_null() {
        None
    } else {
        match unsafe { CStr::from_ptr(extra) }.to_str() {
            Ok(s) if !s.is_empty() => Some(s.to_string()),
            Ok(_) => None,
            Err(_) => return ptr::null_mut(),
        }
    };

    let auth = match auth_kind {
        PIER_AUTH_PASSWORD => AuthMethod::InMemoryPassword {
            password: secret_str.unwrap_or_default(),
        },
        PIER_AUTH_CREDENTIAL => {
            let Some(id) = secret_str else {
                return ptr::null_mut();
            };
            AuthMethod::KeychainPassword { credential_id: id }
        }
        PIER_AUTH_KEY => {
            let Some(path) = secret_str else {
                return ptr::null_mut();
            };
            AuthMethod::PublicKeyFile {
                private_key_path: path,
                passphrase_credential_id: extra_str,
            }
        }
        PIER_AUTH_AGENT => AuthMethod::Agent,
        _ => {
            log::warn!("pier_log_open: unknown auth_kind {auth_kind}");
            return ptr::null_mut();
        }
    };

    let mut config = SshConfig::new(&host_str, &host_str, &user_str);
    config.port = port;
    config.auth = auth;

    let session = match SshSession::connect_blocking(&config, HostKeyVerifier::default()) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_log_open connect failed: {e}");
            return ptr::null_mut();
        }
    };
    let stream = match session.spawn_exec_stream_blocking(&command_str) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_log_open exec failed: {e}");
            return ptr::null_mut();
        }
    };

    Box::into_raw(Box::new(PierLogStream {
        _stream: stream,
        _session: session,
    }))
}

/// M3e: spawn a streaming remote command on an existing
/// shared session instead of dialling fresh. Same behaviour
/// as [`pier_log_open`] minus the connect + auth; the
/// session is expected to already be authenticated.
///
/// # Safety
///
/// `session`, if non-null, must be a live handle produced
/// by [`super::ssh_session::pier_ssh_session_open`].
/// `command` must be a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn pier_log_open_on_session(
    session: *const super::ssh_session::PierSshSession,
    command: *const c_char,
) -> *mut PierLogStream {
    if session.is_null() || command.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller contract.
    let command_str = match unsafe { CStr::from_ptr(command) }.to_str() {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => return ptr::null_mut(),
    };
    // SAFETY: live handle.
    let shared = unsafe { &*session };
    let cloned = shared.session();
    let stream = match cloned.spawn_exec_stream_blocking(&command_str) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_log_open_on_session exec failed: {e}");
            return ptr::null_mut();
        }
    };
    Box::into_raw(Box::new(PierLogStream {
        _stream: stream,
        _session: cloned,
    }))
}

/// Drain every currently available event from `h`. Returns
/// a heap-allocated JSON array (release with
/// [`pier_log_free_string`]) or NULL when there are no
/// pending events. NULL is not an error — the caller should
/// check [`pier_log_is_alive`] separately.
///
/// # Safety
///
/// `h`, if non-null, must be a live handle produced by
/// [`pier_log_open`].
#[no_mangle]
pub unsafe extern "C" fn pier_log_drain(h: *mut PierLogStream) -> *mut c_char {
    if h.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: live handle.
    let handle = unsafe { &*h };
    let events = handle._stream.drain();
    if events.is_empty() {
        return ptr::null_mut();
    }
    let dtos: Vec<ExecEventDto<'_>> = events.iter().map(ExecEventDto::from).collect();
    let json = match serde_json::to_string(&dtos) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_log_drain: json serialize failed: {e}");
            return ptr::null_mut();
        }
    };
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Return 1 if the remote process is still running, 0
/// otherwise (channel closed, exit event delivered, or null
/// handle).
///
/// # Safety
///
/// `h`, if non-null, must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pier_log_is_alive(h: *const PierLogStream) -> c_int {
    if h.is_null() {
        return 0;
    }
    // SAFETY: live handle.
    let handle = unsafe { &*h };
    if handle._stream.is_alive() {
        1
    } else {
        0
    }
}

/// Last reported exit code, or -1 if the process hasn't
/// exited (or didn't report one).
///
/// # Safety
///
/// `h`, if non-null, must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pier_log_exit_code(h: *const PierLogStream) -> c_int {
    if h.is_null() {
        return -1;
    }
    // SAFETY: live handle.
    let handle = unsafe { &*h };
    handle._stream.exit_code()
}

/// Flip the stop flag. The producer task notices on its next
/// iteration and closes the channel, which the remote process
/// sees as SIGPIPE. Calling this on an already-stopped handle
/// is a no-op.
///
/// # Safety
///
/// `h`, if non-null, must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pier_log_stop(h: *mut PierLogStream) {
    if h.is_null() {
        return;
    }
    // SAFETY: live handle.
    let handle = unsafe { &*h };
    handle._stream.stop();
}

/// Free a JSON string returned by [`pier_log_drain`]. Safe to
/// call with NULL.
///
/// # Safety
///
/// `s`, if non-null, must have been returned by
/// [`pier_log_drain`] and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pier_log_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: caller contract — produced via CString::into_raw.
    drop(unsafe { CString::from_raw(s) });
}

/// Drop a log-stream handle. Safe to call with NULL. After
/// this call the handle is invalid.
///
/// # Safety
///
/// `h`, if non-null, must have been returned by
/// [`pier_log_open`] and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pier_log_free(h: *mut PierLogStream) {
    if h.is_null() {
        return;
    }
    // SAFETY: caller contract.
    drop(unsafe { Box::from_raw(h) });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_inputs_are_safe() {
        // SAFETY: all nulls documented as safe.
        unsafe {
            assert!(pier_log_open(
                ptr::null(), 22, ptr::null(),
                PIER_AUTH_PASSWORD,
                ptr::null(), ptr::null(), ptr::null()
            )
            .is_null());
            assert!(pier_log_drain(ptr::null_mut()).is_null());
            assert_eq!(pier_log_is_alive(ptr::null()), 0);
            assert_eq!(pier_log_exit_code(ptr::null()), -1);
            pier_log_stop(ptr::null_mut());
            pier_log_free_string(ptr::null_mut());
            pier_log_free(ptr::null_mut());
        }
    }

    #[test]
    fn unreachable_host_fails_fast() {
        let host = CString::new("192.0.2.1").unwrap();
        let user = CString::new("root").unwrap();
        let pass = CString::new("x").unwrap();
        let cmd = CString::new("echo hi").unwrap();
        let start = std::time::Instant::now();
        // SAFETY: valid NUL-terminated strings.
        let h = unsafe {
            pier_log_open(
                host.as_ptr(), 22, user.as_ptr(),
                PIER_AUTH_PASSWORD,
                pass.as_ptr(), ptr::null(),
                cmd.as_ptr(),
            )
        };
        let elapsed = start.elapsed();
        assert!(h.is_null());
        assert!(
            elapsed < std::time::Duration::from_secs(15),
            "log open should fail fast on unroutable host, took {elapsed:?}",
        );
    }

    #[test]
    fn dto_serializes_each_variant() {
        let stdout = ExecEvent::Stdout("hello".into());
        let s = serde_json::to_string(&ExecEventDto::from(&stdout)).unwrap();
        assert!(s.contains("\"kind\":\"stdout\""));
        assert!(s.contains("\"text\":\"hello\""));
        assert!(!s.contains("exit_code"));

        let stderr = ExecEvent::Stderr("oops".into());
        let s = serde_json::to_string(&ExecEventDto::from(&stderr)).unwrap();
        assert!(s.contains("\"kind\":\"stderr\""));
        assert!(s.contains("\"text\":\"oops\""));

        let exit = ExecEvent::Exit(137);
        let s = serde_json::to_string(&ExecEventDto::from(&exit)).unwrap();
        assert!(s.contains("\"kind\":\"exit\""));
        assert!(s.contains("\"exit_code\":137"));
        assert!(!s.contains("\"text\""));

        let err = ExecEvent::Error("bad".into());
        let s = serde_json::to_string(&ExecEventDto::from(&err)).unwrap();
        assert!(s.contains("\"kind\":\"error\""));
        assert!(s.contains("\"error\":\"bad\""));
    }
}
