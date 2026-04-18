use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use gpui::{div, prelude::*, px, AnyElement, App, IntoElement, SharedString, WeakEntity, Window};
use rust_i18n::t;

use crate::{
    app::PierApp,
    components::{text, Button, Card, SectionLabel, StatusKind, StatusPill},
    theme::{
        radius::RADIUS_SM,
        spacing::{SP_1, SP_2, SP_3, SP_4},
        theme,
    },
};

const MAX_DIRECTORY_ENTRIES: usize = 48;
const COMPACT_PREVIEW_LINES: usize = 14;
const EXPANDED_PREVIEW_LINES: usize = 80;
const COMPACT_PREVIEW_BYTES: usize = 16 * 1024;
const EXPANDED_PREVIEW_BYTES: usize = 128 * 1024;
const MAX_PREVIEW_LINE_CHARS: usize = 160;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PathPreviewMode {
    Compact,
    Expanded,
}

impl PathPreviewMode {
    pub fn label(self) -> String {
        match self {
            Self::Compact => t!("App.PathInspector.Preview.compact").to_string(),
            Self::Expanded => t!("App.PathInspector.Preview.expanded").to_string(),
        }
    }

    const fn max_lines(self) -> usize {
        match self {
            Self::Compact => COMPACT_PREVIEW_LINES,
            Self::Expanded => EXPANDED_PREVIEW_LINES,
        }
    }

