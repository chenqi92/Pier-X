import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { SqlHistoryEntry, SqlTab } from "./DbSqlEditor";

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

  const [tabs, setTabs] = useState<SqlTab[]>(() => [
    { id: "q1", name: initialName, sql: initialSql, dirty: false },
  ]);
  const [activeId, setActiveId] = useState<string>("q1");
  const [history, setHistory] = useState<SqlHistoryEntry[]>(() =>
    readPersistedHistory(storageKey),
  );

  // Persist history on every change so a crash / refresh
  // doesn't lose the most recent queries. The dependency on
  // `storageKey` rehydrates when the panel switches engines
  // mid-session (rare but cheap).
  useEffect(() => {
    writePersistedHistory(storageKey, history);
  }, [storageKey, history]);

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

  return {
    tabs,
    activeTabId: activeId,
    activeTab,
    sql,
    history,
    setActiveTabId: setActiveId,
    setSql,
    addTab,
    closeTab,
    replaceActiveSql,
    markActiveSaved,
    pushHistory,
    loadHistory,
    clearHistory,
  };
}
