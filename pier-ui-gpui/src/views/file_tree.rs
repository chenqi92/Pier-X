//! Local file browser — enter-into flat-list view backed by a cwd cache
//! owned by [`crate::views::left_panel_view::LeftPanelView`].
//!
//! Mirrors `Pier/PierApp/Sources/Views/FilePanel/LocalFileView.swift`'s
//! "single-click enters directories" interaction model. NO recursive
//! expand/collapse — clicking a folder replaces the current cwd; click
//! ⤴ or a breadcrumb segment to navigate up.
//!
//! ## Perf invariant
//!
//! **No filesystem IO is performed during `render`.** All directory
//! listings are pre-cached in `LeftPanelView::file_tree_entries`,
//! populated by [`enter_dir`] / [`cd_up`] / [`refresh`] handlers on user
//! actions. See CLAUDE.md "Render is paint-only".
//!
//! ## Header (5 elements, mirrors Pier)
//!
//!   1. ⤴ Up button (disabled at `/`)
//!   2. Folder icon + cwd basename (mono)
//!   3. ⋯ quick-targets popover (Home / Desktop / Projects / Choose…)
//!   4. 🔄 Refresh button
//!   5. Breadcrumb path bar (separate row, each segment clickable)

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{div, prelude::*, px, App, Corner, IntoElement, SharedString, Window};
use gpui_component::{popover::Popover, scroll::ScrollableElement, Icon as UiIcon, IconName};
use rust_i18n::t;

use crate::components::{
    text, IconButton, IconButtonSize, IconButtonVariant, SectionLabel, StatusKind, StatusPill,
};
use crate::theme::{
    heights::{BUTTON_SM_H, GLYPH_SM, ICON_SM, ROW_MD_H, ROW_SM_H},
    radius::RADIUS_SM,
    spacing::{SP_0_5, SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};

/// Children past this many entries are truncated. Defensive for
/// pathological dirs (e.g. `/usr/bin` with thousands of entries) so the
/// element tree stays bounded.
const MAX_CHILDREN_PER_DIR: usize = 1000;

pub type EnterDirHandler = Rc<dyn Fn(&PathBuf, &mut Window, &mut App) + 'static>;
pub type OpenFileHandler = Rc<dyn Fn(&PathBuf, &mut Window, &mut App) + 'static>;
/// `cx.listener` returns a closure with a `&E` first argument, so we pass
/// `&()` for buttons that don't carry payload.
pub type GoUpHandler = Rc<dyn Fn(&(), &mut Window, &mut App) + 'static>;
pub type RefreshHandler = Rc<dyn Fn(&(), &mut Window, &mut App) + 'static>;
pub type NavigateToHandler = Rc<dyn Fn(&PathBuf, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct FileTree {
    cwd: PathBuf,
    entries: Vec<FsEntry>,
    /// `Some(err)` when listing the cwd itself failed (perm denied, etc.)
    error: Option<String>,
    /// Case-insensitive substring filter on entry names. Empty = show all.
    filter: String,
    on_enter_dir: EnterDirHandler,
    on_open_file: OpenFileHandler,
    on_go_up: GoUpHandler,
    on_refresh: RefreshHandler,
    on_navigate_to: NavigateToHandler,
}

impl FileTree {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cwd: PathBuf,
        entries: Vec<FsEntry>,
        error: Option<String>,
        filter: String,
        on_enter_dir: EnterDirHandler,
        on_open_file: OpenFileHandler,
        on_go_up: GoUpHandler,
        on_refresh: RefreshHandler,
        on_navigate_to: NavigateToHandler,
    ) -> Self {
        Self {
            cwd,
            entries,
            error,
            filter,
            on_enter_dir,
            on_open_file,
            on_go_up,
            on_refresh,
            on_navigate_to,
        }
    }
}

impl RenderOnce for FileTree {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let FileTree {
            cwd,
            entries,
            error,
            filter,
            on_enter_dir,
            on_open_file,
            on_go_up,
            on_refresh,
            on_navigate_to,
        } = self;
        let filter_lower = filter.to_lowercase();
        let cwd_name: SharedString = cwd
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| cwd.display().to_string())
            .into();
        let at_root = cwd.parent().is_none();

        // ── Header row ──
        let header = render_header(
            t,
            &cwd_name,
            at_root,
            on_go_up.clone(),
            on_refresh.clone(),
            on_navigate_to.clone(),
        );

        // ── Breadcrumb row ──
        let crumbs = render_breadcrumbs(t, &cwd, on_navigate_to.clone());

        // ── List body ──
        let mut body = div().flex().flex_col().px(SP_2).py(SP_2).gap(SP_0_5);
        if let Some(err) = error {
            body = body.child(
                div()
                    .px(SP_2)
                    .py(SP_2)
                    .flex()
                    .flex_col()
                    .gap(SP_1)
                    .rounded(RADIUS_SM)
                    .bg(t.color.bg_surface)
                    .border_1()
                    .border_color(t.color.border_subtle)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(SP_2)
                            .child(SectionLabel::new(t!("App.FileTree.Errors.cannot_read_directory")))
                            .child(StatusPill::new(t!("App.FileTree.Errors.io_error"), StatusKind::Error)),
                    )
                    .child(text::body(SharedString::from(err)).secondary()),
            );
        } else if entries.is_empty() {
            body = body.child(
                div()
                    .px(SP_2)
                    .py(SP_2)
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child(SharedString::from(
                        t!("App.FileTree.empty_directory").to_string(),
                    )),
            );
        } else {
            let mut visible = 0usize;
            for entry in entries.iter().take(MAX_CHILDREN_PER_DIR) {
                if !filter_lower.is_empty() && !entry.name.to_lowercase().contains(&filter_lower) {
                    continue;
                }
                body = body.child(row(t, entry, on_enter_dir.clone(), on_open_file.clone()));
                visible += 1;
            }
            if visible == 0 {
                body = body.child(
                    div()
                        .px(SP_2)
                        .py(SP_2)
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(SharedString::from(
                            t!("App.Common.no_matches", query = filter).to_string(),
                        )),
                );
            }
            if entries.len() > MAX_CHILDREN_PER_DIR {
                body = body.child(
                    div()
                        .px(SP_2)
                        .py(SP_1)
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(SharedString::from(
                            t!(
                                "App.FileTree.more_entries",
                                count = entries.len() - MAX_CHILDREN_PER_DIR
                            )
                            .to_string(),
                        )),
                );
            }
        }

        div()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .bg(t.color.bg_surface)
                    .border_b_1()
                    .border_color(t.color.border_subtle)
                    .child(header)
                    .child(crumbs),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .child(body),
            )
    }
}

