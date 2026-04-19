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

use gpui::{div, prelude::*, px, App, Corner, IntoElement, Pixels, SharedString, Window};
use gpui_component::{popover::Popover, scroll::ScrollableElement, Icon as UiIcon, IconName};
use rust_i18n::t;

use crate::components::{
    text, IconButton, IconButtonSize, IconButtonVariant, SectionLabel, StatusKind, StatusPill,
};
use crate::data::{
    file_icon, format_date, format_file_size, format_permissions, format_windows_attrs,
    FileIconTone,
};
use crate::theme::{
    heights::{BUTTON_SM_H, GLYPH_SM, ICON_SM, ROW_MD_H, ROW_SM_H},
    radius::{RADIUS_MD, RADIUS_SM},
    shadow,
    spacing::{SP_0_5, SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_CAPTION, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};

/// Children past this many entries are truncated. Defensive for
/// pathological dirs (e.g. `/usr/bin` with thousands of entries) so the
/// element tree stays bounded.
const MAX_CHILDREN_PER_DIR: usize = 1000;

/// Fixed widths for the right-aligned metadata columns in a file row.
/// Each column is only shown when the panel is wider than the threshold
/// below — keeping the name column readable down to 180 px.
const SIZE_COLUMN_W: Pixels = px(56.0);
/// Width of the mtime column. Sized for the full `YYYY-MM-DD` label
/// (10 mono chars ≈ 76 px) plus a couple of pixels of right margin.
/// We render the full date instead of a relative label so the column
/// never visually jitters day-over-day and the "which file is newer"
/// question is always answerable at a glance.
const MTIME_COLUMN_W: Pixels = px(78.0);
/// POSIX permissions are 10 mono chars (`drwxr-xr-x`) and Windows uses
/// a 4-char mask (`drw-`). We size the column to the wider (POSIX)
/// since that's what macOS / Linux users see.
const PERMS_COLUMN_W: Pixels = px(76.0);

/// Show the size column when the panel has at least this much space.
/// Below this we only render [icon] [name].
const SIZE_COLUMN_MIN_W: Pixels = px(220.0);
/// Reveal the mtime column once the panel is wide enough to host the
/// full `YYYY-MM-DD` label without starving the name column.
const MTIME_COLUMN_MIN_W: Pixels = px(300.0);
/// The permissions column is the last to appear; on Windows the mask
/// is narrower but we gate on the POSIX width so macOS renders the
/// full `drwxr-xr-x` without truncation.
const PERMS_COLUMN_MIN_W: Pixels = px(380.0);

pub type EnterDirHandler = Rc<dyn Fn(&PathBuf, &mut Window, &mut App) + 'static>;
pub type OpenFileHandler = Rc<dyn Fn(&PathBuf, &mut Window, &mut App) + 'static>;
/// `cx.listener` returns a closure with a `&E` first argument, so we pass
/// `&()` for buttons that don't carry payload.
pub type GoUpHandler = Rc<dyn Fn(&(), &mut Window, &mut App) + 'static>;
pub type RefreshHandler = Rc<dyn Fn(&(), &mut Window, &mut App) + 'static>;
pub type NavigateToHandler = Rc<dyn Fn(&PathBuf, &mut Window, &mut App) + 'static>;
pub type ChooseFolderHandler = Rc<dyn Fn(&(), &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct FileTree {
    cwd: PathBuf,
    entries: Vec<FsEntry>,
    /// `Some(err)` when listing the cwd itself failed (perm denied, etc.)
    error: Option<String>,
    /// Case-insensitive substring filter on entry names. Empty = show all.
    filter: String,
    /// Current width of the left panel. Drives which metadata columns
    /// are visible (size / mtime / permissions).
    content_width: Pixels,
    on_enter_dir: EnterDirHandler,
    on_open_file: OpenFileHandler,
    on_go_up: GoUpHandler,
    on_refresh: RefreshHandler,
    on_navigate_to: NavigateToHandler,
    on_choose_folder: ChooseFolderHandler,
}

impl FileTree {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cwd: PathBuf,
        entries: Vec<FsEntry>,
        error: Option<String>,
        filter: String,
        content_width: Pixels,
        on_enter_dir: EnterDirHandler,
        on_open_file: OpenFileHandler,
        on_go_up: GoUpHandler,
        on_refresh: RefreshHandler,
        on_navigate_to: NavigateToHandler,
        on_choose_folder: ChooseFolderHandler,
    ) -> Self {
        Self {
            cwd,
            entries,
            error,
            filter,
            content_width,
            on_enter_dir,
            on_open_file,
            on_go_up,
            on_refresh,
            on_navigate_to,
            on_choose_folder,
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
            content_width,
            on_enter_dir,
            on_open_file,
            on_go_up,
            on_refresh,
            on_navigate_to,
            on_choose_folder,
        } = self;
        let show_size = content_width >= SIZE_COLUMN_MIN_W;
        let show_mtime = content_width >= MTIME_COLUMN_MIN_W;
        let show_perms = content_width >= PERMS_COLUMN_MIN_W;
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
            on_choose_folder.clone(),
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
                            .child(SectionLabel::new(t!(
                                "App.FileTree.Errors.cannot_read_directory"
                            )))
                            .child(StatusPill::new(
                                t!("App.FileTree.Errors.io_error"),
                                StatusKind::Error,
                            )),
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
                body = body.child(row(
                    t,
                    entry,
                    show_size,
                    show_mtime,
                    show_perms,
                    on_enter_dir.clone(),
                    on_open_file.clone(),
                ));
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
    on_choose_folder: ChooseFolderHandler,
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
                .truncate()
                .child(cwd_name.clone()),
        )
        // 3. ⋯ Quick targets popover (needs Selectable — handwritten).
        .child(quick_menu(t, on_navigate_to, on_choose_folder))
        // 4. 🔄 Refresh.
        .child(
            IconButton::new("ft-refresh", IconName::RefreshCw)
                .size(IconButtonSize::Sm)
                .variant(IconButtonVariant::Filled)
                .on_click(move |_, w, app| on_refresh(&(), w, app)),
        )
}

fn quick_menu(
    t: &crate::theme::Theme,
    on_navigate_to: NavigateToHandler,
    on_choose_folder: ChooseFolderHandler,
) -> impl IntoElement {
    let t = t.clone();
    // `.appearance(false)` turns OFF the Popover's default surface
    // (white rounded box + p_3 padding). We draw our own container
    // inside `quick_menu_body` with Pier-style bg_elevated + border +
    // popover shadow so the menu has a single crisp frame instead of
    // the doubled-box look the default appearance gives us.
    Popover::new("ft-quick-menu")
        .anchor(Corner::TopRight)
        .appearance(false)
        .trigger(QuickMenuTrigger {
            bg_idle: t.color.bg_surface,
            bg_hover: t.color.bg_hover,
            fg: t.color.text_primary,
        })
        .content(move |_state, _w, _cx| {
            let nav = on_navigate_to.clone();
            let choose = on_choose_folder.clone();
            quick_menu_body(&t, nav, choose)
        })
}

/// Internal trigger element for the ⋯ popover. Implementing Selectable is
/// required by [`Popover::trigger`]. Visual parity with the adjacent
/// `IconButton::new(_, Ellipsis).variant(Filled).size(Sm)` — bg_surface
/// fill, no border, ICON_SM glyph, 22px square — so the three header
/// icons (← / ⋯ / 🔄) line up pixel-for-pixel.
#[derive(IntoElement)]
struct QuickMenuTrigger {
    bg_idle: gpui::Rgba,
    bg_hover: gpui::Rgba,
    fg: gpui::Rgba,
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
        let hover = self.bg_hover;
        div()
            .id("ft-quick-trigger")
            .w(BUTTON_SM_H)
            .h(BUTTON_SM_H)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(RADIUS_SM)
            .bg(self.bg_idle)
            .text_color(self.fg)
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .child(
                UiIcon::new(IconName::Ellipsis)
                    .size(ICON_SM)
                    .text_color(self.fg),
            )
    }
}

