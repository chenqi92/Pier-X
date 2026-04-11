/*
 * pier-core — SFTP C ABI
 * ──────────────────────
 *
 * File-oriented API on top of a fresh SSH session. One opaque
 * `PierSftp *` per file-browser panel; each handle owns its
 * own SshSession + SFTP channel.
 *
 * Threading: every function is synchronous and blocking. The
 * C++ wrapper runs them on a worker thread and posts results
 * back via QMetaObject::invokeMethod, matching the async
 * pattern used for `pier_terminal_new_ssh_*`.
 *
 * Auth kind discriminator — the `auth_kind` argument to
 * pier_sftp_new selects which secret shape the function
 * expects. `extra` is only used for PIER_AUTH_KEY.
 *
 *   PIER_AUTH_PASSWORD   (0) — secret = plaintext password
 *   PIER_AUTH_CREDENTIAL (1) — secret = keychain credential id
 *   PIER_AUTH_KEY        (2) — secret = private key file path,
 *                               extra  = passphrase credential id
 *                                        (or NULL for unencrypted)
 *   PIER_AUTH_AGENT      (3) — both secret and extra ignored
 *
 * Memory ownership:
 *   * `PierSftp *` must be released via pier_sftp_free.
 *   * Strings returned by list_dir / canonicalize are owned
 *     by Rust and must be released via pier_sftp_free_string.
 *     Do NOT call C free() on them — the allocator differs.
 *
 * Error codes (for functions that return int32_t):
 *      0   success
 *     -1   null handle / null required string
 *     -2   non-UTF-8 input
 *     -3   I/O or protocol error
 *     -4   unknown auth_kind
 */

#ifndef PIER_SFTP_H
#define PIER_SFTP_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle. */
typedef struct PierSftp PierSftp;

/* Auth kind constants — mirror pier_core::ffi::sftp. */
#define PIER_AUTH_PASSWORD    0
#define PIER_AUTH_CREDENTIAL  1
#define PIER_AUTH_KEY         2
#define PIER_AUTH_AGENT       3

/* Spawn a new SFTP session. Returns NULL on any failure
 * (invalid arg, connect failed, auth rejected, channel open
 * refused). See file header for auth_kind semantics. */
PierSftp *pier_sftp_new(
    const char *host,
    uint16_t port,
    const char *user,
    int32_t auth_kind,
    const char *secret,  /* NULL allowed for AUTH_AGENT */
    const char *extra    /* NULL unless AUTH_KEY passphrase */
);

/* Release an SFTP handle. Safe to call with NULL. */
void pier_sftp_free(PierSftp *t);

/* Release a heap-allocated C string returned by
 * pier_sftp_list_dir or pier_sftp_canonicalize.
 * Safe to call with NULL. */
void pier_sftp_free_string(char *s);

/* List the contents of `path`. Returns a JSON array of
 * RemoteFileEntry objects as a heap-allocated NUL-terminated
 * UTF-8 string, or NULL on error. Release with
 * pier_sftp_free_string.
 *
 * Each entry has fields:
 *   { "name":  string,
 *     "path":  string,
 *     "is_dir": bool,
 *     "is_link": bool,
 *     "size":  int,
 *     "modified": int | null,
 *     "permissions": int | null } */
char *pier_sftp_list_dir(PierSftp *t, const char *path);

/* Canonicalize `path` on the remote (resolves relative
 * paths and symlinks). Useful for "pwd" at startup: pass ".".
 * Returns a heap string, release with pier_sftp_free_string. */
char *pier_sftp_canonicalize(PierSftp *t, const char *path);

/* Create a directory. Non-recursive — parent must exist. */
int32_t pier_sftp_mkdir(PierSftp *t, const char *path);

/* Remove a regular file. */
int32_t pier_sftp_remove_file(PierSftp *t, const char *path);

/* Remove an empty directory. */
int32_t pier_sftp_remove_dir(PierSftp *t, const char *path);

/* Rename (or move) `from` to `to`. */
int32_t pier_sftp_rename(PierSftp *t, const char *from, const char *to);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_SFTP_H */
