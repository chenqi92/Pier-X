// Pier-X GPUI spike — shell chrome, modelled on the React/Tauri shell.
//
// Layout mirrors the web version (see docs/PRODUCT-SPEC + pier-x-copy/screens/b1.png):
//   ┌───────────────────────── TopBar ─────────────────────────┐
//   │ Sidebar │           TabBar (center + right)              │
//   │ (left)  ├───────────────┬───────────────┬───────────────┤
//   │         │    Center     │  RightPanel   │  ToolStrip(R)  │
//   ├─────────┴───────────────┴───────────────┴───────────────┤
//   │                       StatusBar                          │
//   └──────────────────────────────────────────────────────────┘
// Interactions wired: switch/close tabs, switch right tool, Files/Servers
// sidebar toggle, connection-row selection, collapse right panel — all native
// GPUI state on the Shell entity. The center is the real TerminalView.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use futures::channel::{mpsc, oneshot};
use futures::StreamExt;
use gpui::prelude::*;
use gpui::{
    deferred, div, px, svg, AnyElement, Context, Entity, FocusHandle, Focusable, FontWeight, Hsla,
    InteractiveElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, SharedString, Svg, Window,
};
use gpui_component::{h_flex, v_flex, TitleBar};

use pier_core::ssh::{AuthMethod, HostKeyDecision, SshConfig};

use crate::data::{self, ConnRow, DetectedService, FileEntry, MonStat, ServiceStatus};
use crate::dialogs::{
    BroadcastDialog, BroadcastTarget, DialogEvent, EgressDialog, HostKeyDialog, HostKeyEvent,
    HostsHealthDialog, TunnelDialog,
};
use crate::git_panel::GitPanelView;
use crate::i18n::{self, Lang};
use crate::panels::PanelViews;
use crate::settings::SettingsView;
use crate::terminal::TerminalView;
use crate::theme::Theme;
use crate::ui;

/// A bundled lucide SVG, sized and tinted. `name` is the file stem under
/// `assets/icons/` (see src/assets.rs); the glyph picks up `color` because the
/// SVGs paint with `currentColor`.
fn icon(name: &str, sz: Pixels, color: Hsla) -> Svg {
    svg()
        .flex_none()
        .w(sz)
        .h(sz)
        .path(SharedString::from(format!("icons/{name}.svg")))
        .text_color(color)
}

/// Localized label for a tool-rail entry, keyed off its stable English name
/// (`"DOCKER"` → `tool.docker`). Brand/protocol names have no `zh` override
/// and stay in English; see [`crate::i18n`].
fn tool_label(name: &str) -> SharedString {
    i18n::t(&format!("tool.{}", name.to_ascii_lowercase()))
}

#[derive(Clone, Copy, PartialEq)]
pub enum Svc {
    Markdown,
    Git,
    Monitor,
    Firewall,
    Sftp,
    Log,
    Search,
    Docker,
    Mysql,
    Postgres,
    Redis,
    Sqlite,
    Webserver,
    Software,
}

/// (service, icon stem, full name, category index).
const TOOLS: &[(Svc, &str, &str, u8)] = &[
    (Svc::Markdown, "file-text", "MARKDOWN", 0),
    (Svc::Git, "git-branch", "GIT", 0),
    (Svc::Monitor, "activity", "MONITOR", 1),
    (Svc::Firewall, "shield", "FIREWALL", 1),
    (Svc::Sftp, "folder", "SFTP", 2),
    (Svc::Log, "scroll-text", "LOGS", 2),
    (Svc::Search, "search", "SEARCH", 2),
    (Svc::Docker, "container", "DOCKER", 3),
    (Svc::Mysql, "database", "MYSQL", 4),
    (Svc::Postgres, "database", "POSTGRES", 4),
    (Svc::Redis, "database", "REDIS", 4),
    (Svc::Sqlite, "database", "SQLITE", 4),
    (Svc::Webserver, "server", "WEBSERVER", 5),
    (Svc::Software, "package", "SOFTWARE", 5),
];

// Db/Markdown are reserved for future tab types (DB consoles, markdown tabs).
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq)]
enum TabKind {
    Local,
    Ssh,
    Db,
    Markdown,
}

struct Tab {
    title: String,
    kind: TabKind,
    /// Optional color label, an index into [`Shell::tab_palette`].
    color: Option<usize>,
    /// Each tab owns its own terminal session; dropping the tab drops the
    /// entity, which drops PierTerminal and closes the PTY.
    terminal: Entity<TerminalView>,
}

/// Drag payload identifying the tab being dragged (its index). A
/// newtype so drop targets only accept tab drags, not other usizes.
#[derive(Clone, Copy)]
struct TabDrag(usize);

/// The floating chip rendered under the cursor while a tab is dragged.
struct TabDragPreview {
    title: String,
    theme: Theme,
}

impl Render for TabDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        div()
            .px(t.sp3)
            .py(px(4.0))
            .rounded(t.radius_sm)
            .bg(t.elev)
            .border_1()
            .border_color(t.accent)
            .font_family(t.sans.clone())
            .text_size(t.fs_ui)
            .text_color(t.ink)
            .child(self.title.clone())
    }
}

pub struct Shell {
    theme: Theme,
    tabs: Vec<Tab>,
    active_tab: usize,
    active_tool: usize,
    show_servers: bool,
    selected_conn: usize,
    right_collapsed: bool,
    // Real data loaded from pier-core / the local working dir.
    cwd: PathBuf,
    cwd_label: String,
    files: Vec<FileEntry>,
    conns: Vec<ConnRow>,
    /// The Git panel's independent view (hosted by `right_panel`; owns all git
    /// state and is fed the browse cwd via `set_cwd`).
    git_panel_view: Entity<GitPanelView>,
    mon: Option<MonStat>,
    panels: PanelViews,
    /// The Settings overlay's independent view (hosted in overlay_layer).
    settings_view: Entity<SettingsView>,
    /// The four overlay dialogs (broadcast/egress/port-forward/hosts-health),
    /// each an independent view fed its data on open and hosted in
    /// overlay_layer. Constructed once; each emits [`DialogEvent::Close`] to
    /// dismiss the overlay (subscribed in `Shell::new`).
    broadcast_view: Entity<BroadcastDialog>,
    egress_view: Entity<EgressDialog>,
    tunnel_view: Entity<TunnelDialog>,
    hosts_health_view: Entity<HostsHealthDialog>,
    /// Which top-bar menu is open (index into MENUS), if any.
    open_menu: Option<usize>,
    /// Active centered overlay (Settings / command palette), if any.
    overlay: Overlay,
    /// User-dragged widths overriding the defaults; None = theme default.
    sidebar_w: Option<Pixels>,
    right_w: Option<Pixels>,
    /// The divider currently being dragged, if any.
    dragging: Option<DragTarget>,
    /// Command-palette filter text + its focus handle for keyboard input.
    palette_query: String,
    palette_focus: FocusHandle,
    /// Open tab context menu: (window position, tab index).
    tab_menu: Option<(Point<Pixels>, usize)>,
    /// Files sidebar filter text + its focus handle.
    file_filter: String,
    file_focus: FocusHandle,
    /// New Connection form: [name, host, port, user, secret, group] +
    /// focused field + focus. `secret` is the password or key path,
    /// depending on `conn_auth`.
    conn_form: [String; CONN_FIELD_COUNT],
    conn_field: usize,
    conn_focus: FocusHandle,
    /// Selected auth method for the form's secret field.
    conn_auth: ConnAuthKind,
    /// Last Test-connection result line (latency / error), if any.
    conn_test: Option<String>,
    /// Error from the last add-connection attempt.
    conn_error: Option<String>,
    /// When the New Connection overlay is editing an existing saved
    /// connection, the store index being edited; `None` = add a new one.
    conn_edit: Option<usize>,
    /// The original [`AuthMethod`] of the connection being edited, kept so a
    /// blank/unchanged secret field preserves the saved credential
    /// (`KeychainPassword.credential_id`, a key's `passphrase_credential_id`,
    /// or a stored `DirectPassword`) instead of overwriting it with an empty
    /// one. `None` when adding a new connection.
    conn_orig_auth: Option<AuthMethod>,
    /// Servers sidebar filter text + its focus handle.
    conn_search: String,
    conn_search_focus: FocusHandle,
    /// Favorite connection names (persisted via `data::save_favorites`).
    conn_favorites: HashSet<String>,
    /// Store index of the connection row awaiting delete confirmation.
    conn_confirm_delete: Option<usize>,
    /// True while a background connection-health probe is in flight, so the
    /// periodic driver never stacks overlapping probes.
    health_probing: bool,
    /// Services detected per connected SSH host, keyed by its `user@host`
    /// identity (the SSH tab title). Keyed by connection identity rather than
    /// tab index so the cache survives tab reorder/close without going stale
    /// and same-host tabs share a result; drives the Servers sidebar chips.
    detected_services: HashMap<String, Vec<DetectedService>>,
    /// `user@host` keys whose service detection is currently in flight.
    detecting_services: HashSet<String>,
    /// The host-key trust dialog (hosted in `overlay_layer`), fed a request
    /// when a connect task hits an unknown / changed key.
    hostkey_view: Entity<HostKeyDialog>,
    /// Cloned into each new SSH tab so its connect task can ship a
    /// [`data::HostKeyPrompt`] to the receiver task spawned in `new`.
    hostkey_tx: mpsc::UnboundedSender<data::HostKeyPrompt>,
    /// Decision channel for the host-key prompt currently on screen, if any.
    /// Resolved by an Accept / Reject click; dropped (→ Reject) when the
    /// overlay is dismissed so the parked connect task never hangs.
    pending_hostkey: Option<oneshot::Sender<HostKeyDecision>>,
}

/// A draggable layout divider.
#[derive(Clone, Copy, PartialEq)]
enum DragTarget {
    Sidebar,
    Right,
}

/// A centered modal layer over the shell.
#[derive(Clone, Copy, PartialEq)]
enum Overlay {
    None,
    Settings,
    Palette,
    NewConn,
    Broadcast,
    Egress,
    PortForward,
    HostsHealth,
    HostKey,
}

/// Number of text fields in the connection form (name, host, port,
/// user, secret, group). The secret slot's meaning depends on the
/// selected [`ConnAuthKind`].
const CONN_FIELD_COUNT: usize = 6;

/// Index of the secret field (password / key path) in `conn_form`.
const CONN_SECRET: usize = 4;

/// Which authentication method the New Connection form is editing.
#[derive(Clone, Copy, PartialEq)]
enum ConnAuthKind {
    Password,
    Key,
    Agent,
}

/// A per-row action in the Servers sidebar (dispatched by store index).
#[derive(Clone, Copy)]
enum ConnAction {
    ToggleFavorite,
    Edit,
    AskDelete,
    ConfirmDelete,
    CancelDelete,
}

/// A shell-wide command, dispatched from menus, the command palette, and
/// title-bar buttons through `Shell::run`.
#[derive(Clone, Copy)]
enum Cmd {
    NewTerminal,
    ToggleTheme,
    ToggleRightPanel,
    SelectTool(usize),
    OpenSettings,
    OpenPalette,
    OpenNewConn,
    OpenBroadcast,
    OpenEgress,
    OpenPortForward,
    OpenHostsHealth,
    CloseOverlay,
    CloseTab,
    CloseTabAt(usize),
    CloseOthers(usize),
    CloseToLeft(usize),
    CloseToRight(usize),
    SetTabColor(usize, Option<usize>),
}

// Global actions bound to keyboard shortcuts in main.rs. Each maps to a Cmd.
gpui::actions!(
    pier_x,
    [CmdPalette, CmdNewTerminal, CmdCloseTab, CmdToggleTheme, CmdSettings]
);

/// Top-bar menus: (label-key, items). Each item is (text-key, command). The
/// stored strings are `i18n` keys resolved to display text at render time (and
/// reused verbatim as stable element ids).
const MENUS: &[(&str, &[(&str, Cmd)])] = &[
    (
        "menu.file",
        &[
            ("tab.new_terminal", Cmd::NewTerminal),
            ("menu.command_palette", Cmd::OpenPalette),
            ("set.title", Cmd::OpenSettings),
        ],
    ),
    ("menu.edit", &[("set.title", Cmd::OpenSettings)]),
    (
        "menu.view",
        &[
            ("menu.toggle_theme", Cmd::ToggleTheme),
            ("menu.toggle_right_panel", Cmd::ToggleRightPanel),
            ("menu.command_palette", Cmd::OpenPalette),
            ("dlg.host_health", Cmd::OpenHostsHealth),
            ("dlg.egress", Cmd::OpenEgress),
        ],
    ),
    (
        "menu.session",
        &[
            ("tab.new_terminal", Cmd::NewTerminal),
            ("dlg.broadcast", Cmd::OpenBroadcast),
            ("dlg.port_forward", Cmd::OpenPortForward),
        ],
    ),
    ("menu.help", &[("menu.about", Cmd::OpenSettings)]),
];