    const fn max_bytes(self) -> usize {
        match self {
            Self::Compact => COMPACT_PREVIEW_BYTES,
            Self::Expanded => EXPANDED_PREVIEW_BYTES,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PathKind {
    Waiting,
    Directory,
    File,
    Unavailable,
}

#[derive(Clone)]
pub struct PathInspectorEntry {
    pub label: SharedString,
    pub detail_label: SharedString,
    pub kind_label: SharedString,
    pub status_kind: StatusKind,
    pub target: SharedString,
}

#[derive(Clone)]
pub struct PathInspectorSnapshot {
    kind: PathKind,
    inspect_target: Option<SharedString>,
    parent_target: Option<SharedString>,
    pub requested_target: SharedString,
    pub resolved_path: SharedString,
    pub status_label: SharedString,
    pub status_kind: StatusKind,
    pub kind_label: SharedString,
    pub parent_label: SharedString,
    pub size_label: SharedString,
    pub detail_label: SharedString,
    pub preview_title: SharedString,
    pub preview_mode: PathPreviewMode,
    pub preview_toggle_available: bool,
    pub preview_meta: Vec<SharedString>,
    pub preview_lines: Vec<SharedString>,
    pub directory_entries: Vec<PathInspectorEntry>,
}

impl PathInspectorSnapshot {
    pub fn inspect(target: &str) -> Self {
        Self::inspect_with_mode(target, PathPreviewMode::Compact)
    }

    pub fn inspect_with_mode(target: &str, preview_mode: PathPreviewMode) -> Self {
        let requested_target = target.trim();
        if requested_target.is_empty() {
            return Self::empty();
        }

        let requested_path = PathBuf::from(requested_target);
        let resolved_path =
            fs::canonicalize(&requested_path).unwrap_or_else(|_| requested_path.clone());
        let resolved_label: SharedString = resolved_path.to_string_lossy().into_owned().into();
        let parent_target = resolved_path
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned().into());
        let parent_label = parent_target
            .clone()
            .unwrap_or_else(|| SharedString::from(t!("App.PathInspector.no_parent").to_string()));

        match fs::metadata(&resolved_path) {
            Ok(metadata) if metadata.is_dir() => {
                let preview = build_directory_preview(&resolved_path);
                Self {
                    kind: PathKind::Directory,
                    inspect_target: Some(resolved_label.clone()),
                    parent_target,
                    requested_target: requested_target.to_string().into(),
                    resolved_path: resolved_label,
                    status_label: t!("App.PathInspector.status_local_path").into(),
                    status_kind: StatusKind::Success,
                    kind_label: t!("App.PathInspector.kind_directory").into(),
                    parent_label,
                    size_label: "—".into(),
                    detail_label: preview.detail_label.into(),
                    preview_title: t!("App.PathInspector.entries_title").into(),
                    preview_mode,
                    preview_toggle_available: false,
                    preview_meta: preview.meta,
                    preview_lines: preview.lines,
                    directory_entries: preview.entries,
                }
            }
            Ok(metadata) => {
                let preview = build_file_preview(&resolved_path, metadata.len(), preview_mode);
                Self {
                    kind: PathKind::File,
                    inspect_target: Some(resolved_label.clone()),
                    parent_target,
                    requested_target: requested_target.to_string().into(),
                    resolved_path: resolved_label,
                    status_label: t!("App.PathInspector.status_local_path").into(),
                    status_kind: StatusKind::Success,
                    kind_label: t!("App.PathInspector.kind_file").into(),
                    parent_label,
                    size_label: format_bytes(metadata.len()).into(),
                    detail_label: preview.detail_label.into(),
                    preview_title: format!("Preview ({})", preview_mode.label()).into(),
                    preview_mode,
                    preview_toggle_available: preview.preview_toggle_available,
                    preview_meta: preview.meta,
                    preview_lines: preview.lines,
                    directory_entries: Vec::new(),
                }
            }
            Err(err) => Self {
                kind: PathKind::Unavailable,
                inspect_target: Some(requested_target.to_string().into()),
                parent_target,
                requested_target: requested_target.to_string().into(),
                resolved_path: resolved_label,
                status_label: t!("App.PathInspector.status_missing").into(),
                status_kind: StatusKind::Warning,
                kind_label: t!("App.PathInspector.kind_unavailable").into(),
                parent_label,
                size_label: "—".into(),
                detail_label: t!("App.PathInspector.metadata_error", error = err.to_string()).into(),
                preview_title: t!("App.Common.preview").into(),
                preview_mode,
                preview_toggle_available: false,
                preview_meta: vec![t!("App.PathInspector.inspect_parent_hint").into()],
                preview_lines: vec![t!("App.PathInspector.path_missing_body").into()],
                directory_entries: Vec::new(),
            },
        }
    }

    pub fn empty() -> Self {
        Self {
            kind: PathKind::Waiting,
            inspect_target: None,
            parent_target: None,
            requested_target: t!("App.PathInspector.no_local_target").into(),
            resolved_path: t!("App.PathInspector.empty_resolved_path").into(),
            status_label: t!("App.Common.Status.idle").into(),
            status_kind: StatusKind::Info,
            kind_label: t!("App.PathInspector.kind_waiting").into(),
            parent_label: "—".into(),
            size_label: "—".into(),
            detail_label: t!("App.PathInspector.empty_detail").into(),
            preview_title: t!("App.Common.preview").into(),
            preview_mode: PathPreviewMode::Compact,
            preview_toggle_available: false,
            preview_meta: vec![t!("App.PathInspector.empty_meta").into()],
            preview_lines: vec![t!("App.PathInspector.empty_body").into()],
            directory_entries: Vec::new(),
        }
    }

    pub fn inspect_target_string(&self) -> Option<String> {
        self.inspect_target
            .as_ref()
            .map(|target| target.as_ref().to_string())
    }

    pub fn parent_target_string(&self) -> Option<String> {
        self.parent_target
            .as_ref()
            .map(|target| target.as_ref().to_string())
    }

    pub fn is_directory(&self) -> bool {
        matches!(self.kind, PathKind::Directory)
    }

    pub fn is_file(&self) -> bool {
        matches!(self.kind, PathKind::File)
    }
}

#[derive(IntoElement)]
pub struct PathInspectorView {
    snapshot: PathInspectorSnapshot,
    app: WeakEntity<PierApp>,
}

impl PathInspectorView {
    pub fn new(snapshot: Option<PathInspectorSnapshot>, app: WeakEntity<PierApp>) -> Self {
        Self {
            snapshot: snapshot.unwrap_or_else(PathInspectorSnapshot::empty),
            app,
        }
    }
}

impl RenderOnce for PathInspectorView {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx).clone();
        let snapshot = self.snapshot;
        let app = self.app;

        let mut metadata = Card::new()
            .child(SectionLabel::new(t!("App.Common.metadata")))
            .child(text::body(snapshot.kind_label.clone()))
            .child(text::body(snapshot.detail_label.clone()).secondary())
            .child(
                text::mono(t!(
                    "App.PathInspector.parent_label",
                    parent = snapshot.parent_label.as_ref()
                ))
                .secondary(),
            )
            .child(
                text::mono(t!("App.PathInspector.size_label", size = snapshot.size_label.as_ref()))
                    .secondary(),
            );

        let actions = inspector_action_elements(&snapshot, &app);
        if !actions.is_empty() {
            metadata = metadata.child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(SP_2)
                    .pt(SP_2)
                    .children(actions),
            );
        }

