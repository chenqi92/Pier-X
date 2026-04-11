/*
 * pier-core — remote service discovery C ABI
 * ───────────────────────────────────────────
 *
 * One entry point: pier_services_detect runs four concurrent
 * probes (MySQL / Redis / PostgreSQL / Docker) on a freshly-
 * opened SSH session and returns the results as a JSON array.
 *
 * JSON shape
 * ──────────
 *   [
 *     { "name": "mysql",      "version": "8.0.35",
 *       "status": "running",  "port": 3306 },
 *     { "name": "redis",      "version": "7.0.11",
 *       "status": "stopped",  "port": 6379 },
 *     ...
 *   ]
 *
 *   status is one of "running", "stopped", "installed".
 *   port is 0 for services that don't expose a TCP port
 *   (e.g. docker uses a Unix socket).
 *
 * Threading / blocking
 * ────────────────────
 * Blocking — typical LAN detection runs 0.5-1.5 s. Call from
 * a worker thread, post the result to the main thread via
 * QMetaObject::invokeMethod(Qt::QueuedConnection).
 *
 * Auth kind
 * ─────────
 * Same discriminator + secret/extra table as pier_sftp.h.
 * See PIER_AUTH_PASSWORD / PIER_AUTH_CREDENTIAL /
 * PIER_AUTH_KEY / PIER_AUTH_AGENT.
 *
 * Memory ownership
 * ────────────────
 * The returned string is heap-allocated by Rust and must be
 * released via pier_services_free_json. Do NOT call C free().
 */

#ifndef PIER_SERVICES_H
#define PIER_SERVICES_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Detect services on the remote host. Returns a heap-allocated
 * NUL-terminated UTF-8 JSON string, or NULL on any failure. */
char *pier_services_detect(
    const char *host,
    uint16_t port,
    const char *user,
    int32_t auth_kind,
    const char *secret,  /* NULL allowed for AUTH_AGENT */
    const char *extra    /* NULL unless AUTH_KEY passphrase */
);

/* Release a JSON string returned by pier_services_detect.
 * Safe to call with NULL. */
void pier_services_free_json(char *s);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_SERVICES_H */
