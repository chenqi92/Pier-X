//! Local-repo Git session state.
//!
//! One `GitState` entity lives on `PierApp` (there is only ever one
//! working directory). It mirrors the role `DbSessionState` plays for
//! MySQL / PostgreSQL tabs and `SshSessionState` plays for SFTP:
//!
//! 1. The view calls `PierApp::schedule_git_*` after a click.
//! 2. PierApp dispatches the blocking `pier-core::services::git`
//!    call on `cx.background_executor()` so the UI thread never
//!    freezes on `git status` / `git log` / `git diff`.
//! 3. The task hands a `*Result` back to the state entity, which
//!    updates the cached snapshot + clears the pending-action flag.
//!
//! The view stays pure — it renders from the cached snapshot only.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::SharedString;
use pier_core::services::git::{
    BranchInfo, CommitInfo, GitClient, GitError, GitFileChange, StashEntry,
};
use pier_core::ssh::SshSession;

/// Where a [`GitState`] points. A local path means "run the `git`
/// subprocess in this directory"; a remote target means "exec
/// `git -C <cwd> …` across this SSH session". The client
/// transport picks the right one at refresh time.
#[derive(Clone)]
pub enum GitTarget {
    Local(PathBuf),
    Remote { session: SshSession, cwd: String },
}

impl GitTarget {
    /// Human-readable label for debug logs and the status pill.
    pub fn label(&self) -> String {
        match self {
            Self::Local(p) => p.to_string_lossy().into_owned(),
            Self::Remote { cwd, .. } => format!("remote:{cwd}"),
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote { .. })
    }

    /// True when the two targets are the "same repo" — same
    /// variant and same path / cwd. `SshSession` identity is not
    /// compared: two clones of the same session are cheap and
    /// indistinguishable anyway, and swapping between unrelated
    /// sessions always comes with a cwd change in practice
    /// (different `$HOME`, OSC 7 reports a new path).
    pub fn same_repo(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Local(a), Self::Local(b)) => a == b,
            (Self::Remote { cwd: a, .. }, Self::Remote { cwd: b, .. }) => a == b,
            _ => false,
        }
    }
}

/// Top-level repo status, drives the status pill + gates which
/// controls render.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GitStatus {
    /// No probe has finished yet.
    #[default]
    Idle,
    /// Initial / explicit refresh is in flight.
    Loading,
    /// The cached snapshot is current.
    Ready,
    /// The current working directory is not inside a Git repo.
    /// The probe result is cached so the view can render the
    /// "open a repo elsewhere" placeholder without re-probing.
    NotARepo,
    /// Last probe failed with a real error (not "not-a-repo").
    Failed,
}

/// Pending mutation against the repo — drives the spinner label
/// and blocks additional actions until the current one settles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitPendingAction {
    Refresh,
    Stage { path: String },
    Unstage { path: String },
    Discard { path: String },
    StageAll,
    UnstageAll,
    Commit { message: String },
    CheckoutBranch { name: String },
    StashPush { message: String },
    StashApply { index: String },
    StashPop { index: String },
    StashDrop { index: String },
    Push,
    Pull,
}

impl GitPendingAction {
    pub fn label(&self) -> SharedString {
        match self {
            Self::Refresh => "refresh".into(),
            Self::Stage { .. } => "stage".into(),
            Self::Unstage { .. } => "unstage".into(),
            Self::Discard { .. } => "discard".into(),
            Self::StageAll => "stage-all".into(),
            Self::UnstageAll => "unstage-all".into(),
            Self::Commit { .. } => "commit".into(),
            Self::CheckoutBranch { .. } => "checkout".into(),
            Self::StashPush { .. } => "stash-push".into(),
            Self::StashApply { .. } => "stash-apply".into(),
            Self::StashPop { .. } => "stash-pop".into(),
            Self::StashDrop { .. } => "stash-drop".into(),
            Self::Push => "push".into(),
            Self::Pull => "pull".into(),
        }
    }
}

