// SFTP panel — remote file browser over an SSH session.
//
// Pick a saved connection, open an SFTP channel off the render path, and walk
// the remote tree: directories first, click a folder to descend, a ".." row to
// go back up. Each row shows its permission bits (rwx) and size, and exposes
// inline actions:
//
//   * New file / New folder — header buttons open an inline name input.
//   * Rename — a row button flips the name cell into an inline input.
//   * Delete — a trash button asks for inline confirmation first.
//   * chmod — clicking the permission cell opens an inline octal input.
//   * Download / Upload — native save / open dialogs.
//
// Every mutation runs over the cached SftpClient on the background executor and
// re-lists the current directory on success. Failures surface as one error line.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{
    div, px, Context, FocusHandle, Hsla, KeyDownEvent, MouseButton, MouseDownEvent,
    PathPromptOptions, SharedString, Window,
};
use gpui_component::{h_flex, v_flex};

use pier_core::ssh::{RemoteFileEntry, SftpClient, SshConfig, SshSession};

use crate::data;
use crate::theme::Theme;
use crate::ui;

/// An inline editing action that temporarily captures keyboard input. Only one
/// is active at a time; the panel renders the matching inline control and
/// `on_input_key` feeds keystrokes into the active buffer.
enum Edit {
    None,
    /// New file (`is_dir = false`) or folder in the current directory.
    New { is_dir: bool, name: String },
    /// Rename the entry at `path`; `name` is the edited leaf name.
    Rename { path: String, name: String },
    /// Change permissions on `path`; `mode` accumulates octal digits.
    Chmod { path: String, mode: String },
    /// Awaiting confirmation before deleting `path`.
    ConfirmDelete { path: String, is_dir: bool },
}

/// The inline text editor that temporarily replaces the listing. It is a plain
/// append / end-backspace buffer — no cursor positioning, selection, or
/// mid-line insertion (gpui has no text-input widget here). Meant for tweaking
/// small remote config files, not as an IDE.
struct Editor {
    /// Absolute remote path being edited.
    path: String,
    /// Leaf name, shown in the editor header.
    name: String,
    /// The editable text. Edits only ever append to or pop from the end.
    buf: String,
    /// Buffer differs from the loaded contents.
    dirty: bool,
    /// The contents loaded successfully and the buffer is safe to write back.
    /// Stays false on a read or decode failure so Save can't clobber the remote
    /// file with an empty/partial buffer.
    ready: bool,
    /// A read is in flight off the render path.
    loading: bool,
    /// A write is in flight off the render path.
    saving: bool,
    /// Last load/save error, shown in the editor header area.
    error: Option<String>,
    /// Set after one close request on a dirty buffer; the next one discards.
    confirm_close: bool,
}

pub struct SftpPanel {
    theme: Theme,
    /// Saved connections, loaded once on construction.
    conns: Vec<SshConfig>,
    /// Live session + SFTP channel once connected. The session is held so the
    /// underlying SSH connection (and thus the SFTP channel) stays open.
    session: Option<SshSession>,
    sftp: Option<SftpClient>,
    /// Name of the connection we're browsing, for the header meta.
    conn_name: String,
    /// Current remote directory and its listing.
    cwd: String,
    entries: Vec<RemoteFileEntry>,
    /// A connect or list is in flight off the render path.
    loading: bool,
    /// Last connect/list error, shown as one line.
    error: Option<String>,
    /// Focus handle for whichever inline input is currently shown.
    input_focus: FocusHandle,
    /// The in-progress inline action, if any.
    edit: Edit,
    /// `"{user}@{host}:{port}"` of the live connection — the bookmark key.
    host_key: String,
    /// Bookmarked directory paths for the current host (persisted on change).
    bookmarks: Vec<String>,
    /// Whether the bookmarks dropdown is expanded under the toolbar.
    bookmarks_open: bool,
    /// The open file editor, if any. While set it takes over the panel body.
    editor: Option<Editor>,
    /// Focus handle for the editor's key capture.
    editor_focus: FocusHandle,
}

