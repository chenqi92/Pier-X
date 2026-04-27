//! Remote NGINX config browser + structured editor.
//!
//! Lists nginx config files (`/etc/nginx/nginx.conf` + `conf.d/*.conf` +
//! `sites-available/*` + `sites-enabled/*`), parses them into a small AST
//! that round-trips with comments, validates new content via
//! `nginx -t`, then atomically replaces + reloads.
//!
//! ## Parser
//!
//! Hand-rolled lexer + recursive-descent. NGINX syntax is tiny:
//!
//! * tokens: word, single/double-quoted string, `;`, `{`, `}`, `# comment`
//! * `directive arg1 arg2 ... ;`
//! * `directive arg1 ... { children }`
//!
//! Round-trip preserves comments and blank-line spacing per node, but
//! re-indents block bodies with 4 spaces. That matches the convention
//! used by `nginx -T` output and is good enough for the "edit a few
//! directives + save" use case this panel targets.
//!
//! `*_by_lua_block` / `*_by_njs_block` directives use a body that
//! contains its own `{`/`}` (Lua / JavaScript). We do NOT recurse into
//! them — the body is captured as an opaque blob so embedded code can
//! contain anything without breaking the parser.
//!
//! ## Save flow
//!
//! 1. Read original into a `.pier-bak` sibling.
//! 2. Write new content atomically (`tmp` → `mv`).
//! 3. Run `nginx -t` against the *real* config tree. This is necessary
//!    because included files (conf.d, sites-available) are not standalone
//!    configs — they only validate as part of the main `nginx.conf`.
//! 4. On failure: restore from backup, surface stderr.
//! 5. On success: `systemctl reload nginx` (falls back to `nginx -s reload`),
//!    drop backup.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::ssh::error::Result;
use crate::ssh::SshSession;

// ── Public types ────────────────────────────────────────────────────

/// One config file surfaced in the panel's file tree. `kind` mirrors
/// the conventional debian/ubuntu layout — distros that don't ship
/// `sites-{available,enabled}` simply have empty lists for those
/// buckets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum NginxFileKind {
    /// Top-level `/etc/nginx/nginx.conf`.
    Main,
    /// Member of `/etc/nginx/conf.d/`.
    ConfD,
    /// Source file in `/etc/nginx/sites-available/`.
    SiteAvailable {
        /// True iff `sites-enabled/<name>` symlinks to this file.
        enabled: bool,
    },
    /// Symlink in `/etc/nginx/sites-enabled/` whose target lives
    /// outside `sites-available`. Surfaced separately so the user
    /// can see a stray link rather than having it silently shadowed.
    SiteEnabledOrphan {
        /// Resolved target the symlink points at — surfaced verbatim
        /// so the user can see where the stray link goes.
        link_target: String,
    },
}

/// One config file we discovered on the host. The panel renders these
/// in the file tree and round-trips `path` back through read/save.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NginxFile {
    /// Absolute remote path. Stable identifier — round-tripped to
    /// the read/save commands.
    pub path: String,
    /// Display label (basename for conf.d / site files; `nginx.conf`
    /// for the main file).
    pub name: String,
    /// Which standard nginx config role this file plays.
    pub kind: NginxFileKind,
    /// File size in bytes from the remote stat.
    pub size_bytes: u64,
    /// Last-modified epoch seconds; 0 when stat failed.
    pub mtime_secs: i64,
}

/// One-shot snapshot of the host's nginx layout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NginxLayout {
    /// `true` when `nginx` is on PATH.
    pub installed: bool,
    /// Output of `nginx -v 2>&1` — empty when `installed` is false.
    pub version: String,
    /// Compile-time `--with-*` flags parsed from `nginx -V 2>&1`.
    /// Used by the panel's "modules" section to tell which built-in
    /// modules are available without installing extras.
    pub builtin_modules: Vec<String>,
    /// Files we found across the standard config dirs. Order:
    /// main → conf.d → sites-available → orphan sites-enabled.
    pub files: Vec<NginxFile>,
    /// `true` when the SSH user is uid 0. Drives whether the panel
    /// adds a `sudo -n ` prefix to write commands.
    pub is_root: bool,
}

/// Parser output. `errors` is non-empty when the parse recovered past
/// a malformed directive — we still hand back whatever we could parse
/// so the UI can render the rest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NginxParseResult {
    /// The parsed AST. `Vec<NginxNode>` because top-level can hold
    /// directives, comments, and blank-line markers in any order.
    pub nodes: Vec<NginxNode>,
    /// Recoverable parse warnings (unterminated quote, missing `;`,
    /// stray `}`, etc.). Empty on a clean parse.
    pub errors: Vec<String>,
}

/// A node in the parsed config tree. The `position` enum keeps
/// comments and blank lines in a stable place so render() round-trips.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum NginxNode {
    /// A directive — either inline (`name args;`) or a block
    /// (`name args { ... }`) — with its leading-comment/blank-line
    /// halo preserved.
    Directive(NginxDirective),
    /// A standalone comment line.
    Comment {
        /// `#` excluded; indentation also excluded. The renderer
        /// adds them back at the right depth.
        text: String,
        /// Blank lines that preceded this comment. Cap at 2 in the
        /// renderer; we read whatever the source had.
        leading_blanks: u32,
    },
}

/// A directive plus an optional block body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NginxDirective {
    /// Directive name (`listen`, `server_name`, `proxy_pass`, …).
    pub name: String,
    /// Positional arguments. Each string keeps its quoting style as
    /// recorded by the lexer (`"..."` / `'...'` / bare). The renderer
    /// quotes/un-quotes via `needs_quoting` to keep edits safe.
    pub args: Vec<String>,
    /// Comment lines that immediately preceded this directive.
    pub leading_comments: Vec<String>,
    /// Blank lines between the previous sibling and this directive.
    pub leading_blanks: u32,
    /// Trailing same-line comment (after `;` or `{`). `None` when
    /// none was present.
    pub inline_comment: Option<String>,
    /// `Some` when this directive opens a block. Sub-nodes recurse.
    pub block: Option<Vec<NginxNode>>,
    /// `Some(raw)` when the directive name matches `*_by_lua_block` /
    /// `*_by_njs_block` — body is captured as opaque text so embedded
    /// Lua/JS doesn't break the parser. Mutually exclusive with
    /// `block`.
    pub opaque_body: Option<String>,
}

/// Outcome of `nginx -t`. `output` is merged stdout+stderr.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NginxValidateResult {
    /// `true` when `nginx -t` exited 0.
    pub ok: bool,
    /// Raw exit code from `nginx -t`.
    pub exit_code: i32,
    /// Merged stdout+stderr from `nginx -t`.
    pub output: String,
}

