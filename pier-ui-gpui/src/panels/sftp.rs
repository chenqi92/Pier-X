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
//   * Download / Upload — native save / open dialogs feed a transfer queue.
//
// Every mutation runs over the cached SftpClient on the background executor and
// re-lists the current directory on success. Failures surface as one error line.
//
// Downloads and uploads don't block: each picked file is appended to a transfer
// queue pinned below the listing. A single drain worker runs them one at a time
// via the chunked, cancellable SftpClient transfers, writing byte progress into
// per-item atomics that a ~8 fps ticker samples into a live progress bar. Each
// row can be cancelled (queued items drop immediately; a running one aborts
// between chunks) and finished rows can be cleared.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{
    div, px, relative, AnyElement, Context, FocusHandle, FontWeight, Hsla, KeyDownEvent,
    MouseButton, MouseDownEvent, PathPromptOptions, SharedString, Window,
};
use gpui_component::{h_flex, v_flex};
use tokio_util::sync::CancellationToken;

use pier_core::ssh::{RemoteFileEntry, SftpClient, SshConfig, SshSession};

use crate::data;
use crate::i18n;
use crate::theme::Theme;
use crate::ui;

/// Repaint cadence for the transfer queue while a transfer is running — fast
/// enough for a smooth progress bar (~8 fps) without flooding the main thread
/// with re-renders. The bytes themselves are written by the background
/// progress callback into atomics; this timer just samples them.
const XFER_TICK: Duration = Duration::from_millis(120);

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

/// Which way a queued transfer moves bytes.
#[derive(Clone, Copy, PartialEq)]
enum XferDir {
    /// Local → remote.
    Up,
    /// Remote → local.
    Down,
}

/// Lifecycle of one transfer-queue item. `Queued` and the three terminal
/// states are only ever set on the main thread; `Running`'s byte counters live
/// in the [`Transfer`] atomics so the background transfer can update them
/// without touching the View.
enum XferState {
    /// Waiting for the drain worker to pick it up.
    Queued,
    /// Currently streaming bytes (see the `done`/`total` atomics).
    Running,
    /// Finished successfully.
    Done,
    /// Finished with an error (message shown under the row).
    Failed(String),
    /// Cancelled by the user before completing.
    Cancelled,
}

/// One entry in the upload/download queue. Cheap to keep around after it
/// finishes so the user can see the result until they clear it. `done`/`total`
/// are shared with the running background transfer via its progress callback;
/// `cancel` is shared with the in-flight transfer so the row's cancel button
/// can abort it between chunks.
struct Transfer {
    /// Monotonic id, stable for row keying and lookup across re-renders.
    id: u64,
    dir: XferDir,
    /// Leaf name shown on the row.
    name: String,
    /// Absolute remote path.
    remote: String,
    /// Absolute local path.
    local: PathBuf,
    state: XferState,
    /// Bytes transferred so far (written by the background progress callback).
    done: Arc<AtomicU64>,
    /// Total bytes, 0 until the first progress event reports it.
    total: Arc<AtomicU64>,
    /// Fires to abort the transfer mid-flight; shared with the running call.
    cancel: CancellationToken,
}

impl Transfer {
    fn is_queued(&self) -> bool {
        matches!(self.state, XferState::Queued)
    }
    fn is_running(&self) -> bool {
        matches!(self.state, XferState::Running)
    }
    fn is_finished(&self) -> bool {
        matches!(
            self.state,
            XferState::Done | XferState::Failed(_) | XferState::Cancelled
        )
    }
}

/// The data the drain worker needs to run one transfer off the render path —
/// a snapshot of a [`Transfer`]'s fields (the SftpClient, paths, shared
/// progress atomics, and cancel token) so the blocking call owns no borrow of
/// the panel.
struct RunHandle {
    id: u64,
    sftp: SftpClient,
    dir: XferDir,
    remote: String,
    local: PathBuf,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    cancel: CancellationToken,
}

