//! Three-pane shell mirroring `Pier/PierApp/Sources/Views/MainWindow/MainView.swift`.
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │ Toolbar: [☰L]  Pier-X            [+] [☰R] [☀/☾] │
//! ├──────────┬───────────────────────┬──────────────┤
//! │   Left   │  Tab bar (terminals)  │     Right    │
//! │ Files /  │  ─────────────────    │ 10 mode      │
//! │ Servers  │  Active terminal      │ container +  │
//! │          │   OR Welcome cover    │ icon sidebar │
//! └──────────┴───────────────────────┴──────────────┘
//! ```

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    div, prelude::*, px, AnyElement, ClickEvent, Context, Entity, IntoElement, SharedString,
    Window,
};
use gpui_component::{Icon as UiIcon, IconName, WindowExt as _};
use pier_core::connections::ConnectionStore;
use pier_core::ssh::SshConfig;

use crate::app::layout::{
    RightMode, LEFT_PANEL_DEFAULT_W, RIGHT_PANEL_DEFAULT_W, TOOLBAR_HEIGHT,
};
use crate::app::ssh_session::SshSessionState;
use crate::app::{
    ActivationHandler, CloseActiveTab, NewTab, ToggleLeftPanel, ToggleRightPanel,
};
use crate::components::{StatusKind, StatusPill};
use crate::data::ShellSnapshot;
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_CAPTION, SIZE_SMALL, WEIGHT_MEDIUM},
    ThemeMode,
};
use crate::views::left_panel_view::{icons as toolbar_icons, LeftPanelView};
use crate::views::right_panel::{ModeSelector, RightPanel};
use crate::views::terminal::TerminalPanel;
use crate::views::welcome::WelcomeView;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;

pub struct PierApp {
    // ─── Layout state ───
    left_visible: bool,
    right_visible: bool,
    right_mode: RightMode,

    // ─── Backend snapshots ───
    snapshot: ShellSnapshot,
    connections: Vec<SshConfig>,

    // ─── Terminal sessions (Pier mirror: multi-tab) ───
    terminals: Vec<Entity<TerminalPanel>>,
    active_terminal: Option<usize>,

    /// Last file the user opened from the left panel that should drive the
    /// right-panel Markdown mode. Set by [`Self::open_markdown_file`].
    last_opened_file: Option<PathBuf>,

    // ─── Active SSH session (right-panel SFTP / future remote modes) ───
    active_session: Option<Entity<SshSessionState>>,

    // ─── Subviews owning their own state (Phase 9 perf split) ───
    /// Filter input + file-browser cwd cache + tab state live inside this
    /// entity so its `cx.notify()` only repaints the left column rather
    /// than the whole shell on every keystroke. PierApp talks to it only
    /// via `cx.observe` (LeftPanelView pulls fresh `connections` on PierApp
    /// notify) and read-only accessors like [`Self::connections_snapshot`].
    left_panel: Entity<LeftPanelView>,
}

