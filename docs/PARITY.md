# Pier → Pier-X Feature Parity Plan

> How Pier-X catches up with the macOS-only [Pier](https://github.com/chenqi92/Pier), and then surpasses it by shipping the same experience on Windows.

**Status date:** 2026-04-11
**Reference upstream:** `../Pier` (sibling working copy)

---

## 1 · Why this document exists

Pier-X was started as a cross-platform rewrite of Pier, with a fresh Qt 6 / QML shell replacing the SwiftUI one. The UI shell, design system, build system, and Rust skeleton are all in place — but the **product** is not yet Pier. The feature gap is large and concrete:

| Dimension | Upstream Pier | Pier-X today |
|---|---|---|
| Platforms | macOS only | macOS + Windows (scaffold) |
| UI lines of code | ~27,000 Swift | ~4,400 QML + ~60 C++ |
| Rust core lines of code | ~3,400 | ~200 |
| Rust modules | `terminal` (vte+forkpty), `ssh` (russh+sftp+service_detector), `crypto`, `git_graph`, `search`, `ffi` | `paths`, `credentials`, `ffi` (stub) |
| Right-panel tools | MySQL, Redis, PostgreSQL, Docker, Log viewer, Git, Markdown, AI, File browser, Server Monitor, SQLite | none wired |
| Protocol clients | PTY, SSH, SFTP | none |
| i18n | en + zh-Hans (80 keys each) | `qsTr()` wiring only, no catalogs |
| Cross-platform blockers | – | cxx-qt bridge not wired; pier-core not linked into pier-ui-qt |

The goal of this plan: **reach Pier functional parity on both macOS and Windows, in six phased milestones, without breaking the layering rule (`pier-core` must know nothing about the UI).**

---

## 2 · What upstream Pier actually does

These are the upstream modules Pier-X must replicate, grouped by layer:

### 2.1 Rust core (`Pier/pier-core/src`)

| Module | LOC | What it does | Key deps |
|---|---:|---|---|
| `terminal/pty.rs` | 208 | Unix PTY via `forkpty` + non-blocking IO | libc |
| `terminal/emulator.rs` | 306 | VT100/ANSI parser + grid state | `vte 0.15` |
| `ssh/session.rs` | 393 | SSH connection lifecycle, exec, port forward | `russh 0.57` |
| `ssh/sftp.rs` | 150 | SFTP list/upload/download/mkdir/delete | `russh-sftp 2.1` |
| `ssh/service_detector.rs` | 222 | **Core feature**: probes remote for MySQL/Redis/Postgres/Docker via `which` + `systemctl` + version parsing | tokio join |
| `search/mod.rs` | 110 | ripgrep-style file search | `ignore` |
| `crypto/mod.rs` | 75 | AES-256-GCM | `ring` |
| `git_graph.rs` | 1008 | Branch/commit graph, blame, diff, stash, tag, remote, submodule, rebase | `git2` |
| `ffi.rs` | 799 | C ABI wrapping all of the above for Swift | `libc`, `cbindgen` |

**Observation:** `git_graph.rs` + `ffi.rs` alone are ~1,800 LOC. Git is by far the biggest single subsystem.

### 2.2 Swift app (`Pier/PierApp/Sources`)

Roughly 70 files, grouped:

- **Services** (9 files): `CommandRunner`, `KeychainService`, `LLMService` (OpenAI/Claude/Ollama), `RemoteServiceManager` (**the glue** — drives service detection, tunnel lifecycle, right-panel visibility), `SSHBackend` / `SystemSSHBackend` / `SSHControlMaster`, `UpdateChecker` (Sparkle).
- **ViewModels** (17 files): one per panel — Terminal, AI, Database, Docker, File, Git (+ 7 feature-split extensions), Log, PostgreSQL, Redis, RemoteFile, SQLite, ServerMonitor.
- **Views** (40+ files) across:
  - `MainWindow/MainView.swift` (512 LOC) — three-pane HSplitView with drag-to-resize and state persistence
  - `Terminal/` — TerminalView (2606 LOC!), MetalTerminalRenderer (326 LOC), TerminalKeyboardHandler, TerminalSplitView
  - `LeftPanel/` — LocalFileView (local file tree), LeftPanelView (658 LOC)
  - `RightPanel/` — DatabaseClientView, RedisClientView, PostgreSQLView, SQLiteView, DockerManageView, LogViewerView, GitPanelView (+ BranchGraph, Blame, Diff, MergeConflict, Rebase, Remote, Submodule, Tag, Config, CommitDetail, BranchManager), ServerMonitorView, AIChatView, ERDiagramView, MarkdownWebView
  - `Connection/` — ConnectionManagerView
  - `SettingsView`, `SSHKeyManagerView`, `AboutView`

**Observation:** The single biggest Swift file is `TerminalView.swift` at **2606 lines** — it wires keyboard, rendering, drag-and-drop, scrollback, URL detection, theming. This is the critical path and the most effort-intensive UI port.

### 2.3 Product pitch (from upstream `FEATURES.md`)

> Through SSH, detect what services (MySQL/Redis/Docker/Postgres/etc.) are running on the remote server, and auto-expose UI panels for them. Solves the "service only listens on 127.0.0.1, I can't point DBeaver at it" problem.

This is the **core value prop** — it's not just a terminal, it's a server-admin IDE. Any port of Pier that doesn't ship remote service discovery + tunnel management is missing the thing that makes Pier *Pier*.

---

## 3 · Cross-platform strategy

Upstream Pier is macOS-only because it uses:

- `NSViewRepresentable` / AppKit for the terminal hosting view
- `Metal` for text rendering (`MetalTerminalRenderer.swift`)
- `forkpty` for PTYs
- `/usr/bin/ssh` + `ControlMaster` (`SystemSSHBackend`) for the terminal's SSH transport
- macOS Keychain for credentials
- Sparkle for auto-update
- SF Symbols for icons

The Pier-X replacement per capability:

| Concern | macOS-only upstream | Pier-X cross-platform choice |
|---|---|---|
| Window host | SwiftUI + AppKit | Qt 6 QML (already in place) |
| Text rendering | Metal | **Qt Quick Scene Graph** (Metal on macOS, D3D11 on Windows, both via Qt RHI) — no custom renderer needed for v1 |
| PTY | `forkpty` | `forkpty` on Unix, **`ConPTY`** on Windows — same `Pty` trait |
| SSH transport | system `ssh` + ControlMaster | `russh` **only** — already cross-platform, no fork/exec |
| Service detection | Swift → FFI → Rust | Rust (same module, already written upstream) |
| Credentials | Keychain | `keyring` crate (Keychain + DPAPI + Secret Service) — **already in Pier-X** |
| Auto-update | Sparkle | Sparkle on macOS + **WinSparkle** on Windows |
| Icons | SF Symbols | **Lucide** SVGs bundled in resources |
| Fonts | System Inter / JetBrains Mono | Bundled font files in resources |
| Git | `git2` via FFI | `git2` via cxx-qt — same crate works everywhere |

**Key simplification for v1:** skip the custom Metal/D3D text renderer. Qt Quick's scene graph can render a terminal grid at 60fps for the typical 120×40 cell window. `TerminalView.qml` becomes a C++ `QQuickPaintedItem` (or better, a scene-graph `QSGNode`) driven by the Rust VTE emulator. Upstream's 326-line `MetalTerminalRenderer` becomes unnecessary.

---

## 4 · Milestone plan

Each milestone is sized to land as a coherent PR with working tests and a demoable result. Ordering is driven by dependencies — nothing can be built before the cxx-qt bridge exists, and nothing remote can be built before SSH works.

### M1 — Rust ↔ Qt bridge (unblocks everything)

**Goal:** Stop pretending pier-core is detached. Wire it into pier-ui-qt so QML can call Rust functions.

- [ ] Add `cxx-qt-build 0.7+` as a build dependency in `pier-core` (keep `staticlib` + `rlib`)
- [ ] Uncomment `add_subdirectory(pier-core)` in root `CMakeLists.txt`, add `corrosion-cmake` or use `cxx-qt`'s own CMake integration
- [ ] First bridged type: `PierCore` QObject exposing `version()` and `hasFeature(name)`
- [ ] Wire into `pier-ui-qt` as a registered QML singleton under the `Pier` module
- [ ] Replace `StatusBar.qml`'s hardcoded version with `PierCore.version` (smoke test)
- [ ] Windows CI: verify the combined Rust + MSVC build works (`msbuild` + `cargo` must coexist cleanly, usually means `corrosion`)

**Deliverable:** One end-to-end Rust → Qt call in production code. No features yet, but the plumbing is done.

### M2 — Local terminal (the first real feature)

**Goal:** Open a local shell tab, type commands, see output — on both macOS and Windows. No SSH yet.

- [ ] `pier-core::terminal::pty` — `Pty` trait with two impls:
  - `UnixPty` using `nix::pty::forkpty` (port from upstream's `forkpty` code)
  - `WindowsPty` using the Win32 **ConPTY** API (`CreatePseudoConsole`)
- [ ] `pier-core::terminal::emulator` — port upstream's `vte`-backed emulator verbatim (it's already cross-platform)
- [ ] cxx-qt `TerminalSession` QObject: `start(shell, cwd)`, `write(data)`, `resize(cols, rows)`, signals `dataReceived(QString)`, `exited(int)`
- [ ] Terminal grid rendering in QML:
  - `TerminalGrid.qml` — scene-graph-backed item showing the cell buffer
  - Monospace font metrics, cursor, basic attributes (fg/bg, bold, underline)
  - `TerminalView.qml` wraps `TerminalGrid` + scrollback viewport
- [ ] Keyboard routing: QML `Keys` handlers → `TerminalSession.write()`
- [ ] New-terminal command: `Ctrl/Cmd+T` already exists in the shortcut table, wire it to actually open a tab
- [ ] Scrollback buffer (bounded ring, default 10k lines)

**Deliverable:** `./run.sh` → click "New Terminal" → real interactive shell inside Pier-X. Matches Pier's v0.1 experience but also works on Windows.

**Test checklist:** `vim`, `htop`, `ssh` from inside the shell (using system ssh for now), `fg`/`bg`/`jobs`, window resize, paste from clipboard.

### M3 — SSH + SFTP end-to-end

**Goal:** Click "New SSH Connection" in the existing dialog → actual remote shell, SFTP sidebar shows remote files.

- [ ] `pier-core::ssh::session` — port from upstream, `russh 0.57`, methods: `connect(config)`, `open_shell()`, `exec(cmd)`, `forward_local(local_port, remote_host, remote_port)`
- [ ] `pier-core::ssh::sftp` — port from upstream, `russh-sftp 2.1`
- [ ] Connection model persistence — serialize `NewConnectionDialog` state to `~/.config/pier-x/connections.json` via `pier-core::paths`
- [ ] Credential storage via existing `pier-core::credentials` (keyring crate already present)
- [ ] `SshSession` cxx-qt QObject: mirrors `TerminalSession` but reads from a remote channel
- [ ] Known-hosts verification UI — first-connect fingerprint prompt
- [ ] Connection manager dialog (port upstream's `ConnectionManagerView.swift`) — add/edit/delete/group saved servers
- [ ] SSH key manager dialog (port upstream's `SSHKeyManagerView.swift`) — generate/import/view public keys

**Cross-platform note:** upstream uses `/usr/bin/ssh` + ControlMaster. We deliberately go `russh`-only so Windows gets the same stack for free. ControlMaster-style multiplexing is handled inside `russh` by keeping one `SshSession` and opening multiple channels.

**Deliverable:** Saved server list → connect → interactive remote shell + file tree.

### M4 — Remote service discovery & tunnel management (THE core feature)

**Goal:** Replicate Pier's signature move — after SSH connects, detect what's installed and light up the right-panel tabs.

- [ ] Port `pier-core::ssh::service_detector` verbatim from upstream (already 222 LOC of working, tested Rust)
- [ ] Tunnel manager: maintain active `russh` local-forward channels, expose their local port back to the UI
- [ ] cxx-qt `ServiceDiscovery` QObject: `runDetection(session)` → emits `servicesDetected(list)`
- [ ] `RightPanel.qml` (new) — tab strip that is populated *dynamically* from detected services
- [ ] Status indicator per service (`running` / `stopped` / `installed`) — reuse existing `StatusPill.qml`
- [ ] Tunnel lifecycle UI — "MySQL tunnel on local :53306" chip with close button

**Deliverable:** SSH into a box with MySQL + Docker → right panel automatically shows MySQL + Docker tabs. This is the "pier moment."

### M5 — Right-panel tools, phased

Each sub-bullet is one focused session. Order is by dependency and value density:

- [ ] **Markdown preview** (local) — port upstream's `MarkdownRenderView`. Smallest, no deps, kicks the tires on the generic right-panel content contract.
- [ ] **Log viewer** (remote) — `tail -f` via SSH exec + level coloring. Simple and immediately useful. Port upstream's `LogViewerView.swift`.
- [ ] **Docker panel** (remote) — container/image/volume list + start/stop/rm + live logs. Port `DockerManageView.swift`. All via `docker` CLI over SSH exec.
- [ ] **MySQL client** (remote via tunnel) — table browser + SQL editor + result grid. Use `sqlx` or `mysql_async` from Rust, results marshaled via cxx-qt. Port the view layout from upstream.
- [ ] **Redis client** (remote via tunnel) — key browser + value inspector + TTL + basic commands. Use `redis` crate.
- [ ] **PostgreSQL client** (remote via tunnel) — same shape as MySQL, `tokio-postgres`.
- [ ] **SQLite** (local) — file picker → same grid view as MySQL/Postgres. Use `rusqlite`.
- [ ] **Local file panel** — port `LocalFileView.swift`. Already partly there in `Sidebar.qml`, needs lazy loading + context menu + drag-to-terminal.
- [ ] **Git panel (local)** — BIGGEST ITEM. Port `git_graph.rs` (1008 LOC) + 7-file GitViewModel + 12+ views. Realistically this is its own 3–4 milestone mini-roadmap. Ship in this order: status → diff → branch list → log graph → blame → rebase/merge conflict → stash/tag/remote/submodule.
- [ ] **Server monitor** — port `ServerMonitorView` (remote CPU/mem/disk via SSH exec of `top`/`free`/`df`).
- [ ] **AI chat panel** — port `LLMService` + `AIChatView`. Streaming response, terminal-context-aware. Providers: OpenAI, Claude, Ollama. Secrets via existing `keyring`.

**Deliverable for M5 as a whole:** all right-panel tools present, even if some are MVP-quality.

### M6 — Release engineering & polish

Nothing in here is blocking functionality, but you cannot ship without it.

- [ ] **Icons** — swap Unicode glyph placeholders in `IconButton.qml` for bundled Lucide SVGs
- [ ] **Bundled fonts** — ship Inter + JetBrains Mono in `resources/fonts/`, register via `QFontDatabase::addApplicationFont`, set as Theme defaults (no more "Populating font family aliases" warning)
- [ ] **Frameless title bar** — macOS traffic lights + Windows 11 Mica backdrop
- [ ] **i18n** — port upstream's en + zh-Hans catalogs to Qt `.ts` files, generate `.qm`, register in `main.cpp`; wire language switcher in Settings
- [ ] **Drag-to-reorder tabs**
- [ ] **Shortcut customization UI** (upstream has this in Settings)
- [ ] **Code signing & notarization** on macOS (Developer ID Application + notarytool)
- [ ] **Code signing on Windows** (EV cert or SignTool + timestamp)
- [ ] **Auto-update** — Sparkle framework on macOS, WinSparkle on Windows, driven by `docs/appcast.xml`
- [ ] **Crash reporting** — `breakpad` or `sentry-native`
- [ ] **Installer** — `.dmg` on macOS (already in CI for draft releases), MSI via WiX or `cargo-wix` on Windows
- [ ] **Performance profiling** — scrollback @ 100k lines, remote file tree @ 10k files, terminal @ 1k chars/sec sustained

---

## 5 · Non-negotiable architecture rules

These come from `docs/TECH-STACK.md §12` and are re-stated here because every PR in this plan must respect them:

1. **`pier-core` must not depend on any Qt type.** Not transitively either. If you find yourself writing `#include <QString>` in a Rust-adjacent header, stop. The cxx-qt bridge layer is the *only* place Rust ↔ Qt types meet.
2. **Platform code goes behind traits.** `Pty`, `Credentials`, `ServicesDir` — one trait, N impls. No `#[cfg(target_os = "macos")]` sprinkled through business logic.
3. **No new features without tests.** Upstream ships `cargo test` for 8 pier-core unit tests. Every new Rust module here adds at least one happy-path test.
4. **Every new QML component matches the design system.** `.claude/skills/pier-design-system/SKILL.md` is the source of truth. `Theme.` tokens only; no raw hex colors.
5. **CI matrix stays green.** macOS + Windows for the Qt build, macOS + Windows + Linux for the Rust core. A broken CI cell is a P0.

---

## 6 · Open decisions

These are called out so future sessions don't ambush them:

| Question | Options | Recommendation |
|---|---|---|
| cxx-qt vs raw cbindgen + QObject hand-wiring | cxx-qt (magic, fast) / cbindgen (ugly, flexible) | **cxx-qt** — it's 2026, cxx-qt 0.7+ is production-ready and Pier-X is greenfield |
| Windows SSH transport | `russh` only / `russh` + optional OpenSSH via WSL | **`russh` only** — zero deps, identical behavior |
| Terminal renderer | Custom QSGNode / `QQuickPaintedItem` / `QtTermWidget` vendored | **QSGNode** in a C++ subclass, driven by Rust emulator. Fastest + still portable |
| Database client UI — do we vendor a code editor? | `QScintilla` / plain `TextArea` with regex highlighter / `KSyntaxHighlighting` | Start with `TextArea` + a tiny highlighter; revisit only if users complain |
| LLM streaming SSE transport in Rust | `eventsource-client` / hand-rolled `reqwest` / `async-openai` | **`async-openai`** for OpenAI, hand-rolled for Claude + Ollama |
| Git module — port `git_graph.rs` or rewrite? | Verbatim port / clean-room rewrite | **Verbatim port** first, refactor later — it's 1008 lines of working code |

---

## 7 · What is already done on the Pier-X side

Don't rebuild these — they exist and work:

- Full Qt 6 / QML shell (Main, TabBar, Sidebar, StatusBar, WelcomeView, CommandPalette, SettingsDialog, NewConnectionDialog)
- Theme singleton with dark/light + follow-system + smooth color transitions
- Component library (buttons, text field, combo, tooltip, card, pill, section label, separator, terminal tab)
- Command palette with Ctrl/Cmd+K and filter+arrow nav
- Keyboard shortcuts wired in `main.cpp` (Ctrl/Cmd+K/T/W/N/,)
- CMake build (`CMakeLists.txt` + `pier-ui-qt/CMakeLists.txt`) producing a bundled `.app` on macOS
- Rust skeleton with `paths`, `credentials`, `ffi` placeholder
- CI matrix (macOS + Windows Qt, macOS + Windows + Linux Rust) + tag-triggered release workflow
- Design system skill at `.claude/skills/pier-design-system/SKILL.md`
- `run.sh` / `build.sh` (+ Windows `.ps1` equivalents) with Qt auto-detection

---

## 8 · How to use this document

- `docs/ROADMAP.md` is the high-level, tag-by-tag release plan. This document is the ground-level porting plan.
- When picking a task: find a milestone section (M1–M6), pick an unchecked bullet, open an issue referencing `docs/PARITY.md §4 Mx`, write the code, tick the box in the PR.
- When a bullet is bigger than expected, split it in the PR description — don't let the parity doc silently lie.
- When upstream Pier lands a new feature, add it here *before* implementing it in Pier-X, so the parity table stays truthful.
