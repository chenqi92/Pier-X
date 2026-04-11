/*
 * pier-core — credentials C ABI
 * ─────────────────────────────
 *
 * Write-only access from C++ to the OS keyring (Keychain on
 * macOS, Credential Manager on Windows, Secret Service on
 * Linux). All entries live under the service name
 * `com.kkape.pier-x` and are keyed by a caller-provided
 * `id` string.
 *
 * Read is deliberately NOT exposed. The plaintext password
 * collected by the New Connection dialog crosses this header
 * exactly once (into pier_credential_set) and is then owned
 * by the OS. The Rust SSH session layer pulls it back out
 * via crate::credentials::get from inside the handshake
 * task, which has no symbols visible to C++ code. This means
 * a runtime compromise of the C++/QML half of pier-x can
 * write arbitrary credentials but cannot exfiltrate the ones
 * that are already stored.
 *
 * Error codes
 * ───────────
 *     0   success
 *    -1   null pointer / empty id
 *    -2   id or value is not valid UTF-8
 *    -3   OS keyring rejected or could not service the call
 */

#ifndef PIER_CREDENTIALS_H
#define PIER_CREDENTIALS_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Store `value` under `id` in the OS keyring. Overwrites any
 * existing entry under the same id. Both pointers must be valid
 * NUL-terminated UTF-8 C strings; `id` must be non-empty. */
int32_t pier_credential_set(const char *id, const char *value);

/* Delete the entry stored under `id` in the OS keyring. A
 * missing entry is treated as success. `id` must be a valid
 * NUL-terminated UTF-8 C string. */
int32_t pier_credential_delete(const char *id);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_CREDENTIALS_H */
