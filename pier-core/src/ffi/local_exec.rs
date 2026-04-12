//! C ABI for local command execution — Docker, metrics, logs
//! without SSH. All functions are synchronous and blocking.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use crate::services::local_exec;

fn to_json(s: &str) -> *mut c_char {
    CString::new(s).map(|c| c.into_raw()).unwrap_or(ptr::null_mut())
}

/// Free a string returned by any pier_local_* function.
#[no_mangle]
pub unsafe extern "C" fn pier_local_free_string(s: *mut c_char) {
    if !s.is_null() { drop(unsafe { CString::from_raw(s) }); }
}

// ─── Local Docker ───────────────────────────────────────

/// List local containers. Returns NDJSON string or NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_local_docker_list_containers(all: i32) -> *mut c_char {
    match local_exec::docker_list_containers(all != 0) {
        Ok(s) => to_json(&s),
        Err(e) => { log::warn!("pier_local_docker_list_containers: {e}"); ptr::null_mut() }
    }
}

/// List local images. Returns NDJSON string or NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_local_docker_list_images() -> *mut c_char {
    match local_exec::docker_list_images() {
        Ok(s) => to_json(&s),
        Err(e) => { log::warn!("pier_local_docker_list_images: {e}"); ptr::null_mut() }
    }
}

/// List local volumes. Returns NDJSON string or NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_local_docker_list_volumes() -> *mut c_char {
    match local_exec::docker_list_volumes() {
        Ok(s) => to_json(&s),
        Err(e) => { log::warn!("{e}"); ptr::null_mut() }
    }
}

/// List local networks. Returns NDJSON string or NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_local_docker_list_networks() -> *mut c_char {
    match local_exec::docker_list_networks() {
        Ok(s) => to_json(&s),
        Err(e) => { log::warn!("{e}"); ptr::null_mut() }
    }
}

/// Local docker action (start/stop/restart/rm). Returns 0 or -1.
#[no_mangle]
pub unsafe extern "C" fn pier_local_docker_action(
    verb: *const c_char, id: *const c_char, force: i32,
) -> i32 {
    if verb.is_null() || id.is_null() { return -1; }
    let v = match unsafe { CStr::from_ptr(verb) }.to_str() { Ok(s) => s, Err(_) => return -1 };
    let i = match unsafe { CStr::from_ptr(id) }.to_str() { Ok(s) => s, Err(_) => return -1 };
    match local_exec::docker_action(v, i, force != 0) { Ok(()) => 0, Err(_) => -1 }
}

/// Local docker inspect. Returns JSON string or NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_local_docker_inspect(id: *const c_char) -> *mut c_char {
    if id.is_null() { return ptr::null_mut(); }
    let i = match unsafe { CStr::from_ptr(id) }.to_str() { Ok(s) => s, Err(_) => return ptr::null_mut() };
    match local_exec::docker_inspect(i) {
        Ok(s) => to_json(&s),
        Err(_) => ptr::null_mut(),
    }
}

// ─── Local System Metrics ───────────────────────────────

/// Get local system metrics as JSON. Returns heap string or NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_local_system_metrics() -> *mut c_char {
    match local_exec::system_metrics() {
        Ok(m) => match serde_json::to_string(&m) {
            Ok(j) => to_json(&j),
            Err(_) => ptr::null_mut(),
        },
        Err(e) => { log::warn!("pier_local_system_metrics: {e}"); ptr::null_mut() }
    }
}

// ─── Local Shell Exec ───────────────────────────────────

/// Run a local command and return stdout. Returns heap string or NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_local_exec(cmd: *const c_char) -> *mut c_char {
    if cmd.is_null() { return ptr::null_mut(); }
    let c = match unsafe { CStr::from_ptr(cmd) }.to_str() { Ok(s) => s, Err(_) => return ptr::null_mut() };
    match local_exec::exec(c) {
        Ok((_, stdout)) => to_json(&stdout),
        Err(e) => { log::warn!("pier_local_exec: {e}"); ptr::null_mut() }
    }
}
