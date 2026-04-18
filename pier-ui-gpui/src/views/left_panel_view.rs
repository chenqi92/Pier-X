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
//! - **`cx.observe(weak_app)`** keeps a cached `connections` snapshot in
//!   sync — fires on every PierApp `cx.notify()`, which is rare-ish
//!   (tab/mode/connection/session changes).
//!
//! ## Callbacks back to PierApp
//!
//! Server actions (open / edit / delete / add) call into PierApp via
//! `weak_app.update(cx, |pa, cx| pa.open_ssh_terminal(idx, cx))` etc.
//! File `.md` opens call `weak_app.update(cx, |pa, cx| pa.open_markdown_file(...))`.

use std::collections::BTreeMap;
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
use pier_core::ssh::{AuthMethod, SshConfig};

use crate::app::layout::LeftTab;
use crate::app::PierApp;
use crate::components::{text, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_BODY, SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};
use crate::views::file_tree::{self, FileTree, FsEntry};

const UNGROUPED: &str = "Ungrouped";

pub struct LeftPanelView {
    weak_app: WeakEntity<PierApp>,
    active_tab: LeftTab,

    files_filter: Entity<InputState>,
    servers_filter: Entity<InputState>,

    /// File browser state (formerly in PierApp).
    file_tree_cwd: PathBuf,
    file_tree_entries: Vec<FsEntry>,
    file_tree_error: Option<String>,

    /// Cached connections snapshot. Refreshed via `cx.observe(weak_app)`.
    connections: Vec<SshConfig>,
}

