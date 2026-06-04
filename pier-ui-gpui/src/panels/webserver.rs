// Web server panel — nginx/apache/caddy overview for a selected host.
//
// Renders a connection selector backed by data::connections_raw(). Picking a
// connection runs data::connect_blocking() + the pier-core web-server probes on
// a background task and stores an owned snapshot back on the View — the render
// path only paints from that cache. The live session is kept so config files
// can be read on demand off the render path.
//
// When more than one product is detected the body carries an nginx/Apache/Caddy
// segmented control (each pill shows a run-state dot); the rest of the body
// paints the active product only: its run state, parsed site/vhost rows
// (domain / port / root / config), and the discovered config-file paths.
// Clicking a config path expands the file inline (mono, read-only). Reload is
// command-injection style: a "Copy reload cmd" chip writes `systemctl reload
// <binary>` to the clipboard for the user to review and run — the panel never
// executes it.

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, ClipboardItem, Context, MouseButton, MouseDownEvent, SharedString, Window,
};
use gpui_component::{h_flex, v_flex};

use pier_core::services::apache::{self, ApacheNode};
use pier_core::services::nginx::{self, NginxNode};
use pier_core::services::web_server::{self, WebServerKind, WebServerRunState};
use pier_core::ssh::{SshConfig, SshSession};

use crate::data;
use crate::theme::Theme;
use crate::ui;

/// Cap the inlined config preview so a pathological file can't blow up the
/// layout; mirrors the read-only `take(50)` cap on discovered files.
const MAX_CONFIG_LINES: usize = 500;

/// One detected web server and its run state.
struct SvcRow {
    binary: String,
    version: String,
    running: WebServerRunState,
    config_root: String,
}

/// One parsed site / virtual host.
struct SiteRow {
    domain: String,
    port: String,
    root: String,
    file: String,
}

/// Everything the panel paints for one detected product.
struct ProductView {
    kind: WebServerKind,
    svc: SvcRow,
    sites: Vec<SiteRow>,
    /// Full config-file paths (kept absolute so on-demand reads validate
    /// against the product's config root).
    config_files: Vec<String>,
}

/// The owned snapshot the panel paints once a scan finishes.
struct WebOverview {
    products: Vec<ProductView>,
}

/// Lifecycle of the right-hand overview for the selected connection.
enum ScanState {
    Idle,
    Scanning,
    Loaded(WebOverview),
    Failed(String),
}

/// Load state of the inlined config file under `open_path`.
enum FileState {
    Idle,
    Loading,
    Loaded(Vec<String>),
    Failed(String),
}

pub struct WebserverPanel {
    theme: Theme,
    conns: Vec<SshConfig>,
    /// Index into `conns` for the connection being shown, if any.
    selected: Option<usize>,
    state: ScanState,
    /// Live session for the selected host, cached after a successful scan so
    /// config files can be read on demand.
    session: Option<SshSession>,
    /// Which detected product the segmented control is showing.
    active_kind: Option<WebServerKind>,
    /// Config-file path currently expanded inline, if any.
    open_path: Option<String>,
    file_state: FileState,
    /// True once the active product's reload command was copied; reset when the
    /// segment or connection changes.
    reload_copied: bool,
}