/// Cached live state for the current working directory's repo.
/// Everything the UI reads lives here.
pub struct GitState {
    /// Transport + location the client is bound to. Written
    /// through `set_cwd` / `set_remote_target`; read by
    /// `run_refresh` to decide whether to open a subprocess
    /// `GitClient::open` or an SSH-backed `GitClient::open_remote`.
    pub target: GitTarget,
    /// Directory currently mirrored from the left file tree —
    /// kept around for label rendering. For remote targets this
    /// is a `PathBuf` built from the remote cwd string; it is
    /// *never* treated as a live filesystem path.
    pub cwd: PathBuf,
    /// Top-level status, updated by `apply_refresh_result`.
    pub status: GitStatus,
    /// Client keeps a resolved `repo_path`. Wrapped in `Arc` so
    /// background tasks clone it cheaply.
    pub client: Option<Arc<GitClient>>,
    /// Resolved repo root — useful for the branch card.
    pub repo_path: Option<PathBuf>,
    /// Current branch + upstream divergence.
    pub branch: Option<BranchInfo>,
    /// All local branches (for the branch switcher).
    pub branches: Vec<String>,
    /// Working tree changes (staged + unstaged).
    pub changes: Vec<GitFileChange>,
    /// Recent commit log, capped by `log_limit`.
    pub log: Vec<CommitInfo>,
    /// Stash entries (oldest → newest).
    pub stashes: Vec<StashEntry>,
    /// Maximum number of log commits to fetch per refresh.
    pub log_limit: usize,
    /// Stale-result guard — bumped on every `begin_*` call.
    pub refresh_nonce: u64,
    /// The currently in-flight action, if any.
    pub pending: Option<GitPendingAction>,
    /// Last probe error (drives the `RepoState::Error` placeholder
    /// in the view).
    pub last_error: Option<SharedString>,
    /// Last action-level error (commit / checkout / stash failed).
    /// Rendered in its own error card so the user can see what
    /// happened without losing the snapshot.
    pub action_error: Option<SharedString>,
    /// Short confirmation for the last successful action (e.g.
    /// the commit hash, stash ref). Cleared on the next action.
    pub last_confirmation: Option<SharedString>,
    /// Currently selected file for the diff panel. `None` collapses
    /// the diff card; `Some((path, staged))` shows the diff for that
    /// entry. `staged` drives `diff(path, true|false)` vs
    /// `diff_untracked(path)`.
    pub diff_selection: Option<DiffSelection>,
    /// Cached diff output for `diff_selection`. `None` until the
    /// background task resolves, or if the selection was cleared.
    pub diff_output: Option<SharedString>,
    /// `true` while `run_diff` is in flight.
    pub diff_loading: bool,
    /// Most recent diff error (separate from the probe-level error).
    pub diff_error: Option<SharedString>,
    /// Stale-result guard for the diff fetch — bumped on every
    /// `begin_diff` / `clear_diff_selection`.
    pub diff_nonce: u64,
}

/// One selected file change, drives the diff card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffSelection {
    pub path: String,
    /// `true` = staged (index) side, `false` = worktree side.
    pub staged: bool,
    /// `true` = this file is untracked, so `diff_untracked` is used
    /// instead of the normal `diff`.
    pub untracked: bool,
}

