//! Pier-X core engine.
//!
//! Cross-platform Rust crate that powers Pier-X. The full module set
//! (terminal, ssh, sftp, rdp, vnc, db, crypto, git, search) is being ported
//! incrementally from the macOS-only Pier project. This file is the public
//! surface; modules are added one at a time as they're proven cross-platform.
//!
//! ## Architectural rule
//!
//! `pier-core` MUST NOT depend on any UI types (Qt, QML, Slint, etc.).
//! All public APIs are exposed via:
//!   1. A stable C ABI (the `ffi` module) — for foreign-language consumers
//!   2. Pure Rust traits — for in-process consumers
//!
//! See `docs/TECH-STACK.md §12` for the design rationale.

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

pub mod connections;
pub mod credentials;
pub mod ffi;
pub mod paths;
pub mod services;
pub mod ssh;
pub mod terminal;

/// Crate version, derived from `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_populated() {
        assert!(!VERSION.is_empty());
        // Should match semver-ish pattern
        assert!(VERSION.split('.').count() >= 2);
    }
}
