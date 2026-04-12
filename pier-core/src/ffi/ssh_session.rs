//! Shared SSH session handle — the M3e refactor.
//!
//! ## Why this exists
//!
//! Every session-based FFI pier-core shipped through M3-M5
//! (`pier_sftp_new`, `pier_services_detect`, `pier_tunnel_open`,
//! `pier_log_open`, `pier_docker_open`,
//! `pier_terminal_new_ssh*`) opens its own [`SshSession`]
//! internally. On a typical Pier-X workflow that means:
//!
//!   1. SSH terminal tab → session A
//!   2. SFTP browser on the same host → session B
//!   3. Docker panel on the same host → session C
//!   4. Click a MySQL service pill to open a tunnel → session D
//!
//! …four separate TCP connections, four separate authentication
//! round-trips, and four keepalives on the server. Each
//! handshake is ~300 ms on a LAN and can be several seconds on
//! a slow WAN link.
//!
//! M3e adds a shared [`PierSshSession`] opaque handle that
//! any consumer FFI can **borrow** via a new `*_on_session`
//! constructor. The C++ side opens one session per host via
//! [`pier_ssh_session_open`] and hands the handle into every
//! panel that needs it; each panel clones the underlying
//! [`SshSession`] (which is internally a cheap `Arc<Handle>`)
//! and holds onto it for its own lifetime.
//!
//! The existing constructors that take host/port/user/auth
//! continue to work unchanged — the new `_on_session`
//! variants sit alongside them, so the C++ side can migrate
//! panel-by-panel.
//!
//! ## Handle model
//!
//! One opaque `*mut PierSshSession` per live SSH connection.
//! Freeing the handle drops Pier-X's reference to the session,
//! but does **not** guarantee the underlying russh connection
//! closes immediately — any child FFI (SFTP, tunnel, log,
//! docker, terminal) that was built via `_on_session` holds
//! its own clone of the session and keeps the russh `Handle`
//! alive until the last clone drops.
//!
//! This is the exact semantics every C++ panel already
//! expects: closing the "session tab" releases ownership, but
//! the backing SSH connection lingers as long as something is
//! using it.
//!
//! ## Last-error reporting
//!
//! [`pier_ssh_session_open`] returns NULL on failure, same
//! convention as every other pier_* FFI. Structured error
//! details go through a thread-local last-error string, which
//! the C++ side can read via
//! [`pier_ssh_session_last_error`] immediately after a
//! failing open call to surface the underlying
//! [`crate::ssh::SshError`] variant to the user. The pattern
//! mirrors the existing [`super::terminal::pier_terminal_last_ssh_error`]
//! helper and follows the same OpenSSL/libcurl idiom.

#![allow(clippy::missing_safety_doc)]

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use crate::ssh::{AuthMethod, HostKeyVerifier, SshConfig, SshError, SshSession};

use super::sftp::{PIER_AUTH_AGENT, PIER_AUTH_CREDENTIAL, PIER_AUTH_KEY, PIER_AUTH_PASSWORD};

/// Opaque handle to a shared SSH session.
///
/// The handle is the only thing pier-ui-qt ever sees — the
/// wrapped [`SshSession`] is cheap to clone (`Arc<Handle>`
/// under the hood), and every `_on_session` constructor on
/// the consumer FFIs takes one of these pointers and clones
/// out a private copy of the session for its own lifetime.
pub struct PierSshSession {
    session: SshSession,
}

impl PierSshSession {
    /// Internal accessor used by sibling FFI modules to grab
    /// a cheap clone of the wrapped session. `pub(crate)` so
    /// the other `_on_session` constructors can see it but
    /// the C ABI surface doesn't leak the internal shape.
    pub(crate) fn session(&self) -> SshSession {
        self.session.clone()
    }
}