impl GitState {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            target: GitTarget::Local(cwd.clone()),
            cwd,
            status: GitStatus::Idle,
            client: None,
            repo_path: None,
            branch: None,
            branches: Vec::new(),
            changes: Vec::new(),
            log: Vec::new(),
            stashes: Vec::new(),
            log_limit: 30,
            refresh_nonce: 0,
            pending: None,
            last_error: None,
            action_error: None,
            last_confirmation: None,
            diff_selection: None,
            diff_output: None,
            diff_loading: false,
            diff_error: None,
            diff_nonce: 0,
        }
    }

    /// Current transport target.
    pub fn target(&self) -> &GitTarget {
        &self.target
    }

    /// Retarget the cached Git state to a new left-panel directory.
    /// Returns `true` when the cwd actually changed. Shim over
    /// `set_target` kept for callers that still pass a `PathBuf`.
    pub fn set_cwd(&mut self, cwd: PathBuf) -> bool {
        self.set_target(GitTarget::Local(cwd))
    }

    /// Retarget to a remote repo reachable through `session` at
    /// `cwd`. Clears the cached snapshot so the view falls back to
    /// the loading state until the next refresh lands.
    pub fn set_remote_target(&mut self, session: SshSession, cwd: String) -> bool {
        self.set_target(GitTarget::Remote { session, cwd })
    }

    fn set_target(&mut self, target: GitTarget) -> bool {
        if self.target.same_repo(&target) {
            return false;
        }

        // Keep the `cwd: PathBuf` field in sync for UI components
        // that still read it (e.g. label rendering). For remote
        // targets we mirror the remote path into `cwd` as a plain
        // PathBuf — good enough for display; it is never used as
        // a real filesystem path.
        match &target {
            GitTarget::Local(p) => self.cwd = p.clone(),
            GitTarget::Remote { cwd, .. } => self.cwd = PathBuf::from(cwd),
        }
        self.target = target;
        self.client = None;
        self.repo_path = None;
        self.branch = None;
        self.branches.clear();
        self.changes.clear();
        self.log.clear();
        self.stashes.clear();
        self.status = GitStatus::Idle;
        self.pending = None;
        self.last_error = None;
        self.action_error = None;
        self.last_confirmation = None;
        self.diff_selection = None;
        self.diff_output = None;
        self.diff_loading = false;
        self.diff_error = None;
        self.refresh_nonce = self.refresh_nonce.wrapping_add(1);
        self.diff_nonce = self.diff_nonce.wrapping_add(1);
        true
    }

    /// Mint a diff fetch request for the given selection. Bumps the
    /// nonce so any in-flight diff for a previous selection drops.
    pub fn begin_diff(&mut self, selection: DiffSelection) -> DiffRequest {
        self.diff_nonce = self.diff_nonce.wrapping_add(1);
        self.diff_loading = true;
        self.diff_output = None;
        self.diff_error = None;
        self.diff_selection = Some(selection.clone());
        DiffRequest {
            nonce: self.diff_nonce,
            // Safe: caller only reaches `begin_diff` after checking
            // `client.is_some()` (the view reads from the cached
            // snapshot). If the client was just cleared, the diff
            // will fail with a plain "no client" error.
            client: self.client.clone(),
            selection,
        }
    }

    pub fn apply_diff_result(&mut self, result: DiffResult) {
        if result.nonce != self.diff_nonce {
            return;
        }
        self.diff_loading = false;
        match result.outcome {
            Ok(text) => {
                self.diff_output = Some(text.into());
                self.diff_error = None;
            }
            Err(err) => {
                self.diff_output = None;
                self.diff_error = Some(err.into());
            }
        }
    }

    pub fn clear_diff_selection(&mut self) {
        self.diff_nonce = self.diff_nonce.wrapping_add(1);
        self.diff_loading = false;
        self.diff_output = None;
        self.diff_error = None;
        self.diff_selection = None;
    }

    /// Open a fresh handle to the repo. For `Local` targets we
    /// probe synchronously (cheap subprocess); for `Remote`
    /// targets we defer — opening an SSH-backed client calls
    /// `rev-parse --show-toplevel` over the network, which can't
    /// run on the UI thread. The first refresh does the open and
    /// caches the result via `apply_refresh_result`.
    pub fn ensure_client(&mut self) {
        if self.client.is_some() {
            return;
        }
        match &self.target {
            GitTarget::Remote { .. } => {
                // Defer — the first `run_refresh` opens + caches.
            }
            GitTarget::Local(_) => self.ensure_local_client(),
        }
    }

    fn ensure_local_client(&mut self) {
        let path = self.cwd.to_string_lossy().to_string();
        match GitClient::open(&path) {
            Ok(client) => {
                self.repo_path = Some(client.repo_path().to_path_buf());
                self.client = Some(Arc::new(client));
                self.status = GitStatus::Idle;
                self.last_error = None;
            }
            Err(GitError::NotARepo(_)) => {
                self.client = None;
                self.repo_path = None;
                self.status = GitStatus::NotARepo;
                self.last_error = None;
            }
            Err(err) => {
                self.client = None;
                self.repo_path = None;
                self.status = GitStatus::Failed;
                self.last_error = Some(err.to_string().into());
            }
        }
    }

    /// Mint a refresh request + bump the nonce. Always produces a
    /// request — Local targets that aren't a repo bubble up as
    /// `NotARepo` through the refresh result, same as the Remote
    /// path. Previously only emitted when `self.client.is_some()`,
    /// which blocked the first refresh on a Remote target (the
    /// client is opened *inside* the background task).
    pub fn begin_refresh(&mut self) -> Option<RefreshRequest> {
        self.refresh_nonce = self.refresh_nonce.wrapping_add(1);
        self.status = GitStatus::Loading;
        self.pending = Some(GitPendingAction::Refresh);
        Some(RefreshRequest {
            nonce: self.refresh_nonce,
            target: self.target.clone(),
            client: self.client.clone(),
            log_limit: self.log_limit,
        })
    }

    pub fn apply_refresh_result(&mut self, result: RefreshResult) {
        if result.nonce != self.refresh_nonce {
            return;
        }
        // Refresh may have opened a fresh client (Remote first-refresh
        // path). Cache it so subsequent actions / refreshes reuse
        // the live handle.
        if let Some(client) = result.client.clone() {
            self.repo_path = Some(client.repo_path().to_path_buf());
            self.client = Some(client);
        }
        self.pending = None;
        match result.outcome {
            Ok(snapshot) => {
                self.branch = Some(snapshot.branch);
                self.branches = snapshot.branches;
                self.changes = snapshot.changes;
                self.log = snapshot.log;
                self.stashes = snapshot.stashes;
                self.status = GitStatus::Ready;
                self.last_error = None;
            }
            Err(err) => {
                // "NotARepo" is a legitimate state, not a hard error
                // — fold it into the dedicated status so the view
                // shows the placeholder.
                if err.starts_with("not a git repository") {
                    self.status = GitStatus::NotARepo;
                    self.last_error = None;
                } else {
                    self.status = GitStatus::Failed;
                    self.last_error = Some(err.into());
                }
            }
        }
    }

    /// Mint an action request. Returns `None` when another action
    /// is already in flight or the client is missing.
    pub fn begin_action(&mut self, action: GitPendingAction) -> Option<ActionRequest> {
        if self.pending.is_some() {
            return None;
        }
        let client = self.client.clone()?;
        self.pending = Some(action.clone());
        self.action_error = None;
        self.last_confirmation = None;
        Some(ActionRequest { client, action })
    }

    pub fn apply_action_result(&mut self, result: ActionResult) {
        self.pending = None;
        match result.outcome {
            Ok(confirmation) => {
                self.last_confirmation = confirmation.map(SharedString::from);
                self.action_error = None;
            }
            Err(err) => {
                self.action_error = Some(err.into());
                self.last_confirmation = None;
            }
        }
    }
}

