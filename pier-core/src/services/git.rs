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
//! `GitClient` is `Send + Sync`, so app runtimes can dispatch
//! operations on background workers the same way they do for
//! MySQL / Redis / Docker.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::process_util::configure_background_command;
use crate::ssh::SshSession;

// ─────────────────────────────────────────────────────────
// Transport
// ─────────────────────────────────────────────────────────

/// How a `GitClient` reaches its repository. Local transports
/// spawn `git` as a subprocess in a cwd; remote transports bounce
/// every command through an SSH session against a server-side
/// working tree. Parsers and methods on [`GitClient`] consume the
/// stdout identically either way.
pub trait GitTransport: Send + Sync {
    /// Run `git <args>` against the transport's repository and
    /// return the captured stdout as UTF-8 (lossy).
    fn run(&self, args: &[&str]) -> Result<String, GitError>;

    /// Best-effort read of a file inside the repository. Optional
    /// because only the `diff_untracked` path needs it today —
    /// transports that can't provide raw file contents return
    /// `NotSupported`. Local → fs read; SSH → `cat` via exec.
    fn read_repo_file(&self, _relative: &str) -> Result<String, GitError> {
        Err(GitError::Command(
            "raw file read not supported by this git transport".into(),
        ))
    }
}

/// Subprocess-backed transport — runs `git` as a child process in
/// `repo_path`. This is the original implementation pulled out of
/// the old monolithic `GitClient::git`.
pub struct LocalGitTransport {
    repo_path: PathBuf,
}

impl LocalGitTransport {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }
}

impl GitTransport for LocalGitTransport {
    fn run(&self, args: &[&str]) -> Result<String, GitError> {
        let mut command = Command::new("git");
        command.current_dir(&self.repo_path);
        command.args(args);
        configure_background_command(&mut command);
        let output = command.output().map_err(|e| {
            GitError::Command(format!("failed to run git {}: {}", args.join(" "), e))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Some git commands (e.g. `diff --no-index`) use exit
            // code 1 for "files differ", which isn't an error for
            // our callers.
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

    fn read_repo_file(&self, relative: &str) -> Result<String, GitError> {
        std::fs::read_to_string(self.repo_path.join(relative))
            .map_err(|e| GitError::Command(format!("cannot read {relative}: {e}")))
    }
}

/// SSH-backed transport — turns every `git <args>` call into
/// `git -C '<repo>' <args>` via [`SshSession::exec_command_blocking`].
/// All arguments are POSIX-single-quote escaped so paths and
/// commit messages with spaces / quotes pass through intact.
///
/// Targets POSIX remotes (Linux, macOS, BSD, Windows with git-bash
/// exposed through OpenSSH). Windows-native OpenSSH with cmd.exe
/// as default shell would need `-Command "git -C …"` instead —
/// out of scope for this transport; use the bash-powered integration
/// path if that's the target.
pub struct SshGitTransport {
    session: SshSession,
    repo_path: String,
}

impl SshGitTransport {
    pub fn new(session: SshSession, repo_path: String) -> Self {
        Self { session, repo_path }
    }

    fn build_command(&self, args: &[&str]) -> String {
        let mut cmd = String::from("git -C ");
        cmd.push_str(&posix_single_quote(&self.repo_path));
        for a in args {
            cmd.push(' ');
            cmd.push_str(&posix_single_quote(a));
        }
        cmd
    }
}

impl GitTransport for SshGitTransport {
    fn run(&self, args: &[&str]) -> Result<String, GitError> {
        let cmd = self.build_command(args);
        let (code, stdout) = self
            .session
            .exec_command_blocking(&cmd)
            .map_err(|e| GitError::Command(format!("ssh exec: {e}")))?;
        if code == 0 {
            return Ok(stdout);
        }
        // Exit 1 with empty stdout commonly means "not a repo" or
        // "files differ" depending on command — preserve the
        // existing local behaviour of returning stdout in that
        // edge case so `diff --no-index` keeps working.
        if code == 1 && stdout.trim().is_empty() {
            return Ok(stdout);
        }
        Err(GitError::Command(format!(
            "git {} failed on remote (exit {code})",
            args.join(" ")
        )))
    }

    fn read_repo_file(&self, relative: &str) -> Result<String, GitError> {
        // Cheap and universal: `cat <repo>/<rel>` over exec. We
        // rebuild the path on the remote instead of passing the
        // client-side `repo_path.join` so Windows-on-POSIX path
        // quirks don't sneak in.
        let mut cmd = String::from("cat ");
        cmd.push_str(&posix_single_quote(&format!(
            "{}/{}",
            self.repo_path, relative
        )));
        let (code, stdout) = self
            .session
            .exec_command_blocking(&cmd)
            .map_err(|e| GitError::Command(format!("ssh exec: {e}")))?;
        if code == 0 {
            Ok(stdout)
        } else {
            Err(GitError::Command(format!("cat {relative} exit {code}")))
        }
    }
}

/// POSIX-safe single-quote quoting for shell arguments.
/// `O'Brien` → `'O'\''Brien'`. Every possible byte survives.
fn posix_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

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

/// A single commit entry from `git log`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    /// Full 40-char hash.
    pub hash: String,
    /// Short hash (7-8 chars).
    pub short_hash: String,
    /// Commit message (first line).
    pub message: String,
    /// Author name.
    pub author: String,
    /// Unix timestamp.
    pub timestamp: i64,
    /// Relative date string (e.g. "2 hours ago").
    pub relative_date: String,
    /// Ref decorations (branch names, tags).
    pub refs: String,
}

/// A stash entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StashEntry {
    /// Stash index (e.g. "stash@{0}").
    pub index: String,
    /// Stash message.
    pub message: String,
    /// Relative date.
    pub relative_date: String,
}

/// A single blame line.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameLine {
    pub line_number: u32,
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub timestamp: i64,
    pub content: String,
}

