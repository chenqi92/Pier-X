// SFTP panel — read-only remote file browser over an SSH session.
//
// Pick a saved connection, open an SFTP channel off the render path, and walk
// the remote tree: directories first, click a folder to descend, a ".." row to
// go back up. Connect/list failures surface as a single error line. Upload /
// download are out of scope for this pass.

use gpui::prelude::*;
use gpui::{div, px, Context, MouseButton, MouseDownEvent, SharedString, Window};
use gpui_component::{h_flex, v_flex};

use pier_core::ssh::{RemoteFileEntry, SftpClient, SshConfig, SshSession};

use crate::data;
use crate::theme::Theme;
use crate::ui;

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
}

impl SftpPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
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

    fn entry_row(&self, cx: &mut Context<Self>, e: &RemoteFileEntry) -> impl IntoElement {
        let t = &self.theme;
        let glyph = if e.is_dir { "folder" } else { "file" };
        let glyph_color = if e.is_dir { t.accent } else { t.muted };
        let size = if e.is_dir {
            String::new()
        } else {
            human_size(e.size)
        };
        let path = e.path.clone();
        let is_dir = e.is_dir;
        h_flex()
            .id(SharedString::from(format!("sftp-entry-{}", e.path)))
            .items_center()
            .gap(t.sp2)
            .h(px(26.0))
            .px(t.sp3)
            .text_color(t.ink_2)
            .hover(|s| s.bg(t.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    if is_dir {
                        this.navigate(path.clone(), cx);
                    }
                }),
            )
            .child(ui::icon(glyph, px(14.0), glyph_color))
            .child(div().flex_1().overflow_hidden().child(e.name.clone()))
            .child(
                div()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(size),
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

        let mut body = v_flex().id("sftp-body").flex_1().min_h(px(0.0)).overflow_y_scroll();

        if self.error.is_some() {
            body = body.child(self.error_line());
        }

        if self.sftp.is_some() {
            // Connected: header shows the active connection, body is the listing.
            body = body.child(ui::section_label(&t, self.conn_name.clone()));
            if parent_of(&self.cwd) != self.cwd {
                body = body.child(self.up_row(cx));
            }
            if self.entries.is_empty() && !self.loading {
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

/// Compact human-readable byte size, e.g. `4.0 K`, `1.2 M`.
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
