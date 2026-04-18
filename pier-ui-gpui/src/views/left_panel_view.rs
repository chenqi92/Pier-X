//! Left panel as an independent `Entity<LeftPanelView>` so its filter
//! inputs and file-browser state are isolated from the rest of the shell.
//!
//! ## Why a dedicated entity (Phase 9 architectural perf fix)
//!
//! Earlier the filter `InputState` lived in `PierApp` and `cx.subscribe`
//! triggered `cx.notify()` on `PierApp` for every keystroke — which
//! re-rendered the **entire** shell tree (toolbar + center + right
//! panel). Lifting the left panel out means filter changes only repaint
//! the ~260px left column.
//!
//! ## State ownership
//!
//! - **PierApp** owns: connections, terminals, active SSH session, theme,
//!   layout flags. Single source of truth for cross-panel state.
//! - **LeftPanelView** owns: file-browser cwd / entries cache, filter
//!   inputs, active tab. Locally scoped state.
//! - **`cx.observe(weak_app)`** keeps a cached servers sidebar snapshot in
//!   sync — fires on every PierApp `cx.notify()`, which is rare-ish
//!   (tab/mode/connection/session changes).
//!
//! ## Callbacks back to PierApp
//!
//! Server actions (open / edit / delete / add) call into PierApp via
//! `weak_app.update(cx, |pa, cx| pa.open_ssh_terminal(idx, cx))` etc.
//! File `.md` opens call `weak_app.update(cx, |pa, cx| pa.open_markdown_file(...))`.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    div, prelude::*, px, App, ClickEvent, Context, Entity, IntoElement, SharedString, WeakEntity,
    Window,
};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    Icon as UiIcon, IconName,
};
use pier_core::paths;
use pier_core::ssh::{AuthMethod, DetectedService, ServiceStatus, SshConfig};
use rust_i18n::t;

use crate::app::layout::LeftTab;
use crate::app::ssh_session::{ConnectStatus, ServiceProbeStatus, TunnelStatus};
use crate::app::PierApp;
use crate::components::{text, Button, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_BODY, SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};
use crate::views::file_tree::{self, FileTree, FsEntry};

const UNGROUPED: &str = "Ungrouped";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServersSidebarSnapshot {
    pub connections: Vec<SshConfig>,
    pub active_session: Option<ActiveServerSessionSnapshot>,
}

