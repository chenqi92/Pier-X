/**
 * Frontend SQL import / export helpers shared by the MySQL,
 * PostgreSQL, and SQLite panels.
 *
 * Scope is intentionally narrow — data-only INSERT generation and
 * naive `;`-split execution. Full DDL-aware dumps belong in a
 * future backend that calls `mysqldump` / `pg_dump` / `sqlite3
 * .dump`; this module exists so the right-click "Import SQL" /
 * "Export SQL" actions in the schema tree have a working v1 path
 * before that lands.
 */

import { quoteIdent, escapeValue } from "./dbColumnRules";
import type { QueryExecutionResult } from "../../lib/types";

type Dialect = "mysql" | "postgres" | "sqlite";

export type ExportOptions = {
  /** Cap the number of rows exported per table. Defaults to 50000.
   *  We surface the cap in the dialog so the user knows when their
   *  export was truncated. */
  rowsLimit?: number;
};

export type ExportResult = {
  sql: string;
  /** Per-table stat: how many rows ended up in the output. The
   *  caller surfaces this in the success notice ("Exported 3
   *  tables, 12,400 rows"). */
  perTableRowCounts: Record<string, number>;
  /** Tables where the row limit kicked in — populated so the
   *  caller can warn the user. */
  truncatedTables: string[];
};

/**
 * Generate INSERT statements for the rows currently in `tables`.
 * Schema (CREATE TABLE / indexes / triggers / FKs) is **not**
 * emitted — for that, call `mysqldump` / `pg_dump` / `sqlite3
 * .dump`. The caller is expected to hand back a SQL string the
 * user can replay against an empty schema of the same shape.
 */
export async function exportTablesAsInserts(
  runQuery: (sql: string) => Promise<QueryExecutionResult>,
  dialect: Dialect,
  qualifyArgs: { database?: string; schema?: string },
  tables: string[],
  opts: ExportOptions = {},
): Promise<ExportResult> {
  const cap = opts.rowsLimit ?? 50_000;
  const perTableRowCounts: Record<string, number> = {};
  const truncatedTables: string[] = [];
  const chunks: string[] = [];

  chunks.push(
    `-- Pier-X export · ${new Date().toISOString()}\n` +
      `-- Engine: ${dialect}\n` +
      `-- Data-only · row cap ${cap.toLocaleString()} per table\n` +
      `-- Schema (CREATE TABLE etc.) is not included; use\n` +
      `-- mysqldump / pg_dump / sqlite3 .dump for full backups.\n`,
  );

  for (const table of tables) {
    const tableRef = qualify(dialect, qualifyArgs, table);
    // +1 over the cap so we can detect a truncation deterministically.
    const r = await runQuery(`SELECT * FROM ${tableRef} LIMIT ${cap + 1}`);
    const overflow = r.rows.length > cap;
    const slice = overflow ? r.rows.slice(0, cap) : r.rows;
    perTableRowCounts[table] = slice.length;
    if (overflow) truncatedTables.push(table);

    chunks.push(`\n-- ${table} · ${slice.length} row(s)\n`);
    if (slice.length === 0) continue;

    const colSql = r.columns.map((c) => quoteIdent(dialect, c)).join(", ");
    for (const row of slice) {
      const valSql = row
        .map((cell) => (cell === null ? "NULL" : escapeNonNull(cell)))
        .join(", ");
      chunks.push(`INSERT INTO ${tableRef} (${colSql}) VALUES (${valSql});\n`);
    }
  }

  return { sql: chunks.join(""), perTableRowCounts, truncatedTables };
}

/**
 * Naive SQL splitter — breaks on `;` outside string literals and
 * line/block comments. Good enough for round-tripping our own
 * `exportTablesAsInserts` output (and most hand-written `.sql`
 * files); not robust against MySQL's `DELIMITER` directive or
 * dollar-quoted Postgres functions. Callers should disclose this
 * limitation in the UI.
 */
export function splitSqlStatements(input: string): string[] {
  const out: string[] = [];
  let buf = "";
  let i = 0;
  let inSingle = false;
  let inDouble = false;
  let inLineComment = false;
  let inBlockComment = false;
  while (i < input.length) {
    const ch = input[i];
    const next = input[i + 1];
    if (inLineComment) {
      buf += ch;
      if (ch === "\n") inLineComment = false;
      i++;
      continue;
    }
    if (inBlockComment) {
      buf += ch;
      if (ch === "*" && next === "/") {
        buf += "/";
        inBlockComment = false;
        i += 2;
        continue;
      }
      i++;
      continue;
    }
    if (inSingle) {
      buf += ch;
      // Doubled '' is an escape for '; just keep eating.
      if (ch === "'" && next !== "'") inSingle = false;
      else if (ch === "'" && next === "'") {
        buf += "'";
        i += 2;
        continue;
      }
      i++;
      continue;
    }
    if (inDouble) {
      buf += ch;
      if (ch === '"') inDouble = false;
      i++;
      continue;
    }
    if (ch === "-" && next === "-") {
      inLineComment = true;
      buf += "--";
      i += 2;
      continue;
    }
    if (ch === "/" && next === "*") {
      inBlockComment = true;
      buf += "/*";
      i += 2;
      continue;
    }
    if (ch === "'") {
      inSingle = true;
      buf += ch;
      i++;
      continue;
    }
    if (ch === '"') {
      inDouble = true;
      buf += ch;
      i++;
      continue;
    }
    if (ch === ";") {
      const s = buf.trim();
      if (s) out.push(s);
      buf = "";
      i++;
      continue;
    }
    buf += ch;
    i++;
  }
  const tail = buf.trim();
  if (tail) out.push(tail);
  return out;
}

function escapeNonNull(value: string): string {
  // The cells already come back as strings from QueryExecutionResult.
  // Use `escapeValue` with `numeric: false` so a string-shaped cell
  // gets the single-quote treatment; numeric cells re-parse fine
  // through MySQL/PG/SQLite even when quoted, so we don't need to
  // probe per-column types here.
  return escapeValue(value, false);
}

function qualify(
  dialect: Dialect,
  args: { database?: string; schema?: string },
  table: string,
): string {
  const segs: string[] = [];
  if (dialect === "mysql" && args.database) segs.push(quoteIdent(dialect, args.database));
  if (dialect === "postgres" && args.schema) segs.push(quoteIdent(dialect, args.schema));
  segs.push(quoteIdent(dialect, table));
  return segs.join(".");
}
