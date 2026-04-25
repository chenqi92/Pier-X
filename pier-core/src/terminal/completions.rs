//! Tab-completion candidates for smart-mode.
//!
//! Given the current input line and cursor position, produce a flat
//! `Vec<Completion>` the UI shows in a popover. Two main sources for
//! M4:
//!
//! * **Command names** (when the cursor sits in the first word of a
//!   compound). Returns shell builtins + every executable found on
//!   `$PATH` whose name starts with the typed prefix.
//! * **File paths** (any later word). Lists entries inside the
//!   shell's last-known cwd (or `dirname(prefix)` resolved against
//!   it) whose name starts with the typed basename.
//!
//! The two history-flavoured sources from the plan (M5 history-ring,
//! M6 man-known options) are intentionally absent — they need state
//! that doesn't exist yet. Tab will still pop a useful menu without
//! them; the later milestones layer on top.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// One row in the completion popover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    pub kind: CompletionKind,
    /// Full text the UI should produce when this row is selected.
    /// For directories, includes a trailing `/` so the next Tab
    /// drills in.
    pub value: String,
    /// What the UI shows in the row's main label. Usually the same
    /// as `value`; differs for paths where we show only the basename
    /// even though `value` carries the directory portion.
    pub display: String,
    /// Optional muted right-side hint — for `binary` kind, the
    /// resolved absolute path; for `file`/`directory`, blank.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompletionKind {
    Builtin,
    Binary,
    File,
    Directory,
}

/// Top-level entry point. `cursor` is a byte offset into `line`; if
/// it points past the end of `line` we treat it as end-of-line.
/// `cwd` is the shell's last-known directory (from OSC 7) — pass
/// `None` to fall back to the process cwd.
pub fn complete(line: &str, cursor: usize, cwd: Option<&Path>) -> Vec<Completion> {
    let cursor = cursor.min(line.len());
    let word_start = find_word_start(line, cursor);
    let prefix = &line[word_start..cursor];

    if is_command_position(line, word_start) {
        complete_command(prefix)
    } else {
        let resolved_cwd = resolve_cwd(cwd);
        complete_file(prefix, resolved_cwd.as_deref())
    }
}

/// Walk back from `cursor` while the char is part of a "word" — not
/// whitespace, not a shell operator. Returns the byte offset at
/// which the current word starts. The split mirrors the lexer's
/// notion of a word boundary so the resulting prefix matches what
/// the syntax overlay shows as one token.
fn find_word_start(line: &str, cursor: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = cursor;
    while i > 0 {
        let prev = bytes[i - 1];
        if matches!(
            prev,
            b' ' | b'\t' | b'|' | b'&' | b';' | b'>' | b'<' | b'\n'
        ) {
            break;
        }
        i -= 1;
    }
    i
}

/// True when `word_start` sits at the beginning of a compound — i.e.
/// the previous non-whitespace character is one of the operators
/// that begins a fresh command (or the line itself starts here).
fn is_command_position(line: &str, word_start: usize) -> bool {
    let mut i = word_start;
    while i > 0 {
        let prev = line.as_bytes()[i - 1];
        if prev == b' ' || prev == b'\t' {
            i -= 1;
            continue;
        }
        return matches!(prev, b'|' | b'&' | b';' | b'\n');
    }
    true
}

/// Builtins + PATH binaries whose name begins with `prefix`.
///
/// Listing is alphabetical with builtins first, capped at
/// `MAX_RESULTS` so a low-effort `Tab` on an empty prefix still
/// yields a usable list (and not, say, every binary on the system).
const MAX_RESULTS: usize = 64;

