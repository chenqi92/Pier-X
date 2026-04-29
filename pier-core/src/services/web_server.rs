//! Web-server presence detection + generic validate/reload across
//! nginx / apache / caddy.
//!
//! `detect` runs a single batched probe per host that asks "which web
//! server binaries are installed, what version, and what's the config
//! root". The unified `WebServerPanel` consumes this to decide whether
//! to render the rich nginx panel, an apache placeholder, a caddy
//! placeholder, or a "no web server detected" empty state.
//!
//! The `validate` / `reload` helpers exist for the apache/caddy
//! placeholder buttons. nginx still goes through `services::nginx::*`
//! because its panel needs the richer save-and-reload pipeline.

// The struct/enum/function bodies in this module are deliberately
// thin (parse / shell-out / serialise) and the rationale lives in the
// module-level docs above + comments next to the shell commands.
// Forcing per-field doc comments would add noise without actually
// teaching the reader anything new.
#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

use crate::ssh::error::Result;
use crate::ssh::SshSession;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WebServerKind {
    Nginx,
    Apache,
    Caddy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebServerInfo {
    pub kind: WebServerKind,
    /// "nginx", "apache2", "httpd", or "caddy" — the actual binary the
    /// host has installed (apache reports its packaging-flavored name).
    pub binary: String,
    /// Free-form version string trimmed from `<binary> -v` / `caddy version`.
    pub version: String,
    /// Conventional config root for this product on this distro.
    pub config_root: String,
    /// Loaded-modules summary — nginx uses `nginx -V`, apache uses
    /// `apachectl -M`, caddy uses `caddy list-modules`. Truncated to a
    /// reasonable preview; the panel renders it inside a <details>.
    pub modules_summary: String,
    /// Whether the daemon is currently active per `systemctl is-active`.
    /// Best-effort — `Unknown` when systemctl isn't on the host.
    pub running: WebServerRunState,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WebServerRunState {
    Active,
    Inactive,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebServerDetection {
    pub detected: Vec<WebServerInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebServerActionResult {
    pub ok: bool,
    pub exit_code: i32,
    pub output: String,
}

// ── Detection ───────────────────────────────────────────────────────

pub async fn detect(session: &SshSession) -> Result<WebServerDetection> {
    let mut detected = Vec::new();

    // Order matters for the panel's default segment selection: nginx
    // first when ambiguity exists.
    if let Some(info) = probe_nginx(session).await {
        detected.push(info);
    }
    if let Some(info) = probe_apache(session).await {
        detected.push(info);
    }
    if let Some(info) = probe_caddy(session).await {
        detected.push(info);
    }

    Ok(WebServerDetection { detected })
}

pub fn detect_blocking(session: &SshSession) -> Result<WebServerDetection> {
    crate::ssh::runtime::shared().block_on(detect(session))
}

async fn probe_nginx(session: &SshSession) -> Option<WebServerInfo> {
    let (code, out) = session
        .exec_command("command -v nginx >/dev/null 2>&1 && nginx -v 2>&1")
        .await
        .ok()?;
    if code != 0 {
        return None;
    }
    let version = trim_version_line(&out);
    let modules_summary = session
        .exec_command("nginx -V 2>&1 | head -c 4096")
        .await
        .map(|(_, o)| o.trim().to_string())
        .unwrap_or_default();
    let running = systemctl_state(session, "nginx").await;
    Some(WebServerInfo {
        kind: WebServerKind::Nginx,
        binary: "nginx".to_string(),
        version,
        config_root: "/etc/nginx".to_string(),
        modules_summary,
        running,
    })
}

async fn probe_apache(session: &SshSession) -> Option<WebServerInfo> {
    // Debian/Ubuntu ships `apache2`, RHEL/Fedora ships `httpd`. Probe
    // both and pick the first that responds.
    for binary in ["apache2", "httpd"] {
        let (code, out) = session
            .exec_command(&format!(
                "command -v {binary} >/dev/null 2>&1 && {binary} -v 2>&1"
            ))
            .await
            .ok()?;
        if code != 0 {
            continue;
        }
        let version = trim_version_line(&out);
        // `apachectl -M` requires a working config to load modules; on a
        // half-configured box it can fail loudly. Cap the body length
        // and tolerate non-zero exit codes — the user just wants a
        // module list overview.
        let modules_summary = session
            .exec_command(&format!(
                "{binary} -M 2>&1 | head -c 4096 || apachectl -M 2>&1 | head -c 4096"
            ))
            .await
            .map(|(_, o)| o.trim().to_string())
            .unwrap_or_default();
        let config_root = if binary == "apache2" {
            "/etc/apache2"
        } else {
            "/etc/httpd"
        };
        let running = systemctl_state(session, binary).await;
        return Some(WebServerInfo {
            kind: WebServerKind::Apache,
            binary: binary.to_string(),
            version,
            config_root: config_root.to_string(),
            modules_summary,
            running,
        });
    }
    None
}

async fn probe_caddy(session: &SshSession) -> Option<WebServerInfo> {
    let (code, out) = session
        .exec_command("command -v caddy >/dev/null 2>&1 && caddy version 2>&1")
        .await
        .ok()?;
    if code != 0 {
        return None;
    }
    let version = trim_version_line(&out);
    let modules_summary = session
        .exec_command("caddy list-modules 2>&1 | head -c 4096")
        .await
        .map(|(_, o)| o.trim().to_string())
        .unwrap_or_default();
    let running = systemctl_state(session, "caddy").await;
    Some(WebServerInfo {
        kind: WebServerKind::Caddy,
        binary: "caddy".to_string(),
        version,
        config_root: "/etc/caddy".to_string(),
        modules_summary,
        running,
    })
}

async fn systemctl_state(session: &SshSession, unit: &str) -> WebServerRunState {
    // `systemctl is-active` exits 0 for active, 3 for inactive, and
    // some other code (or fails the shell) when systemctl is absent.
    match session
        .exec_command(&format!(
            "command -v systemctl >/dev/null 2>&1 && systemctl is-active {unit} 2>&1"
        ))
        .await
    {
        Ok((0, _)) => WebServerRunState::Active,
        Ok((_, out)) => {
            let s = out.trim();
            if s == "inactive" || s == "failed" || s == "unknown" {
                WebServerRunState::Inactive
            } else {
                WebServerRunState::Unknown
            }
        }
        Err(_) => WebServerRunState::Unknown,
    }
}

fn trim_version_line(out: &str) -> String {
    out.lines().next().unwrap_or("").trim().to_string()
}

// ── Validate / Reload (apache + caddy placeholder) ──────────────────

pub async fn validate(
    session: &SshSession,
    kind: WebServerKind,
) -> Result<WebServerActionResult> {
    let cmd = match kind {
        WebServerKind::Nginx => "nginx -t 2>&1",
        WebServerKind::Apache => {
            "sh -c 'if command -v apachectl >/dev/null 2>&1; then \
                apachectl configtest 2>&1; \
             elif command -v apache2ctl >/dev/null 2>&1; then \
                apache2ctl configtest 2>&1; \
             elif command -v httpd >/dev/null 2>&1; then \
                httpd -t 2>&1; \
             else \
                echo \"no apache control binary found\" >&2; exit 127; \
             fi'"
        }
        WebServerKind::Caddy => {
            // `caddy validate` requires --config to be specific; the
            // default /etc/caddy/Caddyfile covers the common case.
            "sh -c 'caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile 2>&1'"
        }
    };
    run_with_sudo(session, cmd).await
}

pub fn validate_blocking(
    session: &SshSession,
    kind: WebServerKind,
) -> Result<WebServerActionResult> {
    crate::ssh::runtime::shared().block_on(validate(session, kind))
}

pub async fn reload(
    session: &SshSession,
    kind: WebServerKind,
) -> Result<WebServerActionResult> {
    let cmd = match kind {
        WebServerKind::Nginx => {
            "sh -c 'if command -v systemctl >/dev/null 2>&1; then \
                systemctl reload nginx 2>&1; \
             else \
                nginx -s reload 2>&1; \
             fi'"
        }
        WebServerKind::Apache => {
            "sh -c 'if command -v systemctl >/dev/null 2>&1; then \
                (systemctl reload apache2 2>&1 || systemctl reload httpd 2>&1); \
             elif command -v apachectl >/dev/null 2>&1; then \
                apachectl graceful 2>&1; \
             else \
                apache2ctl graceful 2>&1; \
             fi'"
        }
        WebServerKind::Caddy => {
            "sh -c 'if command -v systemctl >/dev/null 2>&1; then \
                systemctl reload caddy 2>&1; \
             else \
                caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile 2>&1; \
             fi'"
        }
    };
    run_with_sudo(session, cmd).await
}

pub fn reload_blocking(
    session: &SshSession,
    kind: WebServerKind,
) -> Result<WebServerActionResult> {
    crate::ssh::runtime::shared().block_on(reload(session, kind))
}

/// Run a deeper static-analysis pass per server kind. Apache uses
/// `apachectl -S` to dump the resolved vhost map; this surfaces
/// duplicate `ServerName`s and overlapping `_default_:port` listens
/// that `configtest` happily accepts. Caddy's `caddy adapt
/// --pretty` re-reads the canonical Caddyfile and prints any
/// adapter warnings to stderr alongside the JSON output.
///
/// The result is purely advisory — not used as a save-gate. Panels
/// that want to surface lint hints call this *after* a successful
/// `validate`.
pub async fn lint_hints(
    session: &SshSession,
    kind: WebServerKind,
) -> Result<WebServerActionResult> {
    let cmd = match kind {
        WebServerKind::Nginx => {
            // `nginx -T 2>&1 | head -200` would dump the merged config
            // — too noisy. Stick to `nginx -t -q` which only prints
            // warnings (and is silent on a clean config).
            "sh -c 'nginx -t -q 2>&1 || true'"
        }
        WebServerKind::Apache => {
            "sh -c 'if command -v apachectl >/dev/null 2>&1; then \
                apachectl -S 2>&1; \
             elif command -v apache2ctl >/dev/null 2>&1; then \
                apache2ctl -S 2>&1; \
             elif command -v httpd >/dev/null 2>&1; then \
                httpd -S 2>&1; \
             else \
                echo \"no apache control binary found\" >&2; exit 127; \
             fi'"
        }
        WebServerKind::Caddy => {
            // `caddy adapt` writes the JSON to stdout and warnings to
            // stderr. We merge both so the panel sees everything in
            // one stream; the panel filters to lines that start with
            // `WARN:` / `ERROR:` for display.
            "sh -c 'caddy adapt --config /etc/caddy/Caddyfile --pretty 2>&1 >/dev/null || true'"
        }
    };
    run_with_sudo(session, cmd).await
}

pub fn lint_hints_blocking(
    session: &SshSession,
    kind: WebServerKind,
) -> Result<WebServerActionResult> {
    crate::ssh::runtime::shared().block_on(lint_hints(session, kind))
}

async fn run_with_sudo(session: &SshSession, cmd: &str) -> Result<WebServerActionResult> {
    let is_root = match session.exec_command("id -u").await {
        Ok((0, stdout)) => stdout.trim() == "0",
        _ => false,
    };
    let prefix = if is_root { "" } else { "sudo -n " };
    let (code, out) = session.exec_command(&format!("{prefix}{cmd}")).await?;
    Ok(WebServerActionResult {
        ok: code == 0,
        exit_code: code,
        output: out,
    })
}

// ── Layout / read / save (apache + caddy raw editor) ────────────────
//
// nginx has its own rich panel routed through `services::nginx`. The
// helpers below cover apache + caddy: enough to list config files,
// open them in a textarea, and run the standard backup → write →
// validate → restore-on-fail → reload pipeline. Structured editing
// for those products is on the roadmap; this is the credible
// interim stop.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum WebServerFileKind {
    /// Top-level config (apache2.conf / httpd.conf / Caddyfile).
    Main,
    /// Distro `conf.d` style include.
    ConfD,
    /// Apache `sites-available/*`. `enabled` reflects whether
    /// `sites-enabled/<name>` symlinks to it.
    SiteAvailable {
        enabled: bool,
    },
    /// Apache `mods-available/*` / `conf-available/*`.
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebServerFile {
    pub path: String,
    pub label: String,
    pub kind: WebServerFileKind,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebServerLayout {
    pub kind: WebServerKind,
    pub binary: String,
    pub version: String,
    pub config_root: String,
    pub installed: bool,
    pub is_root: bool,
    pub files: Vec<WebServerFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebServerSaveResult {
    pub validate: WebServerActionResult,
    pub reloaded: bool,
    pub reload_output: String,
    /// `true` when the original file is in a clean state — true on
    /// success AND on a validation-fail-then-restore path. False only
    /// when the restore step itself failed.
    pub restored: bool,
    pub restore_error: Option<String>,
    pub backup_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebServerBatchSaveEntry {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebServerBatchSaveResult {
    /// Per-file backup paths, paired by index with the input entries.
    /// Always present — even on validate-fail we keep the backups
    /// around until restore confirms.
    pub backup_paths: Vec<String>,
    /// Single `apachectl configtest` / `nginx -t` / `caddy validate`
    /// run after all writes land.
    pub validate: WebServerActionResult,
    /// True when the daemon was reloaded (only happens on validate.ok).
    pub reloaded: bool,
    pub reload_output: String,
    /// True when on-disk state is clean — true on save+reload path AND
    /// on validate-fail-then-restore path. False only if any restore
    /// step itself failed.
    pub restored: bool,
    /// Per-file restore failure messages, paired by index. Empty
    /// strings for entries that restored cleanly or didn't need to.
    pub restore_errors: Vec<String>,
}

pub async fn list_layout(
    session: &SshSession,
    kind: WebServerKind,
) -> Result<WebServerLayout> {
    let info = match kind {
        WebServerKind::Nginx => probe_nginx(session).await,
        WebServerKind::Apache => probe_apache(session).await,
        WebServerKind::Caddy => probe_caddy(session).await,
    };

    let is_root = match session.exec_command("id -u").await {
        Ok((0, stdout)) => stdout.trim() == "0",
        _ => false,
    };

    let Some(info) = info else {
        return Ok(WebServerLayout {
            kind,
            binary: String::new(),
            version: String::new(),
            config_root: default_config_root(kind).to_string(),
            installed: false,
            is_root,
            files: Vec::new(),
        });
    };

    let mut files = Vec::new();
    match kind {
        WebServerKind::Nginx => {
            // Reserved for nginx parity routing if a future caller
            // ever asks for a nginx layout via this entry point —
            // today the nginx panel uses `services::nginx::list_layout`.
        }
        WebServerKind::Apache => {
            collect_apache_files(session, &info.binary, &info.config_root, &mut files).await;
        }
        WebServerKind::Caddy => {
            collect_caddy_files(session, &info.config_root, &mut files).await;
        }
    }

    Ok(WebServerLayout {
        kind,
        binary: info.binary,
        version: info.version,
        config_root: info.config_root,
        installed: true,
        is_root,
        files,
    })
}

pub fn list_layout_blocking(
    session: &SshSession,
    kind: WebServerKind,
) -> Result<WebServerLayout> {
    crate::ssh::runtime::shared().block_on(list_layout(session, kind))
}

fn default_config_root(kind: WebServerKind) -> &'static str {
    match kind {
        WebServerKind::Nginx => "/etc/nginx",
        WebServerKind::Apache => "/etc/apache2",
        WebServerKind::Caddy => "/etc/caddy",
    }
}

async fn collect_apache_files(
    session: &SshSession,
    binary: &str,
    config_root: &str,
    out: &mut Vec<WebServerFile>,
) {
    // Main config first.
    let main_candidates: &[&str] = if binary == "apache2" {
        &["/etc/apache2/apache2.conf"]
    } else {
        &["/etc/httpd/conf/httpd.conf", "/etc/httpd/httpd.conf"]
    };
    for path in main_candidates {
        if let Some(file) = stat_remote(session, path, "main", WebServerFileKind::Main).await {
            out.push(file);
            break;
        }
    }

    // ports.conf is non-main but high-traffic.
    if binary == "apache2" {
        if let Some(file) = stat_remote(
            session,
            "/etc/apache2/ports.conf",
            "ports.conf",
            WebServerFileKind::ConfD,
        )
        .await
        {
            out.push(file);
        }
    }

    // conf.d (RHEL) / conf-available (Debian).
    let conf_dirs: &[&str] = if binary == "apache2" {
        &["/etc/apache2/conf-available", "/etc/apache2/conf-enabled"]
    } else {
        &["/etc/httpd/conf.d"]
    };
    for dir in conf_dirs {
        list_dir_remote(session, dir, "*.conf", WebServerFileKind::ConfD, out).await;
    }

    // sites-available + which ones are enabled (Debian only).
    if binary == "apache2" {
        let enabled_set = list_apache_enabled_set(session).await;
        let avail_dir = format!("{config_root}/sites-available");
        let listing = exec_listing(session, &avail_dir, "*").await;
        for path in listing {
            let label = path
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_string();
            let enabled = enabled_set.contains(&label);
            let size_bytes = stat_size(session, &path).await;
            out.push(WebServerFile {
                path,
                label,
                kind: WebServerFileKind::SiteAvailable { enabled },
                size_bytes,
            });
        }
    }
}

async fn list_apache_enabled_set(session: &SshSession) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let cmd = "ls -1 /etc/apache2/sites-enabled 2>/dev/null";
    if let Ok((0, out)) = session.exec_command(cmd).await {
        for name in out.lines() {
            let name = name.trim();
            if !name.is_empty() {
                set.insert(name.to_string());
            }
        }
    }
    set
}

async fn collect_caddy_files(
    session: &SshSession,
    config_root: &str,
    out: &mut Vec<WebServerFile>,
) {
    // Main Caddyfile.
    let main_path = format!("{config_root}/Caddyfile");
    if let Some(file) = stat_remote(session, &main_path, "Caddyfile", WebServerFileKind::Main).await {
        out.push(file);
    }
    // /etc/caddy/conf.d/* — convention for split configs that the
    // main Caddyfile pulls in via `import`.
    let conf_dir = format!("{config_root}/conf.d");
    list_dir_remote(session, &conf_dir, "*", WebServerFileKind::ConfD, out).await;
}

async fn stat_remote(
    session: &SshSession,
    path: &str,
    label: &str,
    kind: WebServerFileKind,
) -> Option<WebServerFile> {
    let q = crate::services::nginx::shell_single_quote(path);
    // `stat -c %s` on linux, `stat -f %z` on macos. We're SSHing into
    // mostly-linux servers so the GNU form is fine; if it fails we
    // fall through to test+wc.
    let cmd = format!("test -f {q} && stat -c %s {q} 2>/dev/null");
    let (code, out) = session.exec_command(&cmd).await.ok()?;
    if code != 0 {
        return None;
    }
    let size_bytes = out.trim().parse::<u64>().unwrap_or(0);
    Some(WebServerFile {
        path: path.to_string(),
        label: label.to_string(),
        kind,
        size_bytes,
    })
}

async fn stat_size(session: &SshSession, path: &str) -> u64 {
    let q = crate::services::nginx::shell_single_quote(path);
    let cmd = format!("stat -c %s {q} 2>/dev/null");
    if let Ok((0, out)) = session.exec_command(&cmd).await {
        return out.trim().parse::<u64>().unwrap_or(0);
    }
    0
}

async fn list_dir_remote(
    session: &SshSession,
    dir: &str,
    glob: &str,
    kind: WebServerFileKind,
    out: &mut Vec<WebServerFile>,
) {
    let q_dir = crate::services::nginx::shell_single_quote(dir);
    // -maxdepth 1 keeps it shallow; we don't recurse into nested
    // directories. -name uses fnmatch globbing.
    let cmd = format!(
        "find {q_dir} -maxdepth 1 -type f -name {glob} 2>/dev/null | sort",
        glob = crate::services::nginx::shell_single_quote(glob),
    );
    let Ok((0, listing)) = session.exec_command(&cmd).await else {
        return;
    };
    for path in listing.lines() {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        let label = path.rsplit('/').next().unwrap_or("").to_string();
        let size_bytes = stat_size(session, path).await;
        out.push(WebServerFile {
            path: path.to_string(),
            label,
            kind: kind.clone(),
            size_bytes,
        });
    }
}

async fn exec_listing(session: &SshSession, dir: &str, glob: &str) -> Vec<String> {
    let q_dir = crate::services::nginx::shell_single_quote(dir);
    let cmd = format!(
        "find {q_dir} -maxdepth 1 -type f -name {glob} 2>/dev/null | sort",
        glob = crate::services::nginx::shell_single_quote(glob),
    );
    match session.exec_command(&cmd).await {
        Ok((0, listing)) => listing
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

pub async fn read_file(
    session: &SshSession,
    kind: WebServerKind,
    path: &str,
) -> Result<String> {
    if !is_path_under_config_root(kind, path) {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "refusing to read {path}: must live under {}",
            default_config_root(kind)
        )));
    }
    let cmd = format!(
        "cat {} 2>&1",
        crate::services::nginx::shell_single_quote(path)
    );
    let (code, out) = session.exec_command(&cmd).await?;
    if code != 0 {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "read {path} failed (exit {code}): {}",
            out.trim()
        )));
    }
    Ok(out)
}

pub fn read_file_blocking(
    session: &SshSession,
    kind: WebServerKind,
    path: &str,
) -> Result<String> {
    crate::ssh::runtime::shared().block_on(read_file(session, kind, path))
}

/// Cap backups per source file. After a successful save we keep at
/// most this many `.pier-bak.<ts>` siblings; older ones are removed
/// so a noisy edit cycle doesn't accumulate hundreds of stale files
/// next to a vhost. Picked at 10 because:
///  - generous enough that you can roll back ~10 edits worth of work
///  - small enough that it's painless even on tiny `/etc` partitions
const BACKUP_RETENTION: usize = 10;

/// Trim `<path>.pier-bak.*` siblings down to the most-recent
/// `BACKUP_RETENTION` entries. Best-effort: a removal failure is
/// logged via the shell's exit code but never propagates as a save
/// error, since the user's primary save already succeeded.
async fn trim_old_backups(session: &SshSession, prefix: &str, path: &str) {
    let q_path = crate::services::nginx::shell_single_quote(path);
    // `ls -1` then `sort -r` (lexical) is good enough: backup names are
    // `<path>.pier-bak.<unix-secs>[.<idx>]`, all the same prefix, so
    // a reverse string sort puts the newest first. We keep the top N
    // and `rm -f` the rest. `2>/dev/null` swallows the no-match case.
    let inner = format!(
        "ls -1 {q_path}.pier-bak.* 2>/dev/null | sort -r | tail -n +{} | xargs -r rm -f",
        BACKUP_RETENTION + 1,
    );
    let cmd = format!(
        "{prefix}sh -c {}",
        crate::services::nginx::shell_single_quote(&inner)
    );
    let _ = session.exec_command(&cmd).await;
}

fn is_path_under_config_root(kind: WebServerKind, path: &str) -> bool {
    if path.split('/').any(|seg| seg == "..") {
        return false;
    }
    let allowed: &[&str] = match kind {
        WebServerKind::Nginx => &["/etc/nginx/"],
        WebServerKind::Apache => &["/etc/apache2/", "/etc/httpd/"],
        WebServerKind::Caddy => &["/etc/caddy/"],
    };
    allowed.iter().any(|root| path.starts_with(root))
}

pub async fn save_file_validate_reload(
    session: &SshSession,
    kind: WebServerKind,
    path: &str,
    content: &str,
) -> Result<WebServerSaveResult> {
    if !is_path_under_config_root(kind, path) {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "refusing to write {path}: must live under {}",
            default_config_root(kind)
        )));
    }

    let is_root = match session.exec_command("id -u").await {
        Ok((0, stdout)) => stdout.trim() == "0",
        _ => false,
    };
    let prefix = if is_root { "" } else { "sudo -n " };

    let ts = match session.exec_command("date +%s").await {
        Ok((0, out)) => out.trim().to_string(),
        _ => "0".to_string(),
    };
    let backup_path = format!("{path}.pier-bak.{ts}");

    let q_path = crate::services::nginx::shell_single_quote(path);
    let q_backup = crate::services::nginx::shell_single_quote(&backup_path);

    // 1) Backup. cp -p preserves mode/owner so a later restore doesn't
    //    ratchet permissions.
    let backup_cmd = format!("{prefix}cp -p {q_path} {q_backup}");
    let (backup_code, backup_out) = session.exec_command(&backup_cmd).await?;
    if backup_code != 0 {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "backup {path} → {backup_path} failed: {}",
            backup_out.trim()
        )));
    }

    // 2) Atomic write via base64 → tmp → mv.
    use std::io::Write;
    let mut encoded = String::new();
    {
        let mut writer = crate::services::nginx::base64_writer(&mut encoded);
        writer.write_all(content.as_bytes()).ok();
        writer.flush().ok();
    }
    let tmp_path = format!("/tmp/pier-webserver-{ts}.conf");
    let q_tmp = crate::services::nginx::shell_single_quote(&tmp_path);
    let q_b64 = crate::services::nginx::shell_single_quote(&encoded);
    let inner = format!(
        "echo {q_b64} | base64 -d > {q_tmp} && chmod --reference={q_path} {q_tmp} 2>/dev/null || true; mv {q_tmp} {q_path}"
    );
    let write_cmd = format!(
        "{prefix}sh -c {}",
        crate::services::nginx::shell_single_quote(&inner)
    );
    let (write_code, write_out) = session.exec_command(&write_cmd).await?;
    if write_code != 0 {
        let _ = session
            .exec_command(&format!("{prefix}mv {q_backup} {q_path}"))
            .await;
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "write {path} failed: {}",
            write_out.trim()
        )));
    }

    // 3) Validate.
    let validate = validate(session, kind).await?;

    if !validate.ok {
        // 4) Restore on validation failure.
        let restore_cmd = format!("{prefix}mv {q_backup} {q_path}");
        let (rc, rout) = session
            .exec_command(&restore_cmd)
            .await
            .unwrap_or((-1, String::new()));
        let restored = rc == 0;
        return Ok(WebServerSaveResult {
            validate,
            reloaded: false,
            reload_output: String::new(),
            restored,
            restore_error: if restored { None } else { Some(rout) },
            backup_path,
        });
    }

    // 5) Reload.
    let reload_result = reload(session, kind).await?;

    // Trim aged backups now that we know the new content is good and
    // reloaded — keeping the latest backup intact in case the user
    // hits "restore" through the file tree.
    trim_old_backups(session, prefix, path).await;

    Ok(WebServerSaveResult {
        validate,
        reloaded: reload_result.ok,
        reload_output: reload_result.output,
        restored: true,
        restore_error: None,
        backup_path,
    })
}

