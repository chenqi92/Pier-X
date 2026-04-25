/**
 * Per-dialect helpers for the result grid's inline-CRUD flow. Owns:
 *   - identifier quoting (`mysql backticks` vs "pg/sqlite double-quotes")
 *   - schema-qualified table refs
 *   - value escaping with NULL handling and numeric pass-through
 *   - UPDATE / INSERT / DELETE statement builders
 *
 * Each panel passes `kind` plus its own column metadata, then ships
 * the generated SQL through its existing *_execute Tauri command.
 */

import type {
  MysqlColumnView,
  PostgresColumnView,
  SqliteColumnView,
} from "../../lib/types";

export type DbDialect = "mysql" | "postgres" | "sqlite";

/** Subset of column info the grid needs to commit edits. Each panel
 *  derives this from its own browser-state column shape so the grid
 *  doesn't import every backend type. */
export type GridColumnMeta = {
  name: string;
  /** Engine-specific type string (e.g. "int", "varchar(64)", "TEXT"). */
  type: string;
  /** Treat as numeric for sort + emit unquoted in SQL. */
  numeric: boolean;
  /** Part of the primary key — required to identify rows for UPDATE/DELETE. */
  pk: boolean;
};

export type DbMutation =
  | { kind: "update"; pk: Record<string, string>; changes: Record<string, string | null> }
  | { kind: "insert"; values: Record<string, string | null> }
  | { kind: "delete"; pk: Record<string, string> };

/** Numeric-type regex shared across dialects. */
const NUMERIC_RE =
  /^(tiny|small|medium|big)?int|^integer|^decimal|^numeric|^float|^double|^real|^money|^serial|^bigserial/i;

export function isNumericType(typeStr: string | null | undefined): boolean {
  if (!typeStr) return false;
  return NUMERIC_RE.test(typeStr.toLowerCase());
}

/** Quote a single identifier per dialect. SQLite uses double-quotes
 *  by spec; MySQL backticks; Postgres double-quotes. */
export function quoteIdent(dialect: DbDialect, name: string): string {
  if (dialect === "mysql") return `\`${name.replace(/`/g, "``")}\``;
  return `"${name.replace(/"/g, '""')}"`;
}

/** Build a fully-qualified table reference (e.g. `db`.`table`,
 *  "schema"."table"). Empty parts are skipped. */
export function qualifyTable(
  dialect: DbDialect,
  parts: { database?: string | null; schema?: string | null; table: string },
): string {
  const segs: string[] = [];
  // MySQL uses database.table; Postgres uses schema.table; SQLite is bare.
  if (dialect === "mysql" && parts.database) segs.push(quoteIdent(dialect, parts.database));
  if (dialect === "postgres" && parts.schema) segs.push(quoteIdent(dialect, parts.schema));
  segs.push(quoteIdent(dialect, parts.table));
  return segs.join(".");
}

/** Quote a value for inline SQL. NULL passes through unquoted; numerics
 *  pass through if parseable; everything else is single-quote escaped. */
export function escapeValue(
  value: string | null,
  numeric: boolean,
): string {
  if (value === null || value === undefined) return "NULL";
  if (value === "") return "NULL";
  if (numeric) {
    const n = Number(value);
    if (Number.isFinite(n)) return value.trim();
    // Fall through to quoted — backend will reject if truly invalid,
    // which is more honest than silently coercing to 0.
  }
  return `'${value.replace(/'/g, "''")}'`;
}

type BuildSqlArgs = {
  dialect: DbDialect;
  table: string; // already-qualified table reference
  columns: GridColumnMeta[];
};

export function buildUpdateSql(
  args: BuildSqlArgs,
  pk: Record<string, string>,
  changes: Record<string, string | null>,
): string {
  const colByName = new Map(args.columns.map((c) => [c.name, c]));
  const setClauses = Object.entries(changes).map(([col, val]) => {
    const meta = colByName.get(col);
    return `${quoteIdent(args.dialect, col)} = ${escapeValue(val, meta?.numeric ?? false)}`;
  });
  const whereClauses = Object.entries(pk).map(([col, val]) => {
    const meta = colByName.get(col);
    return `${quoteIdent(args.dialect, col)} = ${escapeValue(val, meta?.numeric ?? false)}`;
  });
  return `UPDATE ${args.table} SET ${setClauses.join(", ")} WHERE ${whereClauses.join(" AND ")}`;
}

export function buildInsertSql(
  args: BuildSqlArgs,
  values: Record<string, string | null>,
): string {
  const colByName = new Map(args.columns.map((c) => [c.name, c]));
  const cols = Object.keys(values);
  const colSql = cols.map((c) => quoteIdent(args.dialect, c)).join(", ");
  const valSql = cols
    .map((c) => escapeValue(values[c], colByName.get(c)?.numeric ?? false))
    .join(", ");
  return `INSERT INTO ${args.table} (${colSql}) VALUES (${valSql})`;
}

export function buildDeleteSql(
  args: BuildSqlArgs,
  pk: Record<string, string>,
): string {
  const colByName = new Map(args.columns.map((c) => [c.name, c]));
  const whereClauses = Object.entries(pk).map(([col, val]) => {
    const meta = colByName.get(col);
    return `${quoteIdent(args.dialect, col)} = ${escapeValue(val, meta?.numeric ?? false)}`;
  });
  return `DELETE FROM ${args.table} WHERE ${whereClauses.join(" AND ")}`;
}

/** Produce a one-shot SQL string for a single mutation. */
export function mutationToSql(args: BuildSqlArgs, mut: DbMutation): string {
  if (mut.kind === "update") return buildUpdateSql(args, mut.pk, mut.changes);
  if (mut.kind === "insert") return buildInsertSql(args, mut.values);
  return buildDeleteSql(args, mut.pk);
}

// ── Per-engine column adapters ────────────────────────────────────

export function gridColumnsFromMysql(cols: MysqlColumnView[]): GridColumnMeta[] {
  return cols.map((c) => ({
    name: c.name,
    type: c.columnType,
    numeric: isNumericType(c.columnType),
    pk: c.key.toUpperCase() === "PRI",
  }));
}

export function gridColumnsFromPostgres(cols: PostgresColumnView[]): GridColumnMeta[] {
  return cols.map((c) => ({
    name: c.name,
    type: c.columnType,
    numeric: isNumericType(c.columnType),
    pk: c.key.toUpperCase() === "PRI",
  }));
}

export function gridColumnsFromSqlite(cols: SqliteColumnView[]): GridColumnMeta[] {
  return cols.map((c) => ({
    name: c.name,
    type: c.colType,
    numeric: isNumericType(c.colType),
    pk: c.primaryKey,
  }));
}
