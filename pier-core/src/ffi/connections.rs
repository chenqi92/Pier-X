//! C ABI for the persisted connections store.
//!
//! ## Shape
//!
//! The C++ side never touches the on-disk file directly. It
//! only talks to two functions:
//!
//!  * [`pier_connections_load_json`] returns a heap-allocated
//!    NUL-terminated UTF-8 C string containing the entire store
//!    serialized as JSON, or `NULL` on failure. The caller owns
//!    the buffer and must hand it back to
//!    [`pier_connections_free_json`] when done.
//!
//!  * [`pier_connections_save_json`] takes a JSON string,
//!    parses it through serde into a [`ConnectionStore`], and
//!    writes it back to disk atomically. Returns `0` on success
//!    or a negative error code.
//!
//! Keeping the C++ side stateless about the file format means
//! the on-disk schema can evolve (versioning, new fields,
//! migrations) without any C++ changes — the JSON-shaped
//! contract is the only stable surface.
//!
//! ## Why heap-allocated rather than thread-local?
//!
//! [`pier_terminal_last_ssh_error`] uses thread-local storage
//! because the error is consumed within the same call. The
//! connections JSON, by contrast, is consumed on the main
//! thread but produced by a load that we expect to be slow
//! enough to want to run on a worker thread eventually.
//! Heap-allocating + a matching free is the only safe shape
//! that supports cross-thread handoff.
//!
//! ## Error codes for save
//!
//! | value | meaning                                       |
//! |------:|-----------------------------------------------|
//! |  `0`  | success                                       |
//! | `-1`  | null json pointer                             |
//! | `-2`  | json is not valid UTF-8                       |
//! | `-3`  | json failed to parse into a ConnectionStore   |
//! | `-4`  | I/O error writing the file                    |
//! | `-5`  | no usable application data directory          |

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use crate::connections::{ConnectionStore, ConnectionStoreError};

/// Load the persisted connections store and return its JSON
/// representation as an owned C string. Returns `NULL` if no
/// data directory exists or if the file is malformed; in the
/// success-but-empty case (no file yet) returns a JSON document
/// for an empty store rather than NULL.
///
/// The returned pointer is heap-allocated by Rust and **must**
/// be released by [`pier_connections_free_json`]. Calling
/// `free` from C is undefined behavior because the allocator
/// may differ.
///
/// # Safety
///
/// Always safe to call. The returned pointer is either null or
/// a valid NUL-terminated UTF-8 C string until the matching
/// `pier_connections_free_json` call.
#[no_mangle]
pub unsafe extern "C" fn pier_connections_load_json() -> *mut c_char {
    let store = match ConnectionStore::load_default() {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_connections_load_json failed: {e}");
            return ptr::null_mut();
        }
    };
    match serde_json::to_string(&store) {
        Ok(json) => match CString::new(json) {
            Ok(cstring) => cstring.into_raw(),
            Err(e) => {
                // serde_json output never contains NUL bytes, but
                // surface this just in case.
                log::warn!("pier_connections_load_json CString conversion failed: {e}");
                ptr::null_mut()
            }
        },
        Err(e) => {
            log::warn!("pier_connections_load_json serialize failed: {e}");
            ptr::null_mut()
        }
    }
}

/// Release a JSON string previously returned by
/// [`pier_connections_load_json`]. Safe to call with null.
///
/// # Safety
///
/// `json`, if non-null, must be a pointer previously returned
/// by [`pier_connections_load_json`] and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pier_connections_free_json(json: *mut c_char) {
    if json.is_null() {
        return;
    }
    // SAFETY: caller contract — pointer came from CString::into_raw.
    let _ = unsafe { CString::from_raw(json) };
}

/// Persist a JSON-serialized [`ConnectionStore`] to disk
/// atomically. Returns `0` on success or a negative error code.
///
/// # Safety
///
/// `json` must be a valid NUL-terminated UTF-8 C string holding
/// a serialized `ConnectionStore`.
#[no_mangle]
pub unsafe extern "C" fn pier_connections_save_json(json: *const c_char) -> i32 {
    if json.is_null() {
        return -1;
    }
    // SAFETY: caller contract.
    let json_str = match unsafe { CStr::from_ptr(json) }.to_str() {
        Ok(s) => s,
        Err(_) => return -2,
    };
    let store: ConnectionStore = match serde_json::from_str(json_str) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_connections_save_json parse failed: {e}");
            return -3;
        }
    };
    match store.save_default() {
        Ok(()) => 0,
        Err(ConnectionStoreError::NoDataDir) => -5,
        Err(ConnectionStoreError::Io(e)) => {
            log::warn!("pier_connections_save_json io failed: {e}");
            -4
        }
        Err(e) => {
            log::warn!("pier_connections_save_json other failure: {e}");
            -4
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    #[test]
    fn save_rejects_null_pointer() {
        // SAFETY: null is defined.
        assert_eq!(unsafe { pier_connections_save_json(ptr::null()) }, -1);
    }

    #[test]
    fn save_rejects_garbage_json() {
        let s = std::ffi::CString::new("not valid json").unwrap();
        // SAFETY: s is a valid C string.
        let rc = unsafe { pier_connections_save_json(s.as_ptr()) };
        assert_eq!(rc, -3);
    }

    #[test]
    fn free_null_is_a_noop() {
        // SAFETY: null is defined to be a no-op.
        unsafe { pier_connections_free_json(ptr::null_mut()) };
    }
}