fn complete_command(prefix: &str) -> Vec<Completion> {
    use super::validate::SHELL_BUILTINS;

    let mut out: Vec<Completion> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for builtin in SHELL_BUILTINS {
        if !builtin.starts_with(prefix) {
            continue;
        }
        if !seen.insert((*builtin).to_string()) {
            continue;
        }
        out.push(Completion {
            kind: CompletionKind::Builtin,
            value: (*builtin).to_string(),
            display: (*builtin).to_string(),
            hint: Some("builtin".to_string()),
        });
        if out.len() >= MAX_RESULTS {
            break;
        }
    }

    if out.len() < MAX_RESULTS {
        if let Ok(path_var) = std::env::var("PATH") {
            let separator = if cfg!(windows) { ';' } else { ':' };
            for dir in path_var.split(separator) {
                if dir.is_empty() {
                    continue;
                }
                let entries = match std::fs::read_dir(dir) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                for entry in entries.flatten() {
                    let name = match entry.file_name().into_string() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    if !name.starts_with(prefix) {
                        continue;
                    }
                    let path = entry.path();
                    if !is_executable(&path) {
                        continue;
                    }
                    if !seen.insert(name.clone()) {
                        continue;
                    }
                    out.push(Completion {
                        kind: CompletionKind::Binary,
                        value: name.clone(),
                        display: name,
                        hint: Some(path.to_string_lossy().into_owned()),
                    });
                    if out.len() >= MAX_RESULTS {
                        break;
                    }
                }
                if out.len() >= MAX_RESULTS {
                    break;
                }
            }
        }
    }

    out.sort_by(|a, b| a.display.cmp(&b.display));
    out
}

