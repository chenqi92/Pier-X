//! Three-pane shell mirroring `Pier/PierApp/Sources/Views/MainWindow/MainView.swift`.
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │ Toolbar: [☰L]  Pier-X            [+] [☰R] [☀/☾] │
//! ├──────────┬───────────────────────┬──────────────┤
//! │   Left   │  Tab bar (terminals)  │     Right    │
//! │ Files /  │  ─────────────────    │ 10 mode      │
//! │ Servers  │  Active terminal      │ container +  │
//! │          │   OR Welcome cover    │ icon sidebar │
//! └──────────┴───────────────────────┴──────────────┘
//! ```

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    canvas, div, prelude::*, px, AnyElement, App, Bounds, ClickEvent, Context, DragMoveEvent,
    Empty, Entity, EntityId, IntoElement, MouseButton, MouseDownEvent, Pixels, Render,
    ScrollHandle, SharedString, Window,
};
use gpui_component::{input::InputState, Icon as UiIcon, IconName, PixelsExt as _, WindowExt as _};
use rust_i18n::t;
use std::collections::HashMap;

use pier_core::connections::ConnectionStore;
use pier_core::db_connections::{DbConnection, DbConnectionStore};
use pier_core::ssh::SshConfig;

use crate::app::db_session::{
    run_connect as db_run_connect, run_execute as db_run_execute, run_list as db_run_list,
    DbSessionState,
};
use crate::app::git_session::{
    default_cwd as git_default_cwd, run_action as git_run_action, run_diff as git_run_diff,
    run_refresh as git_run_refresh, DiffSelection, GitPendingAction, GitState,
};
use crate::app::layout::{
    RightMode, CENTER_PANEL_MIN_W, LEFT_PANEL_DEFAULT_W, LEFT_PANEL_MAX_W, LEFT_PANEL_MIN_W,
    RIGHT_PANEL_DEFAULT_W, RIGHT_PANEL_MAX_W, RIGHT_PANEL_MIN_W,
};
use crate::app::route::DbKind;
use crate::app::ssh_session::{
    run_bootstrap, run_docker_command, run_docker_refresh, run_logs_start, run_monitor_refresh,
    run_refresh, run_sftp_mutation, run_tunnel, ServiceProbeStatus, SftpMutationKind,
    SshSessionState, TransferDirection,
};
use crate::app::{
    ActivationHandler, CloseActiveTab, NewTab, OpenSettings, ToggleLeftPanel, ToggleRightPanel,
};
use crate::data::ShellSnapshot;
use crate::theme::{
    heights::{BUTTON_SM_H, GLYPH_SM, GLYPH_XS, ICON_SM, ROW_MD_H},
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_CAPTION, WEIGHT_MEDIUM, WEIGHT_REGULAR},
    ui_font_with,
};
use crate::views::left_panel_view::{
    ActiveServerSessionSnapshot, LeftPanelView, ServerTunnelSnapshot, ServersSidebarSnapshot,
};
use crate::views::right_panel::{
    DockerActionHandler, DockerActionRequest, DockerRefreshHandler, LogsAction, LogsActionHandler,
    ModeSelector, RightPanel,
};
use crate::views::terminal::TerminalPanel;
use crate::views::welcome::WelcomeView;

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;

