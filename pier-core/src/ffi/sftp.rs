//! C ABI for the SFTP subsystem.
//!
//! ## Handle model
//!
//! One opaque `*mut PierSftp` per file-browser panel. The C++
//! side calls [`pier_sftp_new`] with an auth-kind discriminator
//! plus whichever secret the kind needs. Internally we build
//! an `SshConfig`, open a fresh `SshSession`, call its
//! `open_sftp()`, and box the result.
//!
//! ## Auth kind discriminator
//!
//! Unlike the 4 terminal-side constructors, the SFTP FFI
//! collapses all four auth methods into one function with a
//! `auth_kind: i32` parameter. The meaning of the `secret` and
//! `extra` C strings depends on the kind:
//!
//! | `auth_kind`           | value | `secret`         | `extra`                     |
//! |-----------------------|------:|------------------|-----------------------------|
//! | `PIER_AUTH_PASSWORD`  |   0   | plaintext password | NULL                        |
//! | `PIER_AUTH_CREDENTIAL`|   1   | keychain id        | NULL                        |
//! | `PIER_AUTH_KEY`       |   2   | key file path      | passphrase credential id or NULL |
//! | `PIER_AUTH_AGENT`     |   3   | NULL               | NULL                        |
//!
//! This is cleaner than duplicating 4 constructors the way
//! terminal does. M3e will retrofit terminal to the same
//! shape once the SFTP path has proven it in practice.
//!
//! ## Ownership for listings and strings
//!
//! [`pier_sftp_list_dir`] returns a heap-allocated JSON
//! C-string via `CString::into_raw`. The caller must free it
//! via [`pier_sftp_free_string`] — not C `free`, because the
//! allocator differs between Rust and libc.
//!
//! Same pattern for [`pier_sftp_canonicalize`].
//!
//! ## Error codes
//!
//! | value | meaning                                        |
//! |------:|------------------------------------------------|
//! |  `0`  | success                                        |
//! | `-1`  | null handle / null required string             |
//! | `-2`  | non-UTF-8 input                                |
//! | `-3`  | I/O / protocol error — details in log::warn!  |
//! | `-4`  | unknown auth_kind discriminator                |

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use crate::ssh::{AuthMethod, HostKeyVerifier, SftpClient, SshConfig, SshSession};

/// Password auth: `secret` is the plaintext password. Used
/// mostly by tests and ad-hoc one-off connects.
pub const PIER_AUTH_PASSWORD: i32 = 0;
/// Keychain password auth: `secret` is an opaque credential
/// id previously stored via `pier_credential_set`.
pub const PIER_AUTH_CREDENTIAL: i32 = 1;
/// Private-key auth: `secret` is the absolute path to an
/// OpenSSH private key file. `extra` is an optional keychain
/// id holding the passphrase; NULL means unencrypted key.
pub const PIER_AUTH_KEY: i32 = 2;
/// OS SSH-agent auth: both `secret` and `extra` are ignored.
pub const PIER_AUTH_AGENT: i32 = 3;

/// Opaque handle. From the C side this is just a pointer.
/// Internally it bundles the SshSession plus the SftpClient
/// so dropping the handle drops both in the right order.
pub struct PierSftp {
    // Session kept alive so the channel stays open.
    _session: SshSession,
    client: SftpClient,
}

/// Spawn an SFTP session against the given host, authenticating
/// with whichever method the `auth_kind` discriminator selects.
///
/// Blocking on the calling thread — typical LAN handshake is
/// under 300 ms. Callers should invoke from a worker thread.
///
/// Returns `NULL` on failure. Specific error details are
/// surfaced via `log::warn!`; a structured last-error API
/// (parallel to `pier_terminal_last_ssh_error`) lands with M3e
/// when SFTP error handling needs structured UI reporting.
///
/// # Safety
///
/// `host` and `user` must be valid NUL-terminated UTF-8.
/// `secret` and `extra` may be NULL per the table in this
/// module's doc comment. `auth_kind` must match one of the
/// documented values.
#[no_mangle]
pub unsafe extern "C" fn pier_sftp_new(
    host: *const c_char,
    port: u16,
    user: *const c_char,
    auth_kind: c_int,
    secret: *const c_char,
    extra: *const c_char,
) -> *mut PierSftp {
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

    // Decode optional strings.
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

    // Build the AuthMethod.
    let auth = match auth_kind {
        PIER_AUTH_PASSWORD => AuthMethod::DirectPassword {
            password: secret_str.unwrap_or_default(),
        },
        PIER_AUTH_CREDENTIAL => {
            let Some(cred_id) = secret_str else {
                log::warn!("pier_sftp_new: credential auth requires secret=credential_id");
                return ptr::null_mut();
            };
            AuthMethod::KeychainPassword {
                credential_id: cred_id,
            }
        }
        PIER_AUTH_KEY => {
            let Some(key_path) = secret_str else {
                log::warn!("pier_sftp_new: key auth requires secret=private_key_path");
                return ptr::null_mut();
            };
            AuthMethod::PublicKeyFile {
                private_key_path: key_path,
                passphrase_credential_id: extra_str,
            }
        }
        PIER_AUTH_AGENT => AuthMethod::Agent,
        _ => {
            log::warn!("pier_sftp_new: unknown auth_kind {auth_kind}");
            return ptr::null_mut();
        }
    };

    let mut config = SshConfig::new(&host_str, &host_str, &user_str);
    config.port = port;
    config.auth = auth;

    // Blocking connect + authenticate.
    let session = match SshSession::connect_blocking(&config, HostKeyVerifier::default()) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_sftp_new connect failed: {e}");
            return ptr::null_mut();
        }
    };

    let client = match session.open_sftp_blocking() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("pier_sftp_new open_sftp failed: {e}");
            return ptr::null_mut();
        }
    };

    Box::into_raw(Box::new(PierSftp {
        _session: session,
        client,
    }))
}

