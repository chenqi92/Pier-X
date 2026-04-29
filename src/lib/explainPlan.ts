// Parsers for PostgreSQL `EXPLAIN (FORMAT JSON, ANALYZE, BUFFERS)` and
// MySQL `EXPLAIN FORMAT=JSON` output → a unified `PlanNode` tree the
// `ExplainPlanView` component can render uniformly.
//
// Both engines wrap their JSON in a single text cell on the first row
// of the result set. PG returns a one-element array `[{"Plan": ...}]`;
// MySQL returns `{"query_block": ...}`. Each engine has its own node
// shape — we normalize them here so the renderer stays dialect-agnostic.

export type PlanNode = {
  /** Display label for the operator (e.g. "Seq Scan", "Hash Join"). */
  label: string;
  /** Free-form detail line (target table / index / condition). */
  detail?: string;
  /** Estimated rows from the planner. */
  rows?: number;
  /** Actual rows produced (only present for ANALYZE). */
  actualRows?: number;
  /** Estimated cost. PG returns startup..total; MySQL has a single
   *  `cost` per node. We surface both ends when available. */
  cost?: { startup?: number; total: number };
  /** Actual elapsed time per loop, ms. ANALYZE only. */
  actualTime?: { startup?: number; total: number };
  /** Buffers / I/O counters as a plain string ("hits=12 reads=0").
   *  PG only — present when EXPLAIN was run with BUFFERS. */
  buffers?: string;
  /** Children. Empty array for leaf nodes. */
  children: PlanNode[];
  /** The original engine-specific JSON for this node, surfaced so the
   *  UI can show a raw-JSON expander on click. Undefined for synthetic
   *  wrapper nodes (e.g. the MySQL "Nested loop" parent we inject to
   *  preserve join topology). */
  raw?: unknown;
};

/**
 * Parse a PostgreSQL `EXPLAIN (FORMAT JSON, ...)` text payload.
 * Returns `null` when the input doesn't look like a JSON plan.
 */
export function parsePostgresPlan(raw: string): PlanNode | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!Array.isArray(parsed) || parsed.length === 0) return null;
  const root = (parsed[0] as Record<string, unknown>)?.["Plan"];
  if (!root || typeof root !== "object") return null;
  return convertPgPlan(root as Record<string, unknown>);
}

function convertPgPlan(node: Record<string, unknown>): PlanNode {
  const label = String(node["Node Type"] ?? "Plan");
  const relation = node["Relation Name"];
  const indexName = node["Index Name"];
  const alias = node["Alias"];
  const detailParts: string[] = [];
  if (typeof relation === "string") {
    detailParts.push(
      typeof alias === "string" && alias !== relation
        ? `${relation} (${alias})`
        : relation,
    );
  }
  if (typeof indexName === "string") {
    detailParts.push(`using ${indexName}`);
  }
  for (const key of [
    "Index Cond",
    "Recheck Cond",
    "Filter",
    "Hash Cond",
    "Merge Cond",
    "Join Filter",
  ] as const) {
    const v = node[key];
    if (typeof v === "string" && v.length > 0) {
      detailParts.push(`${key}: ${v}`);
    }
  }
  const startupCost = numOrUndef(node["Startup Cost"]);
  const totalCost = numOrUndef(node["Total Cost"]);
  const actualStartup = numOrUndef(node["Actual Startup Time"]);
  const actualTotal = numOrUndef(node["Actual Total Time"]);
  const loops = numOrUndef(node["Actual Loops"]) ?? 1;

  const buffersBits: string[] = [];
  for (const [k, label] of [
    ["Shared Hit Blocks", "hits"],
    ["Shared Read Blocks", "reads"],
    ["Shared Dirtied Blocks", "dirtied"],
    ["Shared Written Blocks", "written"],
  ] as const) {
    const v = numOrUndef(node[k]);
    if (v !== undefined && v > 0) buffersBits.push(`${label}=${v}`);
  }

  const childrenRaw = node["Plans"];
  const children =
    Array.isArray(childrenRaw)
      ? childrenRaw.map((c) => convertPgPlan(c as Record<string, unknown>))
      : [];

  return {
    label,
    detail: detailParts.length ? detailParts.join(" · ") : undefined,
    rows: numOrUndef(node["Plan Rows"]),
    actualRows:
      numOrUndef(node["Actual Rows"]) !== undefined && loops !== 1
        ? Math.round((numOrUndef(node["Actual Rows"]) ?? 0) * loops)
        : numOrUndef(node["Actual Rows"]),
    cost:
      totalCost !== undefined ? { startup: startupCost, total: totalCost } : undefined,
    actualTime:
      actualTotal !== undefined
        ? { startup: actualStartup, total: actualTotal }
        : undefined,
    buffers: buffersBits.length ? buffersBits.join(" ") : undefined,
    children,
    raw: node,
  };
}

/**
 * Parse a MySQL `EXPLAIN FORMAT=JSON` text payload.
 * MySQL nests sub-plans under varying keys (`nested_loop`,
 * `query_block`, `table`, `attached_subqueries`, …) so we walk
 * recursively, collapsing single-child wrappers and surfacing the
 * leaf `table` operators.
 */