/// One entry in the ⋯ quick-targets popover.
struct QuickMenuEntry {
    /// Element id — must be unique across the menu.
    id: &'static str,
    /// Localized label.
    label: SharedString,
    /// Icon shown to the left of the label. Folder-glyph variants
    /// (Folder / Home / Download / FileText) cover all the standard
    /// directory-role cues we need.
    icon: IconName,
    /// Destination path. Absent only for the "Choose Folder…" row,
    /// which triggers a native picker instead of jumping directly.
    target: Option<PathBuf>,
}

fn quick_menu_body(
    t: &crate::theme::Theme,
    on_navigate_to: NavigateToHandler,
    on_choose_folder: ChooseFolderHandler,
) -> impl IntoElement {
    let entries = quick_menu_entries();
    let colors = t.color;
    let mono = t.font_mono.clone();

    let render_item = |entry: QuickMenuEntry,
                       on_navigate: NavigateToHandler,
                       on_choose: ChooseFolderHandler|
     -> gpui::AnyElement {
        let label_color = colors.text_primary;
        let icon_color = colors.text_tertiary;
        let hover_bg = colors.bg_hover;
        let hint_color = colors.text_tertiary;
        let hint: Option<SharedString> = entry
            .target
            .as_ref()
            .and_then(|p| p.to_str())
            .map(|s| {
                // Replace the user's home prefix with `~` so the hint
                // stays short and doesn't reveal the user account name.
                let home = user_home_dir();
                let home_str = home.to_string_lossy().to_string();
                if let Some(rest) = s.strip_prefix(&home_str) {
                    if rest.is_empty() {
                        "~".to_string()
                    } else {
                        format!("~{}", rest)
                    }
                } else {
                    s.to_string()
                }
            })
            .map(SharedString::from);

        div()
            .id(entry.id)
            .h(ROW_MD_H)
            .min_w(px(240.0))
            .px(SP_3)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .text_size(SIZE_CAPTION)
            .text_color(label_color)
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .on_click({
                let target = entry.target.clone();
                move |_, w, app| match &target {
                    Some(path) => on_navigate(path, w, app),
                    None => on_choose(&(), w, app),
                }
            })
            .child(
                div()
                    .w(ICON_SM)
                    .h(ICON_SM)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        UiIcon::new(entry.icon)
                            .size(GLYPH_SM)
                            .text_color(icon_color),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .font_weight(WEIGHT_MEDIUM)
                    .child(entry.label),
            )
            .when_some(hint, |row, hint_text| {
                row.child(
                    div()
                        .flex_none()
                        .max_w(px(180.0))
                        .truncate()
                        .text_size(SIZE_MONO_SMALL)
                        .font_family(mono.clone())
                        .text_color(hint_color)
                        .child(hint_text),
                )
            })
            .into_any_element()
    };

    let mut container = div()
        .flex()
        .flex_col()
        .py(SP_1)
        .bg(colors.bg_elevated)
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(colors.border_subtle)
        .shadow(shadow::popover());

    for entry in entries {
        let is_divider = entry.id == "__divider__";
        if is_divider {
            container = container.child(
                div()
                    .h(px(1.0))
                    .w_full()
                    .my(SP_1)
                    .bg(colors.border_subtle),
            );
        } else {
            container =
                container.child(render_item(entry, on_navigate_to.clone(), on_choose_folder.clone()));
        }
    }

    container
}

