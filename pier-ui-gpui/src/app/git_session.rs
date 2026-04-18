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
    /// Directory the view was opened against (cwd at startup).
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
    /// Per-file selection state for `stage` / `unstage` — keyed by
    /// `GitFileChange.path`. Currently unused; reserved for the
    /// follow-up diff slice.
    #[allow(dead_code)]
    pub selected_path: Option<String>,
}

impl GitState {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
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
            selected_path: None,
        }
    }

    /// Open a fresh handle to the repo on disk. Does not probe;
    /// caller still needs to schedule a refresh.
    pub fn ensure_client(&mut self) {
        if self.client.is_some() {
            return;
        }
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

    /// Mint a refresh request + bump the nonce. Returns `None` if
    /// there is no client (e.g. the cwd is not a repo).
    pub fn begin_refresh(&mut self) -> Option<RefreshRequest> {
        let client = self.client.clone()?;
        self.refresh_nonce = self.refresh_nonce.wrapping_add(1);
        self.status = GitStatus::Loading;
        self.pending = Some(GitPendingAction::Refresh);
        Some(RefreshRequest {
            nonce: self.refresh_nonce,
            client,
            log_limit: self.log_limit,
        })
    }

    pub fn apply_refresh_result(&mut self, result: RefreshResult) {
        if result.nonce != self.refresh_nonce {
            return;
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
                self.status = GitStatus::Failed;
                self.last_error = Some(err.into());
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
    pub client: Arc<GitClient>,
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
    pub outcome: Result<RefreshSnapshot, String>,
}

pub struct ActionRequest {
    pub client: Arc<GitClient>,
    pub action: GitPendingAction,
}

pub struct ActionResult {
    /// Most actions don't have a meaningful confirmation string;
    /// the ones that do (commit → hash, stash_push → ref) surface
    /// it here so the view can show it briefly.
    pub outcome: Result<Option<String>, String>,
}

// ─── Background-task workers ──────────────────────────────────────────

pub fn run_refresh(request: RefreshRequest) -> RefreshResult {
    let outcome = refresh_inner(&request.client, request.log_limit);
    RefreshResult {
        nonce: request.nonce,
        outcome,
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
        GitPendingAction::Push => request
            .client
            .push()
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::Pull => request
            .client
            .pull()
            .map(Some)
            .map_err(|e| e.to_string()),
    };
    ActionResult { outcome }
}

/// Resolve the starting cwd for the view — used by `PierApp::new`.
pub fn default_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
