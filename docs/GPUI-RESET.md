# GPUI Reset

> Date: 2026-04-16

## Goal

Pier-X is moving off the Tauri shell. The backend remains in `pier-core/`; the new desktop runtime is a native Rust shell built with `GPUI`.

This reset is a shell replacement, not a backend rewrite:

- keep `pier-core` as the durable backend
- remove the TypeScript / Tauri IPC layer from the active path
- build the desktop shell directly in Rust
- reintroduce terminal, Git, SSH, and data tools as native GPUI views

## What Landed

- root Cargo workspace created for `pier-core/` + `pier-ui-gpui/`
- minimal `pier-ui-gpui/` shell scaffolded
- first GPUI window renders direct `pier-core` data:
  - core version
  - current workspace path
  - Git repository state
  - persisted SSH connection store summary
  - local machine metrics
- root `run.*` / `build.*` scripts now target the GPUI shell
- old `pier-ui-tauri/` remains in the repository as an archived baseline, not the active shell

## Current Architecture

```text
GPUI shell (Rust)
        |
 Pier application state (Rust)
        |
      pier-core
        |
terminal / ssh / git / mysql / sqlite / redis / more
```

## Next Slices

1. Replace the placeholder dashboard with a real workbench and dock layout.
2. Move terminal sessions to direct event-driven rendering instead of snapshot polling.
3. Rebuild Git, SSH, and connection management views as GPUI panels.
4. Rebuild database and service panels as native Rust views.
5. Delete the archived Tauri shell only after GPUI reaches daily-driver parity.
