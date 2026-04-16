# Tauri Reset

> Historical note: this document is kept only as migration history. The active shell baseline now lives in `docs/GPUI-RESET.md`.

> Date: 2026-04-14  
> Branch: `codex/tauri-ui-reset`

## Goal

Pier-X is moving off the Qt desktop shell. The backend remains in `pier-core/`; the new desktop runtime is `Tauri 2 + React + TypeScript`.

This reset is a shell replacement, not a visual touch-up:

- keep `pier-core` as the durable backend
- replace the desktop UI runtime with Tauri
- rebuild the workbench around an IDE-style three-pane layout
- wire terminal, Git, database, and plugin surfaces back in slice by slice

## What Landed

- `pier-ui-tauri/` scaffolded with `react-ts`
- `pier-core` added as a direct Rust dependency in `pier-ui-tauri/src-tauri`
- Tauri commands:
  - `core_info`
  - `list_directory`
  - `git_overview`
  - `git_diff`
  - `git_stage_paths`
  - `git_unstage_paths`
  - `git_stage_all`
  - `git_unstage_all`
  - `git_discard_paths`
  - `git_commit`
  - `git_branch_list`
  - `git_checkout_branch`
  - `git_recent_commits`
  - `git_push`
  - `git_pull`
  - `git_stash_list`
  - `git_stash_push`
  - `git_stash_apply`
  - `git_stash_pop`
  - `git_stash_drop`
  - `mysql_browse`
  - `mysql_execute`
  - `sqlite_browse`
  - `sqlite_execute`
  - `redis_browse`
  - `redis_execute`
  - `ssh_connections_list`
  - `ssh_connection_save`
  - `ssh_connection_delete`
  - `terminal_create`
  - `terminal_create_ssh`
  - `terminal_create_ssh_saved`
  - `terminal_write`
  - `terminal_resize`
  - `terminal_snapshot`
  - `terminal_close`
- first IDE shell implemented:
  - top command bar
  - left explorer
  - center migration/workbench surface
  - right inspector
  - bottom terminal panel backed by `pier-core::terminal::PierTerminal`
- current repo status now renders in the right inspector via `GitClient`
- change selection, diff preview, and stage / unstage actions now work from the new shell
- branch switching, staged commit submission, and recent commit history now work from the new shell
- push / pull, stash creation, and stash restore now work from the new shell
- tracked worktree discard and stash drop now work from the new shell
- SSH password-based terminal sessions now connect through `pier-core::ssh::SshSession`
- SSH terminal target now supports password, agent, and key-file auth entry modes
- saved SSH connections now persist through `pier-core::connections::ConnectionStore`
- saved password-based SSH connections now resolve secrets through the OS keyring at connect time
- MySQL browse surface now connects through `pier-core::services::mysql::MysqlClient`
- MySQL query editor now executes arbitrary SQL through `pier-core::services::mysql::MysqlClient`
- SQLite browse surface now opens local `.db` files through `pier-core::services::sqlite::SqliteClient`
- SQLite query editor now executes arbitrary SQL through `pier-core::services::sqlite::SqliteClient`
- Redis browse surface now scans keys and previews values through `pier-core::services::redis::RedisClient`
- Redis command editor now executes arbitrary commands through `pier-core::services::redis::RedisClient`
- MySQL / SQLite query flows now default to read-only and require explicit write unlock + confirmation
- MySQL / SQLite query result grids now support TSV copy for spreadsheet handoff
- terminal keyboard routing now supports copy-selection and clipboard paste
- Windows desktop bundles built successfully:
  - debug exe
  - MSI
  - NSIS installer

## Current Architecture

```text
Tauri shell (React + TypeScript)
        |
Tauri command layer (Rust)
        |
     pier-core
        |
terminal / ssh / git / mysql / sqlite / redis / more
```

## Commands

```bash
cd pier-ui-tauri
npm install
npm run tauri -- dev
```

```bash
cd pier-ui-tauri
npm run tauri -- build --debug
```

## Next Slices

1. Add richer terminal capabilities: block selection polish and explicit scrollbars.
2. Expose more Git actions and views: remote management, richer revert flows, and history graph.
3. Deepen the data panels with richer result grids, safer Redis workflows, and saved data connections.
4. Design a plugin host boundary for future third-party extensions.
5. Prune the remaining Qt-era docs and comments as the Tauri shell fully absorbs the daily workflow.

## Migration Rule

Until parity is reached:

- `pier-core` is the source of truth for backend capability
- `pier-ui-tauri` is the only active desktop shell
- Qt-era plans and comments are historical context only, not the target direction
