//! C ABI for the Redis browser panel (M5a).
//!
//! ## Handle model
//!
//! One opaque `*mut PierRedis` per live connection. The handle
//! wraps a [`crate::services::redis::RedisClient`] whose inner
//! `ConnectionManager` auto-reconnects on socket drop — so the
//! C++ side just calls methods and either gets a result or an
//! error, without needing to track connection state.
//!
//! ## JSON-shaped results
//!
//! All of the interesting read operations here (`scan_keys`,
//! `inspect`, `info`) return their payload as a heap-allocated
//! UTF-8 JSON string via `*mut c_char`. The caller frees it
//! with [`pier_redis_free_string`]. This matches the SFTP FFI
//! conventions and keeps the C header tiny — no typed structs
//! to version across releases.
//!
//! ## No auth (yet)
//!
//! M5a is tunnel-only, so the typical endpoint is
//! `127.0.0.1:16379` on localhost. When M5b adds remote-direct
//! Redis connections with AUTH, a new `pier_redis_open_auth`
//! function can be added here without breaking the existing
//! symbol.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use crate::services::redis::{KeyDetails, RedisClient, RedisConfig, ScanResult};

/// Opaque Redis handle, returned by [`pier_redis_open`] and
/// freed by [`pier_redis_free`]. Internally holds the `RedisClient`
/// (which itself is cheap to clone and reference-counts the
/// underlying connection manager).
pub struct PierRedis {
    client: RedisClient,
}

/// Open a Redis connection to `host:port` at database `db`.
/// Performs the TCP handshake + RESP ping synchronously on the
/// calling thread and returns NULL on failure.
///
/// `host` must be a valid NUL-terminated UTF-8 string. `port`
/// must be non-zero. `db` is the logical database index (0-15
/// on standalone Redis; ignored on clusters).
///
/// # Safety
///
/// `host` must point to a valid NUL-terminated C string for the
/// duration of the call.
#[no_mangle]
pub unsafe extern "C" fn pier_redis_open(
    host: *const c_char,
    port: u16,
    db: i64,
) -> *mut PierRedis {
    if host.is_null() || port == 0 {
        return ptr::null_mut();
    }
    // SAFETY: caller contract.
    let host_str = match unsafe { CStr::from_ptr(host) }.to_str() {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => return ptr::null_mut(),
    };

    let config = RedisConfig {
        host: host_str,
        port,
        db,
    };
    match RedisClient::connect_blocking(config) {
        Ok(client) => Box::into_raw(Box::new(PierRedis { client })),
        Err(e) => {
            log::warn!("pier_redis_open failed: {e}");
            ptr::null_mut()
        }
    }
}

/// Round-trip PING. Returns 1 on success, 0 on failure
/// (connection lost, ACL block, unexpected reply).
///
/// # Safety
///
/// `h`, if non-null, must be a live handle produced by
/// [`pier_redis_open`].
#[no_mangle]
pub unsafe extern "C" fn pier_redis_ping(h: *mut PierRedis) -> c_int {
    if h.is_null() {
        return 0;
    }
    // SAFETY: live handle.
    let handle = unsafe { &*h };
    match handle.client.ping_blocking() {
        Ok(reply) if reply == "PONG" => 1,
        _ => 0,
    }
}

/// Enumerate keys matching `pattern` via SCAN. Returns a
/// heap-allocated JSON string with this shape:
///
/// ```json
/// { "keys": ["foo", "bar"], "truncated": false, "limit": 1000 }
/// ```
///
/// Returns NULL on failure. Caller must release the returned
/// string with [`pier_redis_free_string`].
///
/// # Safety
///
/// `h`, if non-null, must be a live handle. `pattern` must be a
/// valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn pier_redis_scan_keys(
    h: *mut PierRedis,
    pattern: *const c_char,
    limit: usize,
) -> *mut c_char {
    if h.is_null() || pattern.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller contract.
    let handle = unsafe { &*h };
    let pat = match unsafe { CStr::from_ptr(pattern) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let result: ScanResult = match handle.client.scan_keys_blocking(pat, limit) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("pier_redis_scan_keys failed: {e}");
            return ptr::null_mut();
        }
    };
    into_json_cstring(&result)
}

