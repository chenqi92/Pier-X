//! Left panel — Files / Servers tab switcher.
//!
//! Mirrors `Pier/PierApp/Sources/Views/LeftPanel/LeftPanelView.swift`.
//! Files tab now hosts the real lazy-load tree from
//! [`crate::views::file_tree::FileTree`]; Servers tab is a compact list of
//! saved [`SshConfig`] entries.

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::{div, prelude::*, px, App, ClickEvent, Entity, IntoElement, SharedString, Window};
use gpui_component::{
    input::{Input, InputState},
    Icon as UiIcon, IconName,
};
use pier_core::ssh::{AuthMethod, SshConfig};

use crate::app::layout::LeftTab;
use crate::components::{text, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_BODY, SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};
use crate::views::file_tree::FileTree;

pub type TabSelector = Rc<dyn Fn(&LeftTab, &mut Window, &mut App) + 'static>;
pub type ServerSelector = Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>;
pub type AddConnectionHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Sentinel group label used when an [`SshConfig`] has no `tags`. Sorted
/// last in the rendered list (BTreeMap ordering pulls it before `~`-prefixed
/// alphabetical groups; we work around that via the `(usize, &str)` key
/// trick — see [`group_servers`]).
const UNGROUPED: &str = "Ungrouped";

#[derive(IntoElement)]
pub struct LeftPanel {
    active_tab: LeftTab,
    connections: Vec<SshConfig>,
    file_tree: FileTree,
    files_filter: Entity<InputState>,
    servers_filter: Entity<InputState>,
    /// Pre-computed servers query (read once by PierApp) so the row builder
    /// doesn't need an `&mut App` borrow during list construction.
    servers_query: String,
    on_select_tab: TabSelector,
    on_select_server: ServerSelector,
    on_edit_server: ServerSelector,
    on_delete_server: ServerSelector,
    on_add_connection: AddConnectionHandler,
}

impl LeftPanel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        active_tab: LeftTab,
        connections: Vec<SshConfig>,
        file_tree: FileTree,
        files_filter: Entity<InputState>,
        servers_filter: Entity<InputState>,
        servers_query: String,
        on_select_tab: TabSelector,
        on_select_server: ServerSelector,
        on_edit_server: ServerSelector,
        on_delete_server: ServerSelector,
        on_add_connection: AddConnectionHandler,
    ) -> Self {
        Self {
            active_tab,
            connections,
            file_tree,
            files_filter,
            servers_filter,
            servers_query,
            on_select_tab,
            on_select_server,
            on_edit_server,
            on_delete_server,
            on_add_connection,
        }
    }
}

impl RenderOnce for LeftPanel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let LeftPanel {
            active_tab,
            connections,
            file_tree,
            files_filter,
            servers_filter,
            servers_query,
            on_select_tab,
            on_select_server,
            on_edit_server,
            on_delete_server,
            on_add_connection,
        } = self;

        let body = match active_tab {
            LeftTab::Files => div()
                .h_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(SP_2)
                        .py(SP_1)
                        .child(Input::new(&files_filter)),
                )
                .child(div().flex_1().min_h(px(0.0)).child(file_tree))
                .into_any_element(),
            LeftTab::Servers => div()
                .h_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(SP_2)
                        .py(SP_1)
                        .child(Input::new(&servers_filter)),
                )
                .child(
                    div().flex_1().min_h(px(0.0)).child(render_servers_list(
                        t,
                        &connections,
                        &servers_query,
                        on_select_server,
                        on_edit_server,
                        on_delete_server,
                        on_add_connection,
                    )),
                )
                .into_any_element(),
        };

        div()
            .h_full()
            .flex()
            .flex_col()
            .bg(t.color.bg_panel)
            .border_r_1()
            .border_color(t.color.border_subtle)
            .child(render_tab_bar(t, active_tab, on_select_tab))
            .child(
                div()
                    .h(px(1.0))
                    .w_full()
                    .bg(t.color.border_subtle),
            )
            .child(div().flex_1().min_h(px(0.0)).child(body))
    }
}

