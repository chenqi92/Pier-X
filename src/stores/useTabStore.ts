import { create } from "zustand";
import * as cmd from "../lib/commands";
import { translate } from "../i18n/useI18n";
import { useSettingsStore } from "./useSettingsStore";
import type { RightTool, TabState } from "../lib/types";
import { DEFAULT_LOG_SOURCE } from "../lib/types";

type TabStore = {
  tabs: TabState[];
  activeTabId: string | null;
  addTab: (partial: Partial<TabState> & { backend: TabState["backend"] }) => string;
  closeTab: (id: string) => void;
  closeOtherTabs: (id: string) => void;
  closeTabsToLeft: (id: string) => void;
  closeTabsToRight: (id: string) => void;
  setActiveTab: (id: string) => void;
  updateTab: (id: string, patch: Partial<TabState>) => void;
  moveTab: (fromIndex: number, toIndex: number) => void;
  setTabColor: (id: string, color: number) => void;
  setTabRightTool: (id: string, tool: RightTool) => void;
};

const TABS_STORAGE_KEY = "pierx:tabs-v1";

type PersistedShape = {
  tabs: TabState[];
  activeTabId: string | null;
};

// Fields that must be reset on rehydration — either runtime handles
// (terminal session / ssh tunnel IDs) that are invalid after reload,
// or plaintext secrets that should never hit localStorage. Saved
// connections keep `sshSavedConnectionIndex`; the backend pulls the
// actual password from the OS keyring on reconnect.
function scrubRuntimeFields(tab: TabState): TabState {
  return {
    ...tab,
    terminalSessionId: null,
    sshPassword: "",
    redisPassword: "",
    redisTunnelId: null,
    redisTunnelPort: null,
    mysqlPassword: "",
    mysqlTunnelId: null,
    mysqlTunnelPort: null,
    pgPassword: "",
    pgTunnelId: null,
    pgTunnelPort: null,
  };
}

function loadPersisted(): PersistedShape {
  try {
    const raw = localStorage.getItem(TABS_STORAGE_KEY);
    if (!raw) return { tabs: [], activeTabId: null };
    const parsed = JSON.parse(raw) as PersistedShape;
    if (!parsed || !Array.isArray(parsed.tabs)) {
      return { tabs: [], activeTabId: null };
    }
    const tabs = parsed.tabs.map(scrubRuntimeFields);
    const activeTabId =
      parsed.activeTabId && tabs.some((t) => t.id === parsed.activeTabId)
        ? parsed.activeTabId
        : tabs[0]?.id ?? null;
    return { tabs, activeTabId };
  } catch {
    return { tabs: [], activeTabId: null };
  }
}

// Debounced so a burst of mutations (session id assignment after
// terminal spawn, rapid tab color flicker, ResizeObserver-driven
// state churn) produces at most one disk write per tick window.
let saveTimer: ReturnType<typeof setTimeout> | null = null;
let pendingState: PersistedShape | null = null;
function flushSave() {
  saveTimer = null;
  if (!pendingState) return;
  try {
    const payload: PersistedShape = {
      tabs: pendingState.tabs.map(scrubRuntimeFields),
      activeTabId: pendingState.activeTabId,
    };
    localStorage.setItem(TABS_STORAGE_KEY, JSON.stringify(payload));
  } catch {
    /* quota / serialization failures are non-fatal */
  }
  pendingState = null;
}
function savePersisted(state: PersistedShape) {
  pendingState = state;
  if (saveTimer !== null) return;
  saveTimer = setTimeout(flushSave, 250);
}

// Module-scope counter for tab id generation. Seeded from any
// persisted state so rehydrated ids (`tab-5`) don't collide with
// fresh ones (`tab-1`).
let nextId = 1;
function genId() {
  return `tab-${nextId++}`;
}
function bumpNextIdFrom(tabs: TabState[]) {
  let max = 0;
  for (const t of tabs) {
    const m = /^tab-(\d+)$/.exec(t.id);
    if (m) {
      const n = Number.parseInt(m[1], 10);
      if (Number.isFinite(n) && n > max) max = n;
    }
  }
  nextId = max + 1;
}

function closeTunnel(tunnelId: string | null | undefined) {
  if (!tunnelId) {
    return;
  }
  void cmd.sshTunnelClose(tunnelId).catch(() => {});
}

function closeTabTunnels(tab: TabState | undefined) {
  if (!tab) {
    return;
  }
  closeTunnel(tab.redisTunnelId);
  closeTunnel(tab.mysqlTunnelId);
  closeTunnel(tab.pgTunnelId);
}

