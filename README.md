# Pier-X

> **Cross-platform terminal management. Built on Qt 6 + Rust core.**
> 跨平台终端管理工具，基于 Qt 6 + Rust 核心。

The cross-platform successor to [Pier](https://github.com/chenqi92/Pier) (macOS-only). Same name, same purpose, different foundation — designed to run on **macOS** and **Windows** with the same engineered IDE feel.

---

## Status

Early development. The architecture and design system are in place; UI implementation is in progress.

- ✅ Technology stack decided: see [docs/TECH-STACK.md](./docs/TECH-STACK.md)
- ✅ Design system codified: see [`.claude/skills/pier-design-system/SKILL.md`](./.claude/skills/pier-design-system/SKILL.md)
- ✅ Qt 6 / QML skeleton with Theme singleton + dark/light switching
- ⬜ pier-core (Rust) port from sibling project
- ⬜ Terminal / SSH / SFTP / RDP / VNC features

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

### Build steps

```bash
cmake -B build -S .
cmake --build build --config Release
```

### Run

```bash
# macOS
./build/pier-ui-qt/pier-x.app/Contents/MacOS/pier-x

# Windows
./build/pier-ui-qt/Release/pier-x.exe
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