impl ServersSidebarSnapshot {
    pub fn from_connections(connections: Vec<SshConfig>) -> Self {
        Self {
            connections,
            active_session: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveServerSessionSnapshot {
    pub config: SshConfig,
    pub status: ConnectStatus,
    pub service_probe_status: ServiceProbeStatus,
    pub service_probe_error: Option<String>,
    pub services: Vec<DetectedService>,
    pub tunnels: Vec<ServerTunnelSnapshot>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerTunnelSnapshot {
    pub service_name: String,
    pub remote_port: u16,
    pub local_port: Option<u16>,
    pub status: TunnelStatus,
    pub last_error: Option<String>,
}

pub struct LeftPanelView {
    weak_app: WeakEntity<PierApp>,
    active_tab: LeftTab,

    files_filter: Entity<InputState>,
    servers_filter: Entity<InputState>,

    /// File browser state (formerly in PierApp).
    file_tree_cwd: PathBuf,
    file_tree_entries: Vec<FsEntry>,
    file_tree_error: Option<String>,

    /// Cached servers sidebar snapshot. Refreshed via `cx.observe(weak_app)`.
    servers_snapshot: ServersSidebarSnapshot,
    collapsed_server_groups: BTreeSet<String>,
}

impl LeftPanelView {
    pub fn new(
        weak_app: WeakEntity<PierApp>,
        initial_connections: Vec<SshConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let files_filter =
            cx.new(|c| InputState::new(window, c).placeholder(t!("App.FileTree.filter")));
        let servers_filter =
            cx.new(|c| InputState::new(window, c).placeholder(t!("App.LeftPanel.filter")));

        // Filter Change → repaint just LeftPanelView. PierApp is NOT touched
        // — that's the whole point of extracting this into its own entity.
        cx.subscribe(&files_filter, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&servers_filter, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();

        // Observe PierApp to keep our local servers snapshot fresh.
        if let Some(app_entity) = weak_app.upgrade() {
            cx.observe(&app_entity, |this, app, cx| {
                let next = app.read(cx).servers_sidebar_snapshot(cx);
                if next != this.servers_snapshot {
                    this.servers_snapshot = next;
                    prune_collapsed_groups(
                        &mut this.collapsed_server_groups,
                        &this.servers_snapshot.connections,
                    );
                    cx.notify();
                }
            })
            .detach();
        }

        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let (entries, error) = match file_tree::list_dir(&cwd) {
            Ok(entries) => (entries, None),
            Err(err) => (Vec::new(), Some(format!("{err}"))),
        };

        let servers_snapshot = ServersSidebarSnapshot::from_connections(initial_connections);

        Self {
            weak_app,
            active_tab: LeftTab::Files,
            files_filter,
            servers_filter,
            file_tree_cwd: cwd,
            file_tree_entries: entries,
            file_tree_error: error,
            servers_snapshot,
            collapsed_server_groups: BTreeSet::new(),
        }
    }

    // ─── Tab switch ───
    pub fn select_tab(&mut self, tab: LeftTab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

    #[allow(dead_code)]
    pub fn active_tab(&self) -> LeftTab {
        self.active_tab
    }

    pub fn file_tree_cwd(&self) -> PathBuf {
        self.file_tree_cwd.clone()
    }

    // ─── File browser navigation ───
    pub fn enter_dir(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match file_tree::list_dir(&path) {
            Ok(entries) => {
                self.file_tree_cwd = path.clone();
                self.file_tree_entries = entries;
                self.file_tree_error = None;
                let _ = self.weak_app.update(cx, |app, cx| {
                    app.sync_git_cwd(path, cx);
                });
            }
            Err(err) => {
                self.file_tree_error = Some(format!("{err}"));
            }
        }
        cx.notify();
    }

    pub fn cd_up(&mut self, cx: &mut Context<Self>) {
        if let Some(parent) = self.file_tree_cwd.parent() {
            let parent = parent.to_path_buf();
            self.enter_dir(parent, cx);
        }
    }

    pub fn refresh_cwd(&mut self, cx: &mut Context<Self>) {
        let cwd = self.file_tree_cwd.clone();
        self.enter_dir(cwd, cx);
    }

    fn toggle_server_group(&mut self, group: &str, cx: &mut Context<Self>) {
        if self.collapsed_server_groups.contains(group) {
            self.collapsed_server_groups.remove(group);
        } else {
            self.collapsed_server_groups.insert(group.to_string());
        }
        cx.notify();
    }
}

impl Render for LeftPanelView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let active_tab = self.active_tab;

        let body = match active_tab {
            LeftTab::Files => self.render_files(&t, cx).into_any_element(),
            LeftTab::Servers => self.render_servers(&t, cx).into_any_element(),
        };

        div()
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .bg(t.color.bg_panel)
            .border_r_1()
            .border_color(t.color.border_subtle)
            .child(self.render_tab_bar(&t, active_tab, cx))
            .child(div().w_full().flex_1().min_h(px(0.0)).child(body))
    }
}

impl LeftPanelView {
    fn render_tab_bar(
        &self,
        t: &crate::theme::Theme,
        active: LeftTab,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut row = div()
            .h(px(36.0))
            .px(SP_2)
            .py(SP_1)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1)
            .bg(t.color.bg_surface)
            .border_b_1()
            .border_color(t.color.border_subtle);

        for tab in LeftTab::ALL {
            let is_active = tab == active;
            let id_str: SharedString = format!("left-tab-{}", tab.id()).into();
            let icon = match tab {
                LeftTab::Files => IconName::Folder,
                LeftTab::Servers => IconName::Globe,
            };
            let on_click = cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.select_tab(tab, cx);
            });

            let mut btn = div()
                .id(gpui::ElementId::Name(id_str))
                .h(px(24.0))
                .px(SP_3)
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_1_5)
                .rounded(RADIUS_SM)
                .text_size(SIZE_BODY)
                .font_weight(WEIGHT_MEDIUM)
                .cursor_pointer()
                .text_color(if is_active {
                    t.color.accent
                } else {
                    t.color.text_secondary
                })
                .hover(|s| s.bg(t.color.bg_hover).text_color(t.color.text_primary))
                .on_click(on_click)
                .child(UiIcon::new(icon).size(px(12.0)).text_color(if is_active {
                    t.color.accent
                } else {
                    t.color.text_secondary
                }))
                .child(tab.label());

            if is_active {
                btn = btn
                    .bg(t.color.accent_subtle)
                    .border_1()
                    .border_color(t.color.accent_muted);
            }
            row = row.child(btn);
        }
        row
    }