function makeDefaultTab(
  partial: Partial<TabState> & { backend: TabState["backend"] },
): TabState {
  const locale = useSettingsStore.getState().locale;
  return {
    id: genId(),
    title:
      partial.title ??
      translate(locale, partial.backend === "local" ? "Terminal" : "SSH"),
    tabColor: partial.tabColor ?? -1,
    backend: partial.backend,
    sshHost: partial.sshHost ?? "",
    sshPort: partial.sshPort ?? 22,
    sshUser: partial.sshUser ?? "",
    sshAuthMode: partial.sshAuthMode ?? "password",
    sshPassword: partial.sshPassword ?? "",
    sshKeyPath: partial.sshKeyPath ?? "",
    sshSavedConnectionIndex: partial.sshSavedConnectionIndex ?? null,
    terminalSessionId: partial.terminalSessionId ?? null,
    rightTool: partial.rightTool ?? (partial.backend === "local" ? "markdown" : "monitor"),
    redisHost: partial.redisHost ?? "127.0.0.1",
    redisPort: partial.redisPort ?? 6379,
    redisDb: partial.redisDb ?? 0,
    redisUser: partial.redisUser ?? "",
    redisPassword: partial.redisPassword ?? "",
    redisTunnelId: partial.redisTunnelId ?? null,
    redisTunnelPort: partial.redisTunnelPort ?? null,
    mysqlHost: partial.mysqlHost ?? "127.0.0.1",
    mysqlPort: partial.mysqlPort ?? 3306,
    mysqlUser: partial.mysqlUser ?? "root",
    mysqlPassword: partial.mysqlPassword ?? "",
    mysqlDatabase: partial.mysqlDatabase ?? "",
    mysqlTunnelId: partial.mysqlTunnelId ?? null,
    mysqlTunnelPort: partial.mysqlTunnelPort ?? null,
    pgHost: partial.pgHost ?? "127.0.0.1",
    pgPort: partial.pgPort ?? 5432,
    pgUser: partial.pgUser ?? "postgres",
    pgPassword: partial.pgPassword ?? "",
    pgDatabase: partial.pgDatabase ?? "",
    pgTunnelId: partial.pgTunnelId ?? null,
    pgTunnelPort: partial.pgTunnelPort ?? null,
    mysqlActiveCredentialId: partial.mysqlActiveCredentialId ?? null,
    pgActiveCredentialId: partial.pgActiveCredentialId ?? null,
    redisActiveCredentialId: partial.redisActiveCredentialId ?? null,
    sqliteActiveCredentialId: partial.sqliteActiveCredentialId ?? null,
    logCommand: partial.logCommand ?? "",
    logSource: partial.logSource ?? { ...DEFAULT_LOG_SOURCE },
    markdownPath: partial.markdownPath ?? "",
    startupCommand: partial.startupCommand ?? "",
    dockerRegistryMirror: partial.dockerRegistryMirror ?? "",
    dockerPullProxy: partial.dockerPullProxy ?? "",
    nestedSshTarget: partial.nestedSshTarget ?? null,
  };
}

const initialPersisted = loadPersisted();
bumpNextIdFrom(initialPersisted.tabs);

if (typeof window !== "undefined") {
  window.addEventListener("beforeunload", () => {
    if (saveTimer !== null) {
      clearTimeout(saveTimer);
      flushSave();
    }
  });
}

export const useTabStore = create<TabStore>((set, get) => ({
  tabs: initialPersisted.tabs,
  activeTabId: initialPersisted.activeTabId,

  addTab: (partial) => {
    const tab = makeDefaultTab(partial);
    set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tab.id }));
    savePersisted(get());
    return tab.id;
  },

  closeTab: (id) => {
    closeTabTunnels(get().tabs.find((t) => t.id === id));
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
    savePersisted(get());
  },

  closeOtherTabs: (id) => {
    get().tabs.filter((t) => t.id !== id).forEach(closeTabTunnels);
    set((s) => ({
      tabs: s.tabs.filter((t) => t.id === id),
      activeTabId: id,
    }));
    savePersisted(get());
  },

  closeTabsToLeft: (id) => {
    const { tabs, activeTabId } = get();
    const idx = tabs.findIndex((t) => t.id === id);
    if (idx <= 0) return;
    tabs.slice(0, idx).forEach(closeTabTunnels);
    const next = tabs.slice(idx);
    const keepActive = next.some((t) => t.id === activeTabId);
    set({ tabs: next, activeTabId: keepActive ? activeTabId : id });
    savePersisted(get());
  },

  closeTabsToRight: (id) => {
    const { tabs, activeTabId } = get();
    const idx = tabs.findIndex((t) => t.id === id);
    if (idx < 0 || idx === tabs.length - 1) return;
    tabs.slice(idx + 1).forEach(closeTabTunnels);
    const next = tabs.slice(0, idx + 1);
    const keepActive = next.some((t) => t.id === activeTabId);
    set({ tabs: next, activeTabId: keepActive ? activeTabId : id });
    savePersisted(get());
  },

  setActiveTab: (id) => {
    set({ activeTabId: id });
    savePersisted(get());
  },

  updateTab: (id, patch) => {
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === id ? { ...t, ...patch } : t)),
    }));
    savePersisted(get());
  },

  moveTab: (fromIndex, toIndex) => {
    set((s) => {
      const tabs = [...s.tabs];
      const [moved] = tabs.splice(fromIndex, 1);
      tabs.splice(toIndex, 0, moved);
      return { tabs };
    });
    savePersisted(get());
  },

  setTabColor: (id, color) => {
    get().updateTab(id, { tabColor: color });
  },

  setTabRightTool: (id, tool) => {
    get().updateTab(id, { rightTool: tool });
  },
}));
