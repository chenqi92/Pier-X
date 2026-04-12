//! Git client for the local repository panel.
//!
//! Unlike other services (Redis, MySQL, Docker) that connect to
//! a remote host over an SSH tunnel, the Git client operates on
//! a local `.git` directory. All operations are subprocess-based
//! (invoking the `git` CLI) and fully synchronous — no tokio
//! runtime needed.
//!
//! ## Design
//!
//! The heavy-lifting graph layout computation lives in
//! [`crate::git_graph`] and uses libgit2 for ref decoration.
//! This module focuses on the **working-tree** side: status,
//! staging, diff, commit, push, pull, and branch info.
//!
//! ## Thread safety
//!
//! `GitClient` is `Send + Sync` — the Qt bridge dispatches
//! every FFI call on a worker thread, same as MySQL/Redis.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────
// Error
// ──���──────────────────────────────────────────────────────

/// Errors surfaced by the Git client.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// The path is not inside a Git working tree.
    #[error("not a git repository: {0}")]
    NotARepo(String),

    /// A `git` subprocess failed.
    #[error("git: {0}")]
    Command(String),

    /// The supplied path was empty or invalid UTF-8.
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

// ─────────────────────────────────────────────────────────
// Data types
// ─────────────────────────────────────────────────────────

/// Status of a single file in the working tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileStatus {
    /// Modified (tracked, changed in worktree or index).
    Modified,
    /// Newly added to the index (not yet committed).
    Added,
    /// Deleted.
    Deleted,
    /// Renamed.
    Renamed,
    /// Untracked (not in index, not ignored).
    Untracked,
    /// Unmerged (conflict).
    Conflicted,
    /// Copied.
    Copied,
}

impl FileStatus {
    /// Single-char code matching `git status --porcelain`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "?",
            Self::Conflicted => "U",
            Self::Copied => "C",
        }
    }

    fn from_porcelain(c: char) -> Self {
        match c {
            'M' => Self::Modified,
            'A' => Self::Added,
            'D' => Self::Deleted,
            'R' => Self::Renamed,
            'C' => Self::Copied,
            'U' => Self::Conflicted,
            '?' => Self::Untracked,
            _ => Self::Modified,
        }
    }
}

/// A file change entry from `git status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitFileChange {
    /// Full path relative to repo root.
    pub path: String,
    /// File status code.
    pub status: FileStatus,
    /// Whether this entry is staged (index) vs unstaged (worktree).
    pub staged: bool,
}

/// Current branch information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    /// Current branch name (or "HEAD" if detached).
    pub name: String,
    /// Remote tracking branch (e.g. "origin/main"), empty if none.
    pub tracking: String,
    /// Commits ahead of tracking branch.
    pub ahead: i32,
    /// Commits behind tracking branch.
    pub behind: i32,
}

// ─────────────────────────────────────────────────────────
// Client
// ─────────────────────────────────────────────────────────

/// Git client bound to a specific repository path.
pub struct GitClient {
    /// Absolute path to the repository root (the directory
    /// containing `.git/`).
    repo_path: PathBuf,
}

impl GitClient {
    // ── Construction ──────────────────────────────────────

