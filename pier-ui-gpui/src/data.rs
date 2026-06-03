// Read-only data sources for the shell.
//
// Wraps pier-core (git status, saved connections) plus a std::fs directory
// listing for the local Files tree, so shell.rs renders from plain owned
// structs and never touches the backend directly. All calls here are
// synchronous and cheap (single git invocation / one read_dir / one JSON
// load); the shell calls them on construction and on demand, not per frame.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use pier_core::connections::ConnectionStore;
use pier_core::services::git::{FileStatus, GitClient};

/// One entry in the Files sidebar.
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    /// Relative modification age, e.g. "2h", "3d".
    pub age: String,
}

/// One saved SSH connection in the Servers sidebar.
pub struct ConnRow {
    pub name: String,
    pub addr: String,
    pub online: bool,
}

/// A single changed file, carrying its porcelain status for mark/colour.
pub struct GitChange {
    pub status: FileStatus,
    pub path: String,
}

/// The current repository's status, as the Git panel renders it.
pub struct GitData {
    pub branch: String,
    pub tracking: String,
    pub ahead: i32,
    pub behind: i32,
    pub staged: Vec<GitChange>,
    pub unstaged: Vec<GitChange>,
}

/// The process working directory (the repo we were launched in).
pub fn current_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Directory entries, directories first then files, each case-insensitively
/// sorted. Hidden entries are kept (the web sidebar shows `.git`, `.github`).
pub fn list_dir(path: &Path) -> Vec<FileEntry> {
    let mut entries: Vec<FileEntry> = Vec::new();
    if let Ok(read) = fs::read_dir(path) {
        for e in read.flatten() {
            let md = e.metadata().ok();
            let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let age = md
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(rel_age)
                .unwrap_or_default();
            entries.push(FileEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir,
                age,
            });
        }
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

/// Saved connections from the default store, or empty if none/unreadable.
/// Online state is unknown here (no probe), so it's reported false.
pub fn load_connections() -> Vec<ConnRow> {
    let Ok(store) = ConnectionStore::load_default() else {
        return Vec::new();
    };
    store
        .connections
        .iter()
        .map(|c| ConnRow {
            name: c.name.clone(),
            addr: format!("{}@{}:{}", c.user, c.host, c.port),
            online: false,
        })
        .collect()
}

/// Git status for the repo at `path`, or `None` if it isn't a repo.
pub fn git_status(path: &Path) -> Option<GitData> {
    let client = GitClient::open(&path.to_string_lossy()).ok()?;
    let branch = client.branch_info().ok()?;
    let changes = client.status().ok()?;
    let (mut staged, mut unstaged) = (Vec::new(), Vec::new());
    for c in changes {
        let change = GitChange {
            status: c.status,
            path: c.path,
        };
        if c.staged {
            staged.push(change);
        } else {
            unstaged.push(change);
        }
    }
    Some(GitData {
        branch: branch.name,
        tracking: branch.tracking,
        ahead: branch.ahead,
        behind: branch.behind,
        staged,
        unstaged,
    })
}

fn rel_age(t: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        86_400..=604_799 => format!("{}d", secs / 86_400),
        604_800..=2_591_999 => format!("{}w", secs / 604_800),
        _ => format!("{}mo", secs / 2_592_000),
    }
}
