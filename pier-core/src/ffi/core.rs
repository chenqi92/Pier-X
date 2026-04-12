//! C ABI surface — the stable boundary between `pier-core` and any UI layer.
//!
//! Functions in this module are `extern "C"` with C-compatible types only.
//! They are intended to be consumed from Qt (via a thin C++ wrapper today,
//! via cxx-qt once signals/slots are needed), Swift, or any other language
//! that speaks the C ABI.
//!
//! ## Memory rules
//!
//! - All `*const c_char` returned from this module are owned by `pier-core`.
//!   Callers MUST NOT free them. They live for the lifetime of the process.
//! - All inputs are `*const c_char` NUL-terminated UTF-8; passing a null
//!   pointer is defined to return the same result as an empty string.
//! - Numeric return values use `0` for "false/off" and `1` for "true/on"
//!   unless otherwise documented.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::OnceLock;

/// Returns the `pier-core` crate version as a NUL-terminated C string.
///
/// The returned pointer is statically allocated and valid for the lifetime
/// of the process. Do not free.
#[no_mangle]
pub extern "C" fn pier_core_version() -> *const c_char {
    static VERSION_C: OnceLock<CString> = OnceLock::new();
    VERSION_C
        .get_or_init(|| CString::new(crate::VERSION).expect("VERSION contains NUL byte"))
        .as_ptr()
}

/// Returns a short human-readable build descriptor: `"<version> (<profile>)"`.
///
/// `<profile>` is `release` or `debug` based on how pier-core was compiled.
/// Statically allocated, do not free.
#[no_mangle]
pub extern "C" fn pier_core_build_info() -> *const c_char {
    static INFO_C: OnceLock<CString> = OnceLock::new();
    INFO_C
        .get_or_init(|| {
            let profile = if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            };
            let s = format!("{} ({})", crate::VERSION, profile);
            CString::new(s).expect("build info contains NUL byte")
        })
        .as_ptr()
}

/// Returns 1 if `pier-core` was built with the given feature flag, 0 otherwise.
///
/// A null `name` is treated as an empty feature name (always returns 0).
/// Recognised names will grow as per-protocol modules land (`ssh`, `sftp`,
/// `pty`, `rdp`, `vnc`, `git`, etc.). Today no feature flags exist.
///
/// # Safety
///
/// `name`, if non-null, must point at a NUL-terminated UTF-8 byte
/// sequence readable for the lifetime of this call. Null is defined
/// to return 0 without touching memory.
#[no_mangle]
pub unsafe extern "C" fn pier_core_has_feature(name: *const c_char) -> i32 {
    if name.is_null() {
        return 0;
    }
    // SAFETY: caller contract, and we verified non-null immediately above.
    let Ok(name) = (unsafe { CStr::from_ptr(name) }).to_str() else {
        return 0;
    };
    match name {
        // Placeholder — real entries land with each protocol module.
        "" => 0,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_string_is_returned() {
        let ptr = pier_core_version();
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, crate::VERSION);
    }

    #[test]
    fn build_info_contains_version_and_profile() {
        let ptr = pier_core_build_info();
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert!(s.contains(crate::VERSION));
        assert!(s.contains("release") || s.contains("debug"));
    }

    #[test]
    fn has_feature_null_safe() {
        // SAFETY: null is defined to be handled without touching memory.
        assert_eq!(unsafe { pier_core_has_feature(std::ptr::null()) }, 0);
    }

    #[test]
    fn has_feature_unknown_name_returns_zero() {
        let c = CString::new("not_a_real_feature").unwrap();
        // SAFETY: CString produces a NUL-terminated UTF-8 byte sequence
        // that lives for the duration of this assertion.
        assert_eq!(unsafe { pier_core_has_feature(c.as_ptr()) }, 0);
    }
}
