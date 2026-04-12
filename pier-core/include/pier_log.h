/*
 * pier-core — Streaming log viewer C ABI
 * ──────────────────────────────────────
 *
 * M5b per-service tool. Thin wrapper around pier-core's
 * streaming SSH exec (ssh::ExecStream): spawn a long-running
 * remote command like `tail -f /var/log/syslog` or
 * `docker logs -f <id>` and pull its stdout / stderr / exit
 * events line-by-line from the UI thread.
 *
 * Threading: pier_log_open is synchronous and blocking (runs
 * the full SSH handshake). Once it returns, an internal
 * producer task on the shared tokio runtime reads channel
 * data, frames it into lines, and queues the events. The C++
 * side polls pier_log_drain from a Qt timer and appends new
 * rows to its model. Zero work on the main thread between
 * polls — the drain itself is a non-blocking std::mpsc
 * try_recv loop.
 *
 * Auth kind discriminator — same table as every other
 * session-based FFI (pier_sftp.h, pier_tunnel.h, etc):
 *
 *   PIER_AUTH_PASSWORD   (0) — secret = plaintext password
 *   PIER_AUTH_CREDENTIAL (1) — secret = keychain credential id
 *   PIER_AUTH_KEY        (2) — secret = private key file path,
 *                               extra  = passphrase credential id
 *                                        (or NULL for unencrypted)
 *   PIER_AUTH_AGENT      (3) — both secret and extra ignored
 *
 * Memory ownership:
 *   * PierLogStream * must be released via pier_log_free.
 *   * JSON strings returned by pier_log_drain are owned by
 *     Rust and must be released via pier_log_free_string.
 */

#ifndef PIER_LOG_H
#define PIER_LOG_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle. */
typedef struct PierLogStream PierLogStream;

/* Open a new SSH session, spawn `command` on the remote, and
 * return a handle to the resulting streaming output. Blocking.
 * Returns NULL on any failure (invalid arg, connect failed,
 * auth rejected, exec refused). */
PierLogStream *pier_log_open(
    const char *host,
    uint16_t port,
    const char *user,
    int32_t auth_kind,
    const char *secret,    /* NULL allowed for AUTH_AGENT */
    const char *extra,     /* NULL unless AUTH_KEY passphrase */
    const char *command
);

/* M3e: spawn a streaming remote command on an existing
 * shared SSH session (see pier_ssh_session.h). No auth
 * parameters — the session is pre-authenticated. Same
 * drain / free / stop semantics as pier_log_open. */
struct PierSshSession;
PierLogStream *pier_log_open_on_session(
    const struct PierSshSession *session,
    const char *command
);

/* Drain every event currently buffered. Returns a heap JSON
 * array of events, or NULL if no events are pending.
 *
 * Each event is an object of shape:
 *   { "kind": "stdout" | "stderr" | "exit" | "error",
 *     "text": string | null,
 *     "exit_code": int | null,
 *     "error": string | null }
 *
 * Release the returned string with pier_log_free_string.
 * NULL is NOT an error — check pier_log_is_alive for that. */
char *pier_log_drain(PierLogStream *h);

/* Return 1 if the remote process is still running, 0 otherwise
 * (channel closed, exit observed, or NULL handle). */
int32_t pier_log_is_alive(const PierLogStream *h);

/* Last reported exit code, or -1 if the remote process hasn't
 * exited (or didn't report one). */
int32_t pier_log_exit_code(const PierLogStream *h);

/* Ask the remote process to stop. Best-effort: we close the
 * local end of the SSH channel on the next producer iteration,
 * which makes the remote see SIGPIPE on its next write. Safe
 * to call more than once; safe on a NULL handle. */
void pier_log_stop(PierLogStream *h);

/* Release a JSON string returned by pier_log_drain. Safe on
 * NULL. Do NOT use C free() — the Rust allocator owns it. */
void pier_log_free_string(char *s);

/* Release a log-stream handle. Safe on NULL. After this call
 * the handle is invalid. */
void pier_log_free(PierLogStream *h);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_LOG_H */
