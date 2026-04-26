//! Smart-mode history persistence.
//!
//! `terminal::history` writes the user's executed commands into
//! per-shell jsonl files under `~/.local/share/pier-x/` (or the
//! platform equivalent via `directories`), so the M5 autosuggest
//! survives an app restart and stops looking like an amnesiac.
//!
//! The in-memory ring in `useTerminalHistoryStore` (frontend) keeps
//! authoritative ordering for the running session — these on-disk
//! files exist purely to seed that ring on next startup. Format:
//!
//! ```text
//! {"ts":1717000000,"cmd":"git status"}
//! {"ts":1717000010,"cmd":"ls -la"}
//! ```
//!
//! One file per shell name (`bash`, `zsh`, `pwsh`, …) so the user
//! can clean up `~/.local/share/pier-x/terminal-history-bash.jsonl`
//! independently of the others, and so the per-shell load on
//! startup doesn't have to scan unrelated entries.
//!
//! ### Sensitivity filter
//!
//! Per PRODUCT-SPEC §4.2.1, lines containing common credential
//! markers are dropped before disk write — the in-memory ring still
//! holds them so the current session's autosuggest works, but they
//! never land on the filesystem. Filtering is intentionally
//! conservative on the side of dropping (false-positives like
//! `man secret` lose disk persistence; that's an acceptable price
//! for never persisting a real secret).

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Per-line jsonl record. `cmd` is the literal command, `ts` is
/// seconds-since-epoch — used to ensure stable ordering when we
/// merge across files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// UNIX timestamp (seconds since epoch) when the command ran.
    pub ts: u64,
    /// Literal command text as the user typed it.
    pub cmd: String,
}

/// Errors history I/O can return. Mostly file-system, plus a
/// distinct case for "no data dir on this platform" so the caller
/// can fail soft (silently skip persistence rather than toast an
/// error).
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// The current platform exposes no standard data directory; the
    /// caller should skip persistence silently.
    #[error("no platform data dir available")]
    NoDataDir,
    /// Underlying filesystem read/write failure.
    #[error("history I/O: {0}")]
    Io(#[from] std::io::Error),
    /// JSONL record could not be parsed.
    #[error("history parse: {0}")]
    Json(#[from] serde_json::Error),
}

/// Substrings that mark a line as too sensitive to persist. Match
/// is case-insensitive and substring-only — keeps the rule simple
/// and fail-safe (we'd rather drop a benign `man secret` line than
/// persist `export GITHUB_TOKEN=…`).
const SENSITIVE_KEYWORDS: &[&str] = &[
    "password",
    "passwd",
    "token",
    "secret",
    "apikey",
    "api_key",
    "api-key",
    "private_key",
    "private-key",
    "credential",
];

/// Returns `true` when `line` contains any of [`SENSITIVE_KEYWORDS`]
/// case-insensitively. Public so the frontend ring can apply the
/// same rule before invoking persistence (avoids one round-trip
/// per known-bad line).
pub fn is_sensitive(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    SENSITIVE_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

/// Sanitise the shell name for use as a filename component.
/// Strips any path separators / `.exe` and lowercases. Empty input
/// becomes `"shell"` so we still produce a valid file path.
fn shell_slug(shell: &str) -> String {
    let leaf = shell
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    let stripped = leaf.strip_suffix(".exe").unwrap_or(leaf.as_str());
    let cleaned: String = stripped
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if cleaned.is_empty() {
        "shell".to_string()
    } else {
        cleaned
    }
}

/// Resolve the on-disk path for `shell`'s history file. Creates the
/// containing directory if needed. Returns `Err(NoDataDir)` on
/// platforms where `directories` can't determine a sensible
/// location (rare — usually just headless CI).
pub fn path_for(shell: &str) -> Result<PathBuf, HistoryError> {
    let dirs = directories::ProjectDirs::from("", "", "pier-x").ok_or(HistoryError::NoDataDir)?;
    let dir = dirs.data_dir();
    fs::create_dir_all(dir)?;
    Ok(dir.join(format!("terminal-history-{}.jsonl", shell_slug(shell))))
}

/// Load the entries for `shell`, sorted most-recent first, and
/// de-duplicated against the most-recent occurrence. A truncated
/// or partially-corrupt jsonl line is skipped silently — the file
/// is append-only and a power loss could leave the last line half-
/// written; failing the entire load over one bad row would be a
/// poor user experience.
pub fn load(shell: &str) -> Result<Vec<String>, HistoryError> {
    let path = path_for(shell)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(&path)?;
    let reader = BufReader::new(file);
    // `(file_seq, entry)` tuples — `file_seq` is the line number,
    // used as a secondary sort key so two commands appended within
    // the same second still come out in append order. Without it
    // a power user typing back-to-back lines would see the older
    // one rank as more recent for autosuggest.
    let mut entries: Vec<(usize, HistoryEntry)> = Vec::new();
    for (seq, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            _ => continue,
        };
        if let Ok(entry) = serde_json::from_str::<HistoryEntry>(&line) {
            if !entry.cmd.is_empty() {
                entries.push((seq, entry));
            }
        }
    }
    // Sort newest-first by timestamp; ties broken by file-order
    // descending (later append = more recent).
    entries.sort_by(|a, b| b.1.ts.cmp(&a.1.ts).then_with(|| b.0.cmp(&a.0)));

    // Dedup: keep the first (most-recent) occurrence of each cmd
    // string. The autosuggest only looks at strings, so duplicate
    // entries waste ring slots without adding value.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::with_capacity(entries.len());
    for (_, e) in entries {
        if seen.insert(e.cmd.clone()) {
            out.push(e.cmd);
        }
    }
    Ok(out)
}

/// Append `cmd` to `shell`'s history file. Skipped silently when
/// `cmd` matches the sensitivity filter — the caller already has
/// the line in memory for the current session, and we don't want
/// to leak it to disk. Returns `Ok(())` on the skip path so callers
/// can fire-and-forget without special-casing the safe-drop.
pub fn append(shell: &str, cmd: &str) -> Result<(), HistoryError> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() || is_sensitive(trimmed) {
        return Ok(());
    }
    let path = path_for(shell)?;
    let entry = HistoryEntry {
        ts: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        cmd: trimmed.to_string(),
    };
    let line = serde_json::to_string(&entry)?;
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{}", line)?;
    Ok(())
}

