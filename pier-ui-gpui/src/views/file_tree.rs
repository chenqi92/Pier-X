//! Local file tree — render-only view backed by a cache owned by [`PierApp`].
//!
//! Mirrors `Pier/PierApp/Sources/Views/FilePanel/LocalFileView.swift`.
//!
//! ## Perf invariant
//!
//! **No filesystem IO is performed during `render`.** All directory listings
//! are pre-cached in `PierApp::file_tree_root_entries` and
//! `PierApp::file_tree_children`, populated lazily on user actions
//! (expand a directory / cd_up). Earlier versions of this file did
//! `std::fs::read_dir` on every render which froze the UI for hundreds of
//! milliseconds on every keystroke / tab switch — see CLAUDE.md
//! "Render is paint-only" rule.
//!
//! ## Scope
//!
//! Ships:
//!   - lazy expand/collapse of directories on click
//!   - depth-indented rows with chevron / folder / file glyphs
//!   - header with current root + "go up" button
//!   - file click → callback (feeds right-panel Markdown mode)
//!
//! Deferred:
//!   - drag-and-drop into terminal
//!   - right-click context menu
//!   - file system change watcher (notify crate or DispatchSource)
//!   - explicit "refresh" button (kbd shortcut?)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{div, prelude::*, px, App, IntoElement, SharedString, Window};
use gpui_component::{Icon as UiIcon, IconName};

use crate::components::{text, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};

/// Rows past this depth are clamped (defensive — symlink loops etc.).
const MAX_DEPTH: u32 = 32;
/// Children past this many entries are truncated with a "+ N more" footer.
const MAX_CHILDREN_PER_DIR: usize = 500;
/// Width step per nesting level.
const INDENT_PER_DEPTH: f32 = 14.0;

pub type ToggleDirHandler = Rc<dyn Fn(&PathBuf, &mut Window, &mut App) + 'static>;
pub type OpenFileHandler = Rc<dyn Fn(&PathBuf, &mut Window, &mut App) + 'static>;
/// `cx.listener` returns a closure with a `&E` first argument, so we pass
/// `&()` for buttons that don't carry payload — keeps wiring uniform.
pub type GoUpHandler = Rc<dyn Fn(&(), &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct FileTree {
    root: PathBuf,
    /// Pre-loaded entries for `root`. Owned by [`PierApp`].
    root_entries: Vec<FsEntry>,
    /// Pre-loaded children indexed by directory path. Empty for collapsed
    /// or never-expanded dirs.
    children: HashMap<PathBuf, Vec<FsEntry>>,
    expanded: HashSet<PathBuf>,
    /// Case-insensitive substring filter. Empty = show everything.
    /// When non-empty, entries whose name doesn't contain the query are
    /// hidden — but expanded dirs still recurse so a match deep in the
    /// tree is reachable through its (matching) ancestor.
    filter: String,
    /// `Some(err)` when the root directory itself failed to list (perm
    /// denied, etc.). Rendered as a status pill instead of an empty list.
    root_error: Option<String>,
    on_toggle_dir: ToggleDirHandler,
    on_open_file: OpenFileHandler,
    on_go_up: GoUpHandler,
}

impl FileTree {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: PathBuf,
        root_entries: Vec<FsEntry>,
        children: HashMap<PathBuf, Vec<FsEntry>>,
        expanded: HashSet<PathBuf>,
        filter: String,
        root_error: Option<String>,
        on_toggle_dir: ToggleDirHandler,
        on_open_file: OpenFileHandler,
        on_go_up: GoUpHandler,
    ) -> Self {
        Self {
            root,
            root_entries,
            children,
            expanded,
            filter,
            root_error,
            on_toggle_dir,
            on_open_file,
            on_go_up,
        }
    }
}

impl RenderOnce for FileTree {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let FileTree {
            root,
            root_entries,
            children,
            expanded,
            filter,
            root_error,
            on_toggle_dir,
            on_open_file,
            on_go_up,
        } = self;
        let filter_lower = filter.to_lowercase();

        let root_label: SharedString = root.display().to_string().into();

        // Header: root path (mono) + go-up button.
        let header = div()
            .h(px(28.0))
            .px(SP_2)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .border_b_1()
            .border_color(t.color.border_subtle)
            .child(
                div()
                    .id("file-tree-up")
                    .w(px(20.0))
                    .h(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(RADIUS_SM)
                    .text_color(t.color.text_secondary)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.color.bg_hover))
                    .on_click({
                        let go_up = on_go_up.clone();
                        move |_, w, app| go_up(&(), w, app)
                    })
                    .child(UiIcon::new(IconName::ArrowUp).size(px(12.0))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(SIZE_MONO_SMALL)
                    .font_family(t.font_mono.clone())
                    .text_color(t.color.text_tertiary)
                    .child(root_label),
            );

        // Body rows — read entirely from the cache, never from disk.
        let mut body = div().flex().flex_col().py(SP_1);
        if let Some(err) = root_error {
            body = body.child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .flex()
                    .flex_col()
                    .gap(SP_1)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(SP_2)
                            .child(SectionLabel::new("Cannot read directory"))
                            .child(StatusPill::new("io error", StatusKind::Error)),
                    )
                    .child(text::body(SharedString::from(err)).secondary()),
            );
        } else if root_entries.is_empty() {
            body = body.child(
                div()
                    .px(SP_3)
                    .py(SP_2)
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child("(empty directory)"),
            );
        } else {
            for entry in &root_entries {
                body = render_entry(
                    body,
                    t,
                    entry,
                    0,
                    &expanded,
                    &children,
                    &filter_lower,
                    on_toggle_dir.clone(),
                    on_open_file.clone(),
                );
            }
        }

        div()
            .h_full()
            .flex()
            .flex_col()
            .child(header)
            .child(div().flex_1().min_h(px(0.0)).child(body))
    }
}

