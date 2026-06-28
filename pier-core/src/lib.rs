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
//! The active repository consumes this crate directly from `src-tauri/`.

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![allow(clippy::too_many_arguments)]

pub mod connections;
pub mod credentials;
pub mod egress;
pub mod git_graph;
pub mod local_secret_store;
pub mod logging;
pub mod markdown;
pub mod paths;
pub mod preview;
pub(crate) mod process_util;
pub mod remote_desktop;
pub mod services;
pub mod sql_guard;
pub mod ssh;
pub mod sudo;
pub mod terminal;

/// Crate version, derived from `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Generate a cryptographically-random, lowercase-hex token of
/// `byte_len` random bytes (the returned string is `2 * byte_len`
/// characters). Backed by the OS CSPRNG via `getrandom`. Used for
/// capability tokens that must be unguessable — e.g. the `pierfs`
/// asset-protocol grants in the Tauri layer.
pub fn random_hex_token(byte_len: usize) -> std::io::Result<String> {
    let mut buf = vec![0u8; byte_len];
    getrandom::getrandom(&mut buf).map_err(std::io::Error::other)?;
    use std::fmt::Write as _;
    Ok(buf
        .iter()
        .fold(String::with_capacity(byte_len * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_populated() {
        // Should match semver-ish pattern
        assert!(VERSION.split('.').count() >= 2);
    }
}
