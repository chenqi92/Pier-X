// Software panel — installed packages / package-manager view.
//
// Renders a saved-server selector; on selection it opens a blocking SSH
// session OFF the render path (cx.background_executor) and gathers the host's
// package-manager environment, its installed-package list, and the current
// mirror via pier-core's `package_manager` / `package_mirror` services. The
// curated registry probe supplies versions + service state + category, which
// we join back onto the host's installed package names so a row shows a
// version, a service dot, and lands in the right category section when Pier-X
// recognises the software.
//
// Per-row actions are command-injection only (PRODUCT-SPEC-safe): the panel
// never runs a privileged command itself. Each action button copies the exact
// `sudo apt install …` / `sudo systemctl restart …` command (built for the
// host's detected manager via crate::data) to the clipboard for the user to
// paste into a terminal.

use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, ClipboardItem, Context, FocusHandle, Hsla, KeyDownEvent, MouseButton,
    MouseDownEvent, SharedString, Window,
};
use gpui_component::{h_flex, v_flex};

use crate::data::{self, PkgAction};
use crate::theme::Theme;
use crate::i18n;
use crate::ui;

use pier_core::services::package_manager::{self, PackageDescriptor, PackageManager};
use pier_core::services::package_mirror;
use pier_core::ssh::SshConfig;

/// How many rows to paint in the catch-all "Other" section before collapsing
/// the tail into a "+N more" line. Recognised category sections are bounded by
/// the curated registry and always shown in full; only the long tail of
/// unrecognised host packages is capped.
const MAX_ROWS: usize = 80;

/// Registry category id → section header, in the same order the web panel
/// (`SoftwarePanel.tsx`) groups its app-store sections. Anything with an empty
/// or unlisted category falls into "Other".
const CATEGORY_ORDER: &[(&str, &str)] = &[
    ("database", "DATABASES"),
    ("container", "CONTAINERS"),
    ("web", "WEB SERVERS"),
    ("runtime", "LANGUAGES & RUNTIMES"),
    ("dev", "BUILD TOOLS"),
    ("editor", "EDITORS"),
    ("terminal", "SHELLS & MULTIPLEXERS"),
    ("network", "NETWORK TOOLS"),
    ("text", "TEXT & SEARCH"),
    ("system", "SYSTEM UTILITIES"),
];

pub struct SoftwarePanel {
    theme: Theme,
    /// Focus handle backing the package filter input.
    focus: FocusHandle,
    /// Saved SSH connections, loaded once on construction.
    conns: Vec<SshConfig>,
    /// Index into `conns` of the host being inspected.
    selected: Option<usize>,
    /// Live filter text matched against each row's name / display / category.
    query: String,
    /// The command most recently copied to the clipboard, echoed in a banner
    /// so the otherwise-invisible copy is visible to the user.
    last_copied: Option<SharedString>,
    load: Load,
}

/// Lifecycle of the per-host gather. Owns the collected data so render stays a
/// pure paint of cached state.
enum Load {
    /// Nothing selected yet.
    Idle,
    /// Connecting + collecting in the background.
    Loading,
    /// Gather succeeded.
    Ready(SoftwareData),
    /// Connect / collect failed; carries the display message.
    Failed(String),
}

/// Everything the panel paints for one host, pre-formatted so render does no
/// work beyond layout.
struct SoftwareData {
    /// `PRETTY_NAME` (or bare distro id) from `/etc/os-release`.
    distro: String,
    /// Detected package-manager id (`apt` / `dnf` / …), or `None` when the
    /// distro isn't recognised.
    manager: Option<String>,
    /// `true` when the remote session is root.
    is_root: bool,
    /// Friendly current-mirror label (catalog name, hostname, or "upstream").
    mirror: String,
    /// Count of manually-installed packages reported by the manager.
    total: usize,
    /// One entry per installed package, in the manager's order.
    rows: Vec<PkgRow>,
}