        let mut preview = Card::new().child(SectionLabel::new(snapshot.preview_title.clone()));
        if !snapshot.preview_meta.is_empty() {
            preview = preview.child(
                div().flex().flex_col().gap(SP_1).children(
                    snapshot
                        .preview_meta
                        .iter()
                        .cloned()
                        .map(|line| text::caption(line).secondary().into_any_element()),
                ),
            );
        }

        if snapshot.is_directory() {
            if snapshot.directory_entries.is_empty() {
                preview = preview.child(text::body(t!("App.PathInspector.directory_empty")).secondary());
            } else {
                preview = preview.child(div().flex().flex_col().gap(SP_2).pt(SP_2).children(
                    directory_entry_elements(&snapshot.directory_entries, &app, &t),
                ));
            }
        } else {
            preview = preview.child(div().flex().flex_col().gap(SP_1).pt(SP_2).children(
                snapshot.preview_lines.iter().cloned().map(|line| {
                    div()
                        .text_color(t.color.text_secondary)
                        .font_family(t.font_mono.clone())
                        .child(line)
                        .into_any_element()
                }),
            ));
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(SP_4)
            .p(SP_4)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(SP_2)
                    .child(text::h2(t!("App.PathInspector.title")))
                    .child(StatusPill::new(
                        snapshot.status_label.clone(),
                        snapshot.status_kind,
                    )),
            )
            .child(
                Card::new()
                    .child(SectionLabel::new(t!("App.Common.target")))
                    .child(text::mono(snapshot.requested_target.clone()))
                    .child(text::body(snapshot.resolved_path.clone()).secondary()),
            )
            .child(metadata)
            .child(preview)
    }
}

struct DirectoryPreview {
    detail_label: String,
    meta: Vec<SharedString>,
    lines: Vec<SharedString>,
    entries: Vec<PathInspectorEntry>,
}

fn build_directory_preview(path: &Path) -> DirectoryPreview {
    let Ok(entries) = fs::read_dir(path) else {
        return DirectoryPreview {
            detail_label: t!("App.PathInspector.directory_preview_unavailable").to_string(),
            meta: vec![t!("App.PathInspector.failed_to_enumerate").into()],
            lines: vec![t!("App.PathInspector.directory_preview_unavailable").into()],
            entries: Vec::new(),
        };
    };

    let mut rows = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let metadata = entry.metadata().ok();
            let is_dir = metadata.as_ref().is_some_and(|meta| meta.is_dir());
            let name = entry.file_name().to_string_lossy().into_owned();
            let detail = metadata
                .map(|meta| {
                    if meta.is_dir() {
                        t!("App.PathInspector.kind_directory").to_string()
                    } else {
                        format_bytes(meta.len())
                    }
                })
                .unwrap_or_else(|| t!("App.PathInspector.metadata_unavailable").to_string());
            PathInspectorEntry {
                label: format!("{}{}", name, if is_dir { "/" } else { "" }).into(),
                detail_label: detail.into(),
                kind_label: if is_dir {
                    t!("App.PathInspector.kind_dir_short").into()
                } else {
                    t!("App.PathInspector.kind_file_short").into()
                },
                status_kind: if is_dir {
                    StatusKind::Success
                } else {
                    StatusKind::Info
                },
                target: entry.path().to_string_lossy().into_owned().into(),
            }
        })
        .collect::<Vec<_>>();

    rows.sort_by(|left, right| {
        let left_is_dir = left.status_kind == StatusKind::Success;
        let right_is_dir = right.status_kind == StatusKind::Success;
        right_is_dir.cmp(&left_is_dir).then_with(|| {
            left.label
                .as_ref()
                .to_ascii_lowercase()
                .cmp(&right.label.as_ref().to_ascii_lowercase())
        })
    });

    let total = rows.len();
    let visible = rows
        .into_iter()
        .take(MAX_DIRECTORY_ENTRIES)
        .collect::<Vec<_>>();
    let mut meta = Vec::new();
    if total > MAX_DIRECTORY_ENTRIES {
        meta.push(
            t!(
                "App.PathInspector.showing_entries",
                shown = visible.len(),
                total = total
            )
            .into(),
        );
    } else {
        meta.push(t!("App.PathInspector.entries_count", count = total).into());
    }

    DirectoryPreview {
        detail_label: if total > MAX_DIRECTORY_ENTRIES {
            t!(
                "App.PathInspector.directory_detail_truncated",
                shown = visible.len(),
                total = total
            )
            .to_string()
        } else {
            t!("App.PathInspector.directory_detail", count = total).to_string()
        },
        meta,
        lines: Vec::new(),
        entries: visible,
    }
}

