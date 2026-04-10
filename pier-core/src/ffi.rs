//! C ABI surface — the stable boundary between `pier-core` and any UI layer.
//!
//! Functions in this module are `extern "C"` with C-compatible types only.
//! They are intended to be consumed from Qt (via cxx-qt or direct FFI),
//! Swift, or any other language that speaks the C ABI.
//!
//! ## Memory rules
//!
//! - All `*const c_char` returned from this module are owned by `pier-core`.
//!   Callers MUST NOT free them. They live for the lifetime of the process.
//! - All errors are surfaced via return codes (non-zero = error). Detailed
//!   error messages can be retrieved via `pier_core_last_error`.

use std::ffi::CString;
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

/// Returns 1 if `pier-core` was built with the given feature flag, 0 otherwise.
///
/// Currently always returns 0 — feature flags will land alongside the
/// per-protocol module ports (ssh, rdp, vnc, etc.).
#[no_mangle]
pub extern "C" fn pier_core_has_feature(_name: *const c_char) -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn version_string_is_returned() {
        let ptr = pier_core_version();
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, crate::VERSION);
    }
}
