#ifndef PIER_SQLITE_H
#define PIER_SQLITE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PierSqlite PierSqlite;

PierSqlite *pier_sqlite_open(const char *path);
void pier_sqlite_free(PierSqlite *h);
void pier_sqlite_free_string(char *s);

/* JSON array of table names. Caller frees. */
char *pier_sqlite_list_tables(PierSqlite *h);

/* JSON array of column info. Caller frees. */
char *pier_sqlite_table_columns(PierSqlite *h, const char *table);

/* Execute SQL. Returns JSON QueryResult. Caller frees. */
char *pier_sqlite_execute(PierSqlite *h, const char *sql);

#ifdef __cplusplus
}
#endif

#endif
