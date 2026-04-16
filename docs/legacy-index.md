# Legacy Reference Index

This page is a lookup table for content removed from the active tree when Pier-X migrated to pure Rust + GPUI. Use it to recover historical implementations from git history without keeping the bytes in the working tree.

## What was removed

- `pier-ui-tauri/` — entire React/Tauri shell (5.2 GB on disk including local `node_modules`/`dist`/`target`; 116 tracked source files). Removed in the PR1 cleanup.
- `pier-ui-qt/` — Qt 6 + QML shell. Already removed earlier in commit `5b75c56`.
- `build/` — CMake build artifacts for the Qt shell. Removed from disk; was never tracked (covered by `.gitignore`).

## What stayed for reference

A curated set of 10 QML files lives at [`docs/legacy-qml-reference/`](legacy-qml-reference/) — kept as **pixel-level design source** for the GPUI port, not as runnable code:

| File | Why kept |
|---|---|
| `shell/WelcomeView.qml` | Cover page being ported to `pier-ui-gpui/src/views/welcome.rs` (PR4) |
| `components/PrimaryButton.qml` | Reference for `Button::primary` variant |
| `components/GhostButton.qml` | Reference for `Button::ghost` variant |
| `components/IconButton.qml` | Reference for `Button::icon` variant |
| `components/Card.qml` | Reference for `Card` component |
| `components/StatusPill.qml` | Reference for `StatusPill` component |
| `components/SectionLabel.qml` | Reference for `SectionLabel` component |
| `components/PopoverPanel.qml` | Reference for future popover/menu component |
| `components/PierTextField.qml` | Reference for future text input component |
| `components/TerminalTab.qml` | Reference for PR6 terminal tab chrome |

## Recovering anything else

Both shells are intact in git history. To inspect a removed file at its last committed state:

```sh
# Tauri shell (any path under pier-ui-tauri/)
git log -- pier-ui-tauri/<path>          # find commits that touched it
git show <hash>:pier-ui-tauri/<path>     # print contents at that commit

# Qt shell (any QML/CMake file)
git show 5b75c56^:pier-ui-qt/<path>      # last commit before deletion
git log --all --diff-filter=D --name-only -- pier-ui-qt/  # list every removed Qt file
```

Useful anchor commits:

| Commit | What it contains |
|---|---|
| `33bb575` | First GPUI shell scaffold (current baseline) |
| `5b75c56` | Last commit *before* the Qt shell was removed — full Qt source available at parent |
| `10c51ba` | Initial Tauri scaffold |
| `485a5d9` | Initial Qt shell scaffold |

## Why we deleted instead of archiving

- Cargo workspace already excluded both shells; nothing built or ran against them.
- `pier-core` is UI-framework-agnostic and has no IPC bridge to either shell — there is no compatibility layer left to maintain.
- 6.2 GB of dormant code on every clone is worse than `git show` on demand.
- The visual design intent is preserved by the curated QML reference plus the `pier-design-system` SKILL.

## What must NOT come back

The cleanup is one-way. Do not reintroduce:

- `tauri`, `@tauri-apps/*`, any `npm`/`pnpm`/`vite`/TypeScript toolchain
- `qt6-*`, `cmake`, `qmake`, any `.qrc`/`.pro`/`CMakeLists.txt`
- IPC bridges between `pier-core` and a non-Rust UI

`pier-ui-gpui` calls `pier-core` directly as Rust functions. That is the architecture.
