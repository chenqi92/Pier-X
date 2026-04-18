//! Pier-X core engine.
//!
//! Cross-platform Rust crate that powers Pier-X. The full module set
//! (terminal, ssh, sftp, rdp, vnc, db, crypto, git, search) is being ported
//! incrementally from the macOS-only Pier project. This file is the public
//! surface; modules are added one at a time as they're proven cross-platform.
//!
//! ## Architectural rule
//!
//! `pier-core` MUST NOT depend on any UI types or shell frameworks.
//! The active repository consumes this crate directly from
//! `pier-ui-gpui`.

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

pub mod connections;
pub mod credentials;
pub mod db_connections;
pub mod git_graph;
pub mod markdown;
pub mod paths;
pub(crate) mod process_util;
pub mod services;
pub mod settings;
pub mod ssh;
pub mod terminal;

/// Crate version, derived from `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_populated() {
        // Should match semver-ish pattern
        assert!(VERSION.split('.').count() >= 2);
    }
}