// ─── Background-task request / result envelopes ───────────────────────

pub struct RefreshRequest {
    pub nonce: u64,
    pub target: GitTarget,
    /// Cached client from a previous refresh, if any. Skipping the
    /// `rev-parse` probe on warm refreshes saves ~50ms over SSH.
    pub client: Option<Arc<GitClient>>,
    pub log_limit: usize,
}

pub struct RefreshSnapshot {
    pub branch: BranchInfo,
    pub branches: Vec<String>,
    pub changes: Vec<GitFileChange>,
    pub log: Vec<CommitInfo>,
    pub stashes: Vec<StashEntry>,
}

pub struct RefreshResult {
    pub nonce: u64,
    /// Client opened / reused by this refresh. Echoed back so
    /// `apply_refresh_result` can cache the live handle for
    /// subsequent actions + warm refreshes.
    pub client: Option<Arc<GitClient>>,
    pub outcome: Result<RefreshSnapshot, String>,
}

pub struct ActionRequest {
    pub client: Arc<GitClient>,
    pub action: GitPendingAction,
}

pub struct DiffRequest {
    pub nonce: u64,
    pub client: Option<Arc<GitClient>>,
    pub selection: DiffSelection,
}

pub struct DiffResult {
    pub nonce: u64,
    pub outcome: Result<String, String>,
}