impl PierApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let connections = ConnectionStore::load_default()
            .map(|s| s.connections)
            .unwrap_or_default();
        let weak_app = cx.entity().downgrade();
        let connections_for_panel = connections.clone();
        let left_panel = cx.new(|lp_cx| {
            LeftPanelView::new(weak_app, connections_for_panel, window, lp_cx)
        });

        Self {
            left_visible: true,
            right_visible: true,
            right_mode: RightMode::Markdown,
            snapshot: ShellSnapshot::load(),
            connections,
            terminals: Vec::new(),
            active_terminal: None,
            last_opened_file: None,
            active_session: None,
            left_panel,
        }
    }

    /// Read-only snapshot of the saved connections, used by
    /// [`LeftPanelView`] to keep its local cache in sync via `cx.observe`.
    pub fn connections_snapshot(&self) -> Vec<SshConfig> {
        self.connections.clone()
    }

    // ─── Terminal session management ───

    pub fn open_terminal_tab(&mut self, cx: &mut Context<Self>) {
        let on_activated: ActivationHandler = Rc::new(|_, _, _| {});
        let entity = cx.new(|cx| TerminalPanel::new(on_activated, cx));
        self.terminals.push(entity);
        self.active_terminal = Some(self.terminals.len() - 1);
        cx.notify();
    }

    /// Open a terminal tab and immediately type `ssh user@host -p port` into
    /// the new PTY. Mirrors Pier's "click a saved server → terminal opens
    /// connecting" flow; the OS-level `ssh` binary handles auth (key + agent
    /// + Keychain pop-ups), so Pier-X doesn't have to ship a parallel SSH
    /// auth UI for the common case.
    ///
    /// Real `russh`-backed sessions land in a later phase — the placeholder
    /// covers 90 % of the UX with 10 lines of code.
    pub fn open_ssh_terminal(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(conn) = self.connections.get(idx).cloned() else {
            eprintln!("[pier] ssh-open: stale index {idx}");
            return;
        };
        self.open_terminal_tab(cx);
        let Some(active) = self.active_terminal else {
            return;
        };
        let entity = self.terminals[active].clone();
        let command = if conn.port == 22 {
            format!("ssh {}@{}\n", conn.user, conn.host)
        } else {
            format!("ssh {}@{} -p {}\n", conn.user, conn.host, conn.port)
        };
        entity.update(cx, |panel, cx| panel.send_input(&command, cx));

        // Attach a parallel native SSH session for the right-panel SFTP
        // browser. Lazy-connects on first list_dir; replacing whatever
        // previous session was active.
        let session_state = cx.new(|_| SshSessionState::new(conn));
        self.active_session = Some(session_state);
        cx.notify();
    }

    pub fn navigate_sftp(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(session) = self.active_session.as_ref() {
            session.update(cx, |s, _| s.navigate_to(path));
            cx.notify();
        }
    }

    pub fn sftp_cd_up(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.active_session.as_ref() {
            session.update(cx, |s, _| s.cd_up());
            cx.notify();
        }
    }

    /// Switch the right panel mode. Lazily triggers any one-shot work the
    /// new mode needs (e.g. SFTP first-connect) here on the click rather
    /// than from inside the render path.
    pub fn set_right_mode(&mut self, mode: RightMode, cx: &mut Context<Self>) {
        self.right_mode = mode;
        if mode == RightMode::Sftp {
            if let Some(session) = self.active_session.as_ref() {
                // Only refresh on the very first SFTP-tab click for this
                // session — subsequent clicks reuse the cached SFTP channel.
                let needs_first = {
                    let s = session.read(cx);
                    s.entries.is_empty() && s.last_error.is_none()
                };
                if needs_first {
                    // ⚠️ Synchronous block: connect_blocking + list_dir_blocking
                    // freezes the UI for the duration of the SSH handshake
                    // (~1-2s on LAN). Acceptable here because it fires at
                    // most once per session — moving to a background task
                    // with a Connecting… placeholder is Phase 9.
                    session.update(cx, |s, _| s.refresh());
                }
            }
        }
        cx.notify();
    }

    fn activate_terminal_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.terminals.len() {
            self.active_terminal = Some(idx);
            cx.notify();
        }
    }

    fn close_terminal_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.terminals.len() {
            return;
        }
        self.terminals.remove(idx);
        if self.terminals.is_empty() {
            self.active_terminal = None;
        } else {
            // Snap active to a valid index, preferring the previous neighbour.
            let new_active = match self.active_terminal {
                Some(active) if active == idx => idx.saturating_sub(1).min(self.terminals.len() - 1),
                Some(active) if active > idx => active - 1,
                Some(active) => active,
                None => 0,
            };
            self.active_terminal = Some(new_active);
        }
        cx.notify();
    }

    /// Called by [`LeftPanelView`] when the user clicks a `.md` file in
    /// the file browser. Routes into the right-panel Markdown mode.
    pub fn open_markdown_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        eprintln!("[pier] markdown opened: {}", path.display());
        self.last_opened_file = Some(path);
        self.right_mode = RightMode::Markdown;
        self.right_visible = true;
        cx.notify();
    }

    pub fn refresh_connections(&mut self) {
        self.connections = ConnectionStore::load_default()
            .map(|s| s.connections)
            .unwrap_or_default();
    }

    pub fn open_add_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        crate::views::edit_connection::open(
            window,
            cx,
            weak,
            crate::views::edit_connection::EditTarget::Add,
        );
    }

    pub fn open_edit_connection(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(original) = self.connections.get(idx).cloned() else {
            eprintln!("[pier] edit: stale index {idx}");
            return;
        };
        let weak = cx.entity().downgrade();
        crate::views::edit_connection::open(
            window,
            cx,
            weak,
            crate::views::edit_connection::EditTarget::Edit { idx, original },
        );
    }

    pub fn delete_connection(&mut self, idx: usize, cx: &mut Context<Self>) {
        let mut store = ConnectionStore::load_default().unwrap_or_default();
        if idx >= store.connections.len() {
            eprintln!("[pier] delete: stale index {idx}");
            return;
        }
        // KeychainPassword and PublicKeyFile entries point at OS-keychain
        // secrets — clean those up so the keyring doesn't accumulate
        // dangling credentials when the user deletes a connection.
        if let Some(conn) = store.connections.get(idx) {
            match &conn.auth {
                pier_core::ssh::AuthMethod::KeychainPassword { credential_id } => {
                    let _ = pier_core::credentials::delete(credential_id);
                }
                pier_core::ssh::AuthMethod::PublicKeyFile {
                    passphrase_credential_id: Some(id),
                    ..
                } => {
                    let _ = pier_core::credentials::delete(id);
                }
                _ => {}
            }
        }
        store.connections.remove(idx);
        if let Err(err) = store.save_default() {
            eprintln!("[pier] delete connection failed: {err}");
            return;
        }
        self.refresh_connections();
        cx.notify();
    }

    /// Open a confirm dialog before actually calling [`Self::delete_connection`].
    /// Wired to the trash icon on each row in the Servers list.
    pub fn confirm_delete_connection(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(conn) = self.connections.get(idx).cloned() else {
            return;
        };
        let weak = cx.entity().downgrade();
        let title: SharedString = format!("Delete \"{}\"?", conn.name).into();
        let detail: SharedString =
            format!("{}@{}:{} will be removed from connections.json.", conn.user, conn.host, conn.port).into();
        window.open_dialog(cx, move |dialog, _w, _app_cx| {
            let weak = weak.clone();
            let body = crate::components::text::body(detail.clone()).secondary();
            dialog
                .title(title.clone())
                .w(px(380.0))
                .confirm()
                .button_props(
                    gpui_component::dialog::DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(gpui_component::button::ButtonVariant::Danger)
                        .cancel_text("Cancel"),
                )
                .on_ok(move |_, _w, app_cx| {
                    let _ = weak.update(app_cx, |this, cx| this.delete_connection(idx, cx));
                    true
                })
                .child(body)
        });
    }
}