pub fn save_file_validate_reload_blocking(
    session: &SshSession,
    kind: WebServerKind,
    path: &str,
    content: &str,
) -> Result<WebServerSaveResult> {
    crate::ssh::runtime::shared()
        .block_on(save_file_validate_reload(session, kind, path, content))
}

/// Batch-save several files atomically: backup each, write each,
/// run validate ONCE for the whole tree, reload ONCE on success.
/// On validate-fail every backup is restored.
///
/// Useful for Apache-style configs where vhosts live in separate
/// `sites-enabled/*` files and `apachectl configtest` covers the
/// whole tree — committing them one-by-one would run validate N
/// times and reload N times.
pub async fn save_files_batch(
    session: &SshSession,
    kind: WebServerKind,
    entries: &[WebServerBatchSaveEntry],
) -> Result<WebServerBatchSaveResult> {
    if entries.is_empty() {
        return Err(crate::ssh::error::SshError::InvalidConfig(
            "save_files_batch: no entries".to_string(),
        ));
    }
    for entry in entries {
        if !is_path_under_config_root(kind, &entry.path) {
            return Err(crate::ssh::error::SshError::InvalidConfig(format!(
                "refusing to write {}: must live under {}",
                entry.path,
                default_config_root(kind)
            )));
        }
    }

    let is_root = match session.exec_command("id -u").await {
        Ok((0, stdout)) => stdout.trim() == "0",
        _ => false,
    };
    let prefix = if is_root { "" } else { "sudo -n " };

    let ts = match session.exec_command("date +%s").await {
        Ok((0, out)) => out.trim().to_string(),
        _ => "0".to_string(),
    };

    // 1) Backup + write each file. If any step fails midway, restore
    //    all completed-so-far backups in reverse order before bailing.
    let mut backup_paths: Vec<String> = Vec::with_capacity(entries.len());
    for (idx, entry) in entries.iter().enumerate() {
        let backup_path = format!("{}.pier-bak.{ts}.{idx}", entry.path);
        let q_path = crate::services::nginx::shell_single_quote(&entry.path);
        let q_backup = crate::services::nginx::shell_single_quote(&backup_path);
        let backup_cmd = format!("{prefix}cp -p {q_path} {q_backup}");
        let (bc, bo) = session.exec_command(&backup_cmd).await?;
        if bc != 0 {
            // Roll back already-completed backups (just unlink them —
            // no source files were touched yet).
            for prev in &backup_paths {
                let q_prev = crate::services::nginx::shell_single_quote(prev);
                let _ = session
                    .exec_command(&format!("{prefix}rm -f {q_prev}"))
                    .await;
            }
            return Err(crate::ssh::error::SshError::InvalidConfig(format!(
                "backup {} failed: {}",
                entry.path,
                bo.trim()
            )));
        }
        backup_paths.push(backup_path);

        // Atomic write via base64 → tmp → mv.
        use std::io::Write;
        let mut encoded = String::new();
        {
            let mut writer = crate::services::nginx::base64_writer(&mut encoded);
            writer.write_all(entry.content.as_bytes()).ok();
            writer.flush().ok();
        }
        let tmp_path = format!("/tmp/pier-webserver-{ts}-{idx}.conf");
        let q_tmp = crate::services::nginx::shell_single_quote(&tmp_path);
        let q_b64 = crate::services::nginx::shell_single_quote(&encoded);
        let inner = format!(
            "echo {q_b64} | base64 -d > {q_tmp} && chmod --reference={q_path} {q_tmp} 2>/dev/null || true; mv {q_tmp} {q_path}"
        );
        let write_cmd = format!(
            "{prefix}sh -c {}",
            crate::services::nginx::shell_single_quote(&inner)
        );
        let (wc, wo) = session.exec_command(&write_cmd).await?;
        if wc != 0 {
            // Restore everything completed so far.
            let restore_errors = restore_all(session, prefix, entries, &backup_paths).await;
            return Err(crate::ssh::error::SshError::InvalidConfig(format!(
                "write {} failed: {}{}",
                entry.path,
                wo.trim(),
                if restore_errors.iter().any(|e| !e.is_empty()) {
                    format!(" — restore had errors: {restore_errors:?}")
                } else {
                    String::new()
                }
            )));
        }
    }

    // 2) Validate once for the whole tree.
    let validate = validate(session, kind).await?;

    if !validate.ok {
        // 3) Restore on validation failure.
        let restore_errors = restore_all(session, prefix, entries, &backup_paths).await;
        let restored = restore_errors.iter().all(|e| e.is_empty());
        return Ok(WebServerBatchSaveResult {
            backup_paths,
            validate,
            reloaded: false,
            reload_output: String::new(),
            restored,
            restore_errors,
        });
    }

    // 4) Reload once.
    let reload_result = reload(session, kind).await?;

    // 5) Trim aged backups for each touched file. Best-effort, won't
    //    fail the batch save.
    for entry in entries {
        trim_old_backups(session, prefix, &entry.path).await;
    }

    Ok(WebServerBatchSaveResult {
        backup_paths,
        validate,
        reloaded: reload_result.ok,
        reload_output: reload_result.output,
        restored: true,
        restore_errors: vec![String::new(); entries.len()],
    })
}