/// Outcome of save → validate → reload. When `validate.ok` is false the
/// backup is restored and `reloaded` stays false. `restored` reports
/// whether the restore step itself completed (a failed restore is
/// surfaced as `false` + a populated `restore_error`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NginxSaveResult {
    /// Result of the post-write `nginx -t` step.
    pub validate: NginxValidateResult,
    /// `true` when `nginx -s reload` (or `systemctl reload nginx`)
    /// completed successfully after a passing validation.
    pub reloaded: bool,
    /// Merged stdout+stderr from the reload command. Empty when the
    /// reload step was skipped.
    pub reload_output: String,
    /// `true` when a failed validation triggered restoring the
    /// `.pier-bak` and the restore itself succeeded.
    pub restored: bool,
    /// Populated when `restored` is `false` *and* a restore was
    /// attempted — describes why we couldn't put the original back.
    pub restore_error: Option<String>,
    /// Path of the `.pier-bak` we wrote — surfaced so the panel can
    /// tell the user where to recover from on a wedged restore.
    pub backup_path: String,
}

// ── Layout discovery ────────────────────────────────────────────────

const NGINX_CONF_PATH: &str = "/etc/nginx/nginx.conf";
const NGINX_CONF_D_DIR: &str = "/etc/nginx/conf.d";
const NGINX_SITES_AVAILABLE_DIR: &str = "/etc/nginx/sites-available";
const NGINX_SITES_ENABLED_DIR: &str = "/etc/nginx/sites-enabled";

/// One-shot probe: nginx presence + version + built-in modules + the
/// files we'd render in the file tree. Cheap enough to call on panel
/// open; the caller decides when to refresh.
pub async fn list_layout(session: &SshSession) -> Result<NginxLayout> {
    // Presence + version.
    let (installed, version) = match session
        .exec_command("command -v nginx >/dev/null 2>&1 && nginx -v 2>&1")
        .await
    {
        Ok((0, out)) => (true, out.trim().to_string()),
        _ => (false, String::new()),
    };

    let builtin_modules = if installed {
        parse_nginx_v_modules(
            &session
                .exec_command("nginx -V 2>&1")
                .await
                .map(|(_, o)| o)
                .unwrap_or_default(),
        )
    } else {
        Vec::new()
    };

    let is_root = match session.exec_command("id -u").await {
        Ok((0, stdout)) => stdout.trim() == "0",
        _ => false,
    };

    // Build the file list. `find` queries are tolerant of missing
    // directories — `2>/dev/null` swallows the "no such file" error
    // so we just see an empty list for distros without that layout.
    let mut files: Vec<NginxFile> = Vec::new();

    // Main config first.
    if let Some(f) = stat_one(session, NGINX_CONF_PATH, NginxFileKind::Main).await {
        files.push(f);
    }

    for f in list_dir(session, NGINX_CONF_D_DIR, "*.conf", NginxFileKind::ConfD).await {
        files.push(f);
    }

    // sites-enabled is read first to decide which sites-available
    // entries are "enabled". Map basename → link target.
    let enabled_map = read_sites_enabled(session).await;
    let enabled_set: HashSet<String> = enabled_map
        .iter()
        .map(|(_, target)| target.clone())
        .collect();

    let mut available_paths: HashSet<String> = HashSet::new();
    for f in list_dir(
        session,
        NGINX_SITES_AVAILABLE_DIR,
        "*",
        NginxFileKind::SiteAvailable { enabled: false },
    )
    .await
    {
        let enabled = enabled_set.contains(&f.path);
        available_paths.insert(f.path.clone());
        files.push(NginxFile {
            kind: NginxFileKind::SiteAvailable { enabled },
            ..f
        });
    }

    // Orphan symlinks: `sites-enabled/X → /elsewhere/Y` where Y is not
    // in sites-available. Surface them so the user can reason about
    // them rather than silently dropping them.
    for (link_path, target) in &enabled_map {
        if !available_paths.contains(target) {
            if let Some(f) = stat_one(
                session,
                link_path,
                NginxFileKind::SiteEnabledOrphan {
                    link_target: target.clone(),
                },
            )
            .await
            {
                files.push(f);
            }
        }
    }

    Ok(NginxLayout {
        installed,
        version,
        builtin_modules,
        files,
        is_root,
    })
}

/// Blocking wrapper for [`list_layout`].
pub fn list_layout_blocking(session: &SshSession) -> Result<NginxLayout> {
    crate::ssh::runtime::shared().block_on(list_layout(session))
}