const REMOTE_PANEL_REFRESH_MS: u64 = 5_000;
const LOG_STREAM_POLL_MS: u64 = 250;
const DEFAULT_LOG_COMMAND: &str = "journalctl -f -n 200 --no-pager";
const PANE_RESIZE_HANDLE_W: Pixels = px(8.0);
const PANE_RESIZE_RULE_W: Pixels = px(1.0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum PaneDivider {
    Left,
    Right,
}

#[derive(Clone)]
struct PaneResizeDrag {
    entity_id: EntityId,
    divider: PaneDivider,
}

impl Render for PaneResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub struct PierApp {
    // ─── Layout state ───
    left_visible: bool,
    right_visible: bool,
    left_panel_width: gpui::Pixels,
    right_panel_width: gpui::Pixels,
    right_mode: RightMode,
    pane_bounds: Bounds<Pixels>,

    // ─── Backend snapshots ───
    connections: Vec<SshConfig>,
    /// User-declared server groups (distinct from tag-derived
    /// groups) — lets an empty group show up in the sidebar as
    /// soon as the user creates it via the "New Group" button.
    declared_groups: Vec<String>,
    /// Saved database connections (MySQL / PostgreSQL for Phase A).
    /// Loaded from `db-connections.json` at startup; mutated by the
    /// connection-form Save / Delete buttons in later commits. The
    /// actual `DbSessionState` entities live elsewhere — this Vec is
    /// just the dropdown source.
    db_connections: Vec<DbConnection>,
    /// Per-tab database session state, lazy-allocated on first
    /// schedule. One entity per `DbKind` tab so each tab has its own
    /// connect/query history.
    db_sessions: HashMap<DbKind, Entity<DbSessionState>>,
    /// Per-tab SQL editor `InputState` entities. Created eagerly at
    /// `PierApp::new` for `Mysql` / `Postgres` (the engines wired in
    /// Phase A) so the multi-line `Input` widget keeps focus and
    /// content across `RenderOnce` rebuilds of `DatabaseView`.
    db_query_inputs: HashMap<DbKind, Entity<InputState>>,

    // ─── Terminal sessions (Pier mirror: multi-tab) ───
    terminals: Vec<Entity<TerminalPanel>>,
    active_terminal: Option<usize>,
    /// Tab right-click menu state. `Some((idx, pos))` while the menu
    /// is open over terminal tab `idx` anchored at viewport `pos`.
    tab_context_menu: Option<(usize, gpui::Point<Pixels>)>,
    /// Horizontal scroll state for the terminal tab bar. Persisted on
    /// PierApp so scroll position survives cross-renders; also makes
    /// it trivial to call `scroll_to_item(active)` on tab activation.
    terminal_tabs_scroll: ScrollHandle,

    /// Last file the user opened from the left panel that should drive the
    /// right-panel Markdown mode. Set by [`Self::open_markdown_file`].
    last_opened_file: Option<PathBuf>,

    // ─── Active SSH session (right-panel SFTP / future remote modes) ───
    active_session: Option<Entity<SshSessionState>>,
    logs_command_input: Entity<InputState>,

    // ─── Local Git session (right-panel Git mode) ───
    /// Cached repo state for the Git mode — probes happen on the
    /// background executor; the view reads from this entity only.
    git_state: Entity<GitState>,
    /// Multi-line commit message input. Pre-created at `new` so
    /// focus / drafts survive `RenderOnce` rebuilds of `GitView`.
    git_commit_input: Entity<InputState>,
    /// Single-line stash-message input.
    git_stash_message_input: Entity<InputState>,
    /// One-shot flag — the first render of a `Git` mode right panel
    /// schedules the initial refresh. Keeps `Render::render` paint-only.
    git_initial_refresh_done: bool,

    // ─── Subviews owning their own state (Phase 9 perf split) ───
    /// Filter input + file-browser cwd cache + tab state live inside this
    /// entity so its `cx.notify()` only repaints the left column rather
    /// than the whole shell on every keystroke. PierApp talks to it only
    /// via `cx.observe` (LeftPanelView pulls fresh `connections` on PierApp
    /// notify) and read-only accessors like [`Self::servers_sidebar_snapshot`].
    left_panel: Entity<LeftPanelView>,
    window_bounds_observer_started: bool,
    window_appearance_observer_started: bool,
    remote_panel_poll_loop_started: bool,
    logs_poll_loop_started: bool,
}

impl PierApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (connections, declared_groups) = ConnectionStore::load_default()
            .map(|s| (s.connections, s.groups))
            .unwrap_or_default();
        let db_connections = DbConnectionStore::load_default()
            .map(|s| s.connections)
            .unwrap_or_default();
        let weak_app = cx.entity().downgrade();
        let connections_for_panel = connections.clone();
        let left_panel =
            cx.new(|lp_cx| LeftPanelView::new(weak_app, connections_for_panel, window, lp_cx));
        let logs_command_input =
            cx.new(|c| InputState::new(window, c).placeholder("journalctl -f -n 200 --no-pager"));
        logs_command_input.update(cx, |state, c| {
            state.set_value(DEFAULT_LOG_COMMAND, window, c);
        });

        // Pre-create the SQL editor for each engine wired in Phase A.
        // Multi-line so users can write long queries; placeholder gives
        // a hint about the dialect.
        let mut db_query_inputs: HashMap<DbKind, Entity<InputState>> = HashMap::new();
        for kind in [DbKind::Mysql, DbKind::Postgres] {
            let placeholder = match kind {
                DbKind::Mysql => "SELECT * FROM information_schema.tables LIMIT 10;",
                DbKind::Postgres => "SELECT schemaname, tablename FROM pg_tables LIMIT 10;",
                _ => "",
            };
            let input = cx.new(|c| {
                InputState::new(window, c)
                    .multi_line(true)
                    .placeholder(placeholder)
            });
            db_query_inputs.insert(kind, input);
        }
        let snapshot = ShellSnapshot::load();
        window.set_window_title(&format!("Pier-X · {}", snapshot.workspace_path));

        // Local Git: probe the working directory up-front so the view
        // can render the "is this a repo?" pill immediately. The
        // actual `git status` / `log` / `branch_list` roundtrip is
        // scheduled on the first render of the Git mode (see
        // `schedule_git_initial_refresh`).
        let git_state = cx.new(|_| {
            let mut state = GitState::new(git_default_cwd());
            state.ensure_client();
            state
        });
        let git_commit_input = cx.new(|c| {
            InputState::new(window, c)
                .multi_line(true)
                .placeholder(t!("App.Git.commit_placeholder"))
        });
        let git_stash_message_input =
            cx.new(|c| InputState::new(window, c).placeholder(t!("App.Git.stash_placeholder")));

        Self {
            left_visible: true,
            right_visible: true,
            left_panel_width: LEFT_PANEL_DEFAULT_W,
            right_panel_width: RIGHT_PANEL_DEFAULT_W,
            right_mode: RightMode::Markdown,
            pane_bounds: Bounds::default(),
            connections,
            declared_groups,
            db_connections,
            db_sessions: HashMap::new(),
            db_query_inputs,
            terminals: Vec::new(),
            tab_context_menu: None,
            active_terminal: None,
            terminal_tabs_scroll: ScrollHandle::new(),
            last_opened_file: None,
            active_session: None,
            logs_command_input,
            git_state,
            git_commit_input,
            git_stash_message_input,
            git_initial_refresh_done: false,
            left_panel,
            window_bounds_observer_started: false,
            window_appearance_observer_started: false,
            remote_panel_poll_loop_started: false,
            logs_poll_loop_started: false,
        }
    }

    /// Read-only snapshot of the Servers sidebar model, used by
    /// [`LeftPanelView`] to keep its local cache in sync via `cx.observe`.
    pub fn servers_sidebar_snapshot(&self, cx: &App) -> ServersSidebarSnapshot {
        let active_session = self.active_session.as_ref().map(|session_entity| {
            let session = session_entity.read(cx);
            ActiveServerSessionSnapshot {
                config: session.config.clone(),
                status: session.status,
                service_probe_status: session.service_probe_status.clone(),
                service_probe_error: session.service_probe_error.clone(),
                services: session.services.clone(),
                tunnels: session
                    .tunnels
                    .iter()
                    .map(|tunnel| ServerTunnelSnapshot {
                        service_name: tunnel.service_name.clone(),
                        remote_port: tunnel.remote_port,
                        local_port: tunnel.local_port,
                        status: tunnel.status,
                        last_error: tunnel.last_error.clone(),
                    })
                    .collect(),
                last_error: session.last_error.clone(),
            }
        });

        ServersSidebarSnapshot {
            connections: self.connections.clone(),
            declared_groups: self.declared_groups.clone(),
            active_session,
        }
    }

    // ─── Terminal session management ───

    pub fn open_terminal_tab(&mut self, cx: &mut Context<Self>) {
        let on_activated: ActivationHandler = Rc::new(|_, _, _| {});
        let entity = cx.new(|cx| TerminalPanel::new(on_activated, cx));
        self.terminals.push(entity);
        self.active_terminal = Some(self.terminals.len() - 1);
        log::info!(
            "app: opened terminal tab active_index={} total_tabs={}",
            self.active_terminal.unwrap_or(0),
            self.terminals.len()
        );
        cx.notify();
    }

    /// Open a terminal tab and immediately type `ssh user@host -p port` into
    /// the new PTY. Mirrors Pier's "click a saved server → terminal opens
    /// connecting" flow; the OS-level `ssh` binary handles auth (key + agent
    /// + Keychain pop-ups), so Pier-X doesn't have to ship a parallel SSH
    /// auth UI for the common case.
    ///
    /// Real `russh`-backed sessions land in a later phase — the placeholder
    /// covers 90 % of the UX with 10 lines of code.
    pub fn open_ssh_terminal(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(conn) = self.connections.get(idx).cloned() else {
            log::warn!("app: ssh-open stale connection index={idx}");
            return;
        };
        log::info!(
            "app: opening ssh terminal connection={} host={} user={} port={}",
            conn.name,
            conn.host,
            conn.user,
            conn.port
        );
        self.open_terminal_tab(cx);
        let Some(active) = self.active_terminal else {
            return;
        };
        let entity = self.terminals[active].clone();
        let command = if conn.port == 22 {
            format!("ssh {}@{}\n", conn.user, conn.host)
        } else {
            format!("ssh {}@{} -p {}\n", conn.user, conn.host, conn.port)
        };
        entity.update(cx, |panel, cx| panel.send_input(&command, cx));

        // Attach a parallel native SSH session for the right-panel SFTP
        // browser. Lazy-connects on first list_dir; replacing whatever
        // previous session was active.
        let session_state = cx.new(|_| SshSessionState::new(conn));
        self.active_session = Some(session_state);
        self.right_visible = true;
        self.normalize_right_mode(cx);
        self.schedule_remote_bootstrap(cx);
        cx.notify();
    }

    pub fn navigate_sftp(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.schedule_sftp_refresh(Some(path), cx);
    }

    pub fn sftp_cd_up(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.active_session.as_ref() else {
            return;
        };
        let next_target = session.read(cx).next_parent_target();
        if let Some(path) = next_target {
            self.schedule_sftp_refresh(Some(path), cx);
        }
    }

    /// Switch the right panel mode. Lazily triggers any one-shot work the
    /// new mode needs (e.g. SFTP first-connect) here on the click rather
    /// than from inside the render path.
    pub fn set_right_mode(&mut self, mode: RightMode, cx: &mut Context<Self>) {
        self.right_mode = mode;
        if matches!(mode, RightMode::Git) {
            let cwd = self.left_panel.read(cx).file_tree_cwd();
            self.sync_git_cwd(cwd, cx);
        }
        self.ensure_right_mode_ready(cx, true);
        cx.notify();
    }

    pub(crate) fn sync_git_cwd(&mut self, cwd: PathBuf, cx: &mut Context<Self>) {
        let changed = self.git_state.update(cx, |state, _| state.set_cwd(cwd));
        if !changed {
            return;
        }

        if matches!(self.right_mode, RightMode::Git) {
            self.git_initial_refresh_done = true;
            self.schedule_git_refresh(cx);
        } else {
            self.git_initial_refresh_done = false;
            cx.notify();
        }
    }

    fn activate_terminal_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.terminals.len() {
            self.active_terminal = Some(idx);
            cx.notify();
        }
    }

    fn close_terminal_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.terminals.len() {
            return;
        }
        self.terminals.remove(idx);
        if self.terminals.is_empty() {
            self.active_terminal = None;
        } else {
            // Snap active to a valid index, preferring the previous neighbour.
            let new_active = match self.active_terminal {
                Some(active) if active == idx => {
                    idx.saturating_sub(1).min(self.terminals.len() - 1)
                }
                Some(active) if active > idx => active - 1,
                Some(active) => active,
                None => 0,
            };
            self.active_terminal = Some(new_active);
        }
        cx.notify();
    }

    /// Close every terminal except `keep_idx`. Active index snaps to
    /// the kept tab, which becomes the only one left.
    pub(crate) fn close_terminal_tabs_others(&mut self, keep_idx: usize, cx: &mut Context<Self>) {
        if keep_idx >= self.terminals.len() {
            return;
        }
        let kept = self.terminals.remove(keep_idx);
        self.terminals.clear();
        self.terminals.push(kept);
        self.active_terminal = Some(0);
        cx.notify();
    }

    /// Close every tab with an index less than `from_idx`.
    pub(crate) fn close_terminal_tabs_left(&mut self, from_idx: usize, cx: &mut Context<Self>) {
        if from_idx == 0 || from_idx > self.terminals.len() {
            return;
        }
        self.terminals.drain(0..from_idx);
        // The reference tab is now at index 0; snap active to it if
        // the previous active was inside the dropped range.
        self.active_terminal = Some(match self.active_terminal {
            Some(active) if active >= from_idx => active - from_idx,
            _ => 0,
        });
        cx.notify();
    }

    /// Close every tab with an index greater than `from_idx`.
    pub(crate) fn close_terminal_tabs_right(&mut self, from_idx: usize, cx: &mut Context<Self>) {
        if from_idx + 1 >= self.terminals.len() {
            return;
        }
        self.terminals.truncate(from_idx + 1);
        self.active_terminal = Some(match self.active_terminal {
            Some(active) if active > from_idx => from_idx,
            Some(active) => active,
            None => from_idx,
        });
        cx.notify();
    }

    pub(crate) fn open_tab_context_menu(
        &mut self,
        idx: usize,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if idx >= self.terminals.len() {
            return;
        }
        self.tab_context_menu = Some((idx, position));
        cx.notify();
    }

    pub(crate) fn close_tab_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.tab_context_menu.take().is_some() {
            cx.notify();
        }
    }

    /// Called by [`LeftPanelView`] when the user clicks a `.md` file in
    /// the file browser. Routes into the right-panel Markdown mode.
    pub fn open_markdown_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        eprintln!("[pier] markdown opened: {}", path.display());
        self.last_opened_file = Some(path);
        self.right_mode = RightMode::Markdown;
        self.right_visible = true;
        cx.notify();
    }

    pub fn refresh_connections(&mut self) {
        let (connections, groups) = ConnectionStore::load_default()
            .map(|s| (s.connections, s.groups))
            .unwrap_or_default();
        self.connections = connections;
        self.declared_groups = groups;
    }

    /// Reload the database connections list from disk after the
    /// editor saves. Mirrors [`Self::refresh_connections`].
    pub fn refresh_db_connections(&mut self) {
        self.db_connections = DbConnectionStore::load_default()
            .map(|s| s.connections)
            .unwrap_or_default();
    }

    /// Read-only access to the saved DB connections list. Used by the
    /// database view's dropdown.
    pub fn db_connections(&self) -> &[DbConnection] {
        &self.db_connections
    }

    /// Session entity for the given DB tab, or `None` if the user has
    /// never interacted with that tab this launch. The entity is
    /// lazy-allocated by the `schedule_db_*` methods — don't call
    /// this to force creation, use `schedule_db_connect` for that.
    pub fn db_session(&self, kind: DbKind) -> Option<Entity<DbSessionState>> {
        self.db_sessions.get(&kind).cloned()
    }

    /// The persistent SQL editor `InputState` for the given tab.
    /// Pre-created at startup for each supported engine, so the
    /// caller can assume `Some` for `Mysql` / `Postgres`.
    pub fn db_query_input(&self, kind: DbKind) -> Option<Entity<InputState>> {
        self.db_query_inputs.get(&kind).cloned()
    }

    /// Remove the DB connection at `idx`, delete its keychain entry
    /// (if any), and persist the shorter list. No-op on stale index.
    /// Called from the database view's Delete button.
    pub fn delete_db_connection(&mut self, idx: usize) {
        let mut store = DbConnectionStore::load_default().unwrap_or_default();
        let Some(removed) = store.remove(idx) else {
            return;
        };
        if let Some(id) = removed.credential_id.as_deref() {
            // Delete is idempotent in pier-core::credentials; we log
            // failures but don't unwind the on-disk removal.
            if let Err(err) = pier_core::credentials::delete(id) {
                log::warn!("delete_db_connection: keychain delete failed for {id}: {err}");
            }
        }
        if let Err(err) = store.save_default() {
            log::warn!("delete_db_connection: save failed: {err}");
            return;
        }
        self.db_connections = store.connections;
    }

    pub fn open_add_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let known_groups = self.known_group_names();
        let weak = cx.entity().downgrade();
        crate::views::edit_connection::open(
            window,
            cx,
            weak,
            crate::views::edit_connection::EditTarget::Add,
            known_groups,
        );
    }

    pub fn open_edit_connection(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(original) = self.connections.get(idx).cloned() else {
            eprintln!("[pier] edit: stale index {idx}");
            return;
        };
        let known_groups = self.known_group_names();
        let weak = cx.entity().downgrade();
        crate::views::edit_connection::open(
            window,
            cx,
            weak,
            crate::views::edit_connection::EditTarget::Edit { idx, original },
            known_groups,
        );
    }

    /// Snapshot of group tags currently in use across saved connections,
    /// for the connection editor's suggestion chips. Computed from `self`
    /// so callers don't need a weak-read while they are mid-`update`
    /// (that path panics with "cannot read PierApp while it is already
    /// being updated").
    fn known_group_names(&self) -> Vec<SharedString> {
        // Union of tag-derived groups (from existing connections)
        // and user-declared empty groups — both are valid chip
        // suggestions in the edit-connection dialog.
        let mut seen = std::collections::BTreeSet::new();
        for g in pier_core::connections::known_groups(&self.connections) {
            seen.insert(g);
        }
        for g in &self.declared_groups {
            seen.insert(g.clone());
        }
        seen.into_iter().map(SharedString::from).collect()
    }

    /// Entry point for the "New Group" folder icon in the Servers
    /// sidebar. Pops the add-group dialog; on save, persists via
    /// [`Self::add_connection_group`].
    pub fn open_add_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        crate::views::add_group::open(window, cx, weak);
    }

    /// Append a user-declared group to the on-disk store. No-op if
    /// the trimmed name is empty or already present. Callers
    /// (typically the add-group dialog) should invoke this from
    /// outside a `PierApp::update` lease.
    pub fn add_connection_group(&mut self, name: String, cx: &mut Context<Self>) {
        let mut store = ConnectionStore::load_default().unwrap_or_default();
        if !store.add_group(&name) {
            return;
        }
        if let Err(err) = store.save_default() {
            log::warn!("add_connection_group: save failed: {err}");
            return;
        }
        self.refresh_connections();
        cx.notify();
    }

    pub fn delete_connection(&mut self, idx: usize, cx: &mut Context<Self>) {
        let mut store = ConnectionStore::load_default().unwrap_or_default();
        if idx >= store.connections.len() {
            eprintln!("[pier] delete: stale index {idx}");
            return;
        }
        // KeychainPassword and PublicKeyFile entries point at OS-keychain
        // secrets — clean those up so the keyring doesn't accumulate
        // dangling credentials when the user deletes a connection.
        if let Some(conn) = store.connections.get(idx) {
            match &conn.auth {
                pier_core::ssh::AuthMethod::KeychainPassword { credential_id } => {
                    let _ = pier_core::credentials::delete(credential_id);
                }
                pier_core::ssh::AuthMethod::PublicKeyFile {
                    passphrase_credential_id: Some(id),
                    ..
                } => {
                    let _ = pier_core::credentials::delete(id);
                }
                _ => {}
            }
        }
        store.connections.remove(idx);
        if let Err(err) = store.save_default() {
            eprintln!("[pier] delete connection failed: {err}");
            return;
        }
        self.refresh_connections();
        cx.notify();
    }

    /// Open a confirm dialog before actually calling [`Self::delete_connection`].
    /// Wired to the trash icon on each row in the Servers list.
    pub fn confirm_delete_connection(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(conn) = self.connections.get(idx).cloned() else {
            return;
        };
        let weak = cx.entity().downgrade();
        let title: SharedString = t!(
            "App.Shell.DeleteConnection.title",
            name = conn.name.as_str()
        )
        .into();
        let detail: SharedString = t!(
            "App.Shell.DeleteConnection.detail",
            user = conn.user.as_str(),
            host = conn.host.as_str(),
            port = conn.port
        )
        .into();
        window.open_dialog(cx, move |dialog, _w, _app_cx| {
            let weak = weak.clone();
            let body = crate::components::text::body(detail.clone()).secondary();
            dialog
                .title(title.clone())
                .w(px(380.0))
                .confirm()
                .button_props(
                    gpui_component::dialog::DialogButtonProps::default()
                        .ok_text(t!("App.Common.delete"))
                        .ok_variant(gpui_component::button::ButtonVariant::Danger)
                        .cancel_text(t!("App.Common.cancel")),
                )
                .on_ok(move |_, _w, app_cx| {
                    let _ = weak.update(app_cx, |this, cx| this.delete_connection(idx, cx));
                    true
                })
                .child(body)
        });
    }
}

