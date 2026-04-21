# Pier-X Roadmap

This file tracks the active Tauri + Rust delivery path only.

---

## Current baseline

- Desktop shell: repo root (`Tauri 2 + React + TypeScript`, sources under `src/`)
- Runtime glue: `src-tauri/`
- Backend: `pier-core/`
- Cargo workspace: root `Cargo.toml` with members `pier-core` and `src-tauri`
- Repo entrypoints: root-level npm scripts (`npm run tauri dev`, `npm run tauri build`, `npm run bump`)
- CI: Tauri shell on macOS + Windows, Rust core on macOS + Windows + Linux

## Shipped

- [x] Tauri shell scaffold and shared workbench layout
- [x] Direct `pier-core` integration from `src-tauri`
- [x] Local terminal session creation and snapshot polling
- [x] SSH terminal sessions with saved connections and keyring-backed secrets
- [x] Git overview, diff, stage / unstage, commit, branch switch, push / pull, stash
- [x] MySQL / SQLite / Redis browse and query flows
- [x] Markdown rendering and local directory listing
- [x] Windows and macOS Tauri CI builds
- [x] Tag-triggered Tauri release workflow
- [x] Qt / CMake / Corrosion / C-ABI legacy build path removed from tracked repo files

## Next up

- [ ] Terminal polish: richer selection, scrollback UX, and stability hardening
- [ ] Git depth: graph/history views, richer revert flows, and remote management
- [ ] Data panels: more complete table tooling, safer write flows, and saved connections
- [ ] Service surfaces: PostgreSQL, Docker, SFTP, and server monitoring refinements
- [ ] Workspace polish: keyboard flow, panel density, and settings cleanup
- [ ] Plugin host boundary for future third-party extensions

## Guardrails

- `pier-core` must remain UI-framework-agnostic.
- The root-level Tauri shell is the only active desktop shell in the repository.
- New build or packaging work must extend the Tauri path, not revive archived Qt-era tooling.