// ─────────────────────────────────────────────────────────
// Header (5 elements: ⤴ + cwd name + ⋯ + 🔄 + breadcrumb on next row)
// ─────────────────────────────────────────────────────────

fn render_header(
    t: &crate::theme::Theme,
    cwd_name: &SharedString,
    at_root: bool,
    on_go_up: GoUpHandler,
    on_refresh: RefreshHandler,
    on_navigate_to: NavigateToHandler,
) -> impl IntoElement {
    div()
        .h(ROW_MD_H)
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .border_b_1()
        .border_color(t.color.border_subtle)
        // 1. ⤴ Up (disabled when already at filesystem root).
        .child(
            IconButton::new("ft-up", IconName::ChevronLeft)
                .size(IconButtonSize::Sm)
                .variant(IconButtonVariant::Filled)
                .disabled(at_root)
                .on_click(move |_, w, app| on_go_up(&(), w, app)),
        )
        // 2. Folder icon + cwd basename — decorative, not a button, so
        //    it stays as a small styled div. (px(18) is an in-view token
        //    for this chip, allowed because the chip is conceptually a
        //    one-off visual atom that doesn't justify its own component.)
        .child(
            div()
                .w(px(18.0))
                .h(px(18.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_SM)
                .bg(t.color.accent_subtle)
                .child(
                    UiIcon::new(IconName::Folder)
                        .size(GLYPH_SM)
                        .text_color(t.color.accent),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child(cwd_name.clone()),
        )
        // 3. ⋯ Quick targets popover (needs Selectable — handwritten).
        .child(quick_menu(t, on_navigate_to))
        // 4. 🔄 Refresh.
        .child(
            IconButton::new("ft-refresh", IconName::Loader)
                .size(IconButtonSize::Sm)
                .variant(IconButtonVariant::Filled)
                .on_click(move |_, w, app| on_refresh(&(), w, app)),
        )
}

fn quick_menu(t: &crate::theme::Theme, on_navigate_to: NavigateToHandler) -> impl IntoElement {
    let trigger_color = t.color.text_secondary;
    let trigger_hover = t.color.bg_hover;
    let trigger_bg = t.color.bg_panel;
    let trigger_border = t.color.border_subtle;
    let menu_colors = t.color;
    Popover::new("ft-quick-menu")
        .anchor(Corner::TopRight)
        .trigger(
            // Selectable trigger element — Popover wraps it with click handling.
            // Using a button-like styled div via gpui_component::button::Button
            // would also work; div + Selectable impl is the lighter path here.
            QuickMenuTrigger {
                color: trigger_color,
                hover: trigger_hover,
                bg: trigger_bg,
                border: trigger_border,
            },
        )
        .content(move |_state, _w, _cx| {
            let nav = on_navigate_to.clone();
            quick_menu_body(menu_colors, nav)
        })
}

/// Internal trigger element for the ⋯ popover. Implementing Selectable is
/// required by [`Popover::trigger`].
#[derive(IntoElement)]
struct QuickMenuTrigger {
    color: gpui::Rgba,
    hover: gpui::Rgba,
    bg: gpui::Rgba,
    border: gpui::Rgba,
}

impl gpui_component::Selectable for QuickMenuTrigger {
    fn selected(self, _selected: bool) -> Self {
        self
    }
    fn is_selected(&self) -> bool {
        false
    }
}

impl RenderOnce for QuickMenuTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let hover = self.hover;
        div()
            .id("ft-quick-trigger")
            .w(BUTTON_SM_H)
            .h(BUTTON_SM_H)
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_SM)
            .bg(self.bg)
            .border_1()
            .border_color(self.border)
            .text_color(self.color)
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .child(
                UiIcon::new(IconName::Ellipsis)
                    .size(GLYPH_SM)
                    .text_color(self.color),
            )
    }
}