struct FilePreview {
    detail_label: String,
    meta: Vec<SharedString>,
    lines: Vec<SharedString>,
    preview_toggle_available: bool,
}

fn build_file_preview(path: &Path, file_size: u64, preview_mode: PathPreviewMode) -> FilePreview {
    let budget_bytes = preview_mode.max_bytes();
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(err) => {
            return FilePreview {
                detail_label: t!("App.PathInspector.file_preview_unavailable").to_string(),
                meta: vec![t!("App.PathInspector.read_error", error = err.to_string()).into()],
                lines: vec![t!("App.PathInspector.file_preview_unavailable").into()],
                preview_toggle_available: false,
            };
        }
    };

    let mut bytes = Vec::with_capacity(budget_bytes.saturating_add(1));
    if let Err(err) = (&mut file)
        .take((budget_bytes.saturating_add(1)) as u64)
        .read_to_end(&mut bytes)
    {
        return FilePreview {
            detail_label: t!("App.PathInspector.file_preview_unavailable").to_string(),
            meta: vec![t!("App.PathInspector.read_error", error = err.to_string()).into()],
            lines: vec![t!("App.PathInspector.file_preview_unavailable").into()],
            preview_toggle_available: false,
        };
    }

    let truncated_by_bytes = bytes.len() > budget_bytes || file_size > budget_bytes as u64;
    if bytes.len() > budget_bytes {
        bytes.truncate(budget_bytes);
    }

    let compact_budget_exceeded = file_size > COMPACT_PREVIEW_BYTES as u64;

    match decode_text_preview(&bytes) {
        Ok(decoded) => {
            let all_lines = split_preview_lines(&decoded.text);
            let truncated_by_lines = all_lines.len() > preview_mode.max_lines();
            let preview_toggle_available =
                compact_budget_exceeded || all_lines.len() > COMPACT_PREVIEW_LINES;
            let mut lines = all_lines
                .iter()
                .take(preview_mode.max_lines())
                .map(|line| truncate_line(&sanitize_preview_line(line)).into())
                .collect::<Vec<SharedString>>();
            if lines.is_empty() {
                lines.push(t!("App.PathInspector.file_empty").into());
            }

            let mut meta = vec![
                t!("App.PathInspector.encoding_label", encoding = decoded.encoding_label).into(),
                t!(
                    "App.PathInspector.line_endings_label",
                    endings = detect_line_endings(&decoded.text)
                )
                .into(),
                t!(
                    "App.PathInspector.preview_budget",
                    budget = format_bytes(preview_mode.max_bytes() as u64),
                    lines = preview_mode.max_lines()
                )
                .into(),
            ];

            if truncated_by_bytes || truncated_by_lines {
                meta.push(
                    t!(
                        "App.PathInspector.preview_truncated",
                        shown = lines.len(),
                        budget = format_bytes(preview_mode.max_bytes() as u64),
                        total = format_bytes(file_size)
                    )
                    .into(),
                );
            }

            FilePreview {
                detail_label: format!(
                    "{} · {} · {}",
                    t!("App.PathInspector.kind_text"),
                    decoded.encoding_label,
                    detect_line_endings(&decoded.text)
                ),
                meta,
                lines,
                preview_toggle_available,
            }
        }
        Err(TextPreviewError::Binary(reason)) => FilePreview {
            detail_label: t!("App.PathInspector.binary_file").to_string(),
            meta: vec![t!("App.PathInspector.binary_detection", reason = reason).into()],
            lines: vec![t!("App.PathInspector.binary_preview_unavailable").into()],
            preview_toggle_available: false,
        },
        Err(TextPreviewError::UnsupportedEncoding(reason)) => FilePreview {
            detail_label: t!("App.PathInspector.unsupported_encoding").to_string(),
            meta: vec![
                t!("App.PathInspector.encoding_label", encoding = reason).into(),
                t!(
                    "App.PathInspector.preview_budget",
                    budget = format_bytes(preview_mode.max_bytes() as u64),
                    lines = preview_mode.max_lines()
                )
                .into(),
            ],
            lines: vec![t!("App.PathInspector.text_preview_unavailable").into()],
            preview_toggle_available: false,
        },
    }
}

