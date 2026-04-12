//! C ABI for SSH local port forwarding.
//!
//! ## Handle model
//!
//! One opaque `*mut PierTunnel` per open forward. The handle
//! bundles an `SshSession` + the [`crate::ssh::Tunnel`] that
//! owns the accept loop; dropping the handle closes the
//! listener and tears down the session in the right order.
//!
//! ## Why a fresh session per tunnel (for now)
//!
//! Same reason as [`super::services`]: M4b is the last piece
//! that would benefit from a shared `*mut PierSshSession`
//! handle. Introducing it means refactoring the terminal FFI,
//! which is M3e work. Until then, every tunnel opens its own
//! handshake — a ~300 ms one-time cost per tunnel that's
//! acceptable for the first slice but will disappear once
//! the session-handle refactor lands.
//!
//! ## Auth kind discriminator
//!
//! Same table as [`super::sftp`] and [`super::services`].

#![allow(clippy::missing_safety_doc)]

use std::ffi::CStr;
use std::os::raw::{c_char, c_int};
use std::ptr;

use crate::ssh::{AuthMethod, HostKeyVerifier, SshConfig, SshSession, Tunnel};

use super::sftp::{PIER_AUTH_AGENT, PIER_AUTH_CREDENTIAL, PIER_AUTH_KEY, PIER_AUTH_PASSWORD};

/// Opaque tunnel handle. Drops the underlying `Tunnel` (which
/// stops the accept loop and releases the local port) and the
/// `SshSession` (which closes the SSH connection) when freed.
///
/// Field order is deliberate: `_tunnel` drops before `_session`
/// because the tunnel's accept loop holds a clone of the
/// session's underlying russh handle, and we want the loop
/// to exit cleanly before the session tears down.
pub struct PierTunnel {
    _tunnel: Tunnel,
    _session: SshSession,
    actual_local_port: u16,
}

/// Open a new local port forward. Returns `NULL` on failure.
///
/// * `local_port = 0` lets the OS pick a free port; the
///   actual bound port is available via [`pier_tunnel_local_port`].
/// * `remote_host` is the host the SSH server should connect to
///   (usually `127.0.0.1` — meaning "from the server's point of
///   view, the local MySQL socket on localhost").
/// * `auth_kind` / `secret` / `extra` match the table in
///   [`super::sftp`].
///
/// Blocking — runs the full SSH handshake on the calling
/// thread, then the accept loop spawns onto the shared
/// runtime. Call from a worker thread; typical LAN handshake
/// is ~300 ms.
///
/// # Safety
///
/// `host` / `user` / `remote_host` must be valid NUL-terminated
/// UTF-8. `secret` and `extra` may be NULL per the auth-kind
/// table.
#[no_mangle]
pub unsafe extern "C" fn pier_tunnel_open(
    host: *const c_char,
    port: u16,
    user: *const c_char,
    auth_kind: c_int,
    secret: *const c_char,
    extra: *const c_char,
    local_port: u16,
    remote_host: *const c_char,
    remote_port: u16,
) -> *mut PierTunnel {
    if host.is_null() || user.is_null() || remote_host.is_null() {
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
    let remote_host_str = match unsafe { CStr::from_ptr(remote_host) }.to_str() {
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
            let Some(id) = secret_str else { return ptr::null_mut() };
            AuthMethod::KeychainPassword { credential_id: id }
        }
        PIER_AUTH_KEY => {
            let Some(path) = secret_str else { return ptr::null_mut() };
            AuthMethod::PublicKeyFile {
                private_key_path: path,
                passphrase_credential_id: extra_str,
            }
        }
        PIER_AUTH_AGENT => AuthMethod::Agent,
        _ => {
            log::warn!("pier_tunnel_open: unknown auth_kind {auth_kind}");
            return ptr::null_mut();
        }
    };

    let mut config = SshConfig::new(&host_str, &host_str, &user_str);
    config.port = port;
    config.auth = auth;

    let session = match SshSession::connect_blocking(&config, HostKeyVerifier::default()) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_tunnel_open connect failed: {e}");
            return ptr::null_mut();
        }
    };

    let tunnel = match session.open_local_forward_blocking(local_port, &remote_host_str, remote_port) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("pier_tunnel_open open_local_forward failed: {e}");
            return ptr::null_mut();
        }
    };
    let actual_local_port = tunnel.local_port();

    Box::into_raw(Box::new(PierTunnel {
        _tunnel: tunnel,
        _session: session,
        actual_local_port,
    }))
}

