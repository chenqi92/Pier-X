//! Per-source extractors — each returns a `CommandPack` populated
//! with whatever subcommands + options it could find.
//!
//! All three sources accept the bare command name; they spawn the
//! tool themselves with the appropriate args. They MUST NOT panic;
//! errors come back as `Err(String)` describing the failure so the
//! caller can decide whether to retry with another source or skip.

pub mod completion_zsh;
pub mod help;
pub mod man;
