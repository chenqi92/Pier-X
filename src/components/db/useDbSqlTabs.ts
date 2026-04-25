import { useCallback, useMemo, useRef, useState } from "react";
import type { SqlHistoryEntry, SqlTab } from "./DbSqlEditor";

/** Maximum number of tabs we keep open. Beyond this the oldest non-active
 *  tab is dropped — a small panel mounted in the right rail can't fit a
 *  long tab strip anyway. */
const MAX_TABS = 8;
/** Maximum history entries we retain in panel memory. */
const MAX_HISTORY = 50;

type UseDbSqlTabsArgs = {
  /** SQL the very first tab opens with. */
  initialSql: string;
  /** Display name for the first tab (e.g. "warehouse"). */
  initialName?: string;
};

/** Lightweight tabs + history state for the SQL editor. Keeps the
 *  editor a controlled component while letting panels share one
 *  consistent multi-tab model. */
export function useDbSqlTabs({ initialSql, initialName = "query" }: UseDbSqlTabsArgs) {
  const counter = useRef(1);
  const makeId = useCallback(() => `q${++counter.current}`, []);

  const [tabs, setTabs] = useState<SqlTab[]>(() => [
    { id: "q1", name: initialName, sql: initialSql, dirty: false },
  ]);
  const [activeId, setActiveId] = useState<string>("q1");
  const [history, setHistory] = useState<SqlHistoryEntry[]>([]);

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
  };
}