impl SftpPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
            conns: data::connections_raw(),
            session: None,
            sftp: None,
            conn_name: String::new(),
            cwd: String::new(),
            entries: Vec::new(),
            loading: false,
            error: None,
            input_focus: cx.focus_handle(),
            edit: Edit::None,
            host_key: String::new(),
            bookmarks: Vec::new(),
            bookmarks_open: false,
            editor: None,
            editor_focus: cx.focus_handle(),
        }
    }

    /// Connect to the saved config at `idx`, open SFTP, and list its home dir.
    /// All blocking work runs on the background executor; only the result is
    /// folded back into the View on the main thread.
    fn connect_to(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(cfg) = self.conns.get(idx).cloned() else {
            return;
        };
        self.loading = true;
        self.error = None;
        let name = cfg.name.clone();
        let host_key = format!("{}@{}:{}", cfg.user, cfg.host, cfg.port);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let session = data::connect_blocking(&cfg)?;
                    let sftp = session.open_sftp_blocking().map_err(|e| e.to_string())?;
                    let cwd = sftp
                        .canonicalize_blocking(".")
                        .unwrap_or_else(|_| "/".to_string());
                    let entries = sftp.list_dir_blocking(&cwd).map_err(|e| e.to_string())?;
                    Ok::<_, String>((session, sftp, cwd, entries))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((session, sftp, cwd, entries)) => {
                        this.session = Some(session);
                        this.sftp = Some(sftp);
                        this.conn_name = name;
                        this.cwd = cwd;
                        this.entries = entries;
                        this.error = None;
                        this.bookmarks = data::load_sftp_bookmarks(&host_key);
                        this.host_key = host_key;
                        this.bookmarks_open = false;
                        this.editor = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// List `path` on the current session and make it the new cwd.
    fn navigate(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(sftp) = self.sftp.clone() else {
            return;
        };
        self.edit = Edit::None;
        self.bookmarks_open = false;
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let listed = {
                let path = path.clone();
                cx.background_executor()
                    .spawn(async move { sftp.list_dir_blocking(&path).map_err(|e| e.to_string()) })
                    .await
            };
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match listed {
                    Ok(entries) => {
                        this.entries = entries;
                        this.cwd = path;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Run a mutating SFTP op off the render path, then re-list the cwd so the
    /// new state is reflected. Mirrors the connect/list background pattern.
    fn mutate<F>(&mut self, op: F, cx: &mut Context<Self>)
    where
        F: FnOnce(&SftpClient) -> Result<(), String> + Send + 'static,
    {
        let Some(sftp) = self.sftp.clone() else {
            return;
        };
        let dir = self.cwd.clone();
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    op(&sftp)?;
                    sftp.list_dir_blocking(&dir).map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok(entries) => {
                        this.entries = entries;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Download a remote file to a local path chosen via the native dialog.
    fn download(&mut self, remote: String, name: String, cx: &mut Context<Self>) {
        let Some(sftp) = self.sftp.clone() else {
            return;
        };
        let dir = data::current_dir();
        cx.spawn(async move |this, cx| {
            let recv = cx.update(|cx| cx.prompt_for_new_path(&dir, Some(name.as_str())));
            let Ok(Ok(Some(local))) = recv.await else {
                return; // cancelled or errored
            };
            let res = cx
                .background_executor()
                .spawn(async move {
                    sftp.download_to_blocking(&remote, &local)
                        .map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.error = res.err();
                cx.notify();
            });
        })
        .detach();
    }

    /// Upload a locally-chosen file into the current remote directory.
    fn upload(&mut self, cx: &mut Context<Self>) {
        let Some(sftp) = self.sftp.clone() else {
            return;
        };
        let remote_dir = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let opts = PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: None,
            };
            let recv = cx.update(|cx| cx.prompt_for_paths(opts));
            let Ok(Ok(Some(paths))) = recv.await else {
                return;
            };
            let Some(local) = paths.into_iter().next() else {
                return;
            };
            let fname = local
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let remote = join_remote(&remote_dir, &fname);
            let listed = cx
                .background_executor()
                .spawn(async move {
                    sftp.upload_from_blocking(&local, &remote)
                        .map_err(|e| e.to_string())?;
                    sftp.list_dir_blocking(&remote_dir).map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match listed {
                    Ok(entries) => {
                        this.entries = entries;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Feed a keystroke into the active inline input. Enter commits, Escape
    /// cancels, Backspace pops; printable characters append (chmod only takes
    /// up to four octal digits).
    fn on_input_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "enter" => {
                self.commit_edit(cx);
                return;
            }
            "escape" => {
                self.edit = Edit::None;
                cx.notify();
                return;
            }
            "backspace" => {
                let changed = match &mut self.edit {
                    Edit::New { name, .. } | Edit::Rename { name, .. } => name.pop().is_some(),
                    Edit::Chmod { mode, .. } => mode.pop().is_some(),
                    _ => false,
                };
                if changed {
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
            if kc.is_empty() || kc.chars().any(|c| c.is_control()) {
                return;
            }
            let changed = match &mut self.edit {
                Edit::New { name, .. } | Edit::Rename { name, .. } => {
                    name.push_str(kc);
                    true
                }
                Edit::Chmod { mode, .. } => {
                    let mut any = false;
                    for c in kc.chars() {
                        if ('0'..='7').contains(&c) && mode.len() < 4 {
                            mode.push(c);
                            any = true;
                        }
                    }
                    any
                }
                _ => false,
            };
            if changed {
                cx.notify();
            }
        }
    }

    /// Apply whatever inline edit is in progress, then clear it.
    fn commit_edit(&mut self, cx: &mut Context<Self>) {
        match std::mem::replace(&mut self.edit, Edit::None) {
            Edit::New { is_dir, name } => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    cx.notify();
                    return;
                }
                let path = join_remote(&self.cwd, &name);
                if is_dir {
                    self.mutate(move |s| s.create_dir_blocking(&path).map_err(|e| e.to_string()), cx);
                } else {
                    self.mutate(move |s| s.create_file_blocking(&path).map_err(|e| e.to_string()), cx);
                }
            }
            Edit::Rename { path, name } => {
                let name = name.trim().to_string();
                let to = join_remote(&parent_of(&path), &name);
                if name.is_empty() || to == path {
                    cx.notify();
                    return;
                }
                self.mutate(move |s| s.rename_blocking(&path, &to).map_err(|e| e.to_string()), cx);
            }
            Edit::Chmod { path, mode } => match u32::from_str_radix(mode.trim(), 8) {
                Ok(m) => {
                    self.mutate(move |s| s.set_permissions_blocking(&path, m).map_err(|e| e.to_string()), cx)
                }
                Err(_) => {
                    self.error = Some(format!("Invalid octal mode: {mode}"));
                    cx.notify();
                }
            },
            Edit::ConfirmDelete { .. } | Edit::None => cx.notify(),
        }
    }

    /// Delete the entry staged by [`Edit::ConfirmDelete`] (driven by the inline
    /// confirm button, not the keyboard).
    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        if let Edit::ConfirmDelete { path, is_dir } = std::mem::replace(&mut self.edit, Edit::None) {
            if is_dir {
                self.mutate(move |s| s.remove_dir_blocking(&path).map_err(|e| e.to_string()), cx);
            } else {
                self.mutate(move |s| s.remove_file_blocking(&path).map_err(|e| e.to_string()), cx);
            }
        } else {
            cx.notify();
        }
    }

    /// Toggle the current directory in this host's bookmarks and persist.
    fn toggle_bookmark(&mut self, cx: &mut Context<Self>) {
        if self.cwd.is_empty() {
            return;
        }
        if let Some(pos) = self.bookmarks.iter().position(|b| *b == self.cwd) {
            self.bookmarks.remove(pos);
        } else {
            self.bookmarks.push(self.cwd.clone());
        }
        data::save_sftp_bookmarks(&self.host_key, &self.bookmarks);
        cx.notify();
    }

    /// Drop `path` from this host's bookmarks and persist.
    fn remove_bookmark(&mut self, path: String, cx: &mut Context<Self>) {
        if let Some(pos) = self.bookmarks.iter().position(|b| *b == path) {
            self.bookmarks.remove(pos);
            data::save_sftp_bookmarks(&self.host_key, &self.bookmarks);
            if self.bookmarks.is_empty() {
                self.bookmarks_open = false;
            }
            cx.notify();
        }
    }

    /// Open the text editor on a remote file. Files over 1 MiB are refused (use
    /// download). The read runs off the render path; until it lands the editor
    /// shows "Loading…".
    fn open_editor(
        &mut self,
        path: String,
        name: String,
        size: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        const MAX_EDIT_BYTES: u64 = 1024 * 1024;
        if size > MAX_EDIT_BYTES {
            self.error = Some(format!("{name} is larger than 1 MiB — download to edit"));
            cx.notify();
            return;
        }
        let Some(sftp) = self.sftp.clone() else {
            return;
        };
        self.error = None;
        self.editor = Some(Editor {
            path: path.clone(),
            name,
            buf: String::new(),
            dirty: false,
            ready: false,
            loading: true,
            saving: false,
            error: None,
            confirm_close: false,
        });
        window.focus(&self.editor_focus, cx);
        cx.notify();
        let read_path = path.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { sftp.read_file_blocking(&read_path).map_err(|e| e.to_string()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Apply only if the editor is still open for this same file.
                if let Some(ed) = &mut this.editor {
                    if ed.path == path {
                        ed.loading = false;
                        match res {
                            Ok(bytes) => match String::from_utf8(bytes) {
                                Ok(text) => {
                                    ed.buf = text;
                                    ed.ready = true;
                                    ed.error = None;
                                }
                                Err(_) => {
                                    ed.error = Some(
                                        "Not a UTF-8 text file — close and download instead"
                                            .to_string(),
                                    );
                                }
                            },
                            Err(e) => ed.error = Some(e),
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Write the editor buffer back to its remote path, then re-list the cwd so
    /// the row's size/mtime refresh. No-op while loading/saving or before a
    /// successful load (so an empty buffer can't clobber the file).
    fn save_editor(&mut self, cx: &mut Context<Self>) {
        let (path, bytes) = {
            let Some(ed) = self.editor.as_mut() else {
                return;
            };
            if !ed.ready || ed.loading || ed.saving {
                return;
            }
            ed.saving = true;
            ed.error = None;
            (ed.path.clone(), ed.buf.clone().into_bytes())
        };
        let Some(sftp) = self.sftp.clone() else {
            return;
        };
        let dir = self.cwd.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    sftp.write_file_blocking(&path, &bytes).map_err(|e| e.to_string())?;
                    sftp.list_dir_blocking(&dir).map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(entries) => {
                        this.entries = entries;
                        if let Some(ed) = &mut this.editor {
                            ed.saving = false;
                            ed.dirty = false;
                            ed.error = None;
                        }
                    }
                    Err(e) => {
                        if let Some(ed) = &mut this.editor {
                            ed.saving = false;
                            ed.error = Some(e);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Close the editor. A dirty buffer needs a second request (Esc or the
    /// Close button) to discard unsaved changes.
    fn request_close_editor(&mut self, cx: &mut Context<Self>) {
        let confirm = matches!(&self.editor, Some(ed) if ed.dirty && !ed.confirm_close);
        if confirm {
            if let Some(ed) = &mut self.editor {
                ed.confirm_close = true;
            }
        } else {
            self.editor = None;
        }
        cx.notify();
    }

    /// Feed a keystroke into the open editor: Ctrl+S saves, Escape closes (with
    /// a dirty-buffer confirm), Enter/Tab/Backspace and printable characters
    /// mutate the end of the buffer. Buffer edits are ignored until the file has
    /// loaded successfully.
    fn on_editor_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        if m.control && !m.alt && ks.key.as_str() == "s" {
            self.save_editor(cx);
            return;
        }
        if ks.key.as_str() == "escape" {
            self.request_close_editor(cx);
            return;
        }
        let Some(ed) = self.editor.as_mut() else {
            return;
        };
        if !ed.ready {
            return;
        }
        match ks.key.as_str() {
            "enter" => ed.buf.push('\n'),
            "tab" => ed.buf.push('\t'),
            "backspace" => {
                if ed.buf.pop().is_none() {
                    return;
                }
            }
            _ => {
                if m.control || m.alt || m.platform {
                    return;
                }
                match &ks.key_char {
                    Some(kc) if !kc.is_empty() && !kc.chars().any(|c| c.is_control()) => {
                        ed.buf.push_str(kc)
                    }
                    _ => return,
                }
            }
        }
        ed.dirty = true;
        ed.confirm_close = false;
        cx.notify();
    }

    fn conn_row(&self, cx: &mut Context<Self>, idx: usize, c: &SshConfig) -> impl IntoElement {
        let t = &self.theme;
        let addr = format!("{}@{}:{}", c.user, c.host, c.port);
        h_flex()
            .id(SharedString::from(format!("sftp-conn-{idx}")))
            .items_center()
            .gap(t.sp2)
            .h(px(42.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.connect_to(idx, cx);
                }),
            )
            .child(ui::icon("folder", px(15.0), t.accent))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(div().overflow_hidden().text_color(t.ink_2).child(c.name.clone()))
                    .child(
                        div()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(addr),
                    ),
            )
    }

    /// Header row: connection name + bookmark / new-folder / new-file / upload
    /// buttons.
    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let starred = !self.cwd.is_empty() && self.bookmarks.iter().any(|b| *b == self.cwd);
        let star_glyph = if starred { "star-fill" } else { "star" };
        let has_bookmarks = !self.bookmarks.is_empty();
        h_flex()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(ui::section_label(t, self.conn_name.clone())),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(4.0))
                    .mr(t.sp3)
                    .child(self.head_btn(cx, "sftp-bookmark", star_glyph, |this, _window, cx| {
                        this.toggle_bookmark(cx);
                    }))
                    .when(has_bookmarks, |d| {
                        d.child(self.head_btn(cx, "sftp-bookmarks", "book-open", |this, _window, cx| {
                            this.bookmarks_open = !this.bookmarks_open;
                            cx.notify();
                        }))
                    })
                    .child(self.head_btn(cx, "sftp-new-dir", "folder", |this, window, cx| {
                        this.edit = Edit::New { is_dir: true, name: String::new() };
                        window.focus(&this.input_focus, cx);
                        cx.notify();
                    }))
                    .child(self.head_btn(cx, "sftp-new-file", "file", |this, window, cx| {
                        this.edit = Edit::New { is_dir: false, name: String::new() };
                        window.focus(&this.input_focus, cx);
                        cx.notify();
                    }))
                    .child(self.head_btn(cx, "sftp-upload", "arrow-up", |this, _window, cx| {
                        this.upload(cx);
                    })),
            )
    }

    /// A 24px ghost icon button used in the header toolbar.
    fn head_btn(
        &self,
        cx: &mut Context<Self>,
        id: &'static str,
        glyph: &'static str,
        handler: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(24.0))
            .h(px(24.0))
            .rounded(t.radius_sm)
            .cursor_pointer()
            .hover(|s| s.bg(t.elev))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| handler(this, window, cx)),
            )
            .child(ui::icon(glyph, px(14.0), t.ink_2))
    }

    /// An 18px ghost icon button used at the end of an entry row.
    fn row_btn(
        &self,
        cx: &mut Context<Self>,
        id: String,
        glyph: &'static str,
        color: Hsla,
        handler: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(SharedString::from(id))
            .flex()
            .items_center()
            .justify_center()
            .w(px(18.0))
            .h(px(18.0))
            .rounded(t.radius_sm)
            .cursor_pointer()
            .hover(|s| s.bg(t.elev))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| handler(this, window, cx)),
            )
            .child(ui::icon(glyph, px(13.0), color))
    }

    /// A single-line inline text input bound to [`Self::input_focus`]. The
    /// caller wraps it in a sized cell; the active buffer lives in `self.edit`.
    fn inline_input(
        &self,
        cx: &mut Context<Self>,
        value: String,
        placeholder: &'static str,
    ) -> impl IntoElement {
        let t = &self.theme;
        let empty = value.is_empty();
        div()
            .track_focus(&self.input_focus)
            .key_context("SftpInput")
            .on_key_down(cx.listener(Self::on_input_key))
            .w_full()
            .h(px(20.0))
            .px(t.sp1)
            .flex()
            .items_center()
            .rounded(t.radius_sm)
            .bg(t.panel_2)
            .border_1()
            .border_color(t.accent)
            .font_family(t.mono.clone())
            .text_size(t.fs_sm)
            .when(empty, |d| d.text_color(t.dim).child(placeholder))
            .when(!empty, |d| d.text_color(t.ink).child(value))
    }

    fn up_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let parent = parent_of(&self.cwd);
        h_flex()
            .id("sftp-up")
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .text_color(t.ink_2)
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.navigate(parent.clone(), cx);
                }),
            )
            .child(ui::icon("folder", px(14.0), t.muted))
            .child(div().flex_1().font_family(t.mono.clone()).child(".."))
    }

    /// The inline "new file/folder" name input, shown under the toolbar.
    fn new_entry_row(&self, cx: &mut Context<Self>, is_dir: bool, name: String) -> impl IntoElement {
        let t = &self.theme;
        let glyph = if is_dir { "folder" } else { "file" };
        let placeholder = if is_dir { "New folder name…" } else { "New file name…" };
        h_flex()
            .items_center()
            .gap(t.sp2)
            .h(px(28.0))
            .px(t.sp3)
            .child(ui::icon(glyph, px(14.0), t.accent))
            .child(div().flex_1().min_w(px(0.0)).child(self.inline_input(cx, name, placeholder)))
    }

    fn entry_row(&self, cx: &mut Context<Self>, e: &RemoteFileEntry) -> impl IntoElement {
        let t = &self.theme;
        let glyph = if e.is_dir { "folder" } else { "file" };
        let glyph_color = if e.is_dir { t.accent } else { t.muted };
        let is_dir = e.is_dir;
        let size = if is_dir { String::new() } else { human_size(e.size) };

        // Which inline control (if any) is bound to this row.
        let editing_name = match &self.edit {
            Edit::Rename { path, name } if *path == e.path => Some(name.clone()),
            _ => None,
        };
        let editing_mode = match &self.edit {
            Edit::Chmod { path, mode } if *path == e.path => Some(mode.clone()),
            _ => None,
        };
        let confirming = matches!(&self.edit, Edit::ConfirmDelete { path, .. } if *path == e.path);

        // Name region (icon + name or rename input). Only directories navigate
        // on click, and only when not being renamed — the input owns its clicks.
        let mut nav = h_flex()
            .id(SharedString::from(format!("sftp-nav-{}", e.path)))
            .items_center()
            .gap(t.sp2)
            .flex_1()
            .min_w(px(0.0))
            .child(ui::icon(glyph, px(14.0), glyph_color));
        if let Some(val) = editing_name {
            nav = nav.child(div().flex_1().min_w(px(0.0)).child(self.inline_input(cx, val, "name…")));
        } else {
            nav = nav.child(div().flex_1().overflow_hidden().child(e.name.clone()));
            if is_dir {
                let np = e.path.clone();
                nav = nav.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                        this.navigate(np.clone(), cx);
                    }),
                );
            }
        }

        // Permission cell — inline octal input, or clickable rwx text (chmod).
        let perm_cell = if let Some(val) = editing_mode {
            div().w(px(62.0)).child(self.inline_input(cx, val, "octal")).into_any_element()
        } else {
            let cp = e.path.clone();
            let seed = e.permissions.map(|p| format!("{:o}", p & 0o777)).unwrap_or_default();
            let perm_text = e.permissions.map(perm_rwx).unwrap_or_else(|| "—".to_string());
            div()
                .id(SharedString::from(format!("sftp-perm-{}", e.path)))
                .w(px(62.0))
                .font_family(t.mono.clone())
                .text_size(t.fs_sm)
                .text_color(t.muted)
                .cursor_pointer()
                .hover(|s| s.text_color(t.ink_2))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.edit = Edit::Chmod { path: cp.clone(), mode: seed.clone() };
                        window.focus(&this.input_focus, cx);
                        cx.notify();
                    }),
                )
                .child(perm_text)
                .into_any_element()
        };

        let size_cell = div()
            .w(px(48.0))
            .flex()
            .justify_end()
            .font_family(t.mono.clone())
            .text_size(t.fs_sm)
            .text_color(t.muted)
            .child(size);

        // Owner (from the server's longname) and last-modified age. Both stay
        // blank when the SFTP server omitted the field.
        let owner_cell = div()
            .w(px(64.0))
            .overflow_hidden()
            .font_family(t.mono.clone())
            .text_size(t.fs_sm)
            .text_color(t.muted)
            .child(e.owner.clone().unwrap_or_default());
        let mod_cell = div()
            .w(px(44.0))
            .flex()
            .justify_end()
            .font_family(t.mono.clone())
            .text_size(t.fs_sm)
            .text_color(t.muted)
            .child(e.modified.map(rel_age).unwrap_or_default());

        // Trailing actions — inline delete confirmation, or the action buttons.
        let trailing = if confirming {
            h_flex()
                .items_center()
                .gap(px(2.0))
                .child(
                    div()
                        .mr(px(2.0))
                        .text_size(t.fs_sm)
                        .text_color(t.neg)
                        .child("Delete?"),
                )
                .child(self.row_btn(cx, format!("sftp-yes-{}", e.path), "check", t.neg, |this, _w, cx| {
                    this.confirm_delete(cx);
                }))
                .child(self.row_btn(cx, format!("sftp-no-{}", e.path), "close", t.muted, |this, _w, cx| {
                    this.edit = Edit::None;
                    cx.notify();
                }))
                .into_any_element()
        } else {
            let rp = e.path.clone();
            let rn = e.name.clone();
            let dp = e.path.clone();
            let mut acts = h_flex()
                .items_center()
                .gap(px(2.0))
                .child(self.row_btn(cx, format!("sftp-rn-{}", e.path), "replace", t.muted, move |this, window, cx| {
                    this.edit = Edit::Rename { path: rp.clone(), name: rn.clone() };
                    window.focus(&this.input_focus, cx);
                    cx.notify();
                }))
                .child(self.row_btn(cx, format!("sftp-rm-{}", e.path), "delete", t.muted, move |this, _w, cx| {
                    this.edit = Edit::ConfirmDelete { path: dp.clone(), is_dir };
                    cx.notify();
                }));
            if !is_dir {
                let ep = e.path.clone();
                let en = e.name.clone();
                let esz = e.size;
                acts = acts.child(self.row_btn(cx, format!("sftp-ed-{}", e.path), "file-text", t.muted, move |this, window, cx| {
                    this.open_editor(ep.clone(), en.clone(), esz, window, cx);
                }));
                let dlp = e.path.clone();
                let dln = e.name.clone();
                acts = acts.child(self.row_btn(cx, format!("sftp-dl-{}", e.path), "arrow-down", t.muted, move |this, _w, cx| {
                    this.download(dlp.clone(), dln.clone(), cx);
                }));
            }
            acts.into_any_element()
        };

        h_flex()
            .id(SharedString::from(format!("sftp-entry-{}", e.path)))
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .text_color(t.ink_2)
            .hover(|s| s.bg(t.hover))
            .child(nav)
            .child(owner_cell)
            .child(mod_cell)
            .child(perm_cell)
            .child(size_cell)
            .child(trailing)
    }

    /// One row in the bookmarks dropdown: a star glyph, the clickable path
    /// (navigates), and a remove button.
    fn bookmark_row(&self, cx: &mut Context<Self>, path: String) -> impl IntoElement {
        let t = &self.theme;
        let nav_path = path.clone();
        let del_path = path.clone();
        h_flex()
            .id(SharedString::from(format!("sftp-bm-{path}")))
            .items_center()
            .gap(t.sp2)
            .h(px(24.0))
            .px(t.sp3)
            .text_color(t.ink_2)
            .hover(|s| s.bg(t.hover))
            .child(ui::icon("star-fill", px(12.0), t.warn))
            .child(
                div()
                    .id(SharedString::from(format!("sftp-bm-go-{path}")))
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.navigate(nav_path.clone(), cx);
                        }),
                    )
                    .child(path.clone()),
            )
            .child(self.row_btn(cx, format!("sftp-bm-del-{path}"), "star-off", t.muted, move |this, _w, cx| {
                this.remove_bookmark(del_path.clone(), cx);
            }))
    }

    /// The full-height text editor that replaces the listing while a file is
    /// open: a header (name + state + Save/Close) over a scrollable, monospace
    /// rendering of the buffer, line by line.
    fn editor_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        // The outer container owns focus + key capture for the whole editor.
        let mut col = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .track_focus(&self.editor_focus)
            .key_context("SftpEditor")
            .on_key_down(cx.listener(Self::on_editor_key));
        let Some(ed) = self.editor.as_ref() else {
            return col;
        };

        // Header: file glyph + name, then a state hint and Save/Close buttons.
        let mut header = h_flex()
            .items_center()
            .gap(t.sp2)
            .h(px(32.0))
            .px(t.sp3)
            .border_b_1()
            .border_color(t.line)
            .child(ui::icon("file-text", px(14.0), t.accent))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .font_family(t.mono.clone())
                    .text_color(t.ink)
                    .child(ed.name.clone()),
            );
        if ed.saving {
            header = header.child(div().text_size(t.fs_sm).text_color(t.muted).child("Saving…"));
        } else if ed.dirty {
            header = header.child(ui::status_dot(t.warn));
        }
        if ed.ready {
            header = header.child(self.head_btn(cx, "sftp-ed-save", "check", |this, _w, cx| {
                this.save_editor(cx);
            }));
        }
        header = header.child(self.head_btn(cx, "sftp-ed-close", "close", |this, _w, cx| {
            this.request_close_editor(cx);
        }));
        col = col.child(header);

        // A load/save error or the discard-confirm hint, just under the header.
        if let Some(err) = &ed.error {
            col = col.child(
                div().px(t.sp3).py(t.sp2).text_size(t.fs_sm).text_color(t.neg).child(err.clone()),
            );
        } else if ed.confirm_close {
            col = col.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_sm)
                    .text_color(t.warn)
                    .child("Unsaved changes — press Esc again or Close to discard."),
            );
        }

        // The buffer, rendered line by line. `split('\n')` keeps a trailing
        // blank line visible (so Enter at the end gives feedback); an empty
        // line renders a space so it keeps its height.
        let mut text = v_flex().w_full();
        if ed.loading {
            text = text.child(div().text_color(t.dim).child("Loading…"));
        } else {
            for line in ed.buf.split('\n') {
                let shown = if line.is_empty() { " ".to_string() } else { line.to_string() };
                text = text.child(div().w_full().child(shown));
            }
        }

        col.child(
            div()
                .id("sftp-editor-scroll")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .px(t.sp3)
                .py(t.sp2)
                .font_family(t.mono.clone())
                .text_size(t.fs_body)
                .text_color(t.ink)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                        window.focus(&this.editor_focus, cx);
                    }),
                )
                .child(text),
        )
    }

    fn error_line(&self) -> impl IntoElement {
        let t = &self.theme;
        div()
            .px(t.sp3)
            .py(t.sp2)
            .text_size(t.fs_sm)
            .text_color(t.neg)
            .child(self.error.clone().unwrap_or_default())
    }
}

