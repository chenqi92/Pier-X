/*
 * pier-core — MySQL client C ABI
 * ──────────────────────────────
 *
 * M5d per-service tool. Thin wrapper around
 * `pier_core::services::mysql::MysqlClient` — every operation
 * runs against a single long-lived connection pool behind
 * the opaque PierMysql handle.
 *
 * Threading: every function is synchronous and blocking. The
 * C++ wrapper runs them on a worker thread and posts results
 * back via QMetaObject::invokeMethod, matching the pattern
 * used by PierRedis / PierLogStream / PierDocker.
 *
 * Auth: only user/password auth for M5d. Pier-X connects via
 * an SSH tunnel, so TLS isn't required. Password may be NULL
 * (or empty) for passwordless dev installs.
 *
 * Memory ownership:
 *   * PierMysql * must be released via pier_mysql_free.
 *   * JSON strings returned by pier_mysql_execute /
 *     pier_mysql_list_* are owned by Rust and must be
 *     released via pier_mysql_free_string. Do NOT use C
 *     free().
 */

#ifndef PIER_MYSQL_H
#define PIER_MYSQL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle. */
typedef struct PierMysql PierMysql;

/* Open a MySQL connection to `host:port` authenticating as
 * `user` / `password`. `database` may be NULL or empty for
 * "no default database" (the caller can USE one later via a
 * query). Performs a full handshake + auth + `SELECT 1`
 * probe synchronously. Returns NULL on any failure. */
PierMysql *pier_mysql_open(
    const char *host,
    uint16_t port,
    const char *user,
    const char *password,   /* NULL or empty = no password */
    const char *database    /* NULL or empty = no default DB */
);

/* Release a MySQL handle. Safe to call with NULL. */
void pier_mysql_free(PierMysql *h);

/* Release a JSON string returned by pier_mysql_execute /
 * pier_mysql_list_*. Safe to call with NULL. */
void pier_mysql_free_string(char *s);

/* Execute a single SQL statement and return a JSON object
 * shaped like:
 *
 *   { "columns":       [string, ...],          // SELECT only
 *     "rows":          [[string|null, ...], ...],
 *     "truncated":     bool,                   // row cap hit?
 *     "affected_rows": int,                    // DML only
 *     "last_insert_id": int | null,
 *     "elapsed_ms":    int,
 *     "error":         string (only on failure) }
 *
 * A successful SELECT returns columns + rows. A successful
 * DML returns affected_rows + last_insert_id (columns / rows
 * empty). A server-side error returns the same shape with
 * the "error" field populated.
 *
 * Returns NULL only on catastrophic serialization failure;
 * every other failure path is carried inside the JSON.
 *
 * Release the returned string with pier_mysql_free_string. */
char *pier_mysql_execute(PierMysql *h, const char *sql);

/* `SHOW DATABASES` with internal schemas (information_schema,
 * performance_schema, mysql, sys) filtered out. Returns a
 * heap JSON array of strings, or NULL on failure. Release
 * with pier_mysql_free_string. */
char *pier_mysql_list_databases(PierMysql *h);

/* `SHOW TABLES FROM <database>`. `database` is validated
 * against a strict identifier allowlist
 * (`^[A-Za-z0-9_$]{1,64}$`) before being interpolated into
 * the SQL. Returns a heap JSON array of table names, or
 * NULL on failure. */
char *pier_mysql_list_tables(PierMysql *h, const char *database);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_MYSQL_H */