/// A single installed package as the list paints it.
struct PkgRow {
    /// Host-reported package name — the exact token install / update /
    /// uninstall commands target.
    name: String,
    /// Friendly label: the registry display name when recognised, else the
    /// raw package name.
    display: String,
    /// Registry category id (e.g. `"database"`); empty when unrecognised.
    category: &'static str,
    /// Version from the curated registry probe, when Pier-X tracks the package.
    version: Option<String>,
    /// Service-unit liveness from the registry probe: `Some(true)` active,
    /// `Some(false)` inactive, `None` for software without a service unit.
    service: Option<bool>,
    /// Resolved systemd unit for the host's manager — `Some` only for
    /// recognised software that declares a service. Drives the start/stop/
    /// restart buttons.
    service_unit: Option<String>,
    /// Packages passed to the install / update / uninstall commands.
    pkgs: Vec<String>,
}

impl SoftwarePanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
            focus: cx.focus_handle(),
            conns: data::connections_raw(),
            selected: None,
            query: String::new(),
            last_copied: None,
            load: Load::Idle,
        }
    }

    /// Begin gathering software state for `idx`. The blocking SSH work runs on
    /// the background executor; the result is stored back on the view and a
    /// `notify` repaints. A stale result (the user picked another host while
    /// this was in flight) is dropped.
    fn start_load(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(cfg) = self.conns.get(idx).cloned() else {
            return;
        };
        self.selected = Some(idx);
        self.query.clear();
        self.last_copied = None;
        self.load = Load::Loading;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { collect(cfg) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.selected != Some(idx) {
                    return;
                }
                this.load = match result {
                    Ok(data) => Load::Ready(data),
                    Err(err) => Load::Failed(err),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Copy `cmd` to the clipboard and record it for the confirmation banner.
    fn copy_command(&mut self, cmd: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(cmd.clone()));
        self.last_copied = Some(cmd.into());
        cx.notify();
    }

    /// Live-filter keystrokes for the package search box.
    fn on_search_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "enter" => return,
            "backspace" => {
                if self.query.pop().is_some() {
                    cx.notify();
                }
                return;
            }
            "escape" => {
                if !self.query.is_empty() {
                    self.query.clear();
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        let m = &ks.modifiers;
        if m.control || m.alt || m.platform {
            return; // leave shortcuts alone
        }
        if let Some(kc) = &ks.key_char {
            if !kc.is_empty() && !kc.chars().any(|c| c.is_control()) {
                self.query.push_str(kc);
                cx.notify();
            }
        }
    }

    /// The saved-server picker, always visible above the body.
    fn selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let mut col = v_flex().child(ui::section_label(
            t,
            format!("{} · {}", i18n::t("sw.server"), self.conns.len()),
        ));
        if self.conns.is_empty() {
            return col.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child(i18n::t("side.no_saved_connections")),
            );
        }
        for (i, c) in self.conns.iter().enumerate() {
            col = col.child(self.conn_row(cx, i, c));
        }
        col
    }

    /// One clickable connection row.
    fn conn_row(&self, cx: &mut Context<Self>, idx: usize, c: &SshConfig) -> impl IntoElement {
        let t = &self.theme;
        let selected = self.selected == Some(idx);
        let addr = format!("{}@{}:{}", c.user, c.host, c.port);
        h_flex()
            .id(SharedString::from(format!("sw-conn-{idx}")))
            .items_center()
            .gap(t.sp2)
            .h(px(38.0))
            .px(t.sp3)
            .when(selected, |d| d.bg(t.accent_dim))
            .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.start_load(idx, cx);
                }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .overflow_hidden()
                            .text_size(t.fs_ui)
                            .text_color(if selected { t.ink } else { t.ink_2 })
                            .child(c.name.clone()),
                    )
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

    /// The state-dependent body below the selector.
    fn body(&self, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        match &self.load {
            Load::Idle => {
                ui::empty_state(t, i18n::t("panel.select_server_software"))
                    .into_any_element()
            }
            Load::Loading => ui::empty_state(t, i18n::t("panel.connecting")).into_any_element(),
            Load::Failed(err) => v_flex()
                .flex_1()
                .child(
                    div()
                        .px(t.sp3)
                        .py(t.sp2)
                        .text_size(t.fs_ui)
                        .text_color(t.neg)
                        .child(format!("{}: {err}", i18n::t("sw.connection_failed"))),
                )
                .into_any_element(),
            Load::Ready(data) => self.ready(data, focused, cx).into_any_element(),
        }
    }

    /// Paint the gathered software state for one host: environment summary +
    /// filter box + category-grouped package rows.
    fn ready(&self, d: &SoftwareData, focused: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let manager = d.manager.as_deref();

        // Environment summary — pinned above the scrolling package list.
        let env = v_flex()
            .child(ui::section_label(t, i18n::t("sw.environment")))
            .child(ui::info_row(
                t,
                i18n::t("sw.package_manager"),
                d.manager
                    .clone()
                    .map(SharedString::from)
                    .unwrap_or_else(|| i18n::t("sw.not_detected")),
            ))
            .child(ui::info_row(t, i18n::t("sw.distro"), d.distro.clone()))
            .child(ui::info_row(t, i18n::t("sw.mirror"), d.mirror.clone()))
            .child(ui::info_row(
                t,
                i18n::t("sw.root"),
                if d.is_root { i18n::t("sw.yes") } else { i18n::t("sw.no") },
            ))
            .child(ui::info_row(t, i18n::t("sw.installed"), d.total.to_string()));

        // Filter, then bucket rows into the registry's category order; the
        // long tail of unrecognised packages collects into "Other".
        let q = self.query.trim().to_lowercase();
        let mut groups: Vec<(&str, &str, Vec<&PkgRow>)> = CATEGORY_ORDER
            .iter()
            .map(|&(id, label)| (id, label, Vec::new()))
            .collect();
        let mut other: Vec<&PkgRow> = Vec::new();
        for r in &d.rows {
            if !row_matches(r, &q) {
                continue;
            }
            match groups.iter_mut().find(|(id, _, _)| *id == r.category) {
                Some((_, _, v)) => v.push(r),
                None => other.push(r),
            }
        }

        let mut col = v_flex();
        let mut any = false;
        for (id, _, rows) in &groups {
            if rows.is_empty() {
                continue;
            }
            any = true;
            col = col.child(ui::section_label(
                t,
                format!("{} · {}", i18n::t(category_label_key(id)), rows.len()),
            ));
            for r in rows.iter().copied() {
                col = col.child(self.pkg_row(cx, r, manager, d.is_root));
            }
        }
        if !other.is_empty() {
            any = true;
            col = col.child(ui::section_label(
                t,
                format!("{} · {}", i18n::t("sw.cat_other"), other.len()),
            ));
            for r in other.iter().take(MAX_ROWS).copied() {
                col = col.child(self.pkg_row(cx, r, manager, d.is_root));
            }
            if other.len() > MAX_ROWS {
                col = col.child(
                    div()
                        .px(t.sp3)
                        .py(t.sp1)
                        .text_size(t.fs_sm)
                        .text_color(t.muted)
                        .child(i18n::tf("sw.more", &[&(other.len() - MAX_ROWS).to_string()])),
                );
            }
        }
        if !any {
            let msg = if q.is_empty() {
                i18n::t("sw.none")
            } else {
                i18n::t("sw.no_matching")
            };
            col = col.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child(msg),
            );
        }

        // "Copied <cmd>" confirmation, since the clipboard write is invisible.
        let banner: Option<AnyElement> = self.last_copied.as_ref().map(|cmd| {
            h_flex()
                .items_center()
                .gap(t.sp2)
                .mx(t.sp3)
                .mb(t.sp2)
                .px(t.sp3)
                .py(px(5.0))
                .rounded(t.radius_md)
                .bg(t.accent_subtle)
                .child(ui::icon("copy", px(13.0), t.accent))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .font_family(t.mono.clone())
                        .text_size(t.fs_sm)
                        .text_color(t.ink_2)
                        .child(cmd.clone()),
                )
                .into_any_element()
        });

        v_flex()
            .flex_1()
            .min_h(px(0.0))
            .child(env)
            .child(self.search_bar(focused, cx))
            .children(banner)
            .child(
                div()
                    .id("sw-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(col),
            )
    }

    /// The package filter input (mirrors the Search panel's query box).
    fn search_bar(&self, focused: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let caret = || div().flex_none().w(px(2.0)).h(px(15.0)).bg(t.accent);
        let content: AnyElement = if self.query.is_empty() {
            if focused {
                h_flex().items_center().child(caret()).into_any_element()
            } else {
                div()
                    .text_color(t.dim)
                    .child(i18n::t("sw.search"))
                    .into_any_element()
            }
        } else {
            let mut row = h_flex().items_center().min_w(px(0.0)).overflow_hidden().child(
                div()
                    .flex_none()
                    .text_color(t.ink)
                    .child(self.query.clone()),
            );
            if focused {
                row = row.child(caret());
            }
            row.into_any_element()
        };

        h_flex()
            .id("sw-search")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_search_key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    cx.notify();
                }),
            )
            .items_center()
            .gap(t.sp2)
            .mx(t.sp3)
            .my(t.sp2)
            .px(t.sp3)
            .h(px(30.0))
            .rounded(t.radius_md)
            .bg(t.panel)
            .border_1()
            .border_color(if focused { t.accent } else { t.line })
            .child(ui::icon("search", px(14.0), t.muted))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(content),
            )
    }

    /// One package row: optional service dot + name on the left; version,
    /// service controls, and package actions on the right.
    fn pkg_row(
        &self,
        cx: &mut Context<Self>,
        r: &PkgRow,
        manager: Option<&str>,
        is_root: bool,
    ) -> impl IntoElement {
        let t = &self.theme;

        let mut left = h_flex()
            .items_center()
            .gap(t.sp2)
            .min_w(px(0.0))
            .flex_1()
            .overflow_hidden();
        if let Some(active) = r.service {
            left = left.child(ui::status_dot(if active { t.pos } else { t.muted }));
        }
        left = left.child(
            div()
                .overflow_hidden()
                .text_size(t.fs_ui)
                .text_color(t.ink_2)
                .child(r.display.clone()),
        );

        let mut right = h_flex().items_center().gap(px(2.0)).flex_none();
        if let Some(version) = &r.version {
            right = right.child(
                div()
                    .flex_none()
                    .mr(t.sp1)
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(version.clone()),
            );
        }

        // Service controls (command-injection): contextual like the Docker
        // panel — active services show restart + stop, stopped ones show start.
        if let Some(unit) = &r.service_unit {
            let active = r.service.unwrap_or(false);
            if active {
                right = right
                    .child(self.copy_btn(
                        cx,
                        format!("{}-restart", r.name),
                        "redo-2",
                        t.info,
                        data::systemctl_command("restart", unit, is_root),
                    ))
                    .child(self.copy_btn(
                        cx,
                        format!("{}-stop", r.name),
                        "pause",
                        t.warn,
                        data::systemctl_command("stop", unit, is_root),
                    ));
            } else {
                right = right.child(self.copy_btn(
                    cx,
                    format!("{}-start", r.name),
                    "play",
                    t.pos,
                    data::systemctl_command("start", unit, is_root),
                ));
            }
        }

        // Package actions (install / update / uninstall), gated on a known
        // manager so the copied command is always correct for the host.
        if let Some(m) = manager {
            if let Some(cmd) = data::pkg_command(m, PkgAction::Install, &r.pkgs, is_root) {
                right = right.child(self.copy_btn(
                    cx,
                    format!("{}-install", r.name),
                    "arrow-down",
                    t.muted,
                    cmd,
                ));
            }
            if let Some(cmd) = data::pkg_command(m, PkgAction::Update, &r.pkgs, is_root) {
                right = right.child(self.copy_btn(
                    cx,
                    format!("{}-update", r.name),
                    "arrow-up",
                    t.info,
                    cmd,
                ));
            }
            if let Some(cmd) = data::pkg_command(m, PkgAction::Uninstall, &r.pkgs, is_root) {
                right = right.child(self.copy_btn(
                    cx,
                    format!("{}-remove", r.name),
                    "delete",
                    t.neg,
                    cmd,
                ));
            }
        }

        h_flex()
            .id(SharedString::from(format!("sw-row-{}", r.name)))
            .items_center()
            .justify_between()
            .gap(t.sp2)
            .px(t.sp3)
            .py(px(3.0))
            .hover(|s| s.bg(t.hover))
            .child(left)
            .child(right)
    }

    /// A 20×20 icon button that copies `cmd` to the clipboard on click.
    fn copy_btn(
        &self,
        cx: &mut Context<Self>,
        key: String,
        glyph: &'static str,
        color: Hsla,
        cmd: String,
    ) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(SharedString::from(format!("sw-act-{key}")))
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
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.copy_command(cmd.clone(), cx);
                }),
            )
            .child(ui::icon(glyph, px(13.0), color))
    }
}

