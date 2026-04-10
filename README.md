# Pier-X

> **Cross-platform terminal management. Built on Qt 6 + Rust core.**
> 跨平台终端管理工具，基于 Qt 6 + Rust 核心。

The cross-platform successor to [Pier](https://github.com/chenqi92/Pier) (macOS-only). Same name, same purpose, different foundation — designed to run on **macOS** and **Windows** with the same engineered IDE feel.

---

## Status

Foundation complete. The full UI shell, design system, build infrastructure, and Rust core skeleton are in place. Protocol modules (terminal, SSH, RDP, VNC) are next.

See [docs/ROADMAP.md](./docs/ROADMAP.md) for the detailed status and what's coming.

- ✅ Technology stack decided — [docs/TECH-STACK.md](./docs/TECH-STACK.md)
- ✅ Design system codified as a Claude Code skill — [`.claude/skills/pier-design-system/SKILL.md`](./.claude/skills/pier-design-system/SKILL.md)
- ✅ Qt 6 / QML UI shell with theme follow + dark/light + smooth transitions
- ✅ Full component library (buttons, inputs, combo, tooltip, card, pill, etc.)
- ✅ Tab bar + content stack + welcome state
- ✅ Command palette (Ctrl/Cmd+K) + connection dialog + settings dialog
- ✅ pier-core Rust skeleton (paths, credentials, FFI surface)
- ✅ CI on macOS + Windows (Qt) and macOS + Windows + Linux (Rust)
- ✅ Tag-triggered release workflow producing draft GH releases
- ⬜ Terminal / SSH / SFTP / RDP / VNC — incremental work, see ROADMAP

---

## Architecture

```
┌────────────────────────────────────────────────────┐
│                Qt 6 / QML (UI shell)               │  pier-ui-qt/
├────────────────────────────────────────────────────┤
│              cxx-qt bridge (Rust ↔ Qt)             │
├────────────────────────────────────────────────────┤
│            pier-core (Rust core engine)            │  pier-core/
├────────────────────────────────────────────────────┤
│  PTY · SSH · SFTP · RDP · VNC · DB · Crypto · Git  │
└────────────────────────────────────────────────────┘
```

**Design rule**: `pier-core` knows nothing about the UI. The UI layer is deliberately replaceable. See [docs/TECH-STACK.md §12](./docs/TECH-STACK.md) for the rationale.

---

## Build

### Requirements

- **Qt 6.8 LTS** (or newer)
- **CMake 3.21+**
- **C++17 compiler** (MSVC 2022 / Apple Clang 15+)
- **Rust 1.75+** (once `pier-core` is wired in)

### Install Qt 6.8 LTS

Pier-X needs Qt 6.8 LTS (or newer). Easiest path is `aqtinstall` — same tool CI uses, no GUI installer needed:

```bash
pip install aqtinstall

# Windows
aqt install-qt windows desktop 6.8.1 win64_msvc2022_64 --outputdir C:\Qt

# macOS
aqt install-qt mac desktop 6.8.1 clang_64 --outputdir ~/Qt

# Linux
aqt install-qt linux desktop 6.8.1 linux_gcc_64 --outputdir ~/Qt
```

Alternatives:
- **Windows / macOS**: official [Qt Online Installer](https://www.qt.io/download-qt-installer)
- **macOS**: `brew install qt`
- **Debian / Ubuntu**: `sudo apt install qt6-base-dev qt6-declarative-dev qt6-shadertools-dev`

### Quickstart

The repo ships with one-shot scripts that auto-detect Qt, configure, build, and launch the app.

```bash
# macOS / Linux
./run.sh

# Windows (PowerShell)
.\run.ps1
```

Build only (no launch):

```bash
./build.sh        # macOS / Linux
.\build.ps1       # Windows
```

Auto-detection looks at, in order: an explicit `QT_DIR` env var, `qmake` in `PATH`, `C:\Qt\<version>\msvc2022_64\` on Windows, `~/Qt/<version>/macos\` on macOS, `~/Qt/<version>/gcc_64\` on Linux, and Homebrew's `/opt/homebrew/opt/qt`. If Qt isn't found, the script prints exact install commands and exits.

All four scripts honour these environment variables:

| Variable | Default | Purpose |
|---|---|---|
| `BUILD_TYPE` | `Release` | CMake build type (`Release`, `Debug`, `RelWithDebInfo`) |
| `BUILD_DIR` | `build` | Build directory |
| `QT_DIR` | _(auto-detect)_ | Override the auto-detected Qt prefix (e.g. `C:\Qt\6.8.1\msvc2022_64`) |

### Manual build

```bash
cmake -B build -S .
cmake --build build --config Release
```

---

## Project layout

```
Pier-X/
├── CMakeLists.txt           # Top-level CMake
├── VERSION                  # Single source of version truth
├── pier-ui-qt/              # Qt 6 / QML UI shell
│   ├── CMakeLists.txt
│   ├── src/main.cpp
│   ├── qml/
│   │   ├── Main.qml
│   │   └── Theme.qml        # Design system singleton
│   └── resources/icons/
├── pier-core/               # Rust core engine (placeholder)
├── docs/
│   └── TECH-STACK.md        # Technology decision record
└── .claude/skills/
    └── pier-design-system/  # Design standard (Claude Code skill)
        ├── SKILL.md
        └── extracted/       # Reference DESIGN.md files
```

---

## Design system

Pier-X follows a strict design standard documented as a Claude Code skill. **Five non-negotiable principles**:

1. Darkness is the medium, not a theme
2. Single chromatic accent (`#3574F0` IntelliJ blue)
3. Borders are always semi-transparent, never solid
4. Inter for UI, JetBrains Mono for code
5. Density over spectacle

See [`.claude/skills/pier-design-system/SKILL.md`](./.claude/skills/pier-design-system/SKILL.md) for the full token reference and component recipes.

---

## License

MIT © 2026 [kkape.com](https://kkape.com)