/// Wipe `shell`'s history file. No-op when the file doesn't exist
/// so the Settings "Clear" button is idempotent.
pub fn clear(shell: &str) -> Result<(), HistoryError> {
    let path = path_for(shell)?;
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each test gets its own shell-scope so the on-disk files
    /// don't collide across parallel runs. The slug is just a
    /// timestamp + pid; not a real shell name, but `shell_slug`
    /// passes alphanumerics through unchanged.
    fn unique_slug(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("test_{}_{}_{}", prefix, std::process::id(), nanos)
    }

    fn cleanup(slug: &str) {
        if let Ok(p) = path_for(slug) {
            let _ = fs::remove_file(p);
        }
    }

    #[test]
    fn sensitivity_filter_catches_common_credential_words() {
        assert!(is_sensitive("export GITHUB_TOKEN=abc"));
        assert!(is_sensitive("psql -h db -U user --password=hunter2"));
        assert!(is_sensitive("aws configure set api_key sk-..."));
        assert!(is_sensitive("PRIVATE_KEY=..."));
        assert!(is_sensitive("export PG_CREDENTIAL=..."));
        // false-positive we accept: `man secret` would also match.
        assert!(is_sensitive("man secret"));
        // benign lines that should pass through:
        assert!(!is_sensitive("git status"));
        assert!(!is_sensitive("ls -la"));
        assert!(!is_sensitive("docker ps"));
    }

    #[test]
    fn shell_slug_handles_paths_and_extensions() {
        assert_eq!(shell_slug("/bin/bash"), "bash");
        assert_eq!(shell_slug("/usr/bin/zsh"), "zsh");
        assert_eq!(shell_slug("C:\\Program Files\\Git\\bin\\BASH.EXE"), "bash");
        assert_eq!(shell_slug(""), "shell");
        assert_eq!(shell_slug("./weird name"), "weirdname");
    }

    #[test]
    fn append_then_load_round_trips() {
        let slug = unique_slug("rt");
        cleanup(&slug);
        append(&slug, "ls -la").unwrap();
        append(&slug, "git status").unwrap();
        // Sensitive line should be silently dropped.
        append(&slug, "export GITHUB_TOKEN=hunter2").unwrap();
        let loaded = load(&slug).unwrap();
        // Most-recent-first ordering. Both benign lines are
        // present; the sensitive one is gone.
        assert_eq!(loaded.len(), 2, "got: {loaded:?}");
        assert_eq!(loaded[0], "git status");
        assert_eq!(loaded[1], "ls -la");
        cleanup(&slug);
    }

    #[test]
    fn load_dedups_against_most_recent_occurrence() {
        let slug = unique_slug("dedup");
        cleanup(&slug);
        // Three appends, second one a duplicate of the first. File-
        // order tie-break gives us deterministic ordering even when
        // all three land in the same second; no sleeps needed.
        append(&slug, "ls").unwrap();
        append(&slug, "ls").unwrap();
        append(&slug, "git status").unwrap();
        let loaded = load(&slug).unwrap();
        assert_eq!(loaded.len(), 2, "got: {loaded:?}");
        assert_eq!(loaded[0], "git status");
        assert_eq!(loaded[1], "ls");
        cleanup(&slug);
    }

    #[test]
    fn clear_removes_the_file() {
        let slug = unique_slug("clear");
        cleanup(&slug);
        append(&slug, "ls").unwrap();
        assert!(path_for(&slug).unwrap().exists());
        clear(&slug).unwrap();
        assert!(!path_for(&slug).unwrap().exists());
        // Idempotent — clearing a non-existent file is fine.
        clear(&slug).unwrap();
    }

    #[test]
    fn load_skips_corrupt_lines() {
        let slug = unique_slug("corrupt");
        cleanup(&slug);
        // Manually drop a malformed line in front, then a valid
        // one. Loader should silently ignore the malformed line.
        let path = path_for(&slug).unwrap();
        std::fs::write(&path, "this is not json\n{\"ts\":1,\"cmd\":\"ls\"}\n").unwrap();
        let loaded = load(&slug).unwrap();
        assert_eq!(loaded, vec!["ls".to_string()]);
        cleanup(&slug);
    }
}
