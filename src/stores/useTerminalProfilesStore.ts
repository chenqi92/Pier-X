import { create } from "zustand";

/**
 * A user-defined local terminal preset. Profiles are local-only:
 * SSH profiles already live in `useConnectionStore`, and this store
 * stays focused on the `cd` / startup-command layer that makes a
 * brand-new local shell immediately useful.
 *
 * `startupCommand` is raw shell — the backend runs it on the PTY
 * verbatim. That means `$VAR` expansion, quoting, and chaining
 * (`&&`, `;`) all work, but the caller is responsible for quoting
 * paths with spaces.
 */
export type TerminalProfile = {
  id: string;
  name: string;
  /** Optional working directory. When set, the tab is seeded with
   *  `cd "<cwd>"` before any custom command runs. */
  cwd?: string;
  /** Optional extra shell command to run after the cd. Chained
   *  with `&&` so a cd failure short-circuits the whole sequence. */
  startupCommand?: string;
  /** Tab color index (-1 = none, 0..7 = palette). */
  tabColor?: number;
};

type ProfilesStore = {
  profiles: TerminalProfile[];
  add: (profile: Omit<TerminalProfile, "id">) => string;
  update: (id: string, patch: Partial<Omit<TerminalProfile, "id">>) => void;
  remove: (id: string) => void;
  reorder: (fromIndex: number, toIndex: number) => void;
};

const STORAGE_KEY = "pierx:terminal-profiles-v1";

function load(): TerminalProfile[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as TerminalProfile[]) : [];
  } catch {
    return [];
  }
}

function save(profiles: TerminalProfile[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(profiles));
  } catch {
    /* quota / serialization failures are non-fatal */
  }
}

let nextSeq = 1;
function genId(): string {
  // Timestamp + monotonic counter keeps ids unique even when two
  // adds happen in the same millisecond.
  return `prof-${Date.now()}-${nextSeq++}`;
}

export const useTerminalProfilesStore = create<ProfilesStore>((set) => ({
  profiles: load(),

  add: (profile) => {
    const id = genId();
    set((s) => {
      const next = [...s.profiles, { id, ...profile }];
      save(next);
      return { profiles: next };
    });
    return id;
  },

  update: (id, patch) => {
    set((s) => {
      let changed = false;
      const next = s.profiles.map((p) => {
        if (p.id !== id) return p;
        changed = true;
        return { ...p, ...patch };
      });
      if (!changed) return s;
      save(next);
      return { profiles: next };
    });
  },

  remove: (id) => {
    set((s) => {
      const next = s.profiles.filter((p) => p.id !== id);
      if (next.length === s.profiles.length) return s;
      save(next);
      return { profiles: next };
    });
  },

  reorder: (fromIndex, toIndex) => {
    set((s) => {
      if (
        fromIndex < 0 ||
        toIndex < 0 ||
        fromIndex >= s.profiles.length ||
        toIndex >= s.profiles.length ||
        fromIndex === toIndex
      ) {
        return s;
      }
      const next = [...s.profiles];
      const [moved] = next.splice(fromIndex, 1);
      next.splice(toIndex, 0, moved);
      save(next);
      return { profiles: next };
    });
  },
}));

/**
 * Compile a profile into the `startupCommand` string that
 * `useTabStore.addTab` passes to the PTY. Matches how App.tsx
 * composes "Open terminal here" commands: quote the path and
 * chain with `&&` so cd failure stops execution.
 */
export function compileProfileStartup(profile: TerminalProfile): string {
  const parts: string[] = [];
  if (profile.cwd && profile.cwd.trim()) {
    parts.push(`cd ${JSON.stringify(profile.cwd.trim())}`);
  }
  if (profile.startupCommand && profile.startupCommand.trim()) {
    parts.push(profile.startupCommand.trim());
  }
  return parts.join(" && ");
}
