# Pier-X Agent Rules

Pier-X now ships on a single active desktop stack:

- shell: `pier-ui-tauri/` (`Tauri 2 + React + TypeScript`)
- runtime glue: `pier-ui-tauri/src-tauri/`
- backend: `pier-core/`

## Build rules

- Do not reintroduce `Qt`, `QML`, `CMake`, `Corrosion`, or C-ABI bridge files into the active build path.
- Use the npm scripts inside `pier-ui-tauri/` as the canonical entrypoints:
  - `npm run tauri dev` — development
  - `npm run tauri build` — release bundles
  - `npm run bump <version>` — sync version across manifests and tag
- Prefer direct Rust crate integration from `pier-ui-tauri/src-tauri` to `pier-core`.
- Releases are tag-driven via `.github/workflows/release.yml` (GitHub, all desktop OS) and `.gitea/workflows/release.yml` (Gitea, Linux); no wrapper shell scripts at the repo root.

## Frontend rules

- Keep shared design tokens in `pier-ui-tauri/src/styles/tokens.css`.
- Put reusable shell primitives in `pier-ui-tauri/src/components/` or `pier-ui-tauri/src/shell/` before adding panel-local one-off styling.
- Shared shell layout and chrome changes belong in `pier-ui-tauri/src/styles/shell.css` or another shared stylesheet, not scattered inline across panels.

## Review rule

- Any UI PR should be rejected if it duplicates shared shell styling or revives archived Qt-era build/assets/docs without a clear migration reason.