/// Fetch type + ttl + bounded preview for a single key.
/// Returns a heap-allocated JSON string shaped like
/// [`crate::services::redis::KeyDetails`]. Caller frees with
/// [`pier_redis_free_string`]. Returns NULL on failure.
///
/// # Safety
///
/// `h`, if non-null, must be a live handle. `key` must be a
/// valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn pier_redis_inspect(
    h: *mut PierRedis,
    key: *const c_char,
) -> *mut c_char {
    if h.is_null() || key.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller contract.
    let handle = unsafe { &*h };
    let key_str = match unsafe { CStr::from_ptr(key) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let details: KeyDetails = match handle.client.inspect_blocking(key_str) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("pier_redis_inspect failed: {e}");
            return ptr::null_mut();
        }
    };
    into_json_cstring(&details)
}

/// Run `INFO <section>` and return the parsed `k: v` map as
/// a JSON object. Pass an empty / NULL `section` for "all".
/// Caller frees the returned string with
/// [`pier_redis_free_string`]. Returns NULL on failure.
///
/// # Safety
///
/// `h`, if non-null, must be a live handle. `section`, if
/// non-null, must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn pier_redis_info(
    h: *mut PierRedis,
    section: *const c_char,
) -> *mut c_char {
    if h.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller contract.
    let handle = unsafe { &*h };
    let section_str = if section.is_null() {
        String::new()
    } else {
        match unsafe { CStr::from_ptr(section) }.to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return ptr::null_mut(),
        }
    };

    let map = match handle.client.info_blocking(&section_str) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("pier_redis_info failed: {e}");
            return ptr::null_mut();
        }
    };
    into_json_cstring(&map)
}

/// Free a JSON string returned by `pier_redis_scan_keys`,
/// `pier_redis_inspect`, or `pier_redis_info`. Safe to call
/// with NULL.
///
/// # Safety
///
/// `s`, if non-null, must have been returned by one of the
/// `pier_redis_*` functions that produce JSON and must not have
/// been freed already.
#[no_mangle]
pub unsafe extern "C" fn pier_redis_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: built via CString::into_raw inside into_json_cstring.
    drop(unsafe { CString::from_raw(s) });
}

/// Drop a Redis handle. Safe to call with NULL.
///
/// # Safety
///
/// `h`, if non-null, must have been returned by
/// [`pier_redis_open`] and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pier_redis_free(h: *mut PierRedis) {
    if h.is_null() {
        return;
    }
    // SAFETY: caller contract — box originally from into_raw.
    drop(unsafe { Box::from_raw(h) });
}

/// Serialize `value` to a heap-allocated C string. Returns NULL
/// on either JSON serialization failure or interior-NUL in the
/// produced JSON (CString rejects `\0`).
fn into_json_cstring<T: serde::Serialize>(value: &T) -> *mut c_char {
    let json = match serde_json::to_string(value) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_redis: json serialize failed: {e}");
            return ptr::null_mut();
        }
    };
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_inputs_are_safe() {
        // SAFETY: all nulls documented as safe.
        unsafe {
            assert!(pier_redis_open(ptr::null(), 6379, 0).is_null());
            assert_eq!(pier_redis_ping(ptr::null_mut()), 0);
            assert!(pier_redis_scan_keys(ptr::null_mut(), ptr::null(), 100).is_null());
            assert!(pier_redis_inspect(ptr::null_mut(), ptr::null()).is_null());
            assert!(pier_redis_info(ptr::null_mut(), ptr::null()).is_null());
            pier_redis_free_string(ptr::null_mut());
            pier_redis_free(ptr::null_mut());
        }
    }

    #[test]
    fn zero_port_rejected() {
        // SAFETY: valid host string.
        let host = CString::new("127.0.0.1").unwrap();
        let h = unsafe { pier_redis_open(host.as_ptr(), 0, 0) };
        assert!(h.is_null());
    }

    #[test]
    fn closed_port_fails_fast() {
        // Port 1 is reserved; nothing should answer there. Make
        // sure we fail fast without hanging.
        let host = CString::new("127.0.0.1").unwrap();
        let start = std::time::Instant::now();
        // SAFETY: valid host string.
        let h = unsafe { pier_redis_open(host.as_ptr(), 1, 0) };
        let elapsed = start.elapsed();
        assert!(h.is_null());
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "redis open should fail fast on closed port, took {elapsed:?}",
        );
    }

    #[test]
    fn json_roundtrip_through_cstring() {
        let r = ScanResult {
            keys: vec!["a".into(), "b".into()],
            truncated: true,
            limit: 10,
        };
        let s = into_json_cstring(&r);
        assert!(!s.is_null());
        // SAFETY: just produced by into_json_cstring.
        let cs = unsafe { CStr::from_ptr(s) };
        let text = cs.to_str().unwrap();
        assert!(text.contains("\"truncated\":true"));
        assert!(text.contains("\"a\""));
        // SAFETY: release the allocation.
        unsafe { pier_redis_free_string(s) };
    }
}
