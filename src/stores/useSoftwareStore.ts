// ── Software panel cache ────────────────────────────────────────
//
// Mirrors the docker store: one snapshot per remote identity (keyed
// by effective ssh target), so switching between SSH tabs doesn't
// re-probe a host whose state we already have.

import { create } from "zustand";

import { effectiveSshTarget, type TabState } from "../lib/types";
import type {
  HostPackageEnv,
  SoftwarePackageStatus,
} from "../lib/commands";

type Key = string;

export type SoftwareSnapshot = {
  env: HostPackageEnv | null;
  statuses: Record<string, SoftwarePackageStatus>;
  /** When the last successful probe landed; 0 = never probed. */
  lastFetchedAt: number;
  /** A probe in flight; concurrent callers await this rather than
   *  fan out duplicate `software_probe_remote` invocations. */
  inFlight: Promise<void> | null;
  error: string;
  /** Per-package install/update activity. The frontend uses this to
   *  disable buttons + render the live log. */
  activity: Record<string, SoftwareActivity>;
  /** Per-package version-list cache. Each entry is freshest-first
   *  (the package manager's natural ordering) and lives 5 minutes
   *  before the dropdown re-fetches. Empty array on pacman / on
   *  managers that returned no rows — the dropdown trigger hides
   *  in that case. */
  versionCache: Record<string, SoftwareVersionCache>;
};

export type SoftwareVersionCache = {
  fetchedAt: number;
  versions: string[];
};

/** TTL for the per-package version-list cache. The dropdown re-fetches
 *  on demand when the entry is older than this. */
export const VERSION_CACHE_TTL_MS = 5 * 60_000;

/** Lifecycle classes a row can be busy with. service-* variants come
 *  from the v2 service-control menu; install / update / uninstall stay
 *  the v1 + v1.1 path. */
export type SoftwareActivityKind =
  | "install"
  | "update"
  | "uninstall"
  | "service-start"
  | "service-stop"
  | "service-restart"
  | "service-reload";

export type SoftwareActivity = {
  /** Stable id we generated for this install — also the event filter. */
  installId: string;
  /** Which lifecycle this row is mid-way through. Drives the busy
   *  label ("Installing…" vs "Updating…" vs "Uninstalling…" vs the
   *  service verbs) and the per-row outcome formatting in
   *  `describeOutcome`. */
  kind: SoftwareActivityKind;
  log: string[];
  error: string;
  busy: boolean;
};

export function softwareKeyForTab(tab: TabState): Key | null {
  const target = effectiveSshTarget(tab);
  if (!target) return null;
  return `ssh:${target.user}@${target.host}:${target.port}`;
}

const STALE_MS = 60_000;

function emptySnapshot(): SoftwareSnapshot {
  return {
    env: null,
    statuses: {},
    lastFetchedAt: 0,
    inFlight: null,
    error: "",
    activity: {},
    versionCache: {},
  };
}

type SoftwareStoreState = {
  snapshots: Record<Key, SoftwareSnapshot>;
  get: (key: Key) => SoftwareSnapshot;
  isStale: (key: Key) => boolean;
  setProbeResult: (
    key: Key,
    env: HostPackageEnv,
    statuses: SoftwarePackageStatus[],
  ) => void;
  setError: (key: Key, error: string) => void;
  setInFlight: (key: Key, promise: Promise<void> | null) => void;
  appendLine: (key: Key, packageId: string, line: string) => void;
  startActivity: (
    key: Key,
    packageId: string,
    installId: string,
    kind: SoftwareActivityKind,
  ) => void;
  finishActivity: (
    key: Key,
    packageId: string,
    error: string,
    nextStatus: SoftwarePackageStatus | null,
  ) => void;
  /** Mirror a freshly-fetched version list into the per-package
   *  cache. Stamped with `Date.now()` for the TTL check below. */
  setVersionCache: (key: Key, packageId: string, versions: string[]) => void;
  invalidate: (key: Key) => void;
};

