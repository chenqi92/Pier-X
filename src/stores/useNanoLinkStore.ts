// ── NanoLink Panel Cache ──────────────────────────────────────────
//
// Like the Docker panel, NanoLinkPanel is mounted conditionally in
// RightSidebar — switching tools destroys and recreates it, and
// StrictMode double-invokes effects. We cache the role/status probe in
// a zustand store keyed by remote identity so remounting renders from
// cache and an in-flight guard collapses the StrictMode double-invoke
// (and rapid refreshes) into one request.
//
// Only the lightweight `status` probe is cached here. The role-specific
// data (agent status text, server summary / agent list) is loaded by the
// panel on demand and kept in local component state — it's user-driven
// (refresh / login / tab switch) and short-lived.

import { create } from "zustand";
import { effectiveSshTarget, type TabState } from "../lib/types";
import type { NanoLinkStatus } from "../lib/commands";

/** Freshness window before a click re-probes instead of rendering cache. */
const STALE_MS = 20_000;

export type NanoLinkSnapshot = {
  status: NanoLinkStatus | null;
  lastFetchedAt: number;
  error: string;
  /** Shared promise so concurrent callers coalesce. */
  inFlight: Promise<void> | null;
};

type Key = string;

// Frozen singleton for missing keys — a fresh object per read trips
// `useSyncExternalStore`'s loop detector (same pattern as useDockerStore).
const EMPTY: NanoLinkSnapshot = Object.freeze({
  status: null,
  lastFetchedAt: 0,
  error: "",
  inFlight: null,
}) as NanoLinkSnapshot;

export function nanolinkKeyForTab(tab: TabState): Key {
  const target = effectiveSshTarget(tab);
  if (!target) return "local";
  return `ssh:${target.user}@${target.host}:${target.port}`;
}

type State = {
  snapshots: Record<Key, NanoLinkSnapshot>;
  get: (key: Key) => NanoLinkSnapshot;
  isStale: (key: Key) => boolean;
  setStatus: (key: Key, status: NanoLinkStatus) => void;
  setError: (key: Key, error: string) => void;
  /** Fetch + cache the role/status. Concurrent callers share the fetch. */
  refresh: (
    key: Key,
    fetcher: () => Promise<NanoLinkStatus>,
    force?: boolean,
  ) => Promise<void>;
  invalidate: (key: Key) => void;
};

export const useNanoLinkStore = create<State>((set, get) => ({
  snapshots: {},

  get: (key) => get().snapshots[key] ?? EMPTY,

  isStale: (key) => {
    const snap = get().snapshots[key];
    if (!snap || !snap.status) return true;
    return Date.now() - snap.lastFetchedAt > STALE_MS;
  },

  setStatus: (key, status) =>
    set((s) => ({
      snapshots: {
        ...s.snapshots,
        [key]: {
          ...(s.snapshots[key] ?? EMPTY),
          status,
          lastFetchedAt: Date.now(),
          error: "",
        },
      },
    })),

  setError: (key, error) =>
    set((s) => ({
      snapshots: {
        ...s.snapshots,
        [key]: { ...(s.snapshots[key] ?? EMPTY), error },
      },
    })),

  refresh: async (key, fetcher, force) => {
    const current = get().snapshots[key];
    if (current?.inFlight) {
      if (!force) return current.inFlight;
      try {
        await current.inFlight;
      } catch {
        // Ignore — we're about to refetch anyway.
      }
    }
    if (!force && current?.status && !get().isStale(key)) return;

    const run = (async () => {
      try {
        const status = await fetcher();
        set((s) => ({
          snapshots: {
            ...s.snapshots,
            [key]: {
              ...(s.snapshots[key] ?? EMPTY),
              status,
              lastFetchedAt: Date.now(),
              error: "",
            },
          },
        }));
      } catch (e) {
        set((s) => ({
          snapshots: {
            ...s.snapshots,
            [key]: { ...(s.snapshots[key] ?? EMPTY), error: String(e) },
          },
        }));
      }
    })();

    set((s) => ({
      snapshots: {
        ...s.snapshots,
        [key]: { ...(s.snapshots[key] ?? EMPTY), inFlight: run },
      },
    }));

    await run;
    set((s) => {
      const prev = s.snapshots[key];
      if (!prev) return s;
      return { snapshots: { ...s.snapshots, [key]: { ...prev, inFlight: null } } };
    });
  },

  invalidate: (key) =>
    set((s) => {
      const next = { ...s.snapshots };
      delete next[key];
      return { snapshots: next };
    }),
}));
