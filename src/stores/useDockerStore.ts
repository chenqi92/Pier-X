// ── Docker Panel Cache ────────────────────────────────────────────
//
// The Docker panel is mounted conditionally in RightSidebar — switching
// between tools (git / monitor / docker / …) destroys and recreates the
// component. In StrictMode this also double-invokes `useEffect`.
//
// Every remount would otherwise re-run Docker discovery. We keep the
// first-open path to `docker ps`; images, volumes, networks, stats, and
// volume usage load only when their tab needs them.
//
// Rather than fight the mounting topology, we cache the data in a
// zustand store keyed by remote identity. Remounting the panel then
// renders instantly from cache and only fetches when the data is stale.
// Per-key and per-section in-flight guards collapse StrictMode's
// double-invoke (and accidental rapid-refresh) into one request.

import { create } from "zustand";
import { effectiveSshTarget, type DockerOverview, type DockerVolumeView, type TabState } from "../lib/types";

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
  loaded: Record<DockerSection, boolean>;
  sectionInFlight: Partial<Record<DockerSection, Promise<void>>>;
  /** Cached `ls -la` output per volume name so expand state survives
   *  a panel remount. */
  volumeFiles: Record<string, string>;
};

type Key = string;
export type DockerSection = "containers" | "images" | "volumes" | "networks";

const SECTIONS: DockerSection[] = ["containers", "images", "volumes", "networks"];

function emptyLoaded(): Record<DockerSection, boolean> {
  return {
    containers: false,
    images: false,
    volumes: false,
    networks: false,
  };
}

function markLoaded(
  current: Record<DockerSection, boolean>,
  sections: DockerSection[],
): Record<DockerSection, boolean> {
  const next = { ...current };
  for (const section of sections) next[section] = true;
  return next;
}

function emptyOverview(): DockerOverview {
  return {
    containers: [],
    images: [],
    volumes: [],
    networks: [],
  };
}

function emptySnapshot(): DockerSnapshot {
  return {
    overview: null,
    lastFetchedAt: 0,
    error: "",
    inFlight: null,
    loaded: emptyLoaded(),
    sectionInFlight: {},
    volumeFiles: {},
  };
}

export function dockerKeyForTab(tab: TabState): Key {
  // Honor nested-ssh overlay so `ssh root@B` inside a hostA tab repoints
  // the cache (and the auto-refresh effect) at hostB. Two tabs targeting
  // the same effective host still share one cache entry.
  const target = effectiveSshTarget(tab);
  if (!target) return "local";
  return `ssh:${target.user}@${target.host}:${target.port}`;
}

export type DockerFetchers = {
  /** Fast path: base listings only. Must resolve quickly. */
  fetchOverview: () => Promise<DockerOverview>;
  /** Sections represented by `fetchOverview`. Defaults to containers so
   *  first-open can stay to a single `docker ps` call. */
  loaded?: DockerSection[];
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
  loadSection: (
    key: Key,
    section: DockerSection,
    fetcher: () => Promise<Partial<DockerOverview>>,
    force?: boolean,
  ) => Promise<void>;
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
          loaded: markLoaded(s.snapshots[key]?.loaded ?? emptyLoaded(), SECTIONS),
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
        const loadedSections = fetchers.loaded ?? ["containers"];
        set((s) => {
          const prev = s.snapshots[key] ?? emptySnapshot();
          const base = prev.overview ?? emptyOverview();
          // The overview endpoint returns containers without cpu/mem
          // fields (stats are a separate, slower call). Preserve any
          // stats we already had for matching ids so a manual refresh
          // doesn't make the CPU/Memory cells flicker to blank and
          // reappear 600ms later once fetchContainerStats re-runs.
          const containers = loadedSections.includes("containers")
            ? (() => {
                const prevById = new Map(base.containers.map((c) => [c.id, c]));
                return overview.containers.map((c) => {
                  const p = prevById.get(c.id);
                  if (!p) return c;
                  if (p.status !== c.status || p.state !== c.state) return c;
                  return {
                    ...c,
                    cpuPerc: c.cpuPerc || p.cpuPerc,
                    memUsage: c.memUsage || p.memUsage,
                    memPerc: c.memPerc || p.memPerc,
                  };
                });
              })()
            : base.containers;
          const nextOverview: DockerOverview = {
            containers,
            images: loadedSections.includes("images") ? overview.images : base.images,
            volumes: loadedSections.includes("volumes") ? overview.volumes : base.volumes,
            networks: loadedSections.includes("networks") ? overview.networks : base.networks,
          };
          return {
            snapshots: {
              ...s.snapshots,
              [key]: {
                ...prev,
                overview: nextOverview,
                lastFetchedAt: Date.now(),
                error: "",
                loaded: markLoaded(prev.loaded, loadedSections),
              },
            },
          };
        });
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

  loadSection: async (key, section, fetcher, force) => {
    const current = get().snapshots[key];
    if (current?.sectionInFlight[section]) {
      return current.sectionInFlight[section];
    }
    if (current?.inFlight) {
      await current.inFlight;
    }

    const ready = get().snapshots[key];
    if (!ready?.overview) return;
    if (!force && ready.loaded[section]) return;

    const run = (async () => {
      try {
        const patch = await fetcher();
        set((s) => {
          const prev = s.snapshots[key];
          if (!prev?.overview) return s;
          return {
            snapshots: {
              ...s.snapshots,
              [key]: {
                ...prev,
                overview: { ...prev.overview, ...patch },
                error: "",
                loaded: markLoaded(prev.loaded, [section]),
              },
            },
          };
        });
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

    set((s) => {
      const prev = s.snapshots[key] ?? emptySnapshot();
      return {
        snapshots: {
          ...s.snapshots,
          [key]: {
            ...prev,
            sectionInFlight: { ...prev.sectionInFlight, [section]: run },
          },
        },
      };
    });

    try {
      await run;
    } finally {
      set((s) => {
        const prev = s.snapshots[key];
        if (!prev) return s;
        const sectionInFlight = { ...prev.sectionInFlight };
        delete sectionInFlight[section];
        return {
          snapshots: { ...s.snapshots, [key]: { ...prev, sectionInFlight } },
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
