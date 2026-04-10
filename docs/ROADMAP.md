# Pier-X Roadmap

A snapshot of what's done, what's in progress, and what's coming next.
For the full architectural decisions, see [TECH-STACK.md](./TECH-STACK.md).
For visual / interaction standards, see [`.claude/skills/pier-design-system/SKILL.md`](../.claude/skills/pier-design-system/SKILL.md).

---

## ✅ Foundation complete

The base of the project is in place. Everything below has been implemented,
committed, CI-verified on macOS + Windows (and Linux for the Rust core), and
pushed to the `main` branch.

### Project infrastructure

- [x] Top-level + per-subproject CMake (Qt 6.8 LTS)
- [x] `VERSION` file as the single source of version truth
- [x] MIT license, README, comprehensive `.gitignore`, `.gitattributes`
- [x] CI workflow: Qt build matrix (Win/Mac) + Rust core matrix (Win/Mac/Linux)
- [x] Release workflow triggered by `v*.*.*` tags — builds installers, creates draft GH release
- [x] Conventional Commits style with `Co-Authored-By` trailer

### Design system

- [x] Codified as a Claude Code skill at `.claude/skills/pier-design-system/SKILL.md`
- [x] Five non-negotiable principles (luminance stacking, single accent, semi-transparent borders, Inter+JetBrains Mono, density)
- [x] Full token reference: colors, typography (with OpenType `cv01, ss03`), spacing (4px grid), radii, shadows, motion
- [x] 8 reference DESIGN.md files extracted from awesome-design-md (Linear, Warp, Raycast, Cursor, etc.)

### Qt 6 / QML UI shell

#### Theme

- [x] `Theme.qml` singleton with full design tokens, dark + light
- [x] `followSystem` mode that auto-tracks `QStyleHints::colorSchemeChanged`
- [x] Smooth `ColorAnimation` transitions on theme switch throughout the app

#### Components (`pier-ui-qt/qml/components/`)

- [x] `PrimaryButton` — accent CTA
- [x] `GhostButton` — transparent w/ subtle border
- [x] `IconButton` — square hover button with built-in tooltip
- [x] `PierTextField` — input with focus accent
- [x] `PierComboBox` — themed select with inline popup
- [x] `PierToolTip` — themed Qt Quick Controls ToolTip wrapper
- [x] `Card` — surface container
- [x] `StatusPill` — colored-dot status indicator
- [x] `SectionLabel` — uppercase section marker
- [x] `Separator` — themed 1px line
- [x] `TerminalTab` — tab with title + close + active accent

#### Shell pieces (`pier-ui-qt/qml/shell/`)

- [x] `TopBar` — brand + toolbar icons + theme toggle + typed signals
- [x] `TabBar` — horizontal tab strip with new-tab button
- [x] `Sidebar` — connection list + add button + local section
- [x] `StatusBar` — status text + version labels
- [x] `WelcomeView` — empty state with hero + buttons + status pills
- [x] `CommandPalette` — Cmd/Ctrl+K filterable command list with arrow nav

#### Dialogs (`pier-ui-qt/qml/dialogs/`)

- [x] `NewConnectionDialog` — modal SSH connection form
- [x] `SettingsDialog` — modal with section nav (General/Appearance/Terminal/Connections)

#### Views (`pier-ui-qt/qml/views/`)

- [x] `TerminalView` — placeholder for the eventual PTY-rendering surface

### pier-core (Rust)

- [x] Crate skeleton with `staticlib` + `rlib` outputs
- [x] `lib.rs` with public surface and design rule comment ("must not depend on UI types")
- [x] `ffi` module — stable C ABI (`pier_core_version`, `pier_core_has_feature`)
- [x] `paths` module — cross-platform app data dirs via `directories`
- [x] `credentials` module — OS keyring wrapper (Keychain / DPAPI / Secret Service)
- [x] `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test` in CI

### Keyboard shortcuts

- [x] `Ctrl/Cmd+K` — command palette
- [x] `Ctrl/Cmd+T` — new local terminal tab
- [x] `Ctrl/Cmd+W` — close current tab
- [x] `Ctrl/Cmd+N` — new SSH connection
- [x] `Ctrl/Cmd+,` — settings

---

## 🚧 Next up — protocol modules

The UI shell is essentially done. The next big chunk is bringing real
functionality through `pier-core`. Each item below is roughly one focused
session of work:

- [ ] **PTY backend** — `forkpty` (Unix) + `ConPTY` (Windows) via a common trait
- [ ] **VTE terminal renderer** — port from sibling Pier, expose to QML via cxx-qt
- [ ] **cxx-qt bridge scaffolding** — first real Rust ↔ QML integration
- [ ] **SSH client** — `russh` 0.57 + connection lifecycle
- [ ] **SFTP client** — `russh-sftp` + remote file browser model
- [ ] **Connection persistence** — serialize `connectionsModel` to disk via `pier-core`
- [ ] **RDP client** — `ironrdp` integration; PoC the SharedPixelBuffer rendering path
- [ ] **VNC client** — `vnc-rs` or `libvncclient` wrapper
- [ ] **Database clients** — MySQL / PostgreSQL / Redis through SSH tunnels
- [ ] **Git panel** — `git2` for the local-context Git tools
- [ ] **Markdown preview** — local file rendering
- [ ] **Search** — `ignore` crate (ripgrep core)

---

## 🎨 Polish backlog

Smaller things that improve fidelity but aren't gating:

- [ ] Real SVG icon set (Lucide or Phosphor) replacing the Unicode glyph placeholders
- [ ] Bundled JetBrains Mono + Inter font files (currently relies on system install)
- [ ] Frameless title bar with custom traffic lights (macOS) and Mica (Windows 11)
- [ ] Drag-to-reorder tabs
- [ ] Full keyboard shortcut customization in Settings
- [ ] Localization (i18n) — currently English-only via `qsTr()`
- [ ] AvaloniaEdit-equivalent code editor for SQL / log panels (likely QScintilla)
- [ ] KDDockWidgets integration for true detachable tool windows
- [ ] Screen reader / accessibility audit

---

## 🚀 Release plan

| Version | Goal |
|---|---|
| **0.1.0** (current) | Foundation: shell, design system, build/CI, Rust skeleton |
| 0.2.0 | First real terminal — local PTY + VTE rendering, no SSH yet |
| 0.3.0 | SSH + SFTP working end-to-end |
| 0.4.0 | RDP / VNC PoC |
| 0.5.0 | Database clients + remote service discovery |
| 1.0.0 | All MVP protocols ship-ready, signed binaries, auto-update via Sparkle/WinSparkle |

---

## How to contribute

1. Read [TECH-STACK.md](./TECH-STACK.md) — especially §12 (architecture rules)
2. Read the design skill at `.claude/skills/pier-design-system/SKILL.md`
3. Pick an item from the "Next up" section above
4. Open an issue describing your approach before writing code
5. PRs should keep the CI matrix green and follow the existing commit style

The cardinal rule: **`pier-core` must not depend on any UI type**. If your
change makes Rust code reach into Qt (or vice versa), that's a sign to
rethink the layering.
