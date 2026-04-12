//! C ABI for remote service discovery.
//!
//! One entry point: [`pier_services_detect`] — opens a fresh
//! SSH session, runs the four concurrent probes from
//! [`crate::ssh::service_detector`], closes the session, and
//! returns the result as a heap-allocated JSON C string. The
//! caller must release the string via
//! [`pier_services_free_json`].
//!
//! ## Why we don't reuse a long-lived session
//!
//! For M4's first slice, detection runs on its own fresh SSH
//! connection rather than borrowing the one that backs the
//! user's current terminal tab. Reasons:
//!
//!  * No FFI shape for "grab the session out of an existing
//!    PierTerminal handle" exists yet — that lives in M3e.
//!  * Service detection is short-lived (one round-trip per
//!    probe, ~800 ms total on LAN). A fresh SSH handshake
//!    adds 200-300 ms of overhead, tolerable for the first
//!    version.
//!  * The detection code path is untangled from the terminal
//!    lifecycle. Later we can hoist into an opaque
//!    `*mut PierSshSession` and share it with the terminal,
//!    SFTP, and (future) tunnel subsystems — without breaking
//!    this FFI shape.
//!
//! ## Auth kind discriminator
//!
//! Same table as [`super::sftp`]:
//!
//! | `auth_kind`           | value | `secret`         | `extra`                     |
//! |-----------------------|------:|------------------|-----------------------------|
//! | `PIER_AUTH_PASSWORD`  |   0   | plaintext password | NULL                        |
//! | `PIER_AUTH_CREDENTIAL`|   1   | keychain id        | NULL                        |
//! | `PIER_AUTH_KEY`       |   2   | key file path      | passphrase credential id or NULL |
//! | `PIER_AUTH_AGENT`     |   3   | NULL               | NULL                        |

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use crate::ssh::{detect_all_blocking, AuthMethod, HostKeyVerifier, SshConfig, SshSession};

use super::sftp::{PIER_AUTH_AGENT, PIER_AUTH_CREDENTIAL, PIER_AUTH_KEY, PIER_AUTH_PASSWORD};

/// Detect known services (MySQL / Redis / PostgreSQL / Docker)
/// on the host reached by the given connection parameters.
///
/// Returns a heap-allocated NUL-terminated UTF-8 JSON string
/// containing a `Vec<DetectedService>`, or `NULL` on failure.
/// Release via [`pier_services_free_json`].
///
/// Blocking: runs the full SSH handshake + four concurrent
/// exec probes on the calling thread. Typical LAN latency is
/// 0.5-1.5 s. Call from a worker thread; the C++ wrapper
/// posts the result back to the main thread via
/// `QMetaObject::invokeMethod(Qt::QueuedConnection)`.
///
/// # Safety
///
/// `host` and `user` must be valid NUL-terminated UTF-8 C
/// strings. `secret` and `extra` may be NULL per the auth-kind
/// table above.
#[no_mangle]
pub unsafe extern "C" fn pier_services_detect(
    host: *const c_char,
    port: u16,
    user: *const c_char,
    auth_kind: c_int,
    secret: *const c_char,
    extra: *const c_char,
) -> *mut c_char {
    if host.is_null() || user.is_null() {
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
        PIER_AUTH_PASSWORD => AuthMethod::DirectPassword {
            password: secret_str.unwrap_or_default(),
        },
        PIER_AUTH_CREDENTIAL => {
            let Some(id) = secret_str else {
                log::warn!("pier_services_detect: credential auth needs secret=credential_id");
                return ptr::null_mut();
            };
            AuthMethod::KeychainPassword { credential_id: id }
        }
        PIER_AUTH_KEY => {
            let Some(path) = secret_str else {
                log::warn!("pier_services_detect: key auth needs secret=private_key_path");
                return ptr::null_mut();
            };
            AuthMethod::PublicKeyFile {
                private_key_path: path,
                passphrase_credential_id: extra_str,
            }
        }
        PIER_AUTH_AGENT => AuthMethod::Agent,
        _ => {
            log::warn!("pier_services_detect: unknown auth_kind {auth_kind}");
            return ptr::null_mut();
        }
    };

    let mut config = SshConfig::new(&host_str, &host_str, &user_str);
    config.port = port;
    config.auth = auth;

    let session = match SshSession::connect_blocking(&config, HostKeyVerifier::default()) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_services_detect connect failed: {e}");
            return ptr::null_mut();
        }
    };

    let services = detect_all_blocking(&session);
    drop(session); // release the SSH session ASAP

    let json = match serde_json::to_string(&services) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_services_detect serialize failed: {e}");
            return ptr::null_mut();
        }
    };
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// M3e: run remote service detection on an existing shared
/// session instead of dialling fresh. Same JSON result
/// shape as [`pier_services_detect`]; release the returned
/// string with [`pier_services_free_json`].
///
/// # Safety
///
/// `session`, if non-null, must be a live handle produced
/// by [`super::ssh_session::pier_ssh_session_open`].
#[no_mangle]
pub unsafe extern "C" fn pier_services_detect_on_session(
    session: *const super::ssh_session::PierSshSession,
) -> *mut c_char {
    if session.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: live handle.
    let shared = unsafe { &*session };
    let cloned = shared.session();
    let services = detect_all_blocking(&cloned);
    drop(cloned);

    let json = match serde_json::to_string(&services) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_services_detect_on_session serialize failed: {e}");
            return ptr::null_mut();
        }
    };
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Release a JSON string previously returned by
/// [`pier_services_detect`]. Safe to call with NULL.
///
/// # Safety
///
/// `s`, if non-null, must be a pointer previously returned by
/// [`pier_services_detect`] and not yet freed. The allocator
/// mismatches between Rust and libc, so do NOT call `free()`
/// on it.
#[no_mangle]
pub unsafe extern "C" fn pier_services_free_json(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: caller contract — pointer came from CString::into_raw.
    let _ = unsafe { CString::from_raw(s) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_args_return_null() {
        // SAFETY: null is defined to be handled without
        // touching memory.
        unsafe {
            assert!(pier_services_detect(
                ptr::null(),
                22,
                ptr::null(),
                PIER_AUTH_PASSWORD,
                ptr::null(),
                ptr::null(),
            )
            .is_null());
            pier_services_free_json(ptr::null_mut()); // no-op
        }
    }

    #[test]
    fn unknown_auth_kind_returns_null() {
        let host = CString::new("example.com").unwrap();
        let user = CString::new("root").unwrap();
        // SAFETY: all strings non-null; kind 999 is unknown.
        let r = unsafe {
            pier_services_detect(
                host.as_ptr(),
                22,
                user.as_ptr(),
                999,
                ptr::null(),
                ptr::null(),
            )
        };
        assert!(r.is_null());
    }

    #[test]
    fn unreachable_host_fails_fast() {
        // RFC 5737 TEST-NET-1.
        let host = CString::new("192.0.2.1").unwrap();
        let user = CString::new("root").unwrap();
        let pass = CString::new("x").unwrap();
        let start = std::time::Instant::now();
        // SAFETY: valid strings.
        let r = unsafe {
            pier_services_detect(
                host.as_ptr(),
                22,
                user.as_ptr(),
                PIER_AUTH_PASSWORD,
                pass.as_ptr(),
                ptr::null(),
            )
        };
        let elapsed = start.elapsed();
        assert!(r.is_null());
        assert!(
            elapsed < std::time::Duration::from_secs(15),
            "detect should fail fast, took {elapsed:?}",
        );
    }
}
