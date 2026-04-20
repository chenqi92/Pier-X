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

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{px, ListAlignment, ListState, SharedString};
use pier_core::git_graph::{
    self, compute_graph_layout, CommitEntry, GraphRow, LayoutInput, LayoutParams,
};
use pier_core::services::git::{
    BlameLine, BranchEntry, BranchInfo, CommitDetail, CommitInfo, ConfigEntry, GitClient, GitError,
    GitFileChange, RemoteInfo, ResetMode, StashEntry, SubmoduleInfo, TagInfo,
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

/// Top-level tab of the Git panel, mirrors Pier's 4-tab
/// picker (Changes / History / Stash / Conflicts). Non-conflict
/// managers (Branches / Tags / Remotes / Config / Submodules /
/// Rebase) are exposed as icon buttons in the branch action row
/// and open as an inline panel above the tab body, not as a
/// top-level tab — matching Pier's visual grammar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GitTab {
    /// Working tree — staged + unstaged, diff.
    #[default]
    Changes,
    /// IDEA-style commit graph + filters.
    History,
    /// Stash list + stash push.
    Stash,
    /// Merge conflict resolver.
    Conflicts,
}

impl GitTab {
    pub fn all() -> [Self; 4] {
        [Self::Changes, Self::History, Self::Stash, Self::Conflicts]
    }

    pub fn id_token(self) -> &'static str {
        match self {
            Self::Changes => "changes",
            Self::History => "history",
            Self::Stash => "stash",
            Self::Conflicts => "conflicts",
        }
    }
}

/// Sub-tab inside the Managers tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ManagerTab {
    /// Branch list / create / delete / rename / switch.
    #[default]
    Branches,
    /// Tag list / create / delete / push.
    Tags,
    /// Remote list / add / edit / remove / fetch.
    Remotes,
    /// Git config entries (user.name, user.email, etc.).
    Config,
    /// Submodule list / add / update / remove.
    Submodules,
    /// Interactive rebase controls (continue / abort / skip).
    Rebase,
    /// Merge conflict resolver.
    Conflicts,
}

impl ManagerTab {
    /// Managers exposed as branch-row icon buttons. Conflicts is
    /// absent because it lives on the top-level `GitTab::Conflicts`
    /// tab — the Pier target treats conflicts as a first-class mode
    /// alongside Changes / History / Stash, not as a manager popup.
    pub fn icons() -> [Self; 6] {
        [
            Self::Branches,
            Self::Tags,
            Self::Remotes,
            Self::Config,
            Self::Submodules,
            Self::Rebase,
        ]
    }

    pub fn id_token(self) -> &'static str {
        match self {
            Self::Branches => "branches",
            Self::Tags => "tags",
            Self::Remotes => "remotes",
            Self::Config => "config",
            Self::Submodules => "submodules",
            Self::Rebase => "rebase",
            Self::Conflicts => "conflicts",
        }
    }
}

/// Diff rendering mode for the diff panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffMode {
    /// Unified single-column diff (legacy + default).
    #[default]
    Inline,
    /// Two-column old / new side-by-side diff.
    SideBySide,
}

/// Which action the commit split-button fires when its primary half
/// is clicked. Picking the other option from the dropdown switches
/// this *and* fires the action — see views/git.rs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommitActionMode {
    #[default]
    Commit,
    CommitAndPush,
}

impl CommitActionMode {
    /// Stable string used as the dropdown option value.
    pub fn id(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::CommitAndPush => "commit_push",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "commit" => Some(Self::Commit),
            "commit_push" => Some(Self::CommitAndPush),
            _ => None,
        }
    }
}

/// Date range filter for the graph toolbar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphDateRange {
    #[default]
    All,
    Today,
    LastWeek,
    LastMonth,
    LastYear,
}

impl GraphDateRange {
    /// Unix timestamp threshold for this range (0 = no filter).
    pub fn after_timestamp(self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        match self {
            Self::All => 0,
            Self::Today => now - 86_400,
            Self::LastWeek => now - 7 * 86_400,
            Self::LastMonth => now - 30 * 86_400,
            Self::LastYear => now - 365 * 86_400,
        }
    }
}

/// Highlight mode for the graph — dims non-matching rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphHighlightMode {
    #[default]
    None,
    MyCommits,
    MergeCommits,
    CurrentBranch,
}

/// IDEA-style graph filter — user-tweakable inputs that drive the
/// next `run_graph`.
#[derive(Clone, Debug, Default)]
pub struct GraphFilter {
    pub branch: Option<String>,
    pub author: Option<String>,
    pub search_text: Option<String>,
    pub date_range: GraphDateRange,
    pub path_filter: Option<String>,
    /// Topo order vs date order.
    pub sort_by_date: bool,
    pub first_parent_only: bool,
    pub no_merges: bool,
    pub show_long_edges: bool,
}

