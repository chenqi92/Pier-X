# Archived Tauri Shell

`pier-ui-tauri/` is now an archived reference shell.

## Status

- It is kept in the repository only as migration history and implementation reference.
- It is no longer the active desktop target for `run.ps1`, `run.sh`, `build.ps1`, or `build.sh`.
- New shell work should land in `pier-ui-gpui/`.

## Why It Stays For Now

- The archived shell still contains feature coverage that has not yet been migrated to GPUI.
- It provides concrete reference behavior for terminal, Git, SSH, and database panels during the rewrite.

## Removal Rule

Delete `pier-ui-tauri/` only after the GPUI shell reaches functional parity for the daily workflow.