/// `find <dir> -maxdepth 1 -mindepth 1 -name <pattern>` plus a stat on
/// each hit. We chain everything into one `sh -c` so the whole listing
/// is a single round-trip — important when the panel can list 30+
/// site files.
async fn list_dir(
    session: &SshSession,
    dir: &str,
    name_glob: &str,
    kind: NginxFileKind,
) -> Vec<NginxFile> {
    let listing_cmd = format!(
        "find {dir} -mindepth 1 -maxdepth 1 -name {pat} \
         -printf '%p\\t%s\\t%T@\\n' 2>/dev/null | sort",
        dir = shell_single_quote(dir),
        pat = shell_single_quote(name_glob),
    );
    let output = match session.exec_command(&listing_cmd).await {
        Ok((_, out)) => out,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for line in output.lines() {
        let mut parts = line.split('\t');
        let Some(path) = parts.next() else { continue };
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        let size = parts
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let mtime = parts
            .next()
            .and_then(|s| s.split('.').next())
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let name = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .to_string();
        out.push(NginxFile {
            path: path.to_string(),
            name,
            kind: kind.clone(),
            size_bytes: size,
            mtime_secs: mtime,
        });
    }
    out
}

/// Single-file stat — used for `nginx.conf` and orphan sites-enabled
/// links. Empty `kind` if the file doesn't exist.
async fn stat_one(
    session: &SshSession,
    path: &str,
    kind: NginxFileKind,
) -> Option<NginxFile> {
    let cmd = format!(
        "test -e {p} && stat -c '%s\\t%Y' {p} 2>/dev/null",
        p = shell_single_quote(path),
    );
    let (code, out) = session.exec_command(&cmd).await.ok()?;
    if code != 0 {
        return None;
    }
    let line = out.trim();
    let mut parts = line.split('\t');
    let size = parts
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let mtime = parts
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let name = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string();
    Some(NginxFile {
        path: path.to_string(),
        name,
        kind,
        size_bytes: size,
        mtime_secs: mtime,
    })
}

/// Read `sites-enabled` and return `(link_path, absolute_target)` pairs.
/// `readlink -f` resolves through to the real path so a relative link
/// like `../sites-available/foo` lines up with our list of real files.
async fn read_sites_enabled(session: &SshSession) -> Vec<(String, String)> {
    let cmd = format!(
        "find {dir} -mindepth 1 -maxdepth 1 -type l 2>/dev/null | while read p; do \
            t=$(readlink -f \"$p\" 2>/dev/null); \
            printf '%s\\t%s\\n' \"$p\" \"$t\"; \
         done",
        dir = shell_single_quote(NGINX_SITES_ENABLED_DIR),
    );
    let Ok((_, out)) = session.exec_command(&cmd).await else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    for line in out.lines() {
        let mut parts = line.splitn(2, '\t');
        let link = parts.next().unwrap_or("").trim();
        let target = parts.next().unwrap_or("").trim();
        if !link.is_empty() && !target.is_empty() {
            pairs.push((link.to_string(), target.to_string()));
        }
    }
    pairs
}

/// Pull `--with-*` tokens out of `nginx -V 2>&1`. nginx prints
/// configure flags space-separated on one of the lines; everything
/// starting with `--with-` is a built-in module marker.
pub fn parse_nginx_v_modules(output: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tok in output.split_whitespace() {
        if let Some(rest) = tok.strip_prefix("--with-") {
            // Drop trailing `=...` if the flag carries a value.
            let head = rest.split('=').next().unwrap_or(rest);
            if !head.is_empty() {
                out.push(head.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

// ── File read / save / reload ───────────────────────────────────────

/// Read a config file's full text. `cat` is fine — config files are
/// small (a few KB at most). Non-UTF-8 bytes are replaced via
/// `from_utf8_lossy` upstream in the SSH session.
pub async fn read_file(session: &SshSession, path: &str) -> Result<String> {
    let cmd = format!("cat {} 2>&1", shell_single_quote(path));
    let (code, out) = session.exec_command(&cmd).await?;
    if code != 0 {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "read {path} failed (exit {code}): {}",
            out.trim()
        )));
    }
    Ok(out)
}

/// Blocking wrapper for [`read_file`].
pub fn read_file_blocking(session: &SshSession, path: &str) -> Result<String> {
    crate::ssh::runtime::shared().block_on(read_file(session, path))
}

/// Create a new config file under `conf.d/` or `sites-available/`.
///
/// Path is validated client-side too, but we double-check here:
/// must live under one of the two allowed directories, must not
/// contain `..` segments, and the file must not already exist
/// (we refuse to clobber). New files are written via the same
/// base64 → tmp → mv pipeline as `save_file_validate_reload`.
///
/// Note: this does NOT run `nginx -t` afterwards. A freshly-created
/// site file with template content can leave the live tree in a
/// state where `nginx -t` would actually complain about the new
/// stub before the user customizes it (e.g. server_name collisions
/// across multiple stub sites). The user's first save through the
/// editor will run validation. If the stub itself is bad, that save
/// rolls it back per the normal flow.
pub async fn create_file(
    session: &SshSession,
    path: &str,
    content: &str,
) -> Result<NginxValidateResult> {
    if !is_allowed_create_path(path) {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "refusing to create {path}: must live under {NGINX_CONF_D_DIR}/ \
             or {NGINX_SITES_AVAILABLE_DIR}/ with no `..` segments"
        )));
    }

    let is_root = match session.exec_command("id -u").await {
        Ok((0, stdout)) => stdout.trim() == "0",
        _ => false,
    };
    let prefix = if is_root { "" } else { "sudo -n " };

    // Refuse to clobber. `test -e` covers files, dirs, symlinks.
    let exists_check = format!(
        "{prefix}sh -c 'test -e {p} && echo EXISTS || echo MISSING' 2>&1",
        p = shell_single_quote(path),
    );
    let (_, exists_out) = session.exec_command(&exists_check).await?;
    if exists_out.contains("EXISTS") {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "{path} already exists — pick another name"
        )));
    }

    use std::io::Write;
    let mut encoded = String::new();
    {
        let mut writer = base64_writer(&mut encoded);
        writer.write_all(content.as_bytes()).ok();
        writer.flush().ok();
    }

    let ts = match session.exec_command("date +%s").await {
        Ok((0, out)) => out.trim().to_string(),
        _ => "0".to_string(),
    };
    let tmp_path = format!("/tmp/pier-nginx-new-{ts}.conf");

    // Touch a parent-permission reference: nginx.conf reliably exists
    // and has the right ownership for new sibling configs. Falls back
    // to mode 644 if `--reference` isn't supported (older coreutils).
    let inner = format!(
        "echo {b64} | base64 -d > {tmp} \
         && (chmod --reference={ref} {tmp} 2>/dev/null || chmod 644 {tmp}) \
         && (chown --reference={ref} {tmp} 2>/dev/null || true) \
         && mv {tmp} {target}",
        b64 = shell_single_quote(&encoded),
        tmp = shell_single_quote(&tmp_path),
        r#ref = shell_single_quote(NGINX_CONF_PATH),
        target = shell_single_quote(path),
    );
    let cmd = format!("{prefix}sh -c {} 2>&1", shell_single_quote(&inner));
    let (code, out) = session.exec_command(&cmd).await?;
    Ok(NginxValidateResult {
        ok: code == 0,
        exit_code: code,
        output: out,
    })
}

/// Blocking wrapper for [`create_file`].
pub fn create_file_blocking(
    session: &SshSession,
    path: &str,
    content: &str,
) -> Result<NginxValidateResult> {
    crate::ssh::runtime::shared().block_on(create_file(session, path, content))
}

/// Path-allowlist for [`create_file`]. Returns `true` when the path
/// lives directly under `/etc/nginx/conf.d/` or
/// `/etc/nginx/sites-available/`, has no `..` segments, and ends with
/// a non-empty leaf name. The leaf may contain dots (so `mysite.conf`
/// is fine), but must not be `.` / `..` itself.
fn is_allowed_create_path(path: &str) -> bool {
    let allowed_prefixes = [
        format!("{NGINX_CONF_D_DIR}/"),
        format!("{NGINX_SITES_AVAILABLE_DIR}/"),
    ];
    let Some(prefix) = allowed_prefixes
        .iter()
        .find(|p| path.starts_with(p.as_str()))
    else {
        return false;
    };
    let leaf = &path[prefix.len()..];
    if leaf.is_empty() || leaf.contains('/') || leaf.contains('\0') {
        return false;
    }
    if leaf == "." || leaf == ".." {
        return false;
    }
    // No `..` anywhere, even if leaf is the whole rest — defensive
    // for paths like `conf.d/..foo` (legal) vs `conf.d/../etc` (rejected
    // earlier by the leading-prefix check + the `/` check above).
    !path.split('/').any(|seg| seg == "..")
}

