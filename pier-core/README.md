# pier-core

Cross-platform Rust core engine for Pier-X.

## Status

`pier-core` is the in-process backend crate consumed by `src-tauri/`.

| Module | Status | Notes |
|---|---|---|
| `connections` | ✅ | Saved connection persistence |
| `paths` | ✅ | Cross-platform app data dirs via `directories` |
| `credentials` | ✅ | OS keyring via `keyring` (Keychain / DPAPI / Secret Service) |
| `terminal` | ✅ | Local + SSH-backed terminal sessions |
| `ssh` | ✅ | Sessions, tunnels, SFTP plumbing, service detection |
| `services` | ✅ | Docker / MySQL / PostgreSQL / Redis / SQLite / search |
| `git_graph` | ✅ | Git history and graph helpers |
| `markdown` | ✅ | Markdown rendering helpers |

## Design rules

`pier-core` is **the durable backend**. App runtimes consume it directly as a
Rust crate.

1. `pier-core` MUST NOT depend on any UI types or UI frameworks
2. Public APIs should stay durable and runtime-agnostic
3. The crate must compile and test cleanly on macOS + Windows + Linux

## Build

```bash
cargo build
cargo test
```

## CI

The `cargo` job in `.github/workflows/ci.yml` builds and tests `pier-core`
on Windows, macOS, and Linux. The keyring round-trip test is `#[ignore]`
because the Linux runner has no unlocked secret service.
