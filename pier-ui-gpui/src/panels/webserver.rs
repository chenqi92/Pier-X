// Web server panel — nginx/apache/caddy overview for a selected host.
//
// Renders a connection selector backed by data::connections_raw(). Picking a
// connection runs data::connect_blocking() + the pier-core web-server probes on
// a background task and stores an owned snapshot back on the View — the render
// path only paints from that cache.
//
// Read-only this iteration: detected services + run state, parsed site/vhost
// rows (domain / port / root), and the discovered config-file paths. No
// start/stop or config editing.

use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, MouseButton, MouseDownEvent, SharedString, Window};
use gpui_component::{h_flex, v_flex};

use pier_core::services::apache::{self, ApacheNode};
use pier_core::services::nginx::{self, NginxNode};
use pier_core::services::web_server::{self, WebServerKind, WebServerRunState};
use pier_core::ssh::SshConfig;

use crate::data;
use crate::theme::Theme;
use crate::ui;

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

/// The owned snapshot the panel paints once a scan finishes.
struct WebOverview {
    services: Vec<SvcRow>,
    sites: Vec<SiteRow>,
    config_files: Vec<String>,
}

/// Lifecycle of the right-hand overview for the selected connection.
enum ScanState {
    Idle,
    Scanning,
    Loaded(WebOverview),
    Failed(String),
}

pub struct WebserverPanel {
    theme: Theme,
    conns: Vec<SshConfig>,
    /// Index into `conns` for the connection being shown, if any.
    selected: Option<usize>,
    state: ScanState,
}

impl WebserverPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            theme: Theme::dark(),
            conns: data::connections_raw(),
            selected: None,
            state: ScanState::Idle,
        }
    }

    /// Connect to `conns[idx]` and collect the web-server overview off the
    /// render path, then store it back on the View.
    fn start_scan(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(cfg) = self.conns.get(idx).cloned() else {
            return;
        };
        self.selected = Some(idx);
        self.state = ScanState::Scanning;
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
                this.state = match result {
                    Ok(overview) => ScanState::Loaded(overview),
                    Err(err) => ScanState::Failed(err),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn header_meta(&self) -> SharedString {
        match &self.state {
            ScanState::Scanning => "scanning…".into(),
            ScanState::Loaded(ov) => {
                if !ov.services.is_empty() {
                    ov.services
                        .iter()
                        .map(|s| s.binary.as_str())
                        .collect::<Vec<_>>()
                        .join(" · ")
                        .into()
                } else if !ov.sites.is_empty() {
                    format!("{} sites", ov.sites.len()).into()
                } else {
                    "no web server".into()
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

    fn svc_row(&self, s: &SvcRow) -> impl IntoElement {
        let t = &self.theme;
        let (color, label) = run_style(t, s.running);
        v_flex()
            .child(
                h_flex()
                    .items_center()
                    .gap(t.sp2)
                    .px(t.sp3)
                    .py(t.sp2)
                    .child(ui::status_dot(color))
                    .child(div().text_color(t.ink).child(s.binary.clone()))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .font_family(t.mono.clone())
                            .text_size(t.fs_sm)
                            .text_color(t.muted)
                            .child(s.version.clone()),
                    )
                    .child(div().text_size(t.fs_sm).text_color(color).child(label)),
            )
            .child(ui::info_row(t, "config", s.config_root.clone()))
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

    fn file_row(&self, path: &str) -> impl IntoElement {
        let t = &self.theme;
        div()
            .px(t.sp3)
            .py(t.sp1)
            .overflow_hidden()
            .font_family(t.mono.clone())
            .text_size(t.fs_sm)
            .text_color(t.muted)
            .child(path.to_string())
    }

    fn overview_body(&self) -> AnyElement {
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
                if ov.services.is_empty() && ov.sites.is_empty() {
                    return v_flex()
                        .px(t.sp3)
                        .py(t.sp3)
                        .child(div().text_color(t.muted).child("No web server detected"))
                        .into_any_element();
                }
                let mut col = v_flex();
                if !ov.services.is_empty() {
                    col = col.child(ui::section_label(t, format!("SERVICES · {}", ov.services.len())));
                    col = col.children(ov.services.iter().map(|s| self.svc_row(s)));
                }
                if !ov.sites.is_empty() {
                    col = col.child(ui::section_label(t, format!("SITES · {}", ov.sites.len())));
                    col = col.children(ov.sites.iter().map(|s| self.site_row(s)));
                }
                if !ov.config_files.is_empty() {
                    col = col.child(ui::section_label(
                        t,
                        format!("CONFIG FILES · {}", ov.config_files.len()),
                    ));
                    col = col.children(ov.config_files.iter().map(|p| self.file_row(p)));
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

        let body = self.overview_body();

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

// ── Background collection (off the render path) ──────────────────────

/// Connect and gather the web-server overview. All calls here block on the
/// network / remote shell, so this runs on the background executor.
fn scan(cfg: &SshConfig) -> Result<WebOverview, String> {
    let session = data::connect_blocking(cfg)?;
    let detection = web_server::detect_blocking(&session).map_err(|e| e.to_string())?;

    let mut services = Vec::new();
    let mut sites = Vec::new();
    let mut config_files = Vec::new();

    for info in &detection.detected {
        services.push(SvcRow {
            binary: info.binary.clone(),
            version: info.version.clone(),
            running: info.running,
            config_root: info.config_root.clone(),
        });

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
    }

    Ok(WebOverview {
        services,
        sites,
        config_files,
    })
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
