//! C ABI wrapper around [`crate::terminal::PierTerminal`].
//!
//! ## Handle model
//!
//! The consumer gets an opaque `*mut PierTerminal` from
//! [`pier_terminal_new`]. Every other function in this module takes
//! that same pointer and is a no-op / error if the pointer is null.
//! The only way to release the handle is [`pier_terminal_free`],
//! which joins the reader thread and reaps the child before
//! returning. Double-free is undefined behavior (standard C ABI
//! contract); the C++ wrapper stores the handle inside a
//! `std::unique_ptr` with a custom deleter to enforce this.
//!
//! ## Notify callback
//!
//! [`pier_terminal_new`] takes a function pointer `notify` and an
//! opaque `user_data`. When the reader thread has fresh output or the
//! child has exited, it invokes `notify(user_data, event)` from the
//! reader thread — NOT the UI thread. The implementation must be
//! thread-safe and quick. The canonical body is a single
//! `QMetaObject::invokeMethod(Qt::QueuedConnection, ...)` that wakes
//! a slot on the Qt main thread; the slot then calls
//! [`pier_terminal_snapshot`] on its own terms. Do not call back into
//! this module synchronously from the callback — while the current
//! implementation releases its mutex before invoking the callback,
//! depending on that is fragile.
//!
//! ## Snapshot model
//!
//! Callers allocate a `PierCell` buffer large enough for `cols*rows`
//! and pass it to [`pier_terminal_snapshot`]. The function fills
//! [`PierGridInfo`] with the current dimensions + cursor, and memcpys
//! the grid cells into the buffer. If the caller's buffer is too
//! small the function returns `-2` without touching the buffer. This
//! shape means zero allocation in Rust, zero lifetime dance, and a
//! single cache-friendly copy for rendering.
//!
//! ## Error codes
//!
//! | value | meaning                                      |
//! |------:|----------------------------------------------|
//! |  `0`  | success                                      |
//! | `-1`  | null handle / null out pointer               |
//! | `-2`  | buffer too small                             |
//! | `-3`  | underlying I/O error (write / resize failed) |
//! | `-4`  | platform does not support this backend yet   |

#![allow(clippy::missing_safety_doc)]
// Every function in this module is `unsafe extern "C"` and its
// Safety section is spelled out in the per-function doc comment
// and/or in pier_terminal.h, rather than a clippy-mandated
// boilerplate `# Safety` line.

use std::os::raw::{c_char, c_void};
use std::ptr;

use crate::terminal::{Cell, Color, NotifyFn, PierTerminal, TerminalError};

/// Plain-data cell struct mirroring the Rust [`Cell`] but with a
/// fixed, stable layout suitable for memcpy across the FFI boundary.
///
/// Fields are ordered and sized so the overall struct fits neatly in
/// 16 bytes on every target — padding + packing are deliberate so
/// pier_terminal.h can declare the same layout without uintptr
/// trickery.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PierCell {
    /// Unicode codepoint. Empty / cleared cells use U+0020.
    pub ch: u32,
    /// Foreground color kind: `0` = default, `1` = indexed palette
    /// (value in `fg_r`), `2` = RGB (`fg_r`/`fg_g`/`fg_b`).
    pub fg_kind: u8,
    /// Foreground red channel or palette index (see `fg_kind`).
    pub fg_r: u8,
    /// Foreground green channel (only meaningful when `fg_kind == 2`).
    pub fg_g: u8,
    /// Foreground blue channel (only meaningful when `fg_kind == 2`).
    pub fg_b: u8,
    /// Background color kind, same encoding as `fg_kind`.
    pub bg_kind: u8,
    /// Background red channel or palette index.
    pub bg_r: u8,
    /// Background green channel.
    pub bg_g: u8,
    /// Background blue channel.
    pub bg_b: u8,
    /// Bit 0 = bold, bit 1 = underline, bit 2 = reverse.
    pub attrs: u8,
    /// Padding so the total size is 16 bytes on every target.
    _padding: [u8; 3],
}

impl PierCell {
    const ATTR_BOLD: u8 = 0b0000_0001;
    const ATTR_UNDERLINE: u8 = 0b0000_0010;
    const ATTR_REVERSE: u8 = 0b0000_0100;