/// Tag information.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagInfo {
    pub name: String,
    pub hash: String,
    pub timestamp: i64,
    pub message: String,
}

/// Remote information.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteInfo {
    pub name: String,
    pub fetch_url: String,
    pub push_url: String,
}

/// Git config entry.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    pub scope: String,
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

/// Git client bound to a specific repository path + transport.
/// The transport is what makes a single client work identically
/// over a local subprocess or a remote SSH session; see the
/// [`GitTransport`] trait.
pub struct GitClient {
    /// Path to the repository root, as known to the transport.
    /// For `LocalGitTransport` this is an absolute filesystem
    /// path; for `SshGitTransport` it's the remote string that
    /// was canonicalised by `rev-parse --show-toplevel`.
    repo_path: PathBuf,
    transport: Box<dyn GitTransport>,
}

impl GitClient {
    // ── Construction ──────────────────────────────────────

    /// Open a Git client against a local repository rooted at or
    /// above `path`. Resolves the real root via
    /// `rev-parse --show-toplevel`; returns `NotARepo` if `path`
    /// doesn't live inside a Git working tree.
    pub fn open(path: &str) -> Result<Self, GitError> {
        if path.is_empty() {
            return Err(GitError::InvalidPath("empty path".into()));
        }
        let p = Path::new(path);
        if !p.exists() {
            return Err(GitError::InvalidPath(format!(
                "path does not exist: {path}"
            )));
        }

        // Temporary local transport pinned to `path` so we can ask
        // git to tell us the real repo root. The permanent
        // transport gets rebound to that canonical path below so
        // subsequent `git -C` subprocess calls land correctly.
        let probe = LocalGitTransport::new(p.to_path_buf());
        let root = probe
            .run(&["rev-parse", "--show-toplevel"])
            .map_err(|_| GitError::NotARepo(path.to_string()))?
            .trim()
            .to_string();

        let repo_path = PathBuf::from(&root);
        Ok(Self {
            transport: Box::new(LocalGitTransport::new(repo_path.clone())),
            repo_path,
        })
    }

    /// Open a Git client against a remote repository that lives at
    /// `cwd` on `session`. Runs `git -C <cwd> rev-parse --show-toplevel`
    /// over exec; returns `NotARepo` when `cwd` isn't inside a
    /// working tree on that host.
    ///
    /// The transport clones `session` cheaply (russh handle is
    /// reference-counted), so the caller can keep using the same
    /// session for its SFTP panel / shell channel in parallel.
    pub fn open_remote(session: SshSession, cwd: &str) -> Result<Self, GitError> {
        if cwd.is_empty() {
            return Err(GitError::InvalidPath("empty remote path".into()));
        }
        let probe = SshGitTransport::new(session.clone(), cwd.to_string());
        let root = probe
            .run(&["rev-parse", "--show-toplevel"])
            .map_err(|_| GitError::NotARepo(cwd.to_string()))?
            .trim()
            .to_string();
        Ok(Self {
            repo_path: PathBuf::from(&root),
            transport: Box::new(SshGitTransport::new(session, root)),
        })
    }