    fn render_files(&self, t: &crate::theme::Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // Capture handlers for FileTree — all bridge back to LeftPanelView
        // methods (file ops never touch PierApp directly, except .md → markdown).
        let on_enter_dir: file_tree::EnterDirHandler =
            Rc::new(cx.listener(|this, path: &PathBuf, _w, cx| this.enter_dir(path.clone(), cx)));
        let weak_app = self.weak_app.clone();
        let on_open_file: file_tree::OpenFileHandler =
            Rc::new(move |path: &PathBuf, _w, app_cx| {
                // .md → forward to PierApp (Markdown mode renders the file).
                let path = path.clone();
                if path
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("md"))
                    .unwrap_or(false)
                {
                    let _ = weak_app.update(app_cx, |pa, cx| {
                        pa.open_markdown_file(path, cx);
                    });
                } else {
                    eprintln!("[pier] file opened: {}", path.display());
                }
            });
        let on_go_up: file_tree::GoUpHandler =
            Rc::new(cx.listener(|this, _: &(), _w, cx| this.cd_up(cx)));
        let on_refresh: file_tree::RefreshHandler =
            Rc::new(cx.listener(|this, _: &(), _w, cx| this.refresh_cwd(cx)));
        let on_navigate_to: file_tree::NavigateToHandler =
            Rc::new(cx.listener(|this, path: &PathBuf, _w, cx| this.enter_dir(path.clone(), cx)));

        let filter_value = self.files_filter.read(cx).value().to_string();
        let file_tree = FileTree::new(
            self.file_tree_cwd.clone(),
            self.file_tree_entries.clone(),
            self.file_tree_error.clone(),
            filter_value,
            on_enter_dir,
            on_open_file,
            on_go_up,
            on_refresh,
            on_navigate_to,
        );

        div()
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .px(SP_2)
                    .py(SP_2)
                    .bg(t.color.bg_surface)
                    .border_b_1()
                    .border_color(t.color.border_subtle)
                    .child(Input::new(&self.files_filter)),
            )
            .child(div().flex_1().min_h(px(0.0)).child(file_tree))
    }

    fn render_servers(&self, t: &crate::theme::Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.servers_filter.read(cx).value().to_string();

        let on_select = cx.listener(|this, idx: &usize, w, cx| {
            let idx = *idx;
            let _ = this
                .weak_app
                .update(cx, |pa, cx| pa.open_ssh_terminal(idx, cx));
            let _ = w; // silence unused
        });
        let on_edit = cx.listener(|this, idx: &usize, window, cx| {
            let idx = *idx;
            let _ = this
                .weak_app
                .update(cx, |pa, cx| pa.open_edit_connection(idx, window, cx));
        });
        let on_delete = cx.listener(|this, idx: &usize, window, cx| {
            let idx = *idx;
            let _ = this
                .weak_app
                .update(cx, |pa, cx| pa.confirm_delete_connection(idx, window, cx));
        });
        let on_add = cx.listener(|this, _: &ClickEvent, window, cx| {
            let _ = this
                .weak_app
                .update(cx, |pa, cx| pa.open_add_connection(window, cx));
        });
        let on_toggle_group = cx.listener(|this, group: &SharedString, _w, cx| {
            this.toggle_server_group(group.as_ref(), cx);
        });

        div()
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .px(SP_2)
                    .py(SP_2)
                    .bg(t.color.bg_surface)
                    .border_b_1()
                    .border_color(t.color.border_subtle)
                    .child(Input::new(&self.servers_filter)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .child(render_servers_list(
                        t,
                        &self.servers_snapshot,
                        &self.collapsed_server_groups,
                        &query,
                        Rc::new(on_select),
                        Rc::new(on_edit),
                        Rc::new(on_delete),
                        Rc::new(on_add),
                        Rc::new(on_toggle_group),
                    )),
            )
    }
}

// ─────────────────────────────────────────────────────────
// Servers list (unchanged from old left_panel.rs, just relocated)
// ─────────────────────────────────────────────────────────

type ServerSelector = Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>;
type AddConnectionHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type GroupToggleHandler = Rc<dyn Fn(&SharedString, &mut Window, &mut App) + 'static>;

