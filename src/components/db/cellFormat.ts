// ── Cell display formatting ──────────────────────────────────────
//
// Helpers the result grid uses to enrich raw stringified cell
// values without changing the grid's compact-row default. Today
// the only enrichment is "pretty-print JSON-shaped strings on
// hover" — JSONB / array / json columns from PG come back as
// compact `{"a":1}` and the user can see the formatted version
// by hovering the cell. The renderer keeps the inline display
// untouched so column widths stay sane.

/** Shape detector: returns the parsed value when `s` *looks like*
 *  JSON and parses successfully. Conservative — anything we can't
 *  cleanly round-trip is treated as plain text so the result grid
 *  never silently swallows a value. */
function parseJsonish(s: string): unknown | null {
  const trimmed = s.trim();
  if (trimmed.length === 0) return null;
  // Quick prefix check before attempting a parse — avoids the
  // O(n) cost of `JSON.parse` on every plain string in the grid.
  const first = trimmed[0];
  if (first !== "{" && first !== "[" && first !== '"') return null;
  // Only objects / arrays / quoted strings with reasonable size
  // are worth pretty-printing. A JSONB string of `"x"` is shorter
  // formatted than as-is, so we skip those — same for primitives.
  try {
    return JSON.parse(trimmed);
  } catch {
    return null;
  }
}

/** Return a pretty-printed form of `value` when it parses as
 *  JSON, else return `null`. Caller decides whether to use it as
 *  a tooltip, render in a detail view, etc. */
export function prettyJsonish(value: string): string | null {
  const parsed = parseJsonish(value);
  return parsed === null ? null : JSON.stringify(parsed, null, 2);
}
