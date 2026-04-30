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
  /** Set the moment the user clicks Cancel and reset when the activity
   *  finishes. Disables the cancel button so a double-click doesn't
   *  fire two `software_install_cancel` invocations (harmless on the
   *  backend, but the second one's `cancelled` event would re-flash
   *  the row state). */
  cancelling: boolean;
  /** Stale third-party repos detected in the install/update output —
   *  populated by `finishActivity` from the backend's
   *  `repoWarnings` field. The row's banner surfaces these as a
   *  yellow advisory regardless of install success/failure: install
   *  succeeds → "we ignored a broken repo, you should clean it up";
   *  install fails → "and here are the repos that may have caused it".
   *  Empty array when nothing was flagged. */
  repoWarnings: string[];
};

export function softwareKeyForTab(tab: TabState): Key | null {
  const target = effectiveSshTarget(tab);
  if (!target) return null;
  return `ssh:${target.user}@${target.host}:${target.port}`;
}

const STALE_MS = 60_000;

// Frozen singleton — returned by `get(key)` when no entry exists yet so the
// selector identity stays stable across reads. A fresh object would trip
// `useSyncExternalStore`'s "getSnapshot should be cached to avoid an
// infinite loop" detector and freeze the UI under high-frequency store
// activity (e.g. a SSH terminal flooded by `docker logs -f`).
const EMPTY_SOFTWARE_SNAPSHOT: SoftwareSnapshot = Object.freeze({
  env: null,
  statuses: {} as SoftwareSnapshot["statuses"],
  lastFetchedAt: 0,
  inFlight: null as SoftwareSnapshot["inFlight"],
  error: "",
  activity: {} as SoftwareSnapshot["activity"],
  versionCache: {} as SoftwareSnapshot["versionCache"],
}) as SoftwareSnapshot;

function emptySnapshot(): SoftwareSnapshot {
  return EMPTY_SOFTWARE_SNAPSHOT;
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
    repoWarnings?: string[],
  ) => void;
  /** Mirror a freshly-fetched version list into the per-package
   *  cache. Stamped with `Date.now()` for the TTL check below. */
  setVersionCache: (key: Key, packageId: string, versions: string[]) => void;
  /** Mark a row as "cancel in flight" so the UI can disable the cancel
   *  button between the click and the resulting `cancelled` event. */
  setCancelling: (key: Key, packageId: string, cancelling: boolean) => void;
  /** Drop the activity entry for `packageId` so the row stops showing
   *  the install log / error / repo-warning advisory blocks. Called
   *  from the row's dismiss button after a finished install — never
   *  while busy (the UI hides the dismiss button mid-flight to keep
   *  the user from accidentally tearing down a live progress feed). */
  dismissActivity: (key: Key, packageId: string) => void;
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
                cancelling: false,
                repoWarnings: [],
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

  finishActivity: (key, packageId, error, nextStatus, repoWarnings) =>
    set((s) => {
      const prev = s.snapshots[key];
      if (!prev) return s;
      const a = prev.activity[packageId];
      const nextActivity = { ...prev.activity };
      if (a) {
        nextActivity[packageId] = {
          ...a,
          busy: false,
          cancelling: false,
          error,
          // Carry the warnings forward when caller supplied them; if
          // omitted (e.g. cancel/error path with no report), preserve
          // whatever was already there so a partial-then-failed run
          // still shows the broken-repo notice the partial output
          // surfaced.
          repoWarnings: repoWarnings ?? a.repoWarnings,
        };
      }
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

  setCancelling: (key, packageId, cancelling) =>
    set((s) => {
      const prev = s.snapshots[key];
      if (!prev) return s;
      const a = prev.activity[packageId];
      if (!a) return s;
      return {
        snapshots: {
          ...s.snapshots,
          [key]: {
            ...prev,
            activity: {
              ...prev.activity,
              [packageId]: { ...a, cancelling },
            },
          },
        },
      };
    }),

  dismissActivity: (key, packageId) =>
    set((s) => {
      const prev = s.snapshots[key];
      if (!prev) return s;
      if (!(packageId in prev.activity)) return s;
      const a = prev.activity[packageId];
      // Refuse to dismiss a still-running activity — the row's
      // dismiss button is hidden in that state, but a stale double-
      // click race could still slip through. Keeping the entry
      // around prevents the live progress feed from going dark.
      if (a && a.busy) return s;
      const nextActivity = { ...prev.activity };
      delete nextActivity[packageId];
      return {
        snapshots: {
          ...s.snapshots,
          [key]: { ...prev, activity: nextActivity },
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
