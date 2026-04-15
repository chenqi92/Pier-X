# Pier-X Agent Rules

Pier-X now ships on a single active desktop stack:

- shell: `pier-ui-tauri/` (`Tauri 2 + React + TypeScript`)
- runtime glue: `pier-ui-tauri/src-tauri/`
- backend: `pier-core/`

## Build rules

- Do not reintroduce `Qt`, `QML`, `CMake`, `Corrosion`, or C-ABI bridge files into the active build path.
- Keep the repo-root entrypoints aligned with the active shell:
  - `run.ps1` / `run.sh`
  - `build.ps1` / `build.sh`
- Prefer direct Rust crate integration from `pier-ui-tauri/src-tauri` to `pier-core`.

## Frontend rules

- Keep shared design tokens in `pier-ui-tauri/src/styles/tokens.css`.
- Put reusable shell primitives in `pier-ui-tauri/src/components/` or `pier-ui-tauri/src/shell/` before adding panel-local one-off styling.
- Shared shell layout and chrome changes belong in `pier-ui-tauri/src/styles/shell.css` or another shared stylesheet, not scattered inline across panels.

## Review rule

- Any UI PR should be rejected if it duplicates shared shell styling or revives archived Qt-era build/assets/docs without a clear migration reason.