/// M3e: open a new SFTP channel on an existing shared
/// session handle instead of building a brand-new session
/// from auth params. The caller must have already obtained a
/// `*mut PierSshSession` via
/// [`super::ssh_session::pier_ssh_session_open`]; we clone
/// its internal [`SshSession`] (cheap Arc bump), open a
/// fresh SFTP channel on top, and box the pair.
///
/// Dropping the returned handle drops the private clone of
/// the session — the original `PierSshSession` handle is
/// unaffected and the underlying russh connection stays
/// alive as long as any clone remains.
///
/// Returns NULL if `session` is null or if the SFTP channel
/// open fails (e.g. the SFTP subsystem isn't enabled on the
/// remote).
///
/// # Safety
///
/// `session`, if non-null, must be a live handle produced by
/// [`super::ssh_session::pier_ssh_session_open`].
#[no_mangle]
pub unsafe extern "C" fn pier_sftp_new_on_session(
    session: *const super::ssh_session::PierSshSession,
) -> *mut PierSftp {
    if session.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller contract — live handle.
    let shared = unsafe { &*session };
    let cloned = shared.session();
    let client = match cloned.open_sftp_blocking() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("pier_sftp_new_on_session open_sftp failed: {e}");
            return ptr::null_mut();
        }
    };
    Box::into_raw(Box::new(PierSftp {
        _session: cloned,
        client,
    }))
}

/// Release a Rust-owned heap C-string returned by
/// `pier_sftp_list_dir` or `pier_sftp_canonicalize`.
/// Safe to call with null.
///
/// # Safety
///
/// `s`, if non-null, must be a pointer previously returned by
/// one of the listed functions and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pier_sftp_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: caller contract — pointer came from CString::into_raw.
    let _ = unsafe { CString::from_raw(s) };
}

/// Release an SFTP handle. Closes the underlying SSH session
/// and SFTP channel.
///
/// # Safety
///
/// `t`, if non-null, must have been produced by
/// [`pier_sftp_new`] and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pier_sftp_free(t: *mut PierSftp) {
    if t.is_null() {
        return;
    }
    // SAFETY: caller contract.
    drop(unsafe { Box::from_raw(t) });
}

/// List the contents of `path` and return a JSON-serialized
/// `Vec<RemoteFileEntry>` as a heap C-string. The caller must
/// release the string via [`pier_sftp_free_string`].
///
/// Returns NULL on any error.
///
/// # Safety
///
/// `t` must be a live handle from [`pier_sftp_new`]. `path`
/// must be a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn pier_sftp_list_dir(
    t: *mut PierSftp,
    path: *const c_char,
) -> *mut c_char {
    if t.is_null() || path.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: live handle by contract.
    let sftp = unsafe { &*t };
    // SAFETY: NUL-terminated UTF-8 by contract.
    let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    let entries = match sftp.client.list_dir_blocking(path_str) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("pier_sftp_list_dir({path_str}) failed: {e}");
            return ptr::null_mut();
        }
    };
    let json = match serde_json::to_string(&entries) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_sftp_list_dir serialize failed: {e}");
            return ptr::null_mut();
        }
    };
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Canonicalize `path` on the remote (resolves relative paths
/// and symlinks). The "pwd" button on the browser panel uses
/// this to resolve `.` to the user's home directory at open.
///
/// Returns NULL on any error. Release with
/// [`pier_sftp_free_string`].
///
/// # Safety
///
/// `t` must be a live handle. `path` must be a valid
/// NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn pier_sftp_canonicalize(
    t: *mut PierSftp,
    path: *const c_char,
) -> *mut c_char {
    if t.is_null() || path.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: live handle.
    let sftp = unsafe { &*t };
    let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    match sftp.client.canonicalize_blocking(path_str) {
        Ok(resolved) => CString::new(resolved).map(|c| c.into_raw()).unwrap_or(ptr::null_mut()),
        Err(e) => {
            log::warn!("pier_sftp_canonicalize({path_str}) failed: {e}");
            ptr::null_mut()
        }
    }
}