impl Render for SftpPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = self.theme.clone();
        let meta: SharedString = if self.sftp.is_some() {
            self.cwd.clone().into()
        } else {
            SharedString::default()
        };

        // An open editor takes over the whole area below the header.
        if self.editor.is_some() {
            return v_flex()
                .size_full()
                .child(ui::panel_header(&t, "folder", "SFTP", meta))
                .child(self.editor_view(cx));
        }

        let mut body = v_flex().id("sftp-body").flex_1().min_h(px(0.0)).overflow_y_scroll();

        if self.error.is_some() {
            body = body.child(self.error_line());
        }

        if self.sftp.is_some() {
            // Connected: header toolbar, then the optional new-entry input row,
            // the ".." row, and the listing.
            body = body.child(self.toolbar(cx));
            let new_entry = match &self.edit {
                Edit::New { is_dir, name } => Some((*is_dir, name.clone())),
                _ => None,
            };
            if let Some((is_dir, name)) = new_entry {
                body = body.child(self.new_entry_row(cx, is_dir, name));
            }
            if self.bookmarks_open && !self.bookmarks.is_empty() {
                for bm in &self.bookmarks {
                    body = body.child(self.bookmark_row(cx, bm.clone()));
                }
            }
            if parent_of(&self.cwd) != self.cwd {
                body = body.child(self.up_row(cx));
            }
            if self.entries.is_empty() && !self.loading && !matches!(self.edit, Edit::New { .. }) {
                body = body.child(
                    div()
                        .px(t.sp3)
                        .py(t.sp2)
                        .text_size(t.fs_sm)
                        .text_color(t.dim)
                        .child("Empty directory"),
                );
            } else {
                for e in &self.entries {
                    body = body.child(self.entry_row(cx, e));
                }
            }
        } else if self.loading {
            body = body.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child("Connecting…"),
            );
        } else if self.conns.is_empty() {
            return v_flex()
                .size_full()
                .child(ui::panel_header(&t, "folder", "SFTP", meta))
                .child(ui::empty_state(&t, "No saved connections"));
        } else {
            // Disconnected: pick a connection to browse.
            body = body.child(ui::section_label(&t, format!("CONNECTIONS · {}", self.conns.len())));
            for (i, c) in self.conns.iter().enumerate() {
                body = body.child(self.conn_row(cx, i, c));
            }
        }

        v_flex()
            .size_full()
            .child(ui::panel_header(&t, "folder", "SFTP", meta))
            .child(body)
    }
}