async fn restore_all(
    session: &SshSession,
    prefix: &str,
    entries: &[WebServerBatchSaveEntry],
    backup_paths: &[String],
) -> Vec<String> {
    let mut errors = Vec::with_capacity(entries.len());
    for (entry, backup) in entries.iter().zip(backup_paths.iter()) {
        let q_path = crate::services::nginx::shell_single_quote(&entry.path);
        let q_backup = crate::services::nginx::shell_single_quote(backup);
        let cmd = format!("{prefix}mv {q_backup} {q_path}");
        match session.exec_command(&cmd).await {
            Ok((0, _)) => errors.push(String::new()),
            Ok((_, out)) => errors.push(out.trim().to_string()),
            Err(e) => errors.push(e.to_string()),
        }
    }
    errors
}

pub fn save_files_batch_blocking(
    session: &SshSession,
    kind: WebServerKind,
    entries: &[WebServerBatchSaveEntry],
) -> Result<WebServerBatchSaveResult> {
    crate::ssh::runtime::shared().block_on(save_files_batch(session, kind, entries))
}

pub async fn toggle_site(
    session: &SshSession,
    kind: WebServerKind,
    site_name: &str,
    enable: bool,
) -> Result<WebServerActionResult> {
    if kind != WebServerKind::Apache {
        return Err(crate::ssh::error::SshError::InvalidConfig(
            "site enable/disable is only supported for Apache".to_string(),
        ));
    }
    if site_name.is_empty()
        || site_name.contains('/')
        || site_name.contains('\0')
        || site_name == "."
        || site_name == ".."
    {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "invalid site name: {site_name}"
        )));
    }
    let q_name = crate::services::nginx::shell_single_quote(site_name);
    // Prefer a2ensite/a2dissite which also reload the config; fall
    // back to manual symlink management for distros that don't ship
    // them. The fallback uses absolute paths (no `cd`) and passes
    // through the same prefix.
    let cmd = if enable {
        format!(
            "sh -c 'if command -v a2ensite >/dev/null 2>&1; then \
                a2ensite {q_name} 2>&1; \
             else \
                ln -sfn /etc/apache2/sites-available/{q_name} \
                  /etc/apache2/sites-enabled/{q_name} 2>&1; \
             fi'"
        )
    } else {
        format!(
            "sh -c 'if command -v a2dissite >/dev/null 2>&1; then \
                a2dissite {q_name} 2>&1; \
             else \
                rm -f /etc/apache2/sites-enabled/{q_name} 2>&1; \
             fi'"
        )
    };
    run_with_sudo(session, &cmd).await
}