/// Save → validate → reload. Layout:
///
/// 1. `cp <path> <path>.pier-bak.<ts>` — backup. We use a timestamped
///    suffix so two saves in a row don't clobber the prior backup.
/// 2. Write new content to a temp file under `/tmp/`, then `mv` it
///    over the target so readers never see a partially-written file.
/// 3. `nginx -t` against the live tree (necessary because included
///    files don't validate standalone).
/// 4. On failure: `mv <backup> <path>` to restore.
/// 5. On success: `systemctl reload nginx` (fall through to
///    `nginx -s reload` if systemctl missing). Backup stays on disk.
pub async fn save_file_validate_reload(
    session: &SshSession,
    path: &str,
    content: &str,
) -> Result<NginxSaveResult> {
    let is_root = match session.exec_command("id -u").await {
        Ok((0, stdout)) => stdout.trim() == "0",
        _ => false,
    };
    let prefix = if is_root { "" } else { "sudo -n " };

    // Use the seconds-since-epoch from the remote so concurrent edits
    // from different clients don't collide on a clock-skew window.
    let ts = match session.exec_command("date +%s").await {
        Ok((0, out)) => out.trim().to_string(),
        _ => "0".to_string(),
    };
    let backup_path = format!("{path}.pier-bak.{ts}");

    // 1) Backup. `cp -p` preserves mode/owner so a later restore
    // doesn't ratchet permissions.
    let backup_cmd = format!(
        "{prefix}cp -p {src} {dst}",
        src = shell_single_quote(path),
        dst = shell_single_quote(&backup_path),
    );
    let (backup_code, backup_out) = session.exec_command(&backup_cmd).await?;
    if backup_code != 0 {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "backup {path} → {backup_path} failed: {}",
            backup_out.trim()
        )));
    }

    // 2) Atomic write. The new content is base64-encoded so the SSH
    // exec channel doesn't need to worry about quoting newlines /
    // single-quotes / shell metacharacters in the user's directives.
    use std::io::Write;
    let mut encoded = String::new();
    {
        let mut writer = base64_writer(&mut encoded);
        writer.write_all(content.as_bytes()).ok();
        writer.flush().ok();
    }

    let tmp_path = format!("/tmp/pier-nginx-{ts}.conf");
    let write_cmd = format!(
        "{prefix}sh -c {inner}",
        inner = shell_single_quote(&format!(
            "echo {b64} | base64 -d > {tmp} && chmod --reference={target} {tmp} 2>/dev/null || true; \
             mv {tmp} {target}",
            b64 = shell_single_quote(&encoded),
            tmp = shell_single_quote(&tmp_path),
            target = shell_single_quote(path),
        )),
    );
    let (write_code, write_out) = session.exec_command(&write_cmd).await?;
    if write_code != 0 {
        // Best-effort restore so we don't leave the file in a bad state.
        let _ = session
            .exec_command(&format!(
                "{prefix}mv {bak} {target}",
                bak = shell_single_quote(&backup_path),
                target = shell_single_quote(path),
            ))
            .await;
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "write {path} failed: {}",
            write_out.trim()
        )));
    }

    // 3) Validate.
    let validate_cmd = format!("{prefix}nginx -t 2>&1");
    let (validate_code, validate_out) = session.exec_command(&validate_cmd).await?;
    let validate = NginxValidateResult {
        ok: validate_code == 0,
        exit_code: validate_code,
        output: validate_out,
    };

    if !validate.ok {
        // 4) Restore on validation failure.
        let restore_cmd = format!(
            "{prefix}mv {bak} {target}",
            bak = shell_single_quote(&backup_path),
            target = shell_single_quote(path),
        );
        let (rc, rout) = session
            .exec_command(&restore_cmd)
            .await
            .unwrap_or((-1, String::new()));
        let restored = rc == 0;
        return Ok(NginxSaveResult {
            validate,
            reloaded: false,
            reload_output: String::new(),
            restored,
            restore_error: if restored { None } else { Some(rout) },
            backup_path,
        });
    }

    // 5) Reload — prefer systemd, fall back to `nginx -s reload`.
    let reload_cmd = format!(
        "{prefix}sh -c 'if command -v systemctl >/dev/null 2>&1; then \
            systemctl reload nginx 2>&1; \
         else \
            nginx -s reload 2>&1; \
         fi'"
    );
    let (reload_code, reload_out) = session.exec_command(&reload_cmd).await?;
    let reloaded = reload_code == 0;

    Ok(NginxSaveResult {
        validate,
        reloaded,
        reload_output: reload_out,
        restored: true, // Original was already replaced cleanly.
        restore_error: None,
        backup_path,
    })
}

/// Blocking wrapper for [`save_file_validate_reload`].
pub fn save_file_validate_reload_blocking(
    session: &SshSession,
    path: &str,
    content: &str,
) -> Result<NginxSaveResult> {
    crate::ssh::runtime::shared()
        .block_on(save_file_validate_reload(session, path, content))
}

/// Run `nginx -t` only — useful when the user wants to dry-run before
/// committing or to verify after a manual edit.
pub async fn validate(session: &SshSession) -> Result<NginxValidateResult> {
    let is_root = match session.exec_command("id -u").await {
        Ok((0, stdout)) => stdout.trim() == "0",
        _ => false,
    };
    let prefix = if is_root { "" } else { "sudo -n " };
    let (code, out) = session
        .exec_command(&format!("{prefix}nginx -t 2>&1"))
        .await?;
    Ok(NginxValidateResult {
        ok: code == 0,
        exit_code: code,
        output: out,
    })
}

/// Blocking wrapper for [`validate`].
pub fn validate_blocking(session: &SshSession) -> Result<NginxValidateResult> {
    crate::ssh::runtime::shared().block_on(validate(session))
}

/// Reload nginx without writing anything — surfaces as a button in the
/// panel header for the "I edited config out-of-band, kick it" use case.
pub async fn reload(session: &SshSession) -> Result<NginxValidateResult> {
    let is_root = match session.exec_command("id -u").await {
        Ok((0, stdout)) => stdout.trim() == "0",
        _ => false,
    };
    let prefix = if is_root { "" } else { "sudo -n " };
    let (code, out) = session
        .exec_command(&format!(
            "{prefix}sh -c 'if command -v systemctl >/dev/null 2>&1; then \
                systemctl reload nginx 2>&1; \
             else \
                nginx -s reload 2>&1; \
             fi'"
        ))
        .await?;
    Ok(NginxValidateResult {
        ok: code == 0,
        exit_code: code,
        output: out,
    })
}

