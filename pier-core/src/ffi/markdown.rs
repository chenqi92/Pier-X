//! C ABI for the local Markdown preview (M5e).
//!
//! ## Shape
//!
//! This is the simplest FFI in pier-core: two pure functions
//! (render + load) plus a free. No handles, no sessions, no
//! worker threads — everything runs synchronously on the
//! caller's thread because pulldown-cmark is fast enough that
//! a 16 MB markdown file still renders in a few milliseconds.
//!
//! Both output paths return heap-allocated C strings that the
//! caller must release with [`pier_markdown_free_string`].
//! Returning `NULL` means "failure"; the C++ side treats it
//! as an empty result and shows a fallback message.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;
use std::ptr;

use crate::markdown;

/// Render UTF-8 markdown `source` to heap-allocated UTF-8
/// HTML. Returns NULL if `source` is null, non-UTF-8, or if
/// the produced HTML contains an interior NUL byte (which
/// pulldown-cmark itself won't emit but we guard against
/// anyway).
///
/// # Safety
///
/// `source` must be a valid NUL-terminated C string for the
/// duration of the call.
#[no_mangle]
pub unsafe extern "C" fn pier_markdown_render_html(source: *const c_char) -> *mut c_char {
    if source.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller contract — NUL-terminated UTF-8.
    let src = match unsafe { CStr::from_ptr(source) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    let html = markdown::render_html(src);
    match CString::new(html) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Read a local file from `path` as UTF-8 markdown, then
/// render it to HTML the same way [`pier_markdown_render_html`]
/// does. Returns NULL on any I/O error, non-UTF-8 content,
/// or oversized file. The unrendered source is not returned —
/// the caller uses [`pier_markdown_load_source`] for that.
///
/// # Safety
///
/// `path` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn pier_markdown_load_html(path: *const c_char) -> *mut c_char {
    if path.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller contract.
    let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) if !s.is_empty() => s,
        _ => return ptr::null_mut(),
    };
    let source = match markdown::load_file(Path::new(path_str)) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_markdown_load_html: {e}");
            return ptr::null_mut();
        }
    };
    let html = markdown::render_html(&source);
    match CString::new(html) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Read a local file at `path` and return its raw UTF-8
/// contents (no rendering). Used by the "Source" split pane
/// alongside [`pier_markdown_load_html`] so the C++ side
/// doesn't need its own file-read helper.
///
/// # Safety
///
/// `path` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn pier_markdown_load_source(path: *const c_char) -> *mut c_char {
    if path.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller contract.
    let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) if !s.is_empty() => s,
        _ => return ptr::null_mut(),
    };
    let source = match markdown::load_file(Path::new(path_str)) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_markdown_load_source: {e}");
            return ptr::null_mut();
        }
    };
    match CString::new(source) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Free a string returned by any `pier_markdown_*` function.
/// Safe to call with NULL.
///
/// # Safety
///
/// `s`, if non-null, must have been returned by a
/// `pier_markdown_*` call and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pier_markdown_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: caller contract — produced via CString::into_raw.
    drop(unsafe { CString::from_raw(s) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn null_inputs_are_safe() {
        // SAFETY: every null path is documented as safe.
        unsafe {
            assert!(pier_markdown_render_html(ptr::null()).is_null());
            assert!(pier_markdown_load_html(ptr::null()).is_null());
            assert!(pier_markdown_load_source(ptr::null()).is_null());
            pier_markdown_free_string(ptr::null_mut());
        }
    }

    #[test]
    fn render_html_round_trips_through_cstring() {
        let src = CString::new("# Hello **world**").unwrap();
        // SAFETY: valid NUL-terminated string.
        let ptr = unsafe { pier_markdown_render_html(src.as_ptr()) };
        assert!(!ptr.is_null());
        // SAFETY: ptr just came from render_html.
        let cs = unsafe { CStr::from_ptr(ptr) };
        let text = cs.to_str().unwrap();
        assert!(text.contains("<h1>"));
        assert!(text.contains("<strong>world</strong>"));
        // SAFETY: release.
        unsafe { pier_markdown_free_string(ptr) };
    }

    #[test]
    fn load_html_from_temp_file() {
        let path = std::env::temp_dir()
            .join(format!("pier_md_ffi_{}.md", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "## Title\n\n- a\n- b").unwrap();
        }
        let c_path = CString::new(path.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: valid NUL-terminated path.
        let html_ptr = unsafe { pier_markdown_load_html(c_path.as_ptr()) };
        assert!(!html_ptr.is_null());
        // SAFETY: ptr is live.
        let text = unsafe { CStr::from_ptr(html_ptr) }.to_str().unwrap();
        assert!(text.contains("<h2>Title</h2>"));
        assert!(text.contains("<li>a</li>"));
        unsafe { pier_markdown_free_string(html_ptr) };

        let src_ptr = unsafe { pier_markdown_load_source(c_path.as_ptr()) };
        assert!(!src_ptr.is_null());
        let src_text = unsafe { CStr::from_ptr(src_ptr) }.to_str().unwrap();
        assert!(src_text.contains("## Title"));
        unsafe { pier_markdown_free_string(src_ptr) };

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_html_missing_file_returns_null() {
        let bad = CString::new("/nope/does/not/exist.md").unwrap();
        // SAFETY: valid NUL-terminated path.
        let ptr = unsafe { pier_markdown_load_html(bad.as_ptr()) };
        assert!(ptr.is_null());
    }
}
