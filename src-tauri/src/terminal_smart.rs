//! Smart-mode Tauri commands.
//!
//! Thin wrappers around `pier_core::terminal::*` smart-mode pieces.
//! Kept in a sibling module rather than directly in `lib.rs` so the
//! M3..M6 surface (validation, completions, history, man-page
//! summaries) doesn't bloat the already-large root command file.
//!
//! Pure-IPC layer — every business-logic decision belongs in
//! `pier-core`. The shapes here just (de)serialise and forward.

use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use pier_core::terminal::{
    complete_with_library, history_append, history_clear, history_load, man_synopsis,
    validate_command, CommandKind, Completion, Library, ManSynopsis,
};
use serde::Serialize;

/// Process-global command library. Wrapped in a `RwLock` so the
/// Settings UI can hot-reload after a user installs / removes a
/// pack on disk — every Tab acquires a read lock; the writer path
/// (Phase D's "reload library" command) takes the write lock for
/// the brief window where it rebuilds the map.
static COMPLETION_LIBRARY: OnceLock<RwLock<Library>> = OnceLock::new();

/// Tauri sets this once at startup with the resolved
/// `${app_data}/Pier-X/completions/packs/` path. We keep it in
/// the cell so subsequent `reload_completion_library` calls don't
/// have to re-resolve the dir.
static USER_PACK_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Set once at startup by the Tauri builder with the resolved
/// user pack directory. Subsequent reloads use this path.
pub fn init_user_pack_dir(dir: Option<PathBuf>) {
    let _ = USER_PACK_DIR.set(dir);
}

fn user_pack_dir() -> Option<&'static PathBuf> {
    USER_PACK_DIR.get().and_then(Option::as_ref)
}

fn completion_library() -> &'static RwLock<Library> {
    COMPLETION_LIBRARY.get_or_init(|| {
        let dir = user_pack_dir().map(|p| p.as_path());
        RwLock::new(Library::from_bundled_and_dir(dir))
    })
}

/// Re-read user packs from disk and swap them into the live
/// library. Called by the Settings UI after a "Check for updates"
/// or manual install. Bundled packs are unchanged.
pub fn reload_completion_library() {
    let dir = user_pack_dir().map(|p| p.as_path());
    let next = Library::from_bundled_and_dir(dir);
    if let Ok(mut guard) = completion_library().write() {
        *guard = next;
    }
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
    let lib_lock = completion_library();
    let guard = match lib_lock.read() {
        Ok(g) => g,
        // Lock poisoned (writer panicked). Recover by reading
        // anyway — the library state is just data, no invariant
        // a panic could have left half-broken.
        Err(p) => p.into_inner(),
    };
    complete_with_library(&line, cursor, cwd_path, &guard, locale_str)
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

// ── Smart-mode command library — Settings panel surface ──────────
//
// Phase D. Three commands drive the Settings UI:
//
// * `completion_library_list` — dump every loaded pack as a
//   shallow summary (no nested options/subcommands; the UI only
//   needs the row count for display).
// * `completion_library_reload` — re-read user packs from disk.
//   Called by the Settings UI after the user installs a pack
//   manually or hits "Update all".
// * `completion_library_install_pack` — write a JSON body to the
//   user pack dir as `<command>.json`. The body is validated as a
//   `CommandPack` before the file lands; malformed input gets
//   rejected without touching disk.

/// One row in the Settings library table. Same shape as the
/// frontend's TS `LibraryEntry`. We strip the heavy
/// `subcommands` / `options` arrays — Settings only shows
/// counts, not the actual data.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    /// `command` name, e.g. `"docker"`.
    pub command: String,
    /// Upstream tool version captured at import time. Empty when
    /// the importer couldn't parse `--version` output.
    pub tool_version: String,
    /// `"bundled-seed"` / `"auto-imported"` / `"user"`. Maps to
    /// the Settings UI's source pill (`bundled` / `user`).
    pub source: String,
    /// `"completion-zsh"` / `"man"` / `"help"` / `"hand-curated"`.
    pub import_method: String,
    /// ISO-8601 (`YYYY-MM-DD`) date the pack was generated.
    pub import_date: String,
    /// Number of subcommands the pack carries.
    pub subcommand_count: usize,
    /// Number of top-level option flags.
    pub option_count: usize,
    /// Sorted list of locale tags present somewhere in the pack
    /// (top-level options + subcommands collapsed). Helps Settings
    /// surface "中文覆盖度" without dumping every i18n map.
    pub locales: Vec<String>,
}

