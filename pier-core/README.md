# pier-core

Rust core engine for Pier-X. **Not yet implemented in this repository.**

## Plan

`pier-core` will house the cross-platform implementation of:

- Terminal emulation (`vte`) + PTY management (`forkpty` on Unix, `ConPTY` on Windows)
- SSH / SFTP client (`russh` + `russh-sftp`)
- RDP client (`ironrdp`)
- VNC client
- Database clients (MySQL / PostgreSQL / Redis)
- Local file search (`ignore`)
- Crypto (`ring`)
- Git operations (`git2`)
- Cross-platform credential storage (`keyring`)

The crate will expose a stable C ABI for the Qt UI to consume via `cxx-qt`.

## Source

Most of the existing Rust code from the macOS-only [Pier](https://github.com/chenqi92/Pier) project is cross-platform and will be ported here once the Qt UI shell is functional. The only platform-specific work needed is the ConPTY backend for Windows.

See [../docs/TECH-STACK.md](../docs/TECH-STACK.md) for the full architectural plan.
