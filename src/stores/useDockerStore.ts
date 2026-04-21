// ── Docker Panel Cache ────────────────────────────────────────────
//
// The Docker panel is mounted conditionally in RightSidebar — switching
// between tools (git / monitor / docker / …) destroys and recreates the
// component. In StrictMode this also double-invokes `useEffect`.
//
// Every remount would otherwise re-run the whole overview pipeline:
//   1. docker ps + images + volumes + networks (1–2s over SSH)
//   2. docker stats --no-stream (1–2s Docker sampling window)
//   3. docker system df -v (0.5–5s depending on host load)
//
// Rather than fight the mounting topology, we cache the data in a
// zustand store keyed by remote identity. Remounting the panel then
// renders instantly from cache and only fetches when the data is stale.
// A per-key in-flight guard collapses StrictMode's double-invoke (and
// any accidental rapid-refresh) into one network request.

import { create } from "zustand";
import type { DockerOverview, DockerVolumeView, TabState } from "../lib/types";

/** Milliseconds of freshness before we treat a snapshot as stale. A click
 *  on the Docker tool within this window re-renders from cache; outside
 *  it, the store fires a background refresh. */
const STALE_MS = 30_000;

export type DockerSnapshot = {
  overview: DockerOverview | null;
  lastFetchedAt: number;
  error: string;
  /** Shared promise for callers that want to await the active fetch. */
  inFlight: Promise<void> | null;
  /** Cached `ls -la` output per volume name so expand state survives
   *  a panel remount. */
  volumeFiles: Record<string, string>;
};

type Key = string;

function emptySnapshot(): DockerSnapshot {
  return {
    overview: null,
    lastFetchedAt: 0,
    error: "",
    inFlight: null,
    volumeFiles: {},
  };
}

export function dockerKeyForTab(tab: TabState): Key {
  if (tab.backend === "local") return "local";
  // Stable identity for the remote, independent of the tab that opened
  // it — if the user has two tabs on the same host they share cache.
  return `ssh:${tab.sshUser}@${tab.sshHost}:${tab.sshPort}`;
}

export type DockerFetchers = {
  /** Fast path: base listings only. Must resolve quickly. */
  fetchOverview: () => Promise<DockerOverview>;
  /** Background enrichment firing concurrently after the overview
   *  lands; failures are swallowed (rows keep placeholder values). */
  enrich?: (overview: DockerOverview) => Promise<void>;
};

type DockerStoreState = {
  snapshots: Record<Key, DockerSnapshot>;
  get: (key: Key) => DockerSnapshot;
  isStale: (key: Key) => boolean;
  setOverview: (key: Key, overview: DockerOverview) => void;
  mergeOverview: (
    key: Key,
    patch: (prev: DockerOverview) => DockerOverview,
  ) => void;
  setVolumes: (key: Key, volumes: DockerVolumeView[]) => void;
  setError: (key: Key, error: string) => void;
  setVolumeFile: (key: Key, name: string, content: string) => void;
  /** Fetch + cache. Concurrent callers share the same in-flight promise. */
  refresh: (key: Key, fetchers: DockerFetchers, force?: boolean) => Promise<void>;
  /** Drop a cache entry (e.g. when credentials changed). */
  invalidate: (key: Key) => void;
};

export const useDockerStore = create<DockerStoreState>((set, get) => ({
  snapshots: {},

  get: (key) => get().snapshots[key] ?? emptySnapshot(),

  isStale: (key) => {
    const snap = get().snapshots[key];
    if (!snap || !snap.overview) return true;
    return Date.now() - snap.lastFetchedAt > STALE_MS;
  },

  setOverview: (key, overview) =>
    set((s) => ({
      snapshots: {
        ...s.snapshots,
        [key]: {
          ...(s.snapshots[key] ?? emptySnapshot()),
          overview,
          lastFetchedAt: Date.now(),
          error: "",
        },
      },
    })),

  mergeOverview: (key, patch) =>
    set((s) => {
      const prev = s.snapshots[key];
      if (!prev || !prev.overview) return s;
      return {
        snapshots: {
          ...s.snapshots,
          [key]: { ...prev, overview: patch(prev.overview) },
        },
      };
    }),

  setVolumes: (key, volumes) =>
    set((s) => {
      const prev = s.snapshots[key];
      if (!prev || !prev.overview) return s;
      return {
        snapshots: {
          ...s.snapshots,
          [key]: { ...prev, overview: { ...prev.overview, volumes } },
        },
      };
    }),

  setError: (key, error) =>
    set((s) => ({
      snapshots: {
        ...s.snapshots,
        [key]: { ...(s.snapshots[key] ?? emptySnapshot()), error },
      },
    })),

  setVolumeFile: (key, name, content) =>
    set((s) => {
      const prev = s.snapshots[key] ?? emptySnapshot();
      return {
        snapshots: {
          ...s.snapshots,
          [key]: {
            ...prev,
            volumeFiles: { ...prev.volumeFiles, [name]: content },
          },
        },
      };
    }),

  refresh: async (key, fetchers, force) => {
    const current = get().snapshots[key];
    if (current?.inFlight) {
      // Coalesce duplicates: StrictMode re-runs, rapid clicks, etc.
      return current.inFlight;
    }
    if (!force && current?.overview && !get().isStale(key)) {
      return; // cache still fresh
    }

    const run = (async () => {
      try {
        const overview = await fetchers.fetchOverview();
        set((s) => ({
          snapshots: {
            ...s.snapshots,
            [key]: {
              ...(s.snapshots[key] ?? emptySnapshot()),
              overview,
              lastFetchedAt: Date.now(),
              error: "",
            },
          },
        }));
        // Fire enrichment without blocking the primary await so the UI
        // thread is free for the first paint.
        if (fetchers.enrich) {
          void fetchers.enrich(overview);
        }
      } catch (e) {
        set((s) => ({
          snapshots: {
            ...s.snapshots,
            [key]: {
              ...(s.snapshots[key] ?? emptySnapshot()),
              error: String(e),
            },
          },
        }));
        throw e;
      }
    })();

    set((s) => ({
      snapshots: {
        ...s.snapshots,
        [key]: {
          ...(s.snapshots[key] ?? emptySnapshot()),
          inFlight: run,
        },
      },
    }));

    try {
      await run;
    } finally {
      set((s) => {
        const prev = s.snapshots[key];
        if (!prev) return s;
        return {
          snapshots: { ...s.snapshots, [key]: { ...prev, inFlight: null } },
        };
      });
    }
  },

  invalidate: (key) =>
    set((s) => {
      const next = { ...s.snapshots };
      delete next[key];
      return { snapshots: next };
    }),
}));
