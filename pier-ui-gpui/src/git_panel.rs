// Pier-X GPUI spike — Git panel view.
//
// Extracted from shell.rs into an independent View, modelled on
// settings.rs's `SettingsView`. It owns the working-tree status, the
// sub-tab state, the commit-message buffer, and the inline diff/blame
// viewer. The shell points it at a directory through `set_cwd` (driven by
// its Files navigation) and reads back a small `(branch, ahead, behind)`
// summary via `status_summary` for the status bar — the shell no longer
// holds any git state itself.
//
// Sub-tabs mirror the web Git panel (PRODUCT-SPEC §5.2): Changes / History
// / Branches / Stash plus Tags / Remotes / Config / Submodules. All git
// reads and mutations go through `crate::data` wrappers; blocking/network
// work (commit, checkout, push/pull/fetch/rebase, submodule ops, diff,
// blame) runs on the background executor and writes back with `cx.notify()`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Context, Div, FocusHandle, Hsla, KeyDownEvent, MouseButton, MouseDownEvent,
    SharedString, Stateful, Window,
};
use gpui_component::{h_flex, v_flex};

use pier_core::services::git::FileStatus;

use crate::data::{self, GitData};
use crate::theme::Theme;
use crate::ui;

/// Cap on how many diff/blame lines the inline viewer paints, so a huge
/// file can't build an unbounded element tree.
const MAX_VIEWER_LINES: usize = 5000;

/// Sub-views inside the Git panel.
#[derive(Clone, Copy, PartialEq)]
pub enum GitTab {
    Changes,
    History,
    Branches,
    Stash,
    Tags,
    Remotes,
    Config,
    Submodules,
}

/// A per-file staging action.
#[derive(Clone, Copy)]
enum GitFileOp {
    Stage,
    Unstage,
    Discard,
}

/// A remote/branch git action dispatched off the render path.
#[derive(Clone, Copy)]
enum GitRemoteOp {
    Push,
    Pull,
    Fetch,
    Rebase,
}

impl GitRemoteOp {
    /// Transient "in progress" line shown while the op runs.
    fn pending(self) -> &'static str {
        match self {
            GitRemoteOp::Push => "Pushing…",
            GitRemoteOp::Pull => "Pulling…",
            GitRemoteOp::Fetch => "Fetching…",
            GitRemoteOp::Rebase => "Rebasing…",
        }
    }
}

/// Which inline create composer is open (Tags / Remotes / Config). The
/// field buffers live in `in_a` / `in_b` / `in_global` on the view.
#[derive(Clone, Copy, PartialEq)]
enum Composer {
    None,
    Tag,
    Remote,
    Config,
}

/// The inline diff / blame sub-view for one file. `staged` / `untracked`
/// are kept so toggling between Diff and Blame re-opens the matching diff.
struct Viewer {
    path: String,
    is_blame: bool,
    staged: bool,
    untracked: bool,
    /// Loaded diff text; `None` while loading or on error.
    diff: Option<String>,
    blame: Vec<data::BlameLine>,
    loading: bool,
    error: Option<String>,
}

pub struct GitPanelView {
    theme: Theme,
    cwd: PathBuf,
    git: Option<GitData>,
    /// Per-file `+adds -dels` keyed by repo-relative path.
    git_numstat: HashMap<String, (u32, u32)>,
    git_tab: GitTab,
    git_history: Vec<data::CommitInfo>,
    git_branch_list: Vec<String>,
    git_stashes: Vec<data::StashEntry>,
    git_tags: Vec<data::TagInfo>,
    git_remotes: Vec<data::RemoteInfo>,
    git_config: Vec<data::ConfigEntry>,
    git_submodules: Vec<data::SubmoduleInfo>,
    /// Transient result line (commit hash / push output / errors).
    git_msg: Option<String>,
    commit_msg: String,
    commit_focus: FocusHandle,
    /// Inline create composer + its field buffers and focus.
    composer: Composer,
    composer_field: usize,
    composer_focus: FocusHandle,
    in_a: String,
    in_b: String,
    in_global: bool,
    /// Inline diff/blame viewer + a generation guard so a slow load for an
    /// earlier file can't overwrite a newer selection.
    viewer: Option<Viewer>,
    viewer_epoch: u64,
}

impl GitPanelView {
    pub fn new(cwd: PathBuf, cx: &mut Context<Self>) -> Self {
        let git = data::git_status(&cwd);
        let git_numstat = data::git_numstat(&cwd);
        Self {
            theme: cx.global::<Theme>().clone(),
            cwd,
            git,
            git_numstat,
            git_tab: GitTab::Changes,
            git_history: Vec::new(),
            git_branch_list: Vec::new(),
            git_stashes: Vec::new(),
            git_tags: Vec::new(),
            git_remotes: Vec::new(),
            git_config: Vec::new(),
            git_submodules: Vec::new(),
            git_msg: None,
            commit_msg: String::new(),
            commit_focus: cx.focus_handle(),
            composer: Composer::None,
            composer_field: 0,
            composer_focus: cx.focus_handle(),
            in_a: String::new(),
            in_b: String::new(),
            in_global: false,
            viewer: None,
            viewer_epoch: 0,
        }
    }

    /// Point the panel at a new working directory (pushed by the shell when
    /// its Files navigation moves). A no-op when unchanged, so repeat calls
    /// from `open_file` stay cheap.
    pub fn set_cwd(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.cwd == path {
            return;
        }
        self.cwd = path;
        // The old repo's diff/blame and half-typed create form no longer apply.
        self.viewer = None;
        self.close_composer();
        self.reload_git_async(cx);
    }