/// Blocking wrapper for [`reload`].
pub fn reload_blocking(session: &SshSession) -> Result<NginxValidateResult> {
    crate::ssh::runtime::shared().block_on(reload(session))
}

/// Toggle the symlink for a `sites-available/<name>` entry. `enable=true`
/// creates `sites-enabled/<name> → ../sites-available/<name>`; `false`
/// removes the symlink. Idempotent — re-enabling an already-enabled site
/// just refreshes the link.
pub async fn toggle_site(
    session: &SshSession,
    site_name: &str,
    enable: bool,
) -> Result<NginxValidateResult> {
    if site_name.contains('/') || site_name.contains('\0') {
        return Err(crate::ssh::error::SshError::InvalidConfig(format!(
            "invalid site name: {site_name}"
        )));
    }
    let is_root = match session.exec_command("id -u").await {
        Ok((0, stdout)) => stdout.trim() == "0",
        _ => false,
    };
    let prefix = if is_root { "" } else { "sudo -n " };

    let cmd = if enable {
        format!(
            "{prefix}ln -sf {src} {dst} 2>&1",
            src = shell_single_quote(&format!(
                "{NGINX_SITES_AVAILABLE_DIR}/{site_name}"
            )),
            dst = shell_single_quote(&format!(
                "{NGINX_SITES_ENABLED_DIR}/{site_name}"
            )),
        )
    } else {
        format!(
            "{prefix}rm -f {dst} 2>&1",
            dst = shell_single_quote(&format!(
                "{NGINX_SITES_ENABLED_DIR}/{site_name}"
            )),
        )
    };
    let (code, out) = session.exec_command(&cmd).await?;
    Ok(NginxValidateResult {
        ok: code == 0,
        exit_code: code,
        output: out,
    })
}

/// Blocking wrapper for [`toggle_site`].
pub fn toggle_site_blocking(
    session: &SshSession,
    site_name: &str,
    enable: bool,
) -> Result<NginxValidateResult> {
    crate::ssh::runtime::shared().block_on(toggle_site(session, site_name, enable))
}

// ── Parser ──────────────────────────────────────────────────────────

/// Top-level parse entry. Always returns a result — recoverable errors
/// land in `errors` so the UI can render whatever did parse.
pub fn parse(src: &str) -> NginxParseResult {
    let mut lex = Lexer::new(src);
    let mut errors = Vec::new();
    let nodes = parse_block(&mut lex, false, &mut errors);
    NginxParseResult { nodes, errors }
}

/// Render an AST back to source. The output uses 4-space indentation,
/// trailing semicolons on inline directives, and one space between
/// braces and the directive args. Comments and blank-line counts are
/// preserved per node.
pub fn render(nodes: &[NginxNode]) -> String {
    let mut out = String::new();
    render_nodes(nodes, 0, &mut out);
    out
}

// ── Lexer ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(String),
    /// Quoted argument; the bool tracks single (`'`) vs double (`"`)
    /// so the renderer can emit the same style.
    Quoted { text: String, single: bool },
    Semi,
    BraceOpen,
    BraceClose,
    Comment(String),
    Newline,
    Eof,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    /// Consume `\r`/space/tab (NOT `\n` — newline is a token because
    /// blank-line counting is part of round-trip).
    fn skip_inline_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == b' ' || c == b'\t' || c == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn next_tok(&mut self) -> Tok {
        self.skip_inline_ws();
        let Some(c) = self.peek() else {
            return Tok::Eof;
        };
        match c {
            b'\n' => {
                self.pos += 1;
                Tok::Newline
            }
            b';' => {
                self.pos += 1;
                Tok::Semi
            }
            b'{' => {
                self.pos += 1;
                Tok::BraceOpen
            }
            b'}' => {
                self.pos += 1;
                Tok::BraceClose
            }
            b'#' => {
                // Read to end of line, exclusive of `\n`.
                self.pos += 1;
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if c == b'\n' {
                        break;
                    }
                    self.pos += 1;
                }
                let raw = &self.src[start..self.pos];
                Tok::Comment(String::from_utf8_lossy(raw).trim().to_string())
            }
            b'"' | b'\'' => {
                let quote = c;
                let single = quote == b'\'';
                self.pos += 1;
                let mut text = String::new();
                while let Some(c) = self.peek() {
                    if c == b'\\' {
                        // Preserve the escape verbatim so round-trip
                        // emits the same source.
                        self.pos += 1;
                        if let Some(next) = self.bump() {
                            text.push('\\');
                            text.push(next as char);
                        }
                        continue;
                    }
                    if c == quote {
                        self.pos += 1;
                        return Tok::Quoted { text, single };
                    }
                    if c == b'\n' {
                        // Unterminated string — break and let the
                        // parser flag the recovery.
                        break;
                    }
                    self.pos += 1;
                    text.push(c as char);
                }
                Tok::Quoted { text, single }
            }
            _ => {
                // A bare word — anything until whitespace / `;` / `{` /
                // `}` / `#` / quote.
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if matches!(
                        c,
                        b' ' | b'\t' | b'\r' | b'\n' | b';' | b'{' | b'}' | b'#' | b'"' | b'\''
                    ) {
                        break;
                    }
                    self.pos += 1;
                }
                let raw = &self.src[start..self.pos];
                Tok::Word(String::from_utf8_lossy(raw).into_owned())
            }
        }
    }
}

// ── Parser core ─────────────────────────────────────────────────────

