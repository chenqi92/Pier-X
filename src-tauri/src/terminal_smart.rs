//! Smart-mode Tauri commands.
//!
//! Thin wrappers around `pier_core::terminal::*` smart-mode pieces.
//! Kept in a sibling module rather than directly in `lib.rs` so the
//! M3..M6 surface (validation, completions, history, man-page
//! summaries) doesn't bloat the already-large root command file.
//!
//! Pure-IPC layer — every business-logic decision belongs in
//! `pier-core`. The shapes here just (de)serialise and forward.

use std::sync::OnceLock;

use pier_core::terminal::{
    complete_with_library, history_append, history_clear, history_load, man_synopsis,
    validate_command, CommandKind, Completion, Library, ManSynopsis,
};
use serde::Serialize;

/// Process-global command library. Populated lazily on first call
/// to [`completion_library`] from the bundled JSON packs. Phase C
/// will widen this into a `RwLock<Library>` so user packs and
/// online updates can replace it without restart; for now it's a
/// read-only cell — the library only contains compile-time data.
static COMPLETION_LIBRARY: OnceLock<Library> = OnceLock::new();

fn completion_library() -> &'static Library {
    COMPLETION_LIBRARY.get_or_init(Library::bundled)
}

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

/// Tab-completion candidates for the input line at `cursor`.
///
/// Stateless — the caller passes the shell's last-known cwd (from
/// `terminal_current_cwd`) so this command doesn't need access to
/// `AppState`. Returning everything in one shot also keeps the IPC
/// path simple; the popover filters as the user types without
/// re-invoking until they hit Tab again.
///
/// `locale` selects the description language emitted by library-
/// driven rows (subcommands / option flags). Frontend passes the
/// active i18n locale (e.g. `"zh-CN"`); fallback chain inside the
/// library is `locale → language root → en → empty`.
#[tauri::command]
pub fn terminal_completions(
    line: String,
    cursor: usize,
    cwd: Option<String>,
    locale: Option<String>,
) -> Vec<Completion> {
    let cwd_path = cwd.as_deref().map(std::path::Path::new);
    let locale_str = locale.as_deref().unwrap_or("en");
    complete_with_library(&line, cursor, cwd_path, completion_library(), locale_str)
}

/// Look up the man-page summary (or `--help` fallback) for `cmd`.
///
/// Returns `Ok(None)` for the "no entry / no --help output" case so
/// the frontend can render an explicit "No documentation found"
/// message instead of treating it as a hard error. Genuine errors
/// (invalid name, I/O failure) come back as `Err(String)` and are
/// surfaced as toasts.
#[tauri::command]
pub fn terminal_man_synopsis(command: String) -> Result<Option<ManSynopsis>, String> {
    use pier_core::terminal::ManError;
    match man_synopsis(&command) {
        Ok(syn) => Ok(Some(syn)),
        Err(ManError::NotFound(_)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Load the persisted command-history ring for `shell` from disk.
/// Returns `Ok(vec![])` for either "no file yet" or "no usable
/// data dir on this platform" so the caller fails soft and falls
/// back to an in-memory-only history.
#[tauri::command]
pub fn terminal_history_load(shell: String) -> Result<Vec<String>, String> {
    use pier_core::terminal::HistoryError;
    match history_load(&shell) {
        Ok(v) => Ok(v),
        Err(HistoryError::NoDataDir) => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// Append `command` to `shell`'s persisted history file. Drops the
/// line silently if it matches the credential-keyword filter (see
/// `pier_core::terminal::history::is_sensitive`); the in-memory
/// ring on the frontend still keeps it for the current session.
#[tauri::command]
pub fn terminal_history_push(shell: String, command: String) -> Result<(), String> {
    use pier_core::terminal::HistoryError;
    match history_append(&shell, &command) {
        Ok(()) => Ok(()),
        Err(HistoryError::NoDataDir) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Wipe the persisted history file for `shell`. Settings exposes
/// this through a "Clear history for this shell" button so the
/// user can purge a leaked entry without having to find the file
/// on disk.
#[tauri::command]
pub fn terminal_history_clear(shell: String) -> Result<(), String> {
    use pier_core::terminal::HistoryError;
    match history_clear(&shell) {
        Ok(()) => Ok(()),
        Err(HistoryError::NoDataDir) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