    fn from_cell(c: &Cell) -> Self {
        let (fg_kind, fg_r, fg_g, fg_b) = encode_color(c.fg);
        let (bg_kind, bg_r, bg_g, bg_b) = encode_color(c.bg);
        let mut attrs = 0u8;
        if c.bold {
            attrs |= Self::ATTR_BOLD;
        }
        if c.underline {
            attrs |= Self::ATTR_UNDERLINE;
        }
        if c.reverse {
            attrs |= Self::ATTR_REVERSE;
        }
        Self {
            ch: c.ch as u32,
            fg_kind,
            fg_r,
            fg_g,
            fg_b,
            bg_kind,
            bg_r,
            bg_g,
            bg_b,
            attrs,
            _padding: [0; 3],
        }
    }
}

fn encode_color(c: Color) -> (u8, u8, u8, u8) {
    match c {
        Color::Default => (0, 0, 0, 0),
        Color::Indexed(i) => (1, i, 0, 0),
        Color::Rgb(r, g, b) => (2, r, g, b),
    }
}

/// Grid metadata returned from [`pier_terminal_snapshot`].
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PierGridInfo {
    /// Columns at the time of the snapshot.
    pub cols: u16,
    /// Rows at the time of the snapshot.
    pub rows: u16,
    /// Cursor column, zero-based, `< cols`.
    pub cursor_x: u16,
    /// Cursor row, zero-based, `< rows`.
    pub cursor_y: u16,
    /// `1` if the backing process is still running, `0` if it has
    /// exited. Updated before the final `Exited` notify event fires.
    pub alive: u8,
    /// Padding so the overall struct is 16 bytes.
    _padding: [u8; 7],
}

// ─────────────────────────────────────────────────────────
// Functions
// ─────────────────────────────────────────────────────────

/// Spawn a new local terminal session.
///
/// `shell` must be a NUL-terminated UTF-8 path (e.g. `/bin/zsh`).
/// Returns `NULL` on failure — the caller can check
/// [`pier_terminal_last_error`] (TODO, lands with the first protocol
/// module that needs it). On success, the returned pointer must
/// eventually be passed to [`pier_terminal_free`].
///
/// The `notify` callback is invoked from the reader thread on any
/// `DataReady` or `Exited` event. See the module-level documentation
/// for the callback contract. `notify` must not be null; pass a
/// no-op function if you don't care about wakeups.
///
/// # Safety
///
/// `shell` must be a valid NUL-terminated C string. `notify` must be
/// a valid function pointer. `user_data` is opaque and is not
/// dereferenced by pier-core.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_new(
    cols: u16,
    rows: u16,
    shell: *const c_char,
    notify: Option<NotifyFn>,
    user_data: *mut c_void,
) -> *mut PierTerminal {
    if shell.is_null() {
        return ptr::null_mut();
    }
    let Some(notify) = notify else {
        return ptr::null_mut();
    };
    // SAFETY: caller guarantees a NUL-terminated UTF-8 C string.
    let shell_str = match unsafe { std::ffi::CStr::from_ptr(shell) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    match PierTerminal::new(cols, rows, shell_str, notify, user_data) {
        Ok(t) => Box::into_raw(Box::new(t)),
        Err(e) => {
            log::warn!("pier_terminal_new failed: {e}");
            ptr::null_mut()
        }
    }
}

/// Send bytes to the shell (keystrokes, paste, ...).
/// Returns the number of bytes accepted or a negative error code.
///
/// # Safety
///
/// `t` must be either null or a valid handle returned by
/// [`pier_terminal_new`] that has not yet been freed. `data` must
/// point at `len` readable bytes, or be null when `len == 0`.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_write(
    t: *mut PierTerminal,
    data: *const u8,
    len: usize,
) -> i64 {
    if t.is_null() {
        return -1;
    }
    if len > 0 && data.is_null() {
        return -1;
    }
    // SAFETY: non-null handle invariant checked; caller promises
    // the handle is still live.
    let term = unsafe { &*t };
    // SAFETY: non-null + len readable bytes per the contract above.
    let slice = if len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(data, len) }
    };
    match term.write(slice) {
        Ok(n) => n as i64,
        Err(_) => -3,
    }
}

/// Tell the terminal its visible area is now `cols × rows` cells.
///
/// # Safety
///
/// `t` must be either null or a valid, not-yet-freed handle.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_resize(
    t: *mut PierTerminal,
    cols: u16,
    rows: u16,
) -> i32 {
    if t.is_null() {
        return -1;
    }
    // SAFETY: non-null handle invariant checked.
    let term = unsafe { &mut *t };
    match term.resize(cols, rows) {
        Ok(()) => 0,
        Err(TerminalError::Unsupported) => -4,
        Err(_) => -3,
    }
}