#[allow(clippy::too_many_arguments)]
fn render_servers_list(
    t: &crate::theme::Theme,
    snapshot: &ServersSidebarSnapshot,
    collapsed_groups: &BTreeSet<String>,
    query: &str,
    on_select: ServerSelector,
    on_edit: ServerSelector,
    on_delete: ServerSelector,
    on_add: AddConnectionHandler,
    on_toggle_group: GroupToggleHandler,
) -> impl IntoElement {
    let mut col = div().p(SP_2).flex().flex_col().gap(SP_2);

    if let Some(active_session) = snapshot.active_session.as_ref() {
        col = col.child(active_connection_card(active_session));
    }

    col = col.child(servers_header(
        t,
        snapshot.connections.len(),
        on_add.clone(),
    ));

    if snapshot.connections.is_empty() {
        col = col.child(servers_empty_state(on_add));
        return col;
    }

    let groups = group_servers(snapshot, query);
        if groups.is_empty() {
            col = col.child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child(SharedString::from(
                        t!("App.Common.no_matches", query = query).to_string(),
                    )),
            );
            return col;
        }
    for group in groups {
        let is_collapsed = collapsed_groups.contains(&group.key);
        col = col.child(server_group_card(
            t,
            &group,
            snapshot.active_session.as_ref(),
            is_collapsed,
            on_toggle_group.clone(),
            on_select.clone(),
            on_edit.clone(),
            on_delete.clone(),
        ));
    }
    col
}

fn servers_header(
    t: &crate::theme::Theme,
    count: usize,
    on_add: AddConnectionHandler,
) -> impl IntoElement {
    let on_add_click = on_add.clone();
    div()
        .h(px(28.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_2)
        .py(SP_1)
        .rounded(RADIUS_SM)
        .bg(t.color.bg_surface)
        .border_1()
        .border_color(t.color.border_subtle)
        .child(SectionLabel::new(t!("App.LeftPanel.Headers.saved_connections")))
        .child(StatusPill::new(
            format!("{count}"),
            if count == 0 {
                StatusKind::Info
            } else {
                StatusKind::Success
            },
        ))
        .child(div().flex_1())
        .child(
            div()
                .id("servers-add")
                .w(px(22.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_SM)
                .bg(t.color.bg_panel)
                .border_1()
                .border_color(t.color.border_subtle)
                .text_color(t.color.text_secondary)
                .cursor_pointer()
                .hover(|s| s.bg(t.color.bg_hover).border_color(t.color.border_default))
                .on_click(move |ev, window, app| on_add_click(ev, window, app))
                .child(
                    UiIcon::new(IconName::Plus)
                        .size(px(12.0))
                        .text_color(t.color.text_secondary),
                ),
        )
}

struct ServerGroup<'a> {
    key: String,
    label: SharedString,
    active_count: usize,
    items: Vec<ServerGroupItem<'a>>,
}

struct ServerGroupItem<'a> {
    idx: usize,
    conn: &'a SshConfig,
    is_active: bool,
}

fn group_servers<'a>(snapshot: &'a ServersSidebarSnapshot, query: &str) -> Vec<ServerGroup<'a>> {
    let q = query.trim().to_lowercase();
    let mut groups: BTreeMap<String, Vec<ServerGroupItem<'a>>> = BTreeMap::new();

    for (idx, conn) in snapshot.connections.iter().enumerate() {
        if !connection_matches_query(conn, &q) {
            continue;
        }

        let key = group_key_for_connection(conn);
        groups.entry(key).or_default().push(ServerGroupItem {
            idx,
            conn,
            is_active: connection_is_active(snapshot, conn),
        });
    }

    groups
        .into_iter()
        .map(|(key, items)| {
            let active_count = items.iter().filter(|item| item.is_active).count();
            ServerGroup {
                label: group_display_label(&key),
                key,
                active_count,
                items,
            }
        })
        .collect()
}

fn group_key_for_connection(conn: &SshConfig) -> String {
    conn.tags
        .first()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| UNGROUPED.to_string())
}

fn connection_matches_query(conn: &SshConfig, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    conn.name.to_lowercase().contains(query)
        || conn.host.to_lowercase().contains(query)
        || conn.user.to_lowercase().contains(query)
        || conn
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(query))
}

fn connection_is_active(snapshot: &ServersSidebarSnapshot, conn: &SshConfig) -> bool {
    snapshot
        .active_session
        .as_ref()
        .is_some_and(|active| active.config == *conn)
}