/// Create a directory at `path`. Returns 0 on success, -1 on
/// null args, -3 on I/O error.
#[no_mangle]
pub unsafe extern "C" fn pier_sftp_mkdir(t: *mut PierSftp, path: *const c_char) -> i32 {
    if t.is_null() || path.is_null() {
        return -1;
    }
    let sftp = unsafe { &*t };
    let Ok(p) = (unsafe { CStr::from_ptr(path) }).to_str() else {
        return -2;
    };
    match sftp.client.create_dir_blocking(p) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("pier_sftp_mkdir({p}) failed: {e}");
            -3
        }
    }
}

/// Remove a regular file.
#[no_mangle]
pub unsafe extern "C" fn pier_sftp_remove_file(t: *mut PierSftp, path: *const c_char) -> i32 {
    if t.is_null() || path.is_null() {
        return -1;
    }
    let sftp = unsafe { &*t };
    let Ok(p) = (unsafe { CStr::from_ptr(path) }).to_str() else {
        return -2;
    };
    match sftp.client.remove_file_blocking(p) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("pier_sftp_remove_file({p}) failed: {e}");
            -3
        }
    }
}

/// Remove an empty directory.
#[no_mangle]
pub unsafe extern "C" fn pier_sftp_remove_dir(t: *mut PierSftp, path: *const c_char) -> i32 {
    if t.is_null() || path.is_null() {
        return -1;
    }
    let sftp = unsafe { &*t };
    let Ok(p) = (unsafe { CStr::from_ptr(path) }).to_str() else {
        return -2;
    };
    match sftp.client.remove_dir_blocking(p) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("pier_sftp_remove_dir({p}) failed: {e}");
            -3
        }
    }
}

/// Rename (or move) `from` to `to`.
#[no_mangle]
pub unsafe extern "C" fn pier_sftp_rename(
    t: *mut PierSftp,
    from: *const c_char,
    to: *const c_char,
) -> i32 {
    if t.is_null() || from.is_null() || to.is_null() {
        return -1;
    }
    let sftp = unsafe { &*t };
    let Ok(f) = (unsafe { CStr::from_ptr(from) }).to_str() else {
        return -2;
    };
    let Ok(to_str) = (unsafe { CStr::from_ptr(to) }).to_str() else {
        return -2;
    };
    match sftp.client.rename_blocking(f, to_str) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("pier_sftp_rename({f} -> {to_str}) failed: {e}");
            -3
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_handle_everywhere_is_safe() {
        // SAFETY: every call passes null at minimum; the
        // function is defined to reject without touching
        // memory.
        unsafe {
            assert!(pier_sftp_new(
                ptr::null(), 22, ptr::null(),
                PIER_AUTH_PASSWORD, ptr::null(), ptr::null(),
            )
            .is_null());
            assert!(pier_sftp_list_dir(ptr::null_mut(), ptr::null()).is_null());
            assert!(pier_sftp_canonicalize(ptr::null_mut(), ptr::null()).is_null());
            assert_eq!(pier_sftp_mkdir(ptr::null_mut(), ptr::null()), -1);
            assert_eq!(pier_sftp_remove_file(ptr::null_mut(), ptr::null()), -1);
            assert_eq!(pier_sftp_remove_dir(ptr::null_mut(), ptr::null()), -1);
            assert_eq!(
                pier_sftp_rename(ptr::null_mut(), ptr::null(), ptr::null()),
                -1,
            );
            pier_sftp_free(ptr::null_mut()); // no-op
            pier_sftp_free_string(ptr::null_mut()); // no-op
        }
    }

    #[test]
    fn unknown_auth_kind_returns_null() {
        let host = CString::new("example.com").unwrap();
        let user = CString::new("root").unwrap();
        // SAFETY: all strings non-null; auth_kind 999 is
        // explicitly unknown.
        let h = unsafe {
            pier_sftp_new(host.as_ptr(), 22, user.as_ptr(), 999, ptr::null(), ptr::null())
        };
        assert!(h.is_null());
    }

    #[test]
    fn unreachable_host_fails_fast() {
        // RFC 5737 TEST-NET-1.
        let host = CString::new("192.0.2.1").unwrap();
        let user = CString::new("root").unwrap();
        let pass = CString::new("x").unwrap();
        let start = std::time::Instant::now();
        // SAFETY: valid strings, password auth.
        let h = unsafe {
            pier_sftp_new(
                host.as_ptr(),
                22,
                user.as_ptr(),
                PIER_AUTH_PASSWORD,
                pass.as_ptr(),
                ptr::null(),
            )
        };
        let elapsed = start.elapsed();
        assert!(h.is_null(), "unroutable must return NULL");
        assert!(
            elapsed < std::time::Duration::from_secs(15),
            "sftp new should fail fast, took {elapsed:?}",
        );
    }
}