impl Shell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Restore persisted layout/state from the last session.
        let st = data::load_ui_state();
        i18n::set(Lang::from_code(&st.lang));
        data::set_smart_mode(st.smart_mode);
        if !st.dark {
            cx.set_global(Theme::light());
        }
        // Keep gpui-component's theme (TitleBar control-icon colours) in sync
        // with the restored app theme.
        gpui_component::Theme::change(
            if st.dark {
                gpui_component::ThemeMode::Dark
            } else {
                gpui_component::ThemeMode::Light
            },
            None,
            cx,
        );
        let cwd = if !st.cwd.is_empty() && std::path::Path::new(&st.cwd).is_dir() {
            PathBuf::from(&st.cwd)
        } else {
            data::current_dir()
        };
        let cwd_label = cwd.display().to_string();
        let files = data::list_dir(&cwd);
        let conns = data::load_connections();
        let git_panel_view = cx.new(|cx| GitPanelView::new(cwd.clone(), cx));
        let tab_title = cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| cwd_label.clone());
        let terminal = cx.new(|cx| TerminalView::new(cx));
        let panels = PanelViews::new(cx);
        let settings_view = cx.new(SettingsView::new);
        let broadcast_view = cx.new(BroadcastDialog::new);
        let egress_view = cx.new(EgressDialog::new);
        let tunnel_view = cx.new(TunnelDialog::new);
        let hosts_health_view = cx.new(HostsHealthDialog::new);
        let hostkey_view = cx.new(HostKeyDialog::new);
        // Background connect tasks send unknown / changed host keys over this
        // channel; the receiver task (started below) pops `hostkey_view` and
        // routes the user's verdict back. The shell keeps the sender to clone
        // into each new SSH tab.
        let (hostkey_tx, hostkey_rx) = mpsc::unbounded::<data::HostKeyPrompt>();
        // Esc / ✕ / scrim-click inside any dialog emits DialogEvent::Close;
        // dismiss the overlay from here (the scrim itself routes through
        // Cmd::CloseOverlay, so both paths converge on Overlay::None).
        for sub in [
            cx.subscribe(&broadcast_view, |this, _, _: &DialogEvent, cx| {
                this.overlay = Overlay::None;
                cx.notify();
            }),
            cx.subscribe(&egress_view, |this, _, _: &DialogEvent, cx| {
                this.overlay = Overlay::None;
                cx.notify();
            }),
            cx.subscribe(&tunnel_view, |this, _, _: &DialogEvent, cx| {
                this.overlay = Overlay::None;
                cx.notify();
            }),
            cx.subscribe(&hosts_health_view, |this, _, _: &DialogEvent, cx| {
                this.overlay = Overlay::None;
                cx.notify();
            }),
        ] {
            sub.detach();
        }
        // The host-key dialog reports Accept / Reject (not a plain Close): feed
        // the verdict to the parked connect task via the pending sender, then
        // dismiss the overlay.
        cx.subscribe(&hostkey_view, |this, _, ev: &HostKeyEvent, cx| {
            let HostKeyEvent::Decide(decision) = ev;
            if let Some(tx) = this.pending_hostkey.take() {
                let _ = tx.send(*decision);
            }
            this.overlay = Overlay::None;
            cx.notify();
        })
        .detach();
        Self::start_monitor(cx);
        Self::start_sidebar_tasks(cx);
        Self::start_hostkey_prompt(hostkey_rx, cx);
        Self {
            theme: if st.dark { Theme::dark() } else { Theme::light() },
            tabs: vec![Tab {
                title: tab_title,
                kind: TabKind::Local,
                color: None,
                terminal,
            }],
            active_tab: 0,
            active_tool: st.active_tool.min(TOOLS.len() - 1),
            show_servers: st.show_servers,
            selected_conn: 0,
            right_collapsed: st.right_collapsed,
            cwd,
            cwd_label,
            files,
            conns,
            git_panel_view,
            mon: None,
            panels,
            settings_view,
            broadcast_view,
            egress_view,
            tunnel_view,
            hosts_health_view,
            open_menu: None,
            overlay: Overlay::None,
            sidebar_w: st.sidebar_w.map(px),
            right_w: st.right_w.map(px),
            dragging: None,
            palette_query: String::new(),
            palette_focus: cx.focus_handle(),
            tab_menu: None,
            file_filter: String::new(),
            file_focus: cx.focus_handle(),
            conn_form: Default::default(),
            conn_field: 0,
            conn_focus: cx.focus_handle(),
            conn_auth: ConnAuthKind::Password,
            conn_test: None,
            conn_error: None,
            conn_edit: None,
            conn_orig_auth: None,
            conn_search: String::new(),
            conn_search_focus: cx.focus_handle(),
            conn_favorites: data::load_favorites(),
            conn_confirm_delete: None,
            health_probing: false,
            detected_services: HashMap::new(),
            detecting_services: HashSet::new(),
            hostkey_view,
            hostkey_tx,
            pending_hostkey: None,
        }
    }

    fn on_conn_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "tab" => {
                let n = CONN_FIELD_COUNT;
                let mut next = self.conn_field;
                loop {
                    next = if ks.modifiers.shift {
                        (next + n - 1) % n
                    } else {
                        (next + 1) % n
                    };
                    // The secret field has no input under Agent auth.
                    if !(self.conn_auth == ConnAuthKind::Agent && next == CONN_SECRET) {
                        break;
                    }
                }
                self.conn_field = next;
                cx.notify();
                return;
            }
            "enter" => {
                self.submit_conn(window, cx);
                return;
            }
            "escape" => {
                self.run(Cmd::CloseOverlay, window, cx);
                return;
            }
            "backspace" => {
                if self.conn_form[self.conn_field].pop().is_some() {
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        let m = &ks.modifiers;
        if m.control || m.alt || m.platform {
            return;
        }
        if let Some(kc) = &ks.key_char {
            if !kc.is_empty() && !kc.chars().any(|c| c.is_control()) {
                self.conn_form[self.conn_field].push_str(kc);
                cx.notify();
            }
        }
    }

    /// Build a validated [`SshConfig`] from the form, mapping the
    /// selected auth kind to an [`AuthMethod`]. Shared by submit + Test.
    fn build_conn_cfg(&self) -> Result<SshConfig, String> {
        let name = self.conn_form[0].trim();
        let host = self.conn_form[1].trim();
        let port_s = self.conn_form[2].trim();
        let user = self.conn_form[3].trim();
        if host.is_empty() || user.is_empty() {
            return Err(i18n::t("conn.host_user_required").to_string());
        }
        let port: u16 = if port_s.is_empty() {
            22
        } else {
            port_s
                .parse()
                .map_err(|_| i18n::t("conn.port_must_be_number").to_string())?
        };
        let secret = self.conn_form[CONN_SECRET].clone();
        // In edit mode an unchanged secret field reuses the saved `AuthMethod`
        // verbatim, so we never downgrade a stored credential — a keychain
        // `credential_id`, a key's `passphrase_credential_id`, or a saved
        // `DirectPassword` — to an empty one. Typing a new secret, or switching
        // auth kind, rebuilds from the form. For a new connection
        // (`conn_orig_auth == None`) this falls through to the form values.
        let auth = match self.conn_auth {
            ConnAuthKind::Password if secret.is_empty() => match &self.conn_orig_auth {
                Some(
                    a @ (AuthMethod::KeychainPassword { .. } | AuthMethod::DirectPassword { .. }),
                ) => a.clone(),
                _ => AuthMethod::KeychainPassword {
                    credential_id: String::new(),
                },
            },
            ConnAuthKind::Password => AuthMethod::DirectPassword { password: secret },
            ConnAuthKind::Key => {
                let path = secret.trim().to_string();
                match &self.conn_orig_auth {
                    Some(a @ AuthMethod::PublicKeyFile { private_key_path, .. })
                        if *private_key_path == path =>
                    {
                        a.clone()
                    }
                    _ => AuthMethod::PublicKeyFile {
                        private_key_path: path,
                        passphrase_credential_id: None,
                    },
                }
            }
            ConnAuthKind::Agent => AuthMethod::Agent,
        };
        let group = {
            let g = self.conn_form[5].trim();
            if g.is_empty() {
                None
            } else {
                Some(g.to_string())
            }
        };
        let label = if name.is_empty() { host } else { name };
        let mut cfg = SshConfig::new(label, host, user);
        cfg.port = port;
        cfg.auth = auth;
        cfg.group = group;
        Ok(cfg)
    }

    /// Validate the form, persist (add or update), and reload. In edit
    /// mode the existing config's databases / tags / egress are kept;
    /// only addressing + auth + group are rewritten.
    fn submit_conn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fresh = match self.build_conn_cfg() {
            Ok(c) => c,
            Err(e) => {
                self.conn_error = Some(e);
                cx.notify();
                return;
            }
        };
        let result = match self.conn_edit {
            Some(idx) => match data::connections_raw().into_iter().nth(idx) {
                Some(mut cfg) => {
                    cfg.name = fresh.name;
                    cfg.host = fresh.host;
                    cfg.user = fresh.user;
                    cfg.port = fresh.port;
                    cfg.auth = fresh.auth;
                    cfg.group = fresh.group;
                    data::update_connection(idx, cfg)
                }
                None => Err(i18n::t("conn.connection_gone").to_string()),
            },
            None => data::add_connection(fresh),
        };
        match result {
            Ok(()) => {
                self.conns = data::load_connections();
                self.probe_connections_async(cx);
                self.conn_error = None;
                self.conn_edit = None;
                self.conn_orig_auth = None;
                self.run(Cmd::CloseOverlay, window, cx);
            }
            Err(e) => {
                self.conn_error = Some(e);
                cx.notify();
            }
        }
    }

    /// Probe the form's connection in the background and report the
    /// round-trip latency or the connect error.
    fn test_conn(&mut self, cx: &mut Context<Self>) {
        let cfg = match self.build_conn_cfg() {
            Ok(c) => c,
            Err(e) => {
                self.conn_test = Some(e);
                cx.notify();
                return;
            }
        };
        self.conn_test = Some(i18n::t("conn.testing").to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let start = std::time::Instant::now();
                    data::connect_blocking(&cfg).map(|_session| start.elapsed())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.conn_test = Some(match res {
                    Ok(d) => i18n::tf("conn.connected_in", &[&d.as_millis().to_string()]),
                    Err(e) => e,
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Open the connection overlay pre-filled to edit store row `idx`.
    fn open_edit_conn(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cfg) = data::connections_raw().into_iter().nth(idx) else {
            return;
        };
        // The key *path* is safe to prefill (it isn't a secret); a stored
        // password is — leave the field blank and keep the saved credential
        // unless the user types a new one (see `build_conn_cfg`).
        let (auth, secret) = match &cfg.auth {
            AuthMethod::PublicKeyFile {
                private_key_path, ..
            } => (ConnAuthKind::Key, private_key_path.clone()),
            AuthMethod::Agent | AuthMethod::Auto | AuthMethod::AutoChain { .. } => {
                (ConnAuthKind::Agent, String::new())
            }
            AuthMethod::DirectPassword { .. } | AuthMethod::KeychainPassword { .. } => {
                (ConnAuthKind::Password, String::new())
            }
        };
        self.conn_form = [
            cfg.name.clone(),
            cfg.host.clone(),
            cfg.port.to_string(),
            cfg.user.clone(),
            secret,
            cfg.group.clone().unwrap_or_default(),
        ];
        self.conn_auth = auth;
        self.conn_field = 0;
        self.conn_test = None;
        self.conn_error = None;
        self.conn_edit = Some(idx);
        self.conn_orig_auth = Some(cfg.auth.clone());
        self.overlay = Overlay::NewConn;
        window.focus(&self.conn_focus, cx);
        cx.notify();
    }

    /// Dispatch a Servers-row action by store index.
    fn conn_action(
        &mut self,
        idx: usize,
        action: ConnAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            ConnAction::ToggleFavorite => {
                if let Some(c) = self.conns.get(idx) {
                    let name = c.name.clone();
                    if !self.conn_favorites.remove(&name) {
                        self.conn_favorites.insert(name);
                    }
                    data::save_favorites(&self.conn_favorites);
                }
            }
            ConnAction::Edit => self.open_edit_conn(idx, window, cx),
            ConnAction::AskDelete => self.conn_confirm_delete = Some(idx),
            ConnAction::CancelDelete => self.conn_confirm_delete = None,
            ConnAction::ConfirmDelete => {
                if data::remove_connection(idx).is_ok() {
                    self.conns = data::load_connections();
                    if self.selected_conn >= self.conns.len() {
                        self.selected_conn = self.conns.len().saturating_sub(1);
                    }
                    // The reload reset every dot to unprobed; refresh promptly.
                    self.probe_connections_async(cx);
                }
                self.conn_confirm_delete = None;
            }
        }
        cx.notify();
    }

    fn on_conn_search_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "backspace" => {
                if self.conn_search.pop().is_some() {
                    cx.notify();
                }
                return;
            }
            "escape" => {
                if !self.conn_search.is_empty() {
                    self.conn_search.clear();
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        let m = &ks.modifiers;
        if m.control || m.alt || m.platform {
            return;
        }
        if let Some(kc) = &ks.key_char {
            if !kc.is_empty() && !kc.chars().any(|c| c.is_control()) {
                self.conn_search.push_str(kc);
                cx.notify();
            }
        }
    }

    /// The palette's entries (icon, label, command), in display order. Labels
    /// are localized so the list and its substring filter both run on the text
    /// the user actually sees.
    fn palette_entries() -> Vec<(&'static str, SharedString, Cmd)> {
        let mut v = vec![
            ("plus", i18n::t("tab.new_terminal"), Cmd::NewTerminal),
            ("server", i18n::t("conn.new"), Cmd::OpenNewConn),
            ("square-terminal", i18n::t("dlg.broadcast"), Cmd::OpenBroadcast),
            ("network", i18n::t("dlg.port_forward"), Cmd::OpenPortForward),
            ("activity", i18n::t("dlg.host_health"), Cmd::OpenHostsHealth),
            ("shield", i18n::t("dlg.egress"), Cmd::OpenEgress),
            ("settings", i18n::t("set.title"), Cmd::OpenSettings),
        ];
        for (i, (_, glyph, name, _)) in TOOLS.iter().enumerate() {
            v.push((glyph, tool_label(name), Cmd::SelectTool(i)));
        }
        v
    }

    /// Entries matching the current palette query (case-insensitive substring).
    fn palette_matches(&self) -> Vec<(&'static str, SharedString, Cmd)> {
        let q = self.palette_query.trim().to_lowercase();
        Self::palette_entries()
            .into_iter()
            .filter(|(_, label, _)| q.is_empty() || label.to_lowercase().contains(&q))
            .collect()
    }

    fn on_palette_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "escape" => {
                if self.palette_query.is_empty() {
                    self.run(Cmd::CloseOverlay, window, cx);
                } else {
                    self.palette_query.clear();
                    cx.notify();
                }
                return;
            }
            "enter" => {
                if let Some((_, _, cmd)) = self.palette_matches().first().cloned() {
                    self.run(cmd, window, cx);
                }
                return;
            }
            "backspace" => {
                if self.palette_query.pop().is_some() {
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        let m = &ks.modifiers;
        if m.control || m.alt || m.platform {
            return;
        }
        if let Some(kc) = &ks.key_char {
            if !kc.is_empty() && !kc.chars().any(|c| c.is_control()) {
                self.palette_query.push_str(kc);
                cx.notify();
            }
        }
    }

    /// A vertical divider the user can drag to resize an adjacent panel.
    fn drag_handle(&self, cx: &mut Context<Self>, target: DragTarget) -> impl IntoElement {
        let t = &self.theme;
        let id = match target {
            DragTarget::Sidebar => "drag-sidebar",
            DragTarget::Right => "drag-right",
        };
        div()
            .id(id)
            .w(px(5.0))
            .h_full()
            .flex_none()
            .cursor_col_resize()
            .hover(|s| s.bg(t.accent_dim))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.dragging = Some(target);
                    cx.notify();
                }),
            )
    }

    /// Persist the current layout/state for the next launch.
    fn persist(&self, cx: &mut Context<Self>) {
        data::save_ui_state(&data::UiState {
            active_tool: self.active_tool,
            right_collapsed: self.right_collapsed,
            show_servers: self.show_servers,
            dark: cx.global::<Theme>().dark,
            sidebar_w: self.sidebar_w.map(f32::from),
            right_w: self.right_w.map(f32::from),
            cwd: self.cwd.display().to_string(),
            lang: i18n::current().code().to_string(),
            smart_mode: data::smart_mode(),
        });
    }

    /// Dispatch a shell-wide command from a menu / palette / title-bar button.
    fn run(&mut self, cmd: Cmd, window: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.tab_menu = None;
        match cmd {
            Cmd::NewTerminal => {
                self.overlay = Overlay::None;
                self.open_terminal_tab(cx);
                return;
            }
            Cmd::ToggleTheme => {
                let dark = cx.global::<Theme>().dark;
                cx.set_global(if dark { Theme::light() } else { Theme::dark() });
                gpui_component::Theme::change(
                    if dark {
                        gpui_component::ThemeMode::Light
                    } else {
                        gpui_component::ThemeMode::Dark
                    },
                    Some(window),
                    cx,
                );
                window.refresh();
                self.persist(cx);
                return;
            }
            Cmd::ToggleRightPanel => self.right_collapsed = !self.right_collapsed,
            Cmd::SelectTool(i) => {
                self.active_tool = i;
                self.right_collapsed = false;
                self.overlay = Overlay::None;
                if matches!(TOOLS[i].0, Svc::Monitor) {
                    self.mon = Some(data::monitor_snapshot());
                }
            }
            Cmd::OpenSettings => {
                self.settings_view.update(cx, |v, cx| v.reload(cx));
                self.overlay = Overlay::Settings;
            }
            Cmd::OpenPalette => {
                self.overlay = Overlay::Palette;
                self.palette_query.clear();
                window.focus(&self.palette_focus, cx);
            }
            Cmd::OpenNewConn => {
                self.overlay = Overlay::NewConn;
                self.conn_form = Default::default();
                self.conn_field = 0;
                self.conn_auth = ConnAuthKind::Password;
                self.conn_test = None;
                self.conn_error = None;
                self.conn_edit = None;
                self.conn_orig_auth = None;
                window.focus(&self.conn_focus, cx);
            }
            Cmd::OpenBroadcast => {
                // Every tab is a candidate; `live` is true only for tabs with a
                // real SSH session, so the dialog never targets a local PTY
                // (and a still-connecting SSH tab reads as not-live until ready).
                let mut targets = Vec::with_capacity(self.tabs.len());
                for (i, tab) in self.tabs.iter().enumerate() {
                    let live = tab.terminal.read(cx).session().is_some();
                    targets.push(BroadcastTarget {
                        tab_index: i,
                        title: tab.title.clone(),
                        terminal: tab.terminal.clone(),
                        live,
                    });
                }
                self.broadcast_view
                    .update(cx, |v, cx| v.set_targets(targets, cx));
                self.overlay = Overlay::Broadcast;
                let fh = self.broadcast_view.read(cx).focus_handle();
                window.focus(&fh, cx);
            }
            Cmd::OpenEgress => {
                self.egress_view.update(cx, |v, cx| v.reload(cx));
                self.overlay = Overlay::Egress;
                let fh = self.egress_view.read(cx).focus_handle();
                window.focus(&fh, cx);
            }
            Cmd::OpenPortForward => {
                // Clone the active tab's live session (if any) so the dialog can
                // open ssh -L forwards over the same multiplexed connection.
                let active = self.tabs.get(self.active_tab).and_then(|tab| {
                    tab.terminal
                        .read(cx)
                        .session()
                        .map(|s| (tab.title.clone(), s))
                });
                self.tunnel_view.update(cx, |v, cx| v.set_active(active, cx));
                self.overlay = Overlay::PortForward;
                let fh = self.tunnel_view.read(cx).focus_handle();
                window.focus(&fh, cx);
            }
            Cmd::OpenHostsHealth => {
                // Hand the dialog the live sessions from open SSH tabs, keyed by
                // `user@host` (the SSH tab title) so its deep probe can ride an
                // existing connection instead of dialing again.
                let mut sessions = HashMap::new();
                for tab in &self.tabs {
                    if matches!(tab.kind, TabKind::Ssh) {
                        if let Some(s) = tab.terminal.read(cx).session() {
                            sessions.insert(tab.title.clone(), s);
                        }
                    }
                }
                self.hosts_health_view
                    .update(cx, |v, cx| v.set_sessions(sessions, cx));
                self.overlay = Overlay::HostsHealth;
                let fh = self.hosts_health_view.read(cx).focus_handle();
                window.focus(&fh, cx);
            }
            Cmd::CloseOverlay => {
                // Dismissing the host-key dialog (scrim / Esc / ✕) is a Reject:
                // drop the pending sender so the parked connect task resolves to
                // Reject instead of hanging. No-op for the other overlays.
                self.pending_hostkey = None;
                self.overlay = Overlay::None;
            }
            Cmd::CloseTab => {
                if !self.tabs.is_empty() {
                    self.tabs.remove(self.active_tab);
                    if self.active_tab >= self.tabs.len() {
                        self.active_tab = self.tabs.len().saturating_sub(1);
                    }
                }
            }
            Cmd::CloseTabAt(i) => {
                if i < self.tabs.len() {
                    self.tabs.remove(i);
                    if self.active_tab >= self.tabs.len() {
                        self.active_tab = self.tabs.len().saturating_sub(1);
                    }
                }
            }
            Cmd::CloseOthers(i) => {
                if i < self.tabs.len() {
                    let keep = self.tabs.remove(i);
                    self.tabs.clear();
                    self.tabs.push(keep);
                    self.active_tab = 0;
                }
            }
            Cmd::CloseToLeft(i) => {
                if i > 0 && i < self.tabs.len() {
                    self.tabs.drain(0..i);
                    // The active tab either survived (shift its index left by
                    // `i`) or sat inside the closed range (fall to the new
                    // first tab, which is the old tab `i`).
                    self.active_tab = if self.active_tab < i {
                        0
                    } else {
                        self.active_tab - i
                    };
                }
            }
            Cmd::CloseToRight(i) => {
                if i + 1 < self.tabs.len() {
                    self.tabs.truncate(i + 1);
                    if self.active_tab > i {
                        self.active_tab = i;
                    }
                }
            }
            Cmd::SetTabColor(i, c) => {
                if let Some(tab) = self.tabs.get_mut(i) {
                    tab.color = c;
                }
            }
        }
        self.persist(cx);
        cx.notify();
    }

    /// Reorder a tab by drag: move the tab at `from` to `to`'s slot. The
    /// active tab is kept by identity (like the web TabBar, which tracks it
    /// by id), so dragging a background tab never steals focus from the
    /// foreground terminal — only dragging the active tab lets active follow.
    fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let was_active = self.active_tab;
        let tab = self.tabs.remove(from);
        let dest = if from < to { to - 1 } else { to };
        let dest = dest.min(self.tabs.len());
        self.tabs.insert(dest, tab);
        self.active_tab = if was_active == from {
            // Dragged the active tab itself: it stays active at its new slot.
            dest
        } else {
            // A different tab is active; remap its index through the same
            // remove(from) + insert(dest) shift so it tracks the same tab.
            let mut a = was_active;
            if a > from {
                a -= 1;
            }
            if a >= dest {
                a += 1;
            }
            a
        };
        cx.notify();
    }

    /// Eight tab-color swatches, drawn from the design tokens.
    fn tab_palette(t: &Theme) -> [Hsla; 8] {
        [
            t.info,
            t.pos,
            t.warn,
            t.neg,
            t.svc_log,
            t.svc_mysql,
            t.svc_postgres,
            t.svc_sftp,
        ]
    }

    /// Refresh the Monitor snapshot on an interval while the Monitor panel is
    /// the visible tool. Sampling is gated so we don't poll sysinfo when the
    /// panel is hidden.
    fn start_monitor(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let alive = this
                .update(cx, |this, cx| {
                    let showing = matches!(TOOLS[this.active_tool].0, Svc::Monitor)
                        && !this.right_collapsed;
                    if showing {
                        this.mon = Some(data::monitor_snapshot());
                        cx.notify();
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        })
        .detach();
    }

    /// TCP-probe every saved connection off the render path and write the
    /// result back onto `conns[i].online` by store index. Skipped while a probe
    /// is already in flight or there are no connections.
    fn probe_connections_async(&mut self, cx: &mut Context<Self>) {
        if self.health_probing || self.conns.is_empty() {
            return;
        }
        self.health_probing = true;
        cx.spawn(async move |this, cx| {
            let results = cx
                .background_executor()
                .spawn(async move { data::probe_connections(2000) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.health_probing = false;
                for (i, online) in results {
                    if let Some(c) = this.conns.get_mut(i) {
                        c.online = Some(online);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// When the active tab is a connected SSH shell, detect the services on its
    /// host (off the render path, over a clone of the live session) and cache
    /// them by connection identity (`user@host`) for the sidebar chips. Runs
    /// once per host: a cache hit or an in-flight probe short-circuits, and a
    /// not-yet-connected session simply returns so a later tick can retry.
    fn detect_services_async(&mut self, cx: &mut Context<Self>) {
        let idx = self.active_tab;
        let (kind, key) = match self.tabs.get(idx) {
            Some(tab) => (tab.kind, tab.title.clone()),
            None => return,
        };
        if !matches!(kind, TabKind::Ssh) {
            return;
        }
        if self.detected_services.contains_key(&key) || self.detecting_services.contains(&key) {
            return;
        }
        let Some(session) = self.tabs[idx].terminal.read(cx).session() else {
            return;
        };
        self.detecting_services.insert(key.clone());
        cx.spawn(async move |this, cx| {
            let services = cx
                .background_executor()
                .spawn(async move { data::detect_services(&session) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.detecting_services.remove(&key);
                this.detected_services.insert(key, services);
                // Drop cache entries for hosts no longer open in any tab.
                let live: HashSet<String> = this
                    .tabs
                    .iter()
                    .filter(|t| matches!(t.kind, TabKind::Ssh))
                    .map(|t| t.title.clone())
                    .collect();
                this.detected_services.retain(|k, _| live.contains(k));
                cx.notify();
            });
        })
        .detach();
    }

    /// Background driver for the Servers sidebar's live bits: it TCP-probes the
    /// saved connections for their health dots and, once the active SSH tab's
    /// shell session is up, detects the services on that host for the per-row
    /// chips. Both checks are cheap each tick and only spawn real network work
    /// when there is something new to do (guarded by `health_probing` /
    /// `detecting_services`). Health probing is throttled to ~20s and gated on
    /// the Servers panel / welcome being visible, mirroring how `start_monitor`
    /// only samples while the Monitor panel is shown.
    fn start_sidebar_tasks(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut tick: u32 = 0;
            loop {
                let alive = this
                    .update(cx, |this, cx| {
                        if this.show_servers {
                            this.detect_services_async(cx);
                        }
                        if tick % 20 == 0 && (this.show_servers || this.tabs.is_empty()) {
                            this.probe_connections_async(cx);
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                tick = tick.wrapping_add(1);
                cx.background_executor()
                    .timer(Duration::from_millis(1000))
                    .await;
            }
        })
        .detach();
    }

    /// Receive host-key prompts from background connect tasks and drive the
    /// [`HostKeyDialog`] overlay. Handles one prompt at a time: it parks on the
    /// per-prompt UI channel before pulling the next request, so concurrent SSH
    /// connects queue rather than clobbering each other's decision sender. A
    /// dismissed dialog (dropped `pending_hostkey`) and a dropped shell both
    /// resolve to Reject, so a connect task is never left hung.
    fn start_hostkey_prompt(
        mut rx: mpsc::UnboundedReceiver<data::HostKeyPrompt>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Some((req, decision_tx)) = rx.next().await {
                // Per-prompt UI channel: the Accept/Reject subscription (or a
                // CloseOverlay drop) resolves `ui_rx`, which we forward to the
                // connect task's `decision_tx`.
                let (ui_tx, ui_rx) = oneshot::channel::<HostKeyDecision>();
                let shown = this
                    .update(cx, |this, cx| {
                        this.pending_hostkey = Some(ui_tx);
                        this.hostkey_view.update(cx, |v, cx| v.set_request(req, cx));
                        this.overlay = Overlay::HostKey;
                        cx.notify();
                    })
                    .is_ok();
                if !shown {
                    // Shell gone — reject so the connect task unblocks.
                    let _ = decision_tx.send(HostKeyDecision::Reject);
                    break;
                }
                // Park here (UI thread free to paint + collect the click) until
                // the user decides; a dropped ui_tx (dismissed dialog) → Reject.
                let decision = ui_rx.await.unwrap_or(HostKeyDecision::Reject);
                let _ = decision_tx.send(decision);
                let _ = this.update(cx, |this, cx| {
                    this.pending_hostkey = None;
                    if this.overlay == Overlay::HostKey {
                        this.overlay = Overlay::None;
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Open a fresh local terminal tab and make it active.
    fn open_terminal_tab(&mut self, cx: &mut Context<Self>) {
        let terminal = cx.new(|cx| TerminalView::new(cx));
        self.tabs.push(Tab {
            title: "pwsh".to_string(),
            kind: TabKind::Local,
            color: None,
            terminal,
        });
        self.active_tab = self.tabs.len() - 1;
        cx.notify();
    }

    fn svc_color(&self, s: Svc) -> Hsla {
        let t = &self.theme;
        match s {
            Svc::Markdown => t.svc_log,
            Svc::Git => t.info,
            Svc::Monitor => t.svc_monitor,
            Svc::Firewall => t.warn,
            Svc::Sftp => t.svc_sftp,
            Svc::Log => t.svc_log,
            Svc::Search => t.warn,
            Svc::Docker => t.svc_docker,
            Svc::Mysql => t.svc_mysql,
            Svc::Postgres => t.svc_postgres,
            Svc::Redis => t.svc_redis,
            Svc::Sqlite => t.svc_sftp,
            Svc::Webserver => t.pos,
            Svc::Software => t.svc_log,
        }
    }

    fn tab_icon(kind: TabKind) -> &'static str {
        match kind {
            TabKind::Local => "square-terminal",
            TabKind::Ssh => "terminal",
            TabKind::Db => "database",
            TabKind::Markdown => "file-text",
        }
    }

    // ── TitleBar (client-side window chrome) ─────────────────────
    /// A title-bar icon button that dispatches `cmd` on click.
    fn action_btn(&self, cx: &mut Context<Self>, name: &'static str, cmd: Cmd) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(SharedString::from(format!("act-{name}")))
            .flex()
            .items_center()
            .justify_center()
            .w(px(26.0))
            .h(px(26.0))
            .rounded(t.radius_sm)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| this.run(cmd, window, cx)),
            )
            .child(icon(name, px(15.0), t.ink_2))
    }

    /// A title-bar EN / 中 toggle that flips the interface language and persists
    /// it (also available in Settings → General).
    fn lang_btn(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let label = if i18n::current() == Lang::Zh { "中" } else { "EN" };
        div()
            .id("lang-toggle")
            .flex()
            .items_center()
            .justify_center()
            .h(px(26.0))
            .min_w(px(26.0))
            .px(t.sp1)
            .rounded(t.radius_sm)
            .cursor_pointer()
            .text_size(t.fs_ui)
            .text_color(t.ink_2)
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    let next = if i18n::current() == Lang::Zh {
                        Lang::En
                    } else {
                        Lang::Zh
                    };
                    i18n::set(next);
                    this.persist(cx);
                    window.refresh();
                    cx.notify();
                }),
            )
            .child(label)
    }

    /// A top-bar menu label plus its drop-down (deferred so it paints on top).
    fn menu_btn(&self, cx: &mut Context<Self>, idx: usize) -> impl IntoElement {
        let t = &self.theme;
        let (label, items) = MENUS[idx];
        let open = self.open_menu == Some(idx);
        div()
            .relative()
            .child(
                div()
                    .id(SharedString::from(format!("menu-{label}")))
                    .px(t.sp2)
                    .py(px(2.0))
                    .rounded(t.radius_sm)
                    .text_size(t.fs_ui)
                    .cursor_pointer()
                    .text_color(if open { t.ink } else { t.ink_2 })
                    .when(open, |d| d.bg(t.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.open_menu = if this.open_menu == Some(idx) {
                                None
                            } else {
                                Some(idx)
                            };
                            cx.notify();
                        }),
                    )
                    .child(i18n::t(label)),
            )
            .when(open, |d| {
                d.child(deferred(self.menu_dropdown(cx, idx, items)))
            })
    }

    fn menu_dropdown(
        &self,
        cx: &mut Context<Self>,
        idx: usize,
        items: &'static [(&'static str, Cmd)],
    ) -> impl IntoElement {
        let t = &self.theme;
        let mut col = v_flex()
            .id("menu-dd")
            .absolute()
            .top(t.titlebar_h)
            .left(px(0.0))
            .min_w(px(190.0))
            .py(t.sp1)
            .bg(t.elev)
            .border_1()
            .border_color(t.line_2)
            .rounded(t.radius_md)
            .on_mouse_down_out(cx.listener(|this, _, _w, cx| {
                this.open_menu = None;
                cx.notify();
            }));
        for (text, cmd) in items {
            let cmd = *cmd;
            let text = *text;
            col = col.child(
                div()
                    .id(SharedString::from(format!("mi-{idx}-{text}")))
                    .px(t.sp3)
                    .py(px(5.0))
                    .text_size(t.fs_ui)
                    .text_color(t.ink_2)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            this.run(cmd, window, cx)
                        }),
                    )
                    .child(i18n::t(text)),
            );
        }
        col
    }

    fn topbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let theme_icon = if t.dark { "moon" } else { "sun" };
        // gpui-component TitleBar handles drag + native min/max/close on the
        // right; we fill the draggable area with the menu bar and quick actions.
        TitleBar::new()
            .h(t.titlebar_h)
            .bg(t.surface)
            .border_color(t.line)
            .child(
                h_flex()
                    .items_center()
                    .w_full()
                    .h_full()
                    .gap(t.sp2)
                    .child(div().w(px(16.0)).h(px(16.0)).rounded(t.radius_sm).bg(t.accent))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink)
                            .child("Pier-X"),
                    )
                    .child(div().text_size(t.fs_sm).text_color(t.muted).child("0.7.2"))
                    .child(div().w(px(8.0)))
                    .child(self.menu_btn(cx, 0))
                    .child(self.menu_btn(cx, 1))
                    .child(self.menu_btn(cx, 2))
                    .child(self.menu_btn(cx, 3))
                    .child(self.menu_btn(cx, 4))
                    .child(div().flex_1())
                    .child(self.action_btn(cx, "command", Cmd::OpenPalette))
                    .child(self.action_btn(cx, "square-terminal", Cmd::OpenBroadcast))
                    .child(self.action_btn(cx, "network", Cmd::OpenPortForward))
                    .child(self.action_btn(cx, "plus", Cmd::NewTerminal))
                    .child(self.lang_btn(cx))
                    .child(self.action_btn(cx, theme_icon, Cmd::ToggleTheme))
                    .child(self.action_btn(cx, "settings", Cmd::OpenSettings)),
            )
    }

    // ── Sidebar ──────────────────────────────────────────────────
    fn sidebar_tab(
        &self,
        cx: &mut Context<Self>,
        label: &'static str,
        servers: bool,
    ) -> impl IntoElement {
        let t = &self.theme;
        let active = self.show_servers == servers;
        div()
            .id(SharedString::from(format!("sbtab-{label}")))
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .h(t.tabbar_h)
            .text_size(t.fs_ui)
            .text_color(if active { t.ink } else { t.muted })
            .when(active, |d| d.border_b_2().border_color(t.accent))
            .hover(|s| s.text_color(t.ink))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.show_servers = servers;
                    if servers {
                        // Refresh the health dots and (if an SSH tab is active)
                        // its service chips the moment the panel is revealed.
                        this.probe_connections_async(cx);
                        this.detect_services_async(cx);
                    }
                    this.persist(cx);
                    cx.notify();
                }),
            )
            .child(i18n::t(label))
    }

    fn section_label(&self, text: impl Into<SharedString>) -> impl IntoElement {
        let t = &self.theme;
        div()
            .px(t.sp3)
            .pt(t.sp3)
            .pb(t.sp1)
            .text_size(t.fs_sm)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.muted)
            .child(text.into())
    }

    /// Open a clicked file. Markdown files render in the Markdown panel; other
    /// types are ignored for now.
    fn open_file(&mut self, name: String, cx: &mut Context<Self>) {
        // Keep the Git panel pointed at the browse cwd (no-op when unchanged).
        let cwd = self.cwd.clone();
        self.git_panel_view.update(cx, |v, cx| v.set_cwd(cwd, cx));
        let lower = name.to_lowercase();
        if lower.ends_with(".md") || lower.ends_with(".markdown") {
            if let Some(i) = TOOLS.iter().position(|(s, _, _, _)| matches!(s, Svc::Markdown)) {
                self.active_tool = i;
                self.right_collapsed = false;
            }
            self.panels.open_markdown(self.cwd.join(&name), cx);
            cx.notify();
        }
    }

    /// Point the Files tree at a new directory and reload its contents + git.
    fn navigate_to(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.cwd_label = path.display().to_string();
        self.files = data::list_dir(&path);
        self.cwd = path;
        let cwd = self.cwd.clone();
        self.git_panel_view.update(cx, |v, cx| v.set_cwd(cwd, cx));
        self.persist(cx);
        cx.notify();
    }

    fn file_row(&self, cx: &mut Context<Self>, f: &FileEntry) -> impl IntoElement {
        let t = &self.theme;
        let glyph = if f.is_dir { "folder" } else { "file" };
        let glyph_color = if f.is_dir { t.accent } else { t.muted };
        let name = f.name.clone();
        let is_dir = f.is_dir;
        h_flex()
            .id(SharedString::from(format!("file-{}", f.name)))
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .text_color(t.ink_2)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .when(is_dir, |d| {
                let name = name.clone();
                d.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                        let target = this.cwd.join(&name);
                        this.navigate_to(target, cx);
                    }),
                )
            })
            .when(!is_dir, |d| {
                let name = name.clone();
                d.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                        this.open_file(name.clone(), cx)
                    }),
                )
            })
            .child(icon(glyph, px(14.0), glyph_color))
            .child(div().flex_1().min_w(px(0.0)).truncate().child(f.name.clone()))
            .child(
                div()
                    .w(px(40.0))
                    .flex_none()
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(f.age.clone()),
            )
            .child(
                div()
                    .w(px(56.0))
                    .flex_none()
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child(f.size.map(human_size).unwrap_or_default()),
            )
    }

    /// NAME / MOD / SIZE column header for the Files list.
    fn files_header(&self) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .h(px(22.0))
            .px(t.sp3)
            .text_size(t.fs_sm)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.muted)
            .child(div().w(px(14.0)))
            .child(div().flex_1().child(i18n::t("side.col_name")))
            .child(div().w(px(40.0)).flex_none().child(i18n::t("side.col_mod")))
            .child(div().w(px(56.0)).flex_none().child(i18n::t("side.col_size")))
    }

    /// Breadcrumb + home/up/refresh toolbar for the Files tree.
    fn files_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let nav = |key: &'static str, glyph: &'static str, cx: &mut Context<Self>, f: fn(&mut Self, &mut Context<Self>)| {
            div()
                .id(SharedString::from(format!("fnav-{key}")))
                .flex()
                .items_center()
                .justify_center()
                .w(px(22.0))
                .h(px(22.0))
                .rounded(t.radius_sm)
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _w, cx| f(this, cx)),
                )
                .child(icon(glyph, px(14.0), t.muted))
        };
        h_flex()
            .items_center()
            .gap(px(2.0))
            .px(t.sp2)
            .py(px(4.0))
            .child(nav("up", "chevron-up", cx, |this, cx| {
                if let Some(p) = this.cwd.parent().map(|p| p.to_path_buf()) {
                    this.navigate_to(p, cx);
                }
            }))
            .child(nav("home", "folder", cx, |this, cx| {
                this.navigate_to(data::current_dir(), cx);
            }))
            .child(nav("refresh", "redo-2", cx, |this, cx| {
                let cwd = this.cwd.clone();
                this.navigate_to(cwd, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .ml(t.sp1)
                    .truncate()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink_2)
                    .child(breadcrumb(&self.cwd)),
            )
    }

    fn on_files_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "backspace" => {
                if self.file_filter.pop().is_some() {
                    cx.notify();
                }
                return;
            }
            "escape" => {
                if !self.file_filter.is_empty() {
                    self.file_filter.clear();
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        let m = &ks.modifiers;
        if m.control || m.alt || m.platform {
            return;
        }
        if let Some(kc) = &ks.key_char {
            if !kc.is_empty() && !kc.chars().any(|c| c.is_control()) {
                self.file_filter.push_str(kc);
                cx.notify();
            }
        }
    }

    /// The "Filter files…" input row.
    fn files_filter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let empty = self.file_filter.is_empty();
        div()
            .track_focus(&self.file_focus)
            .key_context("FileFilter")
            .on_key_down(cx.listener(Self::on_files_key))
            .mx(t.sp3)
            .mb(t.sp1)
            .h(px(26.0))
            .px(t.sp2)
            .flex()
            .items_center()
            .gap(t.sp2)
            .rounded(t.radius_sm)
            .bg(t.panel_2)
            .border_1()
            .border_color(t.line_2)
            .child(icon("search", px(13.0), t.muted))
            .when(empty, |d| d.child(div().text_size(t.fs_sm).text_color(t.dim).child(i18n::t("side.filter_files"))))
            .when(!empty, |d| {
                d.child(div().flex_1().text_size(t.fs_sm).text_color(t.ink).child(self.file_filter.clone()))
            })
    }

    /// The ".." row that ascends to the parent directory.
    fn parent_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .id("file-up")
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .text_color(t.muted)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                    if let Some(parent) = this.cwd.parent().map(|p| p.to_path_buf()) {
                        this.navigate_to(parent, cx);
                    }
                }),
            )
            .child(icon("folder", px(14.0), t.muted))
            .child(div().flex_1().child(".."))
    }

    /// Open a new SSH terminal tab for saved connection `idx`.
    fn open_ssh_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(cfg) = data::connections_raw().into_iter().nth(idx) else {
            return;
        };
        let title = format!("{}@{}", cfg.user, cfg.host);
        let prompt_tx = self.hostkey_tx.clone();
        let terminal = cx.new(|cx| TerminalView::new_ssh(cx, cfg, prompt_tx));
        self.tabs.push(Tab {
            title,
            kind: TabKind::Ssh,
            color: None,
            terminal,
        });
        self.active_tab = self.tabs.len() - 1;
        cx.notify();
    }

    /// A small icon button in a Servers row, dispatching a ConnAction.
    fn conn_btn_icon(
        &self,
        cx: &mut Context<Self>,
        key: String,
        glyph: &'static str,
        color: Hsla,
        idx: usize,
        action: ConnAction,
    ) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(SharedString::from(format!("cab-{key}")))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(20.0))
            .h(px(20.0))
            .rounded(t.radius_sm)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    this.conn_action(idx, action, window, cx)
                }),
            )
            .child(icon(glyph, px(13.0), color))
    }

    /// A non-interactive auth-method glyph for a Servers row.
    fn conn_auth_badge(&self, auth: data::AuthKind) -> impl IntoElement {
        let t = &self.theme;
        let glyph = match auth {
            data::AuthKind::Password => "asterisk",
            data::AuthKind::Key => "file",
            data::AuthKind::Agent => "bot",
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(18.0))
            .h(px(18.0))
            .child(icon(glyph, px(12.0), t.dim))
    }

    /// Three-state health dot colour: unprobed grey, reachable green,
    /// unreachable red.
    fn conn_dot_color(&self, online: Option<bool>) -> Hsla {
        let t = &self.theme;
        match online {
            None => t.dim,
            Some(true) => t.pos,
            Some(false) => t.neg,
        }
    }

    fn conn_row(&self, cx: &mut Context<Self>, idx: usize, c: &ConnRow) -> impl IntoElement {
        let t = &self.theme;
        let selected = self.selected_conn == idx;
        let confirming = self.conn_confirm_delete == Some(idx);
        let fav = self.conn_favorites.contains(&c.name);
        let dot = self.conn_dot_color(c.online);
        let mut row = h_flex()
            .id(SharedString::from(format!("conn-{idx}")))
            .items_center()
            .gap(t.sp2)
            .h(px(42.0))
            .px(t.sp3)
            .when(selected, |d| d.bg(t.accent_dim))
            .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
            .child(div().w(px(7.0)).h(px(7.0)).flex_none().rounded_full().bg(dot))
            .child(
                // Clicking the name/addr region opens an SSH tab; the
                // action buttons are separate siblings so they don't
                // also trigger a connect.
                v_flex()
                    .id(SharedString::from(format!("conn-open-{idx}")))
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.selected_conn = idx;
                            this.open_ssh_tab(idx, cx);
                        }),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .text_color(if selected { t.ink } else { t.ink_2 })
                            .child(c.name.clone()),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(c.addr.clone()),
                    ),
            );
        if confirming {
            row = row
                .child(div().flex_none().text_size(t.fs_sm).text_color(t.neg).child(i18n::t("common.confirm_delete")))
                .child(self.conn_btn_icon(
                    cx,
                    format!("delyes-{idx}"),
                    "check",
                    t.neg,
                    idx,
                    ConnAction::ConfirmDelete,
                ))
                .child(self.conn_btn_icon(
                    cx,
                    format!("delno-{idx}"),
                    "close",
                    t.muted,
                    idx,
                    ConnAction::CancelDelete,
                ));
        } else {
            row = row
                .child(self.conn_auth_badge(c.auth))
                .child(self.conn_btn_icon(
                    cx,
                    format!("fav-{idx}"),
                    if fav { "star-fill" } else { "star" },
                    if fav { t.warn } else { t.muted },
                    idx,
                    ConnAction::ToggleFavorite,
                ))
                .child(self.conn_btn_icon(
                    cx,
                    format!("edit-{idx}"),
                    "settings-2",
                    t.muted,
                    idx,
                    ConnAction::Edit,
                ))
                .child(self.conn_btn_icon(
                    cx,
                    format!("del-{idx}"),
                    "delete",
                    t.muted,
                    idx,
                    ConnAction::AskDelete,
                ));
        }
        row
    }

    /// The "Search connections…" input row in the Servers sidebar.
    fn conn_search_box(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let empty = self.conn_search.is_empty();
        div()
            .track_focus(&self.conn_search_focus)
            .key_context("ConnSearch")
            .on_key_down(cx.listener(Self::on_conn_search_key))
            .mx(t.sp3)
            .mb(t.sp1)
            .h(px(26.0))
            .px(t.sp2)
            .flex()
            .items_center()
            .gap(t.sp2)
            .rounded(t.radius_sm)
            .bg(t.panel_2)
            .border_1()
            .border_color(t.line_2)
            .child(icon("search", px(13.0), t.muted))
            .when(empty, |d| {
                d.child(div().text_size(t.fs_sm).text_color(t.dim).child(i18n::t("side.search_connections")))
            })
            .when(!empty, |d| {
                d.child(
                    div()
                        .flex_1()
                        .text_size(t.fs_sm)
                        .text_color(t.ink)
                        .child(self.conn_search.clone()),
                )
            })
    }

    /// Map a detected service name to the existing right-side tool that handles
    /// it (PRODUCT-SPEC §5: mysql → MySQL, redis → Redis, postgresql →
    /// PostgreSQL, docker → Docker). Returns the `TOOLS` index, or `None` for a
    /// name with no tool — no new tools are introduced.
    fn service_tool_index(name: &str) -> Option<usize> {
        let want = match name {
            "mysql" => Svc::Mysql,
            "redis" => Svc::Redis,
            "postgresql" => Svc::Postgres,
            "docker" => Svc::Docker,
            _ => return None,
        };
        TOOLS.iter().position(|(s, _, _, _)| *s == want)
    }

    /// A wrapped row of service chips hung under the active SSH connection row.
    fn service_chips(
        &self,
        cx: &mut Context<Self>,
        row: usize,
        services: &[DetectedService],
    ) -> impl IntoElement {
        let t = &self.theme;
        let mut chips = h_flex().flex_wrap().gap(t.sp1).pl(t.sp6).pr(t.sp3).pb(t.sp2);
        for svc in services {
            if let Some(tool) = Self::service_tool_index(&svc.name) {
                chips = chips.child(self.service_chip(cx, row, svc, tool));
            }
        }
        chips
    }

    /// One service chip: the tool's glyph + the service name, tinted by running
    /// state, that selects the matching right-side tool when clicked.
    fn service_chip(
        &self,
        cx: &mut Context<Self>,
        row: usize,
        svc: &DetectedService,
        tool: usize,
    ) -> impl IntoElement {
        let t = &self.theme;
        let (svc_enum, glyph, _, _) = TOOLS[tool];
        let running = matches!(svc.status, ServiceStatus::Running);
        let tint = if running { self.svc_color(svc_enum) } else { t.muted };
        h_flex()
            .id(SharedString::from(format!("svc-chip-{row}-{}", svc.name)))
            .items_center()
            .gap(t.sp1)
            .h(px(20.0))
            .px(t.sp2)
            .rounded(t.radius_sm)
            .bg(t.panel_2)
            .border_1()
            .border_color(t.line_2)
            .text_size(t.fs_sm)
            .text_color(if running { t.ink_2 } else { t.muted })
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    this.run(Cmd::SelectTool(tool), window, cx)
                }),
            )
            .child(icon(glyph, px(11.0), tint))
            .child(svc.name.clone())
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let body = if self.show_servers {
            let mut col = v_flex()
                .child(self.section_label(i18n::tf("side.servers_count", &[&self.conns.len().to_string()])))
                .child(
                    h_flex()
                        .id("add-conn")
                        .items_center()
                        .gap(t.sp2)
                        .h(px(26.0))
                        .px(t.sp3)
                        .text_color(t.muted)
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                this.run(Cmd::OpenNewConn, window, cx)
                            }),
                        )
                        .child(icon("plus", px(14.0), t.muted))
                        .child(div().flex_1().child(i18n::t("side.add_connection"))),
                )
                .child(self.conn_search_box(cx));
            let q = self.conn_search.to_lowercase();
            // When an SSH tab is active, its `user@host` identity tags which
            // connection row gets the detected-service chips beneath it.
            let active_ssh_key: Option<String> = match self.tabs.get(self.active_tab) {
                Some(tab) if matches!(tab.kind, TabKind::Ssh) => Some(tab.title.clone()),
                _ => None,
            };
            if self.conns.is_empty() {
                col = col.child(
                    div()
                        .px(t.sp3)
                        .py(t.sp2)
                        .text_size(t.fs_sm)
                        .text_color(t.dim)
                        .child(i18n::t("side.no_saved_connections")),
                );
            } else {
                // Favorites float to the top; the store index travels with
                // each entry so row actions stay correct after sorting.
                let mut order: Vec<(usize, &ConnRow)> = self.conns.iter().enumerate().collect();
                order.sort_by_key(|(_, c)| !self.conn_favorites.contains(&c.name));
                let mut shown = 0usize;
                for (i, c) in order {
                    if !q.is_empty()
                        && !c.name.to_lowercase().contains(&q)
                        && !c.host.to_lowercase().contains(&q)
                        && !c.user.to_lowercase().contains(&q)
                    {
                        continue;
                    }
                    col = col.child(self.conn_row(cx, i, c));
                    // Hang the detected-service chips under the row matching the
                    // active SSH tab's host.
                    if let Some(key) = &active_ssh_key {
                        if *key == format!("{}@{}", c.user, c.host) {
                            if let Some(svcs) = self.detected_services.get(key) {
                                if !svcs.is_empty() {
                                    col = col.child(self.service_chips(cx, i, svcs));
                                }
                            }
                        }
                    }
                    shown += 1;
                }
                if shown == 0 {
                    col = col.child(
                        div()
                            .px(t.sp3)
                            .py(t.sp2)
                            .text_size(t.fs_sm)
                            .text_color(t.dim)
                            .child(i18n::t("side.no_matching_connections")),
                    );
                }
            }
            col
        } else {
            let q = self.file_filter.to_lowercase();
            let mut col = v_flex()
                .child(self.files_toolbar(cx))
                .child(self.files_filter(cx))
                .child(self.files_header());
            if self.cwd.parent().is_some() && q.is_empty() {
                col = col.child(self.parent_row(cx));
            }
            for f in &self.files {
                if !q.is_empty() && !f.name.to_lowercase().contains(&q) {
                    continue;
                }
                col = col.child(self.file_row(cx, f));
            }
            col
        };

        v_flex()
            .w(self.sidebar_w.unwrap_or(t.sidebar_w))
            .h_full()
            .flex_none()
            .bg(t.surface)
            .border_r_1()
            .border_color(t.line)
            .child(
                h_flex()
                    .w_full()
                    .border_b_1()
                    .border_color(t.line)
                    .child(self.sidebar_tab(cx, "side.files", false))
                    .child(self.sidebar_tab(cx, "side.servers", true)),
            )
            .child(
                div()
                    .id("sidebar-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(body),
            )
    }

    // ── TabBar ────────────────────────────────────────────────────
    fn tab_item(&self, cx: &mut Context<Self>, idx: usize) -> impl IntoElement {
        let t = &self.theme;
        let tab = &self.tabs[idx];
        let active = self.active_tab == idx;
        let dot = tab.color.and_then(|c| Self::tab_palette(t).get(c).copied());
        let drag_title = tab.title.clone();
        let drag_theme = self.theme.clone();
        h_flex()
            .id(SharedString::from(format!("tab-{idx}")))
            .flex_none()
            .items_center()
            .gap(t.sp2)
            .h_full()
            .px(t.sp3)
            .border_r_1()
            .border_color(t.line)
            .when(active, |d| d.bg(t.bg).border_b_2().border_color(t.accent))
            .when(!active, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _w, cx| {
                    this.tab_menu = Some((ev.position, idx));
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    // Guard: a close-button click on a sibling tab can shrink the
                    // vec before this select handler runs (event bubbling), so the
                    // captured `idx` may be stale.
                    if idx >= this.tabs.len() {
                        return;
                    }
                    this.active_tab = idx;
                    let handle = this.tabs[idx].terminal.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                    cx.notify();
                }),
            )
            // Drag to reorder; drop a dragged tab here to land it at this slot.
            .on_drag(TabDrag(idx), move |_, _, _, cx| {
                cx.new(|_| TabDragPreview {
                    title: drag_title.clone(),
                    theme: drag_theme.clone(),
                })
            })
            .on_drop(cx.listener(move |this, drag: &TabDrag, _w, cx| {
                this.move_tab(drag.0, idx, cx);
            }))
            .when_some(dot, |d, col| {
                d.child(div().w(px(6.0)).h(px(6.0)).flex_none().rounded_full().bg(col))
            })
            .child(icon(
                Self::tab_icon(tab.kind),
                px(14.0),
                if active { t.accent } else { t.muted },
            ))
            .child(
                div()
                    .max_w(px(150.0))
                    .overflow_hidden()
                    .text_color(if active { t.ink } else { t.muted })
                    .child(tab.title.clone()),
            )
            .child(
                div()
                    .id(SharedString::from(format!("tabx-{idx}")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(16.0))
                    .h(px(16.0))
                    .rounded(t.radius_sm)
                    .hover(|s| s.bg(t.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            // Closing must not also fire the tab's select handler
                            // (which would then index a now-shorter vec).
                            cx.stop_propagation();
                            if idx < this.tabs.len() {
                                this.tabs.remove(idx);
                                if this.active_tab >= this.tabs.len() {
                                    this.active_tab = this.tabs.len().saturating_sub(1);
                                }
                                cx.notify();
                            }
                        }),
                    )
                    .child(icon("close", px(12.0), t.muted)),
            )
    }

    fn tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        // Tabs scroll horizontally when they overflow; the new-tab button is pinned.
        let mut tabs = h_flex().id("tabs-scroll").flex_1().min_w(px(0.0)).overflow_x_scroll();
        for idx in 0..self.tabs.len() {
            tabs = tabs.child(self.tab_item(cx, idx));
        }
        h_flex()
            .w_full()
            .h(t.tabbar_h)
            .bg(t.surface)
            .border_b_1()
            .border_color(t.line)
            .child(tabs)
            .child(
                div()
                    .id("new-tab")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .w(px(34.0))
                    .h_full()
                    .hover(|s| s.bg(t.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                            this.open_terminal_tab(cx);
                        }),
                    )
                    .child(icon("plus", px(15.0), t.muted)),
            )
    }

    // ── Right zone: panel + tool strip ───────────────────────────
    fn tool_btn(&self, cx: &mut Context<Self>, idx: usize) -> impl IntoElement {
        let t = &self.theme;
        let (svc, glyph, _, _) = TOOLS[idx];
        let active = self.active_tool == idx;
        let color = self.svc_color(svc);
        div()
            .id(SharedString::from(format!("tool-{idx}")))
            .flex()
            .items_center()
            .justify_center()
            .w(px(32.0))
            .h(px(32.0))
            .rounded(t.radius_sm)
            .when(active, |d| d.bg(t.accent_dim))
            .when(!active, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.active_tool = idx;
                    this.right_collapsed = false;
                    if matches!(TOOLS[idx].0, Svc::Monitor) {
                        this.mon = Some(data::monitor_snapshot());
                    }
                    cx.notify();
                }),
            )
            .child(icon(glyph, px(17.0), if active { color } else { t.muted }))
    }

    fn tool_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let mut col = v_flex()
            .w(t.toolrail_w)
            .h_full()
            .items_center()
            .py(t.sp2)
            .gap(px(2.0))
            .bg(t.surface)
            .border_l_1()
            .border_color(t.line_2);
        let mut prev_cat = TOOLS[0].3;
        for idx in 0..TOOLS.len() {
            let cat = TOOLS[idx].3;
            if cat != prev_cat {
                col = col.child(
                    div()
                        .my(px(2.0))
                        .w(px(20.0))
                        .h(px(1.0))
                        .bg(t.line_2),
                );
                prev_cat = cat;
            }
            col = col.child(self.tool_btn(cx, idx));
        }
        col.child(div().flex_1()).child(
            div()
                .id("collapse")
                .flex()
                .items_center()
                .justify_center()
                .w(px(32.0))
                .h(px(32.0))
                .rounded(t.radius_sm)
                .text_color(t.muted)
                .hover(|s| s.bg(t.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                        this.right_collapsed = !this.right_collapsed;
                        cx.notify();
                    }),
                )
                .child(icon(
                    if self.right_collapsed {
                        "panel-right-open"
                    } else {
                        "panel-right-close"
                    },
                    px(16.0),
                    t.muted,
                )),
        )
    }

    fn panel_header(
        &self,
        glyph: &'static str,
        title: impl Into<SharedString>,
        meta: impl Into<SharedString>,
    ) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .w_full()
            .h(t.panel_header_h)
            .px(t.sp3)
            .border_b_1()
            .border_color(t.line)
            .child(icon(glyph, px(15.0), t.accent))
            .child(
                div()
                    .font_family(t.mono.clone())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.ink)
                    .child(title.into()),
            )
            .child(div().flex_1())
            .child(div().text_size(t.fs_sm).text_color(t.muted).child(meta.into()))
    }

    /// PROCESS / CPU% / MEM% header for a Monitor top-process table.
    fn mon_proc_header(&self) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .pb(px(2.0))
            .text_size(t.fs_sm)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.muted)
            .child(div().flex_1().child(i18n::t("mon.process")))
            .child(div().w(px(48.0)).flex_none().child(i18n::t("mon.cpu_pct")))
            .child(div().w(px(48.0)).flex_none().child(i18n::t("mon.mem_pct")))
    }

    /// One process row in a Monitor top-process table.
    fn mon_proc_row(&self, p: &data::ProcInfo) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .gap(t.sp2)
            .px(t.sp3)
            .py(px(2.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(t.fs_ui)
                    .text_color(t.ink_2)
                    .child(p.name.clone()),
            )
            .child(
                div()
                    .w(px(48.0))
                    .flex_none()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(p.cpu.clone()),
            )
            .child(
                div()
                    .w(px(48.0))
                    .flex_none()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(p.mem.clone()),
            )
    }

    // ── Monitor panel (real local metrics) ───────────────────────
    fn monitor_panel(&self) -> AnyElement {
        let t = &self.theme;
        let Some(m) = &self.mon else {
            return v_flex()
                .flex_1()
                .child(self.panel_header("activity", i18n::t("tool.monitor"), ""))
                .child(div().p(t.sp4).text_color(t.muted).child(i18n::t("mon.sampling")))
                .into_any_element();
        };
        let gb = |mb: f64| format!("{:.1} GB", mb / 1024.0);
        let mem_pct = if m.mem_total_mb > 0.0 {
            m.mem_used_mb / m.mem_total_mb * 100.0
        } else {
            0.0
        };
        let swap_pct = if m.swap_total_mb > 0.0 {
            m.swap_used_mb / m.swap_total_mb * 100.0
        } else {
            0.0
        };

        let mut col = v_flex()
            .flex_1()
            .min_h(px(0.0))
            .child(self.panel_header("activity", i18n::t("tool.monitor"), "localhost"))
            .child(ui::meter(
                t,
                i18n::t("mon.cpu"),
                i18n::tf("mon.cpu_cores", &[&format!("{:.0}", m.cpu_pct), &m.cpu_count.to_string()]),
                m.cpu_pct,
            ))
            .child(ui::meter(
                t,
                i18n::t("mon.memory"),
                format!("{} / {}", gb(m.mem_used_mb), gb(m.mem_total_mb)),
                mem_pct,
            ));
        if m.swap_total_mb > 0.0 {
            col = col.child(ui::meter(
                t,
                i18n::t("mon.swap"),
                format!("{} / {}", gb(m.swap_used_mb), gb(m.swap_total_mb)),
                swap_pct,
            ));
        }
        col = col
            .child(self.section_label(i18n::t("mon.system")))
            .child(ui::info_row(t, i18n::t("mon.uptime"), m.uptime.clone()))
            .child(ui::info_row(t, i18n::t("mon.processes"), m.proc_count.to_string()));
        if let Some((l1, l5, l15)) = m.load {
            col = col.child(ui::info_row(t, i18n::t("mon.load"), format!("{l1:.2} {l5:.2} {l15:.2}")));
        }
        col = col.child(ui::info_row(t, i18n::t("mon.os"), m.os_label.clone()));

        if !m.disks.is_empty() {
            col = col.child(self.section_label(i18n::t("mon.disks")));
            for d in &m.disks {
                col = col.child(ui::meter(
                    t,
                    d.mount.clone(),
                    format!("{} / {}", d.used, d.total),
                    d.use_pct,
                ));
            }
        }

        col = col
            .child(self.section_label(i18n::t("mon.network")))
            .child(ui::info_row(t, i18n::t("common.download"), fmt_rate(m.net_rx_bps)))
            .child(ui::info_row(t, i18n::t("common.upload"), fmt_rate(m.net_tx_bps)));

        if !m.top_cpu.is_empty() {
            col = col
                .child(self.section_label(i18n::t("mon.top_by_cpu")))
                .child(self.mon_proc_header());
            for p in &m.top_cpu {
                col = col.child(self.mon_proc_row(p));
            }
        }
        if !m.top_mem.is_empty() {
            col = col
                .child(self.section_label(i18n::t("mon.top_by_mem")))
                .child(self.mon_proc_header());
            for p in &m.top_mem {
                col = col.child(self.mon_proc_row(p));
            }
        }
        col.into_any_element()
    }

    fn right_panel(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let (svc, glyph, name, _) = TOOLS[self.active_tool];
        if matches!(svc, Svc::Git) {
            // The Git panel is an independent View owning its layout + scroll.
            self.git_panel_view.clone().into_any_element()
        } else if matches!(svc, Svc::Monitor) {
            div()
                .id("mon-scroll")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(self.monitor_panel())
                .into_any_element()
        } else if let Some(view) = self.panels.for_svc(svc) {
            // Each panel View (src/panels/*.rs) owns its own layout and scroll.
            view
        } else {
            v_flex()
                .flex_1()
                .child(self.panel_header(glyph, tool_label(name), "panel"))
                .child(ui::empty_state(&self.theme, i18n::t("conn.not_implemented")))
                .into_any_element()
        }
    }

    // ── StatusBar ─────────────────────────────────────────────────
    fn status_item(&self, text: impl Into<SharedString>, color: Hsla) -> impl IntoElement {
        div().text_color(color).child(text.into())
    }

    fn status_bar(
        &self,
        cols: u16,
        rows: u16,
        branch: String,
        ahead_behind: String,
    ) -> impl IntoElement {
        let t = &self.theme;
        let (_, _, tool_name, _) = TOOLS[self.active_tool];
        h_flex()
            .items_center()
            .justify_between()
            .w_full()
            .h(t.statusbar_h)
            .px(t.sp3)
            .bg(t.surface)
            .border_t_1()
            .border_color(t.line)
            .text_size(t.fs_sm)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp3)
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(icon("git-branch", px(12.0), t.ink_2))
                            .child(self.status_item(branch, t.ink_2)),
                    )
                    .child(self.status_item(ahead_behind, t.muted))
                    .child(self.status_item(i18n::t("status.local_pwsh"), t.ink_2))
                    .child(self.status_item(format!("{cols}×{rows}"), t.muted)),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp3)
                    .child(self.status_item(i18n::tf("status.panel", &[tool_label(tool_name).as_ref()]), t.accent))
                    .child(self.status_item("UTF-8", t.muted))
                    .child(self.status_item(i18n::t("status.ready"), t.pos))
                    .child(self.status_item("Pier-X v0.7.2", t.muted)),
            )
    }

    // ── Overlays (command palette / dialogs) ─────────────────────

    fn palette_row(
        &self,
        cx: &mut Context<Self>,
        glyph: &'static str,
        label: SharedString,
        cmd: Cmd,
        first: bool,
    ) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .id(SharedString::from(format!("pal-{label}")))
            .items_center()
            .gap(t.sp2)
            .h(px(30.0))
            .px(t.sp3)
            .rounded(t.radius_sm)
            .cursor_pointer()
            .when(first, |d| d.bg(t.accent_subtle))
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| this.run(cmd, window, cx)),
            )
            .child(icon(glyph, px(15.0), t.ink_2))
            .child(div().text_color(t.ink_2).child(label))
    }

    fn palette_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let matches = self.palette_matches();
        let mut list = v_flex().id("palette-list").max_h(px(420.0)).overflow_y_scroll();
        if matches.is_empty() {
            list = list.child(
                div()
                    .px(t.sp3)
                    .py(t.sp3)
                    .text_color(t.dim)
                    .child(i18n::t("palette.no_match")),
            );
        } else {
            for (i, (glyph, label, cmd)) in matches.into_iter().enumerate() {
                // Highlight the first row (what Enter runs).
                let first = i == 0;
                list = list.child(self.palette_row(cx, glyph, label, cmd, first));
            }
        }
        // The query input: a focused box echoing palette_query with a caret.
        let query_box = div()
            .track_focus(&self.palette_focus)
            .key_context("Palette")
            .on_key_down(cx.listener(Self::on_palette_key))
            .h(px(34.0))
            .px(t.sp3)
            .flex()
            .items_center()
            .gap(t.sp2)
            .border_b_1()
            .border_color(t.line)
            .child(icon("command", px(15.0), t.muted))
            .child(
                div()
                    .flex_1()
                    .when(self.palette_query.is_empty(), |d| {
                        d.text_color(t.dim).child(i18n::t("palette.placeholder"))
                    })
                    .when(!self.palette_query.is_empty(), |d| {
                        d.text_color(t.ink).child(self.palette_query.clone())
                    }),
            );
        v_flex()
            .w(px(460.0))
            .bg(t.panel)
            .border_1()
            .border_color(t.line_2)
            .rounded(t.radius_lg)
            .overflow_hidden()
            .child(query_box)
            .child(div().p(t.sp2).child(list))
    }

    fn conn_field_row(
        &self,
        cx: &mut Context<Self>,
        idx: usize,
        label: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        masked: bool,
    ) -> impl IntoElement {
        let t = &self.theme;
        let raw = self.conn_form[idx].clone();
        let active = self.conn_field == idx;
        let empty = raw.is_empty();
        let shown = if masked {
            "•".repeat(raw.chars().count())
        } else {
            raw
        };
        v_flex()
            .gap(px(3.0))
            .child(div().text_size(t.fs_sm).text_color(t.muted).child(label.into()))
            .child(
                div()
                    .id(SharedString::from(format!("cf-{idx}")))
                    .h(px(30.0))
                    .px(t.sp2)
                    .flex()
                    .items_center()
                    .rounded(t.radius_sm)
                    .bg(t.panel_2)
                    .border_1()
                    .border_color(if active { t.accent } else { t.line_2 })
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            this.conn_field = idx;
                            window.focus(&this.conn_focus, cx);
                            cx.notify();
                        }),
                    )
                    .when(empty, |d| d.text_color(t.dim).child(placeholder.into()))
                    .when(!empty, |d| d.text_color(t.ink).child(shown)),
            )
    }

    /// One segment of the auth-kind selector.
    fn conn_auth_btn(
        &self,
        cx: &mut Context<Self>,
        label: &'static str,
        kind: ConnAuthKind,
    ) -> impl IntoElement {
        let t = &self.theme;
        let active = self.conn_auth == kind;
        div()
            .id(SharedString::from(format!("cauth-{label}")))
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .py(px(5.0))
            .rounded(t.radius_sm)
            .text_size(t.fs_ui)
            .cursor_pointer()
            .when(active, |d| d.bg(t.accent).text_color(t.accent_ink))
            .when(!active, |d| d.bg(t.panel_2).text_color(t.ink_2))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.conn_auth = kind;
                    cx.notify();
                }),
            )
            .child(i18n::t(label))
    }

    /// The secondary "Test" button that probes the form's connection.
    fn conn_test_btn(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id("conn-test")
            .px(t.sp3)
            .py(px(5.0))
            .rounded(t.radius_sm)
            .text_size(t.fs_ui)
            .cursor_pointer()
            .bg(t.panel_2)
            .text_color(t.ink_2)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| this.test_conn(cx)),
            )
            .child(i18n::t("common.test"))
    }

    fn conn_btn(&self, cx: &mut Context<Self>, label: &'static str, primary: bool, cmd: Option<Cmd>) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(SharedString::from(format!("connbtn-{label}")))
            .px(t.sp3)
            .py(px(5.0))
            .rounded(t.radius_sm)
            .text_size(t.fs_ui)
            .cursor_pointer()
            .when(primary, |d| d.bg(t.accent).text_color(t.accent_ink))
            .when(!primary, |d| d.bg(t.panel_2).text_color(t.ink_2))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| match cmd {
                    Some(c) => this.run(c, window, cx),
                    None => this.submit_conn(window, cx),
                }),
            )
            .child(i18n::t(label))
    }

    fn conn_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .track_focus(&self.conn_focus)
            .key_context("NewConn")
            .on_key_down(cx.listener(Self::on_conn_key))
            .w(px(420.0))
            .p(t.sp4)
            .gap(t.sp3)
            .bg(t.panel)
            .border_1()
            .border_color(t.line_2)
            .rounded(t.radius_lg)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .child(icon("server", px(16.0), t.accent))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink)
                            .child(i18n::t(if self.conn_edit.is_some() {
                                "conn.edit"
                            } else {
                                "conn.new"
                            })),
                    ),
            )
            .child(self.conn_field_row(cx, 0, i18n::t("common.name"), i18n::t("conn.optional_label"), false))
            .child(
                h_flex()
                    .gap(t.sp2)
                    .child(div().flex_1().child(self.conn_field_row(cx, 1, i18n::t("common.host"), i18n::t("conn.host_or_ip"), false)))
                    .child(div().w(px(96.0)).child(self.conn_field_row(cx, 2, i18n::t("common.port"), "22", false))),
            )
            .child(self.conn_field_row(cx, 3, i18n::t("common.user"), i18n::t("conn.ph_user"), false))
            .child(
                v_flex()
                    .gap(px(3.0))
                    .child(div().text_size(t.fs_sm).text_color(t.muted).child(i18n::t("conn.authentication")))
                    .child(
                        h_flex()
                            .gap(t.sp2)
                            .child(self.conn_auth_btn(cx, "common.password", ConnAuthKind::Password))
                            .child(self.conn_auth_btn(cx, "conn.key_file", ConnAuthKind::Key))
                            .child(self.conn_auth_btn(cx, "conn.agent", ConnAuthKind::Agent)),
                    ),
            )
            .when(self.conn_auth == ConnAuthKind::Password, |d| {
                // Editing a connection that already stores a password: the
                // blank field means "keep it", so hint that instead of
                // prompting as if it were empty.
                let saved = matches!(
                    self.conn_orig_auth,
                    Some(AuthMethod::KeychainPassword { .. } | AuthMethod::DirectPassword { .. })
                );
                let ph = if saved { i18n::t("conn.leave_blank_keep") } else { i18n::t("conn.ph_password") };
                d.child(self.conn_field_row(cx, CONN_SECRET, i18n::t("common.password"), ph, true))
            })
            .when(self.conn_auth == ConnAuthKind::Key, |d| {
                d.child(self.conn_field_row(cx, CONN_SECRET, i18n::t("conn.key_file"), "~/.ssh/id_ed25519", false))
            })
            .when(self.conn_auth == ConnAuthKind::Agent, |d| {
                d.child(
                    div()
                        .text_size(t.fs_sm)
                        .text_color(t.dim)
                        .child(i18n::t("conn.uses_ssh_agent")),
                )
            })
            .child(self.conn_field_row(cx, 5, i18n::t("conn.group"), i18n::t("conn.optional_group"), false))
            .when_some(self.conn_error.clone(), |d, e| {
                d.child(div().text_size(t.fs_sm).text_color(t.neg).child(e))
            })
            .when_some(self.conn_test.clone(), |d, msg| {
                d.child(
                    div()
                        .text_size(t.fs_sm)
                        .font_family(t.mono.clone())
                        .text_color(t.ink_2)
                        .child(msg),
                )
            })
            .child(
                h_flex()
                    .gap(t.sp2)
                    .justify_end()
                    .child(self.conn_btn(cx, "common.cancel", false, Some(Cmd::CloseOverlay)))
                    .child(self.conn_test_btn(cx))
                    .child(self.conn_btn(
                        cx,
                        if self.conn_edit.is_some() { "common.save" } else { "common.add" },
                        true,
                        None,
                    )),
            )
    }

    /// Right-click menu for a tab, anchored at the cursor (paints on top).
    fn tab_context_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let Some((pos, idx)) = self.tab_menu else {
            return div().into_any_element();
        };
        let item = |key: &'static str, label: &'static str, cmd: Cmd, cx: &mut Context<Self>| {
            div()
                .id(SharedString::from(format!("tabctx-{key}")))
                .px(t.sp3)
                .py(px(5.0))
                .text_size(t.fs_ui)
                .text_color(t.ink_2)
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.run(cmd, window, cx)
                    }),
                )
                .child(i18n::t(label))
        };
        // Color swatches: a clear button followed by the eight palette colors.
        let mut swatches = h_flex().gap(px(4.0)).px(t.sp3).py(px(4.0)).child(
            div()
                .id("tabcol-none")
                .flex()
                .items_center()
                .justify_center()
                .w(px(16.0))
                .h(px(16.0))
                .rounded(t.radius_sm)
                .border_1()
                .border_color(t.line_3)
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.run(Cmd::SetTabColor(idx, None), window, cx)
                    }),
                )
                .child(icon("close", px(10.0), t.muted)),
        );
        for (k, col) in Self::tab_palette(t).into_iter().enumerate() {
            swatches = swatches.child(
                div()
                    .id(SharedString::from(format!("tabcol-{k}")))
                    .w(px(16.0))
                    .h(px(16.0))
                    .rounded(t.radius_sm)
                    .bg(col)
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            this.run(Cmd::SetTabColor(idx, Some(k)), window, cx)
                        }),
                    ),
            );
        }
        deferred(
            v_flex()
                .id("tab-ctx")
                .absolute()
                .left(pos.x)
                .top(pos.y)
                .min_w(px(200.0))
                .py(t.sp1)
                .bg(t.elev)
                .border_1()
                .border_color(t.line_2)
                .rounded(t.radius_md)
                .on_mouse_down_out(cx.listener(|this, _, _w, cx| {
                    this.tab_menu = None;
                    cx.notify();
                }))
                .child(item("close", "tab.close", Cmd::CloseTabAt(idx), cx))
                .child(item("others", "tab.close_others", Cmd::CloseOthers(idx), cx))
                .child(item("left", "tab.close_left", Cmd::CloseToLeft(idx), cx))
                .child(item("right", "tab.close_right", Cmd::CloseToRight(idx), cx))
                .child(div().my(px(4.0)).mx(t.sp2).h(px(1.0)).bg(t.line_2))
                .child(
                    div()
                        .px(t.sp3)
                        .pb(px(2.0))
                        .text_size(t.fs_sm)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.muted)
                        .child(i18n::t("tab.color")),
                )
                .child(swatches),
        )
        .into_any_element()
    }

    /// Full-window scrim + centered modal card for the active overlay.
    fn overlay_layer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let card = match self.overlay {
            Overlay::Settings => self.settings_view.clone().into_any_element(),
            Overlay::Palette => self.palette_card(cx).into_any_element(),
            Overlay::NewConn => self.conn_card(cx).into_any_element(),
            Overlay::Broadcast => self.broadcast_view.clone().into_any_element(),
            Overlay::Egress => self.egress_view.clone().into_any_element(),
            Overlay::PortForward => self.tunnel_view.clone().into_any_element(),
            Overlay::HostsHealth => self.hosts_health_view.clone().into_any_element(),
            Overlay::HostKey => self.hostkey_view.clone().into_any_element(),
            Overlay::None => div().into_any_element(),
        };
        div()
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("scrim")
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .size_full()
                    .bg(t.scrim)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            this.run(Cmd::CloseOverlay, window, cx)
                        }),
                    ),
            )
            // `occlude` stops clicks inside the dialog from falling through to
            // the scrim's CloseOverlay — otherwise any click in the card (e.g. a
            // Settings nav item) would dismiss the overlay. Outside clicks still
            // hit the scrim and close it.
            .child(div().occlude().child(card))
    }

    // ── Welcome view (shown when no tab is open) ─────────────────
    /// A quick-action card on the Welcome view.
    fn welcome_action(
        &self,
        cx: &mut Context<Self>,
        glyph: &'static str,
        label: &'static str,
        sub: &'static str,
        cmd: Cmd,
    ) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .id(SharedString::from(format!("wa-{label}")))
            .w(px(168.0))
            .gap(px(6.0))
            .p(t.sp3)
            .rounded(t.radius_md)
            .bg(t.panel)
            .border_1()
            .border_color(t.line)
            .cursor_pointer()
            .hover(|s| s.bg(t.panel_2).border_color(t.line_2))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| this.run(cmd, window, cx)),
            )
            .child(icon(glyph, px(18.0), t.accent))
            .child(div().text_color(t.ink).child(i18n::t(label)))
            .child(div().text_size(t.fs_sm).text_color(t.muted).child(i18n::t(sub)))
    }

    /// A saved-connection shortcut row on the Welcome view (opens SSH).
    fn welcome_conn_row(&self, cx: &mut Context<Self>, idx: usize, c: &ConnRow) -> impl IntoElement {
        let t = &self.theme;
        let dot = self.conn_dot_color(c.online);
        h_flex()
            .id(SharedString::from(format!("wc-{idx}")))
            .items_center()
            .gap(t.sp2)
            .h(px(30.0))
            .px(t.sp3)
            .rounded(t.radius_sm)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.selected_conn = idx;
                    this.open_ssh_tab(idx, cx);
                }),
            )
            .child(div().w(px(7.0)).h(px(7.0)).flex_none().rounded_full().bg(dot))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_color(t.ink_2)
                    .child(c.name.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(c.addr.clone()),
            )
    }

    /// The greeting + quick-action view rendered when no tab is open.
    fn welcome_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let mut conns = v_flex()
            .w(px(440.0))
            .gap(px(2.0))
            .child(self.section_label(i18n::tf("side.saved_connections", &[&self.conns.len().to_string()])));
        if self.conns.is_empty() {
            conns = conns.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child(i18n::t("welcome.no_connections")),
            );
        } else {
            for (i, c) in self.conns.iter().enumerate().take(6) {
                conns = conns.child(self.welcome_conn_row(cx, i, c));
            }
        }

        v_flex()
            .id("welcome")
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .overflow_y_scroll()
            .items_center()
            .gap(t.sp5)
            .pt(px(60.0))
            .pb(t.sp6)
            .px(t.sp6)
            .bg(t.bg)
            .child(
                v_flex()
                    .items_center()
                    .gap(t.sp2)
                    .child(div().w(px(44.0)).h(px(44.0)).rounded(t.radius_lg).bg(t.accent))
                    .child(
                        div()
                            .text_size(t.fs_h3)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink)
                            .child(i18n::t("welcome.title")),
                    )
                    .child(
                        div()
                            .text_size(t.fs_ui)
                            .text_color(t.muted)
                            .child(i18n::t("welcome.subtitle")),
                    ),
            )
            .child(
                v_flex()
                    .gap(t.sp3)
                    .child(
                        h_flex()
                            .gap(t.sp3)
                            .justify_center()
                            .child(self.welcome_action(
                                cx,
                                "square-terminal",
                                "tab.new_terminal",
                                "status.local_shell",
                                Cmd::NewTerminal,
                            ))
                            .child(self.welcome_action(
                                cx,
                                "server",
                                "tab.new_ssh",
                                "welcome.connect_host",
                                Cmd::OpenNewConn,
                            )),
                    )
                    .child(
                        h_flex()
                            .gap(t.sp3)
                            .justify_center()
                            .child(self.welcome_action(
                                cx,
                                "command",
                                "menu.command_palette",
                                "palette.go_to_anything",
                                Cmd::OpenPalette,
                            ))
                            .child(self.welcome_action(
                                cx,
                                "settings",
                                "set.title",
                                "menu.appearance_more",
                                Cmd::OpenSettings,
                            )),
                    ),
            )
            .child(conns)
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Pick up the current global theme so dark/light toggles propagate.
        self.theme = cx.global::<Theme>().clone();
        let t = self.theme.clone();
        let active_terminal = self.tabs.get(self.active_tab).map(|tab| tab.terminal.clone());
        let (cols, rows) = match &active_terminal {
            Some(term) => term.read(cx).size(),
            None => (0, 0),
        };
        // Branch + ahead/behind for the status bar, read from the Git panel view.
        let (branch, ahead_behind) = match self.git_panel_view.read(cx).status_summary() {
            Some((b, ahead, behind)) => (b, format!("↑{ahead} ↓{behind}")),
            None => ("—".to_string(), String::new()),
        };

        // Right zone: optional panel (with a drag handle) + tool strip.
        let mut right_zone = h_flex().h_full();
        if !self.right_collapsed {
            right_zone = right_zone.child(self.drag_handle(cx, DragTarget::Right)).child(
                v_flex()
                    .w(self.right_w.unwrap_or(t.rightpanel_w))
                    .h_full()
                    .flex_none()
                    .bg(t.surface)
                    .border_l_1()
                    .border_color(t.line)
                    .child(self.right_panel(cx)),
            );
        }
        right_zone = right_zone.child(self.tool_strip(cx));

        let body = v_flex()
            .size_full()
            .font_family(t.sans.clone())
            .text_size(t.fs_body)
            .text_color(t.ink)
            .bg(t.bg)
            .child(self.topbar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(self.sidebar(cx))
                    .child(self.drag_handle(cx, DragTarget::Sidebar))
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .min_w(px(0.0))
                            .child(self.tab_bar(cx))
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .w_full()
                                    .child(match active_terminal {
                                        Some(term) => term.into_any_element(),
                                        None => self.welcome_view(cx).into_any_element(),
                                    }),
                            ),
                    )
                    .child(right_zone),
            )
            .child(self.status_bar(cols, rows, branch, ahead_behind));

        // Overlay layer (Settings / command palette) paints on top of the shell.
        let show_overlay = self.overlay != Overlay::None;
        let dragging = self.dragging.is_some();
        div()
            .id("shell-root")
            .relative()
            .size_full()
            // Global keyboard shortcuts (bound in main.rs) dispatch here.
            .on_action(cx.listener(|this, _: &CmdPalette, window, cx| {
                this.run(Cmd::OpenPalette, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CmdNewTerminal, window, cx| {
                this.run(Cmd::NewTerminal, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CmdCloseTab, window, cx| {
                this.run(Cmd::CloseTab, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CmdToggleTheme, window, cx| {
                this.run(Cmd::ToggleTheme, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CmdSettings, window, cx| {
                this.run(Cmd::OpenSettings, window, cx)
            }))
            // While a divider is dragged, track moves at the root and commit
            // widths from the cursor x; release on mouse-up.
            .when(dragging, |d| {
                d.on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                    let Some(target) = this.dragging else { return };
                    let x = f32::from(ev.position.x);
                    match target {
                        DragTarget::Sidebar => {
                            this.sidebar_w = Some(px(x.clamp(180.0, 480.0)));
                        }
                        DragTarget::Right => {
                            let vw = f32::from(window.viewport_size().width);
                            let trw = f32::from(this.theme.toolrail_w);
                            this.right_w = Some(px((vw - trw - x).clamp(260.0, 720.0)));
                        }
                    }
                    cx.notify();
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _w, cx| {
                        this.dragging = None;
                        this.persist(cx);
                        cx.notify();
                    }),
                )
            })
            .child(body)
            .when(self.tab_menu.is_some(), |d| {
                d.child(self.tab_context_menu(cx))
            })
            .when(show_overlay, |d| d.child(self.overlay_layer(cx)))
    }
}

/// Human-readable transfer rate; `—` while the sampler is warming up
/// (the local probe returns a negative rate on its first tick).
fn fmt_rate(bps: f64) -> String {
    if bps < 0.0 {
        "—".to_string()
    } else {
        format!("{}/s", human_size(bps as u64))
    }
}

/// Compact human-readable byte size, e.g. `4.0K`, `1.2M`.
fn human_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n}B")
    } else {
        format!("{v:.1}{}", UNITS[i])
    }
}

/// Last few path components joined with " / " for the Files breadcrumb.
fn breadcrumb(path: &std::path::Path) -> String {
    let parts: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().trim_end_matches('\\').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let tail = if parts.len() > 3 {
        &parts[parts.len() - 3..]
    } else {
        &parts[..]
    };
    let mut s = tail.join("  /  ");
    if parts.len() > 3 {
        s = format!("…  /  {s}");
    }
    s
}

// Git file-status mark/colour and the Git panel itself now live in git_panel.rs
// (GitPanelView), extracted from this shell.
