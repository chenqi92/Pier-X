//! Cross-platform application data paths.
//!
//! Uses the `directories` crate to resolve OS-appropriate locations:
//! - macOS:  `~/Library/Application Support/com.kkape.pier-x/`
//! - Windows: `%APPDATA%\kkape\pier-x\`
//! - Linux:  `~/.config/pier-x/`

use std::path::PathBuf;

use directories::ProjectDirs;

const QUALIFIER: &str = "com";
const ORGANIZATION: &str = "kkape";
const APPLICATION: &str = "pier-x";

/// Returns the project directories handle, or `None` if no valid home is set.
fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
}

/// Configuration directory (e.g. `~/.config/pier-x` on Linux).
pub fn config_dir() -> Option<PathBuf> {
    project_dirs().map(|d| d.config_dir().to_path_buf())
}

/// Per-user data directory (connections, history, etc.).
pub fn data_dir() -> Option<PathBuf> {
    project_dirs().map(|d| d.data_dir().to_path_buf())
}

/// Cache directory — anything that can be regenerated.
pub fn cache_dir() -> Option<PathBuf> {
    project_dirs().map(|d| d.cache_dir().to_path_buf())
}

/// Directory for diagnostic logs captured by the desktop shell.
pub fn logs_dir() -> Option<PathBuf> {
    cache_dir().map(|d| d.join("logs"))
}

/// Path to the persisted connections JSON file. Lives under the
/// data directory (not the config directory) because the file is
/// machine-state, not user-edited config.
///
/// Returns `None` only if no home directory can be resolved at
/// all — every supported platform produces a valid value in
/// practice.
pub fn connections_file() -> Option<PathBuf> {
    data_dir().map(|d| d.join("connections.json"))
}

/// Path to the persisted database connections JSON file. Sibling
/// of [`connections_file`] but stored separately so the SSH and DB
/// sidebar lists can evolve their schemas independently.
pub fn db_connections_file() -> Option<PathBuf> {
    data_dir().map(|d| d.join("db-connections.json"))
}

/// Path to the persisted UI settings JSON file.
///
/// Settings live under the config directory because they are user
/// preferences rather than machine-state caches.
pub fn settings_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("settings.json"))
}

/// Path to the GPUI desktop shell log file.
pub fn ui_log_file() -> Option<PathBuf> {
    logs_dir().map(|d| d.join("pier-ui-gpui.log"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_resolvable() {
        // We don't assert specific values — those are OS-dependent — but
        // at least one of the directories should exist on a sane system.
        let any = config_dir().is_some() || data_dir().is_some() || cache_dir().is_some();
        assert!(any, "no app directories resolvable");
    }
}