export const useSoftwareStore = create<SoftwareStoreState>((set, get) => ({
  snapshots: {},

  get: (key) => get().snapshots[key] ?? emptySnapshot(),

  isStale: (key) => {
    const snap = get().snapshots[key];
    if (!snap || snap.lastFetchedAt === 0) return true;
    return Date.now() - snap.lastFetchedAt > STALE_MS;
  },

  setProbeResult: (key, env, statuses) =>
    set((s) => {
      const prev = s.snapshots[key] ?? emptySnapshot();
      const byId: Record<string, SoftwarePackageStatus> = {};
      for (const st of statuses) byId[st.id] = st;
      return {
        snapshots: {
          ...s.snapshots,
          [key]: {
            ...prev,
            env,
            statuses: byId,
            lastFetchedAt: Date.now(),
            error: "",
          },
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

  setInFlight: (key, promise) =>
    set((s) => ({
      snapshots: {
        ...s.snapshots,
        [key]: { ...(s.snapshots[key] ?? emptySnapshot()), inFlight: promise },
      },
    })),

  startActivity: (key, packageId, installId, kind) =>
    set((s) => {
      const prev = s.snapshots[key] ?? emptySnapshot();
      return {
        snapshots: {
          ...s.snapshots,
          [key]: {
            ...prev,
            activity: {
              ...prev.activity,
              [packageId]: {
                installId,
                kind,
                log: [],
                error: "",
                busy: true,
              },
            },
          },
        },
      };
    }),

  appendLine: (key, packageId, line) =>
    set((s) => {
      const prev = s.snapshots[key];
      if (!prev) return s;
      const a = prev.activity[packageId];
      if (!a) return s;
      // Cap history so a runaway install doesn't blow up React state.
      const log = [...a.log, line];
      if (log.length > 500) log.splice(0, log.length - 500);
      return {
        snapshots: {
          ...s.snapshots,
          [key]: {
            ...prev,
            activity: { ...prev.activity, [packageId]: { ...a, log } },
          },
        },
      };
    }),

  finishActivity: (key, packageId, error, nextStatus) =>
    set((s) => {
      const prev = s.snapshots[key];
      if (!prev) return s;
      const a = prev.activity[packageId];
      const nextActivity = { ...prev.activity };
      if (a) nextActivity[packageId] = { ...a, busy: false, error };
      const nextStatuses = nextStatus
        ? { ...prev.statuses, [packageId]: nextStatus }
        : prev.statuses;
      return {
        snapshots: {
          ...s.snapshots,
          [key]: { ...prev, statuses: nextStatuses, activity: nextActivity },
        },
      };
    }),

  setVersionCache: (key, packageId, versions) =>
    set((s) => {
      const prev = s.snapshots[key] ?? emptySnapshot();
      return {
        snapshots: {
          ...s.snapshots,
          [key]: {
            ...prev,
            versionCache: {
              ...prev.versionCache,
              [packageId]: { fetchedAt: Date.now(), versions },
            },
          },
        },
      };
    }),

  invalidate: (key) =>
    set((s) => {
      const next = { ...s.snapshots };
      delete next[key];
      return { snapshots: next };
    }),
}));

/** Returns `true` when a version-cache entry for `packageId` is
 *  present and younger than {@link VERSION_CACHE_TTL_MS}. The
 *  panel uses this to decide whether to skip the network round-trip
 *  on dropdown open. */
export function isVersionCacheFresh(
  snap: SoftwareSnapshot,
  packageId: string,
): boolean {
  const entry = snap.versionCache[packageId];
  if (!entry) return false;
  return Date.now() - entry.fetchedAt < VERSION_CACHE_TTL_MS;
}

/** Returns the package id of the currently-busy row on this host, or
 *  `null` when nothing is running. The panel uses this to disable
 *  every other button — same-host concurrent installs aren't
 *  supported (apt/dpkg lock would serialise them anyway). */
export function activePackageId(snap: SoftwareSnapshot): string | null {
  for (const [id, a] of Object.entries(snap.activity)) {
    if (a.busy) return id;
  }
  return null;
}