impl WebserverPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
            conns: data::connections_raw(),
            selected: None,
            state: ScanState::Idle,
            session: None,
            active_kind: None,
            open_path: None,
            file_state: FileState::Idle,
            reload_copied: false,
        }
    }

    /// Connect to `conns[idx]` and collect the web-server overview off the
    /// render path, then store it (plus the live session) back on the View.
    fn start_scan(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(cfg) = self.conns.get(idx).cloned() else {
            return;
        };
        self.selected = Some(idx);
        self.state = ScanState::Scanning;
        self.session = None;
        self.active_kind = None;
        self.open_path = None;
        self.file_state = FileState::Idle;
        self.reload_copied = false;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { scan(&cfg) })
                .await;
            let _ = this.update(cx, |this, cx| {
                // A newer selection may have superseded this scan; ignore stale results.
                if this.selected != Some(idx) {
                    return;
                }
                match result {
                    Ok((session, overview)) => {
                        // Default to the first detected product (nginx wins on
                        // ties because detection orders nginx → apache → caddy).
                        this.active_kind = overview.products.first().map(|p| p.kind);
                        this.session = Some(session);
                        this.state = ScanState::Loaded(overview);
                    }
                    Err(err) => this.state = ScanState::Failed(err),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Switch the segmented control to `kind`. Config-file state is per product,
    /// so any inline expansion is collapsed.
    fn set_active(&mut self, kind: WebServerKind, cx: &mut Context<Self>) {
        if self.active_kind == Some(kind) {
            return;
        }
        self.active_kind = Some(kind);
        self.open_path = None;
        self.file_state = FileState::Idle;
        self.reload_copied = false;
        cx.notify();
    }

    /// Toggle the inline expansion of `path`. Re-clicking the open file
    /// collapses it; otherwise the file is read off the render path against the
    /// cached session + active product.
    fn open_config(&mut self, path: String, cx: &mut Context<Self>) {
        if self.open_path.as_deref() == Some(path.as_str()) {
            self.open_path = None;
            self.file_state = FileState::Idle;
            cx.notify();
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(kind) = self.active_kind else {
            return;
        };
        self.open_path = Some(path.clone());
        self.file_state = FileState::Loading;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let p = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { read_config(&session, kind, &p) })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Ignore if a newer open (or collapse) superseded this read.
                if this.open_path.as_deref() != Some(path.as_str()) {
                    return;
                }
                this.file_state = match result {
                    Ok(lines) => FileState::Loaded(lines),
                    Err(err) => FileState::Failed(err),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Hand the active product's reload command to the user via the clipboard.
    /// Intentionally does not run anything — reload stays a reviewed action.
    fn copy_reload(&mut self, cmd: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(cmd));
        self.reload_copied = true;
        cx.notify();
    }

    fn header_meta(&self) -> SharedString {
        match &self.state {
            ScanState::Scanning => "scanning…".into(),
            ScanState::Loaded(ov) => {
                if ov.products.is_empty() {
                    "no web server".into()
                } else {
                    ov.products
                        .iter()
                        .map(|p| p.svc.binary.as_str())
                        .collect::<Vec<_>>()
                        .join(" · ")
                        .into()
                }
            }
            _ => "".into(),
        }
    }

    fn conn_row(&self, cx: &mut Context<Self>, idx: usize) -> impl IntoElement {
        let t = &self.theme;
        let cfg = &self.conns[idx];
        let selected = self.selected == Some(idx);
        let name = cfg.name.clone();
        let addr = format!("{}@{}:{}", cfg.user, cfg.host, cfg.port);
        h_flex()
            .id(SharedString::from(format!("ws-conn-{idx}")))
            .items_center()
            .gap(t.sp2)
            .h(px(42.0))
            .px(t.sp3)
            .when(selected, |d| d.bg(t.accent_dim))
            .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                    this.start_scan(idx, cx);
                }),
            )
            .child(ui::status_dot(if selected { t.accent } else { t.muted }))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .overflow_hidden()
                            .text_color(if selected { t.ink } else { t.ink_2 })
                            .child(name),
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

    /// The product picker, shown only when more than one server is detected.
    fn segmented(&self, cx: &mut Context<Self>, products: &[ProductView]) -> impl IntoElement {
        let t = &self.theme;
        let mut bar = h_flex()
            .items_center()
            .gap(t.sp1)
            .w_full()
            .px(t.sp2)
            .py(t.sp1)
            .bg(t.surface)
            .border_b_1()
            .border_color(t.line);
        for p in products {
            let active = self.active_kind == Some(p.kind);
            bar = bar.child(self.segment(cx, p, active));
        }
        bar
    }

    /// One segmented-control pill: product label + run-state dot.
    fn segment(&self, cx: &mut Context<Self>, p: &ProductView, active: bool) -> impl IntoElement {
        let t = &self.theme;
        let kind = p.kind;
        let (dot, _) = run_style(t, p.svc.running);
        h_flex()
            .id(SharedString::from(format!("ws-seg-{}", kind_key(kind))))
            .items_center()
            .gap(t.sp1)
            .px(t.sp2)
            .py(t.sp1)
            .rounded_full()
            .border_1()
            .cursor_pointer()
            .when(active, |d| {
                d.bg(t.accent_subtle)
                    .border_color(t.accent)
                    .text_color(t.accent_hover)
            })
            .when(!active, |d| {
                d.border_color(gpui::transparent_black())
                    .text_color(t.ink_2)
                    .hover(|s| s.bg(t.hover))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _w, cx| this.set_active(kind, cx)),
            )
            .child(
                div()
                    .font_family(t.mono.clone())
                    .text_size(t.fs_sm)
                    .child(kind_label(kind)),
            )
            .child(ui::status_dot(dot))
    }

    /// The active product's run state, config root, and copy-reload chip.
    fn product_summary(&self, cx: &mut Context<Self>, p: &ProductView) -> impl IntoElement {
        let t = &self.theme;
        let (color, label) = run_style(t, p.svc.running);
        let cmd = reload_cmd(&p.svc);
        v_flex()
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .px(t.sp3)
                    .py(t.sp2)
                    .child(ui::status_dot(color))
                    .child(div().text_color(t.ink).child(p.svc.binary.clone()))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(p.svc.version.clone()),
                    )
                    .child(div().text_size(t.fs_sm).text_color(color).child(label)),
            )
            .child(ui::info_row(t, "config", p.svc.config_root.clone()))
            .child(self.reload_chip(cx, cmd))
    }

    /// A reviewable, click-to-copy reload command. The command text is shown so
    /// the user sees exactly what lands on the clipboard.
    fn reload_chip(&self, cx: &mut Context<Self>, cmd: String) -> impl IntoElement {
        let t = &self.theme;
        let (glyph, label, color) = if self.reload_copied {
            ("check", "reload cmd copied", t.pos)
        } else {
            ("copy", "Copy reload cmd", t.ink_2)
        };
        let cmd_for_click = cmd.clone();
        div().px(t.sp3).py(t.sp1).child(
            v_flex()
                .id("ws-reload-chip")
                .gap(t.sp1)
                .px(t.sp2)
                .py(t.sp2)
                .rounded(t.radius_md)
                .border_1()
                .border_color(t.line_2)
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                        this.copy_reload(cmd_for_click.clone(), cx)
                    }),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap(t.sp2)
                        .child(ui::icon(glyph, px(13.0), color))
                        .child(div().text_size(t.fs_sm).text_color(color).child(label)),
                )
                .child(
                    div()
                        .overflow_hidden()
                        .font_family(t.mono.clone())
                        .text_size(t.fs_sm)
                        .text_color(t.muted)
                        .child(cmd),
                ),
        )
    }

    fn site_row(&self, s: &SiteRow) -> impl IntoElement {
        let t = &self.theme;
        v_flex()
            .gap(t.sp1)
            .px(t.sp3)
            .py(t.sp2)
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_color(t.ink_2)
                            .child(s.domain.clone()),
                    )
                    .child(
                        div()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.accent)
                            .child(format!(":{}", s.port)),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.dim)
                            .child(s.root.clone()),
                    )
                    .child(
                        div()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(s.file.clone()),
                    ),
            )
    }

    /// A clickable config-file path; expands the file inline when open.
    fn file_row(&self, cx: &mut Context<Self>, idx: usize, path: &str) -> impl IntoElement {
        let t = &self.theme;
        let open = self.open_path.as_deref() == Some(path);
        let glyph = if open { "chevron-down" } else { "chevron-right" };
        let path_click = path.to_string();
        v_flex()
            .child(
                h_flex()
                    .id(SharedString::from(format!("ws-file-{idx}")))
                    .items_center()
                    .gap(t.sp2)
                    .px(t.sp3)
                    .py(t.sp1)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                            this.open_config(path_click.clone(), cx)
                        }),
                    )
                    .child(ui::icon(glyph, px(12.0), if open { t.accent } else { t.muted }))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(if open { t.ink_2 } else { t.muted })
                            .child(path.to_string()),
                    ),
            )
            .when(open, |d| d.child(self.config_view()))
    }

    /// The inline, read-only file content for the currently-open config path.
    fn config_view(&self) -> AnyElement {
        let t = &self.theme;
        let body = match &self.file_state {
            FileState::Loading => div()
                .text_size(t.fs_sm)
                .text_color(t.muted)
                .child("Reading…")
                .into_any_element(),
            FileState::Failed(err) => div()
                .text_size(t.fs_sm)
                .text_color(t.neg)
                .child(err.clone())
                .into_any_element(),
            FileState::Loaded(lines) => {
                let mut pre = v_flex().w_full();
                for line in lines {
                    // Empty lines still need height so blank gaps in the
                    // config survive the line-per-div rendering.
                    let text = if line.is_empty() {
                        " ".to_string()
                    } else {
                        line.clone()
                    };
                    pre = pre.child(
                        div()
                            .w_full()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.ink_2)
                            .child(text),
                    );
                }
                pre.into_any_element()
            }
            FileState::Idle => div().into_any_element(),
        };
        div()
            .px(t.sp3)
            .pb(t.sp2)
            .child(
                div()
                    .w_full()
                    .px(t.sp2)
                    .py(t.sp2)
                    .rounded(t.radius_sm)
                    .bg(t.panel_2)
                    .child(body),
            )
            .into_any_element()
    }

    fn overview_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        match &self.state {
            ScanState::Idle => {
                ui::empty_state(t, "Select a connection to scan").into_any_element()
            }
            ScanState::Scanning => v_flex()
                .px(t.sp3)
                .py(t.sp3)
                .child(div().text_color(t.muted).child("Scanning host…"))
                .into_any_element(),
            ScanState::Failed(err) => v_flex()
                .px(t.sp3)
                .py(t.sp3)
                .child(div().text_color(t.neg).child(err.clone()))
                .into_any_element(),
            ScanState::Loaded(ov) => {
                if ov.products.is_empty() {
                    return v_flex()
                        .px(t.sp3)
                        .py(t.sp3)
                        .child(div().text_color(t.muted).child("No web server detected"))
                        .into_any_element();
                }
                let active = ov
                    .products
                    .iter()
                    .find(|p| Some(p.kind) == self.active_kind)
                    .unwrap_or(&ov.products[0]);

                let mut col = v_flex();
                if ov.products.len() > 1 {
                    col = col.child(self.segmented(cx, &ov.products));
                }
                col = col.child(self.product_summary(cx, active));
                if !active.sites.is_empty() {
                    col = col
                        .child(ui::section_label(t, format!("SITES · {}", active.sites.len())));
                    col = col.children(active.sites.iter().map(|s| self.site_row(s)));
                }
                if !active.config_files.is_empty() {
                    col = col.child(ui::section_label(
                        t,
                        format!("CONFIG FILES · {}", active.config_files.len()),
                    ));
                    for (i, p) in active.config_files.iter().enumerate() {
                        col = col.child(self.file_row(cx, i, p));
                    }
                }
                col.into_any_element()
            }
        }
    }
}

