import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { SqlFavoriteEntry, SqlHistoryEntry, SqlTab } from "./DbSqlEditor";

/** Maximum number of tabs we keep open. Beyond this the oldest non-active
 *  tab is dropped — a small panel mounted in the right rail can't fit a
 *  long tab strip anyway. */
const MAX_TABS = 8;
/** Maximum history entries we retain. Bumped from 50 to 200 once
 *  history persists across reloads — a few hundred queries is the
 *  realistic upper bound for "things I might want to recall". */
const MAX_HISTORY = 200;
/** Top-level localStorage namespace for SQL history. Each panel
 *  passes its own key (e.g. `mysql`, `postgres`) and we persist
 *  under `pier-x:sql-history:<key>`. */
const STORAGE_PREFIX = "pier-x:sql-history:";
/** Sibling namespace for pinned/saved queries. Kept in a separate
 *  bucket so clearing history doesn't wipe favorites. */
const FAVORITES_PREFIX = "pier-x:sql-favorites:";
/** Open-tab buffer namespace — survives panel re-mount so the user
 *  doesn't lose half-typed SQL when toggling between right-rail tools. */
const TABS_PREFIX = "pier-x:sql-tabs:";
/** Soft cap on favorites — far smaller than history because each
 *  entry is human-curated. */
const MAX_FAVORITES = 100;

type UseDbSqlTabsArgs = {
  /** SQL the very first tab opens with. */
  initialSql: string;
  /** Display name for the first tab (e.g. "warehouse"). */
  initialName?: string;
  /** When provided, history is rehydrated from localStorage on
   *  mount and persisted on every `pushHistory`. Pass a stable
   *  per-engine key (e.g. `"mysql"` / `"postgres"`). Omit to
   *  keep the history in-memory only — same as before. */
  storageKey?: string;
};

function readPersistedHistory(storageKey: string | undefined): SqlHistoryEntry[] {
  if (!storageKey || typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(STORAGE_PREFIX + storageKey);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    // Trust but verify — drop entries that don't look like the
    // expected shape so we never feed garbage to the editor.
    return parsed
      .filter((e): e is SqlHistoryEntry =>
        !!e && typeof e === "object" && typeof (e as SqlHistoryEntry).sql === "string",
      )
      .slice(0, MAX_HISTORY);
  } catch {
    return [];
  }
}

function writePersistedHistory(storageKey: string | undefined, history: SqlHistoryEntry[]) {
  if (!storageKey || typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      STORAGE_PREFIX + storageKey,
      JSON.stringify(history.slice(0, MAX_HISTORY)),
    );
  } catch {
    // Quota exceeded / private mode etc. — drop silently.
  }
}

type PersistedTabs = {
  tabs: SqlTab[];
  activeId: string;
};

function readPersistedTabs(storageKey: string | undefined): PersistedTabs | null {
  if (!storageKey || typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(TABS_PREFIX + storageKey);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return null;
    const obj = parsed as { tabs?: unknown; activeId?: unknown };
    if (!Array.isArray(obj.tabs) || typeof obj.activeId !== "string") return null;
    const tabs = obj.tabs.filter(
      (t): t is SqlTab =>
        !!t &&
        typeof t === "object" &&
        typeof (t as SqlTab).id === "string" &&
        typeof (t as SqlTab).name === "string" &&
        typeof (t as SqlTab).sql === "string",
    );
    if (tabs.length === 0) return null;
    const activeId = tabs.some((t) => t.id === obj.activeId) ? obj.activeId : tabs[0].id;
    return { tabs: tabs.slice(0, MAX_TABS), activeId };
  } catch {
    return null;
  }
}

function writePersistedTabs(
  storageKey: string | undefined,
  payload: PersistedTabs,
) {
  if (!storageKey || typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      TABS_PREFIX + storageKey,
      JSON.stringify({
        tabs: payload.tabs.slice(0, MAX_TABS),
        activeId: payload.activeId,
      }),
    );
  } catch {
    /* quota exceeded — drop silently */
  }
}

