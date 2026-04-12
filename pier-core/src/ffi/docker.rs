//! C ABI for the Docker panel (M5c).
//!
//! ## Handle model
//!
//! One opaque `*mut PierDocker` per panel. The handle owns
//! a fresh [`crate::ssh::SshSession`] and is used for every
//! subsequent docker operation (list, start/stop/restart/rm).
//!
//! Field order is deliberate: nothing else holds a reference
//! to the session, so dropping the handle closes it cleanly.
//!
//! ## Why not a shared session?
//!
//! Same reason as every other M3-M5 FFI: the shared-handle
//! refactor lives in M3e and we're not there yet. Opening a
//! fresh SSH session per panel costs ~300 ms on handshake
//! but keeps the FFI shape simple. The panel itself is
//! long-lived once opened, so that cost is paid once per
//! docker tab.
//!
//! ## Shell safety
//!
//! Every id that crosses this boundary has to pass
//! [`crate::services::docker::is_safe_id`] before being
//! interpolated into a remote shell command. That's enforced
//! inside the service layer, not here — we just let the
//! error bubble up as a non-zero return code.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use crate::services::docker;
use crate::ssh::{AuthMethod, HostKeyVerifier, SshConfig, SshSession};

use super::sftp::{PIER_AUTH_AGENT, PIER_AUTH_CREDENTIAL, PIER_AUTH_KEY, PIER_AUTH_PASSWORD};

/// Opaque Docker panel handle.
pub struct PierDocker {
    session: SshSession,
}

/// Error codes returned by the action functions. `0` is
/// success; negative values match the convention used by the
/// SFTP FFI in [`super::sftp`]:
///
///   * `-1` — null handle or null required argument
///   * `-2` — non-UTF-8 input
///   * `-3` — action failed (docker non-zero or SSH error)
///   * `-4` — unsafe / rejected id
pub const PIER_DOCKER_OK: c_int = 0;
/// Null pointer / required argument missing.
pub const PIER_DOCKER_ERR_NULL: c_int = -1;
/// Non-UTF-8 input.
pub const PIER_DOCKER_ERR_UTF8: c_int = -2;
/// Docker command returned non-zero, or SSH transport failed.
pub const PIER_DOCKER_ERR_FAILED: c_int = -3;
/// Container id failed [`docker::is_safe_id`].
pub const PIER_DOCKER_ERR_UNSAFE_ID: c_int = -4;

/// Open a Docker panel. Runs the SSH handshake synchronously
/// and returns a handle that survives every subsequent
/// list/start/stop/rm call.
///
/// Auth kind / secret / extra follow the same table as
/// [`super::sftp`] and friends.
///
/// # Safety
///
/// `host` and `user` must be valid NUL-terminated UTF-8.
/// `secret` / `extra` may be NULL per the auth-kind table.
#[no_mangle]
pub unsafe extern "C" fn pier_docker_open(
    host: *const c_char,
    port: u16,
    user: *const c_char,
    auth_kind: c_int,
    secret: *const c_char,
    extra: *const c_char,
) -> *mut PierDocker {
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
            log::warn!("pier_docker_open: unknown auth_kind {auth_kind}");
            return ptr::null_mut();
        }
    };

    let mut config = SshConfig::new(&host_str, &host_str, &user_str);
    config.port = port;
    config.auth = auth;

    let session = match SshSession::connect_blocking(&config, HostKeyVerifier::default()) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_docker_open connect failed: {e}");
            return ptr::null_mut();
        }
    };
    Box::into_raw(Box::new(PierDocker { session }))
}

/// M3e: open a Docker panel on an existing shared session
/// instead of dialling fresh. The panel clones the session
/// and drives every subsequent `docker <verb>` exec through
/// it. Returns NULL if `session` is null.
///
/// # Safety
///
/// `session`, if non-null, must be a live handle produced
/// by [`super::ssh_session::pier_ssh_session_open`].
#[no_mangle]
pub unsafe extern "C" fn pier_docker_open_on_session(
    session: *const super::ssh_session::PierSshSession,
) -> *mut PierDocker {
    if session.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: live handle.
    let shared = unsafe { &*session };
    Box::into_raw(Box::new(PierDocker {
        session: shared.session(),
    }))
}

