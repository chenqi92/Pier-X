// Software panel — installed packages / package-manager view.
//
// Renders a saved-server selector; on selection it opens a blocking SSH
// session OFF the render path (cx.background_executor) and gathers the host's
// package-manager environment, its installed-package list, and the current
// mirror via pier-core's `package_manager` / `package_mirror` services. The
// curated registry probe supplies versions + service state, which we join back
// onto the host's installed package names so a row shows a version when Pier-X
// recognises the software. Read-only this milestone — no install / uninstall /
// mirror switching.

use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, MouseButton, MouseDownEvent, SharedString, Window};
use gpui_component::{h_flex, v_flex};

use crate::data;
use crate::theme::Theme;
use crate::ui;

use pier_core::services::{package_manager, package_mirror};
use pier_core::ssh::SshConfig;

/// How many package rows to paint before collapsing the tail into a "+N more"
/// line. The host's installed list can run to several hundred entries.
const MAX_ROWS: usize = 80;

pub struct SoftwarePanel {
    theme: Theme,
    /// Saved SSH connections, loaded once on construction.
    conns: Vec<SshConfig>,
    /// Index into `conns` of the host being inspected.
    selected: Option<usize>,
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
    name: String,
    /// Version from the curated registry probe, when Pier-X tracks the package.
    version: Option<String>,
    /// Service-unit liveness from the registry probe: `Some(true)` active,
    /// `Some(false)` inactive, `None` for software without a service unit.
    service: Option<bool>,
}

impl SoftwarePanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
            conns: data::connections_raw(),
            selected: None,
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

    /// The saved-server picker, always visible above the body.
    fn selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let mut col =
            v_flex().child(ui::section_label(t, format!("SERVER · {}", self.conns.len())));
        if self.conns.is_empty() {
            return col.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child("No saved connections"),
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
    fn body(&self) -> AnyElement {
        let t = &self.theme;
        match &self.load {
            Load::Idle => {
                ui::empty_state(t, "Select a server to inspect installed software")
                    .into_any_element()
            }
            Load::Loading => ui::empty_state(t, "Connecting…").into_any_element(),
            Load::Failed(err) => v_flex()
                .flex_1()
                .child(
                    div()
                        .px(t.sp3)
                        .py(t.sp2)
                        .text_size(t.fs_ui)
                        .text_color(t.neg)
                        .child(format!("Connection failed: {err}")),
                )
                .into_any_element(),
            Load::Ready(data) => self.ready(data).into_any_element(),
        }
    }

    /// Paint the gathered software state for one host.
    fn ready(&self, d: &SoftwareData) -> impl IntoElement {
        let t = &self.theme;
        let mut col = v_flex()
            .child(ui::section_label(t, "ENVIRONMENT"))
            .child(ui::info_row(
                t,
                "Package manager",
                d.manager.clone().unwrap_or_else(|| "unknown".to_string()),
            ))
            .child(ui::info_row(t, "Distro", d.distro.clone()))
            .child(ui::info_row(t, "Mirror", d.mirror.clone()))
            .child(ui::info_row(t, "Root", if d.is_root { "yes" } else { "no" }))
            .child(ui::info_row(t, "Installed", d.total.to_string()));
        if d.manager.is_none() {
            col = col.child(
                div()
                    .px(t.sp3)
                    .py(t.sp1)
                    .text_size(t.fs_sm)
                    .text_color(t.warn)
                    .child("No supported package manager detected"),
            );
        }

        col = col.child(ui::section_label(t, format!("PACKAGES · {}", d.rows.len())));
        if d.rows.is_empty() {
            col = col.child(
                div()
                    .px(t.sp3)
                    .py(t.sp1)
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child("none"),
            );
        } else {
            for row in d.rows.iter().take(MAX_ROWS) {
                col = col.child(self.pkg_row(row));
            }
            if d.rows.len() > MAX_ROWS {
                col = col.child(
                    div()
                        .px(t.sp3)
                        .py(t.sp1)
                        .text_size(t.fs_sm)
                        .text_color(t.muted)
                        .child(format!("+{} more", d.rows.len() - MAX_ROWS)),
                );
            }
        }

        // Body scrolls; the header + selector above stay pinned.
        div()
            .id("sw-scroll")
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .child(col)
    }

    /// One package row: optional service dot + name on the left, version on the
    /// right (mono) when known.
    fn pkg_row(&self, row: &PkgRow) -> impl IntoElement {
        let t = &self.theme;
        let mut left = h_flex().items_center().gap(t.sp2).min_w(px(0.0)).flex_1().overflow_hidden();
        if let Some(active) = row.service {
            left = left.child(ui::status_dot(if active { t.pos } else { t.muted }));
        }
        left = left.child(
            div()
                .overflow_hidden()
                .text_size(t.fs_ui)
                .text_color(t.ink_2)
                .child(row.name.clone()),
        );

        let mut r = h_flex()
            .items_center()
            .justify_between()
            .gap(t.sp2)
            .px(t.sp3)
            .py(px(3.0))
            .child(left);
        if let Some(version) = &row.version {
            r = r.child(
                div()
                    .flex_none()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .text_color(t.muted)
                    .child(version.clone()),
            );
        }
        r
    }
}

impl Render for SoftwarePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = &self.theme;
        let meta: SharedString = match &self.load {
            Load::Ready(d) => d.total.to_string().into(),
            _ => SharedString::default(),
        };
        v_flex()
            .size_full()
            .child(ui::panel_header(t, "package", "SOFTWARE", meta))
            .child(self.selector(cx))
            .child(self.body())
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
            detected.push(PkgRow {
                name: p.id,
                version: p.version,
                service: p.service_active,
            });
        }
    }

    // Prefer the host's full installed list (names only); fall back to the
    // registry-detected set when the manager reports nothing (or is unknown).
    let rows: Vec<PkgRow> = if names.is_empty() {
        detected
    } else {
        names
            .iter()
            .map(|name| {
                let id = pm.and_then(|m| package_manager::resolve_descriptor_for_package(name, m));
                PkgRow {
                    name: name.clone(),
                    version: id.and_then(|i| versions.get(i).cloned()),
                    service: id.and_then(|i| services.get(i).copied()),
                }
            })
            .collect()
    };

    let total = if names.is_empty() { rows.len() } else { names.len() };
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