impl Render for WebserverPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = cx.global::<Theme>().clone();
        let t = &self.theme;
        let meta = self.header_meta();

        let mut selector =
            v_flex().child(ui::section_label(t, format!("CONNECTIONS · {}", self.conns.len())));
        if self.conns.is_empty() {
            selector = selector.child(
                div()
                    .px(t.sp3)
                    .py(t.sp2)
                    .text_size(t.fs_sm)
                    .text_color(t.dim)
                    .child("No saved connections"),
            );
        } else {
            let rows: Vec<_> = (0..self.conns.len()).map(|i| self.conn_row(cx, i)).collect();
            selector = selector.children(rows);
        }

        let body = self.overview_body(cx);

        v_flex()
            .size_full()
            .child(ui::panel_header(t, "server", "WEBSERVER", meta))
            .child(
                div()
                    .id("ws-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(v_flex().child(selector).child(body)),
            )
    }
}

/// Single colour + label for a run state.
fn run_style(t: &Theme, st: WebServerRunState) -> (gpui::Hsla, &'static str) {
    match st {
        WebServerRunState::Active => (t.pos, "active"),
        WebServerRunState::Inactive => (t.neg, "inactive"),
        WebServerRunState::Unknown => (t.muted, "unknown"),
    }
}

/// Segmented-control display label per product.
fn kind_label(kind: WebServerKind) -> &'static str {
    match kind {
        WebServerKind::Nginx => "nginx",
        WebServerKind::Apache => "Apache",
        WebServerKind::Caddy => "Caddy",
    }
}

