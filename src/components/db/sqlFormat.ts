// ── SQL formatter wrapper ────────────────────────────────────────
//
// Thin shim around the `sql-formatter` package so the panels share
// one place to pin dialect strings + format options. Each panel
// invokes `formatSqlText(sql, "mysql" | "postgresql" | …)` and
// gets back a reformatted string. The package itself does the
// heavy lifting; this file exists so a future swap of the
// formatter (or per-dialect option overrides) doesn't ripple
// across every panel.

import { format as sqlFormatterFormat } from "sql-formatter";

/** Dialect names that map to `sql-formatter`'s `language` option.
 *  Centralising the strings here keeps panel code from carrying
 *  its own copy of the magic discriminator. */
export type SqlDialect = "mysql" | "postgresql" | "sqlite" | "sql";

/** Format `sql` using the dialect-specific rules. Throws on
 *  unrecoverable parse failure inside the formatter — callers
 *  should catch and leave the editor text alone (formatting
 *  should never lose user work).
 *
 *  Default options: 2-space indent, ANSI keyword case (no
 *  upper-case forcing — most users prefer lower-cased
 *  identifiers and the lib's default upper-case for keywords). */
export function formatSqlText(sql: string, dialect: SqlDialect): string {
  return sqlFormatterFormat(sql, {
    language: dialect,
    tabWidth: 2,
    keywordCase: "upper",
    linesBetweenQueries: 2,
  });
}