fn prune_collapsed_groups(collapsed_groups: &mut BTreeSet<String>, connections: &[SshConfig]) {
    let live_groups = connections
        .iter()
        .map(group_key_for_connection)
        .collect::<BTreeSet<_>>();
    collapsed_groups.retain(|group| live_groups.contains(group));
}

fn connections_store_label() -> String {
    paths::connections_file()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| t!("App.LeftPanel.connection_store_fallback").to_string())
}

fn group_display_label(key: &str) -> SharedString {
    if key == UNGROUPED {
        t!("App.LeftPanel.Groups.ungrouped").into()
    } else {
        key.to_string().into()
    }
}

#[allow(clippy::too_many_arguments)]
fn server_group_card(
    t: &crate::theme::Theme,
    group: &ServerGroup<'_>,
    active_session: Option<&ActiveServerSessionSnapshot>,
    is_collapsed: bool,
    on_toggle_group: GroupToggleHandler,
    on_select: ServerSelector,
    on_edit: ServerSelector,
    on_delete: ServerSelector,
) -> impl IntoElement {
    let toggle_key: SharedString = group.key.clone().into();
    let toggle_id: SharedString = format!("servers-group-{}", group.key).into();
    let marker = if is_collapsed { ">" } else { "v" };

    let card = Card::new().padding(SP_2).child(
        div()
            .id(gpui::ElementId::Name(toggle_id))
            .h(px(24.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .cursor_pointer()
            .hover(|style| style.bg(t.color.bg_hover))
            .on_click(move |_, window, app| on_toggle_group(&toggle_key, window, app))
            .child(
                div()
                    .w(px(10.0))
                    .text_size(SIZE_CAPTION)
                    .font_family(t.font_mono.clone())
                    .text_color(t.color.text_tertiary)
                    .child(marker),
            )
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(t.color.text_secondary)
                    .child(group.label.clone()),
            )
            .child(StatusPill::new(
                format!("{}", group.items.len()),
                StatusKind::Info,
            ))
            .child(if group.active_count > 0 {
                StatusPill::new(
                    t!("App.LeftPanel.Groups.live_count", count = group.active_count),
                    StatusKind::Success,
                )
                .into_any_element()
            } else {
                div().into_any_element()
            }),
    );

    if is_collapsed {
        return card;
    }

    let mut rows = div().pt(SP_2).flex().flex_col().gap(SP_2);
    for item in &group.items {
        let row_session = active_session.filter(|session| session.config == *item.conn);
        rows = rows.child(server_row(
            t,
            item.idx,
            item.conn,
            item.is_active,
            row_session,
            on_select.clone(),
            on_edit.clone(),
            on_delete.clone(),
        ));
    }

    card.child(rows)
}

fn active_connection_card(session: &ActiveServerSessionSnapshot) -> impl IntoElement {
    let endpoint = connection_endpoint(&session.config);
    let (connect_label, connect_kind) = connection_status_pill(session.status);
    let (probe_label, probe_kind) =
        service_probe_pill(&session.service_probe_status, !session.services.is_empty());

    let mut card = Card::new()
        .padding(SP_3)
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(SP_2)
                .child(SectionLabel::new(t!("App.LeftPanel.Headers.active_connection")))
                .child(StatusPill::new(connect_label, connect_kind))
                .child(StatusPill::new(probe_label, probe_kind)),
        )
        .child(text::body(session.config.name.clone()))
        .child(text::mono(endpoint).secondary());

    if !session.services.is_empty() {
        let mut services = div().pt(SP_2).flex().flex_row().flex_wrap().gap(SP_2);
        for service in &session.services {
            services = services.child(StatusPill::new(
                service_label(service),
                service_status_kind(service.status),
            ));
        }
        card = card
            .child(text::caption(t!("App.LeftPanel.Headers.detected_services")).secondary())
            .child(services);
    } else {
        let services_empty = match session.service_probe_status {
            ServiceProbeStatus::Idle => t!("App.LeftPanel.Services.discovery_idle"),
            ServiceProbeStatus::Probing => t!("App.LeftPanel.Services.discovery_loading"),
            ServiceProbeStatus::Ready => t!("App.LeftPanel.Services.discovery_empty"),
            ServiceProbeStatus::Failed => t!("App.LeftPanel.Services.discovery_failed"),
        };
        card = card.child(text::body(services_empty).secondary());
    }

    if !session.tunnels.is_empty() {
        let mut tunnels = div().pt(SP_2).flex().flex_row().flex_wrap().gap(SP_2);
        for tunnel in &session.tunnels {
            tunnels = tunnels.child(StatusPill::new(
                tunnel_label(tunnel),
                tunnel_status_kind(tunnel.status),
            ));
        }
        card = card
            .child(text::caption(t!("App.LeftPanel.Headers.active_tunnels")).secondary())
            .child(tunnels);
    } else {
        card = card.child(text::body(t!("App.LeftPanel.Tunnels.empty")).secondary());
    }

    if let Some(err) = session
        .service_probe_error
        .as_ref()
        .or(session.last_error.as_ref())
    {
        card = card.child(text::body(err.clone()).secondary());
    }

    card
}

