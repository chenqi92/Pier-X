# pier-core

Cross-platform Rust core engine for Pier-X.

## Status

Skeleton crate. Three modules in place; the protocol modules
(terminal/SSH/RDP/VNC/databases) will land incrementally.

| Module | Status | Notes |
|---|---|---|
| `paths` | ✅ | Cross-platform app data dirs via `directories` |
| `credentials` | ✅ | OS keyring via `keyring` (Keychain / DPAPI / Secret Service) |
| `ffi` | ✅ (skeleton) | Stable C ABI surface — `pier_core_version`, etc. |
| `terminal` | ⬜ | PTY (`forkpty` Unix + `ConPTY` Windows) + VTE |
| `ssh` | ⬜ | `russh` + `russh-sftp` |
| `rdp` | ⬜ | `ironrdp` |
| `vnc` | ⬜ | `vnc-rs` or `libvncclient` wrapper |
| `db` | ⬜ | MySQL / PostgreSQL / Redis clients |
| `git` | ⬜ | `git2` |
| `search` | ⬜ | `ignore` |
| `crypto` | ⬜ | `ring` (AES-256-GCM) |

## Design rules

`pier-core` is **the asset**. The Qt UI layer is **the consumable**.

1. `pier-core` MUST NOT depend on any UI types (Qt, QML, Slint, etc.)
2. Public APIs go through either the C ABI (`ffi` module) or pure Rust traits
3. The crate must compile and test cleanly on macOS + Windows + Linux

See `docs/TECH-STACK.md §12` for the full architectural rationale.

## Build

```bash
cargo build
cargo test
```

## CI

The `cargo` job in `.github/workflows/ci.yml` builds and tests `pier-core`
on Windows, macOS, and Linux. The keyring round-trip test is `#[ignore]`
because the Linux runner has no unlocked secret service.
