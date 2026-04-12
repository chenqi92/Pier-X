//! File search — pattern matching on file names and content.
//!
//! Uses the system `grep -r` or `find` for file name search,
//! respecting .gitignore via `git ls-files` when inside a git repo.
//! Content search uses `grep -rn` for simplicity.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

/// A single search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    /// For content search: line number (0 = file-name-only match).
    pub line: u32,
    /// For content search: the matching line text.
    pub text: String,
}

/// Search error.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("search: {0}")]
    Command(String),
}

/// Search for files by name pattern in a directory.
///
/// Uses `git ls-files` if inside a git repo (respects .gitignore),
/// falls back to `find` otherwise. Case-insensitive substring match.
pub fn search_files(
    root: &str,
    pattern: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    if pattern.is_empty() || root.is_empty() {
        return Ok(Vec::new());
    }
    let max = if max_results == 0 { 200 } else { max_results };

    // Try git ls-files first (respects .gitignore)
    let output = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .output();

    let file_list = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => {
            // Fallback: find
            let out = Command::new("find")
                .current_dir(root)
                .args([".", "-maxdepth", "10", "-type", "f"])
                .output()
                .map_err(|e| SearchError::Command(e.to_string()))?;
            String::from_utf8_lossy(&out.stdout).to_string()
        }
    };

    let pat_lower = pattern.to_lowercase();
    let mut results = Vec::new();
    for line in file_list.lines() {
        let name = line.strip_prefix("./").unwrap_or(line);
        if name.is_empty() {
            continue;
        }
        if name.to_lowercase().contains(&pat_lower) {
            let file_name = Path::new(name)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(name)
                .to_string();
            results.push(SearchResult {
                path: name.to_string(),
                name: file_name,
                is_dir: false,
                line: 0,
                text: String::new(),
            });
            if results.len() >= max {
                break;
            }
        }
    }
    Ok(results)
}

/// Search file contents using grep.
pub fn search_content(
    root: &str,
    pattern: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    if pattern.is_empty() || root.is_empty() {
        return Ok(Vec::new());
    }
    let max = if max_results == 0 { 200 } else { max_results };

    // Try git grep first (respects .gitignore, faster)
    let output = Command::new("git")
        .current_dir(root)
        .args(["grep", "-n", "-i", "--max-count", "3", pattern])
        .output();

    let grep_output = match output {
        Ok(o) if o.status.success() || o.status.code() == Some(1) => {
            String::from_utf8_lossy(&o.stdout).to_string()
        }
        _ => {
            // Fallback: grep -rn
            let out = Command::new("grep")
                .current_dir(root)
                .args(["-rn", "-i", "--max-count=3", pattern, "."])
                .output()
                .map_err(|e| SearchError::Command(e.to_string()))?;
            String::from_utf8_lossy(&out.stdout).to_string()
        }
    };

    let mut results = Vec::new();
    for line in grep_output.lines() {
        if line.is_empty() {
            continue;
        }
        // Format: path:line:content
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() >= 3 {
            let path = parts[0].strip_prefix("./").unwrap_or(parts[0]);
            let file_name = Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path)
                .to_string();
            results.push(SearchResult {
                path: path.to_string(),
                name: file_name,
                is_dir: false,
                line: parts[1].parse().unwrap_or(0),
                text: parts[2].to_string(),
            });
            if results.len() >= max {
                break;
            }
        }
    }
    Ok(results)
}
