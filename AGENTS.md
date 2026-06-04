# Pier-X Agent Rules

> Short form. The full code rules and architecture boundaries live in [CLAUDE.md](CLAUDE.md).
> Visual tokens live in [.agents/skills/pier-design-system/SKILL.md](.agents/skills/pier-design-system/SKILL.md).
> Product behavior lives in [docs/PRODUCT-SPEC.md](docs/PRODUCT-SPEC.md).

## Stack

A UI-agnostic Rust backend driving two frontends:

- backend: `pier-core/`
- Tauri frontend: repo root (`Tauri 2 + React 19 + TypeScript`, sources under
  `src/`) + runtime glue `src-tauri/`
- GPUI frontend: `pier-ui-gpui/` — native Rust UI (gpui + gpui-component),
  isolated workspace, calls `pier-core` directly; the migration target meant
  to replace the Tauri shell over time

## Build

Use the npm scripts at the repo root as the canonical entrypoints:

- `npm run tauri dev` — development
- `npm run tauri build` — release bundles
- `npm run bump <version>` — sync version across manifests and tag

`pier-core` is consumed by both `src-tauri` and `pier-ui-gpui` as a direct Rust
dependency. The GPUI frontend builds from its own dir (`cd pier-ui-gpui &&
cargo run`). There is no C-ABI bridge and no `Qt` / `QML` / `CMake` /
`Corrosion`. Releases are tag-driven via `.github/workflows/release.yml`
(GitHub, all desktop OS) and `.gitea/workflows/release.yml` (Gitea, Linux).

## Frontend

- All visual values come from `src/styles/tokens.css` — never inline a color,
  font, spacing, radius, or shadow literal in `src/shell/` / `src/panels/` /
  `src/components/` (see CLAUDE.md Rule 1).
- Reusable shell primitives go in `src/components/` or `src/shell/` before
  panel-local one-off styling.
- Shared shell layout / chrome belongs in `src/styles/shell.css` (or a new
  scoped sheet), not scattered inline.

## Review gate

A change should be rejected if it:

1. Inlines a visual literal instead of using a `tokens.css` var.
2. Duplicates shared shell styling inside a panel.
3. Revives `Qt` / `QML` / `CMake` / `Corrosion` build paths or a C-ABI bridge.
4. Adds a UI-crate dependency to `pier-core`.
5. Calls `pier-core` from React without going through a Tauri command (the
   GPUI frontend may call `pier-core` directly via `pier-ui-gpui/src/data.rs`).
6. Adds / removes / re-purposes a right-side tool, or changes a panel's
   default safety stance, without first updating
   [docs/PRODUCT-SPEC.md](docs/PRODUCT-SPEC.md).
