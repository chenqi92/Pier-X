<div align="center">
  <img src="public/pier-icon.png" alt="Pier-X" width="96" />
  <h1>Pier-X</h1>
  <p><strong>An IDE-style desktop workbench for terminal, Git, SSH, databases, AI assistance, and remote ops.</strong></p>
  <p>
    <a href="README.md">中文</a> ·
    <a href="README.en.md">English</a>
  </p>
</div>

---

Pier-X is the cross-platform successor to [Pier](https://github.com/chenqi92/Pier) (macOS-only): same name, same purpose, rebuilt for backend / SRE engineers who need one app instead of five. The stack is **Rust core + Tauri 2 + React 19 + TypeScript**, targeting **macOS** and **Windows** first with Linux kept on the long-term path.

> The full product spec lives in [docs/PRODUCT-SPEC.md](docs/PRODUCT-SPEC.md) (Chinese, authoritative). Visual tokens are in [.agents/skills/pier-design-system/SKILL.md](.agents/skills/pier-design-system/SKILL.md). Code rules are in [CLAUDE.md](CLAUDE.md).

## Screenshots

<table>
  <tr>
    <td width="50%" align="center"><img src="screenShots/ai.png" alt="AI assistant" width="100%"><br><sub><b>AI assistant</b></sub></td>
    <td width="50%" align="center"><img src="screenShots/git.png" alt="Git" width="100%"><br><sub><b>Git</b></sub></td>
  </tr>
  <tr>
    <td width="50%" align="center"><img src="screenShots/md.png" alt="Markdown preview" width="100%"><br><sub><b>Markdown preview</b></sub></td>
    <td width="50%" align="center"><img src="screenShots/监控.png" alt="Server Monitor" width="100%"><br><sub><b>Server Monitor</b></sub></td>
  </tr>
  <tr>
    <td width="50%" align="center"><img src="screenShots/docker.png" alt="Docker" width="100%"><br><sub><b>Docker</b></sub></td>
    <td width="50%" align="center"><img src="screenShots/sftp.png" alt="SFTP" width="100%"><br><sub><b>SFTP</b></sub></td>
  </tr>
  <tr>
    <td width="50%" align="center"><img src="screenShots/db.png" alt="Database" width="100%"><br><sub><b>Database (MySQL / PostgreSQL)</b></sub></td>
    <td width="50%" align="center"><img src="screenShots/redis.png" alt="Redis" width="100%"><br><sub><b>Redis</b></sub></td>
  </tr>
  <tr>
    <td width="50%" align="center"><img src="screenShots/web.png" alt="Web Server" width="100%"><br><sub><b>Web Server</b></sub></td>
    <td width="50%" align="center"><img src="screenShots/软件.png" alt="Software" width="100%"><br><sub><b>Software</b></sub></td>
  </tr>
  <tr>
    <td width="50%" align="center"><img src="screenShots/日志.png" alt="Logs" width="100%"><br><sub><b>Logs</b></sub></td>
    <td width="50%" align="center"><img src="screenShots/端口.png" alt="Port forwarding" width="100%"><br><sub><b>Port forwarding</b></sub></td>
  </tr>
</table>

## Features

The UI is a three-pane IDE layout: **left Sidebar + center Tab workspace + right tool panel**. Every Tab carries its own right-tool preference, and SSH state, database connections, SFTP paths, and AI conversations follow the active Tab.

### Center Workspace

- **Terminal**: xterm.js + `pier-core::terminal::PierTerminal`.
  - Three backends: local PTY (forkpty / ConPTY), SSH shell, and saved SSH connections (credentials resolved from the OS keyring).
  - 256 / RGB color, SGR, visual / audio bell, scrollback, copy selection / paste clipboard, custom right-click menu.
  - Optional **Smart Mode**: syntax highlighting, Tab completion, history / autosuggest, man-page summaries, command-library import; history is memory-only by default and can be persisted with sensitive-line filtering.
- **Markdown**: auto-renders the `.md` file selected in the left Sidebar (pulldown-cmark, CommonMark + GFM).
- **Welcome**: shown when no Tab is open, with shortcuts for local terminal, SSH, recent connections, settings, and the command palette.

### Left Sidebar

- **Files**: rooted at `~`, with breadcrumbs and a Places dropdown. Click Markdown to preview; double-click a directory to open a local terminal there.
- **Servers**: saved SSH connections (YAML + OS keyring), with search, edit, delete, grouping, and health probes. Click a server to open an SSH Tab.
- **Egress Profile**: connections can bind SOCKS5 / HTTP / SSH jump / WireGuard / external VPN routes. Proxy and jump-host routes affect only the bound connection; system VPN profiles clearly warn about route impact.

### Right Tool Panels (per Tab)

| Tool | Scope | Highlights |
|---|---|---|
| **AI** | Any | BYOK assistant with Anthropic / OpenAI-compatible / Ollama providers; per-tab conversations, streaming, redaction, memory-only history option; `run_command` / `read_file` / `list_dir` / `write_file` / `monitor_snapshot` go through L0-L3 risk classification and approval cards |
| **Markdown** | Any | Renders the Markdown file selected on the left |
| **Git** | Any | Overview / diff / stage / commit / push / pull / branches / graph history / blame / stash / tags / remotes / config / rebase / submodules / conflicts |
| **Server Monitor** | Local / SSH | uptime, load, CPU, memory, swap, disks, block-device topology, network throughput, top processes; local uses `sysinfo`, remote uses SSH probes |
| **Firewall** | SSH | Auto-detects firewalld / ufw / nft / iptables; Listening / Rules / Mappings / Traffic; writes are injected into the terminal for user review, never executed silently |
| **SFTP** | SSH | Remote browser, upload / download with progress, directory transfer, chmod, copy / rename / delete, bookmarks, external edit, CodeMirror editor with 5 MB cap plus encoding / EOL detection |
| **Log** | SSH | File / System / Custom sources; journalctl, dmesg, nginx, docker logs presets; search, level counts, rate chip, time-range backfill, frontend drain model |
| **Code Search** | SSH | Searches the terminal cwd with `rg` first and `git grep` fallback; regex, case, glob options; click hits to open and position the SFTP editor |
| **Docker** | Local / SSH | Containers / Images / Volumes / Networks / Projects; start / stop / restart / remove / inspect / pull / prune; Compose projects are label-derived and do not require compose CLI |
| **MySQL / PostgreSQL** | SSH / local direct | Auto SSH tunnel, saved DB credentials, schema / table browser, SQL multi-tabs, history / favorites, format, EXPLAIN / plan, result grid, TSV export, structure and settings editing; read-only by default with explicit unlock + confirmation for writes |
| **Redis** | SSH / local direct | Key tree, SCAN paging, type / TTL / size, string / hash / list / set / zset / stream detail, CLI history, edit / delete / rename, dangerous-command confirmation |
| **SQLite** | SSH | Remote sqlite3 probe, `.db` / `.sqlite` directory scan, tables / indexes / triggers, PRAGMA, scripts, file-size chip; read-only by default |
| **Web Server** | SSH | Unified nginx / Apache / Caddy management: detection, file tree, Feature catalog, Tree / Raw editing, diff preview, batch save, validate / lint / reload, site creation and toggle, external editor |
| **Software** | SSH | Host software registry, install / update / uninstall / cancel, version picker, service start / stop / restart / reload, journal logs, mirror switching, bundles, Compose templates, K8s export, webhooks, history, `software-extras.json` custom entries |

### Cross-Cutting

- **Command palette** (`⌘K` / `Ctrl+K`), AI assistant (`⌘⇧A` / `Ctrl+Shift+A`), new terminal (`⌘T` / `Ctrl+T`), new SSH (`⌘N` / `Ctrl+N`), close Tab (`⌘W` / `Ctrl+W`), settings (`⌘,` / `Ctrl+,`), Git panel (`⌘⇧G` / `Ctrl+Shift+G`).
- **Themes and tokens**: `dark` / `light` / `system`, with every visual value sourced from `src/styles/tokens.css`.
- **i18n**: English and Simplified Chinese.
- **Credentials and safety**:
  - SSH passwords, key passphrases, DB passwords, AI API keys, Egress proxy credentials, and sudo / elevation passwords go through the OS keyring.
  - SSH host keys use OpenSSH-style `known_hosts` TOFU: first connect asks for trust, key changes are blocked and surfaced; settings can list / remove pinned keys.
  - Root-capable panels can use saved sudo passwords through `sudo -S`, and saved SSH connections can optionally auto-enter `sudo -i`; plaintext is never written to config or logs.
  - AI execution is classified in one backend path, `pier-core::services::ai::risk`: L0 read-only can run automatically, L1 needs approval, L2 needs strong confirmation and cannot be allow-listed, L3 red lines are never executed by Pier-X.
- **SSH tunnel manager**: `PortForwardDialog` lists active local forwards and lets you add / close them; tunnels auto-opened by DB / Log panels appear there too.

## Architecture

```
┌────────────────────────────────────────────────────┐
│        Tauri 2 + React 19 + TypeScript (shell)     │  src/
├────────────────────────────────────────────────────┤
│              Tauri command layer (Rust)            │  src-tauri/
├────────────────────────────────────────────────────┤
│              pier-core (Rust core)                 │  pier-core/
├────────────────────────────────────────────────────┤
│ PTY · SSH · SFTP · Git · DB · Redis · Docker · AI · │
│ Monitor · Firewall · Web Server · Software · …      │
└────────────────────────────────────────────────────┘
```

Hard rules (see [CLAUDE.md](CLAUDE.md) for the full list):

- `pier-core` depends on no UI crate (no `tauri`, no `gpui`, no `qt`).
- The frontend never bypasses Tauri commands to reach `pier-core`.
- Tauri commands stay thin; business logic lives in `pier-core`.
- No Qt / QML / CMake / Corrosion / `pier-ui-gpui` / second UI runtime.

## Build & Run

### Requirements

- Node.js 24+, npm 11+
- Rust 1.88+
- WebView2 runtime on Windows

### Commands

```bash
npm install                 # install frontend dependencies
npm run tauri dev           # dev: vite + tauri dev
npm run tauri build         # release build
npm run build:debug         # debug build
cargo build -p pier-core    # backend only
```

## Installation

The current primary install path is the platform bundle attached to [GitHub Releases](https://github.com/chenqi92/Pier-X/releases). Each GitHub Release also attaches package-manager metadata:

- Homebrew cask: `pier-x-homebrew-cask-v<version>.rb`
- Homebrew formula: `pier-x-homebrew-formula-v<version>.rb`
- WinGet manifests: `pier-x-winget-manifests-v<version>.tar.gz`
- SHA256 checksums: `pier-x-release-sha256-v<version>.txt`

Once the Homebrew tap is published, macOS users can install with:

```bash
brew install --cask chenqi92/tap/pier-x
```

Linuxbrew users can install with:

```bash
brew install chenqi92/tap/pier-x
```

After the generated WinGet manifests are accepted into the community source:

```powershell
winget install Chenqi92.PierX
```

See [docs/PACKAGE-MANAGERS.md](docs/PACKAGE-MANAGERS.md) for the maintainer workflow.

## Releases

Version sync:

```bash
npm run bump <version>      # explicit
npm run bump patch          # patch / minor / major
git push
```

Pushing a `main` branch commit that changes `package.json` version triggers:

- **GitHub** (`.github/workflows/release.yml`): builds Linux, Windows x64, Windows ARM64, and macOS universal Tauri bundles, publishes to GitHub Releases, and attaches Homebrew cask / WinGet manifests / SHA256SUMS.
- **Gitea** (`.gitea/workflows/release.yml`): builds Linux `.deb` / `.rpm` / `.AppImage` on `ubuntu-22.04`, uploads through the Gitea API.

CI (`.github/workflows/ci.yml`): Tauri shell on macOS + Windows; Rust core on macOS + Windows + Linux (`fmt --check` + `clippy` + `build` + `test`).

## Project Layout

```
Pier-X/
├── Cargo.toml               # Cargo workspace (members: pier-core, src-tauri)
├── package.json             # Frontend entrypoint (npm run tauri …)
├── src/                     # React frontend (active desktop shell)
│   ├── shell/               # TopBar / Sidebar / TabBar / StatusBar / dialogs
│   ├── panels/              # 15 right-side tool panels + terminal / WebServer subpanels
│   ├── components/          # Reusable UI atoms
│   ├── stores/              # zustand state
│   ├── lib/                 # Tauri command wrappers, AI vendors, pure helpers
│   ├── i18n/                # en / zh resources
│   └── styles/              # tokens.css (single source of truth) + scoped sheets
├── src-tauri/               # Tauri runtime + Rust commands
├── pier-core/               # Rust core (terminal / ssh / services / ai / …)
├── docs/
│   ├── PRODUCT-SPEC.md      # Product spec (authoritative)
│   ├── PACKAGE-MANAGERS.md  # Homebrew / WinGet metadata workflow
│   ├── BACKEND-GAPS.md      # Design → implementation gap tracker
│   └── SOFTWARE-EXTRAS.md   # Software-panel custom-entry format
├── .agents/skills/          # Design system SKILL and repo automation
├── scripts/                 # Version and release helper scripts
└── .github/ · .gitea/       # CI / Release workflows
```

## Docs

| File | Purpose |
|---|---|
| [docs/PRODUCT-SPEC.md](docs/PRODUCT-SPEC.md) | Product spec: the single source of truth for what Pier-X is, which panels exist, default behaviors, and non-goals |
| [docs/PACKAGE-MANAGERS.md](docs/PACKAGE-MANAGERS.md) | Homebrew tap and WinGet manifest generation, publishing, and submission workflow |
| [docs/BACKEND-GAPS.md](docs/BACKEND-GAPS.md) | Tracks gaps between the frontend design and wired backend commands |
| [docs/SOFTWARE-EXTRAS.md](docs/SOFTWARE-EXTRAS.md) | JSON format for Software-panel custom software / bundle entries |
| [.agents/skills/pier-design-system/SKILL.md](.agents/skills/pier-design-system/SKILL.md) | Single source of truth for visual tokens (color / typography / spacing / radius / shadow) |
| [CLAUDE.md](CLAUDE.md) | Code rules and architecture boundaries for AI assistants and contributors |
| [pier-core/README.md](pier-core/README.md) | Rust core crate contract |

## License

MIT © 2026 [kkape.com](https://kkape.com)