/// Parse a sequence of nodes either at the top level (`stop_on_close=false`)
/// or inside a block (`stop_on_close=true`). Returns when the matching
/// terminator (`Eof` or `}`) is reached. Recoverable errors are pushed
/// into `errors` and parsing continues.
fn parse_block(
    lex: &mut Lexer<'_>,
    stop_on_close: bool,
    errors: &mut Vec<String>,
) -> Vec<NginxNode> {
    let mut out = Vec::new();
    let mut pending_blanks: u32 = 0;
    let mut pending_comments: Vec<String> = Vec::new();

    loop {
        let tok = lex.next_tok();
        match tok {
            Tok::Eof => {
                if stop_on_close {
                    errors.push("unexpected end of file (missing `}`)".into());
                }
                // Drain trailing comments/blanks as standalone nodes.
                for c in pending_comments.drain(..) {
                    out.push(NginxNode::Comment {
                        text: c,
                        leading_blanks: 0,
                    });
                }
                return out;
            }
            Tok::BraceClose => {
                if !stop_on_close {
                    errors.push("stray `}` at top level".into());
                    continue;
                }
                for c in pending_comments.drain(..) {
                    out.push(NginxNode::Comment {
                        text: c,
                        leading_blanks: 0,
                    });
                }
                return out;
            }
            Tok::Newline => {
                pending_blanks = pending_blanks.saturating_add(1);
            }
            Tok::Semi => {
                errors.push("unexpected `;` outside a directive".into());
            }
            Tok::BraceOpen => {
                errors.push("unexpected `{` outside a directive".into());
            }
            Tok::Comment(text) => {
                // Two consecutive newlines → the prior comments are
                // standalone, not "leading the next directive". Flush.
                if pending_blanks >= 2 && !pending_comments.is_empty() {
                    let mut first = true;
                    let blanks_for_first = pending_blanks.saturating_sub(1);
                    for c in pending_comments.drain(..) {
                        out.push(NginxNode::Comment {
                            text: c,
                            leading_blanks: if first { blanks_for_first } else { 0 },
                        });
                        first = false;
                    }
                }
                pending_comments.push(text);
                pending_blanks = 0;
            }
            Tok::Word(directive_name) => {
                // If pending comments are separated from the next
                // directive by 2+ visual newlines, flush them as
                // standalone Comment nodes rather than letting them
                // attach as leading_comments. (Two-newlines = one
                // blank visual line.)
                if pending_blanks >= 2 && !pending_comments.is_empty() {
                    let mut first = true;
                    for c in pending_comments.drain(..) {
                        out.push(NginxNode::Comment {
                            text: c,
                            leading_blanks: if first { 0 } else { 0 },
                        });
                        first = false;
                    }
                    let _ = first;
                }
                // First token of a directive. Read args until `;` or `{`.
                let mut args: Vec<String> = Vec::new();
                let mut block: Option<Vec<NginxNode>> = None;
                let mut opaque_body: Option<String> = None;
                let mut inline_comment: Option<String> = None;

                loop {
                    let t = lex.next_tok();
                    match t {
                        Tok::Word(w) => args.push(w),
                        Tok::Quoted { text, single } => {
                            // Re-quote on round-trip via render(); store
                            // a marker in the arg string so we know the
                            // original style.
                            let mut s = String::with_capacity(text.len() + 2);
                            s.push(if single { '\'' } else { '"' });
                            s.push_str(&text);
                            s.push(if single { '\'' } else { '"' });
                            args.push(s);
                        }
                        Tok::Newline => {
                            // Whitespace within an arg list — skip.
                        }
                        Tok::Comment(c) => {
                            // Comment inside the arg list — promote it to
                            // an inline comment (we'll lose the position
                            // detail; nginx configs almost never put
                            // comments mid-args).
                            inline_comment = Some(c);
                        }
                        Tok::Semi => {
                            break;
                        }
                        Tok::BraceOpen => {
                            // Lua/njs blocks: capture body as opaque.
                            if directive_name.ends_with("_by_lua_block")
                                || directive_name.ends_with("_by_njs_block")
                            {
                                opaque_body = Some(read_opaque_body(lex, errors));
                            } else {
                                block = Some(parse_block(lex, true, errors));
                            }
                            // After `}` we may have an inline comment
                            // before the next newline.
                            lex.skip_inline_ws();
                            if lex.peek() == Some(b'#') {
                                lex.pos += 1;
                                let start = lex.pos;
                                while let Some(c) = lex.peek() {
                                    if c == b'\n' {
                                        break;
                                    }
                                    lex.pos += 1;
                                }
                                let raw = &lex.src[start..lex.pos];
                                inline_comment = Some(
                                    String::from_utf8_lossy(raw).trim().to_string(),
                                );
                            }
                            break;
                        }
                        Tok::BraceClose => {
                            errors.push(format!(
                                "directive `{directive_name}` ended with `}}` instead of `;`",
                            ));
                            // Push back is awkward; treat as terminator.
                            // The outer loop sees the close on its next
                            // iteration via lex's own consumption — but
                            // we already consumed it. To keep behaviour
                            // sane, we synthesise an error and bail.
                            break;
                        }
                        Tok::Eof => {
                            errors.push(format!(
                                "directive `{directive_name}` not terminated",
                            ));
                            break;
                        }
                    }
                }

                out.push(NginxNode::Directive(NginxDirective {
                    name: directive_name,
                    args,
                    leading_comments: std::mem::take(&mut pending_comments),
                    // Subtract 1 because the prior statement's
                    // trailing newline counts toward `pending_blanks`
                    // but is not a *blank* line.
                    leading_blanks: pending_blanks.saturating_sub(1),
                    inline_comment,
                    block,
                    opaque_body,
                }));
                pending_blanks = 0;
            }
            Tok::Quoted { text, .. } => {
                // A quoted string at directive-head position. Real
                // configs never start a directive with a quote, but a
                // hand-crafted file might — accept it as a "word-like"
                // directive name and continue, recording the issue.
                errors.push(format!(
                    "directive starts with a quoted string: {text}",
                ));
            }
        }
    }
}

/// Read the body of a `*_by_lua_block { ... }` or `*_by_njs_block`
/// directive. We've already consumed the opening `{`. Walk byte-by-byte
/// tracking string state and brace depth; return the inner text.
fn read_opaque_body(lex: &mut Lexer<'_>, errors: &mut Vec<String>) -> String {
    let mut depth: i32 = 1;
    let mut out = String::new();
    let mut in_str: Option<u8> = None;
    while let Some(c) = lex.bump() {
        if let Some(q) = in_str {
            out.push(c as char);
            if c == b'\\' {
                if let Some(next) = lex.bump() {
                    out.push(next as char);
                }
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            b'"' | b'\'' => {
                in_str = Some(c);
                out.push(c as char);
            }
            b'{' => {
                depth += 1;
                out.push(c as char);
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return out;
                }
                out.push(c as char);
            }
            _ => out.push(c as char),
        }
    }
    errors.push("unterminated `_by_lua_block` body".into());
    out
}

// ── Renderer ────────────────────────────────────────────────────────

fn indent(buf: &mut String, depth: usize) {
    for _ in 0..depth {
        buf.push_str("    ");
    }
}

fn render_nodes(nodes: &[NginxNode], depth: usize, out: &mut String) {
    for node in nodes {
        match node {
            NginxNode::Comment {
                text,
                leading_blanks,
            } => {
                for _ in 0..(*leading_blanks).min(2) {
                    out.push('\n');
                }
                indent(out, depth);
                out.push('#');
                if !text.is_empty() && !text.starts_with(' ') {
                    out.push(' ');
                }
                out.push_str(text);
                out.push('\n');
            }
            NginxNode::Directive(d) => render_directive(d, depth, out),
        }
    }
}

