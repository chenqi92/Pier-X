import type { DbKind } from "./types";

// Heuristic match for authentication-related failures coming back
// from the DB clients. The underlying drivers don't expose a
// structured error code through the Tauri bridge, so we pattern-match
// the human-readable message. Keep this list tight — false positives
// push a "Update password" prompt on users who have an unrelated
// error, which is confusing.
//
// Patterns come from:
//   • MySQL  — mysql_async / server protocol: "Access denied" / 1045
//   • PG     — rust-postgres: "password authentication failed"
//   • Redis  — redis-rs: "NOAUTH", "WRONGPASS", "invalid password"
//   • SQLite — no password concept, never matches
export function isDbAuthError(kind: DbKind, message: string): boolean {
  if (!message) return false;
  const m = message.toLowerCase();
  switch (kind) {
    case "mysql":
      return (
        m.includes("access denied") ||
        m.includes("er_access_denied") ||
        m.includes("authentication plugin") ||
        m.includes("1045")
      );
    case "postgres":
      return (
        m.includes("password authentication failed") ||
        m.includes("authentication failed") ||
        (m.includes("auth") && m.includes("password"))
      );
    case "redis":
      return (
        m.includes("noauth") ||
        m.includes("wrongpass") ||
        m.includes("invalid password") ||
        m.includes("authentication required")
      );
    case "sqlite":
      return false;
  }
}