/// Release a Docker handle. Safe on NULL.
///
/// # Safety
///
/// `h`, if non-null, must have been returned by
/// [`pier_docker_open`] and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pier_docker_free(h: *mut PierDocker) {
    if h.is_null() {
        return;
    }
    // SAFETY: caller contract.
    drop(unsafe { Box::from_raw(h) });
}

/// Release a JSON string returned by
/// [`pier_docker_list_containers`]. Safe on NULL.
///
/// # Safety
///
/// `s`, if non-null, must have come from `pier_docker_*`.
#[no_mangle]
pub unsafe extern "C" fn pier_docker_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: caller contract.
    drop(unsafe { CString::from_raw(s) });
}

/// List containers. Returns a heap JSON string shaped as
/// `[Container, ...]` (see the Rust [`docker::Container`]
/// for field names). Returns NULL on failure. `all != 0`
/// requests stopped containers too.
///
/// Release with [`pier_docker_free_string`].
///
/// # Safety
///
/// `h`, if non-null, must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pier_docker_list_containers(
    h: *mut PierDocker,
    all: c_int,
) -> *mut c_char {
    if h.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: live handle.
    let handle = unsafe { &*h };
    let containers = match docker::list_containers_blocking(&handle.session, all != 0) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("pier_docker_list_containers failed: {e}");
            return ptr::null_mut();
        }
    };
    let json = match serde_json::to_string(&containers) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_docker_list_containers: serialize failed: {e}");
            return ptr::null_mut();
        }
    };
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Inspect a single container and return the raw JSON array
/// from `docker inspect`. Returns NULL on failure.
///
/// Release with [`pier_docker_free_string`].
///
/// # Safety
///
/// `h`, if non-null, must be a live handle. `id` must be a
/// valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn pier_docker_inspect_container(
    h: *mut PierDocker,
    id: *const c_char,
) -> *mut c_char {
    if h.is_null() || id.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: live handle + NUL-terminated id.
    let handle = unsafe { &*h };
    let id_str = match unsafe { CStr::from_ptr(id) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    if !docker::is_safe_id(id_str) {
        return ptr::null_mut();
    }
    let json = match docker::inspect_container_blocking(&handle.session, id_str) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_docker_inspect_container failed: {e}");
            return ptr::null_mut();
        }
    };
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Internal helper: dispatch a simple action by verb.
/// Returns one of the `PIER_DOCKER_ERR_*` codes.
unsafe fn run_action(
    h: *mut PierDocker,
    id: *const c_char,
    verb: &str,
    force: bool,
) -> c_int {
    if h.is_null() || id.is_null() {
        return PIER_DOCKER_ERR_NULL;
    }
    // SAFETY: live handle + NUL-terminated id.
    let handle = unsafe { &*h };
    let id_str = match unsafe { CStr::from_ptr(id) }.to_str() {
        Ok(s) => s,
        Err(_) => return PIER_DOCKER_ERR_UTF8,
    };
    if !docker::is_safe_id(id_str) {
        return PIER_DOCKER_ERR_UNSAFE_ID;
    }
    let result = match verb {
        "start" => docker::start_blocking(&handle.session, id_str),
        "stop" => docker::stop_blocking(&handle.session, id_str),
        "restart" => docker::restart_blocking(&handle.session, id_str),
        "rm" => docker::remove_blocking(&handle.session, id_str, force),
        _ => unreachable!("invalid verb {verb}"),
    };
    match result {
        Ok(()) => PIER_DOCKER_OK,
        Err(e) => {
            log::warn!("pier_docker {verb} failed: {e}");
            PIER_DOCKER_ERR_FAILED
        }
    }
}

