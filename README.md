# Pier-X

> **Cross-platform terminal management on GPUI + Rust core.**
> 跨平台终端管理工具，当前桌面壳基于 GPUI，后端核心基于 Rust。

The cross-platform successor to [Pier](https://github.com/chenqi92/Pier) (macOS-only). Same name, same purpose, different foundation — designed to run on **macOS** and **Windows** with the same engineered IDE feel.

---

## Status

The Rust backend lives in `pier-core/`; the active desktop shell now lives in `pier-ui-gpui/`. The old `pier-ui-tauri/` shell remains in the repository as an archived migration reference only.

See [docs/ROADMAP.md](./docs/ROADMAP.md) for the active delivery plan, [docs/GPUI-RESET.md](./docs/GPUI-RESET.md) for the shell reset baseline, and [docs/ARCHIVE-TAURI-SHELL.md](./docs/ARCHIVE-TAURI-SHELL.md) for the archived Tauri note.

- ✅ Rust backend foundation in `pier-core/`
- ✅ Root Cargo workspace for `pier-core/` + `pier-ui-gpui/`
- ✅ New GPUI desktop shell scaffold in `pier-ui-gpui/`
- ✅ `gpui-component` root/window kit integrated into the active shell
- ✅ First native Rust dashboard rendering `pier-core` data without IPC
- ✅ Repo-root entrypoints now target the GPUI shell
- ✅ Archived Tauri shell kept as migration reference only
- ⬜ Replace the placeholder dashboard with a real workbench and dock layout
- ⬜ Rebuild terminal, Git, SSH, and data panels as native GPUI views
- ⬜ Retire the archived Tauri shell after GPUI parity

---

## Architecture

```
┌────────────────────────────────────────────────────┐
│                GPUI (desktop shell)                │  pier-ui-gpui/
├────────────────────────────────────────────────────┤
│        Rust app state / shell orchestration        │
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

- **Rust 1.88+**
- A desktop GPU driver supported by GPUI on your platform

Run the active shell directly:

```bash
cargo run -p pier-ui-gpui
```

Build the active shell directly:

```bash
cargo build -p pier-ui-gpui --release
```

### Quickstart

The repo ships with one-shot scripts that launch the active shell for you.

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

The scripts run the matching Cargo command. `run.*` launches `cargo run -p pier-ui-gpui`; `build.*` runs `cargo build -p pier-ui-gpui`.

The scripts honour these environment variables:

| Variable | Default | Purpose |
|---|---|---|
| `BUILD_TYPE` | `Debug` for `run.*`, `Release` for `build.*` | Maps to Cargo debug vs release mode |
| `BUILD_DIR` | Cargo default target dir | When set, exported as `CARGO_TARGET_DIR` |
| `PIER_UI_CRATE` | `pier-ui-gpui` | Override the active shell crate name |

### Startup Test Path

Use this quick path when you want to sanity-check a fresh build before handing it to someone else:

1. Start the app with `./run.sh` on macOS/Linux or `.\run.ps1` on Windows.
2. Open the `Terminal` panel.
3. Print or paste three target classes into the terminal output:
   - a local path such as `README.md` or `pier-ui-gpui/src/views/terminal.rs`
   - a `file://` URI pointing at a local file
   - a web URL such as `https://example.com`
4. Verify the `Inspector` side pane can keep browsing after opening a local file or directory:
   - directory entries should drill down in place
   - file previews should offer `Open Parent` plus `Expanded Preview` when the compact preview truncates

### Terminal Open-Target Shortcuts

The terminal recognizes hovered or clicked local paths, `file://` URIs, and web URLs.

- `Cmd+Click` on macOS or `Ctrl+Click` on Windows/Linux: open the target under the pointer
- `Alt+Enter`: open the currently hovered target
- `Alt+C`: copy the currently hovered target
- `Alt+O`: open the parent folder for the currently hovered local path or `file://` target

Expected behavior by target type:

- Local path: opens inside Pier-X `Inspector`
- `file://` URI: resolves to the local file and opens inside `Inspector`
- URL: opens with the platform's external handler

---

## Project layout

```
Pier-X/
├── pier-core/               # Rust core engine
├── pier-ui-gpui/            # Active native GPUI shell
├── pier-ui-tauri/           # Archived Tauri shell reference
├── docs/
│   ├── ROADMAP.md
│   ├── GPUI-RESET.md
│   └── ARCHIVE-TAURI-SHELL.md
└── .agents/skills/          # Archived design references and repo automation skills
```

---

## License

MIT © 2026 [kkape.com](https://kkape.com)