/// Build the OS-appropriate quick-targets list. Order mirrors the
/// sidebar ordering macOS Finder / Windows Explorer use so the list
/// feels native.
fn quick_menu_entries() -> Vec<QuickMenuEntry> {
    let home = user_home_dir();
    let mut out: Vec<QuickMenuEntry> = Vec::new();

    // Home / Desktop / Documents / Downloads — present on every
    // supported OS (Windows + macOS + Linux). We list the folders
    // under $HOME rather than OS-specific special folders so the
    // entries are predictable cross-platform.
    out.push(QuickMenuEntry {
        id: "ft-qm-home",
        label: t!("App.FileTree.Quick.home").into(),
        // Phosphor `user-circle-fill` = SF `person.circle.fill` — Pier
        // uses the same glyph for "the current user" chips.
        icon: IconName::CircleUserFill,
        target: Some(home.clone()),
    });
    out.push(QuickMenuEntry {
        id: "ft-qm-desktop",
        label: t!("App.FileTree.Quick.desktop").into(),
        icon: IconName::Folder,
        target: Some(home.join("Desktop")),
    });
    out.push(QuickMenuEntry {
        id: "ft-qm-documents",
        label: t!("App.FileTree.Quick.documents").into(),
        icon: IconName::FileText,
        target: Some(home.join("Documents")),
    });
    out.push(QuickMenuEntry {
        id: "ft-qm-downloads",
        label: t!("App.FileTree.Quick.downloads").into(),
        // Stand-in for a download-tray glyph (our icon set lacks one);
        // `ArrowDown` reads as "downloads" when paired with the label.
        icon: IconName::ArrowDown,
        target: Some(home.join("Downloads")),
    });
    out.push(QuickMenuEntry {
        id: "ft-qm-projects",
        label: t!("App.FileTree.Quick.projects").into(),
        icon: IconName::Folder,
        target: Some(home.join("Projects")),
    });

    // OS-specific tail: macOS → /Applications; Linux → /; Windows →
    // one entry per mounted drive. Everything else falls back to the
    // filesystem root.
    #[cfg(target_os = "macos")]
    {
        out.push(QuickMenuEntry {
            id: "ft-qm-applications",
            label: t!("App.FileTree.Quick.applications").into(),
            icon: IconName::Folder,
            target: Some(PathBuf::from("/Applications")),
        });
    }
    #[cfg(target_os = "windows")]
    {
        for drive in windows_drive_roots() {
            let letter = drive
                .to_string_lossy()
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_default();
            // Static &str ids would require a hand-rolled cache; we
            // clone into `Box::leak` *once* per drive since the list
            // is ≤ 26 entries and lives for the session.
            let id: &'static str = Box::leak(format!("ft-qm-drive-{letter}").into_boxed_str());
            out.push(QuickMenuEntry {
                id,
                label: t!("App.FileTree.Quick.drive", letter = letter).into(),
                icon: IconName::Folder,
                target: Some(drive),
            });
        }
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        out.push(QuickMenuEntry {
            id: "ft-qm-root",
            label: t!("App.FileTree.Quick.root").into(),
            icon: IconName::Folder,
            target: Some(PathBuf::from("/")),
        });
    }

    // Divider + Choose Folder…
    out.push(QuickMenuEntry {
        id: "__divider__",
        label: SharedString::default(),
        icon: IconName::Folder,
        target: None,
    });
    out.push(QuickMenuEntry {
        id: "ft-qm-choose",
        label: t!("App.FileTree.Quick.choose_folder").into(),
        icon: IconName::FolderOpen,
        target: None,
    });

    out
}

