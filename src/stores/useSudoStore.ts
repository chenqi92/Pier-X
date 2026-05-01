import { create } from "zustand";

import type { SshParams } from "../lib/commands";

/** Stable host key — `user@host:port`. Same host accessed under
 *  different SSH users gets a separate credential, which matches
 *  how sudoers are configured per-user. */
export function sudoKeyFor(params: SshParams): string {
  return `${params.user}@${params.host}:${params.port}`;
}

type SudoStoreState = {
  /** In-memory password cache keyed by host. Never persisted: the
   *  store is a plain `create` (no `persist` middleware). On every
   *  app launch we start with `{}` so a stolen disk image can't
   *  yield a sudo password. */
  passwords: Record<string, string>;

  /** Read the cached password for a host. `null` when the user
   *  hasn't entered one yet (or has cleared it). */
  get: (params: SshParams) => string | null;

  /** Set / replace the password for a host. Empty string clears
   *  the entry — same effect as `clear`. */
  set: (params: SshParams, password: string) => void;

  /** Drop the cached password — used by the "forget" affordance and
   *  by the disconnect handler when a tab closes. */
  clear: (params: SshParams) => void;

  /** Drop every cached password. Wired to "Sign out" / "Disconnect
   *  all" flows so a shared workstation can be reset in one click. */
  clearAll: () => void;
};

export const useSudoStore = create<SudoStoreState>((set, get) => ({
  passwords: {},
  get: (params) => {
    const key = sudoKeyFor(params);
    return get().passwords[key] ?? null;
  },
  set: (params, password) => {
    const key = sudoKeyFor(params);
    set((s) => {
      if (!password) {
        const { [key]: _omit, ...rest } = s.passwords;
        return { passwords: rest };
      }
      return { passwords: { ...s.passwords, [key]: password } };
    });
  },
  clear: (params) => {
    const key = sudoKeyFor(params);
    set((s) => {
      if (!(key in s.passwords)) return s;
      const { [key]: _omit, ...rest } = s.passwords;
      return { passwords: rest };
    });
  },
  clearAll: () => set({ passwords: {} }),
}));