/// Stable element-id fragment per product.
fn kind_key(kind: WebServerKind) -> &'static str {
    match kind {
        WebServerKind::Nginx => "nginx",
        WebServerKind::Apache => "apache",
        WebServerKind::Caddy => "caddy",
    }
}

/// The reload command the chip copies. `binary` already reflects the host's
/// flavour (`apache2` vs `httpd`), so a per-binary `systemctl reload` is the
/// most accurate single command to hand the user.
fn reload_cmd(svc: &SvcRow) -> String {
    format!("systemctl reload {}", svc.binary)
}

// ── Background collection (off the render path) ──────────────────────

/// Read one config file for `kind`, capped to [`MAX_CONFIG_LINES`]. Blocks on
/// the remote shell, so this runs on the background executor. Goes through the
/// pier-core readers, which validate the path against the product's config root
/// and run with sudo when needed.
fn read_config(session: &SshSession, kind: WebServerKind, path: &str) -> Result<Vec<String>, String> {
    let text = match kind {
        WebServerKind::Nginx => nginx::read_file_blocking(session, path),
        WebServerKind::Apache | WebServerKind::Caddy => {
            web_server::read_file_blocking(session, kind, path)
        }
    }
    .map_err(|e| e.to_string())?;

    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if lines.len() > MAX_CONFIG_LINES {
        lines.truncate(MAX_CONFIG_LINES);
        lines.push(format!("… (truncated at {MAX_CONFIG_LINES} lines)"));
    }
    Ok(lines)
}