fn quick_menu_body(
    colors: crate::theme::ColorSet,
    on_navigate_to: NavigateToHandler,
) -> impl IntoElement {
    let home = user_home_dir();
    let desktop = home.join("Desktop");
    let projects = home.join("Projects");

    let make_item = |id: &'static str,
                     label: SharedString,
                     path: PathBuf,
                     handler: NavigateToHandler|
     -> gpui::AnyElement {
        div()
            .id(id)
            .min_w(px(180.0))
            .px(SP_3)
            .py(SP_1_5)
            .text_size(SIZE_CAPTION)
            .text_color(colors.text_primary)
            .cursor_pointer()
            .hover(move |s| s.bg(colors.bg_hover))
            .on_click(move |_, w, app| handler(&path, w, app))
            .child(label)
            .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .py(SP_1)
        .child(make_item(
            "ft-qm-home",
            t!("App.FileTree.Quick.home").into(),
            home,
            on_navigate_to.clone(),
        ))
        .child(make_item(
            "ft-qm-desktop",
            t!("App.FileTree.Quick.desktop").into(),
            desktop,
            on_navigate_to.clone(),
        ))
        .child(make_item(
            "ft-qm-projects",
            t!("App.FileTree.Quick.projects").into(),
            projects,
            on_navigate_to.clone(),
        ))
        .child(
            div()
                .h(px(1.0))
                .w_full()
                .my(px(2.0))
                .bg(colors.border_subtle),
        )
        .child(
            div()
                .id("ft-qm-choose")
                .min_w(px(180.0))
                .px(SP_3)
                .py(SP_1_5)
                .text_size(SIZE_CAPTION)
                .text_color(colors.text_tertiary)
                .cursor_default()
                .child(SharedString::from(
                    t!("App.FileTree.Quick.choose_folder").to_string(),
                )),
        )
}

fn user_home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE")?;
            let path = std::env::var_os("HOMEPATH")?;
            let mut home = PathBuf::from(drive);
            home.push(path);
            Some(home)
        })
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from(std::path::MAIN_SEPARATOR_STR))
}

// ─────────────────────────────────────────────────────────
// Breadcrumb row
// ─────────────────────────────────────────────────────────

fn render_breadcrumbs(
    t: &crate::theme::Theme,
    cwd: &Path,
    on_navigate_to: NavigateToHandler,
) -> impl IntoElement {
    let segments = path_segments(cwd);
    let total = segments.len();
    let mut row = div()
        .h(ROW_SM_H)
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_0_5)
        .bg(t.color.bg_surface);

    for (idx, (label, path)) in segments.into_iter().enumerate() {
        let is_last = idx == total - 1;
        let id_str: SharedString = format!("ft-crumb-{idx}").into();
        let nav = on_navigate_to.clone();
        let label: SharedString = if label.is_empty() {
            "/".into()
        } else {
            label.into()
        };
        if idx > 0 {
            row = row.child(
                div()
                    .text_size(SIZE_MONO_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child("›"),
            );
        }
        let target = path.clone();
        row = row.child(
            div()
                .id(gpui::ElementId::Name(id_str))
                .px(px(4.0))
                .h(px(18.0))
                .flex()
                .items_center()
                .rounded(px(2.0))
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(if is_last {
                    t.color.text_primary
                } else {
                    t.color.text_tertiary
                })
                .cursor_pointer()
                .hover(|s| s.bg(t.color.bg_hover))
                .on_click(move |_, w, app| nav(&target, w, app))
                .child(label),
        );
    }
    row
}