/// Copy the current grid into the caller-provided buffer.
///
/// `out_info` receives cols/rows/cursor info + alive flag.
/// `out_cells` must have capacity for at least `out_cells_capacity`
/// [`PierCell`] entries, and must be at least `info.cols * info.rows`
/// in size after the call. If the buffer is too small, returns `-2`
/// and leaves the buffer untouched. `out_info` is still populated in
/// that case so the caller can allocate a larger buffer and retry.
///
/// # Safety
///
/// `t` must be null or a live handle. `out_info` and `out_cells`
/// must be non-null and point at writable memory of at least
/// `sizeof(PierGridInfo)` and `out_cells_capacity * sizeof(PierCell)`
/// bytes respectively.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_snapshot(
    t: *mut PierTerminal,
    out_info: *mut PierGridInfo,
    out_cells: *mut PierCell,
    out_cells_capacity: usize,
) -> i32 {
    if t.is_null() || out_info.is_null() {
        return -1;
    }
    // SAFETY: non-null handle, live by contract.
    let term = unsafe { &*t };
    let snap = term.snapshot();
    let needed = snap.cols as usize * snap.rows as usize;

    let info = PierGridInfo {
        cols: snap.cols,
        rows: snap.rows,
        cursor_x: snap.cursor_x,
        cursor_y: snap.cursor_y,
        alive: if term.is_alive() { 1 } else { 0 },
        _padding: [0; 7],
    };
    // SAFETY: caller contract: out_info is writable.
    unsafe { ptr::write(out_info, info) };

    if out_cells.is_null() {
        // Caller wanted just the metadata to size their buffer.
        return if out_cells_capacity == 0 { 0 } else { -1 };
    }
    if out_cells_capacity < needed {
        return -2;
    }

    // Convert and memcpy. We go cell-by-cell because PierCell is
    // a different layout from Cell. For a typical 120×40 grid this
    // is ~4800 iterations of trivial arithmetic — sub-microsecond.
    for (i, cell) in snap.cells.iter().enumerate() {
        let pc = PierCell::from_cell(cell);
        // SAFETY: i is bounded by needed ≤ out_cells_capacity,
        // and out_cells points at out_cells_capacity writable cells.
        unsafe { ptr::write(out_cells.add(i), pc) };
    }
    0
}

/// Returns `1` if the underlying child process is still running.
///
/// # Safety
///
/// `t` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_is_alive(t: *const PierTerminal) -> i32 {
    if t.is_null() {
        return 0;
    }
    // SAFETY: non-null handle, live by contract.
    let term = unsafe { &*t };
    if term.is_alive() {
        1
    } else {
        0
    }
}

