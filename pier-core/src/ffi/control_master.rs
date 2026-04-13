//! C ABI for SSH ControlMaster — execute commands through the
//! terminal's SSH connection socket.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use crate::ssh::control_master::ControlMasterSession;

/// Opaque handle.
pub struct PierControlMaster {
    session: ControlMasterSession,
}

#[no_mangle]
/// Create a ControlMaster handle bound to `host:user:port`.
pub unsafe extern "C" fn pier_control_master_new(
    host: *const c_char,
    port: u16,
    user: *const c_char,
) -> *mut PierControlMaster {
    if host.is_null() || user.is_null() {
        return ptr::null_mut();
    }
    let h = match unsafe { CStr::from_ptr(host) }.to_str() {
        Ok(s) if !s.is_empty() => s,
        _ => return ptr::null_mut(),
    };
    let u = match unsafe { CStr::from_ptr(user) }.to_str() {
        Ok(s) if !s.is_empty() => s,
        _ => return ptr::null_mut(),
    };
    Box::into_raw(Box::new(PierControlMaster {
        session: ControlMasterSession::new(h, u, port),
    }))
}

/// Try to connect: wait for socket then spawn master if needed.
/// Returns 1 on success, 0 on failure.
#[no_mangle]
pub unsafe extern "C" fn pier_control_master_connect(
    h: *mut PierControlMaster,
    timeout_secs: u32,
) -> i32 {
    if h.is_null() {
        return 0;
    }
    let cm = unsafe { &*h };
    if cm.session.connect(timeout_secs) {
        1
    } else {
        0
    }
}

/// Execute a command through the ControlMaster socket.
/// Returns heap JSON: {"code": N, "stdout": "..."} or NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_control_master_exec(
    h: *const PierControlMaster,
    command: *const c_char,
) -> *mut c_char {
    if h.is_null() || command.is_null() {
        return ptr::null_mut();
    }
    let cm = unsafe { &*h };
    let cmd = match unsafe { CStr::from_ptr(command) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    match cm.session.exec(cmd) {
        Ok((code, stdout)) => {
            let json = serde_json::json!({"code": code, "stdout": stdout});
            CString::new(json.to_string())
                .map(|c| c.into_raw())
                .unwrap_or(ptr::null_mut())
        }
        Err(_) => ptr::null_mut(),
    }
}

/// Check if the ControlMaster socket is alive.
#[no_mangle]
pub unsafe extern "C" fn pier_control_master_is_alive(h: *const PierControlMaster) -> i32 {
    if h.is_null() {
        return 0;
    }
    let cm = unsafe { &*h };
    if cm.session.is_alive() {
        1
    } else {
        0
    }
}

#[no_mangle]
/// Release a handle previously returned by
/// [`pier_control_master_new`]. Safe to call with NULL.
pub unsafe extern "C" fn pier_control_master_free(h: *mut PierControlMaster) {
    if !h.is_null() {
        drop(unsafe { Box::from_raw(h) });
    }
}

#[no_mangle]
/// Release a heap string returned by this module. Safe to call with
/// NULL.
pub unsafe extern "C" fn pier_control_master_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}
