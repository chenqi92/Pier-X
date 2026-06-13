import { create } from "zustand";

import {
  forgetElevationPassword,
  getElevationPassword,
  setElevationPassword,
} from "../lib/shellCommands";
import type { SshParams } from "../lib/shellCommands";
import { sshSetHostElevation } from "../lib/commands";

/** Stable host key — `user@host:port`. Same host accessed under
 *  different SSH users gets a separate credential, which matches
 *  how sudoers are configured per-user. */
export function sudoKeyFor(params: SshParams): string {
  return `${params.user}@${params.host}:${params.port}`;
}

/** Mirror an in-memory sudo password into the backend host-elevation
 *  map so the whole right side follows it. Fire-and-forget; failures are
 *  non-fatal (the per-command `sudoPassword` path still works). */
function syncHostElevation(params: SshParams, password: string | null): void {
  void sshSetHostElevation({
    host: params.host,
    port: params.port,
    user: params.user,
    authMode: params.authMode,
    password: params.password,
    keyPath: params.keyPath,
    savedConnectionIndex: params.savedConnectionIndex,
    sudoPassword: password,
  }).catch(() => {});
}

type HydrateState = "idle" | "pending" | "done";

type SudoStoreState = {
  /** In-memory password cache keyed by host. The L1 tier of a
   *  two-tier cache: this map is populated either when the user
   *  types a password into `SudoPasswordDialog` or when
   *  `hydrate(params)` lifts a value from the OS keychain (L2). */
  passwords: Record<string, string>;

  /** Per-host hydrate state, so a panel can call `hydrate` without
   *  worrying about double-firing the keychain read. `pending` is
   *  the in-flight marker; `done` is set even when the keychain
   *  had no entry, so we don't keep retrying empty hosts. */
  hydrateState: Record<string, HydrateState>;

  /** Read the cached password for a host. `null` when the user
   *  hasn't entered one yet (or has cleared it). Reads only the
   *  in-memory L1 cache — call `hydrate` first if you need the
   *  keychain L2 to be consulted. */
  get: (params: SshParams) => string | null;

  /** Synchronously set / replace the password for this session
   *  only. Does NOT touch the keychain. Used by panels that want
   *  to remember a password they just received from the user
   *  without persisting it to disk. */
  set: (params: SshParams, password: string) => void;

  /** Set the password for this session AND optionally persist it
   *  to the OS keychain. `remember=false` is equivalent to `set`.
   *  Returns once the keychain write completes (or fails — errors
   *  are swallowed, the L1 cache is always updated). */
  setPersistent: (
    params: SshParams,
    password: string,
    remember: boolean,
  ) => Promise<void>;

  /** If we haven't yet checked the keychain for this host, ask the
   *  backend and lift any persisted entry into the L1 cache.
   *  Idempotent: subsequent calls for the same host are no-ops. */
  hydrate: (params: SshParams) => Promise<void>;

  /** Drop the cached password — used by the "forget" affordance and
   *  by the disconnect handler when a tab closes. By default only
   *  clears the L1 (in-memory) cache; pass `forgetPersistent=true`
   *  to also delete the keychain entry. */
  clear: (params: SshParams, forgetPersistent?: boolean) => Promise<void>;

  /** Drop every cached password from the L1 cache. Wired to "Sign
   *  out" / "Disconnect all" flows. Does NOT touch the keychain —
   *  there's no enumerate API and we don't want a single click to
   *  silently wipe persistent entries the user explicitly opted
   *  into. Use `clear(params, true)` per host to do that. */
  clearAll: () => void;
};

export const useSudoStore = create<SudoStoreState>((set, get) => ({
  passwords: {},
  hydrateState: {},
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
    // Mirror to the backend host-elevation map so every right-side path
    // (detection, monitor, panel reads/writes) follows it — not just the
    // panels that explicitly thread `sudoPassword`.
    void syncHostElevation(params, password || null);
  },
  setPersistent: async (params, password, remember) => {
    const key = sudoKeyFor(params);
    set((s) => {
      if (!password) {
        const { [key]: _omit, ...rest } = s.passwords;
        return { passwords: rest };
      }
      return { passwords: { ...s.passwords, [key]: password } };
    });
    // Mark hydrated so subsequent hydrate() calls don't overwrite
    // what the user just typed with whatever is on disk.
    set((s) => ({ hydrateState: { ...s.hydrateState, [key]: "done" } }));
    syncHostElevation(params, password || null);
    if (!remember) {
      // Caller chose not to persist. Best-effort: remove any
      // stale keychain entry for the same host so the next hydrate
      // doesn't pick up an old password.
      try {
        await forgetElevationPassword(params.user, params.host, params.port);
      } catch (e) {
        console.warn("forget elevation password failed", e);
      }
      return;
    }
    try {
      await setElevationPassword(
        params.user,
        params.host,
        params.port,
        password,
      );
    } catch (e) {
      // Keychain rejected the write (Linux secret-service down,
      // Windows CM group policy, …). The password is still in
      // L1 for this session; the user just won't get it back on
      // next launch. Log and move on — surfacing this as a hard
      // error would be worse UX than silently degrading.
      console.warn("persist elevation password failed", e);
    }
  },
  hydrate: async (params) => {
    const key = sudoKeyFor(params);
    const state = get().hydrateState[key];
    if (state === "pending" || state === "done") return;
    set((s) => ({ hydrateState: { ...s.hydrateState, [key]: "pending" } }));
    try {
      const stored = await getElevationPassword(
        params.user,
        params.host,
        params.port,
      );
      if (stored && stored.length > 0) {
        // Only fill the L1 cache if the panel hasn't already
        // recorded a fresher password (race: hydrate raced with
        // a manual entry).
        set((s) => {
          if (s.passwords[key]) return s;
          return { passwords: { ...s.passwords, [key]: stored } };
        });
        syncHostElevation(params, stored);
      }
    } catch (e) {
      console.warn("hydrate elevation password failed", e);
    } finally {
      set((s) => ({ hydrateState: { ...s.hydrateState, [key]: "done" } }));
    }
  },
  clear: async (params, forgetPersistent = false) => {
    const key = sudoKeyFor(params);
    set((s) => {
      if (!(key in s.passwords)) return s;
      const { [key]: _omit, ...rest } = s.passwords;
      return { passwords: rest };
    });
    syncHostElevation(params, null);
    if (forgetPersistent) {
      try {
        await forgetElevationPassword(params.user, params.host, params.port);
      } catch (e) {
        console.warn("forget elevation password failed", e);
      }
      // Reset hydrate state so a future call re-checks the keychain
      // (now expected to be empty).
      set((s) => {
        const { [key]: _omit, ...rest } = s.hydrateState;
        return { hydrateState: rest };
      });
    }
  },
  clearAll: () => set({ passwords: {} }),
}));