/// Pending mutation against the repo — drives the spinner label
/// and blocks additional actions until the current one settles.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum GitPendingAction {
    Refresh,
    Stage {
        path: String,
    },
    Unstage {
        path: String,
    },
    Discard {
        path: String,
    },
    StageAll,
    UnstageAll,
    Commit {
        message: String,
    },
    CommitAmend {
        message: String,
    },
    CheckoutBranch {
        name: String,
    },
    CheckoutHash {
        hash: String,
    },
    CheckoutTracking {
        local: String,
        remote: String,
    },
    StashPush {
        message: String,
    },
    StashApply {
        index: String,
    },
    StashPop {
        index: String,
    },
    StashDrop {
        index: String,
    },
    Push,
    Pull,
    // Branch ops
    BranchCreate {
        name: String,
        base: Option<String>,
    },
    BranchDelete {
        name: String,
        force: bool,
    },
    BranchRename {
        old: String,
        new: String,
    },
    // Reset / cherry-pick / revert / drop
    Reset {
        mode: ResetMode,
        target: String,
    },
    CherryPick {
        hash: String,
    },
    Revert {
        hash: String,
    },
    UndoCommit {
        hash: String,
    },
    DropCommit {
        hash: String,
        parent: Option<String>,
    },
    // Merge
    Merge {
        branch: String,
    },
    MergeAbort,
    // Rebase
    Rebase {
        onto: String,
    },
    RebaseContinue,
    RebaseAbort,
    RebaseSkip,
    // Tags
    TagCreate {
        name: String,
        message: String,
        at: Option<String>,
    },
    TagDelete {
        name: String,
    },
    TagPush {
        name: String,
    },
    // Remotes
    RemoteAdd {
        name: String,
        url: String,
    },
    RemoteRemove {
        name: String,
    },
    RemoteSetUrl {
        name: String,
        url: String,
    },
    RemoteFetch {
        name: Option<String>,
    },
    // Config
    ConfigSet {
        key: String,
        value: String,
        global: bool,
    },
    ConfigUnset {
        key: String,
        global: bool,
    },
    // Submodules
    SubmoduleAdd {
        url: String,
        path: String,
    },
    SubmoduleUpdate,
    SubmoduleRemove {
        path: String,
    },
    // Conflicts
    ResolveOurs {
        path: String,
    },
    ResolveTheirs {
        path: String,
    },
    MarkResolved {
        path: String,
    },
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
            Self::CommitAmend { .. } => "commit-amend".into(),
            Self::CheckoutBranch { .. } => "checkout".into(),
            Self::CheckoutHash { .. } => "checkout".into(),
            Self::CheckoutTracking { .. } => "checkout".into(),
            Self::StashPush { .. } => "stash-push".into(),
            Self::StashApply { .. } => "stash-apply".into(),
            Self::StashPop { .. } => "stash-pop".into(),
            Self::StashDrop { .. } => "stash-drop".into(),
            Self::Push => "push".into(),
            Self::Pull => "pull".into(),
            Self::BranchCreate { .. } => "branch-create".into(),
            Self::BranchDelete { .. } => "branch-delete".into(),
            Self::BranchRename { .. } => "branch-rename".into(),
            Self::Reset { .. } => "reset".into(),
            Self::CherryPick { .. } => "cherry-pick".into(),
            Self::Revert { .. } => "revert".into(),
            Self::UndoCommit { .. } => "undo-commit".into(),
            Self::DropCommit { .. } => "drop-commit".into(),
            Self::Merge { .. } => "merge".into(),
            Self::MergeAbort => "merge-abort".into(),
            Self::Rebase { .. } => "rebase".into(),
            Self::RebaseContinue => "rebase-continue".into(),
            Self::RebaseAbort => "rebase-abort".into(),
            Self::RebaseSkip => "rebase-skip".into(),
            Self::TagCreate { .. } => "tag-create".into(),
            Self::TagDelete { .. } => "tag-delete".into(),
            Self::TagPush { .. } => "tag-push".into(),
            Self::RemoteAdd { .. } => "remote-add".into(),
            Self::RemoteRemove { .. } => "remote-remove".into(),
            Self::RemoteSetUrl { .. } => "remote-set-url".into(),
            Self::RemoteFetch { .. } => "fetch".into(),
            Self::ConfigSet { .. } => "config-set".into(),
            Self::ConfigUnset { .. } => "config-unset".into(),
            Self::SubmoduleAdd { .. } => "submodule-add".into(),
            Self::SubmoduleUpdate => "submodule-update".into(),
            Self::SubmoduleRemove { .. } => "submodule-remove".into(),
            Self::ResolveOurs { .. } => "resolve-ours".into(),
            Self::ResolveTheirs { .. } => "resolve-theirs".into(),
            Self::MarkResolved { .. } => "mark-resolved".into(),
        }
    }
}

