import { create } from "zustand";

const STORAGE_KEY = "pierx:recent-connections";

type RecentMap = Record<number, number>; // connection index → last-used ms epoch

type RecentStore = {
  recents: RecentMap;
  touch: (index: number) => void;
  clear: () => void;
};

function loadInitial(): RecentMap {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object") return parsed as RecentMap;
  } catch {}
  return {};
}

function persist(map: RecentMap) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(map));
  } catch {}
}

export const useRecentConnectionsStore = create<RecentStore>((set, get) => ({
  recents: loadInitial(),
  touch: (index) => {
    const next = { ...get().recents, [index]: Date.now() };
    persist(next);
    set({ recents: next });
  },
  clear: () => {
    persist({});
    set({ recents: {} });
  },
}));