thread_local! {
    /// Thread-local last error, scoped to calls that went
    /// through this FFI module on the current thread. Same
    /// pattern as [`super::terminal::pier_terminal_last_ssh_error`].
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Store a last-error string for retrieval by the C++ side.
/// Empty `msg` clears the slot.
fn set_last_error(msg: &str) {
    LAST_ERROR.with(|slot| {
        if msg.is_empty() {
            *slot.borrow_mut() = None;
        } else {
            // A NUL inside an error message would break the
            // CString invariant; replace defensively.
            let sanitized = msg.replace('\0', "·");
            *slot.borrow_mut() = CString::new(sanitized).ok();
        }
    });
}

/// Structured error category for [`pier_ssh_session_last_error_kind`].
/// Stable integer codes — the C++ side maps these to UI
/// strings. Keep in sync with the header.
const ERR_KIND_OK: i32 = 0;
const ERR_KIND_INVALID_ARG: i32 = 1;
const ERR_KIND_CONNECT: i32 = 2;
const ERR_KIND_AUTH: i32 = 3;
const ERR_KIND_HOST_KEY: i32 = 4;
const ERR_KIND_PROTOCOL: i32 = 5;
const ERR_KIND_UNKNOWN: i32 = 6;

thread_local! {
    static LAST_ERROR_KIND: RefCell<i32> = const { RefCell::new(ERR_KIND_OK) };
}

fn set_last_error_kind(kind: i32) {
    LAST_ERROR_KIND.with(|slot| *slot.borrow_mut() = kind);
}

fn classify(err: &SshError) -> i32 {
    match err {
        SshError::InvalidConfig(_) => ERR_KIND_INVALID_ARG,
        SshError::Connect(_) | SshError::Timeout(_) => ERR_KIND_CONNECT,
        SshError::AuthRejected { .. } => ERR_KIND_AUTH,
        SshError::HostKeyMismatch { .. } => ERR_KIND_HOST_KEY,
        SshError::Protocol(_) => ERR_KIND_PROTOCOL,
        _ => ERR_KIND_UNKNOWN,
    }
}

/// Decode the auth-kind triple (`auth_kind`, `secret`,
/// `extra`) into a structured [`AuthMethod`]. Returns `None`
/// on any parse error, which callers should surface as a
/// NULL handle return.
///
/// # Safety
///
/// `secret` and `extra` must either be NULL or valid
/// NUL-terminated C strings for the duration of the call.
pub(crate) unsafe fn parse_auth_kind(
    auth_kind: c_int,
    secret: *const c_char,
    extra: *const c_char,
) -> Option<AuthMethod> {
    // Decode optional strings.
    let secret_str: Option<String> = if secret.is_null() {
        None
    } else {
        // SAFETY: caller contract.
        match unsafe { CStr::from_ptr(secret) }.to_str() {
            Ok(s) => Some(s.to_string()),
            Err(_) => return None,
        }
    };
    let extra_str: Option<String> = if extra.is_null() {
        None
    } else {
        // SAFETY: caller contract.
        match unsafe { CStr::from_ptr(extra) }.to_str() {
            Ok(s) if !s.is_empty() => Some(s.to_string()),
            Ok(_) => None,
            Err(_) => return None,
        }
    };

    let auth = match auth_kind {
        PIER_AUTH_PASSWORD => AuthMethod::InMemoryPassword {
            password: secret_str.unwrap_or_default(),
        },
        PIER_AUTH_CREDENTIAL => {
            let cred_id = secret_str?;
            AuthMethod::KeychainPassword {
                credential_id: cred_id,
            }
        }
        PIER_AUTH_KEY => {
            let key_path = secret_str?;
            AuthMethod::PublicKeyFile {
                private_key_path: key_path,
                passphrase_credential_id: extra_str,
            }
        }
        PIER_AUTH_AGENT => AuthMethod::Agent,
        _ => return None,
    };
    Some(auth)
}

/// Open a shared SSH session. Performs the full handshake +
/// authentication synchronously on the calling thread and
/// boxes the result.
///
/// Returns `NULL` on any failure. When NULL is returned, the
/// caller may fetch human-readable details via
/// [`pier_ssh_session_last_error`] and a machine-readable
/// category via [`pier_ssh_session_last_error_kind`].
///
/// Auth kind discriminator:
///
/// * `PIER_AUTH_PASSWORD`   (0) — `secret` = plaintext password
/// * `PIER_AUTH_CREDENTIAL` (1) — `secret` = keychain credential id
/// * `PIER_AUTH_KEY`        (2) — `secret` = private key path, `extra` = passphrase credential id or NULL
/// * `PIER_AUTH_AGENT`      (3) — both `secret` and `extra` ignored
///
/// # Safety
///
/// `host` and `user` must be valid NUL-terminated UTF-8.
/// `secret` and `extra` may be NULL per the table above.
#[no_mangle]
pub unsafe extern "C" fn pier_ssh_session_open(
    host: *const c_char,
    port: u16,
    user: *const c_char,
    auth_kind: c_int,
    secret: *const c_char,
    extra: *const c_char,
) -> *mut PierSshSession {
    // Clear any prior error before entering.
    set_last_error("");
    set_last_error_kind(ERR_KIND_OK);

    if host.is_null() || user.is_null() {
        set_last_error("null host or user");
        set_last_error_kind(ERR_KIND_INVALID_ARG);
        return ptr::null_mut();
    }
    // SAFETY: caller contract — NUL-terminated UTF-8.
    let host_str = match unsafe { CStr::from_ptr(host) }.to_str() {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => {
            set_last_error("invalid host string");
            set_last_error_kind(ERR_KIND_INVALID_ARG);
            return ptr::null_mut();
        }
    };
    let user_str = match unsafe { CStr::from_ptr(user) }.to_str() {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => {
            set_last_error("invalid user string");
            set_last_error_kind(ERR_KIND_INVALID_ARG);
            return ptr::null_mut();
        }
    };

    // SAFETY: parse_auth_kind has the same contract.
    let auth = match unsafe { parse_auth_kind(auth_kind, secret, extra) } {
        Some(a) => a,
        None => {
            set_last_error("invalid auth_kind or secret");
            set_last_error_kind(ERR_KIND_INVALID_ARG);
            return ptr::null_mut();
        }
    };

    let mut config = SshConfig::new(&host_str, &host_str, &user_str);
    config.port = port;
    config.auth = auth;

    match SshSession::connect_blocking(&config, HostKeyVerifier::default()) {
        Ok(session) => Box::into_raw(Box::new(PierSshSession { session })),
        Err(e) => {
            log::warn!("pier_ssh_session_open failed: {e}");
            set_last_error(&e.to_string());
            set_last_error_kind(classify(&e));
            ptr::null_mut()
        }
    }
}

/// Returns 1 if the session's internal russh handle has at
/// least one live strong reference, 0 otherwise (or on NULL).
///
/// Note: this is a liveness hint based on refcount, not a
/// ping over the wire. A session whose TCP connection dropped
/// silently will still report alive until the next operation
/// surfaces the break.
///
/// # Safety
///
/// `h`, if non-null, must be a live handle produced by
/// [`pier_ssh_session_open`].
#[no_mangle]
pub unsafe extern "C" fn pier_ssh_session_is_alive(h: *const PierSshSession) -> c_int {
    if h.is_null() {
        return 0;
    }
    // SAFETY: caller contract.
    let handle = unsafe { &*h };
    if handle.session.handle_refcount() > 0 {
        1
    } else {
        0
    }
}

/// Return the number of strong references currently held on
/// the underlying russh handle. Useful for debugging panel
/// sharing — after opening a session and binding three
/// panels to it via `_on_session`, this should report 4 (the
/// owning handle + three child clones).
///
/// # Safety
///
/// `h`, if non-null, must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pier_ssh_session_refcount(h: *const PierSshSession) -> c_int {
    if h.is_null() {
        return 0;
    }
    // SAFETY: caller contract.
    let handle = unsafe { &*h };
    handle.session.handle_refcount() as c_int
}