pub struct ActionResult {
    /// Most actions don't have a meaningful confirmation string;
    /// the ones that do (commit → hash, stash_push → ref) surface
    /// it here so the view can show it briefly.
    pub outcome: Result<Option<String>, String>,
}

// ─── Background-task workers ──────────────────────────────────────────

pub fn run_refresh(request: RefreshRequest) -> RefreshResult {
    // Reuse cached client when we have one; otherwise open a fresh
    // one matching the target. The open is on the background task
    // — safe even for Remote (which SSH-exec's `rev-parse`).
    let client_result: Result<Arc<GitClient>, String> = match request.client.clone() {
        Some(c) => Ok(c),
        None => match &request.target {
            GitTarget::Local(path) => GitClient::open(&path.to_string_lossy())
                .map(Arc::new)
                .map_err(|e| match e {
                    GitError::NotARepo(p) => format!("not a git repository: {p}"),
                    other => other.to_string(),
                }),
            GitTarget::Remote { session, cwd } => {
                GitClient::open_remote(session.clone(), cwd)
                    .map(Arc::new)
                    .map_err(|e| match e {
                        GitError::NotARepo(p) => format!("not a git repository: {p}"),
                        other => other.to_string(),
                    })
            }
        },
    };

    match client_result {
        Ok(client) => {
            let outcome = refresh_inner(&client, request.log_limit);
            RefreshResult {
                nonce: request.nonce,
                client: Some(client),
                outcome,
            }
        }
        Err(err) => RefreshResult {
            nonce: request.nonce,
            client: None,
            outcome: Err(err),
        },
    }
}

fn refresh_inner(client: &GitClient, log_limit: usize) -> Result<RefreshSnapshot, String> {
    let branch = client.branch_info().map_err(|e| e.to_string())?;
    let branches = client.branch_list().map_err(|e| e.to_string())?;
    let changes = client.status().map_err(|e| e.to_string())?;
    let log = client.log(log_limit).map_err(|e| e.to_string())?;
    let stashes = client.stash_list().map_err(|e| e.to_string())?;
    Ok(RefreshSnapshot {
        branch,
        branches,
        changes,
        log,
        stashes,
    })
}

pub fn run_action(request: ActionRequest) -> ActionResult {
    let outcome = match &request.action {
        GitPendingAction::Refresh => Err("refresh is not dispatched as an action".into()),
        GitPendingAction::Stage { path } => request
            .client
            .stage(std::slice::from_ref(path))
            .map(|_| None)
            .map_err(|e| e.to_string()),
        GitPendingAction::Unstage { path } => request
            .client
            .unstage(std::slice::from_ref(path))
            .map(|_| None)
            .map_err(|e| e.to_string()),
        GitPendingAction::Discard { path } => request
            .client
            .discard(std::slice::from_ref(path))
            .map(|_| None)
            .map_err(|e| e.to_string()),
        GitPendingAction::StageAll => request
            .client
            .stage_all()
            .map(|_| None)
            .map_err(|e| e.to_string()),
        GitPendingAction::UnstageAll => request
            .client
            .unstage_all()
            .map(|_| None)
            .map_err(|e| e.to_string()),
        GitPendingAction::Commit { message } => request
            .client
            .commit(message)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::CheckoutBranch { name } => request
            .client
            .checkout_branch(name)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::StashPush { message } => request
            .client
            .stash_push(message)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::StashApply { index } => request
            .client
            .stash_apply(index)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::StashPop { index } => request
            .client
            .stash_pop(index)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::StashDrop { index } => request
            .client
            .stash_drop(index)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::Push => request.client.push().map(Some).map_err(|e| e.to_string()),
        GitPendingAction::Pull => request.client.pull().map(Some).map_err(|e| e.to_string()),
    };
    ActionResult { outcome }
}

