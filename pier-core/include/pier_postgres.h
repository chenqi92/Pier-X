/*
 * pier-core — PostgreSQL client C ABI (M7a)
 * ─────────────────────────────────────────
 *
 * Mirrors pier_mysql.h byte-for-byte in API shape. The C++
 * and QML layers can reuse the same result-model code for
 * both MySQL and PostgreSQL because the JSON schemas are
 * identical.
 *
 * Threading: every function is synchronous and blocking.
 * Auth: user/password only (tunnel-encrypted transport).
 *
 * Memory ownership:
 *   * PierPostgres * → pier_postgres_free
 *   * JSON strings  → pier_postgres_free_string
 */

#ifndef PIER_POSTGRES_H
#define PIER_POSTGRES_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PierPostgres PierPostgres;

PierPostgres *pier_postgres_open(
    const char *host,
    uint16_t port,
    const char *user,
    const char *password,
    const char *database
);

void pier_postgres_free(PierPostgres *h);
void pier_postgres_free_string(char *s);

/* Same QueryResult JSON shape as pier_mysql_execute. */
char *pier_postgres_execute(PierPostgres *h, const char *sql);

/* JSON array of database names (templates filtered). */
char *pier_postgres_list_databases(PierPostgres *h);

/* JSON array of table names. schema NULL or "" → "public". */
char *pier_postgres_list_tables(PierPostgres *h, const char *schema);

/* JSON array of ColumnInfo objects — same shape as
 * pier_mysql_list_columns. */
char *pier_postgres_list_columns(PierPostgres *h,
                                  const char *schema,
                                  const char *table);

#ifdef __cplusplus
}
#endif

#endif /* PIER_POSTGRES_H */