    /// Branch + ahead/behind for the shell's status bar; `None` when the cwd
    /// isn't a git repo.
    pub fn status_summary(&self) -> Option<(String, i32, i32)> {
        self.git
            .as_ref()
            .map(|g| (g.branch.clone(), g.ahead, g.behind))
    }

    /// Reload working-tree status + per-file line counts off the render path.
    /// A reload whose captured cwd no longer matches the panel's is dropped,
    /// so switching folders quickly can't clobber the new repo's status with
    /// a slower in-flight reload of the old one.
    fn reload_git_async(&mut self, cx: &mut Context<Self>) {
        let cwd = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let probe = cwd.clone();
            let (git, numstat) = cx
                .background_executor()
                .spawn(async move { (data::git_status(&probe), data::git_numstat(&probe)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.cwd != cwd {
                    return;
                }
                this.git = git;
                this.git_numstat = numstat;
                cx.notify();
            });
        })
        .detach();
    }

    /// Synchronously (re)load the active sub-tab's list. Local git reads are
    /// fast; this only ever runs in click handlers, never in render.
    fn load_current_tab(&mut self) {
        match self.git_tab {
            GitTab::History => self.git_history = data::git_log(&self.cwd, 50),
            GitTab::Branches => self.git_branch_list = data::git_branches(&self.cwd),
            GitTab::Stash => self.git_stashes = data::git_stash(&self.cwd),
            GitTab::Tags => self.git_tags = data::git_tags(&self.cwd),
            GitTab::Remotes => self.git_remotes = data::git_remotes(&self.cwd),
            GitTab::Config => self.git_config = data::git_config_list(&self.cwd),
            GitTab::Submodules => self.git_submodules = data::git_submodules(&self.cwd),
            GitTab::Changes => {}
        }
    }

    /// Switch the Git sub-tab, loading its data on demand.
    fn set_git_tab(&mut self, tab: GitTab, cx: &mut Context<Self>) {
        self.git_tab = tab;
        self.close_composer();
        self.load_current_tab();
        cx.notify();
    }

    /// Run a per-file staging action, then refresh status.
    fn git_file_op(&mut self, op: GitFileOp, file: String, cx: &mut Context<Self>) {
        let res = match op {
            GitFileOp::Stage => data::git_stage(&self.cwd, &file),
            GitFileOp::Unstage => data::git_unstage(&self.cwd, &file),
            GitFileOp::Discard => data::git_discard(&self.cwd, &file),
        };
        self.git_msg = res.err();
        self.reload_git_async(cx);
        cx.notify();
    }

    /// Commit the staged changes with the current message off the render path
    /// (a commit can run pre-commit hooks), then refresh status.
    fn do_commit(&mut self, cx: &mut Context<Self>) {
        let msg = self.commit_msg.trim().to_string();
        if msg.is_empty() {
            self.git_msg = Some("Enter a commit message".to_string());
            cx.notify();
            return;
        }
        self.git_msg = Some("Committing…".to_string());
        cx.notify();
        let cwd = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { data::git_commit(&cwd, &msg) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(hash) => {
                        this.commit_msg.clear();
                        let short: String = hash.chars().take(7).collect();
                        this.git_msg = Some(format!("Committed {short}"));
                    }
                    Err(e) => this.git_msg = Some(e),
                }
                this.reload_git_async(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn on_commit_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        match ks.key.as_str() {
            // Enter commits. The message box is single-line, so there is no
            // newline insertion — any Enter, modified or not, submits.
            "enter" => {
                self.do_commit(cx);
                return;
            }
            "backspace" => {
                if self.commit_msg.pop().is_some() {
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        if m.control || m.alt || m.platform {
            return;
        }
        if let Some(kc) = &ks.key_char {
            if !kc.is_empty() && !kc.chars().any(|c| c.is_control()) {
                self.commit_msg.push_str(kc);
                cx.notify();
            }
        }
    }

    /// Run a remote git op (push/pull/fetch/rebase) off the render path and
    /// surface the result line.
    fn git_remote_op(&mut self, op: GitRemoteOp, cx: &mut Context<Self>) {
        self.git_msg = Some(op.pending().to_string());
        cx.notify();
        let cwd = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    match op {
                        GitRemoteOp::Push => data::git_push(&cwd),
                        GitRemoteOp::Pull => data::git_pull(&cwd),
                        GitRemoteOp::Fetch => data::git_fetch(&cwd),
                        GitRemoteOp::Rebase => data::git_rebase(&cwd),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_msg = Some(summarize(res));
                this.reload_git_async(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Switch the working tree to `branch` off the render path (a checkout can
    /// touch many files) and reload status + the branch list. The write-back
    /// is cwd-guarded so a folder change mid-checkout can't write a stale
    /// branch list into the new repo.
    fn checkout_branch(&mut self, branch: String, cx: &mut Context<Self>) {
        self.git_msg = Some(format!("Switching to {branch}…"));
        cx.notify();
        let cwd = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let probe = cwd.clone();
            let (res, branches) = cx
                .background_executor()
                .spawn(async move {
                    let res = data::git_checkout(&probe, &branch);
                    (res.map(|_| branch), data::git_branches(&probe))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.cwd != cwd {
                    return;
                }
                this.git_msg = Some(match res {
                    Ok(branch) => format!("Switched to {branch}"),
                    Err(e) => e,
                });
                this.git_branch_list = branches;
                this.reload_git_async(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Run a network/slow git op (submodule init/update/sync) off the render
    /// path, surface its result, then refresh status + the active tab list.
    fn run_async(
        &mut self,
        pending: &str,
        op: fn(&Path) -> Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        self.git_msg = Some(pending.to_string());
        cx.notify();
        let cwd = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { op(&cwd) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_msg = Some(summarize(res));
                this.reload_git_async(cx);
                this.load_current_tab();
                cx.notify();
            });
        })
        .detach();
    }

    /// A fast local mutation (tag/remote/config create/delete) whose error, if
    /// any, becomes the status line; then reload the active tab list.
    fn local_mutation(&mut self, res: Result<String, String>, cx: &mut Context<Self>) {
        self.git_msg = res.err();
        self.load_current_tab();
        cx.notify();
    }

    // ── Inline create composer (tags / remotes / config) ─────────────

    fn open_composer(&mut self, kind: Composer, window: &mut Window, cx: &mut Context<Self>) {
        self.composer = kind;
        self.composer_field = 0;
        self.in_a.clear();
        self.in_b.clear();
        self.in_global = false;
        window.focus(&self.composer_focus, cx);
        cx.notify();
    }

    fn close_composer(&mut self) {
        self.composer = Composer::None;
        self.composer_field = 0;
        self.in_a.clear();
        self.in_b.clear();
        self.in_global = false;
    }

    fn submit_composer(&mut self, cx: &mut Context<Self>) {
        let a = self.in_a.trim().to_string();
        let b = self.in_b.trim().to_string();
        let res = match self.composer {
            Composer::Tag => {
                if a.is_empty() {
                    self.git_msg = Some("Tag name required".to_string());
                    cx.notify();
                    return;
                }
                data::git_tag_create(&self.cwd, &a, &b)
            }
            Composer::Remote => {
                if a.is_empty() || b.is_empty() {
                    self.git_msg = Some("Remote name and URL required".to_string());
                    cx.notify();
                    return;
                }
                data::git_remote_add(&self.cwd, &a, &b)
            }
            Composer::Config => {
                if a.is_empty() {
                    self.git_msg = Some("Config key required".to_string());
                    cx.notify();
                    return;
                }
                data::git_config_set(&self.cwd, &a, &b, self.in_global)
            }
            Composer::None => return,
        };
        match res {
            Ok(_) => {
                self.git_msg = None;
                self.close_composer();
                self.load_current_tab();
            }
            Err(e) => self.git_msg = Some(e),
        }
        cx.notify();
    }

    fn on_composer_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "tab" => {
                self.composer_field ^= 1;
                cx.notify();
                return;
            }
            "enter" => {
                self.submit_composer(cx);
                return;
            }
            "escape" => {
                self.close_composer();
                cx.notify();
                return;
            }
            "backspace" => {
                let buf = if self.composer_field == 0 {
                    &mut self.in_a
                } else {
                    &mut self.in_b
                };
                if buf.pop().is_some() {
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
            if !kc.is_empty() && !kc.chars().any(|c| c.is_control()) {
                if self.composer_field == 0 {
                    self.in_a.push_str(kc);
                } else {
                    self.in_b.push_str(kc);
                }
                cx.notify();
            }
        }
    }

    // ── Inline diff / blame viewer ───────────────────────────────────

    fn open_diff(&mut self, path: String, staged: bool, untracked: bool, cx: &mut Context<Self>) {
        self.viewer_epoch += 1;
        let gen = self.viewer_epoch;
        self.viewer = Some(Viewer {
            path: path.clone(),
            is_blame: false,
            staged,
            untracked,
            diff: None,
            blame: Vec::new(),
            loading: true,
            error: None,
        });
        cx.notify();
        let cwd = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    if untracked {
                        data::git_diff_untracked(&cwd, &path)
                    } else {
                        data::git_diff(&cwd, &path, staged)
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.viewer_epoch != gen {
                    return;
                }
                if let Some(v) = &mut this.viewer {
                    v.loading = false;
                    match res {
                        Ok(t) => v.diff = Some(t),
                        Err(e) => v.error = Some(e),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_blame(&mut self, path: String, staged: bool, untracked: bool, cx: &mut Context<Self>) {
        self.viewer_epoch += 1;
        let gen = self.viewer_epoch;
        self.viewer = Some(Viewer {
            path: path.clone(),
            is_blame: true,
            staged,
            untracked,
            diff: None,
            blame: Vec::new(),
            loading: true,
            error: None,
        });
        cx.notify();
        let cwd = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { data::git_blame(&cwd, &path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.viewer_epoch != gen {
                    return;
                }
                if let Some(v) = &mut this.viewer {
                    v.loading = false;
                    match res {
                        Ok(lines) => v.blame = lines,
                        Err(e) => v.error = Some(e),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn close_viewer(&mut self) {
        // Bump the epoch so an in-flight load can't repopulate the viewer.
        self.viewer_epoch += 1;
        self.viewer = None;
    }

    // ── Small shared button ──────────────────────────────────────────

    /// A compact pill button. The caller wires the click via `.on_mouse_down`.
    fn pill(
        &self,
        key: impl Into<SharedString>,
        label: impl Into<SharedString>,
        primary: bool,
    ) -> Stateful<Div> {
        let t = &self.theme;
        div()
            .id(key.into())
            .px(t.sp3)
            .py(px(4.0))
            .rounded(t.radius_sm)
            .text_size(t.fs_ui)
            .cursor_pointer()
            .when(primary, |d| d.bg(t.accent).text_color(t.accent_ink))
            .when(!primary, |d| d.bg(t.panel_2).text_color(t.ink_2))
            .child(label.into())
    }

    /// A small square icon button running a fast per-row mutation.
    fn row_icon_btn(
        &self,
        cx: &mut Context<Self>,
        key: String,
        glyph: &'static str,
        color: Hsla,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(SharedString::from(key))
            .flex()
            .items_center()
            .justify_center()
            .w(px(18.0))
            .h(px(18.0))
            .rounded(t.radius_sm)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| on_click(this, cx)),
            )
            .child(ui::icon(glyph, px(13.0), color))
    }

    // ── Render: change rows + commit box ─────────────────────────────

    /// A small icon button performing a per-file git op.
    fn git_file_btn(
        &self,
        cx: &mut Context<Self>,
        key: &str,
        glyph: &'static str,
        color: Hsla,
        op: GitFileOp,
        file: String,
    ) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(SharedString::from(format!("gfb-{key}")))
            .flex()
            .items_center()
            .justify_center()
            .w(px(18.0))
            .h(px(18.0))
            .rounded(t.radius_sm)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.git_file_op(op, file.clone(), cx)
                }),
            )
            .child(ui::icon(glyph, px(13.0), color))
    }

    fn git_change_row(
        &self,
        cx: &mut Context<Self>,
        c: &data::GitChange,
        staged: bool,
    ) -> impl IntoElement {
        let t = &self.theme;
        let (mark, mark_color) = status_style(t, &c.status);
        let path = c.path.clone();
        let untracked = matches!(c.status, FileStatus::Untracked);
        let numstat = self
            .git_numstat
            .get(&c.path)
            .copied()
            .filter(|(add, del)| *add > 0 || *del > 0);
        h_flex()
            .id(SharedString::from(format!("gch-{}-{}", staged, c.path)))
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .border_l_2()
            .border_color(mark_color)
            .hover(|s| s.bg(t.hover))
            .child(
                div()
                    .w(px(14.0))
                    .font_family(t.mono.clone())
                    .text_color(mark_color)
                    .child(mark),
            )
            .child(
                // Clicking the filename opens the inline diff for this file.
                div()
                    .id(SharedString::from(format!("gdiff-{}-{}", staged, c.path)))
                    .flex_1()
                    .overflow_hidden()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink_2)
                    .cursor_pointer()
                    .hover(|s| s.text_color(t.ink))
                    .on_mouse_down(MouseButton::Left, {
                        let path = path.clone();
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.open_diff(path.clone(), staged, untracked, cx)
                        })
                    })
                    .child(c.path.clone()),
            )
            .when_some(numstat, |d, (add, del)| {
                d.child(
                    h_flex()
                        .flex_none()
                        .gap(px(4.0))
                        .font_family(t.mono.clone())
                        .text_size(t.fs_sm)
                        .when(add > 0, |d| {
                            d.child(div().text_color(t.pos).child(format!("+{add}")))
                        })
                        .when(del > 0, |d| {
                            d.child(div().text_color(t.neg).child(format!("-{del}")))
                        }),
                )
            })
            .when(staged, |d| {
                d.child(self.git_file_btn(
                    cx,
                    &format!("uns-{}", c.path),
                    "minus",
                    t.muted,
                    GitFileOp::Unstage,
                    path.clone(),
                ))
            })
            .when(!staged, |d| {
                d.child(self.git_file_btn(
                    cx,
                    &format!("dis-{}", c.path),
                    "delete",
                    t.neg,
                    GitFileOp::Discard,
                    path.clone(),
                ))
                .child(self.git_file_btn(
                    cx,
                    &format!("stg-{}", c.path),
                    "plus",
                    t.pos,
                    GitFileOp::Stage,
                    path.clone(),
                ))
            })
    }

    /// Commit message input + Commit button (shown above staged files).
    fn commit_box(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let empty = self.commit_msg.is_empty();
        h_flex()
            .items_center()
            .gap(t.sp2)
            .mx(t.sp3)
            .mb(t.sp2)
            .child(
                div()
                    .track_focus(&self.commit_focus)
                    .key_context("CommitMsg")
                    .on_key_down(cx.listener(Self::on_commit_key))
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(28.0))
                    .px(t.sp2)
                    .flex()
                    .items_center()
                    .rounded(t.radius_sm)
                    .bg(t.panel_2)
                    .border_1()
                    .border_color(t.line_2)
                    .when(empty, |d| d.text_color(t.dim).child("Commit message…"))
                    .when(!empty, |d| {
                        d.text_color(t.ink).child(self.commit_msg.clone())
                    }),
            )
            .child(
                div()
                    .id("git-commit")
                    .px(t.sp3)
                    .py(px(5.0))
                    .rounded(t.radius_sm)
                    .text_size(t.fs_ui)
                    .cursor_pointer()
                    .bg(t.accent)
                    .text_color(t.accent_ink)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| this.do_commit(cx)),
                    )
                    .child("Commit"),
            )
    }

    fn git_chip(
        &self,
        cx: &mut Context<Self>,
        label: &'static str,
        count: Option<usize>,
        tab: GitTab,
    ) -> impl IntoElement {
        let t = &self.theme;
        let active = self.git_tab == tab;
        h_flex()
            .id(SharedString::from(format!("gtab-{label}")))
            .items_center()
            .gap(px(4.0))
            .px(t.sp2)
            .py(px(2.0))
            .text_size(t.fs_ui)
            .cursor_pointer()
            .text_color(if active { t.ink } else { t.muted })
            .when(active, |d| d.border_b_2().border_color(t.accent))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| this.set_git_tab(tab, cx)),
            )
            .child(label)
            .when_some(count, |d, n| {
                d.child(
                    div()
                        .flex_none()
                        .min_w(px(16.0))
                        .px(px(5.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(t.accent_dim)
                        .text_size(t.fs_sm)
                        .text_color(t.accent)
                        .child(n.to_string()),
                )
            })
    }

    /// `Some(op)` makes the button run that remote op; `None` renders it
    /// inert/dim.
    fn git_btn(
        &self,
        cx: &mut Context<Self>,
        label: &'static str,
        primary: bool,
        op: Option<GitRemoteOp>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let mut d = div()
            .id(SharedString::from(format!("gbtn-{label}")))
            .px(t.sp3)
            .py(px(4.0))
            .rounded(t.radius_sm)
            .text_size(t.fs_ui)
            .when(primary, |d| d.bg(t.accent).text_color(t.accent_ink))
            .when(!primary, |d| d.bg(t.panel_2).text_color(t.ink_2))
            .child(label);
        match op {
            Some(op) => {
                d = d.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _w, cx| this.git_remote_op(op, cx)),
                );
            }
            None => d = d.text_color(t.dim),
        }
        d
    }

    fn git_commit_row(&self, c: &data::CommitInfo) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .gap(px(2.0))
            .px(t.sp3)
            .py(t.sp2)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .child(
                        div()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.accent)
                            .child(c.short_hash.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_color(t.ink_2)
                            .child(c.message.clone()),
                    ),
            )
            .child(
                h_flex()
                    .gap(t.sp2)
                    .child(
                        div()
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(c.author.clone()),
                    )
                    .child(
                        div()
                            .text_size(t.fs_sm)
                            .text_color(t.dim)
                            .child(c.relative_date.clone()),
                    ),
            )
    }

    fn git_branch_row(&self, cx: &mut Context<Self>, name: &str, current: bool) -> impl IntoElement {
        let t = &self.theme;
        let branch = name.to_string();
        h_flex()
            .id(SharedString::from(format!("gbr-{name}")))
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .when(!current, |d| {
                let branch = branch.clone();
                d.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                        this.checkout_branch(branch.clone(), cx)
                    }),
                )
            })
            .child(ui::icon(
                "git-branch",
                px(13.0),
                if current { t.accent } else { t.muted },
            ))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .font_family(t.mono.clone())
                    .text_color(if current { t.ink } else { t.ink_2 })
                    .child(name.to_string()),
            )
            .when(current, |d| {
                d.child(div().text_size(t.fs_sm).text_color(t.accent).child("current"))
            })
            .when(!current, |d| {
                d.child(ui::icon("chevron-right", px(13.0), t.dim))
            })
    }

    fn git_stash_row(&self, s: &data::StashEntry) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .gap(px(2.0))
            .px(t.sp3)
            .py(t.sp2)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .child(
                        div()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.accent)
                            .child(s.index.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_color(t.ink_2)
                            .child(s.message.clone()),
                    ),
            )
            .child(
                div()
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child(s.relative_date.clone()),
            )
    }

    // ── Render: tags / remotes / config / submodules ─────────────────

    /// A section label with a `+` button that opens the create composer.
    fn add_header(
        &self,
        cx: &mut Context<Self>,
        label: &'static str,
        count: usize,
        kind: Composer,
    ) -> impl IntoElement {
        let t = &self.theme;
        h_flex()
            .items_center()
            .justify_between()
            .w_full()
            .child(ui::section_label(t, format!("{label} · {count}")))
            .child(
                div()
                    .id(SharedString::from(format!("git-add-{label}")))
                    .mr(t.sp3)
                    .mt(t.sp3)
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(20.0))
                    .h(px(20.0))
                    .rounded(t.radius_sm)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            this.open_composer(kind, window, cx)
                        }),
                    )
                    .child(ui::icon("plus", px(14.0), t.accent)),
            )
    }

    /// One create-composer text field; the active field is accent-bordered.
    fn composer_field_view(
        &self,
        cx: &mut Context<Self>,
        idx: usize,
        placeholder: &str,
        value: &str,
        active: bool,
    ) -> impl IntoElement {
        let t = &self.theme;
        let empty = value.is_empty();
        div()
            .id(SharedString::from(format!("git-comp-f{idx}")))
            .flex_1()
            .min_w(px(0.0))
            .h(px(26.0))
            .px(t.sp2)
            .flex()
            .items_center()
            .rounded(t.radius_sm)
            .bg(t.panel_2)
            .border_1()
            .border_color(if active { t.accent } else { t.line_2 })
            .font_family(t.mono.clone())
            .text_size(t.fs_sm)
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.composer_field = idx;
                    cx.notify();
                }),
            )
            .when(empty, |d| d.text_color(t.dim).child(placeholder.to_string()))
            .when(!empty, |d| d.text_color(t.ink).child(value.to_string()))
    }

    fn composer_box(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let (ph_a, ph_b) = match self.composer {
            Composer::Tag => ("Tag name", "Message (optional)"),
            Composer::Remote => ("Remote name", "URL"),
            Composer::Config => ("key (e.g. user.name)", "value"),
            Composer::None => ("", ""),
        };
        let is_config = self.composer == Composer::Config;
        let global = self.in_global;

        let mut actions = h_flex().gap(t.sp2).items_center();
        if is_config {
            actions = actions.child(
                div()
                    .id("git-comp-scope")
                    .px(t.sp2)
                    .py(px(3.0))
                    .rounded(t.radius_sm)
                    .text_size(t.fs_sm)
                    .cursor_pointer()
                    .bg(t.panel_2)
                    .border_1()
                    .border_color(t.line_2)
                    .text_color(if global { t.warn } else { t.muted })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                            this.in_global = !this.in_global;
                            cx.notify();
                        }),
                    )
                    .child(if global { "global" } else { "local" }),
            );
        }
        actions = actions
            .child(div().flex_1())
            .child(self.pill("git-comp-ok", "Add", true).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| this.submit_composer(cx)),
            ))
            .child(self.pill("git-comp-cancel", "Cancel", false).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                    this.close_composer();
                    cx.notify();
                }),
            ));

        v_flex()
            .mx(t.sp3)
            .mb(t.sp2)
            .gap(t.sp2)
            .track_focus(&self.composer_focus)
            .key_context("GitComposer")
            .on_key_down(cx.listener(Self::on_composer_key))
            .child(
                h_flex()
                    .gap(t.sp2)
                    .child(self.composer_field_view(cx, 0, ph_a, &self.in_a, self.composer_field == 0))
                    .child(self.composer_field_view(cx, 1, ph_b, &self.in_b, self.composer_field == 1)),
            )
            .child(actions)
    }

    fn tag_row(&self, cx: &mut Context<Self>, tg: &data::TagInfo) -> impl IntoElement {
        let t = &self.theme;
        let name = tg.name.clone();
        h_flex()
            .id(SharedString::from(format!("gtag-{}", tg.name)))
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .child(ui::icon("git-commit-horizontal", px(13.0), t.muted))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .truncate()
                    .text_color(t.ink_2)
                    .child(tg.name.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.accent)
                    .child(tg.hash.clone()),
            )
            .child(self.row_icon_btn(
                cx,
                format!("gtag-del-{}", tg.name),
                "delete",
                t.neg,
                move |this, cx| {
                    let res = data::git_tag_delete(&this.cwd, &name);
                    this.local_mutation(res, cx);
                },
            ))
    }

    fn remote_row(&self, cx: &mut Context<Self>, r: &data::RemoteInfo) -> impl IntoElement {
        let t = &self.theme;
        let name = r.name.clone();
        h_flex()
            .id(SharedString::from(format!("grem-{}", r.name)))
            .items_center()
            .gap(t.sp2)
            .h(px(28.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .child(ui::icon("network", px(13.0), t.muted))
            .child(
                div()
                    .flex_none()
                    .w(px(72.0))
                    .overflow_hidden()
                    .truncate()
                    .text_color(t.ink_2)
                    .child(r.name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .truncate()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(r.fetch_url.clone()),
            )
            .child(self.row_icon_btn(
                cx,
                format!("grem-del-{}", r.name),
                "delete",
                t.neg,
                move |this, cx| {
                    let res = data::git_remote_remove(&this.cwd, &name);
                    this.local_mutation(res, cx);
                },
            ))
    }

    fn config_row(&self, cx: &mut Context<Self>, e: &data::ConfigEntry) -> impl IntoElement {
        let t = &self.theme;
        let key = e.key.clone();
        let global = e.scope == "global";
        h_flex()
            .id(SharedString::from(format!("gcfg-{}-{}", e.scope, e.key)))
            .items_center()
            .gap(t.sp2)
            .h(px(28.0))
            .px(t.sp3)
            .hover(|s| s.bg(t.hover))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .overflow_hidden()
                            .truncate()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.ink_2)
                            .child(e.key.clone()),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .truncate()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(e.value.clone()),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(t.fs_sm)
                    .text_color(if global { t.warn } else { t.dim })
                    .child(e.scope.clone()),
            )
            .child(self.row_icon_btn(
                cx,
                format!("gcfg-del-{}-{}", e.scope, e.key),
                "delete",
                t.neg,
                move |this, cx| {
                    let res = data::git_config_unset(&this.cwd, &key, global);
                    this.local_mutation(res, cx);
                },
            ))
    }

    fn submodule_row(&self, s: &data::SubmoduleInfo) -> impl IntoElement {
        let t = &self.theme;
        let color = match s.status.as_str() {
            "uninitialized" => t.dim,
            "modified" => t.warn,
            "conflict" => t.neg,
            _ => t.pos,
        };
        h_flex()
            .items_center()
            .gap(t.sp2)
            .h(px(28.0))
            .px(t.sp3)
            .border_l_2()
            .border_color(color)
            .child(ui::icon("layers", px(13.0), t.muted))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .truncate()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.ink_2)
                    .child(s.path.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.accent)
                    .child(s.short_hash.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(t.fs_sm)
                    .text_color(color)
                    .child(s.status.clone()),
            )
    }

    /// SUBMODULES section label + Init / Update / Sync action buttons.
    fn submodule_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .child(ui::section_label(
                t,
                format!("SUBMODULES · {}", self.git_submodules.len()),
            ))
            .child(
                h_flex()
                    .gap(t.sp2)
                    .px(t.sp3)
                    .pb(t.sp2)
                    .child(self.pill("git-sub-init", "Init", false).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                            this.run_async("Initializing submodules…", data::git_submodule_init, cx)
                        }),
                    ))
                    .child(self.pill("git-sub-update", "Update", true).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                            this.run_async("Updating submodules…", data::git_submodule_update, cx)
                        }),
                    ))
                    .child(self.pill("git-sub-sync", "Sync", false).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                            this.run_async("Syncing submodules…", data::git_submodule_sync, cx)
                        }),
                    )),
            )
    }

    // ── Render: inline diff / blame viewer ───────────────────────────

    fn diff_body(&self, v: &Viewer) -> impl IntoElement {
        let t = &self.theme;
        let text = v.diff.as_deref().unwrap_or("");
        let total = text.lines().count();
        let mut col = v_flex()
            .p(t.sp3)
            .font_family(t.mono.clone())
            .text_size(t.fs_sm);
        if total == 0 {
            col = col.child(div().text_color(t.muted).child("No changes"));
        }
        for line in text.lines().take(MAX_VIEWER_LINES) {
            let color = if line.starts_with("+++") || line.starts_with("---") {
                t.muted
            } else if line.starts_with('+') {
                t.pos
            } else if line.starts_with('-') {
                t.neg
            } else if line.starts_with("@@") {
                t.info
            } else if line.starts_with("diff ") || line.starts_with("index ") {
                t.muted
            } else {
                t.ink_2
            };
            col = col.child(
                div()
                    .text_color(color)
                    .child(SharedString::from(line.to_string())),
            );
        }
        if total > MAX_VIEWER_LINES {
            col = col.child(
                div()
                    .pt(t.sp2)
                    .text_color(t.dim)
                    .child(format!("… {} more lines", total - MAX_VIEWER_LINES)),
            );
        }
        col
    }

    fn blame_body(&self, v: &Viewer) -> impl IntoElement {
        let t = &self.theme;
        let mut col = v_flex()
            .p(t.sp3)
            .gap(px(1.0))
            .font_family(t.mono.clone())
            .text_size(t.fs_sm);
        if v.blame.is_empty() {
            col = col.child(div().text_color(t.muted).child("No blame data"));
        }
        for bl in v.blame.iter().take(MAX_VIEWER_LINES) {
            col = col.child(
                h_flex()
                    .gap(t.sp2)
                    .child(
                        div()
                            .flex_none()
                            .w(px(60.0))
                            .text_color(t.accent)
                            .child(bl.short_hash.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(76.0))
                            .overflow_hidden()
                            .truncate()
                            .text_color(t.muted)
                            .child(bl.author.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_color(t.ink_2)
                            .child(SharedString::from(bl.content.clone())),
                    ),
            );
        }
        if v.blame.len() > MAX_VIEWER_LINES {
            col = col.child(
                div()
                    .pt(t.sp2)
                    .text_color(t.dim)
                    .child(format!("… {} more lines", v.blame.len() - MAX_VIEWER_LINES)),
            );
        }
        col
    }

    fn viewer_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let Some(v) = &self.viewer else {
            return div().into_any_element();
        };
        let toggle_label = if v.is_blame { "Diff" } else { "Blame" };
        let header = h_flex()
            .items_center()
            .gap(t.sp2)
            .w_full()
            .h(t.panel_header_h)
            .px(t.sp3)
            .border_b_1()
            .border_color(t.line)
            .child(ui::icon(
                if v.is_blame { "git-branch" } else { "file-text" },
                px(15.0),
                t.accent,
            ))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .truncate()
                    .font_family(t.mono.clone())
                    .text_color(t.ink)
                    .child(v.path.clone()),
            )
            .child(self.pill("git-view-toggle", toggle_label, false).on_mouse_down(
                MouseButton::Left,
                {
                    let path = v.path.clone();
                    let (is_blame, staged, untracked) = (v.is_blame, v.staged, v.untracked);
                    cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                        if is_blame {
                            this.open_diff(path.clone(), staged, untracked, cx)
                        } else {
                            this.open_blame(path.clone(), staged, untracked, cx)
                        }
                    })
                },
            ))
            .child(
                div()
                    .id("git-view-close")
                    .ml(t.sp2)
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(22.0))
                    .h(px(22.0))
                    .rounded(t.radius_sm)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                            this.close_viewer();
                            cx.notify();
                        }),
                    )
                    .child(ui::icon("close", px(14.0), t.muted)),
            );

        let body = if v.loading {
            div()
                .p(t.sp4)
                .text_color(t.muted)
                .child("Loading…")
                .into_any_element()
        } else if let Some(err) = &v.error {
            div()
                .p(t.sp4)
                .font_family(t.mono.clone())
                .text_size(t.fs_sm)
                .text_color(t.neg)
                .child(err.clone())
                .into_any_element()
        } else if v.is_blame {
            self.blame_body(v).into_any_element()
        } else {
            self.diff_body(v).into_any_element()
        };

        v_flex()
            .size_full()
            .min_h(px(0.0))
            .child(header)
            .child(
                div()
                    .id("git-view-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(body),
            )
            .into_any_element()
    }

    // ── Render: the panel body (header + chips + tab content) ────────

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let Some(git) = &self.git else {
            return v_flex()
                .flex_1()
                .child(ui::panel_header(t, "git-branch", "GIT", ""))
                .child(
                    div()
                        .p(t.sp4)
                        .text_color(t.muted)
                        .child("Not a git repository"),
                )
                .into_any_element();
        };
        let total = git.staged.len() + git.unstaged.len();
        let ahead_behind = format!("↑{} ↓{}", git.ahead, git.behind);
        let tracking = if git.tracking.is_empty() {
            "no upstream".to_string()
        } else {
            format!("tracking {}", git.tracking)
        };

        let mut col = v_flex()
            .flex_1()
            .min_h(px(0.0))
            .child(ui::panel_header(t, "git-branch", "GIT", git.branch.clone()))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(t.sp2)
                    .px(t.sp3)
                    .py(t.sp2)
                    .border_b_1()
                    .border_color(t.line)
                    .child(self.git_chip(cx, "Changes", Some(total), GitTab::Changes))
                    .child(self.git_chip(cx, "History", None, GitTab::History))
                    .child(self.git_chip(cx, "Branches", None, GitTab::Branches))
                    .child(self.git_chip(cx, "Stash", None, GitTab::Stash))
                    .child(self.git_chip(cx, "Tags", None, GitTab::Tags))
                    .child(self.git_chip(cx, "Remotes", None, GitTab::Remotes))
                    .child(self.git_chip(cx, "Config", None, GitTab::Config))
                    .child(self.git_chip(cx, "Submodules", None, GitTab::Submodules)),
            )
            .child(
                v_flex()
                    .m(t.sp3)
                    .p(t.sp3)
                    .gap(t.sp2)
                    .rounded(t.radius_md)
                    .bg(t.panel)
                    .border_1()
                    .border_color(t.line)
                    .child(
                        h_flex()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .font_family(t.mono.clone())
                                    .text_color(t.ink)
                                    .child(git.branch.clone()),
                            )
                            .child(
                                div()
                                    .text_size(t.fs_sm)
                                    .text_color(t.muted)
                                    .child(ahead_behind),
                            ),
                    )
                    .child(div().text_size(t.fs_sm).text_color(t.muted).child(tracking))
                    .child(
                        h_flex()
                            .gap(t.sp2)
                            .pt(t.sp1)
                            .child(self.git_btn(cx, "Push", true, Some(GitRemoteOp::Push)))
                            .child(self.git_btn(cx, "Pull", false, Some(GitRemoteOp::Pull)))
                            .child(self.git_btn(cx, "Fetch", false, Some(GitRemoteOp::Fetch)))
                            .child(self.git_btn(cx, "Rebase", false, Some(GitRemoteOp::Rebase))),
                    )
                    .when_some(self.git_msg.clone(), |d, msg| {
                        d.child(
                            div()
                                .text_size(t.fs_sm)
                                .font_family(t.mono.clone())
                                .text_color(t.ink_2)
                                .child(msg),
                        )
                    }),
            );

        match self.git_tab {
            GitTab::Changes => {
                if !git.staged.is_empty() {
                    col = col
                        .child(ui::section_label(t, format!("STAGED · {}", git.staged.len())))
                        .child(self.commit_box(cx));
                    for c in &git.staged {
                        col = col.child(self.git_change_row(cx, c, true));
                    }
                }
                if !git.unstaged.is_empty() {
                    col = col
                        .child(ui::section_label(t, format!("CHANGES · {}", git.unstaged.len())));
                    for c in &git.unstaged {
                        col = col.child(self.git_change_row(cx, c, false));
                    }
                }
                if total == 0 {
                    col = col.child(
                        div().p(t.sp4).text_color(t.muted).child("Working tree clean"),
                    );
                }
            }
            GitTab::History => {
                col = col.child(ui::section_label(
                    t,
                    format!("HISTORY · {}", self.git_history.len()),
                ));
                if self.git_history.is_empty() {
                    col = col.child(div().p(t.sp4).text_color(t.muted).child("No commits"));
                } else {
                    for c in &self.git_history {
                        col = col.child(self.git_commit_row(c));
                    }
                }
            }
            GitTab::Branches => {
                col = col.child(ui::section_label(
                    t,
                    format!("BRANCHES · {}", self.git_branch_list.len()),
                ));
                if self.git_branch_list.is_empty() {
                    col = col.child(div().p(t.sp4).text_color(t.muted).child("No branches"));
                } else {
                    for b in &self.git_branch_list {
                        col = col.child(self.git_branch_row(cx, b, b == &git.branch));
                    }
                }
            }
            GitTab::Stash => {
                col = col.child(ui::section_label(
                    t,
                    format!("STASH · {}", self.git_stashes.len()),
                ));
                if self.git_stashes.is_empty() {
                    col = col.child(div().p(t.sp4).text_color(t.muted).child("No stashes"));
                } else {
                    for s in &self.git_stashes {
                        col = col.child(self.git_stash_row(s));
                    }
                }
            }
            GitTab::Tags => {
                col = col.child(self.add_header(cx, "TAGS", self.git_tags.len(), Composer::Tag));
                if self.composer == Composer::Tag {
                    col = col.child(self.composer_box(cx));
                }
                if self.git_tags.is_empty() {
                    col = col.child(div().p(t.sp4).text_color(t.muted).child("No tags"));
                } else {
                    for tg in &self.git_tags {
                        col = col.child(self.tag_row(cx, tg));
                    }
                }
            }
            GitTab::Remotes => {
                col = col.child(self.add_header(
                    cx,
                    "REMOTES",
                    self.git_remotes.len(),
                    Composer::Remote,
                ));
                if self.composer == Composer::Remote {
                    col = col.child(self.composer_box(cx));
                }
                if self.git_remotes.is_empty() {
                    col = col.child(div().p(t.sp4).text_color(t.muted).child("No remotes"));
                } else {
                    for r in &self.git_remotes {
                        col = col.child(self.remote_row(cx, r));
                    }
                }
            }
            GitTab::Config => {
                col = col.child(self.add_header(
                    cx,
                    "CONFIG",
                    self.git_config.len(),
                    Composer::Config,
                ));
                if self.composer == Composer::Config {
                    col = col.child(self.composer_box(cx));
                }
                if self.git_config.is_empty() {
                    col = col.child(div().p(t.sp4).text_color(t.muted).child("No config entries"));
                } else {
                    for e in &self.git_config {
                        col = col.child(self.config_row(cx, e));
                    }
                }
            }
            GitTab::Submodules => {
                col = col.child(self.submodule_header(cx));
                if self.git_submodules.is_empty() {
                    col = col.child(div().p(t.sp4).text_color(t.muted).child("No submodules"));
                } else {
                    for s in &self.git_submodules {
                        col = col.child(self.submodule_row(s));
                    }
                }
            }
        }
        col.into_any_element()
    }
}

impl Render for GitPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        if self.viewer.is_some() {
            self.viewer_view(cx)
        } else {
            div()
                .id("git-scroll")
                .size_full()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(self.body(cx))
                .into_any_element()
        }
    }
}

/// Trim a remote/op result into a one-line status, mapping empty success
/// output to a neutral "Done".
fn summarize(res: Result<String, String>) -> String {
    match res {
        Ok(s) => {
            let s = s.trim().to_string();
            if s.is_empty() {
                "Done".to_string()
            } else {
                s
            }
        }
        Err(e) => e,
    }
}

/// Single-char mark + colour for a git file status.
fn status_style(t: &Theme, s: &FileStatus) -> (&'static str, Hsla) {
    let color = match s {
        FileStatus::Modified => t.warn,
        FileStatus::Added => t.pos,
        FileStatus::Deleted => t.neg,
        FileStatus::Renamed => t.info,
        FileStatus::Untracked => t.muted,
        FileStatus::Conflicted => t.neg,
        FileStatus::Copied => t.info,
    };
    (s.code(), color)
}