/// Start a container by id. Returns 0 on success or a
/// negative `PIER_DOCKER_ERR_*` code.
///
/// # Safety
///
/// `h`, if non-null, must be a live handle. `id` must be a
/// valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn pier_docker_start(h: *mut PierDocker, id: *const c_char) -> c_int {
    // SAFETY: forwarded to run_action.
    unsafe { run_action(h, id, "start", false) }
}

/// Stop a container by id.
///
/// # Safety
/// Same as [`pier_docker_start`].
#[no_mangle]
pub unsafe extern "C" fn pier_docker_stop(h: *mut PierDocker, id: *const c_char) -> c_int {
    // SAFETY: forwarded.
    unsafe { run_action(h, id, "stop", false) }
}

/// Restart a container by id.
///
/// # Safety
/// Same as [`pier_docker_start`].
#[no_mangle]
pub unsafe extern "C" fn pier_docker_restart(h: *mut PierDocker, id: *const c_char) -> c_int {
    // SAFETY: forwarded.
    unsafe { run_action(h, id, "restart", false) }
}

/// Remove a container. `force != 0` passes `--force` to
/// `docker rm`, which also kills running containers.
///
/// # Safety
/// Same as [`pier_docker_start`].
#[no_mangle]
pub unsafe extern "C" fn pier_docker_remove(
    h: *mut PierDocker,
    id: *const c_char,
    force: c_int,
) -> c_int {
    // SAFETY: forwarded.
    unsafe { run_action(h, id, "rm", force != 0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_inputs_are_safe() {
        // SAFETY: every null path is documented.
        unsafe {
            assert!(pier_docker_open(
                ptr::null(), 22, ptr::null(),
                PIER_AUTH_PASSWORD, ptr::null(), ptr::null()
            )
            .is_null());
            assert!(pier_docker_list_containers(ptr::null_mut(), 0).is_null());
            assert!(pier_docker_inspect_container(ptr::null_mut(), ptr::null()).is_null());
            assert_eq!(pier_docker_start(ptr::null_mut(), ptr::null()), PIER_DOCKER_ERR_NULL);
            assert_eq!(pier_docker_stop(ptr::null_mut(), ptr::null()), PIER_DOCKER_ERR_NULL);
            assert_eq!(pier_docker_restart(ptr::null_mut(), ptr::null()), PIER_DOCKER_ERR_NULL);
            assert_eq!(pier_docker_remove(ptr::null_mut(), ptr::null(), 0), PIER_DOCKER_ERR_NULL);
            pier_docker_free_string(ptr::null_mut());
            pier_docker_free(ptr::null_mut());
        }
    }

    #[test]
    fn unreachable_host_fails_fast() {
        let host = CString::new("192.0.2.1").unwrap();
        let user = CString::new("root").unwrap();
        let pass = CString::new("x").unwrap();
        let start = std::time::Instant::now();
        // SAFETY: all valid NUL-terminated strings.
        let h = unsafe {
            pier_docker_open(
                host.as_ptr(), 22, user.as_ptr(),
                PIER_AUTH_PASSWORD, pass.as_ptr(), ptr::null(),
            )
        };
        let elapsed = start.elapsed();
        assert!(h.is_null());
        assert!(
            elapsed < std::time::Duration::from_secs(15),
            "docker open should fail fast on unroutable host, took {elapsed:?}",
        );
    }

    #[test]
    fn error_codes_are_distinct() {
        // Sanity: the four error codes have to be unique so
        // the C++ side can switch on them.
        let codes = vec![
            PIER_DOCKER_OK,
            PIER_DOCKER_ERR_NULL,
            PIER_DOCKER_ERR_UTF8,
            PIER_DOCKER_ERR_FAILED,
            PIER_DOCKER_ERR_UNSAFE_ID,
        ];
        let mut sorted = codes.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len());
    }
}
