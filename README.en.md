<div align="center">
  <img src="public/pier-icon.png" alt="Pier-X" width="96" />
  <h1>Pier-X</h1>
  <p><strong>An IDE-style desktop workbench for terminal, Git, SSH, databases, and remote ops.</strong></p>
  <p>
    <a href="README.md">中文</a> ·
    <a href="README.en.md">English</a>
  </p>
</div>

---

Pier-X is the cross-platform successor to [Pier](https://github.com/chenqi92/Pier) (macOS-only) — same name, same purpose, rebuilt for backend / SRE engineers who need one app instead of five. The stack is **Rust core + Tauri 2 + React + TypeScript**, targeting **macOS** and **Windows** first with Linux kept on the long-term path.

> The full product spec lives in [docs/PRODUCT-SPEC.md](docs/PRODUCT-SPEC.md) (Chinese, authoritative). Visual tokens are in [.agents/skills/pier-design-system/SKILL.md](.agents/skills/pier-design-system/SKILL.md). Code rules are in [CLAUDE.md](CLAUDE.md).

## Features

The UI is a three-pane IDE layout — **left Sidebar + center Tab workspace + right tool panel** — and every Tab carries its own "right tool" preference.

### Center workspace

- **Terminal** — xterm.js + `pier-core::terminal::PierTerminal`.
  - Three backends: local PTY (forkpty / ConPTY), SSH shell, and saved SSH connections (passwords resolved from the OS keyring at connect time).
  - 256 / RGB color, SGR, visual + audio bell, configurable scrollback, copy-on-select / clipboard paste, custom right-click menu.
- **Markdown** — auto-renders the `.md` file selected in the left Sidebar (pulldown-cmark, CommonMark + GFM).
- **Welcome** — shown when no Tab is open: new local terminal / new SSH / recent connections / settings / command palette.

### Left Sidebar

- **Files** — rooted at `~`, with breadcrumbs and a Places dropdown. Click a Markdown file to preview on the right; double-click a directory to open a local terminal there.
- **Servers** — every saved SSH connection (YAML file + OS keyring). Search, edit, delete; click to open an SSH Tab.

### Right tool panels (per-Tab)

| Tool | Scope | Highlights |
|---|---|---|
| **Git** | Any | Overview · diff · stage / unstage · commit · push / pull · branches · history graph (`git2` topology) · stash · tags · remotes · config · rebase · submodules · conflicts |
| **Server Monitor** | SSH | uptime · load · memory/swap · disk · CPU% · process list (user-triggered, no polling) |
| **Docker** | Local / SSH | Containers / Images / Volumes / Networks / Compose Projects; start · stop · restart · remove · inspect · pull · prune · registry proxy |
| **MySQL / PostgreSQL** | Any | Auto SSH tunnel; database / schema / table browser; CodeMirror SQL editor; result grid + TSV export; **read-only by default**, writes require explicit unlock + confirmation |
| **Redis** | Any | Pattern scan + TTL; string / list / hash / set / zset / stream detail; command editor; dangerous commands (FLUSHALL / KEYS \*) require confirmation |
| **SQLite** | Local | Pick a `.db` file; table/column metadata; queries; same read-only default |
| **Log** | SSH | File / System (syslog / nginx / dmesg / journald / docker) / Custom sources; frontend-driven drain model to avoid event storms |
| **SFTP** | SSH | Remote file browser, upload / download (progress events), chmod dialog, in-panel CodeMirror editor (≤ 5 MB, UTF-8 lossy substitution + warning bar) |
| **Firewall** | SSH | Auto-detected backend (firewalld / ufw / nft / iptables); Listening / Rules / Mappings / Traffic tabs; **writes are injected into the terminal** for the user to review, never executed silently |
| **Markdown** | Any | Renders the `.md` file selected on the left |

### Cross-cutting

- **Command palette** (`⌘K` / `Ctrl+K`), new terminal (`⌘T`), new SSH (`⌘N`), close Tab (`⌘W`), settings (`⌘,`), Git panel (`⌘⇧G`).
- **Theming** — `dark` / `light` / `system`, every visual value sourced from `src/styles/tokens.css`.
- **i18n** — English and Simplified Chinese.
- **Credentials** — SSH passwords and key passphrases go through `pier-core::credentials` → OS keyring (macOS Keychain / Windows Credential Manager / Linux secret-service). Never written to files or logs.
- **SSH tunnel manager** — `PortForwardDialog` lists every active local forward and lets you add / close them; tunnels auto-opened by DB / Log panels show up here too.

## Architecture

```
┌────────────────────────────────────────────────────┐
│        Tauri 2 + React 19 + TypeScript (shell)     │  src/
├────────────────────────────────────────────────────┤
│              Tauri command layer (Rust)            │  src-tauri/
├────────────────────────────────────────────────────┤
│              pier-core (Rust core)                 │  pier-core/
├────────────────────────────────────────────────────┤
│  PTY · SSH · SFTP · Git · MySQL · PG · SQLite ·    │
│  Redis · Docker · Server Monitor · Markdown · …    │
└────────────────────────────────────────────────────┘
```

Hard rules (see [CLAUDE.md](CLAUDE.md) for the full list):

- `pier-core` depends on **no** UI crate (no `tauri`, no `gpui`, no `qt`).
- The frontend never bypasses Tauri to reach `pier-core`.
- Tauri commands stay thin; business logic lives in `pier-core`.

## Build & run

### Requirements

- Node.js 24+, npm 11+
- Rust 1.88+
- WebView2 runtime on Windows

### Commands

```bash
npm install                 # install frontend deps
npm run tauri dev           # dev: vite + tauri dev
npm run tauri build         # release build
npm run build:debug         # debug build
cargo build -p pier-core    # backend only
```

### Releases

Version sync + tag:

```bash
npm run bump 0.2.0          # explicit
npm run bump patch          # patch / minor / major
git push && git push --tags
```

Pushing a `v*.*.*` tag triggers:

- **GitHub** (`.github/workflows/release.yml`) — builds Linux, Windows x64, Windows ARM64, and macOS universal Tauri bundles, publishes to GitHub Releases.
- **Gitea** (`.gitea/workflows/release.yml`) — builds Linux `.deb` / `.rpm` / `.AppImage` on `ubuntu-22.04`, uploads via the Gitea API.

CI (`.github/workflows/ci.yml`): Tauri shell on macOS + Windows; Rust core on macOS + Windows + Linux (`fmt --check` + `clippy` + `build` + `test`).

## Project layout

```
Pier-X/
├── Cargo.toml               # Cargo workspace (members: pier-core, src-tauri)
├── package.json             # Frontend entrypoint (npm run tauri …)
├── src/                     # React frontend (active desktop shell)
│   ├── shell/               # TopBar / Sidebar / TabBar / StatusBar / dialogs
│   ├── panels/              # 12 tool panels (Git / Terminal / SFTP / DB / Docker / …)
│   ├── components/          # Reusable UI atoms
│   ├── stores/              # zustand state
│   ├── lib/                 # Tauri command wrappers, pure helpers
│   ├── i18n/                # en / zh resources
│   └── styles/              # tokens.css (single source of truth) + scoped sheets
├── src-tauri/               # Tauri runtime + Rust commands
├── pier-core/               # Rust core (terminal / ssh / services / …)
├── docs/
│   ├── PRODUCT-SPEC.md      # Product spec (authoritative)
│   └── BACKEND-GAPS.md      # Design → impl gap tracker
├── .agents/skills/          # Design system SKILL and repo automation
├── scripts/bump-version.mjs # Version sync + tag
└── .github/ · .gitea/       # CI / Release workflows
```

## Docs

| File | Purpose |
|---|---|
| [docs/PRODUCT-SPEC.md](docs/PRODUCT-SPEC.md) | Product spec — single source of truth for "what Pier-X is, which panels exist, default behaviors, non-goals" |
| [docs/BACKEND-GAPS.md](docs/BACKEND-GAPS.md) | Tracks gaps between the frontend design and wired backend commands |
| [.agents/skills/pier-design-system/SKILL.md](.agents/skills/pier-design-system/SKILL.md) | Single source of truth for visual tokens (color / typography / spacing / radius / shadow) |
| [CLAUDE.md](CLAUDE.md) | Code rules and architecture boundaries for AI assistants and contributors |
| [pier-core/README.md](pier-core/README.md) | Rust core crate contract |

## License

MIT © 2026 [kkape.com](https://kkape.com)
