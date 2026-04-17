# Pier-X — Code Rules for Claude

Pier-X is a cross-platform terminal / Git / SSH / database management tool, aiming for an IntelliJ-grade IDE experience on macOS and Windows. The stack is **pure Rust + GPUI**: no Tauri, no Qt, no npm, no web runtime.

## Authoritative sources

| Concern | File |
|---|---|
| Overall delivery plan | [docs/ROADMAP.md](docs/ROADMAP.md) |
| Architecture reset decision | [docs/GPUI-RESET.md](docs/GPUI-RESET.md) |
| **Visual design tokens & rules** | [.agents/skills/pier-design-system/SKILL.md](.agents/skills/pier-design-system/SKILL.md) — only source of truth for colors, typography, spacing, radius, shadow |
| What was deleted and how to recover it | [docs/legacy-index.md](docs/legacy-index.md) |
| Legacy pixel reference (QML) | [docs/legacy-qml-reference/](docs/legacy-qml-reference/) |

When SKILL.md and this file overlap, SKILL.md wins for visual values; this file wins for Rust code structure.

## Architecture boundaries

- **Cargo workspace**: two members only — [`pier-core`](pier-core/) (UI-framework-agnostic backend) and [`pier-ui-gpui`](pier-ui-gpui/) (the only desktop shell).
- `pier-core` **must stay UI-agnostic**. No `gpui` dependency. No UI state. Public API returns plain Rust types.
- `pier-ui-gpui` **calls `pier-core` directly** as Rust functions. Any proposal to add an IPC layer, a scripting runtime, or a second UI framework is a regression — reject it.
- **Do not reintroduce**: `tauri`, `@tauri-apps/*`, any `npm` / `pnpm` / `vite` / TypeScript toolchain, `qt6-*`, `cmake`, `qmake`, `.qrc`/`.pro`/`CMakeLists.txt`, any cross-language bridge.

## Rust code rules (`pier-ui-gpui`)

### Rule 1 — Theme tokens, never literals

Every color, font family, font size, spacing, radius, and shadow used in a view or a component **must** come from the `crate::theme` module.

**Forbidden in `src/views/` and `src/app/`:**

- `gpui::rgb(0x...)`, `gpui::rgba(...)`, any `Rgba`/`Hsla` literal
- `gpui::px(<numeric literal>)` — even `px(4.)` is banned; use `spacing::SP_1`
- Hardcoded font family strings like `"Inter"`, `"JetBrains Mono"`
- Hardcoded font weights (use `typography::WEIGHT_MEDIUM` / `WEIGHT_EMPHASIS`; the number 700/bold is banned per SKILL.md §3.3)

**Allowed everywhere:**

- `theme(cx).color.*` for colors
- `spacing::SP_0..SP_12` for sizes and gaps
- `radius::RADIUS_XS..RADIUS_PILL` for corners
- `typography::SIZE_*` + `typography::WEIGHT_*` for text
- `shadow::*` builders for elevation

Components in `src/components/` **may** use literals, but only when translating a SKILL.md-defined constant (e.g. `StatusPill` hard-codes its 18px height because that is the token for pill height). Document the SKILL.md section in a one-line comment when you do this.

### Rule 2 — Custom components must be encapsulated

**Any new UI atom is a `struct` in `src/components/` implementing `RenderOnce` (via `#[derive(IntoElement)]`)**. Views compose components; views do not create new atoms.

**Forbidden in views:**

```rust
// ❌ — creating a new visual atom inline
div().bg(t.color.bg_surface).border_1().border_color(t.color.border_subtle).rounded(RADIUS_MD).p(SP_4).child(...)
```

**Required in views:**

```rust
// ✅ — compose an existing component
Card::new().padding(SP_4).child(...)
```

If the existing component set (`Button`, `Card`, `StatusPill`, `SectionLabel`, `IconBadge`, `NavItem`, `Separator`, `text::{display,h1,h2,h3,body,caption,mono}`) cannot express what you need, **add a new component in `src/components/` first** — with a proper name, variant enum, and builder methods — then use it from the view. Do not "just this once" inline a new atom.

### Rule 3 — Variants as enums, not new types

A button with a different look is `Button { variant: ButtonVariant::Ghost, ... }`, not a separate `GhostButton` struct. One struct per component family; visual variants are enum values.

### Rule 4 — Builder style, stable IDs

Components use chainable builders (`Button::primary(id, label).width(px(148.)).on_click(cb)`). Every interactive component takes an `ElementId` as its first argument; the ID must be a descriptive string literal (`"welcome-new-ssh"`), not a counter.

### Rule 5 — Module layout

```
pier-ui-gpui/src/
├── main.rs              # minimal: Application::new(), theme::init, font load, window open
├── app/                 # PierApp state, Route enum, root Render impl
├── theme/               # colors / typography / spacing / radius / shadow (the only place literals live)
├── components/          # ONE file per component family (button.rs, card.rs, …)
├── views/               # one file per full-screen view (welcome.rs, workbench.rs)
└── data/                # pure data loaders (ShellSnapshot, etc.) — no UI code
```

When adding a module, follow this split. Do not put view logic in `main.rs` or component logic in views.

### Rule 6 — Render is paint-only

`Render::render` and `RenderOnce::render` bodies **must not perform IO**.
This includes:

- Filesystem reads (`std::fs::*`, `read_dir`, `metadata`, anything that
  hits a syscall)
- Network calls (HTTP, SSH connect, DB connect)
- OS keychain reads (`pier_core::credentials::get`)
- Process spawns
- Any `*_blocking()` API from pier-core

GPUI re-renders entities on every state change. A 5 ms `read_dir` repeated
30 times during a render call costs 150 ms — felt as obvious lag. A
1 second `connect_blocking` freezes the UI for 1 second.

**Pattern**: cache the data in `PierApp` (or an entity it owns), populate it
from a click / event / startup handler, and have `render` read the cache
only. Examples in tree:

- `PierApp::file_tree_root_entries` + `file_tree_children` cache, populated
  by `toggle_dir` / `cd_up` (file_tree.rs renders from cache)
- `SshSessionState::entries` cache, populated by `set_right_mode(Sftp)`
  (sftp_browser.rs renders from cache)

If the data genuinely needs fresh-on-display, push the IO behind a
background task (`cx.background_executor().spawn(...)` + `weak.update`)
and render a placeholder while it's in flight.

## Review gate

Reject a change if any of these are true:

1. It adds a color/size/font literal in `views/` or `app/`.
2. It inlines a new visual atom instead of adding a component in `src/components/`.
3. It reintroduces Tauri / Qt / npm / cmake in any form.
4. It adds a `pier-core` dependency on `gpui` or any UI crate.
5. It violates one of the five SKILL.md non-negotiables (see SKILL.md §1).
6. It calls a `_blocking` / `read_dir` / `credentials::get` / network /
   process-spawn API from inside a `render` body (Rule 6).

## Build & run

```sh
./run.sh                            # debug run, terminal-icon dock entry
./build.sh                          # release build
./scripts/bundle-macos.sh           # build → wrap in Pier-X.app for proper dock icon
./scripts/run-bundled-macos.sh      # bundle + open the .app
cargo build -p pier-ui-gpui
cargo build -p pier-core
```

No other toolchains are required. If a step asks you to install Node, Qt, or CMake, it is wrong.
