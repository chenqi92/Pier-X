/*
 * pier-core — Redis browser C ABI
 * ───────────────────────────────
 *
 * M5a per-service tool. Thin wrapper around
 * `pier_core::services::redis::RedisClient`, exposing the
 * minimum surface the QML browser panel needs: open, ping,
 * scan keys, inspect a single key, and INFO.
 *
 * Threading: every function is synchronous and blocking. The
 * C++ wrapper runs them on a worker thread and posts results
 * back via QMetaObject::invokeMethod, matching the pattern used
 * by PierSftp / PierServiceDetector / PierTunnel.
 *
 * No auth (yet). M5a targets local-forward tunneled Redis —
 * the typical endpoint is 127.0.0.1:16379 with no AUTH set.
 * A separate `pier_redis_open_auth` can be added in M5b when
 * remote-direct connections with AUTH land.
 *
 * Memory ownership:
 *   * `PierRedis *` must be released via pier_redis_free.
 *   * JSON strings returned by pier_redis_scan_keys /
 *     pier_redis_inspect / pier_redis_info are owned by Rust
 *     and must be released via pier_redis_free_string. Do NOT
 *     call C free() on them — the allocator differs.
 */

#ifndef PIER_REDIS_H
#define PIER_REDIS_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle. */
typedef struct PierRedis PierRedis;

/* Open a new Redis connection to `host:port` at database `db`.
 * Performs the TCP handshake + a RESP PING synchronously and
 * returns NULL on any failure. */
PierRedis *pier_redis_open(
    const char *host,
    uint16_t port,
    int64_t db
);

/* Release a Redis handle. Safe to call with NULL. */
void pier_redis_free(PierRedis *h);

/* Release a heap-allocated JSON string returned by
 * pier_redis_scan_keys / pier_redis_inspect / pier_redis_info.
 * Safe to call with NULL. */
void pier_redis_free_string(char *s);

/* Round-trip PING. Returns 1 on success, 0 otherwise. */
int32_t pier_redis_ping(PierRedis *h);

/* SCAN the keyspace for keys matching `pattern`. Returns a
 * heap JSON string of shape:
 *
 *   { "keys": [string...], "truncated": bool, "limit": int }
 *
 * `limit` is clamped to an internal maximum (see
 * DEFAULT_SCAN_LIMIT in the Rust module — currently 1000).
 * Returns NULL on failure. Release with pier_redis_free_string. */
char *pier_redis_scan_keys(PierRedis *h, const char *pattern, size_t limit);

/* Inspect a single key. Returns a heap JSON string of shape:
 *
 *   { "key": string,
 *     "kind": "string" | "list" | "set" | "zset" | "hash"
 *             | "stream" | "none",
 *     "length": int,
 *     "ttl_seconds": int,        // -1 no TTL, -2 missing
 *     "encoding": string,
 *     "preview": [string, ...],
 *     "preview_truncated": bool }
 *
 * The preview is bounded (32 items or ~1 KB for strings) so
 * this is safe to call on multi-GB keys. Returns NULL on
 * failure. Release with pier_redis_free_string. */
char *pier_redis_inspect(PierRedis *h, const char *key);

/* Run `INFO <section>` and return the parsed `k: v` map as a
 * JSON object. Pass NULL or "" for `section` to request all
 * sections. Returns NULL on failure. Release with
 * pier_redis_free_string. */
char *pier_redis_info(PierRedis *h, const char *section);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_REDIS_H */