#[derive(Debug)]
enum TextPreviewError {
    Binary(&'static str),
    UnsupportedEncoding(&'static str),
}

#[derive(Debug)]
struct DecodedTextPreview {
    text: String,
    encoding_label: &'static str,
}

fn decode_text_preview(bytes: &[u8]) -> Result<DecodedTextPreview, TextPreviewError> {
    if bytes.is_empty() {
        return Ok(DecodedTextPreview {
            text: String::new(),
            encoding_label: "UTF-8",
        });
    }

    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        let text = String::from_utf8(bytes[3..].to_vec())
            .map_err(|_| TextPreviewError::UnsupportedEncoding("invalid UTF-8 with BOM"))?;
        return Ok(DecodedTextPreview {
            text,
            encoding_label: "UTF-8 BOM",
        });
    }

    if bytes.starts_with(&[0xff, 0xfe]) {
        return decode_utf16_preview(&bytes[2..], true, "UTF-16 LE");
    }

    if bytes.starts_with(&[0xfe, 0xff]) {
        return decode_utf16_preview(&bytes[2..], false, "UTF-16 BE");
    }

    if bytes.contains(&0) {
        return Err(TextPreviewError::Binary("NUL byte detected"));
    }

    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => Ok(DecodedTextPreview {
            text,
            encoding_label: "UTF-8",
        }),
        Err(err) => {
            if looks_binary(err.as_bytes()) {
                Err(TextPreviewError::Binary("control-byte density is too high"))
            } else {
                Err(TextPreviewError::UnsupportedEncoding(
                    "non UTF-8 / UTF-16 text",
                ))
            }
        }
    }
}

fn decode_utf16_preview(
    bytes: &[u8],
    little_endian: bool,
    encoding_label: &'static str,
) -> Result<DecodedTextPreview, TextPreviewError> {
    if bytes.contains(&0) && bytes.len() < 2 {
        return Err(TextPreviewError::Binary("incomplete UTF-16 byte pair"));
    }

    let units = bytes
        .chunks_exact(2)
        .map(|chunk| {
            if little_endian {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        })
        .collect::<Vec<_>>();

    let text = String::from_utf16(&units)
        .map_err(|_| TextPreviewError::UnsupportedEncoding("invalid UTF-16 text"))?;
    Ok(DecodedTextPreview {
        text,
        encoding_label,
    })
}

fn looks_binary(bytes: &[u8]) -> bool {
    let suspicious = bytes
        .iter()
        .filter(|byte| matches!(byte, 0x00..=0x08 | 0x0b | 0x0e..=0x1a | 0x1c..=0x1f))
        .count();
    suspicious > 0 && suspicious.saturating_mul(8) >= bytes.len().max(1)
}

fn split_preview_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0usize;
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                let end = if index > 0 && bytes[index - 1] == b'\r' {
                    index - 1
                } else {
                    index
                };
                lines.push(&text[start..end]);
                index += 1;
                start = index;
            }
            b'\r' => {
                lines.push(&text[start..index]);
                index += 1;
                if index < bytes.len() && bytes[index] == b'\n' {
                    index += 1;
                }
                start = index;
            }
            _ => {
                index += 1;
            }
        }
    }

    if start < text.len() {
        lines.push(&text[start..]);
    }

    lines
}

fn detect_line_endings(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut saw_crlf = false;
    let mut saw_lf = false;
    let mut saw_cr = false;
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                if index + 1 < bytes.len() && bytes[index + 1] == b'\n' {
                    saw_crlf = true;
                    index += 2;
                } else {
                    saw_cr = true;
                    index += 1;
                }
            }
            b'\n' => {
                saw_lf = true;
                index += 1;
            }
            _ => {
                index += 1;
            }
        }
    }

    match (saw_crlf, saw_lf, saw_cr) {
        (false, false, false) => t!("App.PathInspector.LineEndings.none").to_string(),
        (true, false, false) => t!("App.PathInspector.LineEndings.crlf").to_string(),
        (false, true, false) => t!("App.PathInspector.LineEndings.lf").to_string(),
        (false, false, true) => t!("App.PathInspector.LineEndings.cr").to_string(),
        _ => t!("App.PathInspector.LineEndings.mixed").to_string(),
    }
}

fn sanitize_preview_line(line: &str) -> String {
    line.chars()
        .flat_map(|ch| match ch {
            '\t' => "    ".chars().collect::<Vec<_>>(),
            ch if ch.is_control() => vec!['?'],
            ch => vec![ch],
        })
        .collect()
}