impl Render for PierApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_window_bounds_observer(window, cx);
        self.ensure_window_appearance_observer(window, cx);
        self.ensure_remote_panel_poll_loop(cx);
        self.ensure_logs_poll_loop(cx);
        let t = theme(cx).clone();

        let toolbar = crate::app::toolbar::render(self, cx);
        // LeftPanelView is its own Entity — embedding the entity instead of
        // re-building a RenderOnce here means PierApp re-renders DON'T
        // rebuild the file tree cache or filter inputs (Phase 9 perf win).
        let left_entity = self.left_panel.clone();
        let center = self.render_center(&t, cx);
        let right = self
            .right_visible
            .then(|| self.render_right(cx))
            .map(IntoElement::into_any_element);
        let statusbar = crate::app::statusbar::render(self, cx);
        let (left_width, right_width) = self.fitted_panel_widths();
        let pane_bounds_target = cx.entity().downgrade();
        let row = div()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .relative()
            .flex()
            .flex_row()
            .when(self.left_visible, |this| {
                this.child(
                    div()
                        .flex_none()
                        .w(left_width)
                        .h_full()
                        .min_h(px(0.0))
                        .overflow_hidden()
                        .child(left_entity),
                )
                .child(self.render_pane_resize_handle(PaneDivider::Left, &t, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_w(CENTER_PANEL_MIN_W)
                    .min_h(px(0.0))
                    .h_full()
                    .overflow_hidden()
                    .child(center),
            )
            .when_some(right, |this, panel| {
                this.child(self.render_pane_resize_handle(PaneDivider::Right, &t, cx))
                    .child(
                        div()
                            .flex_none()
                            .w(right_width)
                            .h_full()
                            .min_h(px(0.0))
                            .overflow_hidden()
                            .child(panel),
                    )
            })
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let _ = pane_bounds_target.update(cx, |this, cx| {
                            this.update_pane_bounds(bounds, cx);
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );

        div()
            .size_full()
            .bg(t.color.bg_canvas)
            .text_color(t.color.text_primary)
            .font(ui_font_with(
                &t.font_ui,
                &t.font_ui_features,
                WEIGHT_REGULAR,
            ))
            .flex()
            .flex_col()
            // Key-binding context — matches `Some("PierApp")` bindings in
            // `main.rs`. Keeps shell shortcuts from leaking out / colliding
            // with terminal-internal key handling when terminal not focused.
            .key_context("PierApp")
            .on_action(cx.listener(|this, _: &ToggleLeftPanel, _, cx| {
                this.left_visible = !this.left_visible;
                this.clamp_panel_widths();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleRightPanel, _, cx| {
                this.right_visible = !this.right_visible;
                this.clamp_panel_widths();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                let weak = cx.entity().downgrade();
                let connections = this.connections.clone();
                crate::views::new_tab_chooser::open(window, cx, weak, connections);
            }))
            .on_action(cx.listener(|_, _: &OpenSettings, window, cx| {
                crate::views::settings_dialog::open(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseActiveTab, _, cx| {
                if let Some(idx) = this.active_terminal {
                    this.close_terminal_tab(idx, cx);
                }
            }))
            .child(toolbar)
            .child(row)
            .child(statusbar)
            .when_some(self.active_transfer_toast(cx), |root, toast| {
                // Pinned bottom-right, hovering above the status bar.
                // Absolute positioning keeps the rest of the layout
                // from reflowing when the toast appears or disappears.
                root.child(
                    div()
                        .absolute()
                        .right(SP_3)
                        .bottom(SP_3)
                        .flex()
                        .flex_col()
                        .child(toast),
                )
            })
            .when_some(self.tab_context_menu, |root, (idx, position)| {
                // Snapshot `total` off of `self` *before* calling the
                // helper — the helper gets `cx` only, and `self` is
                // unreadable via `cx.entity().read(...)` mid-render.
                let total = self.terminals.len();
                root.child(render_tab_context_menu(idx, position, total, cx))
            })
    }
}

impl PierApp {
    /// Snapshot the active session's in-flight transfer (if any) into
    /// a ready-to-render toast. Kept on PierApp (not on the session
    /// view) because the toast floats above the whole shell, not
    /// inside the SFTP panel — it stays visible even if the user
    /// switches to Markdown or Git mid-transfer.
    fn active_transfer_toast(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<crate::components::TransferToast> {
        let session = self.active_session.as_ref()?;
        let state = session.read(cx).active_transfer.clone()?;
        Some(crate::components::TransferToast::new(state))
    }

    pub(crate) fn left_visible(&self) -> bool {
        self.left_visible
    }

    pub(crate) fn right_visible(&self) -> bool {
        self.right_visible
    }

    pub(crate) fn toggle_left_pane(&mut self, cx: &mut Context<Self>) {
        self.left_visible = !self.left_visible;
        self.clamp_panel_widths();
        cx.notify();
    }

    pub(crate) fn toggle_right_pane(&mut self, cx: &mut Context<Self>) {
        self.right_visible = !self.right_visible;
        self.clamp_panel_widths();
        cx.notify();
    }

    pub(crate) fn terminals_len(&self) -> usize {
        self.terminals.len()
    }

    /// Open the Path Inspector dialog on the given local filesystem
    /// target. Called from the file tree and from directory-entry rows
    /// inside an already-open inspector dialog ("drill in"). The
    /// parent-navigation and preview-toggle actions live on the dialog
    /// entity itself — no PierApp plumbing needed.
    pub(crate) fn inspect_local_path(
        &mut self,
        target: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        crate::views::path_inspector::open(window, cx, weak, target);
    }

    pub(crate) fn active_terminal(&self) -> Option<usize> {
        self.active_terminal
    }

    pub(crate) fn open_new_tab_chooser(&self, window: &mut Window, cx: &mut Context<Self>) {
        log::info!("toolbar: open new-tab chooser");
        let weak = cx.entity().downgrade();
        let connections = self.connections.clone();
        crate::views::new_tab_chooser::open(window, cx, weak, connections);
    }

    fn pane_divider_count(&self) -> usize {
        usize::from(self.left_visible) + usize::from(self.right_visible)
    }

    fn pane_divider_total_width(&self) -> Pixels {
        px(self.pane_divider_count() as f32 * PANE_RESIZE_HANDLE_W.as_f32())
    }

    fn fitted_panel_widths(&self) -> (Pixels, Pixels) {
        let mut left = if self.left_visible {
            self.left_panel_width
                .clamp(LEFT_PANEL_MIN_W, LEFT_PANEL_MAX_W)
        } else {
            px(0.0)
        };
        let mut right = if self.right_visible {
            self.right_panel_width
                .clamp(RIGHT_PANEL_MIN_W, RIGHT_PANEL_MAX_W)
        } else {
            px(0.0)
        };

        let pane_width = self.pane_bounds.size.width;
        if pane_width <= px(0.0) {
            return (left, right);
        }

        let available_side_total =
            (pane_width - CENTER_PANEL_MIN_W - self.pane_divider_total_width()).max(px(0.0));
        let total_side = px(left.as_f32() + right.as_f32());
        if total_side <= available_side_total {
            return (left, right);
        }

        let mut overflow = total_side - available_side_total;
        if self.right_visible {
            let reducible = (right - RIGHT_PANEL_MIN_W).max(px(0.0));
            let reduction = overflow.min(reducible);
            right -= reduction;
            overflow -= reduction;
        }
        if overflow > px(0.0) && self.left_visible {
            let reducible = (left - LEFT_PANEL_MIN_W).max(px(0.0));
            let reduction = overflow.min(reducible);
            left -= reduction;
        }

        (left, right)
    }

    fn clamp_panel_widths(&mut self) {
        let (left_width, right_width) = self.fitted_panel_widths();
        if self.left_visible {
            self.left_panel_width = left_width;
        }
        if self.right_visible {
            self.right_panel_width = right_width;
        }
    }

    fn max_left_panel_width(&self, right_width: Pixels) -> Pixels {
        let pane_width = self.pane_bounds.size.width;
        if pane_width <= px(0.0) {
            return LEFT_PANEL_MAX_W;
        }

        (pane_width - CENTER_PANEL_MIN_W - self.pane_divider_total_width() - right_width)
            .min(LEFT_PANEL_MAX_W)
            .max(LEFT_PANEL_MIN_W)
    }

    fn max_right_panel_width(&self, left_width: Pixels) -> Pixels {
        let pane_width = self.pane_bounds.size.width;
        if pane_width <= px(0.0) {
            return RIGHT_PANEL_MAX_W;
        }

        (pane_width - CENTER_PANEL_MIN_W - self.pane_divider_total_width() - left_width)
            .min(RIGHT_PANEL_MAX_W)
            .max(RIGHT_PANEL_MIN_W)
    }

    fn update_pane_bounds(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        let old_left = self.left_panel_width;
        let old_right = self.right_panel_width;
        self.pane_bounds = bounds;
        self.clamp_panel_widths();

        if self.left_panel_width != old_left || self.right_panel_width != old_right {
            cx.notify();
        }
    }

    fn resize_panel_divider(
        &mut self,
        divider: PaneDivider,
        pointer_x: Pixels,
        cx: &mut Context<Self>,
    ) {
        let (left_width, right_width) = self.fitted_panel_widths();

        match divider {
            PaneDivider::Left => {
                if !self.left_visible {
                    return;
                }

                let raw_width = (pointer_x - self.pane_bounds.left()).max(px(0.0));
                self.left_panel_width =
                    raw_width.clamp(LEFT_PANEL_MIN_W, self.max_left_panel_width(right_width));
            }
            PaneDivider::Right => {
                if !self.right_visible {
                    return;
                }

                let raw_width = (self.pane_bounds.right() - pointer_x).max(px(0.0));
                self.right_panel_width =
                    raw_width.clamp(RIGHT_PANEL_MIN_W, self.max_right_panel_width(left_width));
            }
        }

        cx.notify();
    }

    fn render_pane_resize_handle(
        &self,
        divider: PaneDivider,
        t: &crate::theme::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id((
                "pane-divider",
                match divider {
                    PaneDivider::Left => 0_u32,
                    PaneDivider::Right => 1_u32,
                },
            ))
            .h_full()
            .w(PANE_RESIZE_HANDLE_W)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_col_resize()
            .hover(|style| style.bg(t.color.bg_hover))
            .active(|style| style.bg(t.color.bg_active))
            .on_drag(
                PaneResizeDrag {
                    entity_id: cx.entity_id(),
                    divider,
                },
                |drag, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                },
            )
            .on_drag_move(
                cx.listener(move |this, e: &DragMoveEvent<PaneResizeDrag>, _, cx| {
                    let drag = e.drag(cx);
                    if drag.entity_id != cx.entity_id() || drag.divider != divider {
                        return;
                    }

                    this.resize_panel_divider(divider, e.event.position.x, cx);
                }),
            )
            .child(
                div()
                    .h_full()
                    .w(PANE_RESIZE_RULE_W)
                    .bg(t.color.border_subtle),
            )
    }
}

// ─────────────────────────────────────────────────────────
// Title Bar
// ─────────────────────────────────────────────────────────

impl PierApp {
    fn ensure_window_bounds_observer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_bounds_observer_started {
            return;
        }
        self.window_bounds_observer_started = true;

        cx.observe_window_bounds(window, |_, _, cx| {
            cx.notify();
        })
        .detach();
    }

    fn ensure_window_appearance_observer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_appearance_observer_started {
            return;
        }
        self.window_appearance_observer_started = true;

        cx.observe_window_appearance(window, |_, window, cx| {
            crate::theme::sync_system_appearance(Some(window), cx);
            cx.notify();
        })
        .detach();
    }

    fn ensure_remote_panel_poll_loop(&mut self, cx: &mut Context<Self>) {
        if self.remote_panel_poll_loop_started {
            return;
        }
        self.remote_panel_poll_loop_started = true;

        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    loop {
                        background
                            .timer(Duration::from_millis(REMOTE_PANEL_REFRESH_MS))
                            .await;

                        let still_alive = this
                            .update(&mut async_cx, |this, cx| {
                                if this.right_visible {
                                    match this.right_mode {
                                        RightMode::Monitor => this.schedule_monitor_refresh(cx),
                                        RightMode::Docker => this.schedule_docker_refresh(cx),
                                        _ => {}
                                    }
                                }
                            })
                            .is_ok();

                        if !still_alive {
                            break;
                        }
                    }
                }
            },
        )
        .detach();
    }

    fn ensure_logs_poll_loop(&mut self, cx: &mut Context<Self>) {
        if self.logs_poll_loop_started {
            return;
        }
        self.logs_poll_loop_started = true;

        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    loop {
                        background
                            .timer(Duration::from_millis(LOG_STREAM_POLL_MS))
                            .await;

                        let still_alive = this
                            .update(&mut async_cx, |this, cx| {
                                this.poll_logs_stream(cx);
                            })
                            .is_ok();

                        if !still_alive {
                            break;
                        }
                    }
                }
            },
        )
        .detach();
    }

    fn available_right_modes(&self, cx: &App) -> Vec<RightMode> {
        self.active_session
            .as_ref()
            .map(|session| session.read(cx).available_modes())
            .unwrap_or_else(|| RightMode::LOCAL_ONLY.into_iter().collect())
    }

    fn normalize_right_mode(&mut self, cx: &App) {
        let available = self.available_right_modes(cx);
        if available.contains(&self.right_mode) {
            return;
        }
        self.right_mode = if self.active_session.is_some() {
            RightMode::Monitor
        } else {
            RightMode::Markdown
        };
    }

    fn ensure_right_mode_ready(&mut self, cx: &mut Context<Self>, allow_bootstrap: bool) {
        self.normalize_right_mode(cx);

        let Some(session) = self.active_session.clone() else {
            return;
        };
        let should_bootstrap = {
            let state = session.read(cx);
            matches!(
                state.service_probe_status,
                ServiceProbeStatus::Idle | ServiceProbeStatus::Failed
            ) && !state.is_loading()
        };
        if should_bootstrap && allow_bootstrap {
            self.schedule_remote_bootstrap(cx);
            return;
        }

        match self.right_mode {
            RightMode::Monitor => self.schedule_monitor_refresh(cx),
            RightMode::Docker => self.schedule_docker_refresh(cx),
            RightMode::Logs => self.start_logs_stream(false, cx),
            RightMode::Sftp => {
                if session.read(cx).should_bootstrap() {
                    self.schedule_sftp_refresh(None, cx);
                }
            }
            RightMode::Mysql | RightMode::Postgres | RightMode::Redis => {
                self.schedule_service_tunnel(self.right_mode, cx);
            }
            _ => {}
        }
    }

    fn schedule_remote_bootstrap(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let request = session.update(cx, |state, _| state.begin_bootstrap());

        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background
                        .spawn(async move { run_bootstrap(request) })
                        .await;
                    let _ = session.update(&mut async_cx, |state, _| {
                        state.apply_bootstrap_result(result);
                    });
                    let _ = this.update(&mut async_cx, |this, cx| {
                        this.ensure_right_mode_ready(cx, false);
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    // ─── SFTP mutations (P1-5: mkdir / rename / delete / upload / download)
    //
    // schedule_sftp_mutation is the single dispatch point. The 5 thin
    // wrappers below own UX-specific bits (path prompts, confirm
    // dialogs) and converge on this fn.

    /// Run an SFTP mutation in the background and refresh the
    /// listing on success. No-op when there's no live SFTP session
    /// — UI only exposes mutation buttons after a successful refresh
    /// so this guard rarely trips.
    pub(crate) fn schedule_sftp_mutation(
        &mut self,
        kind: SftpMutationKind,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let Some(mut request) = session.update(cx, |state, _| state.begin_sftp_mutation(kind))
        else {
            return;
        };

        // Wire up progress tracking for Upload / Download. The
        // callback fires on pier-core's tokio worker — it writes into
        // a pair of AtomicU64s (transferred + total) and bumps a
        // generation counter. A GPUI-async task polls that slot at
        // ~16 fps and forwards the latest value into
        // `active_transfer`, respecting Rule 6 (render stays paint-
        // only) without needing a cross-runtime channel.
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;

        struct ProgressSlot {
            transferred: AtomicU64,
            total: AtomicU64,
            gen: AtomicU64,
        }

        let transfer_slot: Option<(u64, Arc<ProgressSlot>)> = match &request.kind {
            SftpMutationKind::Upload { local, remote } => {
                let name = std::path::Path::new(remote)
                    .file_name()
                    .or_else(|| local.file_name())
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| remote.clone());
                let total = std::fs::metadata(local).map(|m| m.len()).unwrap_or(0);
                let slot = Arc::new(ProgressSlot {
                    transferred: AtomicU64::new(0),
                    total: AtomicU64::new(total),
                    gen: AtomicU64::new(0),
                });
                let slot_cb = slot.clone();
                request.progress = Some(Arc::new(move |p| {
                    slot_cb.transferred.store(p.transferred, Ordering::Relaxed);
                    slot_cb.total.store(p.total, Ordering::Relaxed);
                    slot_cb.gen.fetch_add(1, Ordering::Release);
                }));
                let id = session.update(cx, |s, _| {
                    s.begin_transfer(TransferDirection::Upload, name, total)
                });
                Some((id, slot))
            }
            SftpMutationKind::Download { remote, local } => {
                let name = std::path::Path::new(remote)
                    .file_name()
                    .or_else(|| local.file_name())
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| remote.clone());
                let slot = Arc::new(ProgressSlot {
                    transferred: AtomicU64::new(0),
                    total: AtomicU64::new(0),
                    gen: AtomicU64::new(0),
                });
                let slot_cb = slot.clone();
                request.progress = Some(Arc::new(move |p| {
                    slot_cb.transferred.store(p.transferred, Ordering::Relaxed);
                    slot_cb.total.store(p.total, Ordering::Relaxed);
                    slot_cb.gen.fetch_add(1, Ordering::Release);
                }));
                let id = session.update(cx, |s, _| {
                    s.begin_transfer(TransferDirection::Download, name, 0)
                });
                Some((id, slot))
            }
            _ => None,
        };

        let transfer_id_for_result = transfer_slot.as_ref().map(|(id, _)| *id);

        // Progress pump — polls the atomic slot at ~16 fps and
        // forwards the latest tick into SshSessionState. Terminates
        // once the toast with this id is no longer the active one
        // (either replaced by a newer transfer or cleared by the
        // auto-hide timer).
        if let Some((id, slot)) = transfer_slot {
            let session_pump = session.clone();
            cx.spawn(move |_: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let mut async_cx = cx.clone();
                async move {
                    let mut last_gen = 0u64;
                    loop {
                        async_cx
                            .background_executor()
                            .timer(Duration::from_millis(60))
                            .await;
                        let gen = slot.gen.load(Ordering::Acquire);
                        let alive = session_pump
                            .update(&mut async_cx, |state, cx| {
                                let still_ours = state
                                    .active_transfer
                                    .as_ref()
                                    .map(|t| t.id == id)
                                    .unwrap_or(false);
                                if still_ours && gen != last_gen {
                                    state.update_transfer_progress(
                                        id,
                                        pier_core::ssh::TransferProgress {
                                            transferred: slot.transferred.load(Ordering::Relaxed),
                                            total: slot.total.load(Ordering::Relaxed),
                                        },
                                    );
                                    cx.notify();
                                }
                                still_ours
                            })
                            .unwrap_or(false);
                        if !alive {
                            break;
                        }
                        last_gen = gen;
                    }
                }
            })
            .detach();
        }

        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background
                        .spawn(async move { run_sftp_mutation(request) })
                        .await;
                    let succeeded = session
                        .update(&mut async_cx, |state, _| {
                            state.apply_sftp_mutation_result(result)
                        })
                        .unwrap_or(false);

                    if let Some(id) = transfer_id_for_result {
                        let _ = session.update(&mut async_cx, |state, cx| {
                            state.finish_transfer(id, succeeded);
                            cx.notify();
                        });
                        // Hold the toast briefly so the user sees the
                        // terminal phase, then clear.
                        let hold = if succeeded {
                            Duration::from_millis(1_500)
                        } else {
                            Duration::from_millis(2_500)
                        };
                        async_cx.background_executor().timer(hold).await;
                        let _ = session.update(&mut async_cx, |state, cx| {
                            state.clear_transfer(id);
                            cx.notify();
                        });
                    }

                    if succeeded {
                        let _ = this.update(&mut async_cx, |this, cx| {
                            this.schedule_sftp_refresh(None, cx);
                        });
                    } else {
                        let _ = this.update(&mut async_cx, |_, cx| {
                            cx.notify();
                        });
                    }
                }
            },
        )
        .detach();
    }

    /// Open the OS file picker, then upload each chosen path into
    /// the current SFTP cwd, sequentially (one mutation in flight at
    /// a time — the existing nonce guard would drop concurrent
    /// uploads anyway).
    pub(crate) fn sftp_upload_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let cwd = session.read(cx).cwd.clone();
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(SharedString::from("Upload to remote")),
        });
        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let mut async_cx = cx.clone();
                async move {
                    let Ok(picked) = receiver.await else { return };
                    let Ok(Some(paths)) = picked else { return };
                    let _ = this.update(&mut async_cx, |this, cx| {
                        this.enqueue_sftp_uploads(&cwd, paths, cx);
                    });
                }
            },
        )
        .detach();
    }

    /// Entry point for drag-and-drop upload: filesystem paths arrive
    /// from the OS (via `on_drop::<ExternalPaths>`) and get uploaded
    /// into the current SFTP cwd, one mutation per file. Mirrors the
    /// post-picker tail of [`Self::sftp_upload_prompt`] without the
    /// dialog round-trip.
    pub(crate) fn sftp_upload_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let cwd = session.read(cx).cwd.clone();
        self.enqueue_sftp_uploads(&cwd, paths, cx);
    }

    /// Shared loop body for both the picker and the drop handler.
    /// Directories are skipped with a warn — recursive upload isn't
    /// in the current transfer primitives, and dropping a folder
    /// silently ignoring its contents would be worse than telling
    /// the user nothing happened.
    fn enqueue_sftp_uploads(
        &mut self,
        cwd: &std::path::Path,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        for local in paths {
            if local.is_dir() {
                log::warn!("sftp_upload: directories are not yet supported: {local:?}");
                continue;
            }
            let Some(name) = local.file_name().and_then(|n| n.to_str()) else {
                log::warn!("sftp_upload: skipping non-utf8 filename: {local:?}");
                continue;
            };
            let remote = join_remote_path(cwd, name);
            let kind = SftpMutationKind::Upload {
                local: local.clone(),
                remote,
            };
            self.schedule_sftp_mutation(kind, cx);
        }
    }

    /// Open the OS save-file dialog and download `remote_path` into
    /// the chosen destination. `suggested_name` becomes the default
    /// filename (i.e. the basename of the remote file).
    pub(crate) fn sftp_download_prompt(
        &mut self,
        remote_path: String,
        suggested_name: String,
        cx: &mut Context<Self>,
    ) {
        let directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let receiver = cx.prompt_for_new_path(&directory, Some(&suggested_name));
        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let mut async_cx = cx.clone();
                async move {
                    let Ok(picked) = receiver.await else { return };
                    let Ok(Some(local)) = picked else { return };
                    let kind = SftpMutationKind::Download {
                        remote: remote_path,
                        local,
                    };
                    let _ = this.update(&mut async_cx, |this, cx| {
                        this.schedule_sftp_mutation(kind, cx);
                    });
                }
            },
        )
        .detach();
    }

    /// Pop the standard delete-confirmation dialog. On OK, dispatch
    /// the matching mutation (`DeleteFile` for files, `DeleteDir`
    /// for dirs — the latter is server-enforced empty-only).
    pub(crate) fn confirm_sftp_delete(
        &mut self,
        path: String,
        name: String,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let title: SharedString = format!("Delete {name}?").into();
        let detail: SharedString = if is_dir {
            format!(
                "Remove the directory {name}? This cannot be undone. The directory must be empty \
                 — non-empty deletes will surface a server error."
            )
            .into()
        } else {
            format!("Remove the file {name}? This cannot be undone.").into()
        };
        window.open_dialog(cx, move |dialog, _w, _app_cx| {
            let weak = weak.clone();
            let path = path.clone();
            let body = crate::components::text::body(detail.clone()).secondary();
            dialog
                .title(title.clone())
                .w(px(380.0))
                .confirm()
                .button_props(
                    gpui_component::dialog::DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(gpui_component::button::ButtonVariant::Danger)
                        .cancel_text("Cancel"),
                )
                .on_ok(move |_, _w, app_cx| {
                    let kind = if is_dir {
                        SftpMutationKind::DeleteDir { path: path.clone() }
                    } else {
                        SftpMutationKind::DeleteFile { path: path.clone() }
                    };
                    let _ = weak.update(app_cx, |this, cx| {
                        this.schedule_sftp_mutation(kind, cx);
                    });
                    true
                })
                .child(body)
        });
    }

    fn schedule_monitor_refresh(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let Some(request) = session.update(cx, |state, _| state.begin_monitor_refresh()) else {
            return;
        };

        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background
                        .spawn(async move { run_monitor_refresh(request) })
                        .await;
                    let _ = session.update(&mut async_cx, |state, _| {
                        state.apply_monitor_result(result);
                    });
                    let _ = this.update(&mut async_cx, |_, cx| {
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    fn schedule_docker_refresh(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let Some(request) = session.update(cx, |state, _| state.begin_docker_refresh()) else {
            return;
        };

        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background
                        .spawn(async move { run_docker_refresh(request) })
                        .await;
                    let _ = session.update(&mut async_cx, |state, _| {
                        state.apply_docker_refresh_result(result);
                    });
                    let _ = this.update(&mut async_cx, |_, cx| {
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    fn run_docker_action(&mut self, action: DockerActionRequest, cx: &mut Context<Self>) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let Some(request) = session.update(cx, |state, _| {
            state.begin_docker_action(action.kind, &action.target_id, &action.target_label)
        }) else {
            return;
        };

        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background
                        .spawn(async move { run_docker_command(request) })
                        .await;
                    let _ = session.update(&mut async_cx, |state, _| {
                        state.apply_docker_command_result(result);
                    });
                    let _ = this.update(&mut async_cx, |_, cx| {
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    fn start_logs_stream(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let should_start = session.read(cx).should_autostart_logs() || force;
        if !should_start {
            return;
        }

        let command = self.logs_command_input.read(cx).value().to_string();
        let Some(request) = session.update(cx, |state, _| state.begin_logs_start(command)) else {
            return;
        };

        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background
                        .spawn(async move { run_logs_start(request) })
                        .await;
                    let _ = session.update(&mut async_cx, |state, _| {
                        state.apply_logs_start_result(result);
                    });
                    let _ = this.update(&mut async_cx, |_, cx| {
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    fn stop_logs_stream(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let changed = session.update(cx, |state, _| state.stop_logs());
        if changed {
            cx.notify();
        }
    }

    fn clear_logs_stream(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let changed = session.update(cx, |state, _| state.clear_logs());
        if changed {
            cx.notify();
        }
    }

    fn poll_logs_stream(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let changed = session.update(cx, |state, _| state.drain_logs_stream());
        if changed {
            cx.notify();
        }
    }

    fn apply_logs_preset(&mut self, command: String, window: &mut Window, cx: &mut Context<Self>) {
        self.logs_command_input
            .update(cx, |state, c| state.set_value(command, window, c));
        self.start_logs_stream(true, cx);
    }

    fn handle_logs_action(
        &mut self,
        action: LogsAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            LogsAction::RunCurrent => self.start_logs_stream(true, cx),
            LogsAction::Stop => self.stop_logs_stream(cx),
            LogsAction::Clear => self.clear_logs_stream(cx),
            LogsAction::Preset { command } => self.apply_logs_preset(command, window, cx),
        }
    }

    fn schedule_sftp_refresh(&mut self, target: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let request = session.update(cx, |state, _| {
            let next_cwd = target.clone().unwrap_or_else(|| state.cwd.clone());
            state.begin_refresh(next_cwd)
        });

        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background.spawn(async move { run_refresh(request) }).await;
                    let _ = session.update(&mut async_cx, |state, _| {
                        state.apply_refresh_result(result);
                    });
                    let _ = this.update(&mut async_cx, |_, cx| {
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    // ─── Database (MySQL / PostgreSQL) schedulers ────────────────
    //
    // Same pattern as schedule_sftp_refresh / schedule_remote_bootstrap:
    //   1. bump a nonce + flip status via begin_*
    //   2. cx.notify() so the UI sees the "Connecting" pill
    //   3. cx.spawn → background.spawn(async { run_*(request) }).await
    //   4. session.update(apply_*_result) — nonce guard drops stale ones
    //
    // Phase A only plumbs MySQL + PostgreSQL; Redis / SQLite variants
    // get their own schedulers when those phases ship.

    /// Ensure a `DbSessionState` entity exists for the given tab and
    /// return a strong handle. Called by every `schedule_db_*` entry
    /// so the UI never has to pre-allocate sessions.
    pub(crate) fn db_session_for(
        &mut self,
        kind: DbKind,
        cx: &mut Context<Self>,
    ) -> Entity<DbSessionState> {
        if let Some(existing) = self.db_sessions.get(&kind) {
            return existing.clone();
        }
        let entity = cx.new(|_| DbSessionState::new());
        self.db_sessions.insert(kind, entity.clone());
        entity
    }

    /// Kick off a connect on the given tab's session. `connection` is
    /// cloned into the background task; `password` is looked up via
    /// the keychain by the caller so pier-core never sees the raw
    /// credential through this entry point.
    pub(crate) fn schedule_db_connect(
        &mut self,
        kind: DbKind,
        connection: DbConnection,
        password: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let session = self.db_session_for(kind, cx);
        let Some(request) = session.update(cx, |state, _| {
            state.select_connection(connection);
            state.begin_connect(password)
        }) else {
            return;
        };

        cx.notify();
        cx.spawn(
            move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background
                        .spawn(async move { db_run_connect(request) })
                        .await;
                    let _ = session.update(&mut async_cx, |state, _| {
                        state.apply_connect_result(result);
                    });
                }
            },
        )
        .detach();
    }

    /// Fetch the database list for the session's active client.
    pub(crate) fn schedule_db_list_databases(&mut self, kind: DbKind, cx: &mut Context<Self>) {
        let session = self.db_session_for(kind, cx);
        let Some(request) = session.update(cx, |state, _| state.begin_list_databases()) else {
            return;
        };

        cx.spawn(
            move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background.spawn(async move { db_run_list(request) }).await;
                    let _ = session.update(&mut async_cx, |state, _| {
                        state.apply_list_result(result);
                    });
                }
            },
        )
        .detach();
    }

    /// Fetch the table list for a specific database. Also updates
    /// `selected_database` on the session (moved into `begin_list_tables`).
    pub(crate) fn schedule_db_list_tables(
        &mut self,
        kind: DbKind,
        database: String,
        cx: &mut Context<Self>,
    ) {
        let session = self.db_session_for(kind, cx);
        let Some(request) = session.update(cx, |state, _| state.begin_list_tables(database)) else {
            return;
        };

        cx.notify();
        cx.spawn(
            move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background.spawn(async move { db_run_list(request) }).await;
                    let _ = session.update(&mut async_cx, |state, _| {
                        state.apply_list_result(result);
                    });
                }
            },
        )
        .detach();
    }

    /// Run a user-supplied SQL statement on the session's active
    /// client. Returns without scheduling if a query is already in
    /// flight (the UI gates the button, this is defense in depth).
    pub(crate) fn schedule_db_execute(
        &mut self,
        kind: DbKind,
        sql: String,
        cx: &mut Context<Self>,
    ) {
        let session = self.db_session_for(kind, cx);
        let Some(request) = session.update(cx, |state, _| state.begin_execute(sql)) else {
            return;
        };

        cx.notify();
        cx.spawn(
            move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background
                        .spawn(async move { db_run_execute(request) })
                        .await;
                    let _ = session.update(&mut async_cx, |state, _| {
                        state.apply_execute_result(result);
                    });
                }
            },
        )
        .detach();
    }

    // ─── Git session accessors + scheduling ───

    pub fn git_state(&self) -> Entity<GitState> {
        self.git_state.clone()
    }

    pub fn git_commit_input(&self) -> Entity<InputState> {
        self.git_commit_input.clone()
    }

    pub fn git_stash_message_input(&self) -> Entity<InputState> {
        self.git_stash_message_input.clone()
    }

    /// Called on each render of the Git mode so the user sees real
    /// data without clicking anything. The flag guard keeps this
    /// render-safe: only the first render of the Git mode actually
    /// schedules IO.
    pub fn schedule_git_initial_refresh(&mut self, cx: &mut Context<Self>) {
        if self.git_initial_refresh_done {
            return;
        }
        self.git_initial_refresh_done = true;
        self.schedule_git_refresh(cx);
    }

    pub fn schedule_git_refresh(&mut self, cx: &mut Context<Self>) {
        // Re-open the client if it never booted (e.g. a repo was
        // just `git init`ed under the cwd).
        self.git_state.update(cx, |state, _| {
            state.ensure_client();
        });
        let state = self.git_state.clone();
        let Some(request) = state.update(cx, |s, _| s.begin_refresh()) else {
            cx.notify();
            return;
        };

        cx.notify();
        cx.spawn(
            move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background
                        .spawn(async move { git_run_refresh(request) })
                        .await;
                    let _ = state.update(&mut async_cx, |s, cx| {
                        s.apply_refresh_result(result);
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    pub fn schedule_git_diff(&mut self, selection: DiffSelection, cx: &mut Context<Self>) {
        let state = self.git_state.clone();
        let request = state.update(cx, |s, _| s.begin_diff(selection));
        cx.notify();
        cx.spawn(
            move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background.spawn(async move { git_run_diff(request) }).await;
                    let _ = state.update(&mut async_cx, |s, cx| {
                        s.apply_diff_result(result);
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    pub fn clear_git_diff_selection(&mut self, cx: &mut Context<Self>) {
        self.git_state.update(cx, |s, _| s.clear_diff_selection());
        cx.notify();
    }

    pub fn schedule_git_action(&mut self, action: GitPendingAction, cx: &mut Context<Self>) {
        let state = self.git_state.clone();
        let Some(request) = state.update(cx, |s, _| s.begin_action(action)) else {
            return;
        };
        cx.notify();
        let this = cx.entity().downgrade();
        cx.spawn(
            move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background
                        .spawn(async move { git_run_action(request) })
                        .await;
                    let _ = state.update(&mut async_cx, |s, cx| {
                        s.apply_action_result(result);
                        cx.notify();
                    });
                    // A successful mutation invalidates the cached
                    // snapshot — always schedule a refresh so the view
                    // shows fresh branches / status / log / stashes.
                    let _ = this.update(&mut async_cx, |this, cx| {
                        this.schedule_git_refresh(cx);
                    });
                }
            },
        )
        .detach();
    }

    fn schedule_service_tunnel(&mut self, mode: RightMode, cx: &mut Context<Self>) {
        let Some(service_name) = mode.required_service_name() else {
            return;
        };
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let Some(request) = session.update(cx, |state, _| state.begin_tunnel(service_name)) else {
            return;
        };

        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let result = background.spawn(async move { run_tunnel(request) }).await;
                    let _ = session.update(&mut async_cx, |state, _| {
                        state.apply_tunnel_result(result);
                    });
                    let _ = this.update(&mut async_cx, |this, cx| {
                        this.normalize_right_mode(cx);
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }
}

// ─────────────────────────────────────────────────────────
// Right / Center
// (Left is now a separate `Entity<LeftPanelView>` — embedded directly
// in the `Render::render` flex row.)
// ─────────────────────────────────────────────────────────

impl PierApp {
    fn render_right(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let on_select_mode: ModeSelector =
            Rc::new(cx.listener(|this, mode: &RightMode, _, cx| this.set_right_mode(*mode, cx)));
        let on_sftp_navigate: crate::views::sftp_browser::NavigateHandler =
            Rc::new(cx.listener(|this, path: &PathBuf, _, cx| {
                this.navigate_sftp(path.clone(), cx);
            }));
        let on_sftp_go_up: crate::views::sftp_browser::GoUpHandler =
            Rc::new(cx.listener(|this, _: &(), _, cx| this.sftp_cd_up(cx)));
        let on_sftp_mkdir: crate::views::sftp_browser::HeaderActionHandler =
            Rc::new(cx.listener(|this, _: &(), window, cx| {
                let Some(session) = this.active_session.as_ref() else {
                    return;
                };
                let cwd = session.read(cx).cwd.to_string_lossy().into_owned();
                let weak = cx.entity().downgrade();
                crate::views::sftp_dialogs::open_mkdir_dialog(window, cx, weak, cwd);
            }));
        let on_sftp_upload: crate::views::sftp_browser::HeaderActionHandler =
            Rc::new(cx.listener(|this, _: &(), _, cx| this.sftp_upload_prompt(cx)));
        let on_sftp_row_action: crate::views::sftp_browser::RowActionHandler =
            Rc::new(cx.listener(
                |this, action: &crate::views::sftp_browser::RowAction, window, cx| {
                    use crate::views::sftp_browser::RowAction;
                    match action.clone() {
                        RowAction::Rename { path, name } => {
                            let weak = cx.entity().downgrade();
                            crate::views::sftp_dialogs::open_rename_dialog(
                                window, cx, weak, path, name,
                            );
                        }
                        RowAction::Delete { path, name, is_dir } => {
                            this.confirm_sftp_delete(path, name, is_dir, window, cx);
                        }
                        RowAction::Download { path, name } => {
                            this.sftp_download_prompt(path, name, cx);
                        }
                    }
                },
            ));
        let on_sftp_drop: crate::views::sftp_browser::DropPathsHandler =
            Rc::new(cx.listener(|this, paths: &Vec<PathBuf>, _, cx| {
                this.sftp_upload_paths(paths.clone(), cx);
            }));
        let on_docker_refresh: DockerRefreshHandler =
            Rc::new(cx.listener(|this, _: &(), _, cx| this.schedule_docker_refresh(cx)));
        let on_docker_action: DockerActionHandler =
            Rc::new(cx.listener(|this, action: &DockerActionRequest, _, cx| {
                this.run_docker_action(action.clone(), cx);
            }));
        let on_logs_action: LogsActionHandler =
            Rc::new(cx.listener(|this, action: &LogsAction, window, cx| {
                this.handle_logs_action(action.clone(), window, cx);
            }));

        // Only forward the path to the Markdown mode if it actually points
        // at a .md file — keeps the empty-state messaging clean.
        let current_markdown = self.last_opened_file.clone().filter(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("md"))
                .unwrap_or(false)
        });
        let (_, right_width) = self.fitted_panel_widths();
        RightPanel::new(
            self.right_mode,
            current_markdown,
            self.active_session.clone(),
            self.logs_command_input.clone(),
            cx.entity().downgrade(),
            on_sftp_navigate,
            on_sftp_go_up,
            on_sftp_mkdir,
            on_sftp_upload,
            on_sftp_row_action,
            on_sftp_drop,
            on_docker_refresh,
            on_docker_action,
            on_logs_action,
            on_select_mode,
            right_width,
        )
    }

    fn render_center(&mut self, t: &crate::theme::Theme, cx: &mut Context<Self>) -> AnyElement {
        if let Some(active) = self.active_terminal {
            let tab_count = self.terminals.len();
            let active_term = self.terminals[active].clone();

            let scroll_handle = self.terminal_tabs_scroll.clone();
            let tab_bar = render_terminal_tab_bar(t, active, tab_count, &scroll_handle, cx);

            return div()
                .h_full()
                .flex()
                .flex_col()
                .child(tab_bar)
                .child(div().flex_1().min_h(px(0.0)).child(active_term))
                .into_any_element();
        }

        // Welcome cover state — buttons stay truthful to their labels:
        // "new SSH" opens the add-connection flow, saved cards open SSH.
        let connections = self.connections.clone();
        let left_panel = self.left_panel.clone();
        let on_new_ssh: ClickHandler =
            Rc::new(cx.listener(move |this, _ev: &ClickEvent, window, cx| {
                this.left_visible = true;
                this.refresh_connections();
                left_panel.update(cx, |lp, cx| {
                    lp.select_tab(crate::app::layout::LeftTab::Servers, cx);
                });
                this.open_add_connection(window, cx);
                cx.notify();
            }));
        let on_open_terminal: ClickHandler =
            Rc::new(cx.listener(|this, _ev: &ClickEvent, _, cx| this.open_terminal_tab(cx)));
        let on_open_recent = Rc::new(cx.listener(|this, idx: &usize, _, cx| {
            this.open_ssh_terminal(*idx, cx);
        }));

        div()
            .size_full()
            .bg(t.color.bg_canvas)
            .flex()
            .items_center()
            .justify_center()
            .child(WelcomeView::new(
                connections,
                on_new_ssh,
                on_open_terminal,
                on_open_recent,
            ))
            .into_any_element()
    }
}

// ─────────────────────────────────────────────────────────
// Terminal tab bar
// ─────────────────────────────────────────────────────────

fn render_terminal_tab_bar(
    t: &crate::theme::Theme,
    active: usize,
    count: usize,
    scroll_handle: &ScrollHandle,
    cx: &mut Context<PierApp>,
) -> impl IntoElement {
    // Tabs container: horizontally scrollable so 20+ terminals don't
    // push the [+] button (and, eventually, the window chrome) off
    // the right edge. Wrapped in a `flex_1 min_w_0` cell so the
    // scrollable steals from the rest of the row, not from [+].
    let mut tabs = div()
        .id("term-tab-scroll")
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        // Horizontal scroll backed by a persistent handle on PierApp.
        // `overflow_x_scroll` alone natively handles trackpad two-finger
        // horizontal gestures and shift+wheel on a traditional mouse.
        // Vertical-only mouse wheel is re-mapped below via
        // `on_scroll_wheel` — macOS & Windows mice without horizontal
        // capability should still be able to scroll the tabs.
        .overflow_x_scroll()
        .track_scroll(scroll_handle);

    for idx in 0..count {
        let is_active = idx == active;
        let label: SharedString = t!("App.Shell.terminal_tab", index = idx + 1).into();
        let tab_id: SharedString = format!("term-tab-{idx}").into();
        let close_id: SharedString = format!("term-close-{idx}").into();

        let on_select = cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.activate_terminal_tab(idx, cx);
        });
        let on_close = cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.close_terminal_tab(idx, cx);
        });
        let on_right_click = cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
            this.open_tab_context_menu(idx, ev.position, cx);
            cx.stop_propagation();
        });

        let mut tab = div()
            .id(gpui::ElementId::Name(tab_id))
            .h(BUTTON_SM_H)
            .px(SP_2)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1_5)
            .rounded(RADIUS_SM)
            .text_size(SIZE_CAPTION)
            .font_weight(WEIGHT_MEDIUM)
            .text_color(if is_active {
                t.color.text_primary
            } else {
                t.color.text_secondary
            })
            .cursor_pointer()
            .hover(|s| s.bg(t.color.bg_hover))
            .on_click(on_select)
            .on_mouse_down(MouseButton::Right, on_right_click)
            .child(
                UiIcon::new(IconName::SquareTerminal)
                    .size(GLYPH_SM)
                    .text_color(if is_active {
                        t.color.text_primary
                    } else {
                        t.color.text_secondary
                    }),
            )
            .child(label)
            .child(
                div()
                    .id(gpui::ElementId::Name(close_id))
                    .w(ICON_SM) // 14px — tight hit target inside tab
                    .h(ICON_SM)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(RADIUS_SM)
                    .text_color(t.color.text_tertiary)
                    .hover(|s| s.bg(t.color.bg_active).text_color(t.color.text_primary))
                    .on_click(on_close)
                    .child(
                        UiIcon::new(IconName::Close)
                            .size(GLYPH_XS)
                            .text_color(t.color.text_tertiary),
                    ),
            );

        if is_active {
            tab = tab.bg(t.color.bg_surface);
        }
        tabs = tabs.child(tab);
    }

    // Inline "+" at end-of-row — same chooser as the toolbar [+].
    let on_new = cx.listener(|this, _: &ClickEvent, window, cx| {
        let weak = cx.entity().downgrade();
        let connections = this.connections.clone();
        crate::views::new_tab_chooser::open(window, cx, weak, connections);
    });
    let plus_button = div()
        .id("term-tab-plus")
        .flex_none()
        .w(BUTTON_SM_H)
        .h(BUTTON_SM_H)
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .text_color(t.color.text_secondary)
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(on_new)
        .child(
            UiIcon::new(IconName::Plus)
                .size(GLYPH_SM)
                .text_color(t.color.text_secondary),
        );

    // Make sure the active tab is always visible after selection —
    // GPUI's `ScrollHandle::scroll_to_item` defers the actual scroll
    // to prepaint, so calling it every render is idempotent.
    scroll_handle.scroll_to_item(active);

    // `overflow_x_scroll` already handles both trackpad horizontal
    // gestures AND vertical mouse-wheel delta (GPUI's native handler
    // at div.rs:2427 redirects `delta.y → delta.x` when only the
    // x-axis is scrollable and `restrict_scroll_to_axis` is false,
    // which is the default). No custom `on_scroll_wheel` needed —
    // the earlier manual redirect was double-applying the delta.

    div()
        .h(ROW_MD_H)
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .bg(t.color.bg_panel)
        .border_b_1()
        .border_color(t.color.border_subtle)
        .child(div().flex_1().min_w(px(0.0)).overflow_hidden().child(tabs))
        .child(plus_button)
}

fn render_tab_context_menu(
    idx: usize,
    position: gpui::Point<Pixels>,
    total: usize,
    cx: &mut Context<PierApp>,
) -> impl IntoElement {
    use crate::components::{ContextMenu, ContextMenuItem};

    // Totals are passed in from the caller — *do not* call
    // `cx.entity().read(cx)` here. During `Render::render` the
    // entity has been "leased" out of the entity map, and any read
    // through a weak handle triggers a `double_lease_panic` (GPUI's
    // way of saying the entity is currently being rendered).
    let last = total.saturating_sub(1);
    let has_others = total > 1;
    let has_left = idx > 0;
    let has_right = idx < last;

    ContextMenu::new(
        gpui::ElementId::Name(format!("tab-ctx-menu-{idx}").into()),
        position,
    )
    .item(
        ContextMenuItem::new(t!("App.Shell.Tabs.close")).on_click(cx.listener(
            move |this, _, _, cx| {
                this.close_terminal_tab(idx, cx);
                this.close_tab_context_menu(cx);
            },
        )),
    )
    .item(
        ContextMenuItem::new(t!("App.Shell.Tabs.close_others"))
            .disabled(!has_others)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.close_terminal_tabs_others(idx, cx);
                this.close_tab_context_menu(cx);
            })),
    )
    .item(
        ContextMenuItem::new(t!("App.Shell.Tabs.close_left"))
            .disabled(!has_left)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.close_terminal_tabs_left(idx, cx);
                this.close_tab_context_menu(cx);
            })),
    )
    .item(
        ContextMenuItem::new(t!("App.Shell.Tabs.close_right"))
            .disabled(!has_right)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.close_terminal_tabs_right(idx, cx);
                this.close_tab_context_menu(cx);
            })),
    )
    .on_dismiss(cx.listener(|this, _: &(), _window, cx| this.close_tab_context_menu(cx)))
}

/// Append `name` onto the SFTP-side `cwd` (which is a `PathBuf`
/// modeling a remote POSIX path). PathBuf's join works on POSIX
/// separators on every platform; just need to handle the cwd ==
/// `.` case so we get `./foo` rather than `foo` (matches what
/// `list_dir_blocking` returned).
fn join_remote_path(cwd: &std::path::Path, name: &str) -> String {
    let mut out = cwd.to_string_lossy().into_owned();
    if !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(name);
    out
}
