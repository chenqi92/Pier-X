# Pier-X

> **Cross-platform terminal management on Tauri + Rust core.**
> 跨平台终端管理工具，当前桌面壳基于 Tauri，后端核心基于 Rust。

The cross-platform successor to [Pier](https://github.com/chenqi92/Pier) (macOS-only). Same name, same purpose, different foundation — designed to run on **macOS** and **Windows** with the same engineered IDE feel.

---

## Status

The Rust backend lives in `pier-core/`; the desktop shell now lives in `pier-ui-tauri/`. The old Qt shell has been retired from the active build path.

See [docs/ROADMAP.md](./docs/ROADMAP.md) for the active delivery plan, and [docs/TAURI-RESET.md](./docs/TAURI-RESET.md) for the shell reset baseline.

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
- ✅ Tauri shell is the only supported desktop shell in repo entrypoints
- ✅ Qt/CMake/Corrosion legacy build chain removed from the active repo
- ⬜ Deepen data panels and add plugin host into the new shell
- ✅ CI on macOS + Windows (Tauri shell) and macOS + Windows + Linux (Rust core)
- ✅ Tag-triggered release workflow publishing Tauri bundles to GitHub Releases
- ⬜ Terminal / SSH / SFTP / RDP / VNC — incremental work, see ROADMAP

See [docs/TAURI-RESET.md](./docs/TAURI-RESET.md) for the migration baseline. The repo now keeps only the active Tauri build path in tracked build and packaging files.

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

- **Node.js 24+**
- **npm 11+**
- **Rust 1.88+**
- **WebView2 runtime** (Windows)

Run the active shell directly:

```bash
cd pier-ui-tauri
npm ci
npm run tauri -- dev
```

Build the active shell directly:

```bash
cd pier-ui-tauri
npm ci
npm run tauri -- build --debug
```

### Release

Tagged releases are driven by the npm `bump` script, which syncs the version across every manifest and creates a matching git tag:

```bash
cd pier-ui-tauri
npm run bump 0.2.0        # explicit
npm run bump patch        # or minor / major
git push && git push --tags
```

Pushing a `v*.*.*` tag triggers two release workflows:

- **GitHub** (`.github/workflows/release.yml`) — builds and publishes Linux, Windows x64, Windows ARM64, and macOS universal Tauri bundles to GitHub Releases.
- **Gitea** (`.gitea/workflows/release.yml`) — builds Linux `.deb` / `.rpm` / `.AppImage` on an `ubuntu-22.04` Gitea runner and uploads them to the Gitea release via API.

---

## Project layout

```
Pier-X/
├── pier-core/               # Rust core engine
├── pier-ui-tauri/           # Active desktop shell rewrite
│   ├── src/                 # React UI
│   └── src-tauri/           # Tauri runtime + Rust commands
├── docs/
│   ├── ROADMAP.md
│   └── TAURI-RESET.md
└── .agents/skills/          # Archived design references and repo automation skills
```

---

## License

MIT © 2026 [kkape.com](https://kkape.com)