pub fn toggle_site_blocking(
    session: &SshSession,
    kind: WebServerKind,
    site_name: &str,
    enable: bool,
) -> Result<WebServerActionResult> {
    crate::ssh::runtime::shared().block_on(toggle_site(session, kind, site_name, enable))
}

// ── New-site wizard ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateSiteResult {
    /// Final absolute path where the file was created.
    pub path: String,
    /// True when the file was also enabled (Apache `a2ensite` flow).
    pub enabled: bool,
    /// `enable` step output, if it ran. Empty when `enable` was false
    /// or the kind doesn't support enable/disable.
    pub enable_output: String,
}

/// Create a new site config file under the conventional directory for
/// each product, then optionally enable it (Apache only). The file
/// must not already exist; we refuse to clobber.
///
/// `leaf_name` is just the filename (no slashes, no `..`). The full
/// path is computed from `kind` + the conventional directory.
pub async fn create_site_file(
    session: &SshSession,
    kind: WebServerKind,
    leaf_name: &str,
    content: &str,
    enable_after: bool,
) -> Result<CreateSiteResult> {
    if leaf_name.is_empty()
        || leaf_name.contains('/')
        || leaf_name.contains('\0')
        || leaf_name == "."
        || leaf_name == ".."
    {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "invalid site filename: {leaf_name}"
        )));
    }

    let target_dir = match kind {
        WebServerKind::Nginx => {
            return Err(crate::ssh::error::SshError::InvalidConfig(
                "nginx site creation goes through services::nginx::create_file".to_string(),
            ))
        }
        WebServerKind::Apache => {
            // Apache layout differs by distro. Prefer Debian's
            // sites-available; fall back to RHEL's conf.d.
            let probe = session
                .exec_command("test -d /etc/apache2/sites-available && echo DEB || echo RHEL")
                .await?;
            if probe.1.trim() == "DEB" {
                "/etc/apache2/sites-available"
            } else {
                "/etc/httpd/conf.d"
            }
        }
        WebServerKind::Caddy => "/etc/caddy/conf.d",
    };

    let target_path = format!("{target_dir}/{leaf_name}");

    let is_root = match session.exec_command("id -u").await {
        Ok((0, stdout)) => stdout.trim() == "0",
        _ => false,
    };
    let prefix = if is_root { "" } else { "sudo -n " };

    // Make sure the parent dir exists (caddy's conf.d isn't created by
    // default on every install).
    let q_dir = crate::services::nginx::shell_single_quote(target_dir);
    let mkdir_cmd = format!("{prefix}mkdir -p {q_dir}");
    let (mkdir_code, mkdir_out) = session.exec_command(&mkdir_cmd).await?;
    if mkdir_code != 0 {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "mkdir {target_dir} failed: {}",
            mkdir_out.trim()
        )));
    }

    // Refuse to clobber.
    let q_target = crate::services::nginx::shell_single_quote(&target_path);
    let exists_check = format!(
        "{prefix}sh -c 'test -e {q_target} && echo EXISTS || echo MISSING' 2>&1"
    );
    let (_, exists_out) = session.exec_command(&exists_check).await?;
    if exists_out.contains("EXISTS") {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "{target_path} already exists — pick another name"
        )));
    }

    // Atomic write via base64 → tmp → mv.
    use std::io::Write;
    let mut encoded = String::new();
    {
        let mut writer = crate::services::nginx::base64_writer(&mut encoded);
        writer.write_all(content.as_bytes()).ok();
        writer.flush().ok();
    }

    let ts = match session.exec_command("date +%s").await {
        Ok((0, out)) => out.trim().to_string(),
        _ => "0".to_string(),
    };
    let tmp_path = format!("/tmp/pier-webserver-new-{ts}.conf");
    let q_tmp = crate::services::nginx::shell_single_quote(&tmp_path);
    let q_b64 = crate::services::nginx::shell_single_quote(&encoded);

    // Match permissions to a sensible reference file (the parent dir
    // is the only thing we can rely on existing).
    let inner = format!(
        "echo {q_b64} | base64 -d > {q_tmp} \
         && chmod 644 {q_tmp} \
         && mv {q_tmp} {q_target}"
    );
    let write_cmd = format!(
        "{prefix}sh -c {}",
        crate::services::nginx::shell_single_quote(&inner)
    );
    let (write_code, write_out) = session.exec_command(&write_cmd).await?;
    if write_code != 0 {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "write {target_path} failed: {}",
            write_out.trim()
        )));
    }

    // Optional enable step.
    let mut enable_output = String::new();
    let mut enabled = false;
    if enable_after {
        match kind {
            WebServerKind::Apache => {
                // Only meaningful on Debian-style layouts.
                let enable_cmd = format!(
                    "sh -c 'if command -v a2ensite >/dev/null 2>&1; then \
                        a2ensite {leaf} 2>&1; \
                     else \
                        ln -sfn /etc/apache2/sites-available/{leaf} \
                          /etc/apache2/sites-enabled/{leaf} 2>&1; \
                     fi'",
                    leaf = crate::services::nginx::shell_single_quote(leaf_name),
                );
                let result = run_with_sudo(session, &enable_cmd).await?;
                enable_output = result.output;
                enabled = result.ok;
            }
            WebServerKind::Caddy => {
                // Caddy auto-includes /etc/caddy/conf.d/* iff the main
                // Caddyfile has `import conf.d/*`. Surface guidance
                // rather than silently doing nothing.
                enable_output = "Caddy doesn't enable per-file the way Apache does. \
                                 Make sure /etc/caddy/Caddyfile has \
                                 `import /etc/caddy/conf.d/*` so this file is loaded."
                    .to_string();
                enabled = true;
            }
            _ => {}
        }
    }

    Ok(CreateSiteResult {
        path: target_path,
        enabled,
        enable_output,
    })
}

pub fn create_site_file_blocking(
    session: &SshSession,
    kind: WebServerKind,
    leaf_name: &str,
    content: &str,
    enable_after: bool,
) -> Result<CreateSiteResult> {
    crate::ssh::runtime::shared()
        .block_on(create_site_file(session, kind, leaf_name, content, enable_after))
}