/// Parent of a remote path. Root (`/`) and the empty path return themselves so
/// callers can detect "no parent" by `parent_of(p) == p`.
fn parent_of(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return path.to_string();
    }
    match trimmed.rsplit_once('/') {
        Some(("", _)) => "/".to_string(),
        Some((parent, _)) => parent.to_string(),
        None => path.to_string(),
    }
}

/// Join a remote directory and a leaf name into an absolute path, normalizing
/// the single separator (root stays `/leaf`).
fn join_remote(dir: &str, leaf: &str) -> String {
    let base = dir.trim_end_matches('/');
    if base.is_empty() {
        format!("/{leaf}")
    } else {
        format!("{base}/{leaf}")
    }
}

/// Render the low nine permission bits as an `rwxr-xr-x` string.
fn perm_rwx(mode: u32) -> String {
    const F: [char; 3] = ['r', 'w', 'x'];
    (0..9u32)
        .map(|i| if mode & (1 << (8 - i)) != 0 { F[(i % 3) as usize] } else { '-' })
        .collect()
}

/// Compact human-readable byte size, e.g. `4.0 K`, `1.2 M`.
/// Format a Unix-epoch timestamp (seconds) as a short relative age — "now",
/// "5m", "3h", "2d", "1w", "4mo". Blank when the time is missing (0) or sits in
/// the future (clock skew), so the cell simply stays empty.
fn rel_age(epoch: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if epoch == 0 || epoch > now {
        return String::new();
    }
    let secs = now - epoch;
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        86_400..=604_799 => format!("{}d", secs / 86_400),
        604_800..=2_591_999 => format!("{}w", secs / 604_800),
        _ => format!("{}mo", secs / 2_592_000),
    }
}

fn human_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}
