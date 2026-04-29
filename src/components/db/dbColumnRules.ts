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
  /** Pre-computed list of valid values when the column's type is an
   *  enum (PG only for now). The grid renders a `<datalist>` so the
   *  edit cell behaves like a dropdown. */
  enumValues?: string[];
};

export type DbMutation =
  | { kind: "update"; pk: Record<string, string>; changes: Record<string, string | null> }
  | { kind: "insert"; values: Record<string, string | null> }
  | { kind: "delete"; pk: Record<string, string> };

/** DDL mutations from the editable structure view. Add / drop /
 *  rename / type-change / comment. Index / FK / PK changes remain
 *  out-of-scope for now; they need full table rebuilds on SQLite and
 *  per-engine grammars. */
export type DdlMutation =
  | {
      kind: "addColumn";
      name: string;
      type: string;
      nullable: boolean;
      /** `null` = no DEFAULT clause (i.e. NULL when nullable, error
       *  on existing rows when NOT NULL). Empty string is treated
       *  as a string literal `''`, not "no default". */
      defaultValue?: string | null;
    }
  | { kind: "dropColumn"; name: string }
  | { kind: "renameColumn"; oldName: string; newName: string }
  | {
      kind: "modifyColumnType";
      name: string;
      newType: string;
      /** Pre-change column shape. MySQL needs the full re-spec on
       *  `MODIFY COLUMN` so we preserve nullable / default / comment;
       *  PG only uses `newType`; SQLite is rejected here (returns an
       *  Error from the builder — caller must surface it). */
      snapshot: {
        nullable: boolean;
        defaultValue?: string | null;
        comment?: string;
      };
    }
  | {
      kind: "setColumnComment";
      name: string;
      comment: string;
      /** Pre-change column shape. MySQL has to repeat the type / null /
       *  default on `MODIFY COLUMN` to set a comment. Other engines
       *  ignore the snapshot. */
      snapshot: {
        type: string;
        nullable: boolean;
        defaultValue?: string | null;
      };
    };

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

// ── DDL (structure-edit) builders ─────────────────────────────────

type BuildDdlArgs = {
  dialect: DbDialect;
  /** Already-qualified table reference (e.g. `db`.`t` / "schema"."t" / "t"). */
  table: string;
};

/** `ALTER TABLE … ADD COLUMN <name> <type> [NULL|NOT NULL] [DEFAULT …]`.
 *  Type is passed through verbatim — the dialog validates a non-empty
 *  type but does not parse it. NOT NULL on existing rows requires a
 *  DEFAULT or the engine will reject the change; we surface that error
 *  rather than silently inserting one. */
export function buildAddColumnSql(
  args: BuildDdlArgs,
  spec: { name: string; type: string; nullable: boolean; defaultValue?: string | null },
): string {
  const colId = quoteIdent(args.dialect, spec.name);
  const nullClause = spec.nullable ? "" : " NOT NULL";
  const defaultClause =
    spec.defaultValue === undefined || spec.defaultValue === null
      ? ""
      : ` DEFAULT ${escapeValue(spec.defaultValue, false)}`;
  return `ALTER TABLE ${args.table} ADD COLUMN ${colId} ${spec.type}${nullClause}${defaultClause}`;
}

/** `ALTER TABLE … DROP COLUMN <name>`. Standard across MySQL 8 / PG /
 *  SQLite ≥ 3.35. SQLite older than 3.35 will reject with a parse
 *  error — we let the engine surface it rather than version-gating
 *  on the frontend (which would race the cached probe). */
export function buildDropColumnSql(
  args: BuildDdlArgs,
  spec: { name: string },
): string {
  return `ALTER TABLE ${args.table} DROP COLUMN ${quoteIdent(args.dialect, spec.name)}`;
}

/** Rename a column in-place. MySQL 8 / PG / SQLite ≥ 3.25 all accept
 *  the same `RENAME COLUMN <old> TO <new>` syntax — the older MySQL
 *  `CHANGE COLUMN` form (which also requires re-typing) is skipped.
 *  Old SQLite emits a parse error which the user sees verbatim. */
export function buildRenameColumnSql(
  args: BuildDdlArgs,
  spec: { oldName: string; newName: string },
): string {
  const oldId = quoteIdent(args.dialect, spec.oldName);
  const newId = quoteIdent(args.dialect, spec.newName);
  return `ALTER TABLE ${args.table} RENAME COLUMN ${oldId} TO ${newId}`;
}

/** Change a column's data type. MySQL repeats the full column spec
 *  on `MODIFY COLUMN`; PG uses `ALTER COLUMN ... TYPE`; SQLite has
 *  no in-place type change and we throw so the panel can surface
 *  the limitation rather than silently emit broken SQL. */
