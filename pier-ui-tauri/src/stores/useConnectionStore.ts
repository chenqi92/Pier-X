import { create } from "zustand";
import type { SavedSshConnection } from "../lib/types";
import * as cmd from "../lib/commands";

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
};

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
      set({ error: String(e), loading: false });
    }
  },

  save: async (params) => {
    try {
      await cmd.sshConnectionSave(params);
      set({ error: "" });
      await get().refresh();
    } catch (e) {
      set({ error: String(e) });
      throw e;
    }
  },

  update: async (params) => {
    try {
      await cmd.sshConnectionUpdate(params);
      set({ error: "" });
      await get().refresh();
    } catch (e) {
      set({ error: String(e) });
      throw e;
    }
  },

  remove: async (index) => {
    try {
      await cmd.sshConnectionDelete(index);
      set({ error: "" });
      await get().refresh();
    } catch (e) {
      set({ error: String(e) });
      throw e;
    }
  },
}));
