# Pier-X

> **Cross-platform terminal management. Rebuilding the desktop shell on Tauri + Rust core.**
> 跨平台终端管理工具，当前分支正在把桌面壳重建为 Tauri + Rust core。

The cross-platform successor to [Pier](https://github.com/chenqi92/Pier) (macOS-only). Same name, same purpose, different foundation — designed to run on **macOS** and **Windows** with the same engineered IDE feel.

---

## Status

The repository is in an active UI reset. The Rust backend stays in `pier-core/`; the desktop shell is being rebuilt in `pier-ui-tauri/`, while the older Qt shell is treated as legacy migration context.

See [docs/ROADMAP.md](./docs/ROADMAP.md) for the tag-by-tag release plan, and [docs/PARITY.md](./docs/PARITY.md) for the ground-level porting plan that tracks every feature in the macOS-only upstream Pier and maps it to work in Pier-X.

- ✅ Rust backend foundation in `pier-core/`
- ✅ New Tauri desktop shell scaffold in `pier-ui-tauri/`
- ✅ IDE-style three-pane workbench + integrated terminal surface
- ✅ Real shell session wired through `pier-core::terminal::PierTerminal`
- ✅ Git overview panel wired through `pier-core::services::git::GitClient`
- ✅ Git diff preview + stage / unstage actions wired in the new shell
- ✅ Commit, local branch switch, and recent history wired in the new shell
- ✅ Push / pull and stash flows wired in the new shell
- ✅ Tracked change discard and stash drop wired in the new shell
- ✅ SSH password-based terminal target wired in the new shell
- ✅ SSH agent and key-file terminal auth wired in the new shell
- ✅ Persisted SSH connections wired through `pier-core::connections::ConnectionStore`
- ✅ MySQL / SQLite / Redis browse surfaces wired through `pier-core` service clients
- ✅ MySQL / SQLite query editors and result tables wired in the new shell
- ✅ Redis command editor and raw reply panel wired in the new shell
- ✅ MySQL / SQLite write-safe execution and TSV result copy wired in the new shell
- ✅ Terminal copy-selection + clipboard paste wired in the new shell
- ✅ Tauri commands wired to `pier-core` runtime, directory listing, terminal, and Git
- ✅ Windows debug bundle built successfully from the new shell
- ⬜ Deepen data panels and add plugin host into the new shell
- ⬜ Remove the legacy Qt shell once feature parity is sufficient
- ✅ CI on macOS + Windows (Qt) and macOS + Windows + Linux (Rust)
- ✅ Tag-triggered release workflow producing draft GH releases
- ⬜ Terminal / SSH / SFTP / RDP / VNC — incremental work, see ROADMAP

See [docs/TAURI-RESET.md](./docs/TAURI-RESET.md) for the new migration baseline.

---

## Architecture

```
┌────────────────────────────────────────────────────┐
│           Tauri 2 + React (desktop shell)          │  pier-ui-tauri/
├────────────────────────────────────────────────────┤
│        Tauri commands / desktop runtime glue       │
├────────────────────────────────────────────────────┤
│            pier-core (Rust core engine)            │  pier-core/
├────────────────────────────────────────────────────┤
│  PTY · SSH · SFTP · RDP · VNC · DB · Crypto · Git  │
└────────────────────────────────────────────────────┘
```

**Design rule**: `pier-core` knows nothing about the UI. The shell is deliberately replaceable.

---

## Build

### Requirements

#### Active shell: `pier-ui-tauri`

- **Node.js 24+**
- **npm 11+**
- **Rust 1.88+**
- **WebView2 runtime** (Windows)

Run the new shell:

```bash
cd pier-ui-tauri
npm install
npm run tauri dev
```

Build the new shell:

```bash
cd pier-ui-tauri
npm run tauri build -- --debug
```

#### Legacy shell: `pier-ui-qt`

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
├── VERSION                  # Single source of version truth
├── pier-core/               # Rust core engine
├── pier-ui-tauri/           # Active desktop shell rewrite
│   ├── src/                 # React UI
│   └── src-tauri/           # Tauri runtime + Rust commands
├── pier-ui-qt/              # Legacy Qt shell retained during migration
├── docs/
│   ├── TAURI-RESET.md
│   └── TECH-STACK.md
└── .agents/skills/
    └── pier-design-system/  # Design standard
        ├── SKILL.md
        └── extracted/       # Reference DESIGN.md files
```

---

## Design system

Pier-X follows a strict design standard documented as an agent skill. **Five non-negotiable principles**:

1. Darkness is the medium, not a theme
2. Single chromatic accent (`#3574F0` IntelliJ blue)
3. Borders are always semi-transparent, never solid
4. Inter for UI, JetBrains Mono for code
5. Density over spectacle

See [`.agents/skills/pier-design-system/SKILL.md`](./.agents/skills/pier-design-system/SKILL.md) for the full token reference and component recipes.

---

## License

MIT © 2026 [kkape.com](https://kkape.com)
