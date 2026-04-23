import { create } from "zustand";

// Per-host bookmark entry. `label` is optional; when absent the UI
// falls back to the basename of the path.
export type SftpBookmark = {
  path: string;
  label?: string;
};

type BookmarkMap = Record<string, SftpBookmark[]>;

type BookmarksStore = {
  bookmarks: BookmarkMap;
  list: (host: string) => SftpBookmark[];
  add: (host: string, bookmark: SftpBookmark) => void;
  remove: (host: string, path: string) => void;
  rename: (host: string, path: string, label: string) => void;
};

const STORAGE_KEY = "pierx:sftp-bookmarks-v1";

function load(): BookmarkMap {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as BookmarkMap;
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function save(state: BookmarkMap) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    /* quota / serialization failures are non-fatal */
  }
}

/**
 * Build the per-host key used to index bookmarks. Same shape as the
 * terminal title / tunnel cache: `user@host:port`. Callers that
 * don't know the port should pass 22 to match the most common SSH
 * default rather than an empty string, so the key stays stable.
 */
export function hostKey(user: string, host: string, port: number): string {
  return `${user}@${host}:${port}`;
}

export const useSftpBookmarksStore = create<BookmarksStore>((set, get) => ({
  bookmarks: load(),

  list: (host) => get().bookmarks[host] ?? [],

  add: (host, bookmark) => {
    set((s) => {
      const existing = s.bookmarks[host] ?? [];
      if (existing.some((b) => b.path === bookmark.path)) {
        return s;
      }
      const next: BookmarkMap = {
        ...s.bookmarks,
        [host]: [...existing, bookmark],
      };
      save(next);
      return { bookmarks: next };
    });
  },

  remove: (host, path) => {
    set((s) => {
      const existing = s.bookmarks[host] ?? [];
      const filtered = existing.filter((b) => b.path !== path);
      if (filtered.length === existing.length) {
        return s;
      }
      const next: BookmarkMap = { ...s.bookmarks };
      if (filtered.length === 0) {
        delete next[host];
      } else {
        next[host] = filtered;
      }
      save(next);
      return { bookmarks: next };
    });
  },

  rename: (host, path, label) => {
    set((s) => {
      const existing = s.bookmarks[host] ?? [];
      let changed = false;
      const updated = existing.map((b) => {
        if (b.path === path && b.label !== label) {
          changed = true;
          return { ...b, label };
        }
        return b;
      });
      if (!changed) return s;
      const next: BookmarkMap = {
        ...s.bookmarks,
        [host]: updated,
      };
      save(next);
      return { bookmarks: next };
    });
  },
}));
