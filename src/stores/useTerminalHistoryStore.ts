// ── Smart-mode history ring ───────────────────────────────────────
// In-memory FIFO of commands the user has executed since the app
// started. Powers the M5 autosuggestion (gray inline suffix that
// arrow-right accepts) and feeds future M4+ completion sources.
//
// Memory-only by design — PRODUCT-SPEC §4.2.1 commits to "history
// ring 默认仅内存". Disk persistence is a future opt-in: it would
// need sensitivity filtering (PASSWORD/TOKEN scrubbing), per-shell
// jsonl files, and an explicit Settings toggle. None of that lands
// in M5; the ring just lives in this store and dies when the app
// quits.
//
// The ring is global (not per-tab) so a command typed in tab A is
// instantly suggestible from tab B. Cross-tab leakage is intentional
// — history feels like the user's, not the session's.

import { create } from "zustand";

const MAX_HISTORY = 500;

type HistoryState = {
  /** Most-recent command first. Capped at `MAX_HISTORY`. Empty
   *  strings and pure-whitespace inputs are never inserted. */
  ring: string[];
  /** Push a freshly-submitted command into the ring. Trims
   *  whitespace, drops empty inputs, and de-duplicates against the
   *  most recent entry — so pressing Enter on the same line twice
   *  doesn't bloat the list. Other duplicates further back move to
   *  the front (most-recent-wins). */
  push: (cmd: string) => void;
};

export const useTerminalHistoryStore = create<HistoryState>((set) => ({
  ring: [],
  push: (cmd: string) => {
    const trimmed = cmd.trim();
    if (!trimmed) return;
    set((s) => {
      // Most-recent-duplicate fast path: zero churn when the user
      // re-runs the same command without anything else in between.
      if (s.ring[0] === trimmed) return s;
      const filtered = s.ring.filter((c) => c !== trimmed);
      const next = [trimmed, ...filtered];
      if (next.length > MAX_HISTORY) next.length = MAX_HISTORY;
      return { ring: next };
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
