// Per-credential connection-quality cache. After a successful browse
// the panel writes the latest roundtrip ms + (when known) database
// size into this cache, keyed by `${kind}:${credId}`. The DB splash
// reads it on render so saved-profile rows can show "23 ms · 4.2 MB"
// without round-tripping the server every time the user lands on the
// splash. Best-effort persistence via localStorage; cache misses are
// fine and just hide the chip.

const STORAGE_KEY = "pier-x:db-conn-cache-v1";

export type DbConnCacheEntry = {
  /** Wall-clock ms of the most recent successful browse. */
  connectMs: number;
  /** Database size in bytes when known (sum of table data+index bytes
   *  for SQL engines, file size for SQLite, used_memory for Redis).
   *  Optional — splash falls back to omitting the chip. */
  sizeBytes?: number;
  /** Unix ms when the cache entry was last refreshed. */
  lastConnectedAt: number;
};

type Cache = Record<string, DbConnCacheEntry>;

function read(): Cache {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    return parsed as Cache;
  } catch {
    return {};
  }
}

function write(cache: Cache) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(cache));
  } catch {
    /* quota exceeded — drop silently */
  }
}

function key(kind: string, credId: string): string {
  return `${kind}:${credId}`;
}

export function getDbConnCache(
  kind: string,
  credId: string,
): DbConnCacheEntry | undefined {
  const cache = read();
  return cache[key(kind, credId)];
}

export function setDbConnCache(
  kind: string,
  credId: string,
  patch: Partial<DbConnCacheEntry> & Pick<DbConnCacheEntry, "connectMs">,
) {
  const cache = read();
  const k = key(kind, credId);
  const prev = cache[k];
  cache[k] = {
    connectMs: patch.connectMs,
    sizeBytes: patch.sizeBytes ?? prev?.sizeBytes,
    lastConnectedAt: patch.lastConnectedAt ?? Date.now(),
  };
  write(cache);
}

export function formatBytes(n: number | undefined): string {
  if (typeof n !== "number" || !Number.isFinite(n) || n <= 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let val = n;
  let u = 0;
  while (val >= 1024 && u < units.length - 1) {
    val /= 1024;
    u++;
  }
  return `${val < 10 && u > 0 ? val.toFixed(1) : Math.round(val)} ${units[u]}`;
}

/** Format a relative timestamp for the splash's "last seen" chip.
 *  Resolution drops to whole minutes / hours / days as the gap grows.
 *  Returns null when the timestamp is missing or in the future. */
export function formatLastSeen(unixMs: number | undefined): string | null {
  if (typeof unixMs !== "number" || !Number.isFinite(unixMs)) return null;
  const delta = Date.now() - unixMs;
  if (delta < 0) return null;
  if (delta < 60_000) return "just now";
  const mins = Math.floor(delta / 60_000);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}