/// Fetch the last-error message set by a failing
/// [`pier_ssh_session_open`] call on the current thread.
/// Returns a borrowed `const char *` pointing into a
/// thread-local `CString`; the pointer is valid until the
/// next call into `pier_ssh_session_*` on the same thread.
///
/// Returns NULL if no error has been recorded.
#[no_mangle]
pub unsafe extern "C" fn pier_ssh_session_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| match slot.borrow().as_ref() {
        Some(c) => c.as_ptr(),
        None => ptr::null(),
    })
}

/// Fetch the last error category. See the `PIER_SSH_ERR_*`
/// constants in `pier_ssh_session.h` for the meaning of each
/// value.
#[no_mangle]
pub unsafe extern "C" fn pier_ssh_session_last_error_kind() -> c_int {
    LAST_ERROR_KIND.with(|slot| *slot.borrow())
}

/// Release a shared SSH session handle. Safe to call with
/// NULL. After this call the handle is invalid.
///
/// The underlying russh connection is only torn down once
/// the last clone of the session is dropped — any child
/// handle produced via a `*_on_session` constructor keeps
/// its own clone, so freeing this master handle while child
/// panels are still open is safe and simply stops future
/// `*_on_session` calls from borrowing this particular
/// pointer.
///
/// # Safety
///
/// `h`, if non-null, must have been returned by
/// [`pier_ssh_session_open`] and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pier_ssh_session_free(h: *mut PierSshSession) {
    if h.is_null() {
        return;
    }
    // SAFETY: caller contract — box came from into_raw.
    drop(unsafe { Box::from_raw(h) });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_inputs_are_safe() {
        // SAFETY: all null paths documented.
        unsafe {
            assert!(pier_ssh_session_open(
                ptr::null(), 22, ptr::null(),
                PIER_AUTH_PASSWORD, ptr::null(), ptr::null()
            )
            .is_null());
            assert_eq!(pier_ssh_session_is_alive(ptr::null()), 0);
            assert_eq!(pier_ssh_session_refcount(ptr::null()), 0);
            pier_ssh_session_free(ptr::null_mut());
        }
    }

    #[test]
    fn null_host_sets_invalid_arg_error() {
        set_last_error("");
        set_last_error_kind(ERR_KIND_OK);
        let user = CString::new("root").unwrap();
        let pass = CString::new("x").unwrap();
        // SAFETY: user and pass valid.
        let h = unsafe {
            pier_ssh_session_open(
                ptr::null(), 22, user.as_ptr(),
                PIER_AUTH_PASSWORD, pass.as_ptr(), ptr::null()
            )
        };
        assert!(h.is_null());
        // SAFETY: no tear-down needed.
        let kind = unsafe { pier_ssh_session_last_error_kind() };
        assert_eq!(kind, ERR_KIND_INVALID_ARG);
        // SAFETY: last_error is borrowed from thread-local storage.
        let ptr = unsafe { pier_ssh_session_last_error() };
        assert!(!ptr.is_null());
        let msg = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert!(msg.contains("null host"));
    }

    #[test]
    fn unreachable_host_fails_fast_and_records_error() {
        set_last_error("");
        set_last_error_kind(ERR_KIND_OK);
        let host = CString::new("192.0.2.1").unwrap();
        let user = CString::new("root").unwrap();
        let pass = CString::new("x").unwrap();
        let start = std::time::Instant::now();
        // SAFETY: all strings valid.
        let h = unsafe {
            pier_ssh_session_open(
                host.as_ptr(),
                22,
                user.as_ptr(),
                PIER_AUTH_PASSWORD,
                pass.as_ptr(),
                ptr::null(),
            )
        };
        let elapsed = start.elapsed();
        assert!(h.is_null());
        assert!(
            elapsed < std::time::Duration::from_secs(15),
            "open should fail fast on unroutable host, took {elapsed:?}",
        );
        // SAFETY: thread-local read. The exact classification
        // depends on how russh flattens the timeout/io error —
        // it may arrive as Connect (io-wrapped) or Protocol
        // (other russh variant). Either way the slot must be
        // populated and non-OK.
        let kind = unsafe { pier_ssh_session_last_error_kind() };
        assert_ne!(kind, ERR_KIND_OK);
        let ptr = unsafe { pier_ssh_session_last_error() };
        assert!(!ptr.is_null());
    }

    #[test]
    fn unknown_auth_kind_rejected() {
        set_last_error("");
        let host = CString::new("example.com").unwrap();
        let user = CString::new("root").unwrap();
        // SAFETY: strings valid; auth_kind = 99 is bogus.
        let h = unsafe {
            pier_ssh_session_open(
                host.as_ptr(), 22, user.as_ptr(),
                99, ptr::null(), ptr::null()
            )
        };
        assert!(h.is_null());
        // SAFETY: thread-local read.
        assert_eq!(unsafe { pier_ssh_session_last_error_kind() }, ERR_KIND_INVALID_ARG);
    }

    #[test]
    fn set_last_error_replaces_nul_bytes() {
        set_last_error("hello\0world");
        // SAFETY: thread-local read.
        let ptr = unsafe { pier_ssh_session_last_error() };
        assert!(!ptr.is_null());
        let msg = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        // NUL replaced with middle dot — the important thing
        // is that CString::new doesn't panic.
        assert!(!msg.contains('\0'));
        assert!(msg.contains("hello"));
        assert!(msg.contains("world"));
    }

    #[test]
    fn set_last_error_empty_clears_slot() {
        set_last_error("boom");
        // SAFETY: thread-local read.
        assert!(!unsafe { pier_ssh_session_last_error() }.is_null());
        set_last_error("");
        // SAFETY: thread-local read.
        assert!(unsafe { pier_ssh_session_last_error() }.is_null());
    }

    #[test]
    fn classify_covers_every_variant_path() {
        assert_eq!(
            classify(&SshError::InvalidConfig("x".into())),
            ERR_KIND_INVALID_ARG
        );
        assert_eq!(
            classify(&SshError::AuthRejected { tried: vec![] }),
            ERR_KIND_AUTH
        );
        assert_eq!(
            classify(&SshError::HostKeyMismatch {
                host: "h".into(),
                fingerprint: "SHA256:abc".into(),
            }),
            ERR_KIND_HOST_KEY
        );
        assert_eq!(
            classify(&SshError::Timeout(std::time::Duration::from_secs(1))),
            ERR_KIND_CONNECT
        );
        assert_eq!(classify(&SshError::ChannelClosed), ERR_KIND_UNKNOWN);
        assert_eq!(classify(&SshError::Dead), ERR_KIND_UNKNOWN);
    }

    #[test]
    fn consumer_on_session_variants_reject_null() {
        // M3e: every consumer FFI exposes a `_on_session`
        // constructor taking a `*const PierSshSession`. Null
        // in → null out, documented everywhere. Exercise each
        // variant here so the safety contract is covered in
        // one test the commit can point at.
        use super::super::{docker, log_stream, services, sftp, terminal, tunnel};
        unsafe {
            // SAFETY: every call passes a null handle, which
            // each FFI explicitly documents as a no-op
            // returning null.
            assert!(sftp::pier_sftp_new_on_session(ptr::null()).is_null());
            assert!(
                tunnel::pier_tunnel_open_on_session(ptr::null(), 0, ptr::null(), 3306).is_null()
            );
            assert!(services::pier_services_detect_on_session(ptr::null()).is_null());
            assert!(log_stream::pier_log_open_on_session(ptr::null(), ptr::null()).is_null());
            assert!(docker::pier_docker_open_on_session(ptr::null()).is_null());
            assert!(terminal::pier_terminal_new_ssh_on_session(
                ptr::null(),
                80,
                24,
                None,
                ptr::null_mut(),
            )
            .is_null());
        }
    }

    #[test]
    fn error_kind_constants_are_distinct_and_stable() {
        let all = [
            ERR_KIND_OK,
            ERR_KIND_INVALID_ARG,
            ERR_KIND_CONNECT,
            ERR_KIND_AUTH,
            ERR_KIND_HOST_KEY,
            ERR_KIND_PROTOCOL,
            ERR_KIND_UNKNOWN,
        ];
        let mut sorted = all.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len());
    }
}
