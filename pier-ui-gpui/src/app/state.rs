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
    Empty, Entity, EntityId, IntoElement, MouseButton, Pixels, Render, SharedString, Window,
};
use gpui_component::{input::InputState, Icon as UiIcon, IconName, PixelsExt as _, WindowExt as _};
use rust_i18n::t;
use std::collections::HashMap;

use pier_core::connections::ConnectionStore;
use pier_core::db_connections::{DbConnection, DbConnectionStore};
use pier_core::ssh::SshConfig;

use crate::app::layout::{
    RightMode, CENTER_PANEL_MIN_W, LEFT_PANEL_DEFAULT_W, LEFT_PANEL_MAX_W, LEFT_PANEL_MIN_W,
    RIGHT_PANEL_DEFAULT_W, RIGHT_PANEL_MAX_W, RIGHT_PANEL_MIN_W,
};
use crate::app::db_session::{
    run_connect as db_run_connect, run_execute as db_run_execute, run_list as db_run_list,
    DbSessionState,
};
use crate::app::git_session::{
    default_cwd as git_default_cwd, run_action as git_run_action, run_refresh as git_run_refresh,
    GitPendingAction, GitState,
};
use crate::app::route::DbKind;
use crate::app::ssh_session::{
    run_bootstrap, run_docker_command, run_docker_refresh, run_logs_start, run_monitor_refresh,
    run_refresh, run_tunnel, ServiceProbeStatus, SshSessionState,
};
use crate::app::{
    ActivationHandler, CloseActiveTab, NewTab, OpenSettings, ToggleLeftPanel, ToggleRightPanel,
};
use crate::components::{StatusKind, StatusPill};
use crate::data::ShellSnapshot;
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_CAPTION, WEIGHT_MEDIUM},
    ThemeMode,
};
use crate::views::left_panel_view::{
    icons as toolbar_icons, ActiveServerSessionSnapshot, LeftPanelView, ServerTunnelSnapshot,
    ServersSidebarSnapshot,
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
    snapshot: ShellSnapshot,
    connections: Vec<SshConfig>,
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
        let connections = ConnectionStore::load_default()
            .map(|s| s.connections)
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
        let git_stash_message_input = cx.new(|c| {
            InputState::new(window, c).placeholder(t!("App.Git.stash_placeholder"))
        });

        Self {
            left_visible: true,
            right_visible: true,
            left_panel_width: LEFT_PANEL_DEFAULT_W,
            right_panel_width: RIGHT_PANEL_DEFAULT_W,
            right_mode: RightMode::Markdown,
            pane_bounds: Bounds::default(),
            snapshot,
            connections,
            db_connections,
            db_sessions: HashMap::new(),
            db_query_inputs,
            terminals: Vec::new(),
            active_terminal: None,
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
        self.ensure_right_mode_ready(cx, true);
        cx.notify();
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
        self.connections = ConnectionStore::load_default()
            .map(|s| s.connections)
            .unwrap_or_default();
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
        let weak = cx.entity().downgrade();
        crate::views::edit_connection::open(
            window,
            cx,
            weak,
            crate::views::edit_connection::EditTarget::Add,
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
        let weak = cx.entity().downgrade();
        crate::views::edit_connection::open(
            window,
            cx,
            weak,
            crate::views::edit_connection::EditTarget::Edit { idx, original },
        );
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
        let title: SharedString =
            t!("App.Shell.DeleteConnection.title", name = conn.name.as_str()).into();
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

        let toolbar = self.render_toolbar(&t, cx);
        // LeftPanelView is its own Entity — embedding the entity instead of
        // re-building a RenderOnce here means PierApp re-renders DON'T
        // rebuild the file tree cache or filter inputs (Phase 9 perf win).
        let left_entity = self.left_panel.clone();
        let center = self.render_center(&t, cx);
        let right = self
            .right_visible
            .then(|| self.render_right(cx))
            .map(IntoElement::into_any_element);
        let statusbar = self.render_statusbar(&t);
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
            .font_family(t.font_ui.clone())
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
    }
}

impl PierApp {
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

    fn render_toolbar(&self, t: &crate::theme::Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let toggle_left_icon = if self.left_visible {
            toolbar_icons::TOGGLE_LEFT_OPEN
        } else {
            toolbar_icons::TOGGLE_LEFT_CLOSED
        };
        let toggle_right_icon = if self.right_visible {
            toolbar_icons::TOGGLE_RIGHT_OPEN
        } else {
            toolbar_icons::TOGGLE_RIGHT_CLOSED
        };
        let theme_icon = if t.mode == ThemeMode::Dark {
            toolbar_icons::SUN
        } else {
            toolbar_icons::MOON
        };

        div()
            .h(px(32.0))
            .px(SP_3)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .bg(t.color.bg_panel)
            .border_b_1()
            .border_color(t.color.border_subtle)
            .child(toolbar_icon_button(
                t,
                "tb-toggle-left",
                toggle_left_icon,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.left_visible = !this.left_visible;
                    this.clamp_panel_widths();
                    cx.notify();
                }),
            ))
            .child(
                div().flex_1().min_w(px(0.0)).overflow_hidden().child(
                    div()
                        .w_full()
                        .text_size(SIZE_CAPTION)
                        .font_family(t.font_mono.clone())
                        .text_color(t.color.text_secondary)
                        .child(self.snapshot.workspace_path.clone()),
                ),
            )
            .child(toolbar_icon_button(
                t,
                "tb-new-tab",
                toolbar_icons::NEW_TAB,
                cx.listener(|this, _: &ClickEvent, window, cx| {
                    log::info!("toolbar: open new-tab chooser");
                    let weak = cx.entity().downgrade();
                    let connections = this.connections.clone();
                    crate::views::new_tab_chooser::open(window, cx, weak, connections);
                }),
            ))
            .child(toolbar_icon_button(
                t,
                "tb-open-settings",
                IconName::Settings,
                |_: &ClickEvent, window, app| {
                    log::info!("toolbar: open settings dialog");
                    crate::views::settings_dialog::open(window, app);
                },
            ))
            .child(toolbar_icon_button(
                t,
                "tb-toggle-right",
                toggle_right_icon,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.right_visible = !this.right_visible;
                    this.clamp_panel_widths();
                    cx.notify();
                }),
            ))
            .child(toolbar_icon_button(
                t,
                "tb-toggle-theme",
                theme_icon,
                |_: &ClickEvent, _, app| {
                    crate::theme::toggle(app);
                    crate::ui_kit::sync_theme(app);
                },
            ))
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
        cx.spawn(move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let background = cx.background_executor().clone();
            let mut async_cx = cx.clone();
            async move {
                let result = background.spawn(async move { db_run_connect(request) }).await;
                let _ = session.update(&mut async_cx, |state, _| {
                    state.apply_connect_result(result);
                });
            }
        })
        .detach();
    }

    /// Fetch the database list for the session's active client.
    pub(crate) fn schedule_db_list_databases(
        &mut self,
        kind: DbKind,
        cx: &mut Context<Self>,
    ) {
        let session = self.db_session_for(kind, cx);
        let Some(request) = session.update(cx, |state, _| state.begin_list_databases()) else {
            return;
        };

        cx.spawn(move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let background = cx.background_executor().clone();
            let mut async_cx = cx.clone();
            async move {
                let result = background.spawn(async move { db_run_list(request) }).await;
                let _ = session.update(&mut async_cx, |state, _| {
                    state.apply_list_result(result);
                });
            }
        })
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
        let Some(request) = session.update(cx, |state, _| state.begin_list_tables(database))
        else {
            return;
        };

        cx.notify();
        cx.spawn(move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let background = cx.background_executor().clone();
            let mut async_cx = cx.clone();
            async move {
                let result = background.spawn(async move { db_run_list(request) }).await;
                let _ = session.update(&mut async_cx, |state, _| {
                    state.apply_list_result(result);
                });
            }
        })
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
        cx.spawn(move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let background = cx.background_executor().clone();
            let mut async_cx = cx.clone();
            async move {
                let result = background.spawn(async move { db_run_execute(request) }).await;
                let _ = session.update(&mut async_cx, |state, _| {
                    state.apply_execute_result(result);
                });
            }
        })
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
        cx.spawn(move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let background = cx.background_executor().clone();
            let mut async_cx = cx.clone();
            async move {
                let result = background.spawn(async move { git_run_refresh(request) }).await;
                let _ = state.update(&mut async_cx, |s, cx| {
                    s.apply_refresh_result(result);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn schedule_git_action(&mut self, action: GitPendingAction, cx: &mut Context<Self>) {
        let state = self.git_state.clone();
        let Some(request) = state.update(cx, |s, _| s.begin_action(action)) else {
            return;
        };
        cx.notify();
        let this = cx.entity().downgrade();
        cx.spawn(move |_this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let background = cx.background_executor().clone();
            let mut async_cx = cx.clone();
            async move {
                let result = background.spawn(async move { git_run_action(request) }).await;
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
        })
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

fn toolbar_icon_button(
    t: &crate::theme::Theme,
    id: &'static str,
    icon: IconName,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(28.0))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .bg(t.color.bg_surface)
        .border_1()
        .border_color(t.color.border_default)
        .text_color(t.color.text_secondary)
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover).text_color(t.color.text_primary))
        .on_mouse_down(MouseButton::Left, |_, window, _| {
            window.prevent_default();
        })
        .on_click(on_click)
        .child(
            UiIcon::new(icon)
                .size(px(14.0))
                .text_color(t.color.text_primary),
        )
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
        RightPanel::new(
            self.right_mode,
            current_markdown,
            self.active_session.clone(),
            self.logs_command_input.clone(),
            cx.entity().downgrade(),
            on_sftp_navigate,
            on_sftp_go_up,
            on_docker_refresh,
            on_docker_action,
            on_logs_action,
            on_select_mode,
        )
    }

    fn render_statusbar(&self, t: &crate::theme::Theme) -> impl IntoElement {
        let term_count = self.terminals.len();
        let active_label: SharedString = match self.active_terminal {
            Some(i) if i < term_count => {
                t!("App.Shell.terminal_position", current = i + 1, total = term_count).into()
            }
            _ if term_count == 0 => t!("App.Shell.no_terminal").into(),
            _ => t!("App.Shell.no_active_tab").into(),
        };
        let mode_label: SharedString =
            t!("App.Shell.right_mode", mode = self.right_mode.label().as_ref()).into();
        let theme_label: SharedString = if t.mode == ThemeMode::Dark {
            t!("App.Shell.theme_dark").into()
        } else {
            t!("App.Shell.theme_light").into()
        };

        div()
            .h(px(22.0))
            .px(SP_3)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .bg(t.color.bg_panel)
            .border_t_1()
            .border_color(t.color.border_subtle)
            .child(StatusPill::new(
                active_label,
                if term_count == 0 {
                    StatusKind::Warning
                } else {
                    StatusKind::Success
                },
            ))
            .child(StatusPill::new(mode_label, StatusKind::Info))
            .child(StatusPill::new(theme_label, StatusKind::Success))
            .child(div().flex_1())
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .text_color(t.color.text_tertiary)
                    .child(SharedString::from(
                        t!("App.Shell.saved_connections_count", count = self.connections.len())
                            .to_string(),
                    )),
            )
    }

    fn render_center(&mut self, t: &crate::theme::Theme, cx: &mut Context<Self>) -> AnyElement {
        if let Some(active) = self.active_terminal {
            let tab_count = self.terminals.len();
            let active_term = self.terminals[active].clone();

            let tab_bar = render_terminal_tab_bar(t, active, tab_count, cx);

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
    cx: &mut Context<PierApp>,
) -> impl IntoElement {
    let mut row = div()
        .h(px(28.0))
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .bg(t.color.bg_panel)
        .border_b_1()
        .border_color(t.color.border_subtle);

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

        let mut tab = div()
            .id(gpui::ElementId::Name(tab_id))
            .h(px(22.0))
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
            .child(
                UiIcon::new(IconName::SquareTerminal)
                    .size(px(12.0))
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
                    .w(px(14.0))
                    .h(px(14.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(2.0))
                    .text_color(t.color.text_tertiary)
                    .hover(|s| s.bg(t.color.bg_active).text_color(t.color.text_primary))
                    .on_click(on_close)
                    .child(
                        UiIcon::new(IconName::Close)
                            .size(px(10.0))
                            .text_color(t.color.text_tertiary),
                    ),
            );

        if is_active {
            tab = tab.bg(t.color.bg_surface);
        }
        row = row.child(tab);
    }

    // Inline "+" at end-of-row — same chooser as the toolbar [+].
    let on_new = cx.listener(|this, _: &ClickEvent, window, cx| {
        let weak = cx.entity().downgrade();
        let connections = this.connections.clone();
        crate::views::new_tab_chooser::open(window, cx, weak, connections);
    });
    row.child(
        div()
            .id("term-tab-plus")
            .w(px(22.0))
            .h(px(22.0))
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
                    .size(px(12.0))
                    .text_color(t.color.text_secondary),
            ),
    )
}
