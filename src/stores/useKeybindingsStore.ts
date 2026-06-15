import { create } from "zustand";
import type { Chord } from "../lib/keybindings";

// User overrides for the rebindable global shortcuts (see
// `src/lib/keybindings.ts`). Persisted to a dedicated localStorage
// namespace — same hand-rolled idiom as `useThemeStore` /
// `useSettingsStore`, separate key so it never collides with them.
const PREFS_KEY = "pierx:keybindings";

/** id (from `KEYBINDINGS`) → the chord the user assigned. Only ids that
 *  differ from the factory default are stored, so a future change to a
 *  default still reaches users who never touched that binding. */
type Overrides = Record<string, Chord>;

type KeybindingsState = {
  overrides: Overrides;
  /** Assign a chord to a binding. */
  setBinding: (id: string, chord: Chord) => void;
  /** Drop a binding's override, reverting it to the factory default. */
  resetBinding: (id: string) => void;
  /** Drop every override. */
  resetAll: () => void;
};

function isChord(v: unknown): v is Chord {
  return (
    typeof v === "object" &&
    v !== null &&
    typeof (v as Chord).key === "string" &&
    typeof (v as Chord).mod === "boolean" &&
    typeof (v as Chord).shift === "boolean" &&
    typeof (v as Chord).alt === "boolean"
  );
}

function loadOverrides(): Overrides {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    // Keep only well-formed entries so one corrupt value can't dead-bind
    // a command (a truthy-but-malformed override never matches and would
    // otherwise mask the factory default).
    const clean: Overrides = {};
    for (const [id, value] of Object.entries(parsed as Record<string, unknown>)) {
      if (isChord(value)) clean[id] = value;
    }
    return clean;
  } catch {
    return {};
  }
}

function saveOverrides(next: Overrides) {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(next));
  } catch {
    /* swallow quota errors */
  }
}

export const useKeybindingsStore = create<KeybindingsState>((set, get) => ({
  overrides: loadOverrides(),
  setBinding: (id, chord) => {
    const next = { ...get().overrides, [id]: chord };
    saveOverrides(next);
    set({ overrides: next });
  },
  resetBinding: (id) => {
    const next = { ...get().overrides };
    delete next[id];
    saveOverrides(next);
    set({ overrides: next });
  },
  resetAll: () => {
    saveOverrides({});
    set({ overrides: {} });
  },
}));