/// Enumerate mounted drive roots on Windows by probing `A:\..Z:\`.
/// Cheap (26 stat calls) and avoids pulling the `windows-sys` crate
/// just for `GetLogicalDrives`.
#[cfg(target_os = "windows")]
fn windows_drive_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for letter in b'A'..=b'Z' {
        let path = PathBuf::from(format!("{}:\\", letter as char));
        if path.exists() {
            out.push(path);
        }
    }
    out
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
        .min_w(px(0.0))
        .overflow_hidden()
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
                .when(is_last, |this| this.flex_1().min_w(px(0.0)))
                .when(!is_last, |this| this.flex_none())
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
                .child(
                    div()
                        .when(is_last, |this| this.min_w(px(0.0)).truncate())
                        .child(label),
                ),
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

/// Resolve a `FileIconTone` bucket to a concrete theme color. Kept at
/// the view level so the tone enum stays UI-framework-agnostic (lives
/// in `crate::data`) and only the renderer knows about `Theme`.
fn tone_color(t: &crate::theme::Theme, tone: FileIconTone) -> gpui::Rgba {
    match tone {
        // Directories keep the accent blue — the primary "this is a
        // navigable target" cue in the panel.
        FileIconTone::Directory => t.color.accent,
        // Docs / markdown lean on status_info (blue) for readability.
        FileIconTone::Docs => t.color.status_info,
        // Shell scripts feel like a "run this" surface — success green.
        FileIconTone::Shell => t.color.status_success,
        // Configs → warning amber (matches the "you are editing infra"
        // convention most IDEs use for YAML / env files).
        FileIconTone::Config => t.color.status_warning,
        // Web files also lean warning-ish but distinct from config —
        // reuse accent to avoid introducing a new hue.
        FileIconTone::Web => t.color.accent,
        // Code / images / media / archives all share the accent since
        // the enum is already communicated by the glyph shape; giving
        // each its own color would turn the panel into a parade.
        FileIconTone::Code
        | FileIconTone::Image
        | FileIconTone::Media
        | FileIconTone::Archive => t.color.text_secondary,
        // Default / unknown — tertiary so the row's name does the work.
        FileIconTone::Neutral => t.color.text_tertiary,
    }
}

