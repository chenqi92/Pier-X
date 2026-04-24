import type { ReactNode } from "react";

// Curated SQL keyword set — covers MySQL / PostgreSQL / SQLite dialects
// at the level the in-panel editor will realistically type. Not meant
// to be exhaustive — just enough to keep colouring useful when the
// user is writing ad-hoc queries.
const KEYWORDS = new Set([
  "select", "from", "where", "and", "or", "not", "in", "is", "null", "as",
  "join", "left", "right", "inner", "outer", "on", "group", "by", "order", "having",
  "limit", "offset", "insert", "into", "values", "update", "set", "delete",
  "create", "table", "index", "view", "alter", "drop", "primary", "key", "foreign",
  "references", "unique", "default", "cascade", "interval", "between", "like", "ilike",
  "any", "all", "exists", "case", "when", "then", "else", "end", "distinct", "union",
  "with", "returning", "conflict", "do", "nothing", "asc", "desc", "true", "false",
  "begin", "commit", "rollback", "transaction", "explain", "analyze",
  "show", "use", "describe", "desc", "database", "databases", "tables", "columns",
]);

const FUNCS = new Set([
  "now", "count", "sum", "avg", "min", "max", "coalesce", "nullif", "length",
  "lower", "upper", "substr", "substring", "concat", "date_trunc", "extract",
  "to_char", "cast", "json_agg", "array_agg", "generate_series", "unnest",
  "current_timestamp", "current_date", "current_time",
]);

const TOKENIZER = /(\s+|[(),;=<>+\-*/]+|'(?:[^']|'')*'|"[^"]*"|--[^\n]*|\/\*[\s\S]*?\*\/)/g;

/**
 * Tokenise a SQL buffer into coloured spans. Cheap, runs on every
 * render of the editor overlay — keep it linear and allocation-light.
 */
export function renderSqlTokens(sql: string): ReactNode {
  const tokens = sql.split(TOKENIZER);
  const out: ReactNode[] = [];
  for (let i = 0; i < tokens.length; i += 1) {
    const t = tokens[i];
    if (!t) continue;
    if (/^--/.test(t) || /^\/\*/.test(t)) {
      out.push(<span key={i} className="sq-cm">{t}</span>);
      continue;
    }
    if (/^'/.test(t) || /^"/.test(t)) {
      out.push(<span key={i} className="sq-st">{t}</span>);
      continue;
    }
    if (/^\d+(\.\d+)?$/.test(t)) {
      out.push(<span key={i} className="sq-nm">{t}</span>);
      continue;
    }
    if (/^[=<>+\-*/(),;]+$/.test(t)) {
      out.push(<span key={i} className="sq-op">{t}</span>);
      continue;
    }
    const low = t.toLowerCase();
    if (KEYWORDS.has(low)) {
      out.push(<span key={i} className="sq-kw">{t}</span>);
      continue;
    }
    if (FUNCS.has(low)) {
      out.push(<span key={i} className="sq-fn">{t}</span>);
      continue;
    }
    out.push(<span key={i} className="sq-id">{t}</span>);
  }
  return out;
}
