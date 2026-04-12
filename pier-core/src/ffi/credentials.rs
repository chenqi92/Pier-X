//! C ABI for the OS keyring layer.
//!
//! ## Design rule: write-only from C++
//!
//! The C++ side is allowed to **set** and **delete** entries by id.
//! It is deliberately NOT allowed to **read** them. The plaintext
//! password collected by the dialog crosses this FFI exactly
//! once (into [`pier_credential_set`]) and is then owned by the
//! OS keyring; the SSH session layer pulls it back out from
//! inside the Rust handshake task via [`crate::credentials::get`]
//! when it actually needs to authenticate.
//!
//! This means that even if the entire C++/QML half of pier-x is
//! compromised at runtime, an attacker can write arbitrary
//! credentials but cannot exfiltrate the ones that are already
//! stored — that operation requires reaching into Rust code that
//! the C++ side has no symbols for.
//!
//! ## Error codes
//!
//! | value | meaning                                         |
//! |------:|-------------------------------------------------|
//! |  `0`  | success                                         |
//! | `-1`  | null pointer / empty id                         |
//! | `-2`  | id or value is not valid UTF-8                  |
//! | `-3`  | OS keyring rejected or could not service the call |

#![allow(clippy::missing_safety_doc)]

use std::ffi::CStr;
use std::os::raw::c_char;

/// Store a secret string under `id` in the OS keyring. Overwrites
/// any existing entry under the same id.
///
/// # Safety
///
/// Both pointers must be valid NUL-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn pier_credential_set(id: *const c_char, value: *const c_char) -> i32 {
    if id.is_null() || value.is_null() {
        return -1;
    }
    // SAFETY: caller contract.
    let id_str = match unsafe { CStr::from_ptr(id) }.to_str() {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => return -1,
        Err(_) => return -2,
    };
    let value_str = match unsafe { CStr::from_ptr(value) }.to_str() {
        Ok(s) => s,
        Err(_) => return -2,
    };
    match crate::credentials::set(id_str, value_str) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("pier_credential_set({id_str}) failed: {e}");
            -3
        }
    }
}

/// Delete the entry stored under `id` in the OS keyring. A
/// missing entry is treated as success — calling delete on an
/// already-absent id returns `0`.
///
/// # Safety
///
/// `id` must be a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn pier_credential_delete(id: *const c_char) -> i32 {
    if id.is_null() {
        return -1;
    }
    // SAFETY: caller contract.
    let id_str = match unsafe { CStr::from_ptr(id) }.to_str() {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => return -1,
        Err(_) => return -2,
    };
    match crate::credentials::delete(id_str) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("pier_credential_delete({id_str}) failed: {e}");
            -3
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::ptr;

    #[test]
    fn null_id_returns_minus_one() {
        // SAFETY: passing null is part of the documented
        // contract — both functions must reject without
        // touching memory.
        unsafe {
            let value = CString::new("v").unwrap();
            assert_eq!(pier_credential_set(ptr::null(), value.as_ptr()), -1);
            assert_eq!(pier_credential_delete(ptr::null()), -1);
        }
    }

    #[test]
    fn null_value_returns_minus_one() {
        let id = CString::new("pier-x-test-null-value").unwrap();
        // SAFETY: id is a valid C string; value is null.
        unsafe {
            assert_eq!(pier_credential_set(id.as_ptr(), ptr::null()), -1);
        }
    }

    #[test]
    fn empty_id_returns_minus_one() {
        let id = CString::new("").unwrap();
        let value = CString::new("v").unwrap();
        // SAFETY: both are valid C strings; id is empty.
        unsafe {
            assert_eq!(pier_credential_set(id.as_ptr(), value.as_ptr()), -1);
            assert_eq!(pier_credential_delete(id.as_ptr()), -1);
        }
    }

    // The actual round-trip against a live OS keyring is
    // covered by credentials::tests::round_trip and is gated
    // behind #[ignore] there because CI runners don't have an
    // unlocked keyring. Local developers run with
    // `cargo test -- --ignored`.
}