fn inspector_action_elements(
    snapshot: &PathInspectorSnapshot,
    app: &WeakEntity<PierApp>,
) -> Vec<AnyElement> {
    let mut actions = Vec::new();

    if snapshot.parent_target_string().is_some() {
        let app = app.clone();
        actions.push(
            Button::ghost("path-inspector-parent", t!("App.PathInspector.open_parent"))
                .on_click(move |_, window, cx| {
                    let _ = app.update(cx, |this, cx| {
                        this.inspect_path_inspector_parent(window, cx);
                    });
                })
                .into_any_element(),
        );
    }

    if snapshot.is_file() && snapshot.preview_toggle_available {
        match snapshot.preview_mode {
            PathPreviewMode::Compact => {
                let app = app.clone();
                actions.push(
                    Button::ghost("path-inspector-expand", t!("App.PathInspector.expand_preview"))
                        .on_click(move |_, window, cx| {
                            let _ = app.update(cx, |this, cx| {
                                this.set_path_inspector_preview_mode(
                                    PathPreviewMode::Expanded,
                                    window,
                                    cx,
                                );
                            });
                        })
                        .into_any_element(),
                );
            }
            PathPreviewMode::Expanded => {
                let app = app.clone();
                actions.push(
                    Button::ghost("path-inspector-compact", t!("App.PathInspector.compact_preview"))
                        .on_click(move |_, window, cx| {
                            let _ = app.update(cx, |this, cx| {
                                this.set_path_inspector_preview_mode(
                                    PathPreviewMode::Compact,
                                    window,
                                    cx,
                                );
                            });
                        })
                        .into_any_element(),
                );
            }
        }
    }

    actions
}

fn directory_entry_elements(
    entries: &[PathInspectorEntry],
    app: &WeakEntity<PierApp>,
    t: &crate::theme::Theme,
) -> Vec<AnyElement> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let app = app.clone();
            let target = entry.target.to_string();
            let label = entry.label.clone();
            let detail = entry.detail_label.clone();
            let kind_label = entry.kind_label.clone();
            let kind = entry.status_kind;

            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(SP_3)
                .p(SP_2)
                .id(("path-inspector-entry", index))
                .rounded(RADIUS_SM)
                .border_1()
                .border_color(t.color.border_subtle)
                .bg(t.color.bg_panel)
                .cursor_pointer()
                .hover({
                    let hover = t.color.bg_hover;
                    move |style| style.bg(hover)
                })
                .on_click(move |_, window, cx| {
                    let target = target.clone();
                    let _ = app.update(cx, |this, cx| {
                        this.inspect_local_path(target, window, cx);
                    });
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SP_1)
                        .min_w(px(0.0))
                        .child(text::mono(label))
                        .child(text::caption(detail).secondary()),
                )
                .child(StatusPill::new(kind_label, kind))
                .into_any_element()
        })
        .collect()
}

fn truncate_line(value: &str) -> String {
    let mut chars = value.chars();
    let truncated = chars
        .by_ref()
        .take(MAX_PREVIEW_LINE_CHARS)
        .collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_text_preview, detect_line_endings, format_bytes, split_preview_lines, truncate_line,
        TextPreviewError,
    };

    #[test]
    fn formats_bytes_for_readable_metadata() {
        assert_eq!(format_bytes(912), "912 B");
        assert_eq!(format_bytes(2_048), "2.0 KB");
    }

    #[test]
    fn truncates_preview_line_with_single_ellipsis() {
        let input = "x".repeat(220);
        let output = truncate_line(&input);

        assert!(output.ends_with('…'));
        assert!(output.chars().count() <= 161);
    }

    #[test]
    fn splits_preview_lines_for_crlf_lf_and_cr() {
        let lines = split_preview_lines("a\r\nb\nc\rd");
        assert_eq!(lines, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn detects_mixed_line_endings() {
        assert_eq!(detect_line_endings("a\r\nb\nc"), "mixed".to_string());
        assert_eq!(detect_line_endings("a\r\nb"), "CRLF".to_string());
        assert_eq!(detect_line_endings("a\nb"), "LF".to_string());
        assert_eq!(detect_line_endings("a\rb"), "CR".to_string());
    }

    #[test]
    fn decodes_utf16le_bom_preview() {
        let decoded = decode_text_preview(&[0xff, 0xfe, b'h', 0, b'i', 0]).expect("utf16");
        assert_eq!(decoded.encoding_label, "UTF-16 LE");
        assert_eq!(decoded.text, "hi");
    }

    #[test]
    fn classifies_nul_bytes_as_binary_preview() {
        let err = decode_text_preview(b"ab\0cd").expect_err("binary");
        assert!(matches!(err, TextPreviewError::Binary(_)));
    }
}