impl Render for SoftwarePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = &self.theme;
        let focused = self.focus.is_focused(window);
        let meta: SharedString = match &self.load {
            Load::Ready(d) => d.total.to_string().into(),
            _ => SharedString::default(),
        };
        v_flex()
            .size_full()
            .child(ui::panel_header(t, "package", i18n::t("tool.software"), meta))
            .child(self.selector(cx))
            .child(self.body(focused, cx))
    }
}

/// Resolve the service unit name for `descriptor` on `manager`.
fn unit_for(d: &PackageDescriptor, manager: PackageManager) -> Option<String> {
    d.service_units
        .iter()
        .find_map(|(m, u)| (*m == manager).then(|| u.to_string()))
}

/// Resolve the install package list for `descriptor` on `manager`.
fn pkgs_for(d: &PackageDescriptor, manager: PackageManager) -> Option<Vec<String>> {
    d.install_packages
        .iter()
        .find_map(|(m, ps)| (*m == manager).then(|| ps.iter().map(|s| s.to_string()).collect()))
}

/// Map a registry category id (the first column of [`CATEGORY_ORDER`]) to its
/// i18n key. Anything unmapped falls back to the catch-all "Other" key, which
/// keeps the section header populated even for an id added without a string.
fn category_label_key(id: &str) -> &'static str {
    match id {
        "database" => "sw.cat_database",
        "container" => "sw.cat_container",
        "web" => "sw.cat_web",
        "runtime" => "sw.cat_runtime",
        "dev" => "sw.cat_dev",
        "editor" => "sw.cat_editor",
        "terminal" => "sw.cat_terminal",
        "network" => "sw.cat_network",
        "text" => "sw.cat_text",
        "system" => "sw.cat_system",
        _ => "sw.cat_other",
    }
}