export function buildModifyColumnTypeSql(
  args: BuildDdlArgs,
  spec: {
    name: string;
    newType: string;
    snapshot: { nullable: boolean; defaultValue?: string | null; comment?: string };
  },
): string {
  const colId = quoteIdent(args.dialect, spec.name);
  if (args.dialect === "sqlite") {
    throw new Error(
      "SQLite does not support changing a column's type in place; recreate the table.",
    );
  }
  if (args.dialect === "postgres") {
    return `ALTER TABLE ${args.table} ALTER COLUMN ${colId} TYPE ${spec.newType}`;
  }
  // MySQL: MODIFY COLUMN must re-state nullable + default + comment
  // because anything omitted is treated as "set to default". Carry
  // the snapshot through so we don't accidentally drop a comment
  // when the user only meant to widen a type.
  const nullClause = spec.snapshot.nullable ? "" : " NOT NULL";
  const defaultClause =
    spec.snapshot.defaultValue === undefined || spec.snapshot.defaultValue === null
      ? ""
      : ` DEFAULT ${escapeValue(spec.snapshot.defaultValue, false)}`;
  const commentClause =
    spec.snapshot.comment && spec.snapshot.comment !== ""
      ? ` COMMENT ${escapeValue(spec.snapshot.comment, false)}`
      : "";
  return `ALTER TABLE ${args.table} MODIFY COLUMN ${colId} ${spec.newType}${nullClause}${defaultClause}${commentClause}`;
}

/** Set or clear a column's comment. MySQL must repeat the full
 *  column spec on `MODIFY COLUMN`; PG has the dedicated
 *  `COMMENT ON COLUMN`. SQLite is rejected (no native column
 *  comments — the panel hides the column entirely there). */
export function buildSetColumnCommentSql(
  args: BuildDdlArgs,
  spec: {
    name: string;
    comment: string;
    snapshot: { type: string; nullable: boolean; defaultValue?: string | null };
  },
): string {
  const colId = quoteIdent(args.dialect, spec.name);
  if (args.dialect === "sqlite") {
    throw new Error("SQLite columns have no native comment.");
  }
  if (args.dialect === "postgres") {
    // Empty comment → IS NULL drops the comment cleanly. Non-empty
    // is single-quote escaped via escapeValue (which routes empty
    // strings to NULL too — so the branch matches PG semantics).
    if (spec.comment === "") {
      return `COMMENT ON COLUMN ${args.table}.${colId} IS NULL`;
    }
    return `COMMENT ON COLUMN ${args.table}.${colId} IS ${escapeValue(spec.comment, false)}`;
  }
  // MySQL: same MODIFY COLUMN dance as buildModifyColumnTypeSql.
  // Empty comment = clear: emit `COMMENT ''`.
  const nullClause = spec.snapshot.nullable ? "" : " NOT NULL";
  const defaultClause =
    spec.snapshot.defaultValue === undefined || spec.snapshot.defaultValue === null
      ? ""
      : ` DEFAULT ${escapeValue(spec.snapshot.defaultValue, false)}`;
  const commentClause =
    spec.comment === ""
      ? " COMMENT ''"
      : ` COMMENT ${escapeValue(spec.comment, false)}`;
  return `ALTER TABLE ${args.table} MODIFY COLUMN ${colId} ${spec.snapshot.type}${nullClause}${defaultClause}${commentClause}`;
}

/** Produce a one-shot DDL string for a single structure mutation. */
export function ddlToSql(args: BuildDdlArgs, mut: DdlMutation): string {
  if (mut.kind === "addColumn") return buildAddColumnSql(args, mut);
  if (mut.kind === "dropColumn") return buildDropColumnSql(args, mut);
  if (mut.kind === "renameColumn") return buildRenameColumnSql(args, mut);
  if (mut.kind === "modifyColumnType") return buildModifyColumnTypeSql(args, mut);
  return buildSetColumnCommentSql(args, mut);
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

export function gridColumnsFromPostgres(
  cols: PostgresColumnView[],
  enums?: { name: string; values: string[] }[],
): GridColumnMeta[] {
  // Build a fast type-name lookup. PG's `format_type` may emit the
  // enum name with `[]` appended for arrays, so we strip that off
  // before matching — array editing lands on the same dropdown.
  const enumMap = new Map<string, string[]>();
  for (const e of enums ?? []) enumMap.set(e.name, e.values);
  return cols.map((c) => {
    const baseType = c.columnType.replace(/\[\]$/, "");
    const enumValues = enumMap.get(baseType);
    return {
      name: c.name,
      type: c.columnType,
      numeric: isNumericType(c.columnType),
      pk: c.key.toUpperCase() === "PRI",
      enumValues,
    };
  });
}

export function gridColumnsFromSqlite(cols: SqliteColumnView[]): GridColumnMeta[] {
  return cols.map((c) => ({
    name: c.name,
    type: c.colType,
    numeric: isNumericType(c.colType),
    pk: c.primaryKey,
  }));
}
