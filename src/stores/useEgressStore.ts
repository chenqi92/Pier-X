import { create } from "zustand";
import { localizeError } from "../i18n/localizeMessage";
import { translate } from "../i18n/useI18n";
import * as cmd from "../lib/commands";
import type { EgressProfile } from "../lib/types";
import { useSettingsStore } from "./useSettingsStore";

type EgressStore = {
  profiles: EgressProfile[];
  loading: boolean;
  error: string;
  refresh: () => Promise<void>;
  save: (profile: EgressProfile) => Promise<void>;
  remove: (id: string) => Promise<void>;
  setBasicAuth: (credentialId: string, user: string, password: string) => Promise<void>;
  clearCredential: (credentialId: string) => Promise<void>;
  vpnStart: (id: string) => Promise<void>;
  vpnStop: (id: string) => Promise<void>;
};

function localize(error: unknown) {
  const locale = useSettingsStore.getState().locale;
  return localizeError(error, (key, vars) => translate(locale, key, vars));
}

export const useEgressStore = create<EgressStore>((set, get) => ({
  profiles: [],
  loading: false,
  error: "",

  refresh: async () => {
    set({ loading: true, error: "" });
    try {
      const profiles = await cmd.egressProfileList();
      set({ profiles, loading: false });
    } catch (e) {
      set({ error: localize(e), loading: false });
    }
  },

  save: async (profile) => {
    try {
      await cmd.egressProfileSave(profile);
      set({ error: "" });
      await get().refresh();
    } catch (e) {
      set({ error: localize(e) });
      throw e;
    }
  },

  remove: async (id) => {
    try {
      await cmd.egressProfileDelete(id);
      set({ error: "" });
      await get().refresh();
    } catch (e) {
      set({ error: localize(e) });
      throw e;
    }
  },

  setBasicAuth: async (credentialId, user, password) => {
    try {
      await cmd.egressSetBasicAuth(credentialId, user, password);
      set({ error: "" });
    } catch (e) {
      set({ error: localize(e) });
      throw e;
    }
  },

  clearCredential: async (credentialId) => {
    try {
      await cmd.egressClearCredential(credentialId);
      set({ error: "" });
    } catch (e) {
      set({ error: localize(e) });
      throw e;
    }
  },

  vpnStart: async (id) => {
    try {
      await cmd.egressVpnStart(id);
      set({ error: "" });
    } catch (e) {
      set({ error: localize(e) });
      throw e;
    }
  },

  vpnStop: async (id) => {
    try {
      await cmd.egressVpnStop(id);
      set({ error: "" });
    } catch (e) {
      set({ error: localize(e) });
      throw e;
    }
  },
}));