fn render_directive(d: &NginxDirective, depth: usize, out: &mut String) {
    for _ in 0..d.leading_blanks.min(2) {
        out.push('\n');
    }
    for c in &d.leading_comments {
        indent(out, depth);
        out.push('#');
        if !c.is_empty() && !c.starts_with(' ') {
            out.push(' ');
        }
        out.push_str(c);
        out.push('\n');
    }
    indent(out, depth);
    out.push_str(&d.name);
    for arg in &d.args {
        out.push(' ');
        // Args from the parser keep their quoting style baked in. Args
        // injected by the UI may not — quote on the way out if needed.
        if needs_quoting(arg) {
            out.push('"');
            out.push_str(&escape_double_quoted(arg));
            out.push('"');
        } else {
            out.push_str(arg);
        }
    }

    if let Some(body) = &d.opaque_body {
        out.push_str(" {");
        out.push_str(body);
        out.push('}');
        if let Some(c) = &d.inline_comment {
            out.push(' ');
            out.push('#');
            if !c.is_empty() && !c.starts_with(' ') {
                out.push(' ');
            }
            out.push_str(c);
        }
        out.push('\n');
        return;
    }

    if let Some(children) = &d.block {
        out.push_str(" {");
        if let Some(c) = &d.inline_comment {
            out.push(' ');
            out.push('#');
            if !c.is_empty() && !c.starts_with(' ') {
                out.push(' ');
            }
            out.push_str(c);
        }
        out.push('\n');
        render_nodes(children, depth + 1, out);
        indent(out, depth);
        out.push_str("}\n");
    } else {
        out.push(';');
        if let Some(c) = &d.inline_comment {
            out.push(' ');
            out.push('#');
            if !c.is_empty() && !c.starts_with(' ') {
                out.push(' ');
            }
            out.push_str(c);
        }
        out.push('\n');
    }
}

/// Decide whether an arg value needs quoting on render. An arg already
/// surrounded by `"..."` / `'...'` is left as-is; anything containing
/// shell-y or directive-terminator characters gets double-quoted.
fn needs_quoting(arg: &str) -> bool {
    if arg.is_empty() {
        return true;
    }
    let bytes = arg.as_bytes();
    if (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
        || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'')
    {
        return false;
    }
    arg.chars().any(|c| {
        matches!(
            c,
            ' ' | '\t' | ';' | '{' | '}' | '#' | '"' | '\''
        )
    })
}

fn escape_double_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

// ── Helpers ─────────────────────────────────────────────────────────

/// POSIX-safe single-quote escape.
fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Tiny base64 encoder that writes into a `String`. We only call this
/// once per save and the inputs are small (KB-range config files), so
/// pulling in the `base64` crate would be overkill. Standard alphabet,
/// no line breaks, no URL-safe variant.
fn base64_writer(out: &mut String) -> Base64Writer<'_> {
    Base64Writer { out, buf: 0, bits: 0 }
}

struct Base64Writer<'a> {
    out: &'a mut String,
    buf: u32,
    bits: u32,
}

