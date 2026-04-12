//! C ABI for file search.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use crate::services::search;

fn to_json(s: &str) -> *mut c_char {
    CString::new(s)
        .map(|c| c.into_raw())
        .unwrap_or(ptr::null_mut())
}
fn err_json(msg: &str) -> *mut c_char {
    to_json(&serde_json::json!({"error": msg}).to_string())
}

/// Search for files by name pattern. Returns JSON array.
#[no_mangle]
pub unsafe extern "C" fn pier_search_files(
    root: *const c_char,
    pattern: *const c_char,
    max_results: u32,
) -> *mut c_char {
    if root.is_null() || pattern.is_null() {
        return err_json("null");
    }
    let r = match unsafe { CStr::from_ptr(root) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("utf8"),
    };
    let p = match unsafe { CStr::from_ptr(pattern) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("utf8"),
    };
    match search::search_files(r, p, max_results as usize) {
        Ok(results) => to_json(&serde_json::to_string(&results).unwrap_or_else(|_| "[]".into())),
        Err(e) => err_json(&e.to_string()),
    }
}

/// Search file contents. Returns JSON array.
#[no_mangle]
pub unsafe extern "C" fn pier_search_content(
    root: *const c_char,
    pattern: *const c_char,
    max_results: u32,
) -> *mut c_char {
    if root.is_null() || pattern.is_null() {
        return err_json("null");
    }
    let r = match unsafe { CStr::from_ptr(root) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("utf8"),
    };
    let p = match unsafe { CStr::from_ptr(pattern) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("utf8"),
    };
    match search::search_content(r, p, max_results as usize) {
        Ok(results) => to_json(&serde_json::to_string(&results).unwrap_or_else(|_| "[]".into())),
        Err(e) => err_json(&e.to_string()),
    }
}

/// Free a search result string.
#[no_mangle]
pub unsafe extern "C" fn pier_search_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}