fn servers_empty_state(on_add: AddConnectionHandler) -> impl IntoElement {
    let on_add_click = on_add.clone();
    let store_path: SharedString = connections_store_label().into();

    Card::new()
        .padding(SP_3)
        .child(SectionLabel::new(t!("App.LeftPanel.Empty.title")))
        .child(
            text::body(t!("App.LeftPanel.Empty.body")).secondary(),
        )
        .child(
            div()
                .pt(SP_2)
                .child(
                    Button::primary("servers-empty-add", t!("App.LeftPanel.Actions.add_ssh"))
                        .on_click(move |ev, window, app| on_add_click(ev, window, app)),
                ),
        )
        .child(
            div()
                .pt(SP_2)
                .flex()
                .flex_col()
                .gap(SP_1)
                .child(text::caption(t!("App.LeftPanel.Headers.connection_store")).secondary())
                .child(text::mono(store_path).secondary()),
        )
}

fn server_row(
    t: &crate::theme::Theme,
    idx: usize,
    conn: &SshConfig,
    is_active: bool,
    active_session: Option<&ActiveServerSessionSnapshot>,
    on_select: ServerSelector,
    on_edit: ServerSelector,
    on_delete: ServerSelector,
) -> impl IntoElement {
    let address: SharedString = format!("{}@{}:{}", conn.user, conn.host, conn.port).into();
    let auth: SharedString = match &conn.auth {
        AuthMethod::Agent => t!("App.LeftPanel.Auth.agent").into(),
        AuthMethod::PublicKeyFile { .. } => t!("App.LeftPanel.Auth.key").into(),
        AuthMethod::KeychainPassword { .. } => t!("App.LeftPanel.Auth.keychain").into(),
        AuthMethod::DirectPassword { .. } => t!("App.LeftPanel.Auth.password").into(),
    };
    let name: SharedString = conn.name.clone().into();
    let row_id: SharedString = format!("left-server-{idx}").into();
    let edit_id: SharedString = format!("left-server-edit-{idx}").into();
    let delete_id: SharedString = format!("left-server-delete-{idx}").into();
    let active_pill = active_session.map(|session| connection_status_pill(session.status));

    div()
        .id(gpui::ElementId::Name(row_id))
        .flex()
        .flex_col()
        .gap(SP_1)
        .px(SP_2)
        .py(SP_2)
        .rounded(RADIUS_SM)
        .bg(if is_active {
            t.color.accent_subtle
        } else {
            t.color.bg_surface
        })
        .border_1()
        .border_color(if is_active {
            t.color.accent_muted
        } else {
            t.color.border_subtle
        })
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover).border_color(t.color.border_default))
        .on_click(move |_, w, app| on_select(&idx, w, app))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .child(
                    div()
                        .text_size(SIZE_BODY)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_primary)
                        .child(name),
                )
                .child(
                    div()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(auth),
                )
                .child(if let Some((label, kind)) = active_pill {
                    StatusPill::new(label, kind).into_any_element()
                } else {
                    div().into_any_element()
                })
                .child(div().flex_1())
                .child(row_action_button(
                    t,
                    edit_id,
                    IconName::Settings,
                    on_edit,
                    idx,
                ))
                .child(row_action_button(
                    t,
                    delete_id,
                    IconName::Delete,
                    on_delete,
                    idx,
                )),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(SP_2)
                .child(
                    div()
                        .text_size(SIZE_MONO_SMALL)
                        .font_family(t.font_mono.clone())
                        .text_color(t.color.text_secondary)
                        .child(address),
                )
                .child(if conn.tags.is_empty() {
                    div().into_any_element()
                } else {
                    div()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(conn.tags.join(", "))
                        .into_any_element()
                }),
        )
}