fn render_tab_bar(
    t: &crate::theme::Theme,
    active: LeftTab,
    on_select: TabSelector,
) -> impl IntoElement {
    let mut row = div()
        .h(px(32.0))
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1);

    for tab in LeftTab::ALL {
        let is_active = tab == active;
        let select = on_select.clone();
        let label = tab.label();
        let id_str: SharedString = format!("left-tab-{}", tab.id()).into();
        let icon = match tab {
            LeftTab::Files => UiIcon::new(icons::FILES),
            LeftTab::Servers => UiIcon::new(icons::SERVERS),
        };

        let mut btn = div()
            .id(gpui::ElementId::Name(id_str))
            .h(px(22.0))
            .px(SP_2)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1_5)
            .rounded(RADIUS_SM)
            .text_size(SIZE_CAPTION)
            .font_weight(WEIGHT_MEDIUM)
            .cursor_pointer()
            .text_color(if is_active {
                t.color.accent
            } else {
                t.color.text_secondary
            })
            .hover(|s| s.bg(t.color.bg_hover))
            .on_click(move |_, w, app| select(&tab, w, app))
            .child(icon.size(px(12.0)))
            .child(label);

        if is_active {
            btn = btn.bg(t.color.accent_subtle);
        }
        row = row.child(btn);
    }
    row.bg(t.color.bg_panel)
}

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
    let mut col = div().p(SP_2).flex().flex_col().gap(SP_1);

    col = col.child(servers_header(t, connections.len(), on_add));

    if connections.is_empty() {
        col = col.child(
            Card::new()
                .padding(SP_2)
                .child(SectionLabel::new("No saved SSH connections"))
                .child(
                    text::body(
                        "Use the + button above (or edit ~/.config/pier-x/connections.json by hand).",
                    )
                    .secondary(),
                ),
        );
        return col;
    }

    // Group by tags[0] — keeps the existing flat ConnectionStore JSON
    // schema intact (no pier-core change) while letting users curate
    // groups by setting a single tag in the editor.
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
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .px(SP_2)
        .py(SP_1)
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
                .text_color(t.color.text_secondary)
                .cursor_pointer()
                .hover(|s| s.bg(t.color.bg_hover))
                .on_click(on_add)
                .child(UiIcon::new(IconName::Plus).size(px(12.0))),
        )
}

fn group_header(
    t: &crate::theme::Theme,
    label: &str,
    count: usize,
) -> impl IntoElement {
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

/// Bucket connections into `(group_label, Vec<(orig_idx, &SshConfig)>)`
/// pairs, with named groups sorted alphabetically and the `Ungrouped`
/// bucket pinned to the end. Connections whose name AND host don't contain
/// `query` (case-insensitive) are dropped before bucketing.
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
        match conn.tags.first().map(|s| s.trim()).filter(|s| !s.is_empty()) {
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

/// Lifetime hack — group labels live for the duration of one render and
/// are immediately consumed by [`group_header`] which clones into a
/// `SharedString`. Leaking is fine here because the leak count equals the
/// number of distinct user-defined tags (small and bounded in practice).
///
/// Trade-off chosen over allocating per-render `String`s carried in `Cow`
/// for cleaner downstream signatures. Revisit if the leak ever shows up
/// in profiling.
fn string_to_static(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
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
        .py(SP_1_5)
        .rounded(RADIUS_SM)
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover))
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

/// Trailing icon button on a server row. Calls `cx.stop_propagation()` so
/// the outer row's connect-on-click doesn't fire when the user really
/// meant "edit / delete this entry".
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
        .text_color(t.color.text_tertiary)
        .hover(|s| s.bg(t.color.bg_active).text_color(t.color.text_primary))
        .on_click(move |_, w, app| {
            handler(&idx, w, app);
            app.stop_propagation();
        })
        .child(UiIcon::new(icon).size(px(12.0)))
}

/// Re-exported icon helpers used by both the left-panel tab bar and the
/// top toolbar in `app/state.rs`. Centralised so glyph swaps land in one
/// place.
pub mod icons {
    use gpui_component::IconName;

    pub const FILES: IconName = IconName::Folder;
    pub const SERVERS: IconName = IconName::Globe;
    pub const TOGGLE_LEFT_OPEN: IconName = IconName::PanelLeftClose;
    pub const TOGGLE_LEFT_CLOSED: IconName = IconName::PanelLeftOpen;
    pub const TOGGLE_RIGHT_OPEN: IconName = IconName::PanelRightClose;
    pub const TOGGLE_RIGHT_CLOSED: IconName = IconName::PanelRightOpen;
    pub const NEW_TAB: IconName = IconName::Plus;
    pub const SUN: IconName = IconName::Sun;
    pub const MOON: IconName = IconName::Moon;
}