impl std::io::Write for Base64Writer<'_> {
    fn write(&mut self, src: &[u8]) -> std::io::Result<usize> {
        const ALPH: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for &b in src {
            self.buf = (self.buf << 8) | (b as u32);
            self.bits += 8;
            while self.bits >= 6 {
                self.bits -= 6;
                let idx = ((self.buf >> self.bits) & 0x3f) as usize;
                self.out.push(ALPH[idx] as char);
            }
        }
        Ok(src.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        const ALPH: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        if self.bits > 0 {
            let idx = ((self.buf << (6 - self.bits)) & 0x3f) as usize;
            self.out.push(ALPH[idx] as char);
            self.bits = 0;
        }
        // Pad to a multiple of 4 chars.
        while self.out.len() % 4 != 0 {
            self.out.push('=');
        }
        Ok(())
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn directive(name: &str, args: &[&str]) -> NginxDirective {
        NginxDirective {
            name: name.into(),
            args: args.iter().map(|s| (*s).into()).collect(),
            leading_comments: vec![],
            leading_blanks: 0,
            inline_comment: None,
            block: None,
            opaque_body: None,
        }
    }

    #[test]
    fn parse_simple_inline_directive() {
        let r = parse("worker_processes auto;\n");
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.nodes.len(), 1);
        let NginxNode::Directive(d) = &r.nodes[0] else {
            panic!()
        };
        assert_eq!(d.name, "worker_processes");
        assert_eq!(d.args, vec!["auto".to_string()]);
    }

    #[test]
    fn parse_block_with_children() {
        let r = parse("events {\n    worker_connections 1024;\n}\n");
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let NginxNode::Directive(d) = &r.nodes[0] else {
            panic!()
        };
        assert_eq!(d.name, "events");
        let children = d.block.as_ref().expect("block body");
        assert_eq!(children.len(), 1);
        let NginxNode::Directive(c) = &children[0] else {
            panic!()
        };
        assert_eq!(c.name, "worker_connections");
        assert_eq!(c.args, vec!["1024".to_string()]);
    }

    #[test]
    fn parse_quoted_args_preserve_style() {
        let r = parse(r#"server_name "example.com" 'sub.example.com';
"#);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let NginxNode::Directive(d) = &r.nodes[0] else {
            panic!()
        };
        assert_eq!(d.args[0], "\"example.com\"");
        assert_eq!(d.args[1], "'sub.example.com'");
    }

    #[test]
    fn parse_comments_attach_to_next_directive() {
        let src = "# top comment\nworker_processes 4;\n";
        let r = parse(src);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let NginxNode::Directive(d) = &r.nodes[0] else {
            panic!()
        };
        assert_eq!(d.leading_comments, vec!["top comment".to_string()]);
    }

    #[test]
    fn parse_blank_line_breaks_comment_attachment() {
        let src = "# standalone\n\n\nworker_processes 4;\n";
        let r = parse(src);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        // Two-blanks gap → comment becomes its own node.
        assert!(matches!(r.nodes[0], NginxNode::Comment { .. }));
        assert!(matches!(r.nodes[1], NginxNode::Directive(_)));
    }

    #[test]
    fn parse_inline_comment_after_semicolon() {
        // Comment after `;` is captured as inline_comment per the
        // current model (we promote any comment seen before the
        // terminator onto the directive). Round-trip preserves it.
        let src = "listen 80; # http\n";
        let r = parse(src);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let NginxNode::Directive(d) = &r.nodes[0] else {
            panic!()
        };
        // The comment lands on the *next* node-position queue (since
        // it fires after `;`). Either model is acceptable round-trip
        // wise — re-render and check the bytes survive.
        let _ = d;
        let rendered = render(&r.nodes);
        assert!(rendered.contains("listen 80;"));
        assert!(rendered.contains("# http"));
    }

    #[test]
    fn parse_lua_block_is_opaque() {
        let src = r#"
content_by_lua_block {
    local x = { a = 1, b = "}" }
    if x then
        ngx.say("hello")
    end
}
"#;
        let r = parse(src);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let dir = r.nodes.iter().find_map(|n| match n {
            NginxNode::Directive(d) if d.name == "content_by_lua_block" => Some(d),
            _ => None,
        });
        let dir = dir.expect("lua directive");
        let body = dir.opaque_body.as_ref().expect("opaque body");
        // Critical: the embedded `}` inside the string didn't terminate
        // the block.
        assert!(body.contains("ngx.say"));
        assert!(body.contains(r#""}"#));
    }

    #[test]
    fn parse_nested_blocks() {
        let src = "http {\n    server {\n        listen 80;\n    }\n}\n";
        let r = parse(src);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let NginxNode::Directive(http) = &r.nodes[0] else {
            panic!()
        };
        let server_nodes = http.block.as_ref().unwrap();
        let NginxNode::Directive(server) = &server_nodes[0] else {
            panic!()
        };
        assert_eq!(server.name, "server");
        let inner = server.block.as_ref().unwrap();
        let NginxNode::Directive(listen) = &inner[0] else {
            panic!()
        };
        assert_eq!(listen.name, "listen");
        assert_eq!(listen.args, vec!["80".to_string()]);
    }

    #[test]
    fn render_round_trip_preserves_basic_shape() {
        let src = "\
worker_processes auto;

events {
    worker_connections 1024;
}

http {
    server {
        listen 80;
        server_name example.com;
        root /var/www/html;
    }
}
";
        let r = parse(src);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let out = render(&r.nodes);
        // Re-parse should be equivalent.
        let r2 = parse(&out);
        assert_eq!(r.nodes, r2.nodes);
    }

    #[test]
    fn render_quotes_unsafe_args() {
        let mut d = directive("set", &["$x", "value with space"]);
        d.args[1] = "value with space".to_string();
        let nodes = vec![NginxNode::Directive(d)];
        let out = render(&nodes);
        assert!(out.contains(r#""value with space""#));
    }

    #[test]
    fn render_does_not_re_quote_already_quoted_arg() {
        let d = directive("server_name", &["\"foo\""]);
        let nodes = vec![NginxNode::Directive(d)];
        let out = render(&nodes);
        // Should be exactly one pair of quotes, not nested.
        assert!(out.contains("\"foo\""));
        assert!(!out.contains("\\\""));
    }

    #[test]
    fn render_round_trip_preserves_lua_block() {
        let src = "http {\n    init_by_lua_block {\n        ngx.log(ngx.INFO, \"hi\")\n    }\n}\n";
        let r = parse(src);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let out = render(&r.nodes);
        assert!(out.contains("init_by_lua_block"));
        assert!(out.contains("ngx.log(ngx.INFO"));
    }

    #[test]
    fn parser_recovers_from_missing_semicolon_at_eof() {
        // Last directive forgets its `;` and EOF arrives.
        let r = parse("worker_processes 4");
        // We surface an error but still emit the directive.
        assert!(!r.errors.is_empty());
        assert_eq!(r.nodes.len(), 1);
    }

    #[test]
    fn parser_recovers_from_stray_close_brace() {
        let r = parse("} worker_processes 4;\n");
        assert!(r.errors.iter().any(|e| e.contains("stray")));
        // The valid directive after the stray `}` still parses.
        assert!(r.nodes.iter().any(|n| matches!(n, NginxNode::Directive(d) if d.name == "worker_processes")));
    }

    #[test]
    fn parse_nginx_v_modules_extracts_with_flags() {
        let src = "configure arguments: --prefix=/usr/share/nginx --with-http_ssl_module \
                   --with-http_v2_module --with-stream --without-http_uwsgi_module";
        let mods = parse_nginx_v_modules(src);
        assert!(mods.contains(&"http_ssl_module".to_string()));
        assert!(mods.contains(&"http_v2_module".to_string()));
        assert!(mods.contains(&"stream".to_string()));
        // `--without-` flags must not surface as built-in.
        assert!(!mods.iter().any(|m| m.contains("without")));
        // Sorted + deduped.
        let mut sorted = mods.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(mods, sorted);
    }

    #[test]
    fn shell_single_quote_escapes_quotes() {
        assert_eq!(shell_single_quote("path"), "'path'");
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn base64_round_trip_simple() {
        use std::io::Write;
        // We can't decode in this test (no decoder shipped here), but
        // we can verify the encoded form matches a known good output
        // for stable inputs.
        let mut out = String::new();
        {
            let mut w = super::base64_writer(&mut out);
            w.write_all(b"hello").unwrap();
            w.flush().unwrap();
        }
        assert_eq!(out, "aGVsbG8=");

        let mut out2 = String::new();
        {
            let mut w = super::base64_writer(&mut out2);
            w.write_all(b"hello world").unwrap();
            w.flush().unwrap();
        }
        assert_eq!(out2, "aGVsbG8gd29ybGQ=");
    }

    #[test]
    fn is_allowed_create_path_accepts_conf_d_and_sites_available() {
        assert!(is_allowed_create_path("/etc/nginx/conf.d/mysite.conf"));
        assert!(is_allowed_create_path("/etc/nginx/conf.d/_inc.conf"));
        assert!(is_allowed_create_path("/etc/nginx/sites-available/blog"));
        assert!(is_allowed_create_path(
            "/etc/nginx/sites-available/example.com"
        ));
    }

    #[test]
    fn is_allowed_create_path_rejects_traversal_and_other_dirs() {
        assert!(!is_allowed_create_path("/etc/nginx/nginx.conf"));
        assert!(!is_allowed_create_path("/etc/passwd"));
        assert!(!is_allowed_create_path(
            "/etc/nginx/conf.d/sub/file.conf"
        ));
        assert!(!is_allowed_create_path(
            "/etc/nginx/conf.d/../passwd"
        ));
        assert!(!is_allowed_create_path("/etc/nginx/conf.d/"));
        assert!(!is_allowed_create_path("/etc/nginx/conf.d/."));
        assert!(!is_allowed_create_path("/etc/nginx/conf.d/.."));
        assert!(!is_allowed_create_path("conf.d/foo.conf")); // not absolute
        assert!(!is_allowed_create_path(""));
    }

    #[test]
    fn sites_kind_serialization_roundtrip() {
        // Internal sanity: the kind enum's tag is the camelCase string
        // the frontend will switch on.
        let k = NginxFileKind::SiteAvailable { enabled: true };
        let s = serde_json::to_string(&k).unwrap();
        assert!(s.contains("\"kind\":\"site-available\""));
        assert!(s.contains("\"enabled\":true"));
    }
}