impl Render for PierApp {
    fn render(&mut self, _win: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();

        let toolbar = self.render_toolbar(&t, cx);
        // LeftPanelView is its own Entity — embedding the entity instead of
        // re-building a RenderOnce here means PierApp re-renders DON'T
        // rebuild the file tree cache or filter inputs (Phase 9 perf win).
        let left_entity = self.left_panel.clone();
        let center = self.render_center(&t, cx);
        let right = self
            .right_visible
            .then(|| self.render_right(cx))
            .map(IntoElement::into_any_element);
        let statusbar = self.render_statusbar(&t);

        let mut row = div().flex().flex_row().flex_1().min_h(px(0.0));
        if self.left_visible {
            row = row.child(div().w(LEFT_PANEL_DEFAULT_W).h_full().child(left_entity));
        }
        row = row.child(div().flex_1().min_w(px(0.0)).h_full().child(center));
        if let Some(panel) = right {
            row = row.child(div().w(RIGHT_PANEL_DEFAULT_W).h_full().child(panel));
        }

        div()
            .size_full()
            .bg(t.color.bg_canvas)
            .text_color(t.color.text_primary)
            .font_family(t.font_ui.clone())
            .flex()
            .flex_col()
            // Key-binding context — matches `Some("PierApp")` bindings in
            // `main.rs`. Keeps shell shortcuts from leaking out / colliding
            // with terminal-internal key handling when terminal not focused.
            .key_context("PierApp")
            .on_action(cx.listener(|this, _: &ToggleLeftPanel, _, cx| {
                this.left_visible = !this.left_visible;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleRightPanel, _, cx| {
                this.right_visible = !this.right_visible;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                let weak = cx.entity().downgrade();
                let connections = this.connections.clone();
                crate::views::new_tab_chooser::open(window, cx, weak, connections);
            }))
            .on_action(cx.listener(|this, _: &CloseActiveTab, _, cx| {
                if let Some(idx) = this.active_terminal {
                    this.close_terminal_tab(idx, cx);
                }
            }))
            .child(toolbar)
            .child(row)
            .child(statusbar)
    }
}

// ─────────────────────────────────────────────────────────
// Toolbar
// ─────────────────────────────────────────────────────────

impl PierApp {
    fn render_toolbar(
        &self,
        t: &crate::theme::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let toggle_left_icon = if self.left_visible {
            toolbar_icons::TOGGLE_LEFT_OPEN
        } else {
            toolbar_icons::TOGGLE_LEFT_CLOSED
        };
        let toggle_right_icon = if self.right_visible {
            toolbar_icons::TOGGLE_RIGHT_OPEN
        } else {
            toolbar_icons::TOGGLE_RIGHT_CLOSED
        };
        let theme_icon = if t.mode == ThemeMode::Dark {
            toolbar_icons::SUN
        } else {
            toolbar_icons::MOON
        };

        div()
            .h(TOOLBAR_HEIGHT)
            .px(SP_3)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .bg(t.color.bg_panel)
            .border_b_1()
            .border_color(t.color.border_subtle)
            .child(toolbar_icon_button(
                t,
                "tb-toggle-left",
                toggle_left_icon,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.left_visible = !this.left_visible;
                    cx.notify();
                }),
            ))
            .child(
                div()
                    .text_size(SIZE_SMALL)
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(t.color.text_primary)
                    .child("Pier-X"),
            )
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .text_color(t.color.text_tertiary)
                    .child(self.snapshot.workspace_path.clone()),
            )
            .child(div().flex_1())
            .child(toolbar_icon_button(
                t,
                "tb-new-tab",
                toolbar_icons::NEW_TAB,
                cx.listener(|this, _: &ClickEvent, window, cx| {
                    let weak = cx.entity().downgrade();
                    let connections = this.connections.clone();
                    crate::views::new_tab_chooser::open(window, cx, weak, connections);
                }),
            ))
            .child(toolbar_icon_button(
                t,
                "tb-toggle-right",
                toggle_right_icon,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.right_visible = !this.right_visible;
                    cx.notify();
                }),
            ))
            .child(toolbar_icon_button(
                t,
                "tb-toggle-theme",
                theme_icon,
                |_: &ClickEvent, _, app| {
                    crate::theme::toggle(app);
                    crate::ui_kit::sync_theme(app);
                },
            ))
    }
}