impl RunHandle {
    /// Run the transfer to completion on the calling (background) thread,
    /// folding byte progress into the shared atomics as it goes. Blocks; call
    /// only off the render path.
    fn execute(self) -> Result<(), String> {
        let done = self.done.clone();
        let total = self.total.clone();
        let on_progress = move |bytes: u64, total_bytes: u64| {
            done.store(bytes, Ordering::Relaxed);
            total.store(total_bytes, Ordering::Relaxed);
        };
        let res = match self.dir {
            XferDir::Up => self.sftp.upload_from_with_progress_cancel_blocking(
                &self.local,
                &self.remote,
                on_progress,
                Some(&self.cancel),
            ),
            XferDir::Down => self.sftp.download_to_with_progress_cancel_blocking(
                &self.remote,
                &self.local,
                on_progress,
                Some(&self.cancel),
            ),
        };
        res.map(|_| ()).map_err(|e| e.to_string())
    }
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
    /// Upload/download queue, newest last. Holds queued, running, and finished
    /// items until cleared.
    transfers: Vec<Transfer>,
    /// Next transfer id to hand out.
    next_xfer_id: u64,
    /// Whether a drain worker (and its repaint ticker) is currently alive. Set
    /// true when a transfer is enqueued onto an idle queue; cleared by the
    /// worker when it finds the queue empty. Gates against spawning duplicate
    /// workers — see [`Self::kick_drain`].
    draining: bool,
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
            transfers: Vec::new(),
            next_xfer_id: 0,
            draining: false,
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
                        // Abort and drop any queue from the previous host: a
                        // queued upload's remote path is meaningless against a
                        // new connection. An in-flight transfer (held by the
                        // old worker) sees its token fire and ends as
                        // Cancelled; the worker then finds the queue empty and
                        // winds itself down.
                        for x in &this.transfers {
                            x.cancel.cancel();
                        }
                        this.transfers.clear();
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

