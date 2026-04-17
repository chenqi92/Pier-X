//! Local file tree — Phase 2 replacement for the Files-tab placeholder.
//!
//! Mirrors `Pier/PierApp/Sources/Views/FilePanel/LocalFileView.swift`
//! at minimum-viable level. Phase 2 ships:
//!   - lazy expand/collapse of directories on click
//!   - depth-indented rows with chevron / folder / file glyphs
//!   - header with current root + "go up" button
//!   - file click → callback (used to feed the right-panel Markdown mode)
//!
//! Deferred to Phase 3+:
//!   - search / filter input
//!   - drag-and-drop into terminal
//!   - right-click context menu
//!   - file system change watcher (DispatchSourceFileSystemObject equivalent)

use std::collections::HashSet;
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
    expanded: HashSet<PathBuf>,
    on_toggle_dir: ToggleDirHandler,
    on_open_file: OpenFileHandler,
    on_go_up: GoUpHandler,
}

impl FileTree {
    pub fn new(
        root: PathBuf,
        expanded: HashSet<PathBuf>,
        on_toggle_dir: ToggleDirHandler,
        on_open_file: OpenFileHandler,
        on_go_up: GoUpHandler,
    ) -> Self {
        Self {
            root,
            expanded,
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
            expanded,
            on_toggle_dir,
            on_open_file,
            on_go_up,
        } = self;

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

        // Body rows.
        let mut body = div().flex().flex_col().py(SP_1);
        match list_dir(&root) {
            Ok(entries) => {
                if entries.is_empty() {
                    body = body.child(
                        div()
                            .px(SP_3)
                            .py(SP_2)
                            .text_size(SIZE_SMALL)
                            .text_color(t.color.text_tertiary)
                            .child("(empty directory)"),
                    );
                } else {
                    for entry in entries {
                        body = render_entry(
                            body,
                            t,
                            &entry,
                            0,
                            &expanded,
                            on_toggle_dir.clone(),
                            on_open_file.clone(),
                        );
                    }
                }
            }
            Err(err) => {
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
                        .child(
                            text::body(SharedString::from(format!("{err}")))
                                .secondary(),
                        ),
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

fn render_entry(
    mut col: gpui::Div,
    t: &crate::theme::Theme,
    entry: &FsEntry,
    depth: u32,
    expanded: &HashSet<PathBuf>,
    on_toggle_dir: ToggleDirHandler,
    on_open_file: OpenFileHandler,
) -> gpui::Div {
    if depth > MAX_DEPTH {
        return col;
    }

    col = col.child(row(t, entry, depth, expanded.contains(&entry.path), {
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

    if entry.is_dir && expanded.contains(&entry.path) {
        match list_dir(&entry.path) {
            Ok(children) => {
                for child in children {
                    col = render_entry(
                        col,
                        t,
                        &child,
                        depth + 1,
                        expanded,
                        on_toggle_dir.clone(),
                        on_open_file.clone(),
                    );
                }
            }
            Err(err) => {
                let indent: f32 =
                    f32::from(SP_3) + (depth as f32 + 1.0) * INDENT_PER_DEPTH;
                col = col.child(
                    div()
                        .px(px(indent))
                        .py(SP_1)
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(SharedString::from(format!("(error: {err})"))),
                );
            }
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

struct FsEntry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

fn list_dir(root: &Path) -> std::io::Result<Vec<FsEntry>> {
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
