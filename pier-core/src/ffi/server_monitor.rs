//! C ABI for the server resource monitor (M7b).
//!
//! One function: `pier_server_monitor_probe` runs a combined
//! `uptime + free + df + /proc/stat` probe via SSH exec and
//! returns a [`ServerSnapshot`] as a heap JSON string. The
//! C++ side polls this on a QTimer, typically every 5 s.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use crate::services::server_monitor;
use crate::ssh::{HostKeyVerifier, SshConfig, SshSession};

/// Opaque monitor handle (holds an SSH session).
pub struct PierServerMonitor {
    session: SshSession,
}

/// Open an SSH session for monitoring. Same auth-kind
/// table as every other session-based FFI.
#[no_mangle]
pub unsafe extern "C" fn pier_server_monitor_open(
    host: *const c_char,
    port: u16,
    user: *const c_char,
    auth_kind: c_int,
    secret: *const c_char,
    extra: *const c_char,
) -> *mut PierServerMonitor {
    if host.is_null() || user.is_null() {
        return ptr::null_mut();
    }
    let host_str = match unsafe { CStr::from_ptr(host) }.to_str() {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => return ptr::null_mut(),
    };
    let user_str = match unsafe { CStr::from_ptr(user) }.to_str() {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => return ptr::null_mut(),
    };
    let auth = match unsafe { super::ssh_session::parse_auth_kind(auth_kind, secret, extra) } {
        Some(a) => a,
        None => return ptr::null_mut(),
    };
    let mut config = SshConfig::new(&host_str, &host_str, &user_str);
    config.port = port;
    config.auth = auth;
    match SshSession::connect_blocking(&config, HostKeyVerifier::default()) {
        Ok(session) => Box::into_raw(Box::new(PierServerMonitor { session })),
        Err(e) => {
            log::warn!("pier_server_monitor_open failed: {e}");
            ptr::null_mut()
        }
    }
}

/// M3e variant: open monitor on a shared session.
#[no_mangle]
pub unsafe extern "C" fn pier_server_monitor_open_on_session(
    session: *const super::ssh_session::PierSshSession,
) -> *mut PierServerMonitor {
    if session.is_null() {
        return ptr::null_mut();
    }
    let shared = unsafe { &*session };
    Box::into_raw(Box::new(PierServerMonitor {
        session: shared.session(),
    }))
}

/// Run a probe and return the snapshot as a heap JSON string.
/// Returns NULL on failure. Release with
/// [`pier_server_monitor_free_string`].
#[no_mangle]
pub unsafe extern "C" fn pier_server_monitor_probe(h: *mut PierServerMonitor) -> *mut c_char {
    if h.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*h };
    let snap = match server_monitor::probe_blocking(&handle.session) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_server_monitor_probe failed: {e}");
            return ptr::null_mut();
        }
    };
    let json = match serde_json::to_string(&snap) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_server_monitor_probe serialize failed: {e}");
            return ptr::null_mut();
        }
    };
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Free a JSON string. Safe on NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_server_monitor_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    drop(unsafe { CString::from_raw(s) });
}

/// Free a monitor handle. Safe on NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_server_monitor_free(h: *mut PierServerMonitor) {
    if h.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(h) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::sftp::PIER_AUTH_PASSWORD;

    #[test]
    fn null_inputs_are_safe() {
        unsafe {
            assert!(pier_server_monitor_open(
                ptr::null(),
                22,
                ptr::null(),
                PIER_AUTH_PASSWORD,
                ptr::null(),
                ptr::null()
            )
            .is_null());
            assert!(pier_server_monitor_probe(ptr::null_mut()).is_null());
            assert!(pier_server_monitor_open_on_session(ptr::null()).is_null());
            pier_server_monitor_free_string(ptr::null_mut());
            pier_server_monitor_free(ptr::null_mut());
        }
    }
}
