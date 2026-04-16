# Pier-X Roadmap

This file tracks the active GPUI + Rust delivery path only.

---

## Current baseline

- Desktop shell: `pier-ui-gpui/` (`GPUI + Rust`)
- Runtime glue: direct Rust integration in `pier-ui-gpui/`
- Backend: `pier-core/`
- Repo entrypoints: `run.ps1`, `run.sh`, `build.ps1`, `build.sh`
- CI: Rust shell + Rust core

## Shipped

- [x] Root Cargo workspace for `pier-core` + `pier-ui-gpui`
- [x] Minimal GPUI shell scaffold
- [x] Direct `pier-core` integration without IPC
- [x] Repo-root entrypoints moved to the GPUI shell
- [x] Tauri shell demoted to archived reference status

## Next up

- [ ] Replace the placeholder dashboard with a docked workbench layout
- [ ] Terminal shell: event-driven session rendering, input routing, and scrollback UX
- [ ] Git depth: status, diff, commit, branch, and history views
- [ ] SSH and connection management: saved targets, auth flows, and tunnel orchestration
- [ ] Data panels: SQLite, MySQL, Redis, PostgreSQL, and local service surfaces
- [ ] Workspace polish: keyboard flow, panel density, and settings cleanup
- [ ] Plugin host boundary for future third-party extensions

## Guardrails

- `pier-core` must remain UI-framework-agnostic.
- `pier-ui-gpui` is the only active desktop shell in the repository.
- New build or packaging work must extend the GPUI path, not revive archived Qt-era or Tauri-era tooling.
