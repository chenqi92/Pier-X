//! Shared subprocess helpers for background work.
//!
//! Pier-X is a GUI app on Windows. Any background `git`, `sqlite3`,
//! `ssh`, `cmd`, or `powershell.exe` process that is spawned without
//! explicit console suppression can flash a transient terminal window.
//! This module centralizes the platform-specific process tweaks so
//! service code can stay focused on arguments and parsing.

use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Configure a subprocess that should run fully in the background.
pub fn configure_background_command(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    // On non-Windows targets we don't need any tweaks; the `command`
    // parameter is consumed by the `cfg(windows)` branch so marking it
    // `_` here would change the public signature. Silence the unused
    // lint locally instead.
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}