/// Destroy a terminal session. Joins the reader thread and reaps
/// the child before returning. Safe to call with null.
///
/// # Safety
///
/// `t`, if non-null, must have been returned by
/// [`pier_terminal_new`] and not yet freed. After this call the
/// pointer is invalid.
#[no_mangle]
pub unsafe extern "C" fn pier_terminal_free(t: *mut PierTerminal) {
    if t.is_null() {
        return;
    }
    // SAFETY: caller contract — the handle was produced by
    // Box::into_raw in pier_terminal_new and has not yet been
    // freed. Box::from_raw reclaims ownership and its Drop runs
    // the full shutdown sequence.
    drop(unsafe { Box::from_raw(t) });
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    extern "C" fn test_notify(user_data: *mut c_void, _event: u32) {
        // SAFETY: tests always pass a leaked AtomicUsize.
        let counter = unsafe { &*(user_data as *const AtomicUsize) };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn wait_for<F: Fn() -> bool>(f: F, limit: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        f()
    }

    #[test]
    fn ffi_roundtrip_spawn_write_snapshot_free() {
        let counter: &'static AtomicUsize = Box::leak(Box::new(AtomicUsize::new(0)));
        let user_data = counter as *const AtomicUsize as *mut c_void;

        let shell = CString::new("/bin/cat").unwrap();
        // SAFETY: shell is a valid NUL-terminated C string,
        // test_notify is a valid function pointer, user_data
        // is the leaked counter.
        let t = unsafe {
            pier_terminal_new(80, 24, shell.as_ptr(), Some(test_notify), user_data)
        };
        assert!(!t.is_null(), "spawn should succeed on Unix");

        let msg = b"ffi-roundtrip\n";
        // SAFETY: t is a live handle, msg is a valid slice.
        let n = unsafe { pier_terminal_write(t, msg.as_ptr(), msg.len()) };
        assert_eq!(n, msg.len() as i64);

        assert!(
            wait_for(|| counter.load(Ordering::Relaxed) > 0, Duration::from_secs(2)),
            "notify callback should have fired after cat echoed our input",
        );

        // Snapshot with a sized buffer.
        let mut info = PierGridInfo {
            cols: 0,
            rows: 0,
            cursor_x: 0,
            cursor_y: 0,
            alive: 0,
            _padding: [0; 7],
        };
        let mut cells = vec![
            PierCell {
                ch: 0,
                fg_kind: 0,
                fg_r: 0,
                fg_g: 0,
                fg_b: 0,
                bg_kind: 0,
                bg_r: 0,
                bg_g: 0,
                bg_b: 0,
                attrs: 0,
                _padding: [0; 3],
            };
            80 * 24
        ];
        // SAFETY: info + cells are both caller-owned writable memory
        // sized for cols*rows = 80*24.
        let rc = unsafe {
            pier_terminal_snapshot(t, &mut info, cells.as_mut_ptr(), cells.len())
        };
        assert_eq!(rc, 0);
        assert_eq!(info.cols, 80);
        assert_eq!(info.rows, 24);
        assert_eq!(info.alive, 1);

        // Walk the grid and look for our needle.
        let mut text = String::new();
        for r in 0..info.rows as usize {
            for c in 0..info.cols as usize {
                let ch = char::from_u32(cells[r * info.cols as usize + c].ch).unwrap_or(' ');
                text.push(ch);
            }
            text.push('\n');
        }
        assert!(
            text.contains("ffi-roundtrip"),
            "grid did not contain echoed text:\n{text}",
        );

        // Clean up.
        // SAFETY: handle is still live and was produced by
        // pier_terminal_new in this test.
        unsafe { pier_terminal_free(t) };
    }

    #[test]
    fn snapshot_rejects_undersized_buffer() {
        let counter: &'static AtomicUsize = Box::leak(Box::new(AtomicUsize::new(0)));
        let user_data = counter as *const AtomicUsize as *mut c_void;
        let shell = CString::new("/bin/cat").unwrap();
        // SAFETY: see above test.
        let t = unsafe {
            pier_terminal_new(80, 24, shell.as_ptr(), Some(test_notify), user_data)
        };
        assert!(!t.is_null());

        let mut info = PierGridInfo {
            cols: 0,
            rows: 0,
            cursor_x: 0,
            cursor_y: 0,
            alive: 0,
            _padding: [0; 7],
        };
        // Deliberately too small.
        let mut cells = vec![
            PierCell {
                ch: 0,
                fg_kind: 0,
                fg_r: 0,
                fg_g: 0,
                fg_b: 0,
                bg_kind: 0,
                bg_r: 0,
                bg_g: 0,
                bg_b: 0,
                attrs: 0,
                _padding: [0; 3],
            };
            10
        ];
        // SAFETY: info + cells are writable.
        let rc = unsafe {
            pier_terminal_snapshot(t, &mut info, cells.as_mut_ptr(), cells.len())
        };
        assert_eq!(rc, -2);
        // info should still be populated so the caller can retry.
        assert_eq!(info.cols, 80);
        assert_eq!(info.rows, 24);

        // SAFETY: still-live handle.
        unsafe { pier_terminal_free(t) };
    }

    #[test]
    fn null_handle_is_safe_everywhere() {
        // SAFETY: all these take null; each function is defined to
        // return an error code without touching memory.
        unsafe {
            assert_eq!(pier_terminal_write(ptr::null_mut(), ptr::null(), 0), -1);
            assert_eq!(pier_terminal_resize(ptr::null_mut(), 80, 24), -1);
            assert_eq!(pier_terminal_is_alive(ptr::null()), 0);
            pier_terminal_free(ptr::null_mut()); // no-op
            let mut info = PierGridInfo {
                cols: 0,
                rows: 0,
                cursor_x: 0,
                cursor_y: 0,
                alive: 0,
                _padding: [0; 7],
            };
            assert_eq!(
                pier_terminal_snapshot(ptr::null_mut(), &mut info, ptr::null_mut(), 0),
                -1,
            );
        }
    }
}