/// Snapshot for the Settings library page.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySnapshot {
    pub entries: Vec<LibraryEntry>,
    /// Absolute path to the user pack directory. Empty when the
    /// platform doesn't have an `app_data_dir` (rare). Settings
    /// shows this as a small footer so power users can drop their
    /// own files in.
    pub user_dir: String,
}

#[tauri::command]
pub fn completion_library_list() -> LibrarySnapshot {
    use std::collections::BTreeSet;

    let lib_lock = completion_library();
    let guard = match lib_lock.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let entries: Vec<LibraryEntry> = guard
        .iter()
        .map(|(name, pack)| {
            let mut locales: BTreeSet<&str> = BTreeSet::new();
            for opt in &pack.options {
                for k in opt.i18n.keys() {
                    locales.insert(k.as_str());
                }
            }
            for sub in &pack.subcommands {
                for k in sub.i18n.keys() {
                    locales.insert(k.as_str());
                }
            }
            LibraryEntry {
                command: name.to_string(),
                tool_version: pack.tool_version.clone(),
                source: pack.source.clone(),
                import_method: pack.import_method.clone(),
                import_date: pack.import_date.clone(),
                subcommand_count: pack.subcommands.len(),
                option_count: pack.options.len(),
                locales: locales.into_iter().map(String::from).collect(),
            }
        })
        .collect();
    LibrarySnapshot {
        entries,
        user_dir: user_pack_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    }
}

#[tauri::command]
pub fn completion_library_reload() -> LibrarySnapshot {
    reload_completion_library();
    completion_library_list()
}

/// Install (or replace) a user pack. `body` is the raw JSON; the
/// command validates it as a `CommandPack` before writing to
/// `${user_pack_dir}/<command>.json`. Returns the snapshot
/// post-install so the UI can re-render in one round-trip.
#[tauri::command]
pub fn completion_library_install_pack(body: String) -> Result<LibrarySnapshot, String> {
    use pier_core::terminal::CommandPack;
    let pack: CommandPack = serde_json::from_str(&body).map_err(|e| format!("invalid JSON: {e}"))?;
    if pack.command.is_empty() {
        return Err(String::from("pack `command` field is empty"));
    }
    if pack
        .command
        .chars()
        .any(|c| matches!(c, '/' | '\\' | '.' | '\0'))
    {
        return Err(format!(
            "refusing unsafe command name {:?} (path-like)",
            pack.command
        ));
    }
    let dir = user_pack_dir()
        .ok_or_else(|| String::from("no app_data_dir on this platform — cannot persist"))?
        .clone();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", pack.command));
    let pretty = serde_json::to_string_pretty(&pack).map_err(|e| e.to_string())?;
    // Atomic write: temp file + rename so the live library never
    // sees a partially-written file mid-load.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, pretty).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    reload_completion_library();
    Ok(completion_library_list())
}

/// Remove a user pack. Bundled packs can't be removed — they're
/// embedded in the binary; the UI hides the remove action for
/// rows whose `source` is `"bundled-seed"`.
#[tauri::command]
pub fn completion_library_remove_pack(command: String) -> Result<LibrarySnapshot, String> {
    if command.is_empty()
        || command
            .chars()
            .any(|c| matches!(c, '/' | '\\' | '.' | '\0'))
    {
        return Err(format!("refusing unsafe command name {:?}", command));
    }
    let dir = user_pack_dir()
        .ok_or_else(|| String::from("no app_data_dir on this platform"))?
        .clone();
    let path = dir.join(format!("{}.json", command));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    reload_completion_library();
    Ok(completion_library_list())
}
