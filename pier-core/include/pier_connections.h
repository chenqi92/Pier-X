/*
 * pier-core — connections store C ABI
 * ───────────────────────────────────
 *
 * Persisted SSH connection list, stored as JSON in the
 * platform's data directory:
 *
 *   macOS:   ~/Library/Application Support/com.kkape.pier-x/connections.json
 *   Windows: %APPDATA%\kkape\pier-x\connections.json
 *   Linux:   ~/.local/share/pier-x/connections.json
 *
 * The C++ side never touches the file directly. It serializes
 * its in-memory list to JSON, hands it to
 * pier_connections_save_json, and on startup gets the persisted
 * list back via pier_connections_load_json.
 *
 * The persisted JSON contains no plaintext secrets — only
 * opaque credential ids that the SSH layer looks up against
 * the OS keychain at handshake time. See pier_credentials.h.
 *
 * JSON shape (schema version 1)
 * ─────────────────────────────
 * {
 *   "version": 1,
 *   "connections": [
 *     {
 *       "name": "prod",
 *       "host": "db.example.com",
 *       "port": 22,
 *       "user": "deploy",
 *       "auth": { "kind": "keychain_password",
 *                 "credential_id": "pier-x.0d3a..." },
 *       "connect_timeout_secs": 10,
 *       "tags": []
 *     }
 *   ]
 * }
 *
 * Memory contract for load
 * ────────────────────────
 * pier_connections_load_json returns a heap-allocated string
 * owned by Rust. The caller MUST release it via
 * pier_connections_free_json (NOT C `free`, which uses a
 * different allocator).
 *
 * Error codes for save
 * ────────────────────
 *     0   success
 *    -1   null json pointer
 *    -2   json is not valid UTF-8
 *    -3   json failed to parse into a ConnectionStore
 *    -4   I/O error writing the file
 *    -5   no usable application data directory
 */

#ifndef PIER_CONNECTIONS_H
#define PIER_CONNECTIONS_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Load the persisted connections store and return its JSON
 * representation as an owned NUL-terminated UTF-8 C string.
 * Returns NULL if the data directory cannot be resolved or the
 * file is malformed. A missing-but-otherwise-fine file produces
 * the JSON for an empty store, never NULL.
 *
 * The caller owns the returned pointer and MUST release it
 * via pier_connections_free_json. */
char *pier_connections_load_json(void);

/* Release a buffer previously returned by pier_connections_load_json.
 * Safe to call with NULL. */
void pier_connections_free_json(char *json);

/* Persist a JSON-serialized ConnectionStore to disk atomically.
 * `json` must be a valid NUL-terminated UTF-8 C string holding
 * a ConnectionStore document. Returns 0 on success or a
 * negative error code (see file header). */
int32_t pier_connections_save_json(const char *json);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_CONNECTIONS_H */
