// ── Left Sidebar browser-path semantics ────────────────────────────
//
// The left file sidebar exposes a "current directory" to the rest of
// the app through `App.browserPath`. Most of the time that's a real
// filesystem path the backend can act on — but the sidebar also has a
// "This PC" view (drive picker on Windows, root-level list on Unix)
// where there IS no current directory. We represent that with a single
// sentinel string so consumers can distinguish the two states without
// having to reach into the Sidebar's internals.
//
// The sentinel is a UI-only concept. It must never reach the backend.
// Anything forwarding `browserPath` to a Rust command (git panel
// state, terminal cwd, file open, …) should filter it out first —
// that's what `isBrowsableRepoPath` is for.

/** Placeholder value written to `browserPath` when the Sidebar is on
 *  the drive-list / "This PC" view. Chosen so it can't collide with a
 *  real filesystem path: `pier:` would be a bare protocol on any OS,
 *  and Windows rejects `:` inside path segments. */
export const DRIVES_PATH = "pier:drives";

/** True when `path` names a real filesystem directory the backend can
 *  meaningfully act on. False for empty strings (pre-bootstrap) and
 *  the drives sentinel. Use this at any boundary that forwards the
 *  sidebar's path to a backend command or renders repo-specific UI. */
export function isBrowsableRepoPath(path: string | null | undefined): boolean {
  return !!path && path !== DRIVES_PATH;
}
