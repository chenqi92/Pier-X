/*
 * pier-core — Shared SSH session C ABI (M3e)
 * ──────────────────────────────────────────
 *
 * Background: every session-based FFI pier-core shipped
 * through M3-M5 (pier_sftp_new, pier_services_detect,
 * pier_tunnel_open, pier_log_open, pier_docker_open,
 * pier_terminal_new_ssh_*) opens its own SSH connection
 * under the hood. On a typical workflow that means four
 * separate handshakes + four TCP connections on the server —
 * wasteful, and noisy in server auth logs.
 *
 * M3e adds a shared PierSshSession opaque handle that any
 * consumer FFI can borrow via a new `*_on_session`
 * constructor. The flow becomes:
 *
 *   1. Open one session per host via pier_ssh_session_open.
 *   2. Pass the resulting handle into any number of
 *      consumer FFIs via their `_on_session` variants:
 *        - pier_sftp_new_on_session
 *        - pier_tunnel_open_on_session
 *        - pier_services_detect_on_session
 *        - pier_log_open_on_session
 *        - pier_docker_open_on_session
 *        - pier_terminal_new_ssh_on_session
 *   3. Free the panels independently as the user closes
 *      tabs; free the session handle when no panels remain
 *      for that host. The underlying russh connection
 *      actually closes once the last clone drops — child
 *      handles hold their own clone, so the master can
 *      outlive or be outlived by children in any order.
 *
 * The existing constructors that take host/port/user/auth
 * continue to work unchanged. The C++ side can migrate
 * panel-by-panel at its own pace.
 *
 * Threading: pier_ssh_session_open is synchronous and
 * blocking (full handshake + auth, ~300 ms on a LAN). Run it
 * on a worker thread and post the result back via
 * QMetaObject::invokeMethod, same convention as every other
 * session-based FFI.
 *
 * Memory ownership:
 *   * PierSshSession * must be released via
 *     pier_ssh_session_free.
 *   * pier_ssh_session_last_error returns a borrowed
 *     pointer into thread-local storage that is valid until
 *     the next call into pier_ssh_session_* on the same
 *     thread. Copy the string if you need it to persist.
 */

#ifndef PIER_SSH_SESSION_H
#define PIER_SSH_SESSION_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle. */
typedef struct PierSshSession PierSshSession;

/* Error-kind constants for pier_ssh_session_last_error_kind.
 * Stable integer codes — the C++ side maps these to user-
 * facing strings. Keep in sync with the Rust module. */
#define PIER_SSH_ERR_OK           0
#define PIER_SSH_ERR_INVALID_ARG  1
#define PIER_SSH_ERR_CONNECT      2
#define PIER_SSH_ERR_AUTH         3
#define PIER_SSH_ERR_HOST_KEY     4
#define PIER_SSH_ERR_PROTOCOL     5
#define PIER_SSH_ERR_UNKNOWN      6

/* Open a shared SSH session. Performs the full handshake +
 * authentication synchronously on the calling thread and
 * returns an opaque handle, or NULL on failure. On NULL
 * return the caller may fetch details via
 * pier_ssh_session_last_error / last_error_kind on the same
 * thread.
 *
 * Auth kind / secret / extra follow the same table as
 * pier_sftp_new (see pier_sftp.h):
 *
 *   PIER_AUTH_PASSWORD   (0) — secret = plaintext password
 *   PIER_AUTH_CREDENTIAL (1) — secret = keychain credential id
 *   PIER_AUTH_KEY        (2) — secret = private key file path,
 *                               extra  = passphrase credential id
 *                                        (or NULL for unencrypted)
 *   PIER_AUTH_AGENT      (3) — both secret and extra ignored */
PierSshSession *pier_ssh_session_open(
    const char *host,
    uint16_t port,
    const char *user,
    int32_t auth_kind,
    const char *secret,
    const char *extra
);

/* Release a shared SSH session handle. Safe on NULL. The
 * underlying russh connection may outlive this call if any
 * child handle produced via `_on_session` is still alive —
 * each child holds its own clone of the session. */
void pier_ssh_session_free(PierSshSession *h);

/* Returns 1 if the session's internal russh handle has at
 * least one live strong reference, 0 otherwise (or on NULL).
 * This is a refcount-based liveness hint, not a wire ping —
 * a silently-dropped TCP connection still reports alive
 * until the next operation fails. */
int32_t pier_ssh_session_is_alive(const PierSshSession *h);

/* Number of strong references currently held on the
 * underlying russh handle. Useful for debugging panel
 * sharing: after binding N panels to one session via
 * _on_session, this should report N+1 (master + N clones). */
int32_t pier_ssh_session_refcount(const PierSshSession *h);

/* Fetch the last-error message set by a failing
 * pier_ssh_session_open on the current thread. Returns a
 * borrowed const char * pointing into thread-local storage;
 * valid until the next pier_ssh_session_* call on the same
 * thread. Returns NULL if no error has been recorded. */
const char *pier_ssh_session_last_error(void);

/* Fetch the last-error category. See the PIER_SSH_ERR_*
 * constants above. */
int32_t pier_ssh_session_last_error_kind(void);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_SSH_SESSION_H */