impl LeftPanelView {
    pub fn new(
        weak_app: WeakEntity<PierApp>,
        initial_connections: Vec<SshConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let files_filter = cx.new(|c| InputState::new(window, c).placeholder("Filter files…"));
        let servers_filter = cx.new(|c| InputState::new(window, c).placeholder("Filter servers…"));

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

        // Observe PierApp to keep our local connections cache fresh.
        if let Some(app_entity) = weak_app.upgrade() {
            cx.observe(&app_entity, |this, app, cx| {
                let next = app.read(cx).connections_snapshot();
                if next != this.connections {
                    this.connections = next;
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

        Self {
            weak_app,
            active_tab: LeftTab::Files,
            files_filter,
            servers_filter,
            file_tree_cwd: cwd,
            file_tree_entries: entries,
            file_tree_error: error,
            connections: initial_connections,
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

    // ─── File browser navigation ───
    pub fn enter_dir(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match file_tree::list_dir(&path) {
            Ok(entries) => {
                self.file_tree_cwd = path;
                self.file_tree_entries = entries;
                self.file_tree_error = None;
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
                        &self.connections,
                        &query,
                        Rc::new(on_select),
                        Rc::new(on_edit),
                        Rc::new(on_delete),
                        Box::new(on_add),
                    )),
            )
    }
}

// ─────────────────────────────────────────────────────────
// Servers list (unchanged from old left_panel.rs, just relocated)
// ─────────────────────────────────────────────────────────

type ServerSelector = Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>;
type AddConnectionHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[allow(clippy::too_many_arguments)]
fn render_servers_list(
    t: &crate::theme::Theme,
    connections: &[SshConfig],
    query: &str,
    on_select: ServerSelector,
    on_edit: ServerSelector,
    on_delete: ServerSelector,
    on_add: AddConnectionHandler,
) -> impl IntoElement {
    let mut col = div().p(SP_2).flex().flex_col().gap(SP_2);

    col = col.child(servers_header(t, connections.len(), on_add));

    if connections.is_empty() {
        col = col.child(
            Card::new()
                .padding(SP_2)
                .child(SectionLabel::new("No saved SSH connections"))
                .child(
                    text::body(format!(
                        "Use the + button above or edit {}.",
                        connections_store_label()
                    ))
                    .secondary(),
                ),
        );
        return col;
    }

    let groups = group_servers(connections, query);
    if groups.is_empty() {
        col = col.child(
            div()
                .px(SP_3)
                .py(SP_2)
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(format!("(no matches for \"{query}\")")),
        );
        return col;
    }
    for (group, items) in groups {
        col = col.child(group_header(t, group, items.len()));
        for (orig_idx, conn) in items {
            col = col.child(server_row(
                t,
                orig_idx,
                conn,
                on_select.clone(),
                on_edit.clone(),
                on_delete.clone(),
            ));
        }
    }
    col
}

fn servers_header(
    t: &crate::theme::Theme,
    count: usize,
    on_add: AddConnectionHandler,
) -> impl IntoElement {
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
        .child(SectionLabel::new("Saved connections"))
        .child(StatusPill::new(
            format!("{count}"),
            if count == 0 {
                StatusKind::Warning
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
                .on_click(on_add)
                .child(
                    UiIcon::new(IconName::Plus)
                        .size(px(12.0))
                        .text_color(t.color.text_secondary),
                ),
        )
}

fn group_header(t: &crate::theme::Theme, label: &str, count: usize) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .px(SP_3)
        .pt(SP_2)
        .child(
            div()
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_tertiary)
                .child(format!("· {count}")),
        )
}

fn group_servers<'a>(
    connections: &'a [SshConfig],
    query: &str,
) -> Vec<(&'static str, Vec<(usize, &'a SshConfig)>)> {
    let q = query.to_lowercase();
    let mut named: BTreeMap<String, Vec<(usize, &SshConfig)>> = BTreeMap::new();
    let mut ungrouped: Vec<(usize, &SshConfig)> = Vec::new();
    for (idx, conn) in connections.iter().enumerate() {
        if !q.is_empty()
            && !conn.name.to_lowercase().contains(&q)
            && !conn.host.to_lowercase().contains(&q)
        {
            continue;
        }
        match conn
            .tags
            .first()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            Some(tag) => named.entry(tag.to_string()).or_default().push((idx, conn)),
            None => ungrouped.push((idx, conn)),
        }
    }
    let mut out: Vec<(&'static str, Vec<(usize, &SshConfig)>)> = named
        .into_iter()
        .map(|(k, v)| (string_to_static(k), v))
        .collect();
    if !ungrouped.is_empty() {
        out.push((UNGROUPED, ungrouped));
    }
    out
}

/// Same Box::leak trick as the previous left_panel.rs — bounded leak count
/// equals number of distinct user tags.
fn string_to_static(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn connections_store_label() -> String {
    paths::connections_file()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "the Pier-X connection store".to_string())
}

fn server_row(
    t: &crate::theme::Theme,
    idx: usize,
    conn: &SshConfig,
    on_select: ServerSelector,
    on_edit: ServerSelector,
    on_delete: ServerSelector,
) -> impl IntoElement {
    let address: SharedString = format!("{}@{}:{}", conn.user, conn.host, conn.port).into();
    let auth: SharedString = match &conn.auth {
        AuthMethod::Agent => "agent".into(),
        AuthMethod::PublicKeyFile { .. } => "key".into(),
        AuthMethod::KeychainPassword { .. } => "keychain".into(),
        AuthMethod::DirectPassword { .. } => "password".into(),
    };
    let name: SharedString = conn.name.clone().into();
    let row_id: SharedString = format!("left-server-{idx}").into();
    let edit_id: SharedString = format!("left-server-edit-{idx}").into();
    let delete_id: SharedString = format!("left-server-delete-{idx}").into();

    div()
        .id(gpui::ElementId::Name(row_id))
        .flex()
        .flex_col()
        .gap(SP_1)
        .px(SP_2)
        .py(SP_2)
        .rounded(RADIUS_SM)
        .bg(t.color.bg_surface)
        .border_1()
        .border_color(t.color.border_subtle)
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
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_secondary)
                .child(address),
        )
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