pub fn run_diff(request: DiffRequest) -> DiffResult {
    let outcome = match request.client.as_ref() {
        None => Err("git client unavailable".to_string()),
        Some(client) => {
            if request.selection.untracked {
                client
                    .diff_untracked(&request.selection.path)
                    .map_err(|e| e.to_string())
            } else {
                client
                    .diff(&request.selection.path, request.selection.staged)
                    .map_err(|e| e.to_string())
            }
        }
    };
    DiffResult {
        nonce: request.nonce,
        outcome,
    }
}

/// Resolve the starting cwd for the view — used by `PierApp::new`.
pub fn default_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::{DiffSelection, GitPendingAction, GitState, GitStatus};
    use gpui::SharedString;
    use pier_core::services::git::{BranchInfo, CommitInfo, FileStatus, GitFileChange, StashEntry};
    use std::path::PathBuf;

    #[test]
    fn set_cwd_resets_cached_snapshot_and_invalidates_requests() {
        let mut state = GitState::new(PathBuf::from("/tmp/one"));
        state.status = GitStatus::Ready;
        state.repo_path = Some(PathBuf::from("/tmp/one/.git"));
        state.branch = Some(BranchInfo {
            name: "main".into(),
            tracking: "origin/main".into(),
            ahead: 1,
            behind: 0,
        });
        state.branches = vec!["main".into()];
        state.changes = vec![GitFileChange {
            path: "src/main.rs".into(),
            status: FileStatus::Modified,
            staged: false,
        }];
        state.log = vec![CommitInfo {
            hash: "0123456789abcdef0123456789abcdef01234567".into(),
            short_hash: "0123456".into(),
            message: "Initial commit".into(),
            author: "Pier".into(),
            timestamp: 1_713_481_600,
            relative_date: "just now".into(),
            refs: "HEAD -> main".into(),
        }];
        state.stashes = vec![StashEntry {
            index: "stash@{0}".into(),
            message: "WIP".into(),
            relative_date: "just now".into(),
        }];
        state.pending = Some(GitPendingAction::Refresh);
        state.last_error = Some(SharedString::from("boom"));
        state.action_error = Some(SharedString::from("nope"));
        state.last_confirmation = Some(SharedString::from("done"));
        state.diff_selection = Some(DiffSelection {
            path: "src/main.rs".into(),
            staged: false,
            untracked: false,
        });
        state.diff_output = Some(SharedString::from("diff"));
        state.diff_loading = true;
        state.diff_error = Some(SharedString::from("diff boom"));
        state.refresh_nonce = 4;
        state.diff_nonce = 9;

        assert!(state.set_cwd(PathBuf::from("/tmp/two")));
        assert_eq!(state.cwd, PathBuf::from("/tmp/two"));
        assert!(state.client.is_none());
        assert!(state.repo_path.is_none());
        assert!(state.branch.is_none());
        assert!(state.branches.is_empty());
        assert!(state.changes.is_empty());
        assert!(state.log.is_empty());
        assert!(state.stashes.is_empty());
        assert!(matches!(state.status, GitStatus::Idle));
        assert!(state.pending.is_none());
        assert!(state.last_error.is_none());
        assert!(state.action_error.is_none());
        assert!(state.last_confirmation.is_none());
        assert!(state.diff_selection.is_none());
        assert!(state.diff_output.is_none());
        assert!(!state.diff_loading);
        assert!(state.diff_error.is_none());
        assert_eq!(state.refresh_nonce, 5);
        assert_eq!(state.diff_nonce, 10);
    }

    #[test]
    fn set_cwd_is_noop_when_path_is_unchanged() {
        let mut state = GitState::new(PathBuf::from("/tmp/one"));
        state.status = GitStatus::Ready;
        state.refresh_nonce = 2;

        assert!(!state.set_cwd(PathBuf::from("/tmp/one")));
        assert!(matches!(state.status, GitStatus::Ready));
        assert_eq!(state.refresh_nonce, 2);
    }
}
