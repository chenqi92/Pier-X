// ── Smart-mode history ring ───────────────────────────────────────
// In-memory FIFO of commands the user has executed since the app
// started. Powers the M5 autosuggestion (gray inline suffix that
// arrow-right accepts) and feeds future M4+ completion sources.
//
// The ring is global (not per-tab) so a command typed in tab A is
// instantly suggestible from tab B. Cross-tab leakage is intentional
// — history feels like the user's, not the session's.
//
// Optional disk persistence (controlled by
// `useSettingsStore.terminalHistoryPersist`, off by default per
// PRODUCT-SPEC §4.2.1) seeds the ring from per-shell jsonl files
// at session-mount time and mirrors each pushed command back to
// disk. Backend filters out lines containing common credential
// keywords before writing, so the file never holds a real token /
// password — but the in-memory ring keeps them for the current
// session so autosuggest still works on freshly-typed `export
// FOO_TOKEN=...` commands.

import { create } from "zustand";
import {
  terminalHistoryClear,
  terminalHistoryLoad,
  terminalHistoryPush,
} from "../lib/terminalSmart";

const MAX_HISTORY = 500;

type HistoryState = {
  /** Most-recent command first. Capped at `MAX_HISTORY`. Empty
   *  strings and pure-whitespace inputs are never inserted. */
  ring: string[];
  /** Set of shell slugs we've already loaded the on-disk history
   *  for in this app session. Prevents re-hydrating the same shell
   *  from disk on every tab open. */
  hydratedShells: Set<string>;
  /** Push a freshly-submitted command into the ring. When `shell`
   *  is provided **and** `persist` is true, the line is also
   *  forwarded to the backend persistence command (which itself
   *  applies the credential-keyword filter). The in-memory ring
   *  always accepts the line regardless of filtering. */
  push: (cmd: string, options?: { shell?: string; persist?: boolean }) => void;
  /** Lazily load a shell's persisted history file from disk and
   *  fold the resulting commands into the ring. No-op if `shell`
   *  is already hydrated, or `persist` is false. Safe to call on
   *  every session mount; deduplicates internally. */
  hydrate: (shell: string, persist: boolean) => Promise<void>;
  /** Clear in-memory ring entries that match `shell`'s persisted
   *  file, and wipe the file itself. Surfaced through Settings →
   *  Terminal as a "Clear history" button (planned; not currently
   *  exposed). Idempotent. */
  clearShell: (shell: string) => Promise<void>;
};

/** Merge a new command into a most-recent-first ring with
 *  capacity bound + dedup. Pure helper so both `push` and
 *  `hydrate` share the same shape. */
function mergeIntoRing(ring: string[], cmd: string): string[] {
  if (ring[0] === cmd) return ring;
  const filtered = ring.filter((c) => c !== cmd);
  const next = [cmd, ...filtered];
  if (next.length > MAX_HISTORY) next.length = MAX_HISTORY;
  return next;
}

export const useTerminalHistoryStore = create<HistoryState>((set, get) => ({
  ring: [],
  hydratedShells: new Set(),
  push: (cmd, options) => {
    const trimmed = cmd.trim();
    if (!trimmed) return;
    set((s) => ({ ring: mergeIntoRing(s.ring, trimmed) }));
    // Fire-and-forget the disk write. Backend filters credential-
    // bearing lines on its own — we don't replicate the rule here
    // to avoid two places to keep in sync.
    if (options?.persist && options.shell) {
      void terminalHistoryPush(options.shell, trimmed).catch(() => {
        // Swallow IPC errors — persistence is opt-in and best-
        // effort; the in-memory ring is the source of truth for
        // the current session either way.
      });
    }
  },
  hydrate: async (shell, persist) => {
    if (!persist) return;
    if (!shell) return;
    if (get().hydratedShells.has(shell)) return;
    // Mark before the await to dedup concurrent calls (two tabs
    // for the same shell mounted in the same render cycle).
    set((s) => {
      const next = new Set(s.hydratedShells);
      next.add(shell);
      return { hydratedShells: next };
    });
    let entries: string[] = [];
    try {
      entries = await terminalHistoryLoad(shell);
    } catch {
      // Backend returned an error — fail soft and stay in-memory-
      // only. We've already marked the shell as hydrated so we
      // don't retry on every render.
      return;
    }
    if (entries.length === 0) return;
    set((s) => {
      // The ring stays most-recent-first. Disk entries arrive
      // sorted that way too (see history.rs `load`), but we may
      // already have entries from the current session that should
      // win on collision — apply oldest first so the most-recent
      // run-time pushes end up at the front.
      let next = s.ring;
      for (const cmd of entries.slice().reverse()) {
        next = mergeIntoRing(next, cmd);
      }
      return { ring: next };
    });
  },
  clearShell: async (shell) => {
    try {
      await terminalHistoryClear(shell);
    } catch {
      // Best effort — if the file's already gone or the platform
      // has no data dir, there's nothing left to do.
    }
    set((s) => {
      const next = new Set(s.hydratedShells);
      next.delete(shell);
      return { hydratedShells: next };
    });
  },
}));

/**
 * Find the best autosuggestion suffix for `prefix` against the
 * given history ring. Returns the bytes the UI should append in
 * muted gray; empty string when no usable match exists.
 *
 * Most-recent-wins: walks the ring from front to back, returning
 * the first entry that strictly extends `prefix`. Equality is not
 * a match — there's no useful suffix to suggest if the user has
 * already typed the exact command.
 */
export function suggestFromHistory(ring: string[], prefix: string): string {
  if (!prefix) return "";
  for (const entry of ring) {
    if (entry.length > prefix.length && entry.startsWith(prefix)) {
      return entry.slice(prefix.length);
    }
  }
  return "";
}
