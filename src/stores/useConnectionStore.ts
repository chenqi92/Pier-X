import { create } from "zustand";
import { localizeError } from "../i18n/localizeMessage";
import { translate } from "../i18n/useI18n";
import type { SavedSshConnection } from "../lib/types";
import * as cmd from "../lib/commands";
import { useSettingsStore } from "./useSettingsStore";

type ConnectionStore = {
  connections: SavedSshConnection[];
  loading: boolean;
  error: string;
  refresh: () => Promise<void>;
  save: (params: {
    name: string;
    host: string;
    port: number;
    user: string;
    authKind: string;
    password: string;
    keyPath: string;
  }) => Promise<void>;
  update: (params: {
    index: number;
    name: string;
    host: string;
    port: number;
    user: string;
    authKind: string;
    password: string;
    keyPath: string;
  }) => Promise<void>;
  remove: (index: number) => Promise<void>;
  /** Atomic reorder + group-reassign across the whole list. */
  reorder: (order: number[], groups: Array<string | null>) => Promise<void>;
  /** Rename a group (to === null strips the group from its members). */
  renameGroup: (from: string, to: string | null) => Promise<void>;
};

function localizeStoreError(error: unknown) {
  const locale = useSettingsStore.getState().locale;
  return localizeError(error, (key, vars) => translate(locale, key, vars));
}

export const useConnectionStore = create<ConnectionStore>((set, get) => ({
  connections: [],
  loading: false,
  error: "",

  refresh: async () => {
    set({ loading: true, error: "" });
    try {
      const connections = await cmd.sshConnectionsList();
      set({ connections, loading: false });
    } catch (e) {
      set({ error: localizeStoreError(e), loading: false });
    }
  },

  save: async (params) => {
    try {
      await cmd.sshConnectionSave(params);
      set({ error: "" });
      await get().refresh();
    } catch (e) {
      set({ error: localizeStoreError(e) });
      throw e;
    }
  },

  update: async (params) => {
    try {
      await cmd.sshConnectionUpdate(params);
      set({ error: "" });
      await get().refresh();
    } catch (e) {
      set({ error: localizeStoreError(e) });
      throw e;
    }
  },

  remove: async (index) => {
    try {
      await cmd.sshConnectionDelete(index);
      set({ error: "" });
      await get().refresh();
    } catch (e) {
      set({ error: localizeStoreError(e) });
      throw e;
    }
  },

  reorder: async (order, groups) => {
    // Optimistic local update so drag-drop feels snappy; the
    // refresh() below reconciles indices with the backend.
    const current = get().connections;
    const next = order.map((oldIdx, slot) => {
      const src = current[oldIdx];
      if (!src) return null;
      const g = (groups[slot] ?? "").trim();
      return { ...src, index: slot, group: g ? g : null };
    }).filter(Boolean) as SavedSshConnection[];
    if (next.length === current.length) set({ connections: next });
    try {
      await cmd.sshConnectionsReorder(order, groups);
      set({ error: "" });
      await get().refresh();
    } catch (e) {
      set({ error: localizeStoreError(e) });
      await get().refresh();
      throw e;
    }
  },

  renameGroup: async (from, to) => {
    try {
      await cmd.sshGroupRename(from, to);
      set({ error: "" });
      await get().refresh();
    } catch (e) {
      set({ error: localizeStoreError(e) });
      throw e;
    }
  },
}));