/// Column in the graph detail table. Used by `toggle_graph_column`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphColumn {
    Hash,
    Author,
    Date,
}

/// Sub-state: IDEA-style commit graph. Populated by `run_graph`.
pub struct GraphState {
    /// Rendered rows (node + segments + arrows), bounded by paging.
    pub rows: Vec<GraphRow>,
    /// Rendering parameters used at last layout — kept so the view
    /// can echo them back for mouse hit-testing.
    pub lane_width: f32,
    pub row_height: f32,
    /// Default branch (e.g. "main" or "origin/main") used as the
    /// main-chain baseline for color assignment.
    pub default_branch: String,
    /// Unpushed commit hashes — drives the "disable destructive
    /// actions on pushed commits" rule.
    pub unpushed: HashSet<String>,
    /// Available branches (for the filter dropdown).
    pub branches: Vec<String>,
    /// Unique authors (for the filter dropdown).
    pub authors: Vec<String>,
    /// Tracked file list (for the path picker).
    pub files: Vec<String>,
    /// User-edited filter state.
    pub filter: GraphFilter,
    /// Current page size — grows as the user scrolls.
    pub page_size: usize,
    /// `true` if the last load returned a full page (more may exist).
    pub has_more: bool,
    /// `true` while a load is in flight.
    pub loading: bool,
    pub load_more_loading: bool,
    /// Last error from `run_graph` — surfaces in the graph toolbar.
    pub error: Option<SharedString>,
    /// Monotonic counter, bumped on every filter change so stale
    /// results drop and the view can reset scroll on reload.
    pub generation: u64,
    /// `none | my-commits | merges | current-branch` — dim filter.
    pub highlight_mode: GraphHighlightMode,
    /// Selected commit hash (drives the detail strip).
    pub selected: Option<String>,
    /// Column visibility toggles — keep the columns selectable.
    pub show_hash_col: bool,
    pub show_author_col: bool,
    pub show_date_col: bool,
    /// Zebra stripes on alternate rows.
    pub zebra_stripes: bool,
    /// Stale-result guard for run_graph.
    pub nonce: u64,
    /// Scroll + measurement state for the History list. Stored here
    /// so the view can use `gpui::list` with variable-height rows
    /// (the selected commit expands its row with an inline detail
    /// strip). `ListState` is Rc-backed so cheap to clone into the
    /// view snapshot; resets are driven from
    /// `apply_graph_result` / `set_graph_selected`.
    pub list_state: ListState,
}

impl GraphState {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            lane_width: 14.0,
            row_height: 22.0,
            default_branch: String::new(),
            unpushed: HashSet::new(),
            branches: Vec::new(),
            authors: Vec::new(),
            files: Vec::new(),
            filter: GraphFilter {
                show_long_edges: false,
                ..Default::default()
            },
            // Bumped to 1000 for Pier-parity — uniform virtualization
            // means only the visible 18 rows cost anything to render,
            // so a 1000-row initial load is fine even on older Macs.
            // Auto-pagination still kicks in past this threshold.
            page_size: 1000,
            has_more: false,
            loading: false,
            load_more_loading: false,
            error: None,
            generation: 0,
            highlight_mode: GraphHighlightMode::None,
            selected: None,
            show_hash_col: true,
            show_author_col: true,
            show_date_col: true,
            zebra_stripes: true,
            nonce: 0,
            list_state: ListState::new(0, ListAlignment::Top, px(200.0)),
        }
    }
}

/// Commit-input footer drag state — captured at `mouse_down` on the
/// splitter, held until `mouse_up` fires globally. A live drag
/// snaps the footer height to `start_height - (current_y -
/// start_y)` so dragging up grows the footer.
#[derive(Clone, Copy, Debug)]
pub struct FooterDrag {
    pub start_mouse_y: f32,
    pub start_height: f32,
}

/// Open context menu for a graph commit — stores everything the
/// menu renderer needs (hash, parsed refs, unpushed flag, browser
/// URL) so `PierApp::Render` can paint the menu without re-reading
/// state mid-render (which double-leases the entity).
#[derive(Clone, Debug)]
pub struct CommitMenuState {
    pub position: gpui::Point<gpui::Pixels>,
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    /// First parent hash (for `rebase --onto` when dropping a non-HEAD commit).
    pub parent: Option<String>,
    /// Ref decoration tokens, already split on `,` and trimmed.
    pub refs: Vec<String>,
    /// `true` if the commit is not yet pushed (enables Undo / Edit /
    /// Drop). Driven by `GraphState::unpushed`.
    pub is_unpushed: bool,
    /// `true` if the selected commit is HEAD (enables `--amend` for
    /// Edit message, and `reset --hard HEAD~1` for Drop).
    pub is_head: bool,
    /// Optional browser URL derived from the first remote — non-None
    /// surfaces an "Open in browser" menu item.
    pub browser_url: Option<String>,
}