/// Connect and gather the web-server overview, grouped per detected product.
/// All calls here block on the network / remote shell, so this runs on the
/// background executor. The live session is returned so the panel can read
/// config files on demand.
fn scan(cfg: &SshConfig) -> Result<(SshSession, WebOverview), String> {
    let session = data::connect_blocking(cfg)?;
    let detection = web_server::detect_blocking(&session).map_err(|e| e.to_string())?;

    let mut products = Vec::new();
    for info in &detection.detected {
        let svc = SvcRow {
            binary: info.binary.clone(),
            version: info.version.clone(),
            running: info.running,
            config_root: info.config_root.clone(),
        };
        let mut sites = Vec::new();
        let mut config_files = Vec::new();

        match info.kind {
            WebServerKind::Nginx => {
                // nginx has its own dedicated layout/read path.
                if let Ok(layout) = nginx::list_layout_blocking(&session) {
                    for f in layout.files.iter().take(50) {
                        config_files.push(f.path.clone());
                        if let Ok(src) = nginx::read_file_blocking(&session, &f.path) {
                            extract_nginx_sites(&src, &f.path, &mut sites);
                        }
                    }
                }
            }
            kind @ (WebServerKind::Apache | WebServerKind::Caddy) => {
                if let Ok(layout) = web_server::list_layout_blocking(&session, kind) {
                    for f in layout.files.iter().take(50) {
                        config_files.push(f.path.clone());
                        // Only Apache has a structured parser here; Caddy
                        // surfaces its config-file paths only this iteration.
                        if kind == WebServerKind::Apache {
                            if let Ok(src) =
                                web_server::read_file_blocking(&session, kind, &f.path)
                            {
                                extract_apache_sites(&src, &f.path, &mut sites);
                            }
                        }
                    }
                }
            }
        }

        products.push(ProductView {
            kind: info.kind,
            svc,
            sites,
            config_files,
        });
    }

    Ok((session, WebOverview { products }))
}

fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Extract the port portion from a `listen` / `<VirtualHost>` address token
/// like `80`, `[::]:80`, `0.0.0.0:8080`, or `*:443`.
fn addr_port(token: &str) -> String {
    let token = token.trim();
    match token.rfind(':') {
        Some(idx) => token[idx + 1..].to_string(),
        None => token.to_string(),
    }
}

fn extract_nginx_sites(src: &str, file: &str, out: &mut Vec<SiteRow>) {
    let parsed = nginx::parse(src);
    walk_nginx(&parsed.nodes, file, out);
}

fn walk_nginx(nodes: &[NginxNode], file: &str, out: &mut Vec<SiteRow>) {
    for node in nodes {
        let NginxNode::Directive(d) = node else {
            continue;
        };
        if d.name.eq_ignore_ascii_case("server") {
            if let Some(block) = &d.block {
                out.push(nginx_server_to_site(block, file));
            }
        } else if let Some(block) = &d.block {
            // Recurse into wrappers like `http { ... }`.
            walk_nginx(block, file, out);
        }
    }
}

fn nginx_server_to_site(block: &[NginxNode], file: &str) -> SiteRow {
    let mut domains: Vec<String> = Vec::new();
    let mut ports: Vec<String> = Vec::new();
    let mut root = String::new();
    for node in block {
        let NginxNode::Directive(d) = node else {
            continue;
        };
        match d.name.to_ascii_lowercase().as_str() {
            "server_name" => domains.extend(d.args.iter().cloned()),
            "listen" => {
                if let Some(a) = d.args.first() {
                    let p = addr_port(a);
                    if !ports.contains(&p) {
                        ports.push(p);
                    }
                }
            }
            "root" => {
                if root.is_empty() {
                    if let Some(a) = d.args.first() {
                        root = a.clone();
                    }
                }
            }
            _ => {}
        }
    }
    SiteRow {
        domain: if domains.is_empty() { "_".to_string() } else { domains.join(" ") },
        port: if ports.is_empty() { "—".to_string() } else { ports.join(",") },
        root: if root.is_empty() { "—".to_string() } else { root },
        file: basename(file),
    }
}

fn extract_apache_sites(src: &str, file: &str, out: &mut Vec<SiteRow>) {
    let parsed = apache::parse(src);
    walk_apache(&parsed.nodes, file, out);
}

fn walk_apache(nodes: &[ApacheNode], file: &str, out: &mut Vec<SiteRow>) {
    for node in nodes {
        let ApacheNode::Directive(d) = node else {
            continue;
        };
        if d.name.eq_ignore_ascii_case("VirtualHost") {
            out.push(apache_vhost_to_site(d, file));
        } else if let Some(section) = &d.section {
            walk_apache(section, file, out);
        }
    }
}

fn apache_vhost_to_site(d: &apache::ApacheDirective, file: &str) -> SiteRow {
    let port = d
        .args
        .first()
        .map(|a| addr_port(a))
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "—".to_string());
    let mut domains: Vec<String> = Vec::new();
    let mut root = String::new();
    if let Some(section) = &d.section {
        for node in section {
            let ApacheNode::Directive(sd) = node else {
                continue;
            };
            match sd.name.to_ascii_lowercase().as_str() {
                "servername" => {
                    if let Some(a) = sd.args.first() {
                        domains.push(a.clone());
                    }
                }
                "serveralias" => domains.extend(sd.args.iter().cloned()),
                "documentroot" => {
                    if root.is_empty() {
                        if let Some(a) = sd.args.first() {
                            root = a.clone();
                        }
                    }
                }
                _ => {}
            }
        }
    }
    SiteRow {
        domain: if domains.is_empty() { "_".to_string() } else { domains.join(" ") },
        port,
        root: if root.is_empty() { "—".to_string() } else { root },
        file: basename(file),
    }
}