fn connection_endpoint(config: &SshConfig) -> SharedString {
    if config.port == 22 {
        format!("{}@{}", config.user, config.host).into()
    } else {
        format!("{}@{}:{}", config.user, config.host, config.port).into()
    }
}

fn connection_status_pill(status: ConnectStatus) -> (SharedString, StatusKind) {
    match status {
        ConnectStatus::Idle => (t!("App.Common.Status.idle").into(), StatusKind::Warning),
        ConnectStatus::Connecting => (t!("App.Common.Status.connecting").into(), StatusKind::Info),
        ConnectStatus::Refreshing => (t!("App.Common.Status.loading").into(), StatusKind::Info),
        ConnectStatus::Connected => (t!("App.Common.Status.connected").into(), StatusKind::Success),
        ConnectStatus::Failed => (t!("App.Common.Status.error").into(), StatusKind::Error),
    }
}

fn service_probe_pill(
    status: &ServiceProbeStatus,
    has_services: bool,
) -> (SharedString, StatusKind) {
    match status {
        ServiceProbeStatus::Idle => (t!("App.Common.Status.idle").into(), StatusKind::Warning),
        ServiceProbeStatus::Probing => {
            (t!("App.LeftPanel.Services.detecting").into(), StatusKind::Info)
        }
        ServiceProbeStatus::Ready if has_services => {
            (t!("App.LeftPanel.Services.ready").into(), StatusKind::Success)
        }
        ServiceProbeStatus::Ready => {
            (t!("App.LeftPanel.Services.none").into(), StatusKind::Warning)
        }
        ServiceProbeStatus::Failed => {
            (t!("App.LeftPanel.Services.error").into(), StatusKind::Error)
        }
    }
}

fn service_label(service: &DetectedService) -> SharedString {
    let mut label = match service.name.as_str() {
        "mysql" => "MySQL".to_string(),
        "postgresql" => "PostgreSQL".to_string(),
        "redis" => "Redis".to_string(),
        "docker" => "Docker".to_string(),
        other => other.to_string(),
    };

    if !service.version.is_empty() && service.version != "unknown" {
        label.push(' ');
        label.push_str(&service.version);
    } else if service.port > 0 {
        label.push_str(&format!(" :{}", service.port));
    }

    label.into()
}

fn service_status_kind(status: ServiceStatus) -> StatusKind {
    match status {
        ServiceStatus::Running => StatusKind::Success,
        ServiceStatus::Stopped => StatusKind::Warning,
        ServiceStatus::Installed => StatusKind::Info,
    }
}

fn tunnel_label(tunnel: &ServerTunnelSnapshot) -> SharedString {
    let service = match tunnel.service_name.as_str() {
        "mysql" => "MySQL",
        "postgresql" => "PostgreSQL",
        "redis" => "Redis",
        "docker" => "Docker",
        other => other,
    };

    match (tunnel.status, tunnel.local_port) {
        (TunnelStatus::Active, Some(local_port)) => {
            format!("{service} localhost:{local_port} -> {}", tunnel.remote_port).into()
        }
        (TunnelStatus::Opening, _) => {
            t!(
                "App.LeftPanel.Tunnels.opening",
                service = service,
                port = tunnel.remote_port
            )
            .into()
        }
        (TunnelStatus::Failed, _) => {
            t!(
                "App.LeftPanel.Tunnels.error",
                service = service,
                port = tunnel.remote_port
            )
            .into()
        }
        (TunnelStatus::Active, None) => format!("{service} -> {}", tunnel.remote_port).into(),
    }
}

fn tunnel_status_kind(status: TunnelStatus) -> StatusKind {
    match status {
        TunnelStatus::Opening => StatusKind::Info,
        TunnelStatus::Active => StatusKind::Success,
        TunnelStatus::Failed => StatusKind::Error,
    }
}