    /// The resolved repository root path — filesystem path for
    /// local transports, remote POSIX path for SSH transports.
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
        self.git(&args)
            .map_err(|e| GitError::Command(e.to_string()))
    }

    /// Get the diff for an untracked file (shows full content).
    pub fn diff_untracked(&self, path: &str) -> Result<String, GitError> {
        self.git(&["diff", "--no-color", "--no-index", "/dev/null", path])
            .or_else(|_| {
                // --no-index returns exit code 1 when files differ,
                // which is normal — try reading the file directly
                // through the transport (local fs read or `cat`
                // over SSH).
                self.transport.read_repo_file(path).map(|content| {
                    let mut out = format!("new file: {path}\n\n");
                    for (i, line) in content.lines().enumerate() {
                        out.push_str(&format!("+{line}\n"));
                        if i > 2000 {
                            out.push_str("\n... (truncated)\n");
                            break;
                        }
                    }
                    out
                })
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
            .git(&["rev-parse", "--abbrev-ref", &format!("{name}@{{upstream}}")])
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        // Ahead/behind
        let (ahead, behind) = if !tracking.is_empty() {
            self.git(&[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{name}...{tracking}"),
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

    // ── Log ──────────────────────────────────────────────

    /// Get the last N commits as a JSON-serializable list.
    pub fn log(&self, limit: usize) -> Result<Vec<CommitInfo>, GitError> {
        let sep = "\x1f";
        let output = self.git(&[
            "log",
            &format!("-{limit}"),
            &format!("--format=%H{sep}%h{sep}%s{sep}%an{sep}%ct{sep}%cr{sep}%D"),
            "--topo-order",
        ])?;

        let mut commits = Vec::new();
        for line in output.lines() {
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.splitn(7, sep).collect();
            if parts.len() < 6 {
                continue;
            }
            commits.push(CommitInfo {
                hash: parts[0].to_string(),
                short_hash: parts[1].to_string(),
                message: parts[2].to_string(),
                author: parts[3].to_string(),
                timestamp: parts[4].parse().unwrap_or(0),
                relative_date: parts[5].to_string(),
                refs: parts.get(6).unwrap_or(&"").to_string(),
            });
        }
        Ok(commits)
    }

    // ── Stash ────────────────────────────────────────────

    /// List stash entries.
    pub fn stash_list(&self) -> Result<Vec<StashEntry>, GitError> {
        let output = self.git(&["stash", "list", "--format=%gd\x1f%gs\x1f%cr"])?;
        let mut stashes = Vec::new();
        for line in output.lines() {
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.splitn(3, '\x1f').collect();
            if parts.len() < 3 {
                continue;
            }
            stashes.push(StashEntry {
                index: parts[0].to_string(),
                message: parts[1].to_string(),
                relative_date: parts[2].to_string(),
            });
        }
        Ok(stashes)
    }

    /// Stash current changes.
    pub fn stash_push(&self, message: &str) -> Result<String, GitError> {
        if message.is_empty() {
            self.git(&["stash", "push"])
        } else {
            self.git(&["stash", "push", "-m", message])
        }
    }

    /// Apply a stash (keep it in the stash list).
    pub fn stash_apply(&self, index: &str) -> Result<String, GitError> {
        self.git(&["stash", "apply", index])
    }

    /// Pop a stash (apply + drop).
    pub fn stash_pop(&self, index: &str) -> Result<String, GitError> {
        self.git(&["stash", "pop", index])
    }

    /// Drop a stash.
    pub fn stash_drop(&self, index: &str) -> Result<String, GitError> {
        self.git(&["stash", "drop", index])
    }

    // ── Branch operations ────────────────────────────────

    /// List local branches.
    pub fn branch_list(&self) -> Result<Vec<String>, GitError> {
        let output = self.git(&["branch", "--format=%(refname:short)"])?;
        Ok(output
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect())
    }

    /// Switch to a branch.
    pub fn checkout_branch(&self, name: &str) -> Result<String, GitError> {
        self.git(&["checkout", name])
    }

    // ── Blame ────────────────────────────────────────────

    /// Get blame annotation for a file.
    pub fn blame(&self, path: &str) -> Result<Vec<BlameLine>, GitError> {
        let output = self.git(&["blame", "--porcelain", path])?;
        let mut lines = Vec::new();
        let mut current_hash = String::new();
        let mut current_author = String::new();
        let mut current_time = 0i64;
        let mut line_no = 0u32;

        for raw in output.lines() {
            if let Some(content) = raw.strip_prefix('\t') {
                // Content line
                lines.push(BlameLine {
                    line_number: line_no,
                    hash: current_hash.clone(),
                    short_hash: if current_hash.len() >= 8 {
                        current_hash[..8].to_string()
                    } else {
                        current_hash.clone()
                    },
                    author: current_author.clone(),
                    timestamp: current_time,
                    content: content.to_string(),
                });
            } else if raw.len() >= 40 && raw.as_bytes()[40] == b' ' {
                // Hash header: <40-char hash> <orig-line> <final-line> [count]
                current_hash = raw[..40].to_string();
                let parts: Vec<&str> = raw[41..].split(' ').collect();
                line_no = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            } else if let Some(author) = raw.strip_prefix("author ") {
                current_author = author.to_string();
            } else if let Some(time) = raw.strip_prefix("author-time ") {
                current_time = time.parse().unwrap_or(0);
            }
        }
        Ok(lines)
    }

    // ── Tags ─────────────────────────────────────────────

    /// List all tags.
    pub fn tag_list(&self) -> Result<Vec<TagInfo>, GitError> {
        let output = self.git(&[
            "tag",
            "-l",
            "--format=%(refname:short)\x1f%(objectname:short)\x1f%(creatordate:unix)\x1f%(subject)",
        ])?;
        let mut tags = Vec::new();
        for line in output.lines() {
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.splitn(4, '\x1f').collect();
            if parts.len() >= 2 {
                tags.push(TagInfo {
                    name: parts[0].to_string(),
                    hash: parts[1].to_string(),
                    timestamp: parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
                    message: parts.get(3).unwrap_or(&"").to_string(),
                });
            }
        }
        Ok(tags)
    }

    /// Create a tag.
    pub fn tag_create(&self, name: &str, message: &str) -> Result<String, GitError> {
        if message.is_empty() {
            self.git(&["tag", name])
        } else {
            self.git(&["tag", "-a", name, "-m", message])
        }
    }

    /// Delete a tag.
    pub fn tag_delete(&self, name: &str) -> Result<String, GitError> {
        self.git(&["tag", "-d", name])
    }

    // ── Remotes ──────────────────────────────────────────

    /// List remotes with URLs.
    pub fn remote_list(&self) -> Result<Vec<RemoteInfo>, GitError> {
        let output = self.git(&["remote", "-v"])?;
        let mut map = std::collections::BTreeMap::new();
        for line in output.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let entry = map
                    .entry(parts[0].to_string())
                    .or_insert_with(|| RemoteInfo {
                        name: parts[0].to_string(),
                        fetch_url: String::new(),
                        push_url: String::new(),
                    });
                if line.contains("(fetch)") {
                    entry.fetch_url = parts[1].to_string();
                } else if line.contains("(push)") {
                    entry.push_url = parts[1].to_string();
                }
            }
        }
        Ok(map.into_values().collect())
    }

    /// Add a remote.
    pub fn remote_add(&self, name: &str, url: &str) -> Result<String, GitError> {
        self.git(&["remote", "add", name, url])
    }

    /// Remove a remote.
    pub fn remote_remove(&self, name: &str) -> Result<String, GitError> {
        self.git(&["remote", "remove", name])
    }

    // ── Config ───────────────────────────────────────────

    /// List git config (local + global merged).
    pub fn config_list(&self) -> Result<Vec<ConfigEntry>, GitError> {
        let output = self.git(&["config", "--list", "--show-origin"])?;
        let mut entries = Vec::new();
        for line in output.lines() {
            // Format: file:<path>\t<key>=<value>
            if let Some(tab_pos) = line.find('\t') {
                let origin = &line[..tab_pos];
                let kv = &line[tab_pos + 1..];
                if let Some(eq_pos) = kv.find('=') {
                    let scope = if origin.contains(".gitconfig") || origin.contains("global") {
                        "global"
                    } else {
                        "local"
                    };
                    entries.push(ConfigEntry {
                        key: kv[..eq_pos].to_string(),
                        value: kv[eq_pos + 1..].to_string(),
                        scope: scope.to_string(),
                    });
                }
            }
        }
        Ok(entries)
    }

    /// Set a config value.
    pub fn config_set(&self, key: &str, value: &str, global: bool) -> Result<String, GitError> {
        if global {
            self.git(&["config", "--global", key, value])
        } else {
            self.git(&["config", key, value])
        }
    }

    /// Unset a config value.
    pub fn config_unset(&self, key: &str, global: bool) -> Result<String, GitError> {
        if global {
            self.git(&["config", "--global", "--unset", key])
        } else {
            self.git(&["config", "--unset", key])
        }
    }

    // ── Internal ─────────────────────────────────────────

    /// Run a git command and return stdout. Delegates to the
    /// active transport so both local and SSH repos take the same
    /// code path through every caller above.
    fn git(&self, args: &[&str]) -> Result<String, GitError> {
        self.transport.run(args)
    }
}

// Send + Sync — no interior mutability, no pointers.
unsafe impl Send for GitClient {}
unsafe impl Sync for GitClient {}