/// Sub-state: commit detail strip shown below the selected graph row.
#[derive(Default)]
pub struct CommitDetailState {
    pub hash: Option<String>,
    pub detail: Option<CommitDetail>,
    pub loading: bool,
    pub error: Option<SharedString>,
    pub nonce: u64,
}

/// Sub-state: blame for a single file.
#[derive(Default)]
pub struct BlameState {
    pub path: Option<String>,
    pub lines: Vec<BlameLine>,
    pub loading: bool,
    pub error: Option<SharedString>,
    pub nonce: u64,
}

/// Sub-state: data behind the Managers tab (branches, tags,
/// remotes, config, submodules, conflicts).
#[derive(Default)]
pub struct ManagersState {
    pub branches: Vec<BranchEntry>,
    pub tags: Vec<TagInfo>,
    pub remotes: Vec<RemoteInfo>,
    pub config: Vec<ConfigEntry>,
    pub submodules: Vec<SubmoduleInfo>,
    pub conflicts: Vec<String>,
    pub user_name: String,
    pub user_email: String,
    pub loading: bool,
    pub error: Option<SharedString>,
    pub nonce: u64,
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
    /// Diff rendering mode (inline vs side-by-side).
    pub diff_mode: DiffMode,
    /// Currently active top-level tab.
    pub tab: GitTab,
    /// Currently open manager overlay panel. `None` = no overlay
    /// (tab body is visible); `Some(t)` = overlay panel drawn above
    /// the tab body for the matching manager. Branch-row icons
    /// toggle this.
    pub manager_panel_open: Option<ManagerTab>,
    /// Currently active manager sub-tab — now only used by the
    /// overlay panel to remember which manager was last opened.
    pub manager_tab: ManagerTab,
    /// Graph sub-state (populated by `run_graph`).
    pub graph: GraphState,
    /// Commit detail strip state.
    pub commit_detail: CommitDetailState,
    /// Blame panel state.
    pub blame: BlameState,
    /// Managers tab state.
    pub managers: ManagersState,
    /// Current height (in logical pixels) of the commit-input footer
    /// on the Changes tab. Persisted across renders; only the
    /// resizer splitter mutates it.
    pub footer_height: f32,
    /// Active drag on the footer splitter, if any.
    pub footer_drag: Option<FooterDrag>,
    /// Remembered default action for the commit split-button.
    pub commit_action_mode: CommitActionMode,
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
            diff_mode: DiffMode::Inline,
            tab: GitTab::Changes,
            manager_panel_open: None,
            manager_tab: ManagerTab::Branches,
            graph: GraphState::new(),
            commit_detail: CommitDetailState::default(),
            blame: BlameState::default(),
            managers: ManagersState::default(),
            // 92 px is the tightest height where the commit input
            // + bottom button row both fit; drag the splitter up
            // for more room.
            footer_height: 92.0,
            footer_drag: None,
            commit_action_mode: CommitActionMode::Commit,
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
        // Reset the graph + detail + blame + managers caches — they
        // belong to the previous repo and would mislead the user
        // otherwise. Preserve user preferences (column toggles,
        // zebra stripes, diff mode, active tab).
        self.graph.rows.clear();
        self.graph.unpushed.clear();
        self.graph.branches.clear();
        self.graph.authors.clear();
        self.graph.files.clear();
        self.graph.default_branch.clear();
        self.graph.selected = None;
        self.graph.error = None;
        self.graph.loading = false;
        self.graph.load_more_loading = false;
        self.graph.has_more = false;
        self.graph.generation = self.graph.generation.wrapping_add(1);
        self.graph.nonce = self.graph.nonce.wrapping_add(1);
        self.commit_detail = CommitDetailState::default();
        self.blame = BlameState::default();
        self.managers = ManagersState::default();
        true
    }

    /// Set the active top-level tab. Closes any open manager
    /// overlay, since the user is asking for a different context.
    pub fn set_tab(&mut self, tab: GitTab) {
        self.tab = tab;
        self.manager_panel_open = None;
    }

    /// Open / close the manager overlay panel above the tab body.
    /// Passing the already-open panel closes it (toggle).
    pub fn set_manager_panel(&mut self, panel: Option<ManagerTab>) {
        self.manager_panel_open = panel;
        if let Some(p) = panel {
            self.manager_tab = p;
        }
    }

    pub fn set_manager_tab(&mut self, tab: ManagerTab) {
        self.manager_tab = tab;
        self.manager_panel_open = Some(tab);
    }

    pub fn begin_footer_drag(&mut self, mouse_y: f32) {
        self.footer_drag = Some(FooterDrag {
            start_mouse_y: mouse_y,
            start_height: self.footer_height,
        });
    }

    pub fn update_footer_drag(&mut self, mouse_y: f32) -> bool {
        if let Some(drag) = self.footer_drag {
            // Dragging upward (current < start) grows the footer.
            let delta = drag.start_mouse_y - mouse_y;
            let next = (drag.start_height + delta).clamp(80.0, 480.0);
            if (next - self.footer_height).abs() > 0.5 {
                self.footer_height = next;
                return true;
            }
        }
        false
    }

    pub fn end_footer_drag(&mut self) {
        self.footer_drag = None;
    }

    pub fn set_diff_mode(&mut self, mode: DiffMode) {
        self.diff_mode = mode;
    }

    pub fn toggle_graph_column(&mut self, kind: GraphColumn) {
        match kind {
            GraphColumn::Hash => self.graph.show_hash_col = !self.graph.show_hash_col,
            GraphColumn::Author => self.graph.show_author_col = !self.graph.show_author_col,
            GraphColumn::Date => self.graph.show_date_col = !self.graph.show_date_col,
        }
    }

    pub fn toggle_zebra_stripes(&mut self) {
        self.graph.zebra_stripes = !self.graph.zebra_stripes;
    }

    pub fn set_graph_highlight(&mut self, mode: GraphHighlightMode) {
        self.graph.highlight_mode = mode;
    }

    pub fn set_graph_filter(&mut self, filter: GraphFilter) {
        self.graph.filter = filter;
    }

    pub fn set_graph_selected(&mut self, hash: Option<String>) {
        let previous = self.graph.selected.clone();
        self.graph.selected = hash.clone();
        // Clear detail state whenever the selection changes; the
        // detail load fills it back in.
        if hash.is_none() {
            self.commit_detail = CommitDetailState::default();
        }
        // gpui::list caches row heights per item — when a row
        // toggles into / out of "selected with inline detail" its
        // height changes, so re-splice those rows to force a
        // fresh measurement pass. Only the two rows that changed
        // need updating.
        if previous != hash {
            for h in [previous.as_deref(), hash.as_deref()].into_iter().flatten() {
                if let Some(idx) = self.graph.rows.iter().position(|r| r.hash == h) {
                    if idx < self.graph.list_state.item_count() {
                        self.graph.list_state.splice(idx..idx + 1, 1);
                    }
                }
            }
        }
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

    // ── Graph ───────────────────────────────────────────

    /// Mint a graph-load request. `initial=true` resets the cache,
    /// `initial=false` appends (infinite scroll "load more").
    pub fn begin_graph(&mut self, initial: bool) -> Option<GraphRequest> {
        let repo_path = self.repo_path.as_ref()?.to_string_lossy().to_string();
        self.graph.nonce = self.graph.nonce.wrapping_add(1);
        if initial {
            self.graph.loading = true;
            self.graph.error = None;
            self.graph.generation = self.graph.generation.wrapping_add(1);
        } else {
            self.graph.load_more_loading = true;
        }
        let limit = if initial {
            self.graph.page_size
        } else {
            self.graph.page_size
        };
        let skip = if initial { 0 } else { self.graph.rows.len() };
        Some(GraphRequest {
            nonce: self.graph.nonce,
            generation: self.graph.generation,
            repo_path,
            filter: self.graph.filter.clone(),
            lane_width: self.graph.lane_width,
            row_height: self.graph.row_height,
            limit,
            skip,
            initial,
        })
    }

    pub fn apply_graph_result(&mut self, result: GraphResult) {
        if result.nonce != self.graph.nonce {
            return;
        }
        if result.initial {
            self.graph.loading = false;
        } else {
            self.graph.load_more_loading = false;
        }
        match result.outcome {
            Ok(payload) => {
                if result.initial {
                    self.graph.rows = payload.rows;
                } else {
                    self.graph.rows.extend(payload.rows);
                }
                self.graph.has_more = payload.has_more;
                self.graph.unpushed = payload.unpushed;
                self.graph.default_branch = payload.default_branch;
                self.graph.branches = payload.branches;
                self.graph.authors = payload.authors;
                self.graph.files = payload.files;
                self.graph.error = None;
                // Resize the list scroll / measurement state so
                // gpui::list knows how many items exist. `reset`
                // clears cached measurements; `splice` just tacks
                // new items onto the end for append-style loads.
                if result.initial {
                    self.graph.list_state.reset(self.graph.rows.len());
                } else {
                    let new_count = self.graph.rows.len();
                    let old_count = self.graph.list_state.item_count();
                    if new_count > old_count {
                        self.graph
                            .list_state
                            .splice(old_count..old_count, new_count - old_count);
                    }
                }
            }
            Err(err) => {
                self.graph.error = Some(err.into());
            }
        }
    }

    // ── Commit detail ───────────────────────────────────

    pub fn begin_commit_detail(&mut self, hash: String) -> Option<CommitDetailRequest> {
        let client = self.client.clone()?;
        self.commit_detail.nonce = self.commit_detail.nonce.wrapping_add(1);
        self.commit_detail.hash = Some(hash.clone());
        self.commit_detail.loading = true;
        self.commit_detail.error = None;
        self.commit_detail.detail = None;
        Some(CommitDetailRequest {
            nonce: self.commit_detail.nonce,
            client,
            hash,
        })
    }

    pub fn apply_commit_detail_result(&mut self, result: CommitDetailResult) {
        if result.nonce != self.commit_detail.nonce {
            return;
        }
        self.commit_detail.loading = false;
        match result.outcome {
            Ok(detail) => {
                // Remeasure the now-expanded selected row so gpui::list
                // picks up the taller geometry from the inline strip.
                if let Some(idx) = self.graph.rows.iter().position(|r| r.hash == detail.hash) {
                    if idx < self.graph.list_state.item_count() {
                        self.graph.list_state.splice(idx..idx + 1, 1);
                    }
                }
                self.commit_detail.detail = Some(detail);
                self.commit_detail.error = None;
            }
            Err(err) => {
                self.commit_detail.error = Some(err.into());
            }
        }
    }

    // ── Blame ───────────────────────────────────────────

    pub fn begin_blame(&mut self, path: String) -> Option<BlameRequest> {
        let client = self.client.clone()?;
        self.blame.nonce = self.blame.nonce.wrapping_add(1);
        self.blame.path = Some(path.clone());
        self.blame.loading = true;
        self.blame.error = None;
        self.blame.lines.clear();
        Some(BlameRequest {
            nonce: self.blame.nonce,
            client,
            path,
        })
    }

    pub fn apply_blame_result(&mut self, result: BlameResult) {
        if result.nonce != self.blame.nonce {
            return;
        }
        self.blame.loading = false;
        match result.outcome {
            Ok(lines) => {
                self.blame.lines = lines;
                self.blame.error = None;
            }
            Err(err) => {
                self.blame.error = Some(err.into());
            }
        }
    }

    pub fn clear_blame(&mut self) {
        self.blame = BlameState::default();
    }

    // ── Managers ────────────────────────────────────────

    pub fn begin_managers(&mut self) -> Option<ManagersRequest> {
        let client = self.client.clone()?;
        self.managers.nonce = self.managers.nonce.wrapping_add(1);
        self.managers.loading = true;
        self.managers.error = None;
        Some(ManagersRequest {
            nonce: self.managers.nonce,
            client,
        })
    }

    pub fn apply_managers_result(&mut self, result: ManagersResult) {
        if result.nonce != self.managers.nonce {
            return;
        }
        self.managers.loading = false;
        match result.outcome {
            Ok(payload) => {
                self.managers.branches = payload.branches;
                self.managers.tags = payload.tags;
                self.managers.remotes = payload.remotes;
                self.managers.config = payload.config;
                self.managers.submodules = payload.submodules;
                self.managers.conflicts = payload.conflicts;
                self.managers.user_name = payload.user_name;
                self.managers.user_email = payload.user_email;
                self.managers.error = None;
            }
            Err(err) => {
                self.managers.error = Some(err.into());
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

pub struct GraphRequest {
    pub nonce: u64,
    pub generation: u64,
    pub repo_path: String,
    pub filter: GraphFilter,
    pub lane_width: f32,
    pub row_height: f32,
    pub limit: usize,
    pub skip: usize,
    pub initial: bool,
}

pub struct GraphPayload {
    pub rows: Vec<GraphRow>,
    pub has_more: bool,
    pub unpushed: HashSet<String>,
    pub default_branch: String,
    pub branches: Vec<String>,
    pub authors: Vec<String>,
    pub files: Vec<String>,
}

pub struct GraphResult {
    pub nonce: u64,
    pub initial: bool,
    pub outcome: Result<GraphPayload, String>,
}

pub struct CommitDetailRequest {
    pub nonce: u64,
    pub client: Arc<GitClient>,
    pub hash: String,
}

pub struct CommitDetailResult {
    pub nonce: u64,
    pub outcome: Result<CommitDetail, String>,
}

pub struct BlameRequest {
    pub nonce: u64,
    pub client: Arc<GitClient>,
    pub path: String,
}

pub struct BlameResult {
    pub nonce: u64,
    pub outcome: Result<Vec<BlameLine>, String>,
}

pub struct ManagersRequest {
    pub nonce: u64,
    pub client: Arc<GitClient>,
}

pub struct ManagersPayload {
    pub branches: Vec<BranchEntry>,
    pub tags: Vec<TagInfo>,
    pub remotes: Vec<RemoteInfo>,
    pub config: Vec<ConfigEntry>,
    pub submodules: Vec<SubmoduleInfo>,
    pub conflicts: Vec<String>,
    pub user_name: String,
    pub user_email: String,
}

pub struct ManagersResult {
    pub nonce: u64,
    pub outcome: Result<ManagersPayload, String>,
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
            GitTarget::Remote { session, cwd } => GitClient::open_remote(session.clone(), cwd)
                .map(Arc::new)
                .map_err(|e| match e {
                    GitError::NotARepo(p) => format!("not a git repository: {p}"),
                    other => other.to_string(),
                }),
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
        GitPendingAction::CommitAmend { message } => request
            .client
            .commit_amend(message)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::CheckoutHash { hash } => request
            .client
            .checkout_branch(hash)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::CheckoutTracking { local, remote } => request
            .client
            .checkout_tracking(local, remote)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::BranchCreate { name, base } => request
            .client
            .branch_create(name, base.as_deref())
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::BranchDelete { name, force } => request
            .client
            .branch_delete(name, *force)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::BranchRename { old, new } => request
            .client
            .branch_rename(old, new)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::Reset { mode, target } => request
            .client
            .reset(*mode, target)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::CherryPick { hash } => request
            .client
            .cherry_pick(hash)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::Revert { hash } => request
            .client
            .revert(hash)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::UndoCommit { hash } => request
            .client
            .reset(ResetMode::Soft, &format!("{hash}~1"))
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::DropCommit { hash, parent } => {
            // HEAD → `reset --hard HEAD~1`; non-HEAD → rebase --onto <parent> <hash>.
            if let Some(parent) = parent {
                request
                    .client
                    .rebase_drop(parent, hash)
                    .map(Some)
                    .map_err(|e| e.to_string())
            } else {
                request
                    .client
                    .reset(ResetMode::Hard, "HEAD~1")
                    .map(Some)
                    .map_err(|e| e.to_string())
            }
        }
        GitPendingAction::Merge { branch } => request
            .client
            .merge(branch)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::MergeAbort => request
            .client
            .merge_abort()
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::Rebase { onto } => request
            .client
            .rebase(onto)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::RebaseContinue => request
            .client
            .rebase_continue()
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::RebaseAbort => request
            .client
            .rebase_abort()
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::RebaseSkip => request
            .client
            .rebase_skip()
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::TagCreate { name, message, at } => {
            // Need to tag at a specific commit if requested; tag_create
            // uses HEAD, so run raw command with optional target.
            let result = if let Some(target) = at {
                if message.is_empty() {
                    request.client.tag_create_at(name, "", target)
                } else {
                    request.client.tag_create_at(name, message, target)
                }
            } else {
                request.client.tag_create(name, message)
            };
            result.map(Some).map_err(|e| e.to_string())
        }
        GitPendingAction::TagDelete { name } => request
            .client
            .tag_delete(name)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::TagPush { name } => request
            .client
            .tag_push(name, None)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::RemoteAdd { name, url } => request
            .client
            .remote_add(name, url)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::RemoteRemove { name } => request
            .client
            .remote_remove(name)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::RemoteSetUrl { name, url } => request
            .client
            .remote_set_url(name, url)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::RemoteFetch { name } => request
            .client
            .remote_fetch(name.as_deref())
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::ConfigSet { key, value, global } => request
            .client
            .config_set(key, value, *global)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::ConfigUnset { key, global } => request
            .client
            .config_unset(key, *global)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::SubmoduleAdd { url, path } => request
            .client
            .submodule_add(url, path)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::SubmoduleUpdate => request
            .client
            .submodule_update()
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::SubmoduleRemove { path } => request
            .client
            .submodule_remove(path)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::ResolveOurs { path } => request
            .client
            .resolve_ours(path)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::ResolveTheirs { path } => request
            .client
            .resolve_theirs(path)
            .map(Some)
            .map_err(|e| e.to_string()),
        GitPendingAction::MarkResolved { path } => request
            .client
            .mark_resolved(path)
            .map(Some)
            .map_err(|e| e.to_string()),
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

pub fn run_graph(request: GraphRequest) -> GraphResult {
    let outcome = run_graph_inner(&request);
    GraphResult {
        nonce: request.nonce,
        initial: request.initial,
        outcome,
    }
}

fn run_graph_inner(request: &GraphRequest) -> Result<GraphPayload, String> {
    // Resolve default branch first — used as the main-chain baseline
    // for lane color assignment.
    let default_branch = git_graph::detect_default_branch(&request.repo_path)?;

    // Load the commit slice for this page.
    let core_filter = pier_core::git_graph::GraphFilter {
        branch: request.filter.branch.clone(),
        author: request.filter.author.clone(),
        search_text: request
            .filter
            .search_text
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned(),
        after_timestamp: request.filter.date_range.after_timestamp(),
        topo_order: !request.filter.sort_by_date,
        first_parent_only: request.filter.first_parent_only,
        no_merges: request.filter.no_merges,
        paths: request
            .filter
            .path_filter
            .as_ref()
            .map(|s| {
                s.split('\n')
                    .filter(|p| !p.is_empty())
                    .map(|p| p.to_string())
                    .collect()
            })
            .unwrap_or_default(),
    };

    let entries: Vec<CommitEntry> = git_graph::graph_log(
        &request.repo_path,
        request.limit,
        request.skip,
        &core_filter,
    )?;

    let has_more = entries.len() >= request.limit;

    // Main chain hashes — first-parent traversal of the default branch.
    let main_chain_vec = git_graph::first_parent_chain(
        &request.repo_path,
        &default_branch,
        request.limit + request.skip,
    )
    .unwrap_or_default();
    let main_chain_set: HashSet<String> = main_chain_vec.into_iter().collect();

    // Convert CommitEntry → LayoutInput (same schema).
    let layout_inputs: Vec<LayoutInput> = entries
        .iter()
        .map(|c| LayoutInput {
            hash: c.hash.clone(),
            parents: c.parents.clone(),
            short_hash: c.short_hash.clone(),
            refs: c.refs.clone(),
            message: c.message.clone(),
            author: c.author.clone(),
            date_timestamp: c.date_timestamp,
        })
        .collect();

    let rows = compute_graph_layout(
        &layout_inputs,
        &main_chain_set,
        &LayoutParams {
            lane_width: request.lane_width,
            row_height: request.row_height,
            show_long_edges: request.filter.show_long_edges,
        },
    );

    // Only do the expensive "all branches / all authors / all files"
    // sidebar queries on the initial load — saves ~200ms per page.
    let (branches, authors, files, unpushed) = if request.initial {
        let branches = git_graph::list_branches(&request.repo_path).unwrap_or_default();
        let authors = git_graph::list_authors(&request.repo_path, 500).unwrap_or_default();
        let files = git_graph::list_tracked_files(&request.repo_path).unwrap_or_default();
        // Unpushed: use git CLI via a fresh client.
        let unpushed = match GitClient::open(&request.repo_path) {
            Ok(c) => c
                .unpushed_hashes()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            Err(_) => HashSet::new(),
        };
        (branches, authors, files, unpushed)
    } else {
        (Vec::new(), Vec::new(), Vec::new(), HashSet::new())
    };

    Ok(GraphPayload {
        rows,
        has_more,
        unpushed,
        default_branch,
        branches,
        authors,
        files,
    })
}

pub fn run_commit_detail(request: CommitDetailRequest) -> CommitDetailResult {
    let outcome = request
        .client
        .commit_detail(&request.hash)
        .map_err(|e| e.to_string());
    CommitDetailResult {
        nonce: request.nonce,
        outcome,
    }
}

pub fn run_blame(request: BlameRequest) -> BlameResult {
    let outcome = request
        .client
        .blame(&request.path)
        .map_err(|e| e.to_string());
    BlameResult {
        nonce: request.nonce,
        outcome,
    }
}

pub fn run_managers(request: ManagersRequest) -> ManagersResult {
    let outcome = run_managers_inner(&request.client);
    ManagersResult {
        nonce: request.nonce,
        outcome,
    }
}

fn run_managers_inner(client: &GitClient) -> Result<ManagersPayload, String> {
    let branches = client.branch_entries().map_err(|e| e.to_string())?;
    let tags = client.tag_list().map_err(|e| e.to_string())?;
    let remotes = client.remote_list().map_err(|e| e.to_string())?;
    let config = client.config_list().map_err(|e| e.to_string())?;
    let submodules = client.submodule_list().unwrap_or_default();
    let conflicts = client.list_conflicts().unwrap_or_default();
    let user_name = client.user_name();
    let user_email = client.user_email();
    Ok(ManagersPayload {
        branches,
        tags,
        remotes,
        config,
        submodules,
        conflicts,
        user_name,
        user_email,
    })
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