    /// Queue a download of a remote file to a local path chosen via the native
    /// save dialog. The bytes move through the drain worker with a live
    /// progress bar in the transfer queue, not inline here.
    fn download(&mut self, remote: String, name: String, cx: &mut Context<Self>) {
        if self.sftp.is_none() {
            return;
        }
        let dir = data::current_dir();
        cx.spawn(async move |this, cx| {
            let recv = cx.update(|cx| cx.prompt_for_new_path(&dir, Some(name.as_str())));
            let Ok(Ok(Some(local))) = recv.await else {
                return; // cancelled or errored
            };
            let _ = this.update(cx, |this, cx| {
                this.push_transfer(XferDir::Down, name, remote, local);
                this.kick_drain(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Queue an upload of one or more locally-chosen files into the current
    /// remote directory. Each picked file becomes its own queue item.
    fn upload(&mut self, cx: &mut Context<Self>) {
        if self.sftp.is_none() {
            return;
        }
        let remote_dir = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let opts = PathPromptOptions {
                files: true,
                directories: false,
                multiple: true,
                prompt: None,
            };
            let recv = cx.update(|cx| cx.prompt_for_paths(opts));
            let Ok(Ok(Some(paths))) = recv.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                for local in paths {
                    let fname = local
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if fname.is_empty() {
                        continue;
                    }
                    let remote = join_remote(&remote_dir, &fname);
                    this.push_transfer(XferDir::Up, fname, remote, local);
                }
                this.kick_drain(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Append a transfer to the queue in the `Queued` state. Does not start
    /// the drain worker — the caller follows with [`Self::kick_drain`] once all
    /// items for this action are pushed.
    fn push_transfer(&mut self, dir: XferDir, name: String, remote: String, local: PathBuf) {
        let id = self.next_xfer_id;
        self.next_xfer_id += 1;
        self.transfers.push(Transfer {
            id,
            dir,
            name,
            remote,
            local,
            state: XferState::Queued,
            done: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(0)),
            cancel: CancellationToken::new(),
        });
    }

    /// Start draining the queue if it isn't already being drained. Spawns two
    /// cooperating tasks: a worker that runs queued transfers one at a time,
    /// and a ticker that repaints the running bar at [`XFER_TICK`]. Both stop
    /// when the queue empties. A no-op while a worker is already alive (it
    /// picks up newly-queued items on its next iteration) or when nothing is
    /// queued.
    fn kick_drain(&mut self, cx: &mut Context<Self>) {
        if self.draining || !self.transfers.iter().any(Transfer::is_queued) {
            return;
        }
        self.draining = true;

        // Repaint ticker — samples the running transfer's byte atomics into a
        // re-render a few times a second. It owns no transfer state; the
        // background callback writes the bytes, this just asks for a frame.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(XFER_TICK).await;
                let alive = this.update(cx, |this, cx| {
                    if this.transfers.iter().any(Transfer::is_running) {
                        cx.notify();
                    }
                    this.draining
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        })
        .detach();

        // Drain worker — runs queued transfers sequentially. Taking the next
        // item and clearing `draining` when none remain happen in one update
        // so an enqueue can never slip between them and strand an item.
        cx.spawn(async move |this, cx| {
            loop {
                let next = this.update(cx, |this, cx| match this.take_next_run() {
                    Some(run) => {
                        cx.notify();
                        Some(run)
                    }
                    None => {
                        this.draining = false;
                        cx.notify();
                        None
                    }
                });
                let run = match next {
                    Ok(Some(run)) => run,
                    _ => break, // queue drained, or the View is gone
                };
                let id = run.id;
                let res = cx
                    .background_executor()
                    .spawn(async move { run.execute() })
                    .await;
                if this
                    .update(cx, |this, cx| this.finish_run(id, res, cx))
                    .is_err()
                {
                    break; // View gone
                }
            }
        })
        .detach();
    }

    /// Flip the first queued transfer to `Running` and snapshot what the worker
    /// needs to run it. Returns `None` when nothing is queued or the session is
    /// gone (so the worker stops draining).
    fn take_next_run(&mut self) -> Option<RunHandle> {
        let sftp = self.sftp.clone()?;
        let t = self.transfers.iter_mut().find(|t| t.is_queued())?;
        t.state = XferState::Running;
        Some(RunHandle {
            id: t.id,
            sftp,
            dir: t.dir,
            remote: t.remote.clone(),
            local: t.local.clone(),
            done: t.done.clone(),
            total: t.total.clone(),
            cancel: t.cancel.clone(),
        })
    }

    /// Fold a finished transfer's result back into its row. A cancelled token
    /// (rather than the error text) is the source of truth for distinguishing
    /// a user cancel from a real failure. A successful upload into the current
    /// directory triggers a quiet re-list so the new file appears.
    fn finish_run(&mut self, id: u64, res: Result<(), String>, cx: &mut Context<Self>) {
        let cwd = self.cwd.clone();
        let mut relist = false;
        if let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) {
            match res {
                Ok(()) => {
                    // Snap the bar to 100% — the last progress event may have
                    // landed a chunk short of total.
                    let total = t.total.load(Ordering::Relaxed);
                    if total > 0 {
                        t.done.store(total, Ordering::Relaxed);
                    }
                    t.state = XferState::Done;
                    if t.dir == XferDir::Up && parent_of(&t.remote) == cwd {
                        relist = true;
                    }
                }
                Err(msg) => {
                    t.state = if t.cancel.is_cancelled() {
                        XferState::Cancelled
                    } else {
                        XferState::Failed(msg)
                    };
                }
            }
        }
        if relist {
            self.refresh_listing(cx);
        }
        cx.notify();
    }

    /// Re-list the current directory without disturbing the transfer queue or
    /// the panel's error line (a transient listing failure shouldn't surface
    /// over a transfer that just succeeded).
    fn refresh_listing(&mut self, cx: &mut Context<Self>) {
        let Some(sftp) = self.sftp.clone() else {
            return;
        };
        let dir = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let listed = cx
                .background_executor()
                .spawn(async move { sftp.list_dir_blocking(&dir).map_err(|e| e.to_string()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(entries) = listed {
                    this.entries = entries;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Cancel a transfer: a queued one flips straight to `Cancelled`; a running
    /// one has its token fired and flips when the worker observes the abort.
    fn cancel_transfer(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) {
            match t.state {
                XferState::Queued => {
                    t.cancel.cancel();
                    t.state = XferState::Cancelled;
                }
                XferState::Running => t.cancel.cancel(),
                _ => {}
            }
        }
        cx.notify();
    }

    /// Drop a single finished transfer from the queue (the row's ✕ once it is
    /// done/failed/cancelled). Never removes a queued or running item.
    fn remove_transfer(&mut self, id: u64, cx: &mut Context<Self>) {
        self.transfers
            .retain(|t| !(t.id == id && t.is_finished()));
        cx.notify();
    }

    /// Drop every finished transfer, keeping queued and running ones.
    fn clear_finished(&mut self, cx: &mut Context<Self>) {
        self.transfers.retain(|t| !t.is_finished());
        cx.notify();
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
                    self.error = Some(i18n::tf("sftp.invalid_octal", &[&mode]));
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
            self.error = Some(i18n::tf("sftp.file_too_large", &[&name]));
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
                                    ed.error = Some(i18n::t("sftp.not_utf8").to_string());
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
        placeholder: SharedString,
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
        let placeholder = if is_dir {
            i18n::t("sftp.new_folder_ph")
        } else {
            i18n::t("sftp.new_file_ph")
        };
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
            nav = nav.child(div().flex_1().min_w(px(0.0)).child(self.inline_input(cx, val, i18n::t("sftp.rename_ph"))));
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
            div().w(px(62.0)).child(self.inline_input(cx, val, i18n::t("sftp.octal_ph"))).into_any_element()
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
                        .child(i18n::t("common.confirm_delete")),
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

    /// The transfer queue, pinned below the listing: a header with the count
    /// and a clear-finished button, over a capped, scrollable list of rows.
    fn transfers_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let any_finished = self.transfers.iter().any(Transfer::is_finished);
        let header = h_flex()
            .items_center()
            .gap(t.sp2)
            .h(px(28.0))
            .px(t.sp3)
            .border_t_1()
            .border_color(t.line)
            .child(ui::icon("inbox", px(13.0), t.muted))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(t.fs_sm)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.muted)
                    .child(i18n::tf("sftp.transfers_count", &[&self.transfers.len().to_string()])),
            )
            .when(any_finished, |d| {
                d.child(self.head_btn(cx, "sftp-xfer-clear", "close", |this, _w, cx| {
                    this.clear_finished(cx);
                }))
            });
        let mut list = v_flex().id("sftp-xfers").max_h(px(168.0)).overflow_y_scroll();
        for x in &self.transfers {
            list = list.child(self.transfer_row(cx, x));
        }
        v_flex().child(header).child(list)
    }

    /// One transfer row: a direction glyph + name on the first line, a
    /// state-dependent trailing cluster (progress %, status glyph, cancel /
    /// remove button), and an optional second line — a progress bar with byte
    /// counts while running, or the error message when failed.
    fn transfer_row(&self, cx: &mut Context<Self>, x: &Transfer) -> impl IntoElement {
        let t = &self.theme;
        let id = x.id;
        let dir_glyph = match x.dir {
            XferDir::Up => "arrow-up",
            XferDir::Down => "arrow-down",
        };

        // Trailing cluster: status hint + cancel (queued/running) or remove
        // (finished) button.
        let trailing = match &x.state {
            XferState::Queued => h_flex()
                .items_center()
                .gap(px(4.0))
                .child(div().text_size(t.fs_sm).text_color(t.dim).child(i18n::t("sftp.xfer_queued")))
                .child(self.row_btn(cx, format!("sftp-xfer-c-{id}"), "close", t.muted, move |this, _w, cx| {
                    this.cancel_transfer(id, cx);
                }))
                .into_any_element(),
            XferState::Running => {
                let done = x.done.load(Ordering::Relaxed);
                let total = x.total.load(Ordering::Relaxed);
                let pct = if total > 0 {
                    ((done as f64 / total as f64) * 100.0).round() as u32
                } else {
                    0
                };
                h_flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        div()
                            .w(px(34.0))
                            .flex()
                            .justify_end()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(format!("{pct}%")),
                    )
                    .child(self.row_btn(cx, format!("sftp-xfer-c-{id}"), "close", t.muted, move |this, _w, cx| {
                        this.cancel_transfer(id, cx);
                    }))
                    .into_any_element()
            }
            XferState::Done => h_flex()
                .items_center()
                .gap(px(4.0))
                .child(ui::icon("check", px(13.0), t.pos))
                .child(self.row_btn(cx, format!("sftp-xfer-r-{id}"), "close", t.muted, move |this, _w, cx| {
                    this.remove_transfer(id, cx);
                }))
                .into_any_element(),
            XferState::Failed(_) => h_flex()
                .items_center()
                .gap(px(4.0))
                .child(ui::icon("triangle-alert", px(13.0), t.neg))
                .child(self.row_btn(cx, format!("sftp-xfer-r-{id}"), "close", t.muted, move |this, _w, cx| {
                    this.remove_transfer(id, cx);
                }))
                .into_any_element(),
            XferState::Cancelled => h_flex()
                .items_center()
                .gap(px(4.0))
                .child(div().text_size(t.fs_sm).text_color(t.dim).child(i18n::t("sftp.xfer_cancelled")))
                .child(self.row_btn(cx, format!("sftp-xfer-r-{id}"), "close", t.muted, move |this, _w, cx| {
                    this.remove_transfer(id, cx);
                }))
                .into_any_element(),
        };

        let line1 = h_flex()
            .items_center()
            .gap(t.sp2)
            .child(ui::icon(dir_glyph, px(13.0), t.muted))
            .child(div().flex_1().min_w(px(0.0)).overflow_hidden().child(x.name.clone()))
            .child(trailing);

        // Optional second line: a progress bar + byte counts while running, or
        // the error string when failed.
        let detail: Option<AnyElement> = match &x.state {
            XferState::Running => {
                let done = x.done.load(Ordering::Relaxed);
                let total = x.total.load(Ordering::Relaxed);
                let frac = if total > 0 {
                    (done as f64 / total as f64) as f32
                } else {
                    0.0
                };
                Some(
                    h_flex()
                        .items_center()
                        .gap(t.sp2)
                        .child(
                            div()
                                .flex_1()
                                .h(px(4.0))
                                .rounded(px(2.0))
                                .bg(t.panel_2)
                                .child(div().h_full().w(relative(frac)).rounded(px(2.0)).bg(t.accent)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .font_family(t.mono.clone())
                                .text_size(t.fs_sm)
                                .text_color(t.muted)
                                .child(format!("{} / {}", human_size(done), human_size(total))),
                        )
                        .into_any_element(),
                )
            }
            XferState::Failed(msg) => Some(
                div()
                    .overflow_hidden()
                    .text_size(t.fs_sm)
                    .text_color(t.neg)
                    .child(msg.clone())
                    .into_any_element(),
            ),
            _ => None,
        };

        let mut row = v_flex()
            .id(SharedString::from(format!("sftp-xfer-{id}")))
            .gap(px(3.0))
            .px(t.sp3)
            .py(px(5.0))
            .text_color(t.ink_2)
            .hover(|s| s.bg(t.hover))
            .child(line1);
        if let Some(detail) = detail {
            row = row.child(detail);
        }
        row
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
            header = header.child(div().text_size(t.fs_sm).text_color(t.muted).child(i18n::t("sftp.saving")));
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
                    .child(i18n::t("sftp.unsaved_changes")),
            );
        }

        // The buffer, rendered line by line. `split('\n')` keeps a trailing
        // blank line visible (so Enter at the end gives feedback); an empty
        // line renders a space so it keeps its height.
        let mut text = v_flex().w_full();
        if ed.loading {
            text = text.child(div().text_color(t.dim).child(i18n::t("common.loading")));
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
                .child(ui::panel_header(&t, "folder", i18n::t("tool.sftp"), meta))
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
                        .child(i18n::t("sftp.empty_dir")),
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
                    .child(i18n::t("panel.connecting")),
            );
        } else if self.conns.is_empty() {
            return v_flex()
                .size_full()
                .child(ui::panel_header(&t, "folder", i18n::t("tool.sftp"), meta))
                .child(ui::empty_state(&t, i18n::t("side.no_saved_connections")));
        } else {
            // Disconnected: pick a connection to browse.
            body = body.child(ui::section_label(&t, i18n::tf("sftp.connections_count", &[&self.conns.len().to_string()])));
            for (i, c) in self.conns.iter().enumerate() {
                body = body.child(self.conn_row(cx, i, c));
            }
        }

        let mut root = v_flex()
            .size_full()
            .child(ui::panel_header(&t, "folder", i18n::t("tool.sftp"), meta))
            .child(body);
        // The transfer queue is pinned below the listing whenever it has items.
        if !self.transfers.is_empty() {
            root = root.child(self.transfers_view(cx));
        }
        root
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