function readPersistedFavorites(storageKey: string | undefined): SqlFavoriteEntry[] {
  if (!storageKey || typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(FAVORITES_PREFIX + storageKey);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter((e): e is SqlFavoriteEntry =>
        !!e &&
        typeof e === "object" &&
        typeof (e as SqlFavoriteEntry).sql === "string" &&
        typeof (e as SqlFavoriteEntry).id === "string",
      )
      .slice(0, MAX_FAVORITES);
  } catch {
    return [];
  }
}

function writePersistedFavorites(storageKey: string | undefined, favorites: SqlFavoriteEntry[]) {
  if (!storageKey || typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      FAVORITES_PREFIX + storageKey,
      JSON.stringify(favorites.slice(0, MAX_FAVORITES)),
    );
  } catch {
    // Quota exceeded / private mode — drop silently.
  }
}

/** Lightweight tabs + history state for the SQL editor. Keeps the
 *  editor a controlled component while letting panels share one
 *  consistent multi-tab model. */
export function useDbSqlTabs({
  initialSql,
  initialName = "query",
  storageKey,
}: UseDbSqlTabsArgs) {
  const counter = useRef(1);
  const makeId = useCallback(() => `q${++counter.current}`, []);

  // Rehydrate tabs from localStorage when storageKey is provided —
  // otherwise seed a single tab with the caller's initial SQL.
  const persisted = useMemo(() => readPersistedTabs(storageKey), [storageKey]);
  const [tabs, setTabs] = useState<SqlTab[]>(() => {
    if (persisted) {
      // Bump the id counter past the highest restored id so newly-
      // created tabs don't collide with rehydrated ones.
      for (const t of persisted.tabs) {
        const m = /^q(\d+)$/.exec(t.id);
        if (m) {
          const n = Number(m[1]);
          if (Number.isFinite(n) && n > counter.current) counter.current = n;
        }
      }
      return persisted.tabs;
    }
    return [{ id: "q1", name: initialName, sql: initialSql, dirty: false }];
  });
  const [activeId, setActiveId] = useState<string>(() => persisted?.activeId ?? "q1");
  const [history, setHistory] = useState<SqlHistoryEntry[]>(() =>
    readPersistedHistory(storageKey),
  );
  const [favorites, setFavorites] = useState<SqlFavoriteEntry[]>(() =>
    readPersistedFavorites(storageKey),
  );

  // Persist history on every change so a crash / refresh
  // doesn't lose the most recent queries. The dependency on
  // `storageKey` rehydrates when the panel switches engines
  // mid-session (rare but cheap).
  useEffect(() => {
    writePersistedHistory(storageKey, history);
  }, [storageKey, history]);
  useEffect(() => {
    writePersistedFavorites(storageKey, favorites);
  }, [storageKey, favorites]);
  useEffect(() => {
    writePersistedTabs(storageKey, { tabs, activeId });
  }, [storageKey, tabs, activeId]);

  const activeTab = useMemo(
    () => tabs.find((t) => t.id === activeId) ?? tabs[0],
    [tabs, activeId],
  );
  const sql = activeTab?.sql ?? "";

  const setSql = useCallback((next: string) => {
    setTabs((prev) =>
      prev.map((t) => (t.id === activeId ? { ...t, sql: next, dirty: true } : t)),
    );
  }, [activeId]);

  const addTab = useCallback((seed?: { name?: string; sql?: string }) => {
    const id = makeId();
    setTabs((prev) => {
      const next: SqlTab = {
        id,
        name: seed?.name ?? `query ${prev.length + 1}`,
        sql: seed?.sql ?? "",
        dirty: false,
      };
      const all = [...prev, next];
      // Drop oldest non-active tab if we exceed the cap.
      if (all.length > MAX_TABS) {
        const dropIdx = all.findIndex((t) => t.id !== activeId);
        if (dropIdx >= 0) all.splice(dropIdx, 1);
      }
      return all;
    });
    setActiveId(id);
    return id;
  }, [activeId, makeId]);

  const closeTab = useCallback((id: string) => {
    setTabs((prev) => {
      if (prev.length <= 1) return prev;
      const idx = prev.findIndex((t) => t.id === id);
      if (idx < 0) return prev;
      const next = prev.filter((t) => t.id !== id);
      if (id === activeId) {
        const fallback = next[Math.min(idx, next.length - 1)];
        if (fallback) setActiveId(fallback.id);
      }
      return next;
    });
  }, [activeId]);

  /** Replace the active tab's SQL and rename it. Used when the user
   *  clicks a table in the schema tree — we re-purpose the current
   *  tab rather than spawning a new one (matches IntelliJ DataGrip). */
  const replaceActiveSql = useCallback((next: string, name?: string) => {
    setTabs((prev) =>
      prev.map((t) =>
        t.id === activeId ? { ...t, sql: next, dirty: false, name: name ?? t.name } : t,
      ),
    );
  }, [activeId]);

  const markActiveSaved = useCallback(() => {
    setTabs((prev) =>
      prev.map((t) => (t.id === activeId ? { ...t, dirty: false } : t)),
    );
  }, [activeId]);

  const pushHistory = useCallback((entry: SqlHistoryEntry) => {
    setHistory((prev) => [entry, ...prev].slice(0, MAX_HISTORY));
  }, []);

  /** Load a history entry into the active tab. */
  const loadHistory = useCallback((entry: SqlHistoryEntry) => {
    replaceActiveSql(entry.sql);
  }, [replaceActiveSql]);

  /** Wipe both in-memory history and any persisted copy. */
  const clearHistory = useCallback(() => {
    setHistory([]);
    if (storageKey && typeof window !== "undefined") {
      try {
        window.localStorage.removeItem(STORAGE_PREFIX + storageKey);
      } catch {
        /* private mode — best-effort */
      }
    }
  }, [storageKey]);

  /** Pin the currently-active SQL as a favorite. The label
   *  defaults to a truncated single-line preview of the SQL
   *  when the caller doesn't supply one. Duplicate-SQL pins
   *  bubble the existing entry to the top instead of inserting
   *  a second copy — same shape the history ring uses. */
  const addFavorite = useCallback(
    (entry: { sql: string; name?: string }) => {
      const trimmed = entry.sql.trim();
      if (!trimmed) return;
      const fallbackName =
        entry.name?.trim() ||
        trimmed.replace(/\s+/g, " ").slice(0, 60) || "query";
      const id =
        typeof crypto !== "undefined" && "randomUUID" in crypto
          ? (crypto as Crypto).randomUUID()
          : `fav-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      setFavorites((prev) => {
        const dup = prev.findIndex((f) => f.sql === trimmed);
        if (dup >= 0) {
          // Move to top with a fresh timestamp; preserve user's
          // existing label.
          const next = [...prev];
          const [hit] = next.splice(dup, 1);
          return [{ ...hit, savedAt: Date.now() }, ...next].slice(0, MAX_FAVORITES);
        }
        return [
          { id, name: fallbackName, sql: trimmed, savedAt: Date.now() },
          ...prev,
        ].slice(0, MAX_FAVORITES);
      });
    },
    [],
  );

  /** Remove a favorite by id. */
  const removeFavorite = useCallback((id: string) => {
    setFavorites((prev) => prev.filter((f) => f.id !== id));
  }, []);

  /** Load a favorite into the active tab — same UX as
   *  `loadHistory`, just sourced from the pinned list. */
  const loadFavorite = useCallback(
    (entry: SqlFavoriteEntry) => {
      replaceActiveSql(entry.sql, entry.name);
    },
    [replaceActiveSql],
  );

  return {
    tabs,
    activeTabId: activeId,
    activeTab,
    sql,
    history,
    favorites,
    setActiveTabId: setActiveId,
    setSql,
    addTab,
    closeTab,
    replaceActiveSql,
    markActiveSaved,
    pushHistory,
    loadHistory,
    clearHistory,
    addFavorite,
    removeFavorite,
    loadFavorite,
  };
}
