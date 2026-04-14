import { create } from "zustand";
import type { RightTool, TabState } from "../lib/types";

type TabStore = {
  tabs: TabState[];
  activeTabId: string | null;
  addTab: (partial: Partial<TabState> & { backend: TabState["backend"] }) => string;
  closeTab: (id: string) => void;
  closeOtherTabs: (id: string) => void;
  setActiveTab: (id: string) => void;
  updateTab: (id: string, patch: Partial<TabState>) => void;
  moveTab: (fromIndex: number, toIndex: number) => void;
  setTabColor: (id: string, color: number) => void;
  setTabRightTool: (id: string, tool: RightTool) => void;
};

let nextId = 1;
function genId() {
  return `tab-${nextId++}`;
}

function makeDefaultTab(
  partial: Partial<TabState> & { backend: TabState["backend"] },
): TabState {
  return {
    id: genId(),
    title: partial.title ?? (partial.backend === "local" ? "Terminal" : "SSH"),
    tabColor: partial.tabColor ?? -1,
    backend: partial.backend,
    sshHost: partial.sshHost ?? "",
    sshPort: partial.sshPort ?? 22,
    sshUser: partial.sshUser ?? "",
    sshAuthMode: partial.sshAuthMode ?? "password",
    sshPassword: partial.sshPassword ?? "",
    sshKeyPath: partial.sshKeyPath ?? "",
    terminalSessionId: partial.terminalSessionId ?? null,
    rightTool: partial.rightTool ?? (partial.backend === "local" ? "git" : "monitor"),
    redisHost: partial.redisHost ?? "127.0.0.1",
    redisPort: partial.redisPort ?? 6379,
    redisDb: partial.redisDb ?? 0,
    mysqlHost: partial.mysqlHost ?? "127.0.0.1",
    mysqlPort: partial.mysqlPort ?? 3306,
    mysqlUser: partial.mysqlUser ?? "root",
    mysqlPassword: partial.mysqlPassword ?? "",
    mysqlDatabase: partial.mysqlDatabase ?? "",
    pgHost: partial.pgHost ?? "127.0.0.1",
    pgPort: partial.pgPort ?? 5432,
    pgUser: partial.pgUser ?? "postgres",
    pgPassword: partial.pgPassword ?? "",
    pgDatabase: partial.pgDatabase ?? "",
    logCommand: partial.logCommand ?? "",
    markdownPath: partial.markdownPath ?? "",
    startupCommand: partial.startupCommand ?? "",
  };
}

export const useTabStore = create<TabStore>((set, get) => ({
  tabs: [],
  activeTabId: null,

  addTab: (partial) => {
    const tab = makeDefaultTab(partial);
    set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tab.id }));
    return tab.id;
  },

  closeTab: (id) => {
    set((s) => {
      const idx = s.tabs.findIndex((t) => t.id === id);
      if (idx < 0) return s;
      const next = s.tabs.filter((t) => t.id !== id);
      let nextActive = s.activeTabId;
      if (s.activeTabId === id) {
        if (next.length === 0) {
          nextActive = null;
        } else if (idx < next.length) {
          nextActive = next[idx].id;
        } else {
          nextActive = next[next.length - 1].id;
        }
      }
      return { tabs: next, activeTabId: nextActive };
    });
  },

  closeOtherTabs: (id) => {
    set((s) => ({
      tabs: s.tabs.filter((t) => t.id === id),
      activeTabId: id,
    }));
  },

  setActiveTab: (id) => set({ activeTabId: id }),

  updateTab: (id, patch) => {
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === id ? { ...t, ...patch } : t)),
    }));
  },

  moveTab: (fromIndex, toIndex) => {
    set((s) => {
      const tabs = [...s.tabs];
      const [moved] = tabs.splice(fromIndex, 1);
      tabs.splice(toIndex, 0, moved);
      return { tabs };
    });
  },

  setTabColor: (id, color) => {
    get().updateTab(id, { tabColor: color });
  },

  setTabRightTool: (id, tool) => {
    get().updateTab(id, { rightTool: tool });
  },
}));
