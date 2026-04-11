/*
 * pier-core — terminal C ABI
 * ──────────────────────────
 *
 * Handle-based wrapper around pier_core::terminal::PierTerminal. Lets
 * a C or C++ consumer (currently: pier-ui-qt's PierTerminalSession
 * QObject) spawn a local shell, write keystrokes to it, receive
 * notify wakeups when something changed, and snapshot the current
 * grid into a caller-allocated buffer.
 *
 * Threading contract
 * ──────────────────
 *   * pier_terminal_new spawns a dedicated reader thread inside
 *     pier-core. That thread loops on pty read + VT emulate and
 *     invokes your `notify` function on any change. The callback
 *     runs on the READER THREAD, not the UI thread, and must be
 *     quick and thread-safe. The canonical Qt body is:
 *
 *         QMetaObject::invokeMethod(self,
 *             "onCoreNotify", Qt::QueuedConnection,
 *             Q_ARG(int, (int)event));
 *
 *   * pier_terminal_write / pier_terminal_resize / pier_terminal_snapshot
 *     are safe to call from any thread; internally they take a short
 *     mutex.
 *
 *   * pier_terminal_free joins the reader thread and reaps the child
 *     before returning. Do NOT call it from inside the notify
 *     callback itself (it would deadlock on join).
 *
 * Error codes
 * ───────────
 *     0   success
 *    -1   null handle / null out pointer
 *    -2   caller's snapshot buffer is too small (info is still set)
 *    -3   underlying I/O error (write / resize failed)
 *    -4   platform does not yet have this backend (currently: Windows)
 *
 * Memory contract
 * ───────────────
 *   * The opaque `PierTerminal *` handle is owned by the caller from
 *     the moment pier_terminal_new returns until pier_terminal_free
 *     is called. Double-free is undefined behavior.
 *   * PierCell / PierGridInfo buffers passed to _snapshot are
 *     caller-allocated and caller-freed. pier-core never retains
 *     pointers to them beyond the duration of the call.
 *   * `shell` passed to pier_terminal_new is copied into Rust before
 *     the reader thread starts; callers may free it immediately.
 *   * `user_data` is opaque to pier-core and is not dereferenced
 *     here — we only pass it back to your notify function.
 */

#ifndef PIER_TERMINAL_H
#define PIER_TERMINAL_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle to a live terminal session. Returned by
 * pier_terminal_new, released by pier_terminal_free. */
typedef struct PierTerminal PierTerminal;

/* Notify event kinds. Must match pier_core::terminal::NotifyEvent. */
typedef enum PierNotifyEvent {
    PIER_NOTIFY_DATA_READY = 0,
    PIER_NOTIFY_EXITED     = 1
} PierNotifyEvent;

/* Notify callback. Runs on the reader thread. Keep it short. */
typedef void (*PierTerminalNotifyFn)(void *user_data, uint32_t event);

/* A single cell in the terminal grid. Stable 16-byte layout. */
typedef struct PierCell {
    uint32_t ch;       /* unicode codepoint                                  */
    uint8_t  fg_kind;  /* 0 = default, 1 = palette index in fg_r, 2 = RGB    */
    uint8_t  fg_r;
    uint8_t  fg_g;
    uint8_t  fg_b;
    uint8_t  bg_kind;
    uint8_t  bg_r;
    uint8_t  bg_g;
    uint8_t  bg_b;
    uint8_t  attrs;    /* bit 0 = bold, bit 1 = underline, bit 2 = reverse   */
    uint8_t  _pad[3];
} PierCell;

/* Grid metadata. Stable 16-byte layout. */
typedef struct PierGridInfo {
    uint16_t cols;
    uint16_t rows;
    uint16_t cursor_x;
    uint16_t cursor_y;
    uint8_t  alive;    /* 1 = child still running, 0 = exited                */
    uint8_t  _pad[7];
} PierGridInfo;

/* ── Functions ─────────────────────────────────────────── */

/* Spawn a new local terminal session running `shell`.
 * Returns NULL on failure (e.g. unsupported platform, fork error,
 * null shell). */
PierTerminal *pier_terminal_new(
    uint16_t cols,
    uint16_t rows,
    const char *shell,
    PierTerminalNotifyFn notify,
    void *user_data
);

/* Send bytes to the shell. Returns bytes written (≥ 0) or a
 * negative error code. */
int64_t pier_terminal_write(
    PierTerminal *t,
    const uint8_t *data,
    size_t len
);

/* Tell the shell its visible area is now cols × rows cells. */
int32_t pier_terminal_resize(PierTerminal *t, uint16_t cols, uint16_t rows);

/* Copy the current grid into caller-allocated out_cells. out_info
 * is always populated on success or -2 (buffer too small). Pass
 * out_cells = NULL and out_cells_capacity = 0 to only read info. */
int32_t pier_terminal_snapshot(
    PierTerminal *t,
    PierGridInfo *out_info,
    PierCell *out_cells,
    size_t out_cells_capacity
);

/* Returns 1 if the child is still running, 0 otherwise. */
int32_t pier_terminal_is_alive(const PierTerminal *t);

/* Joins the reader thread, reaps the child, releases the handle.
 * Safe to call with NULL. After this call `t` is invalid. */
void pier_terminal_free(PierTerminal *t);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_TERMINAL_H */