export function parseMysqlPlan(raw: string): PlanNode | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  const block = (parsed as Record<string, unknown>)?.["query_block"];
  if (!block || typeof block !== "object") return null;
  return convertMysqlBlock(block as Record<string, unknown>, "Query block");
}

function convertMysqlBlock(
  node: Record<string, unknown>,
  fallbackLabel: string,
): PlanNode {
  const selectId = node["select_id"];
  const label =
    typeof selectId === "number"
      ? `Query block #${selectId}`
      : fallbackLabel;
  const costInfo = node["cost_info"] as Record<string, unknown> | undefined;
  const totalCost = costInfo
    ? numOrUndef(costInfo["query_cost"]) ?? numOrUndef(costInfo["read_cost"])
    : undefined;

  const children: PlanNode[] = [];
  collectMysqlChildren(node, children);

  return {
    label,
    cost: totalCost !== undefined ? { total: totalCost } : undefined,
    children,
    raw: node,
  };
}

function collectMysqlChildren(
  node: Record<string, unknown>,
  out: PlanNode[],
): void {
  // `nested_loop` is an array of per-table operators forming a
  // left-deep join. We surface them under a single synthetic "Nested
  // loop" parent so a 7-table join keeps its grouping in the UI
  // instead of flattening into the enclosing block. Each entry can
  // itself carry sub-features (subqueries, materialized tables) so we
  // recurse with a fresh `out` and attach the result as siblings.
  if (Array.isArray(node["nested_loop"])) {
    const arr = node["nested_loop"] as unknown[];
    const joined: PlanNode[] = [];
    for (const sub of arr) {
      const obj = sub as Record<string, unknown>;
      collectMysqlChildren(obj, joined);
    }
    out.push({
      label: "Nested loop",
      detail:
        joined.length > 1 ? `joins ${joined.length} table(s)` : undefined,
      children: joined,
      raw: node,
    });
    return;
  }
  // Common single-table operator. Folds index / access / filter into detail.
  if (node["table"]) {
    out.push(convertMysqlTable(node["table"] as Record<string, unknown>));
    return;
  }
  // Subquery / union / materialized blocks.
  for (const key of [
    "ordering_operation",
    "grouping_operation",
    "duplicates_removal",
    "windowing",
  ] as const) {
    const sub = node[key];
    if (sub && typeof sub === "object") {
      collectMysqlChildren(sub as Record<string, unknown>, out);
      return;
    }
  }
  for (const key of ["materialized_from_subquery", "attached_subqueries"] as const) {
    const sub = node[key];
    if (Array.isArray(sub)) {
      for (const s of sub as unknown[]) {
        const block = (s as Record<string, unknown>)?.["query_block"];
        if (block) {
          out.push(
            convertMysqlBlock(
              block as Record<string, unknown>,
              key === "attached_subqueries" ? "Subquery" : "Materialized",
            ),
          );
        }
      }
    } else if (sub && typeof sub === "object") {
      const block = (sub as Record<string, unknown>)?.["query_block"];
      if (block) {
        out.push(
          convertMysqlBlock(block as Record<string, unknown>, "Materialized"),
        );
      }
    }
  }
}

function convertMysqlTable(t: Record<string, unknown>): PlanNode {
  const tableName = strOrUndef(t["table_name"]);
  const accessType = strOrUndef(t["access_type"]) ?? "table";
  const labelBits: string[] = [accessType];
  const detailBits: string[] = [];
  if (tableName) detailBits.push(tableName);
  const usedKey = strOrUndef(t["key"]);
  if (usedKey) detailBits.push(`using ${usedKey}`);
  const cond = strOrUndef(t["attached_condition"]);
  if (cond) detailBits.push(`filter: ${cond}`);
  const usedColumns = t["used_columns"];
  if (Array.isArray(usedColumns) && usedColumns.length > 0) {
    detailBits.push(`cols: ${usedColumns.length}`);
  }
  const costInfo = t["cost_info"] as Record<string, unknown> | undefined;
  const cost =
    costInfo !== undefined
      ? {
          total:
            numOrUndef(costInfo["read_cost"]) ??
            numOrUndef(costInfo["eval_cost"]) ??
            0,
        }
      : undefined;
  const rows =
    numOrUndef(t["rows_examined_per_scan"]) ?? numOrUndef(t["rows"]);
  return {
    label: labelBits.join(" "),
    detail: detailBits.length ? detailBits.join(" · ") : undefined,
    rows,
    cost,
    children: [],
    raw: t,
  };
}

function numOrUndef(v: unknown): number | undefined {
  if (typeof v === "number" && Number.isFinite(v)) return v;
  if (typeof v === "string") {
    const n = Number(v);
    if (Number.isFinite(n)) return n;
  }
  return undefined;
}

function strOrUndef(v: unknown): string | undefined {
  return typeof v === "string" && v.length > 0 ? v : undefined;
}

/**
 * Pull the JSON plan blob out of a query result. Both engines return a
 * single column with one row containing the JSON; this helper hides the
 * exact shape so panels can pass `r.rows` and a dialect tag.
 */
export function extractJsonPlanCell(rows: unknown[][]): string | null {
  if (rows.length === 0) return null;
  const first = rows[0];
  if (!first || first.length === 0) return null;
  const cell = first[0];
  if (typeof cell !== "string") return null;
  return cell;
}