    /// Open a Git client for the repository at or above `path`.
    ///
    /// Runs `git rev-parse --show-toplevel` to resolve the repo
    /// root. Returns an error if the path is not inside a Git
    /// working tree.
    pub fn open(path: &str) -> Result<Self, GitError> {
        if path.is_empty() {
            return Err(GitError::InvalidPath("empty path".into()));
        }
        let p = Path::new(path);
        if !p.exists() {
            return Err(GitError::InvalidPath(format!("path does not exist: {}", path)));
        }

        let output = Command::new("git")
            .current_dir(p)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(|e| GitError::Command(format!("failed to run git: {}", e)))?;

        if !output.status.success() {
            return Err(GitError::NotARepo(path.to_string()));
        }

        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Self {
            repo_path: PathBuf::from(root),
        })
    }

    /// The resolved repository root path.
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    // ── Status ───────────────────────────────────────────

    /// Get the working tree status.
    ///
    /// Parses `git status --porcelain=v1` and returns two lists:
    /// staged (index) changes and unstaged (worktree) changes.
    pub fn status(&self) -> Result<Vec<GitFileChange>, GitError> {
        let output = self
            .git(&["status", "--porcelain=v1", "-uall"])
            .map_err(|e| GitError::Command(e.to_string()))?;

        let mut changes = Vec::new();

        for line in output.lines() {
            if line.len() < 3 {
                continue;
            }
            let bytes = line.as_bytes();
            let index_status = bytes[0] as char;
            let worktree_status = bytes[1] as char;
            let path = &line[3..];

            // Handle renames: "R  old -> new"
            let file_path = if path.contains(" -> ") {
                path.split(" -> ").last().unwrap_or(path).to_string()
            } else {
                path.to_string()
            };

            // Index (staged) change
            if index_status != ' ' && index_status != '?' {
                changes.push(GitFileChange {
                    path: file_path.clone(),
                    status: FileStatus::from_porcelain(index_status),
                    staged: true,
                });
            }

            // Worktree (unstaged) change
            if worktree_status != ' ' {
                let status = if index_status == '?' {
                    FileStatus::Untracked
                } else {
                    FileStatus::from_porcelain(worktree_status)
                };
                changes.push(GitFileChange {
                    path: file_path,
                    status,
                    staged: false,
                });
            }
        }

        Ok(changes)
    }

    // ── Diff ─────────────────────────────────────────────

    /// Get the unified diff for a file.
    ///
    /// If `staged` is true, shows the diff between HEAD and the
    /// index (`--cached`). Otherwise shows the diff between the
    /// index and the working tree.
    ///
    /// If `path` is empty, returns the full diff for all files.
    pub fn diff(&self, path: &str, staged: bool) -> Result<String, GitError> {
        let mut args = vec!["diff", "--no-color"];
        if staged {
            args.push("--cached");
        }
        if !path.is_empty() {
            args.push("--");
            args.push(path);
        }
        self.git(&args).map_err(|e| GitError::Command(e.to_string()))
    }

    /// Get the diff for an untracked file (shows full content).
    pub fn diff_untracked(&self, path: &str) -> Result<String, GitError> {
        self.git(&["diff", "--no-color", "--no-index", "/dev/null", path])
            .or_else(|_| {
                // --no-index returns exit code 1 when files differ,
                // which is normal — try reading the file directly
                let full_path = self.repo_path.join(path);
                std::fs::read_to_string(&full_path)
                    .map(|content| {
                        // Format as a pseudo-diff
                        let mut out = format!("new file: {}\n\n", path);
                        for (i, line) in content.lines().enumerate() {
                            out.push_str(&format!("+{}\n", line));
                            if i > 2000 {
                                out.push_str("\n... (truncated)\n");
                                break;
                            }
                        }
                        out
                    })
                    .map_err(|e| GitError::Command(format!("cannot read {}: {}", path, e)))
            })
    }

    // ── Branch info ──────────────────────────────────────

    /// Get the current branch name and tracking information.
    pub fn branch_info(&self) -> Result<BranchInfo, GitError> {
        // Current branch name
        let name = self
            .git(&["rev-parse", "--abbrev-ref", "HEAD"])
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "HEAD".to_string());

        // Tracking branch
        let tracking = self
            .git(&[
                "rev-parse",
                "--abbrev-ref",
                &format!("{}@{{upstream}}", name),
            ])
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        // Ahead/behind
        let (ahead, behind) = if !tracking.is_empty() {
            self.git(&[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{}...{}", name, tracking),
            ])
            .map(|s| {
                let parts: Vec<&str> = s.trim().split('\t').collect();
                let a = parts.first().and_then(|v| v.parse().ok()).unwrap_or(0);
                let b = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                (a, b)
            })
            .unwrap_or((0, 0))
        } else {
            (0, 0)
        };

        Ok(BranchInfo {
            name,
            tracking,
            ahead,
            behind,
        })
    }

    // ── Staging ──────────────────────────────────────────

    /// Stage specific files.
    pub fn stage(&self, paths: &[String]) -> Result<(), GitError> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args: Vec<&str> = vec!["add", "--"];
        for p in paths {
            args.push(p.as_str());
        }
        self.git(&args)?;
        Ok(())
    }

    /// Stage all changes.
    pub fn stage_all(&self) -> Result<(), GitError> {
        self.git(&["add", "-A"])?;
        Ok(())
    }

    /// Unstage specific files.
    pub fn unstage(&self, paths: &[String]) -> Result<(), GitError> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args: Vec<&str> = vec!["reset", "HEAD", "--"];
        for p in paths {
            args.push(p.as_str());
        }
        self.git(&args)?;
        Ok(())
    }

    /// Unstage all files.
    pub fn unstage_all(&self) -> Result<(), GitError> {
        self.git(&["reset", "HEAD"])?;
        Ok(())
    }

    /// Discard working tree changes for specific files.
    pub fn discard(&self, paths: &[String]) -> Result<(), GitError> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args: Vec<&str> = vec!["checkout", "--"];
        for p in paths {
            args.push(p.as_str());
        }
        self.git(&args)?;
        Ok(())
    }

    // ── Commit ───────────────────────────────────────────

    /// Create a commit with the given message.
    pub fn commit(&self, message: &str) -> Result<String, GitError> {
        if message.is_empty() {
            return Err(GitError::Command("commit message cannot be empty".into()));
        }
        self.git(&["commit", "-m", message])
    }

    // ── Remote ───────────────────────────────────────────

    /// Push the current branch to the remote.
    pub fn push(&self) -> Result<String, GitError> {
        self.git(&["push"])
    }

    /// Pull from the remote.
    pub fn pull(&self) -> Result<String, GitError> {
        self.git(&["pull"])
    }

    // ── Log (simple, for Phase 1) ────────────────────────

    /// Get the last N commits as a simple list.
    pub fn log_simple(&self, limit: usize) -> Result<String, GitError> {
        self.git(&[
            "log",
            &format!("-{}", limit),
            "--format=%H\x1f%h\x1f%s\x1f%an\x1f%ct\x1f%cr",
            "--topo-order",
        ])
    }

    // ── Internal ─────────────────────────────────────────

    /// Run a git command and return stdout as a String.
    fn git(&self, args: &[&str]) -> Result<String, GitError> {
        let output = Command::new("git")
            .current_dir(&self.repo_path)
            .args(args)
            .output()
            .map_err(|e| GitError::Command(format!("failed to run git {}: {}", args.join(" "), e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Some commands (like diff --no-index) use exit code 1 for "files differ"
            // which is not actually an error
            if output.status.code() == Some(1) && stderr.is_empty() {
                return Ok(String::from_utf8_lossy(&output.stdout).to_string());
            }
            return Err(GitError::Command(format!(
                "git {} failed: {}",
                args.join(" "),
                stderr.trim()
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

// Send + Sync — no interior mutability, no pointers.
unsafe impl Send for GitClient {}
unsafe impl Sync for GitClient {}
