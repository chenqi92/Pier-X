// ── Smart-mode caches ──────────────────────────────────────────────
// Cross-tab in-memory caches for smart-mode lookups so each unique
// command name only crosses the Tauri IPC boundary once per app
// session. Validation results don't change while Pier-X is running
// (PATH and the builtins list are stable), so a process-lifetime
// cache is safe; user-visible effects are bounded by reload.
//
// The cache lives at the store level rather than in component state
// so multiple terminal tabs share results — typing `git` in tab 1
// after typing it in tab 2 hits the cache, no roundtrip.

import { create } from "zustand";
import {
  terminalValidateCommand,
  type CommandValidation,
} from "../lib/terminalSmart";

type SmartState = {
  /** Resolved validations keyed by command name. */
  cache: Map<string, CommandValidation>;
  /** Names with an in-flight invoke. Prevents the same name from
   *  being requested concurrently when the overlay re-renders mid-
   *  network-call. */
  pending: Set<string>;
  /**
   * Read-through accessor. If `name` is cached, returns the
   * validation. Otherwise kicks off a background invoke and
   * returns `undefined`; the store updates after the invoke
   * resolves, which triggers a re-render in any subscribed
   * component.
   */
  validateCommand: (name: string) => CommandValidation | undefined;
};

export const useTerminalSmartStore = create<SmartState>((set, get) => ({
  cache: new Map(),
  pending: new Set(),
  validateCommand: (name: string) => {
    const cached = get().cache.get(name);
    if (cached) return cached;
    if (get().pending.has(name)) return undefined;

    // Mark pending synchronously so the next render in the same
    // tick sees us as in-flight and doesn't re-fire.
    set((s) => {
      const pending = new Set(s.pending);
      pending.add(name);
      return { pending };
    });

    void terminalValidateCommand(name)
      .then((v) => {
        set((s) => {
          const cache = new Map(s.cache);
          cache.set(name, v);
          const pending = new Set(s.pending);
          pending.delete(name);
          return { cache, pending };
        });
      })
      .catch(() => {
        // Drop the pending flag so a later render can retry. We
        // don't cache failures — a transient IPC hiccup shouldn't
        // permanently mark a name as unstyled.
        set((s) => {
          const pending = new Set(s.pending);
          pending.delete(name);
          return { pending };
        });
      });

    return undefined;
  },
}));