/// File and directory entries under `cwd / dirname(prefix)` whose
/// basename begins with `basename(prefix)`. Returns directories
/// before files for stable ordering.
fn complete_file(prefix: &str, cwd: Option<&Path>) -> Vec<Completion> {
    let (dir_part, base_part) = split_path_prefix(prefix);

    let base_dir = if dir_part.is_empty() {
        cwd.map(|c| c.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
    } else if dir_part == "~" || dir_part.starts_with("~/") {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        if dir_part == "~" {
            home
        } else {
            home.join(&dir_part[2..])
        }
    } else if Path::new(&dir_part).is_absolute() {
        PathBuf::from(&dir_part)
    } else {
        cwd.map(|c| c.join(&dir_part))
            .unwrap_or_else(|| PathBuf::from(&dir_part))
    };

    let entries = match std::fs::read_dir(&base_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut dirs: Vec<Completion> = Vec::new();
    let mut files: Vec<Completion> = Vec::new();
    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        // Hide dotfiles unless the user typed at least one leading
        // dot — matches typical shell behaviour and keeps the popup
        // clean on home / project directories.
        if name.starts_with('.') && !base_part.starts_with('.') {
            continue;
        }
        if !name.starts_with(&base_part) {
            continue;
        }

        let is_dir = entry
            .file_type()
            .map(|t| t.is_dir())
            .unwrap_or(false);

        // Reattach the user's typed dir prefix so inserting `value`
        // into the line preserves what they had. Trailing `/` for
        // directories so the next Tab drills in.
        let mut value = String::new();
        if !dir_part.is_empty() {
            value.push_str(&dir_part);
            if !dir_part.ends_with('/') {
                value.push('/');
            }
        }
        value.push_str(&name);
        if is_dir {
            value.push('/');
        }

        let completion = Completion {
            kind: if is_dir {
                CompletionKind::Directory
            } else {
                CompletionKind::File
            },
            value,
            display: if is_dir { format!("{}/", name) } else { name },
            hint: None,
        };

        if is_dir {
            dirs.push(completion);
        } else {
            files.push(completion);
        }
    }
    dirs.sort_by(|a, b| a.display.cmp(&b.display));
    files.sort_by(|a, b| a.display.cmp(&b.display));
    dirs.extend(files);
    if dirs.len() > MAX_RESULTS {
        dirs.truncate(MAX_RESULTS);
    }
    dirs
}

/// Split `prefix` into a `(dir, base)` pair using the last `/`. For
/// `foo/bar/baz` → (`foo/bar`, `baz`). For `~/proj/x` → (`~/proj`,
/// `x`). For a bare basename without slashes → (``, `prefix`).
fn split_path_prefix(prefix: &str) -> (String, String) {
    if let Some(idx) = prefix.rfind('/') {
        (prefix[..idx + 1].trim_end_matches('/').to_string(), prefix[idx + 1..].to_string())
    } else {
        (String::new(), prefix.to_string())
    }
}

fn resolve_cwd(supplied: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = supplied {
        return Some(p.to_path_buf());
    }
    std::env::current_dir().ok()
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(m) => m.is_file() && (m.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn find_word_start_walks_back_to_whitespace() {
        assert_eq!(find_word_start("git st", 6), 4);
        assert_eq!(find_word_start("git", 3), 0);
        assert_eq!(find_word_start("a | b", 5), 4);
        assert_eq!(find_word_start("", 0), 0);
    }

    #[test]
    fn command_position_reflects_pipe_and_separator() {
        assert!(is_command_position("git st", 0));
        assert!(is_command_position("ls | gr", 5)); // after `| `
        assert!(is_command_position("a; b", 3)); // after `; `
        assert!(!is_command_position("git st", 4)); // arg word
    }

    #[test]
    fn empty_input_command_returns_builtins_at_least() {
        let results = complete_command("");
        // Caller controls MAX_RESULTS; we ship 64.
        assert!(!results.is_empty(), "empty prefix should still yield results");
        assert!(results.len() <= MAX_RESULTS);
        let names: HashSet<_> = results.iter().map(|c| c.display.as_str()).collect();
        // `cd` and `echo` are universal — any sane PATH should at least
        // expose them as builtins via the static table.
        assert!(names.contains("cd"));
        assert!(names.contains("echo"));
    }

    #[test]
    fn command_prefix_filters_correctly() {
        let results = complete_command("ec");
        // `echo` + maybe other ec-prefixed binaries
        assert!(results.iter().any(|c| c.display == "echo"));
        for r in &results {
            assert!(
                r.display.starts_with("ec"),
                "every result must match prefix, got {:?}",
                r.display,
            );
        }
    }

    #[test]
    fn file_completion_lists_cwd_entries() {
        // Build a temp dir with a known set of entries and probe it.
        let tmp = std::env::temp_dir().join(format!(
            "pier-x-completions-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("alpha.txt"), b"").unwrap();
        fs::write(tmp.join("beta.txt"), b"").unwrap();
        fs::create_dir(tmp.join("subdir")).unwrap();

        let results = complete_file("", Some(&tmp));
        let names: HashSet<_> = results.iter().map(|c| c.display.as_str()).collect();
        assert!(names.contains("alpha.txt"), "got {names:?}");
        assert!(names.contains("beta.txt"), "got {names:?}");
        assert!(names.contains("subdir/"), "got {names:?}");
        // Dotfiles hidden by default — none should appear with empty prefix.
        for n in &names {
            assert!(!n.starts_with('.'), "dotfile leaked: {n}");
        }
        // Directory should sort before files.
        let positions: Vec<_> = results
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if c.display == "subdir/" || c.display == "alpha.txt" {
                    Some((c.display.clone(), i))
                } else {
                    None
                }
            })
            .collect();
        let dir_idx = positions.iter().find(|(n, _)| n == "subdir/").unwrap().1;
        let file_idx = positions.iter().find(|(n, _)| n == "alpha.txt").unwrap().1;
        assert!(dir_idx < file_idx, "dir should come before file");

        // Cleanup
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn split_path_prefix_handles_dirs_and_basenames() {
        assert_eq!(split_path_prefix("foo"), (String::new(), "foo".to_string()));
        assert_eq!(
            split_path_prefix("foo/bar"),
            ("foo".to_string(), "bar".to_string()),
        );
        assert_eq!(
            split_path_prefix("foo/bar/"),
            ("foo/bar".to_string(), "".to_string()),
        );
        assert_eq!(
            split_path_prefix("~/proj/x"),
            ("~/proj".to_string(), "x".to_string()),
        );
    }

    #[test]
    fn complete_dispatches_to_command_then_file() {
        // First word on the line → command completion.
        let cmd_results = complete("ec", 2, None);
        assert!(cmd_results.iter().any(|c| c.display == "echo"));

        // After a space → file completion (cwd defaults to process cwd
        // which is the workspace dir during cargo test).
        let file_results = complete("ls Cargo.t", 10, None);
        // Cargo.toml exists at the crate root during cargo test runs;
        // the test is forgiving about other matches.
        assert!(
            file_results.iter().any(|c| c.display.starts_with("Cargo.")),
            "expected Cargo.* in results, got {:?}",
            file_results.iter().map(|c| &c.display).collect::<Vec<_>>(),
        );
    }
}