/// Case-insensitive substring match of `q` (already lowercased) against a
/// row's name, display label, or category id. Empty `q` matches everything.
fn row_matches(r: &PkgRow, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    r.name.to_lowercase().contains(q)
        || r.display.to_lowercase().contains(q)
        || r.category.contains(q)
}

/// Build a row from a registry descriptor id (the host's installed list was
/// empty, so we fall back to the registry probe's detected set).
fn row_for_descriptor(
    id: &str,
    version: Option<String>,
    service: Option<bool>,
    pm: Option<PackageManager>,
) -> PkgRow {
    let d = package_manager::descriptor(id);
    let display = d
        .map(|d| d.display_name.to_string())
        .unwrap_or_else(|| id.to_string());
    let category = d.map(|d| d.category).unwrap_or("");
    let (service_unit, pkgs) = match (d, pm) {
        (Some(d), Some(m)) => (
            unit_for(d, m),
            pkgs_for(d, m).unwrap_or_else(|| vec![id.to_string()]),
        ),
        _ => (None, vec![id.to_string()]),
    };
    PkgRow {
        name: id.to_string(),
        display,
        category,
        version,
        service,
        service_unit,
        pkgs,
    }
}

/// Build a row from a host-reported package name, joining the registry's
/// version / service state / category / unit when Pier-X recognises it. The
/// install/update/uninstall commands always target the actual installed
/// package name.
fn row_for_package(
    name: &str,
    versions: &HashMap<String, String>,
    services: &HashMap<String, bool>,
    pm: Option<PackageManager>,
) -> PkgRow {
    let id = pm.and_then(|m| package_manager::resolve_descriptor_for_package(name, m));
    let d = id.and_then(package_manager::descriptor);
    let display = d
        .map(|d| d.display_name.to_string())
        .unwrap_or_else(|| name.to_string());
    let category = d.map(|d| d.category).unwrap_or("");
    let service_unit = match (d, pm) {
        (Some(d), Some(m)) => unit_for(d, m),
        _ => None,
    };
    PkgRow {
        name: name.to_string(),
        display,
        category,
        version: id.and_then(|i| versions.get(i).cloned()),
        service: id.and_then(|i| services.get(i).copied()),
        service_unit,
        pkgs: vec![name.to_string()],
    }
}