// ─────────────────────────────────────────────────────────
// Recursive row renderer
// ─────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn render_entry(
    mut col: gpui::Div,
    t: &crate::theme::Theme,
    entry: &FsEntry,
    depth: u32,
    expanded: &HashSet<PathBuf>,
    children: &HashMap<PathBuf, Vec<FsEntry>>,
    filter: &str,
    on_toggle_dir: ToggleDirHandler,
    on_open_file: OpenFileHandler,
) -> gpui::Div {
    if depth > MAX_DEPTH {
        return col;
    }

    let is_expanded = expanded.contains(&entry.path);

    // Filter: entries whose name doesn't contain the query are hidden
    // unless they're expanded directories (the user explicitly wants to
    // see inside, even if the dir name is irrelevant to the query).
    let name_matches = filter.is_empty() || entry.name.to_lowercase().contains(filter);
    if !name_matches && !(entry.is_dir && is_expanded) {
        return col;
    }

    col = col.child(row(t, entry, depth, is_expanded, {
        let path = entry.path.clone();
        let toggle = on_toggle_dir.clone();
        let open = on_open_file.clone();
        let is_dir = entry.is_dir;
        move |w, app| {
            if is_dir {
                toggle(&path, w, app);
            } else {
                open(&path, w, app);
            }
        }
    }));

    if entry.is_dir && is_expanded {
        if let Some(child_entries) = children.get(&entry.path) {
            for child in child_entries {
                col = render_entry(
                    col,
                    t,
                    child,
                    depth + 1,
                    expanded,
                    children,
                    filter,
                    on_toggle_dir.clone(),
                    on_open_file.clone(),
                );
            }
        } else {
            // Cache miss — should be rare (only if the user expanded a
            // dir while we were reloading). Show a hint instead of doing
            // sync IO here.
            let indent: f32 =
                f32::from(SP_3) + (depth as f32 + 1.0) * INDENT_PER_DEPTH;
            col = col.child(
                div()
                    .px(px(indent))
                    .py(SP_1)
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child("(loading…)"),
            );
        }
    }
    col
}

fn row(
    t: &crate::theme::Theme,
    entry: &FsEntry,
    depth: u32,
    is_expanded: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id_str: SharedString = format!("file-tree-{}", entry.path.display()).into();
    let label: SharedString = entry.name.clone().into();
    let indent: f32 = f32::from(SP_3) + depth as f32 * INDENT_PER_DEPTH;

    let chevron = if entry.is_dir {
        let name = if is_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };
        Some(UiIcon::new(name).size(px(12.0)))
    } else {
        None
    };
    let glyph = if entry.is_dir {
        if is_expanded {
            IconName::FolderOpen
        } else {
            IconName::Folder
        }
    } else {
        IconName::File
    };

    let mut row = div()
        .id(gpui::ElementId::Name(id_str))
        .h(px(20.0))
        .pl(px(indent))
        .pr(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .text_size(SIZE_CAPTION)
        .text_color(if entry.is_dir {
            t.color.text_primary
        } else {
            t.color.text_secondary
        })
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(move |_, w, app| on_click(w, app));

    if let Some(ch) = chevron {
        row = row.child(
            div()
                .w(px(12.0))
                .h(px(12.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.color.text_tertiary)
                .child(ch),
        );
    } else {
        row = row.child(div().w(px(12.0)));
    }

    row.child(
        div()
            .w(px(14.0))
            .h(px(14.0))
            .flex()
            .items_center()
            .justify_center()
            .text_color(if entry.is_dir {
                t.color.accent
            } else {
                t.color.text_tertiary
            })
            .child(UiIcon::new(glyph).size(px(12.0))),
    )
    .child(
        div()
            .flex_1()
            .min_w(px(0.0))
            .text_size(SIZE_CAPTION)
            .font_weight(WEIGHT_MEDIUM)
            .child(label),
    )
}

// ─────────────────────────────────────────────────────────
// Filesystem listing
// ─────────────────────────────────────────────────────────

/// Single directory entry as seen by [`list_dir`]. Held in
/// [`crate::app::state::PierApp`]'s cache; rendered by [`FileTree`] without
/// further IO.
#[derive(Clone, Debug)]
pub struct FsEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// Read a single directory level from disk. Called from PierApp's expand /
/// cd_up handlers — never from a `Render::render` body.
pub fn list_dir(root: &Path) -> std::io::Result<Vec<FsEntry>> {
    let mut entries: Vec<FsEntry> = std::fs::read_dir(root)?
        .filter_map(|res| res.ok())
        .filter(|entry| {
            // Hide dotfiles by default — Pier does the same.
            entry
                .file_name()
                .to_str()
                .map(|s| !s.starts_with('.'))
                .unwrap_or(true)
        })
        .map(|entry| {
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            FsEntry {
                name: entry
                    .file_name()
                    .to_string_lossy()
                    .into_owned(),
                path: entry.path(),
                is_dir,
            }
        })
        .collect();

    // Directories first, then files; both alphabetical (case-insensitive).
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    if entries.len() > MAX_CHILDREN_PER_DIR {
        entries.truncate(MAX_CHILDREN_PER_DIR);
    }
    Ok(entries)
}
