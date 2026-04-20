# Pier-X — Code Rules for Claude

Pier-X is a cross-platform terminal / Git / SSH / database management tool,
aiming for an IntelliJ-grade IDE experience on macOS and Windows. The stack is
**Rust backend + Tauri 2 + React + TypeScript**: the previous Qt/QML shell is
archived, and the Rust/GPUI experiment on `backup/rust-gpui` is abandoned.

## Authoritative sources

| Concern | File |
|---|---|
| Overall delivery plan | [docs/ROADMAP.md](docs/ROADMAP.md) |
| Shell-reset decision & wired Tauri commands | [docs/TAURI-RESET.md](docs/TAURI-RESET.md) |
| Build / frontend / review rules (short form) | [AGENTS.md](AGENTS.md) |
| **Visual design tokens & rules** | [.agents/skills/pier-design-system/SKILL.md](.agents/skills/pier-design-system/SKILL.md) — only source of truth for colors, typography, spacing, radius, shadow |

When SKILL.md and this file overlap, SKILL.md wins for visual values; this file
wins for code structure.

## Architecture boundaries

- **Cargo workspace**: two members only — [`pier-core`](pier-core/)
  (UI-framework-agnostic backend) and
  [`pier-ui-tauri/src-tauri`](pier-ui-tauri/src-tauri/) (the Tauri runtime
  glue).
- **Frontend**: [`pier-ui-tauri/`](pier-ui-tauri/) — Vite + React 19 +
  TypeScript. State via `zustand`; terminals via `@xterm/xterm`; panels via
  `react-resizable-panels`; icons from `lucide-react`.
- `pier-core` **must stay UI-agnostic**. No `tauri`, `gpui`, `qt`, or any UI
  crate dependency. Public API returns plain Rust types.
- `pier-ui-tauri/src-tauri` **calls `pier-core` directly** as Rust functions
  and exposes them to the frontend as Tauri commands. React code calls those
  commands via `@tauri-apps/api`'s `invoke`. The frontend **must not** bypass
  Tauri to reach pier-core.
- **Do not reintroduce**: `qt6-*`, `qml`, `cmake`, `qmake`, `corrosion`, any
  C-ABI bridge, or the `pier-ui-gpui` crate. The Qt and GPUI shells are gone
  on purpose — propose a new feature, not a third UI runtime.

## Frontend code rules (`pier-ui-tauri/src/`)

### Rule 1 — Design tokens, never literals

Every color, font family, font size, spacing, radius, and shadow used in a
component or panel **must** reference a CSS custom property defined in
[`src/styles/tokens.css`](pier-ui-tauri/src/styles/tokens.css).

**Forbidden in `src/shell/`, `src/panels/`, and `src/components/`:**

- Hex / rgb / hsl color literals in `.css` or inline styles
  (`color: "#0f1115"`, `background: rgb(...)`, etc.)
- Hardcoded pixel values for spacing, radius, or typography when a token
  exists
- Hardcoded font family strings like `"Inter"`, `"JetBrains Mono"`

**Allowed:**

- `var(--bg-surface)`, `var(--text-primary)`, `var(--accent)`, etc. for color
- `var(--sp-0..sp-12)` for spacing, `var(--radius-xs..radius-pill)` for
  corners, `var(--size-display..size-small)` for type scale
- `var(--font-ui)` / `var(--font-mono)` for font families

If a token is missing, **add it to `tokens.css` first** (dark + light), then
consume it. Do not "just this once" inline a raw value.

### Rule 2 — Module layout

```
pier-ui-tauri/src/
├── main.tsx              # entrypoint; mounts <App/>
├── App.tsx               # top-level routing / layout shell
├── shell/                # chrome: TopBar, Sidebar, StatusBar, TabBar, WelcomeView, dialogs
├── panels/               # one file per tool panel (Git, Terminal, Sftp, MySql, …)
├── components/           # reusable UI atoms (ContextMenu, PreviewTable, ResizeHandle, …)
├── stores/               # zustand stores — UI state, never business logic
├── lib/                  # Tauri-command wrappers, pure helpers
├── i18n/                 # locale resources
└── styles/               # tokens.css (single source of truth) + shell.css + scoped css
```

When adding code, follow this split:

- A new tool surface → a file in `src/panels/`.
- A new piece of shell chrome → a file in `src/shell/`.
- A reusable atom used by ≥2 panels → a file in `src/components/`.
- Shared layout / chrome styling → `src/styles/shell.css` (or a new scoped
  sheet), not inline across panels.

### Rule 3 — State in stores, not in panels

Cross-panel state (connections, active tab, selected host, pending diffs)
lives in a `zustand` store under `src/stores/`. Panels subscribe to slices
they need. Keep stores focused on UI state; don't put business logic there —
that belongs in `pier-core`.

### Rule 4 — Tauri IPC is the only bridge

- React components call backend behavior by invoking a Tauri command declared
  in [`pier-ui-tauri/src-tauri/src/lib.rs`](pier-ui-tauri/src-tauri/src/lib.rs)
  (or a sibling module like `git_panel.rs`).
- Wrap `invoke` calls in typed helpers under `src/lib/` so panels stay free of
  raw `invoke("...")` strings.
- New backend capability: add it to `pier-core` first, expose a thin
  command in `src-tauri`, then a typed wrapper in `src/lib/`, then consume it
  from the panel. Do not grow `src-tauri` into a business-logic layer.

### Rule 5 — Render is paint-only

React render paths (component bodies, `useMemo` deps, JSX children) **must
not** call `invoke` synchronously or block on IO. Load data in `useEffect` /
event handlers, store it in a zustand store or local state, and render from
the cache. Tauri commands that can be slow (SSH connect, DB connect,
directory walks) must stream/return via awaited calls off the render path.

## Review gate

Reject a change if any of these are true:

1. It adds a color/size/font literal in `shell/`, `panels/`, or `components/`
   instead of a `tokens.css` var.
2. It inlines a new visual atom in a panel instead of adding a component in
   `src/components/`.
3. It reintroduces Qt / QML / CMake / Corrosion / `pier-ui-gpui` in any form.
4. It adds a `pier-core` dependency on `tauri`, `gpui`, or any UI crate.
5. It calls pier-core from React without going through a Tauri command.
6. It violates one of the SKILL.md non-negotiables (see SKILL.md §1).
7. It invokes a backend command synchronously inside a render body (Rule 5).

## Build & run

```sh
./run.sh                            # dev: vite + tauri dev
./build.sh                          # release: vite build + tauri build
cargo build -p pier-core            # backend only
cd pier-ui-tauri && npm install     # first-time frontend deps
```

Windows equivalents: `run.ps1` / `build.ps1`. Node + npm and the Rust toolchain
are required; no Qt, CMake, or GPUI toolchain is needed. If a step asks you to
install Qt or to run `cargo build -p pier-ui-gpui`, it is out of date.
