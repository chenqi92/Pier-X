//! Smart-mode Tauri commands.
//!
//! Thin wrappers around `pier_core::terminal::*` smart-mode pieces.
//! Kept in a sibling module rather than directly in `lib.rs` so the
//! M3..M6 surface (validation, completions, history, man-page
//! summaries) doesn't bloat the already-large root command file.
//!
//! Pure-IPC layer — every business-logic decision belongs in
//! `pier-core`. The shapes here just (de)serialise and forward.

use pier_core::terminal::{validate_command, CommandKind};
use serde::Serialize;

/// Result of [`terminal_validate_command`].
///
/// `kind` is one of `"builtin"` / `"binary"` / `"missing"` so the
/// frontend can branch on a discriminator without rebuilding the
/// Rust enum on the TS side. `path` carries the absolute resolved
/// binary path when `kind == "binary"`, `null` otherwise.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandValidation {
    pub kind: &'static str,
    pub path: Option<String>,
}

/// Resolve `name` against shell builtins + `$PATH`.
///
/// Called by the smart-mode syntax overlay each time it sees a new
/// command token in the user's currently-typed line. The frontend
/// caches results in a per-session LRU so a name only crosses the
/// IPC boundary once per session.
#[tauri::command]
pub fn terminal_validate_command(name: String) -> CommandValidation {
    match validate_command(&name) {
        CommandKind::Builtin => CommandValidation {
            kind: "builtin",
            path: None,
        },
        CommandKind::Binary(p) => CommandValidation {
            kind: "binary",
            path: Some(p.to_string_lossy().into_owned()),
        },
        CommandKind::Missing => CommandValidation {
            kind: "missing",
            path: None,
        },
    }
}