fn row(
    t: &crate::theme::Theme,
    entry: &FsEntry,
    show_size: bool,
    show_mtime: bool,
    show_perms: bool,
    on_enter_dir: EnterDirHandler,
    on_open_file: OpenFileHandler,
) -> impl IntoElement {
    let id_str: SharedString = format!("ft-row-{}", entry.path.display()).into();
    let label: SharedString = entry.name.clone().into();
    let (glyph, tone) = file_icon(&entry.name, entry.is_dir);
    let glyph_color = tone_color(t, tone);
    let path = entry.path.clone();
    let is_dir = entry.is_dir;

    let size_label: SharedString = if entry.is_dir {
        "—".into()
    } else {
        format_file_size(entry.size).into()
    };
    let mtime_label: SharedString = entry
        .modified_secs
        .map(format_date)
        .unwrap_or_else(|| "—".to_string())
        .into();
    // Prefer the POSIX mode when the filesystem supplied one; fall
    // back to the Windows attribute mask otherwise. A remote mount
    // (e.g. NFS on macOS) may expose neither, in which case we render
    // an em-dash so the column still aligns.
    let perms_label: SharedString = if let Some(mode) = entry.unix_mode {
        format_permissions(mode, entry.is_dir, entry.is_link).into()
    } else if let Some(attrs) = entry.windows_attrs {
        format_windows_attrs(attrs, entry.is_dir, entry.is_link).into()
    } else {
        "—".into()
    };

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
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .text_color(glyph_color)
                .child(UiIcon::new(glyph).size(GLYPH_SM).text_color(glyph_color)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .child(label),
        )
        .when(show_size, |row| {
            row.child(
                div()
                    .flex_none()
                    .w(SIZE_COLUMN_W)
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .text_right()
                    .child(size_label),
            )
        })
        .when(show_mtime, |row| {
            row.child(
                div()
                    .flex_none()
                    .w(MTIME_COLUMN_W)
                    .text_size(SIZE_MONO_SMALL)
                    .font_family(t.font_mono.clone())
                    .text_color(t.color.text_tertiary)
                    .text_right()
                    .child(mtime_label),
            )
        })
        .when(show_perms, |row| {
            row.child(
                div()
                    .flex_none()
                    .w(PERMS_COLUMN_W)
                    .text_size(SIZE_MONO_SMALL)
                    .font_family(t.font_mono.clone())
                    .text_color(t.color.text_tertiary)
                    .text_right()
                    .child(perms_label),
            )
        })
}

// ─────────────────────────────────────────────────────────
// Filesystem listing (called from LeftPanelView, NEVER from render)
// ─────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct FsEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
    /// File byte count. `0` for directories (we render `—`).
    pub size: u64,
    /// Last-modified time in Unix seconds. `None` when the OS didn't
    /// expose it (e.g. FAT filesystems, remote mounts without mtime).
    pub modified_secs: Option<u64>,
    /// POSIX permission bits (low 9 bits used). Always populated on
    /// Unix; on Windows this stays `None` — Windows reports attributes
    /// via [`Self::windows_attrs`] instead.
    pub unix_mode: Option<u32>,
    /// Windows file attributes (`FILE_ATTRIBUTE_*` mask). Always
    /// populated on Windows so the permissions column can render
    /// `RO`/`H` hints; `None` on Unix.
    pub windows_attrs: Option<u32>,
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
            let file_type = entry.file_type().ok();
            let is_dir = file_type.as_ref().map(|t| t.is_dir()).unwrap_or(false);
            let is_link = file_type.as_ref().map(|t| t.is_symlink()).unwrap_or(false);
            // One `metadata()` call per row — the cost is paid here in
            // the LeftPanelView::enter_dir handler, never during render,
            // so paint latency stays flat. `read_dir` already walks the
            // directory entry by entry; `metadata()` just stats it.
            let metadata = entry.metadata().ok();
            let size = if is_dir {
                0
            } else {
                metadata.as_ref().map(|m| m.len()).unwrap_or(0)
            };
            let modified_secs = metadata.as_ref().and_then(|m| {
                m.modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
            });
            #[cfg(unix)]
            let unix_mode = {
                use std::os::unix::fs::PermissionsExt;
                metadata.as_ref().map(|m| m.permissions().mode())
            };
            #[cfg(not(unix))]
            let unix_mode = None::<u32>;
            #[cfg(windows)]
            let windows_attrs = {
                use std::os::windows::fs::MetadataExt;
                metadata.as_ref().map(|m| m.file_attributes())
            };
            #[cfg(not(windows))]
            let windows_attrs = None::<u32>;
            FsEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path(),
                is_dir,
                is_link,
                size,
                modified_secs,
                unix_mode,
                windows_attrs,
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
