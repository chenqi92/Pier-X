// ── Log level detection ────────────────────────────────────────────
// Shared by every surface that shows raw log lines (Log tool panel,
// container-logs dialog) so "what counts as an error" stays one
// definition. Pure string heuristics — cheap enough to run once per
// appended line on a busy stream.

export type LogLevel = "info" | "warn" | "error" | "debug";

/** Classify one log line. `kindHint` carries transport-level
 *  knowledge when the caller has it: a stream `error` event is an
 *  error outright; `stderr` only downgrades to warn when the text
 *  itself names no level (lots of well-behaved daemons write INFO
 *  to stderr). Only the head of the line is scanned — levels live
 *  near the front in every common format (logback/log4j, journald,
 *  nginx, docker). */
export function detectLogLevel(text: string, kindHint?: string): LogLevel {
  if (kindHint === "error") return "error";
  const head = text.slice(0, 120).toUpperCase();
  if (/\b(ERROR|ERR|FATAL|PANIC|CRIT|CRITICAL|SEVERE)\b/.test(head)) return "error";
  if (/\b(WARN|WARNING)\b/.test(head)) return "warn";
  if (/\b(DEBUG|TRACE|VERBOSE)\b/.test(head)) return "debug";
  if (kindHint === "stderr") return "warn";
  return "info";
}