/// Blocking gather, run on the background executor. Opens an SSH session and
/// pulls the read-only package-manager state pier-core exposes. (Available-
/// update counts have no read-only blocking API — the update path is
/// write-only — so they aren't surfaced this milestone.)
fn collect(cfg: SshConfig) -> Result<SoftwareData, String> {
    let session = data::connect_blocking(&cfg)?;
    let env = package_manager::probe_host_env_blocking(&session);
    let pm = env.package_manager;
    let names = package_manager::list_user_installed_blocking(&session).unwrap_or_default();

    let mirror = match pm {
        Some(m) => mirror_label(&package_mirror::detect_mirror_blocking(&session, m)),
        None => "—".to_string(),
    };

    // The curated registry probe is the only source of versions + service
    // state; index its installed entries by descriptor id so host package
    // names can be joined back to a version below.
    let mut versions: HashMap<String, String> = HashMap::new();
    let mut services: HashMap<String, bool> = HashMap::new();
    let mut detected: Vec<PkgRow> = Vec::new();
    if pm.is_some() {
        for p in package_manager::probe_all_blocking(&session) {
            if !p.installed {
                continue;
            }
            if let Some(v) = &p.version {
                versions.insert(p.id.clone(), v.clone());
            }
            if let Some(active) = p.service_active {
                services.insert(p.id.clone(), active);
            }
            detected.push(row_for_descriptor(&p.id, p.version, p.service_active, pm));
        }
    }

    // Prefer the host's full installed list (names only); fall back to the
    // registry-detected set when the manager reports nothing (or is unknown).
    let rows: Vec<PkgRow> = if names.is_empty() {
        detected
    } else {
        names
            .iter()
            .map(|name| row_for_package(name, &versions, &services, pm))
            .collect()
    };

    // `rows` already mirrors whichever source we chose above (the host list, or
    // the registry-detected set when the manager reports nothing), so its length
    // is the count the list actually shows under both paths.
    let total = rows.len();
    let distro = if env.distro_pretty.is_empty() {
        env.distro_id
    } else {
        env.distro_pretty
    };

    Ok(SoftwareData {
        distro,
        manager: pm.map(|m| m.as_str().to_string()),
        is_root: env.is_root,
        mirror,
        total,
        rows,
    })
}

/// Friendly label for the detected mirror: the catalog name when the hostname
/// matches a known mirror, else the raw hostname, else "upstream".
fn mirror_label(state: &package_mirror::MirrorState) -> String {
    if let Some(id) = &state.current_id {
        if let Some(mid) = package_mirror::MirrorId::from_str(id) {
            if let Some(choice) = package_mirror::mirror_by_id(mid) {
                return choice.label.to_string();
            }
        }
    }
    state
        .current_host
        .clone()
        .unwrap_or_else(|| "upstream".to_string())
}