fn toolbar_icon_button(
    t: &crate::theme::Theme,
    id: &'static str,
    icon: IconName,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(28.0))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .text_color(t.color.text_secondary)
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover))
        .on_click(on_click)
        .child(UiIcon::new(icon).size(px(14.0)))
}

// ─────────────────────────────────────────────────────────
// Right / Center
// (Left is now a separate `Entity<LeftPanelView>` — embedded directly
// in the `Render::render` flex row.)
// ─────────────────────────────────────────────────────────

impl PierApp {
    fn render_right(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let on_select_mode: ModeSelector = Rc::new(cx.listener(
            |this, mode: &RightMode, _, cx| this.set_right_mode(*mode, cx),
        ));
        let on_sftp_navigate: crate::views::sftp_browser::NavigateHandler =
            Rc::new(cx.listener(|this, path: &PathBuf, _, cx| {
                this.navigate_sftp(path.clone(), cx);
            }));
        let on_sftp_go_up: crate::views::sftp_browser::GoUpHandler =
            Rc::new(cx.listener(|this, _: &(), _, cx| this.sftp_cd_up(cx)));

        // Only forward the path to the Markdown mode if it actually points
        // at a .md file — keeps the empty-state messaging clean.
        let current_markdown = self.last_opened_file.clone().filter(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("md"))
                .unwrap_or(false)
        });
        RightPanel::new(
            self.right_mode,
            self.snapshot.clone(),
            current_markdown,
            self.active_session.clone(),
            on_sftp_navigate,
            on_sftp_go_up,
            on_select_mode,
        )
    }

    fn render_statusbar(&self, t: &crate::theme::Theme) -> impl IntoElement {
        let term_count = self.terminals.len();
        let active_label: SharedString = match self.active_terminal {
            Some(i) if i < term_count => format!("Terminal {} of {}", i + 1, term_count).into(),
            _ if term_count == 0 => "no terminal".into(),
            _ => "no active tab".into(),
        };
        let mode_label: SharedString = format!("right: {}", self.right_mode.label()).into();
        let theme_label: SharedString = if t.mode == ThemeMode::Dark {
            "theme: dark".into()
        } else {
            "theme: light".into()
        };

        div()
            .h(px(22.0))
            .px(SP_3)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .bg(t.color.bg_panel)
            .border_t_1()
            .border_color(t.color.border_subtle)
            .child(StatusPill::new(
                active_label,
                if term_count == 0 {
                    StatusKind::Warning
                } else {
                    StatusKind::Success
                },
            ))
            .child(StatusPill::new(mode_label, StatusKind::Info))
            .child(StatusPill::new(theme_label, StatusKind::Success))
            .child(div().flex_1())
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .text_color(t.color.text_tertiary)
                    .child(format!("{} saved connections", self.connections.len())),
            )
    }

    fn render_center(
        &mut self,
        t: &crate::theme::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(active) = self.active_terminal {
            let tab_count = self.terminals.len();
            let active_term = self.terminals[active].clone();

            let tab_bar = render_terminal_tab_bar(t, active, tab_count, cx);

            return div()
                .h_full()
                .flex()
                .flex_col()
                .child(tab_bar)
                .child(div().flex_1().min_h(px(0.0)).child(active_term))
                .into_any_element();
        }

        // Welcome cover state — uses the existing WelcomeView, but its
        // buttons map to 3-pane semantics (open Servers tab / open terminal).
        let connections = self.connections.clone();
        let left_panel = self.left_panel.clone();
        let on_new_ssh: ClickHandler = Box::new(cx.listener(
            move |this, _ev: &ClickEvent, _, cx| {
                this.left_visible = true;
                this.refresh_connections();
                left_panel.update(cx, |lp, cx| {
                    lp.select_tab(crate::app::layout::LeftTab::Servers, cx);
                });
                cx.notify();
            },
        ));
        let on_open_terminal: ClickHandler = Box::new(cx.listener(
            |this, _ev: &ClickEvent, _, cx| this.open_terminal_tab(cx),
        ));

        div()
            .size_full()
            .bg(t.color.bg_canvas)
            .flex()
            .items_center()
            .justify_center()
            .child(WelcomeView::new(connections, on_new_ssh, on_open_terminal))
            .into_any_element()
    }
}

