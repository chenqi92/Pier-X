# Pier-X Agent Rules

Pier-X now ships on a single active desktop stack:

- shell: `pier-ui-gpui/` (`GPUI + Rust`)
- runtime glue: direct Rust integration inside `pier-ui-gpui/`
- backend: `pier-core/`

## Build rules

- Do not reintroduce `Qt`, `QML`, `CMake`, `Corrosion`, or C-ABI bridge files into the active build path.
- Keep the repo-root entrypoints aligned with the active shell:
  - `run.ps1` / `run.sh`
  - `build.ps1` / `build.sh`
- Prefer direct Rust crate integration from `pier-ui-gpui` to `pier-core`.

## Frontend rules

- Keep reusable view primitives in `pier-ui-gpui/src/` instead of duplicating ad hoc panel structures.
- Keep application state and service orchestration out of render code where possible.
- Do not rebuild a TypeScript/IPC wrapper layer on top of `pier-core`; the active shell should call Rust services directly.

## Review rule

- Any UI PR should be rejected if it revives archived Qt-era or Tauri-era build paths without a clear migration reason.
