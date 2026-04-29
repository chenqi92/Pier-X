import type { DbForeignKeyView } from "../../lib/types";
import type { DbRowFk } from "./DbRowDetail";

/** Quote a SQL identifier for the given dialect. MySQL uses backticks,
 *  Postgres uses double quotes. We're conservative and reject anything
 *  outside the same allowlist the backend uses (`is_safe_ident` /
 *  similar) to keep the generated SQL trivially injection-safe. */
function quoteIdent(dialect: "mysql" | "postgres", name: string): string {
  if (!/^[A-Za-z_][A-Za-z0-9_$]*$/.test(name)) {
    throw new Error(`unsafe identifier: ${name}`);
  }
  return dialect === "mysql" ? `\`${name}\`` : `"${name}"`;
}

/** Quote a string literal — doubles single quotes (works for MySQL +
 *  PG identically because the rendered grid values come back as
 *  strings either way). NULL passes through as the literal NULL. */
function quoteLiteral(value: string | null): string {
  if (value === null || value === "") return "NULL";
  return `'${value.replace(/'/g, "''")}'`;
}

/**
 * Build an array of foreign-key edges suitable for `DbRowDetail`'s
 * `foreignKeys` prop, given:
 *   - the result-grid column ordering (so we can map each FK column
 *     name to its index in the row)
 *   - the row's cell values
 *   - the FK metadata returned by `mysqlBrowse` / `postgresBrowse`
 *
 * Each edge clicks through to a `SELECT * FROM ref_table WHERE
 * refCols = ?` query that the parent panel runs via its own
 * `setSql + runQuery` callback. Multi-column FKs AND the components
 * together. NULL values short-circuit the edge (an FK to NULL has no
 * meaningful target).
 */
export function buildFkEdges(
  columns: string[],
  row: string[],
  foreignKeys: DbForeignKeyView[],
  dialect: "mysql" | "postgres",
  onNavigate: (sql: string) => void,
  t: (s: string, vars?: Record<string, string | number>) => string,
): DbRowFk[] {
  const edges: DbRowFk[] = [];
  for (const fk of foreignKeys) {
    if (fk.columns.length === 0 || fk.columns.length !== fk.refColumns.length) {
      continue;
    }
    // Look up local column indices.
    const localValues: (string | null)[] = [];
    let missing = false;
    for (const col of fk.columns) {
      const idx = columns.indexOf(col);
      if (idx < 0) {
        missing = true;
        break;
      }
      const cell = row[idx];
      localValues.push(cell === undefined ? null : cell);
    }
    if (missing) continue;
    if (localValues.every((v) => v === null || v === "")) continue;

    let sql: string;
    try {
      const tableRef =
        fk.refSchema && dialect === "postgres"
          ? `${quoteIdent(dialect, fk.refSchema)}.${quoteIdent(
              dialect,
              fk.refTable,
            )}`
          : quoteIdent(dialect, fk.refTable);
      const where = fk.refColumns
        .map(
          (col, i) =>
            `${quoteIdent(dialect, col)} = ${quoteLiteral(localValues[i])}`,
        )
        .join(" AND ");
      sql = `SELECT * FROM ${tableRef} WHERE ${where} LIMIT 50`;
    } catch {
      continue;
    }

    const summary = fk.columns
      .map((col, i) => `${col}=${localValues[i] ?? "NULL"}`)
      .join(", ");
    edges.push({
      label: t("{ref} ({summary})", {
        ref: fk.refTable,
        summary,
      }),
      onClick: () => onNavigate(sql),
    });
  }
  return edges;
}
