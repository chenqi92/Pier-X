//! Platform-specific UI integrations.
//!
//! This module is the narrow hatch through which the GPUI shell reads
//! per-OS appearance values that GPUI itself doesn't surface — today
//! that's the system accent color; eventually it may grow to include
//! Dynamic Type scale on macOS, high-contrast mode, etc.
//!
//! Every call here is cheap and synchronous (suitable for startup /
//! settings reload). Nothing in this module belongs in `pier-core`,
//! which stays UI-agnostic per the CLAUDE.md architecture rules.

pub mod accent;