// ─────────────────────────────────────────────────────────
// Terminal tab bar
// ─────────────────────────────────────────────────────────

fn render_terminal_tab_bar(
    t: &crate::theme::Theme,
    active: usize,
    count: usize,
    cx: &mut Context<PierApp>,
) -> impl IntoElement {
    let mut row = div()
        .h(px(28.0))
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .bg(t.color.bg_panel)
        .border_b_1()
        .border_color(t.color.border_subtle);

    for idx in 0..count {
        let is_active = idx == active;
        let label: SharedString = format!("Terminal {}", idx + 1).into();
        let tab_id: SharedString = format!("term-tab-{idx}").into();
        let close_id: SharedString = format!("term-close-{idx}").into();

        let on_select = cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.activate_terminal_tab(idx, cx);
        });
        let on_close = cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.close_terminal_tab(idx, cx);
        });

        let mut tab = div()
            .id(gpui::ElementId::Name(tab_id))
            .h(px(22.0))
            .px(SP_2)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1_5)
            .rounded(RADIUS_SM)
            .text_size(SIZE_CAPTION)
            .font_weight(WEIGHT_MEDIUM)
            .text_color(if is_active {
                t.color.text_primary
            } else {
                t.color.text_secondary
            })
            .cursor_pointer()
            .hover(|s| s.bg(t.color.bg_hover))
            .on_click(on_select)
            .child(UiIcon::new(IconName::SquareTerminal).size(px(12.0)))
            .child(label)
            .child(
                div()
                    .id(gpui::ElementId::Name(close_id))
                    .w(px(14.0))
                    .h(px(14.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(2.0))
                    .text_color(t.color.text_tertiary)
                    .hover(|s| s.bg(t.color.bg_active).text_color(t.color.text_primary))
                    .on_click(on_close)
                    .child(UiIcon::new(IconName::Close).size(px(10.0))),
            );

        if is_active {
            tab = tab.bg(t.color.bg_surface);
        }
        row = row.child(tab);
    }

    // Inline "+" at end-of-row — same chooser as the toolbar [+].
    let on_new = cx.listener(|this, _: &ClickEvent, window, cx| {
        let weak = cx.entity().downgrade();
        let connections = this.connections.clone();
        crate::views::new_tab_chooser::open(window, cx, weak, connections);
    });
    row.child(
        div()
            .id("term-tab-plus")
            .w(px(22.0))
            .h(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_SM)
            .text_color(t.color.text_secondary)
            .cursor_pointer()
            .hover(|s| s.bg(t.color.bg_hover))
            .on_click(on_new)
            .child(UiIcon::new(IconName::Plus).size(px(12.0))),
    )
}