fn row_action_button(
    t: &crate::theme::Theme,
    id: SharedString,
    icon: IconName,
    handler: ServerSelector,
    idx: usize,
) -> impl IntoElement {
    div()
        .id(gpui::ElementId::Name(id))
        .w(px(20.0))
        .h(px(20.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .bg(t.color.bg_panel)
        .border_1()
        .border_color(t.color.border_subtle)
        .text_color(t.color.text_tertiary)
        .cursor_pointer()
        .hover(|s| {
            s.bg(t.color.bg_active)
                .border_color(t.color.border_default)
                .text_color(t.color.text_primary)
        })
        .on_click(move |_, w, app| {
            handler(&idx, w, app);
            app.stop_propagation();
        })
        .child(
            UiIcon::new(icon)
                .size(px(12.0))
                .text_color(t.color.text_tertiary),
        )
}

#[cfg(test)]
mod tests {
    use super::{
        group_servers, prune_collapsed_groups, ActiveServerSessionSnapshot, ServerTunnelSnapshot,
        ServersSidebarSnapshot, UNGROUPED,
    };
    use crate::app::ssh_session::{ConnectStatus, ServiceProbeStatus, TunnelStatus};
    use pier_core::ssh::{AuthMethod, DetectedService, ServiceStatus, SshConfig};
    use std::collections::BTreeSet;

    fn sample_config(name: &str, host: &str, tag: Option<&str>) -> SshConfig {
        SshConfig {
            name: name.into(),
            host: host.into(),
            port: 22,
            user: "pier".into(),
            auth: AuthMethod::Agent,
            connect_timeout_secs: 5,
            tags: tag.into_iter().map(|value| value.to_string()).collect(),
        }
    }

    #[test]
    fn group_servers_tracks_active_connection_and_ungrouped_bucket() {
        let active = sample_config("api-prod", "10.0.0.2", Some("Production"));
        let snapshot = ServersSidebarSnapshot {
            connections: vec![
                active.clone(),
                sample_config("db-prod", "10.0.0.3", Some("Production")),
                sample_config("sandbox", "10.0.0.8", None),
            ],
            active_session: Some(ActiveServerSessionSnapshot {
                config: active,
                status: ConnectStatus::Connected,
                service_probe_status: ServiceProbeStatus::Ready,
                service_probe_error: None,
                services: vec![DetectedService {
                    name: "redis".into(),
                    version: "7.2".into(),
                    status: ServiceStatus::Running,
                    port: 6379,
                }],
                tunnels: vec![ServerTunnelSnapshot {
                    service_name: "redis".into(),
                    remote_port: 6379,
                    local_port: Some(16379),
                    status: TunnelStatus::Active,
                    last_error: None,
                }],
                last_error: None,
            }),
        };

        let groups = group_servers(&snapshot, "");
        assert_eq!(groups.len(), 2);

        let production = groups
            .iter()
            .find(|group| group.key == "Production")
            .expect("production group");
        assert_eq!(production.active_count, 1);
        assert!(production.items.iter().any(|item| item.is_active));

        let ungrouped = groups
            .iter()
            .find(|group| group.key == UNGROUPED)
            .expect("ungrouped group");
        assert_eq!(ungrouped.items.len(), 1);
        assert_eq!(ungrouped.active_count, 0);
    }

    #[test]
    fn prune_collapsed_groups_drops_removed_group_keys() {
        let mut collapsed = BTreeSet::from(["Production".to_string(), "Legacy".to_string()]);
        let connections = vec![
            sample_config("api-prod", "10.0.0.2", Some("Production")),
            sample_config("sandbox", "10.0.0.8", None),
        ];

        prune_collapsed_groups(&mut collapsed, &connections);

        assert!(collapsed.contains("Production"));
        assert!(!collapsed.contains("Legacy"));
    }
}

// ─────────────────────────────────────────────────────────
// Toolbar icon constants — kept here to preserve the public path used by
// `app/state.rs` (`crate::views::left_panel::icons` previously). After
// dropping `views/left_panel.rs` the toolbar imports `left_panel_view::icons`.
// ─────────────────────────────────────────────────────────

pub mod icons {
    use gpui_component::IconName;

    pub const TOGGLE_LEFT_OPEN: IconName = IconName::PanelLeftClose;
    pub const TOGGLE_LEFT_CLOSED: IconName = IconName::PanelLeftOpen;
    pub const TOGGLE_RIGHT_OPEN: IconName = IconName::PanelRightClose;
    pub const TOGGLE_RIGHT_CLOSED: IconName = IconName::PanelRightOpen;
    pub const NEW_TAB: IconName = IconName::Plus;
    pub const SUN: IconName = IconName::Sun;
    pub const MOON: IconName = IconName::Moon;
}