/// Decompose a path into `(segment_label, accumulated_path)` pairs in
/// display order. The first segment is the root (`/` on Unix → empty
/// label, rendered as "/" by the breadcrumb row).
fn path_segments(p: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut acc = PathBuf::new();
    let mut components = p.components().peekable();

    while let Some(comp) = components.next() {
        match comp {
            std::path::Component::RootDir => {
                acc.push("/");
                out.push((String::new(), acc.clone()));
            }
            std::path::Component::Normal(s) => {
                acc.push(s);
                out.push((s.to_string_lossy().to_string(), acc.clone()));
            }
            std::path::Component::CurDir | std::path::Component::ParentDir => {}
            std::path::Component::Prefix(prefix) => {
                let label = prefix.as_os_str().to_string_lossy().to_string();
                if matches!(components.peek(), Some(std::path::Component::RootDir)) {
                    let rooted = PathBuf::from(format!(
                        "{}{}",
                        prefix.as_os_str().to_string_lossy(),
                        std::path::MAIN_SEPARATOR
                    ));
                    acc = rooted.clone();
                    out.push((label, rooted));
                    components.next();
                } else {
                    acc.push(prefix.as_os_str());
                    out.push((label, acc.clone()));
                }
            }
        }
    }
    if out.is_empty() {
        out.push((p.display().to_string(), p.to_path_buf()));
    }
    out
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::path::{Path, PathBuf};

    #[cfg(windows)]
    use super::path_segments;

    #[cfg(windows)]
    #[test]
    fn keeps_windows_drive_as_single_root_segment() {
        let segments = path_segments(Path::new(r"E:\workspace-freq\Pier-X"));
        let labels: Vec<_> = segments.iter().map(|(label, _)| label.clone()).collect();

        assert_eq!(labels, vec!["E:", "workspace-freq", "Pier-X"]);
        assert_eq!(segments[0].1, PathBuf::from(r"E:\"));
    }
}

// ─────────────────────────────────────────────────────────
// Single row
// ─────────────────────────────────────────────────────────

fn row(
    t: &crate::theme::Theme,
    entry: &FsEntry,
    on_enter_dir: EnterDirHandler,
    on_open_file: OpenFileHandler,
) -> impl IntoElement {
    let id_str: SharedString = format!("ft-row-{}", entry.path.display()).into();
    let label: SharedString = entry.name.clone().into();
    let glyph = if entry.is_dir {
        IconName::Folder
    } else {
        IconName::File
    };
    let path = entry.path.clone();
    let is_dir = entry.is_dir;

    div()
        .id(gpui::ElementId::Name(id_str))
        .h(ROW_SM_H)
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .rounded(RADIUS_SM)
        .text_size(SIZE_CAPTION)
        .text_color(if entry.is_dir {
            t.color.text_primary
        } else {
            t.color.text_secondary
        })
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(move |_, w, app| {
            if is_dir {
                on_enter_dir(&path, w, app);
            } else {
                on_open_file(&path, w, app);
            }
        })
        .child(
            div()
                .w(ICON_SM)
                .h(ICON_SM)
                .flex()
                .items_center()
                .justify_center()
                .text_color(if entry.is_dir {
                    t.color.accent
                } else {
                    t.color.text_tertiary
                })
                .child(
                    UiIcon::new(glyph)
                        .size(GLYPH_SM)
                        .text_color(if entry.is_dir {
                            t.color.accent
                        } else {
                            t.color.text_tertiary
                        }),
                ),
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
// Filesystem listing (called from LeftPanelView, NEVER from render)
// ─────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct FsEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// Read a single directory level from disk. Called from
/// LeftPanelView::enter_dir / cd_up / refresh handlers — never from
/// inside a `Render::render` body. See CLAUDE.md Rule 6.
pub fn list_dir(root: &Path) -> std::io::Result<Vec<FsEntry>> {
    let mut entries: Vec<FsEntry> = std::fs::read_dir(root)?
        .filter_map(|res| res.ok())
        .filter(|entry| {
            // Hide dotfiles by default — matches Pier's LocalFileView.
            entry
                .file_name()
                .to_str()
                .map(|s| !s.starts_with('.'))
                .unwrap_or(true)
        })
        .map(|entry| {
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            FsEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
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
    Ok(entries)
}