/// M3e: open a new local port forward on an existing shared
/// session instead of dialling fresh. Same semantics as
/// [`pier_tunnel_open`] minus the auth parameters — the
/// session is pre-authenticated. Returns NULL if `session`
/// is null or the forward fails to bind.
///
/// # Safety
///
/// `session`, if non-null, must be a live handle produced by
/// [`super::ssh_session::pier_ssh_session_open`].
/// `remote_host` must be a valid NUL-terminated UTF-8 C
/// string.
#[no_mangle]
pub unsafe extern "C" fn pier_tunnel_open_on_session(
    session: *const super::ssh_session::PierSshSession,
    local_port: u16,
    remote_host: *const c_char,
    remote_port: u16,
) -> *mut PierTunnel {
    if session.is_null() || remote_host.is_null() || remote_port == 0 {
        return ptr::null_mut();
    }
    // SAFETY: caller contract.
    let remote_host_str = match unsafe { CStr::from_ptr(remote_host) }.to_str() {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => return ptr::null_mut(),
    };
    // SAFETY: live handle.
    let shared = unsafe { &*session };
    let cloned = shared.session();
    let tunnel = match cloned.open_local_forward_blocking(local_port, &remote_host_str, remote_port)
    {
        Ok(t) => t,
        Err(e) => {
            log::warn!("pier_tunnel_open_on_session open_local_forward failed: {e}");
            return ptr::null_mut();
        }
    };
    let actual_local_port = tunnel.local_port();
    Box::into_raw(Box::new(PierTunnel {
        _tunnel: tunnel,
        _session: cloned,
        actual_local_port,
    }))
}

/// Return the port the tunnel is actually listening on. This
/// is the same value the caller passed to [`pier_tunnel_open`]
/// unless they passed `0` and let the OS pick, in which case
/// it's the OS-chosen port.
///
/// Returns 0 on null handle.
///
/// # Safety
///
/// `t`, if non-null, must be a live handle produced by
/// [`pier_tunnel_open`].
#[no_mangle]
pub unsafe extern "C" fn pier_tunnel_local_port(t: *const PierTunnel) -> u16 {
    if t.is_null() {
        return 0;
    }
    // SAFETY: live handle.
    let tunnel = unsafe { &*t };
    tunnel.actual_local_port
}

/// Return 1 if the tunnel's accept loop is still running,
/// 0 otherwise (closed handle or errored loop).
///
/// # Safety
///
/// `t`, if non-null, must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pier_tunnel_is_alive(t: *const PierTunnel) -> c_int {
    if t.is_null() {
        return 0;
    }
    // SAFETY: live handle.
    let tunnel = unsafe { &*t };
    if tunnel._tunnel.is_alive() {
        1
    } else {
        0
    }
}

/// Close the tunnel and release its resources. Safe to call
/// with NULL. After this call the handle is invalid.
///
/// # Safety
///
/// `t`, if non-null, must have been returned by
/// [`pier_tunnel_open`] and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pier_tunnel_free(t: *mut PierTunnel) {
    if t.is_null() {
        return;
    }
    // SAFETY: caller contract — box originally from into_raw.
    drop(unsafe { Box::from_raw(t) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn null_handle_safe_everywhere() {
        // SAFETY: null handling is documented per function.
        unsafe {
            assert!(pier_tunnel_open(
                ptr::null(), 22, ptr::null(),
                PIER_AUTH_PASSWORD, ptr::null(), ptr::null(),
                13306, ptr::null(), 3306,
            )
            .is_null());
            assert_eq!(pier_tunnel_local_port(ptr::null()), 0);
            assert_eq!(pier_tunnel_is_alive(ptr::null()), 0);
            pier_tunnel_free(ptr::null_mut()); // no-op
        }
    }

    #[test]
    fn unknown_auth_kind_returns_null() {
        let host = CString::new("example.com").unwrap();
        let user = CString::new("root").unwrap();
        let rhost = CString::new("127.0.0.1").unwrap();
        // SAFETY: strings are all valid NUL-terminated C strings.
        let h = unsafe {
            pier_tunnel_open(
                host.as_ptr(), 22, user.as_ptr(),
                999,
                ptr::null(), ptr::null(),
                13306, rhost.as_ptr(), 3306,
            )
        };
        assert!(h.is_null());
    }

    #[test]
    fn unreachable_host_fails_fast() {
        // RFC 5737 TEST-NET-1.
        let host = CString::new("192.0.2.1").unwrap();
        let user = CString::new("root").unwrap();
        let pass = CString::new("x").unwrap();
        let rhost = CString::new("127.0.0.1").unwrap();
        let start = std::time::Instant::now();
        // SAFETY: all valid strings.
        let h = unsafe {
            pier_tunnel_open(
                host.as_ptr(), 22, user.as_ptr(),
                PIER_AUTH_PASSWORD,
                pass.as_ptr(), ptr::null(),
                13306, rhost.as_ptr(), 3306,
            )
        };
        let elapsed = start.elapsed();
        assert!(h.is_null());
        assert!(
            elapsed < std::time::Duration::from_secs(15),
            "tunnel open should fail fast on unroutable host, took {elapsed:?}",
        );
    }
}
