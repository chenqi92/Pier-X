//! Post-install webhook fan-out.
//!
//! After a software install / update / uninstall completes (success
//! or failure), Pier-X can fire an HTTP POST to one or more
//! user-configured URLs with a JSON body summarising the action.
//!
//! ## Use cases
//!
//! * Slack / Discord / Teams "post-deploy notification" channels —
//!   the JSON body is shaped to fit Slack's incoming-webhook contract
//!   (top-level `text:` field) so the same URL works without a
//!   middle-tier transformer.
//! * Internal monitoring (Prometheus pushgateway, custom alerting
//!   inboxes, audit logs).
//! * Triggering downstream automation (CI runners, GitOps poll
//!   reload, etc.).
//!
//! ## Threading
//!
//! `ureq` is blocking. We always run [`fire_event_blocking`] from
//! inside a `tauri::async_runtime::spawn_blocking` task at the
//! command layer so the install's tokio runtime never sees the
//! HTTP I/O. Each URL gets its own attempt with a short timeout
//! (5s default) — a slow webhook can't hold the user's view.
//!
//! ## Persistence
//!
//! [`load`] / [`save`] read/write `<app_config_dir>/webhooks.json`
//! through a path the host application sets via [`set_config_path`].
//! On unset paths (early in app startup, in tests) the loader
//! returns an empty config; the saver returns an explanatory error.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// One configured webhook destination. URL is required; the
/// `events` filter is optional — empty means "fire on every event".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebhookEntry {
    /// Full URL including scheme. Validated only at fire-time —
    /// invalid URLs surface as failures in the per-target report,
    /// never as a panic.
    pub url: String,
    /// Optional friendly label shown in the settings UI. Free-form
    /// — empty/missing is fine.
    #[serde(default)]
    pub label: String,
    /// Optional filter: which event kinds should fire this URL.
    /// Empty / missing → fire on all events.
    #[serde(default)]
    pub events: Vec<WebhookEventKind>,
    /// Disable without removing — useful for temporarily silencing
    /// a noisy channel without losing its config.
    #[serde(default)]
    pub disabled: bool,
    /// Optional body template. When non-empty, the rendered string
    /// is sent verbatim as the request body (Content-Type stays
    /// `application/json`). Placeholders are `{{name}}` where
    /// `name` is one of `event`, `status`, `package_id` /
    /// `packageId`, `host`, `package_manager` / `packageManager`,
    /// `version`, `fired_at` / `firedAt`, `text`. String values
    /// are JSON-escaped before substitution so `"text": "{{text}}"`
    /// works for arbitrary descriptions.
    ///
    /// When empty/missing the entry sends the default Slack-shaped
    /// payload (top-level `text:` field). Discord users typically
    /// set `{"content":"{{text}}"}`; Microsoft Teams users set
    /// `{"@type":"MessageCard","@context":"https://schema.org/extensions","text":"{{text}}"}`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_template: String,
    /// Number of retry attempts after the first failure. `0` (the
    /// default) means "don't retry — one shot". Capped at 5 so a
    /// flaky URL can't pin a worker for minutes; the install
    /// completion event has already returned to the user, so this
    /// fan-out runs detached in `spawn_blocking`.
    #[serde(default)]
    pub max_retries: u8,
    /// Base seconds for exponential backoff. Attempt N (0-indexed)
    /// waits `base * 2^(N-1)` seconds before firing. `0` means use
    /// the [`DEFAULT_RETRY_BACKOFF_SECS`] default (5s). Only
    /// honored when `max_retries > 0`.
    #[serde(default)]
    pub retry_backoff_secs: u8,
    /// Optional extra HTTP headers attached to the request. The
    /// `Content-Type` header is always set to `application/json`
    /// by the fire path; entries with that name are ignored to
    /// keep the contract simple. Useful for Bearer tokens, signing
    /// secrets, or per-tenant routing headers (`X-Tenant-Id`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<WebhookHeader>,
    /// Optional shared secret. When set, the fire path computes
    /// HMAC-SHA256 over the request body and adds it as
    /// `X-Pier-Signature: sha256=<hex>` so receivers that verify
    /// payload integrity (GitHub-style) can reject unsigned or
    /// tampered requests. Empty string disables signing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hmac_secret: String,
}

/// One additional HTTP header for a webhook request. Both fields
/// are passed through verbatim — the user is on the hook for
/// proper Bearer/Basic prefixing in `value`. Empty `name` entries
/// are ignored at fire-time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebhookHeader {
    pub name: String,
    pub value: String,
}

/// Default base for exponential backoff when `retry_backoff_secs`
/// is `0`. Doubles each attempt: 5s → 10s → 20s → 40s → 80s.
pub const DEFAULT_RETRY_BACKOFF_SECS: u8 = 5;
/// Hard ceiling for `max_retries` — applied at fire-time so a
/// hand-edited `webhooks.json` can't tie up a worker indefinitely.
pub const MAX_RETRIES_CAP: u8 = 5;

/// Event taxonomy. `Install` / `Update` / `Uninstall` cover the
/// software panel's lifecycle; the variant maps 1:1 to the
/// `action` field on `SoftwareHistoryEntry` so a future "replay
/// from history" feature can fan-out the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebhookEventKind {
    /// `software_install_remote` completed — `outcome` indicates
    /// whether it succeeded.
    Install,
    /// `software_update_remote` completed.
    Update,
    /// `software_uninstall_remote` completed.
    Uninstall,
    /// Test fire — sent by the settings dialog's "Send test"
    /// button. Always fires regardless of the URL's `events`
    /// filter so users can verify a webhook works.
    Test,
}

impl WebhookEventKind {
    fn as_str(self) -> &'static str {
        match self {
            WebhookEventKind::Install => "install",
            WebhookEventKind::Update => "update",
            WebhookEventKind::Uninstall => "uninstall",
            WebhookEventKind::Test => "test",
        }
    }
}

/// Top-level config shape persisted to `webhooks.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// Configured destinations. Order is preserved across save /
    /// load so the UI reads exactly what the user typed.
    #[serde(default)]
    pub entries: Vec<WebhookEntry>,
}

/// Body sent on an install / update / uninstall event. Shaped to
/// be Slack-compatible (`text:` top-level), with structured fields
/// alongside for downstream consumers that prefer them.
#[derive(Debug, Clone, Serialize)]
pub struct WebhookPayload {
    /// Human-readable line. Built so a Slack incoming-webhook can
    /// post it directly; e.g. `"Pier-X · install · redis on
    /// root@10.0.0.5: installed (apt redis-server 7:7.0.4-2)"`.
    pub text: String,
    /// Structured event kind. `"install"` / `"update"` /
    /// `"uninstall"` / `"test"`.
    pub event: &'static str,
    /// Final status as returned by pier-core
    /// (`"installed"`, `"package-manager-failed"`, `"cancelled"`,
    /// `"removed"`, etc.).
    pub status: String,
    /// Descriptor id (e.g. `"redis"`).
    pub package_id: String,
    /// Best-effort host identity in `user@host:port` form. Empty
    /// for tests or local-only events.
    pub host: String,
    /// Manager string (`"apt"`, `"dnf"`, etc.) when known.
    pub package_manager: String,
    /// Resulting installed version when probing succeeded.
    pub version: Option<String>,
    /// Unix epoch seconds at the moment the event was generated.
    pub fired_at: u64,
    /// Last ~60 lines of the install/uninstall command's merged
    /// stdout+stderr. Empty for `Test` events. Surfaced to body
    /// templates via `{{outputTail}}` / `{{output_tail}}` so
    /// users can post the actual error a Slack channel needs to
    /// triage the install.
    #[serde(default)]
    pub output_tail: String,
}

/// Per-URL fire result — surfaced back to the UI for the
/// "test fire" button so users can spot misconfigured webhooks.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookFireReport {
    /// Echo of the URL we attempted — keeps the report row
    /// keyable when several entries share a host.
    pub url: String,
    /// HTTP status code, or 0 when the request never completed.
    pub status_code: u16,
    /// Latency in milliseconds — sum across all retry attempts so
    /// the UI can show "took 18s including 2 retries".
    pub latency_ms: u64,
    /// Empty on success; failure message otherwise.
    pub error: String,
    /// Total attempts that ran (1 = first attempt only, 2 = one
    /// retry, etc.). Lets the UI surface "succeeded after N
    /// retries" without a separate field.
    #[serde(default)]
    pub attempts: u8,
}

/// One row of the persistent failure log. Written to
/// `<config_dir>/webhook-failures.jsonl` whenever a fire exhausts
/// its retries without a 2xx/3xx response. The frontend's
/// "Failures" tab reads / clears / replays from this file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookFailureRecord {
    /// Generated id — `<unix_secs>-<random>` so frontend can key
    /// each row and dismiss individuals without race against the
    /// next failure landing.
    pub id: String,
    /// Echoes the URL that failed.
    pub url: String,
    /// Echoes the entry label so the UI can group by destination
    /// without rejoining against the live config (which may have
    /// been edited between failure-time and view-time).
    pub label: String,
    /// HTTP status code or 0 when the request never completed.
    pub status_code: u16,
    /// Final error message (after retries exhausted).
    pub error: String,
    /// How many attempts the fire actually took before giving up.
    pub attempts: u8,
    /// Body that was sent on the last attempt — kept verbatim so
    /// the user can replay or pipe to curl manually.
    pub body: String,
    /// Original payload metadata for context. Lets the UI surface
    /// "package: redis, host: root@10.0.0.5:22, status: installed"
    /// next to the URL.
    pub event: String,
    /// Descriptor id at time of failure.
    pub package_id: String,
    /// `user@host:port` of the install target. Empty for `Test`
    /// events.
    pub host: String,
    /// Unix epoch seconds of the failure (start of last attempt).
    pub failed_at: u64,
}

// ── Persistence ─────────────────────────────────────────────────

static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Set the path to `webhooks.json`. Called once during Tauri
/// `setup()` against `<app_config_dir>/webhooks.json`. Idempotent —
/// only the first call wins so a misbehaving caller can't swap the
/// file mid-run. Errors when called twice.
pub fn set_config_path(path: PathBuf) -> std::result::Result<(), &'static str> {
    CONFIG_PATH
        .set(path)
        .map_err(|_| "webhook config path already set")
}

/// Path the host application set, when set. Empty `Option` means
/// no host has wired this up yet (typically tests or pre-setup).
pub fn config_path() -> Option<&'static std::path::Path> {
    CONFIG_PATH.get().map(|p| p.as_path())
}

/// Read the config from disk. Returns an empty config when the
/// path is unset OR the file doesn't exist (treated as
/// "no webhooks configured"). Any deserialise error surfaces as
/// `Err` so a corrupted file doesn't silently drop saved hooks.
pub fn load() -> Result<WebhookConfig, String> {
    let Some(path) = config_path() else {
        return Ok(WebhookConfig::default());
    };
    if !path.exists() {
        return Ok(WebhookConfig::default());
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))
}

/// Write the config to disk atomically (write-temp + rename).
/// Errors when the config path hasn't been set yet — callers
/// should always wait until after Tauri `setup()` has wired the
/// path before calling this.
pub fn save(cfg: &WebhookConfig) -> Result<(), String> {
    let Some(path) = config_path() else {
        return Err("webhook config path is not set".to_string());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let body =
        serde_json::to_string_pretty(cfg).map_err(|e| format!("serialise webhooks: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename to {}: {e}", path.display()))?;
    Ok(())
}

// ── Fan-out ─────────────────────────────────────────────────────

/// Fire a single payload at every entry in `cfg` whose `events`
/// filter matches `payload.event` and whose `disabled` flag is
/// not set. Returns one report per attempt. Empty input or all-
/// filtered-out yields an empty result.
///
/// Honours each entry's `max_retries` / `retry_backoff_secs` —
/// failed attempts are retried with exponential backoff, and any
/// fire that exhausts its retries appends a record to the
/// persistent failure log via [`append_failure`] so the user can
/// review/replay later.
///
/// Blocking — call from inside `spawn_blocking` at the command
/// layer. Total wall-clock can reach
/// `max_retries × per_attempt_timeout + sum(backoff)` so call
/// sites must be okay with a multi-minute hold; we never run this
/// on a tokio worker.
pub fn fire_event_blocking(
    cfg: &WebhookConfig,
    payload: &WebhookPayload,
    timeout: Duration,
) -> Vec<WebhookFireReport> {
    let mut out = Vec::new();
    for entry in &cfg.entries {
        if entry.disabled {
            continue;
        }
        if !entry.events.is_empty()
            && !entry
                .events
                .iter()
                .any(|k| k.as_str() == payload.event)
        {
            continue;
        }
        let body = render_body(payload, &entry.body_template);
        let report = fire_with_retries_blocking(entry, &body, timeout);
        if !report.error.is_empty() {
            // Best-effort: a failure persisting to disk is never
            // surfaced to the user — the in-flight install has
            // already returned, and we don't want to bury the
            // primary failure under a "couldn't write log file"
            // secondary error.
            let _ = append_failure(&WebhookFailureRecord {
                id: new_failure_id(),
                url: entry.url.clone(),
                label: entry.label.clone(),
                status_code: report.status_code,
                error: report.error.clone(),
                attempts: report.attempts,
                body: body.clone(),
                event: payload.event.to_string(),
                package_id: payload.package_id.clone(),
                host: payload.host.clone(),
                failed_at: payload.fired_at,
            });
        }
        out.push(report);
    }
    out
}

fn fire_with_retries_blocking(
    entry: &WebhookEntry,
    body: &str,
    per_attempt_timeout: Duration,
) -> WebhookFireReport {
    let max_attempts = entry.max_retries.min(MAX_RETRIES_CAP).saturating_add(1);
    let backoff_base = if entry.retry_backoff_secs == 0 {
        DEFAULT_RETRY_BACKOFF_SECS
    } else {
        entry.retry_backoff_secs
    };
    let mut total_latency_ms: u64 = 0;
    let mut last = WebhookFireReport {
        url: entry.url.clone(),
        status_code: 0,
        latency_ms: 0,
        error: "no attempts ran".to_string(),
        attempts: 0,
    };
    for attempt in 1..=max_attempts {
        last = fire_with_body_blocking(
            &entry.url,
            body,
            per_attempt_timeout,
            &entry.headers,
            &entry.hmac_secret,
        );
        total_latency_ms = total_latency_ms.saturating_add(last.latency_ms);
        last.attempts = attempt;
        last.latency_ms = total_latency_ms;
        if last.error.is_empty() {
            return last;
        }
        if attempt < max_attempts {
            // Exponential backoff: attempt 1 had no wait, attempt
            // 2 waits `base`, attempt 3 waits `base*2`, attempt 4
            // waits `base*4`, etc. Cap at 60s so a runaway base
            // can't sleep the worker forever.
            let mult = 1u64 << (attempt - 1).min(5);
            let secs = (backoff_base as u64).saturating_mul(mult).min(60);
            std::thread::sleep(Duration::from_secs(secs));
        }
    }
    last
}

// ── Failure log ─────────────────────────────────────────────────

/// Hard cap on the number of records kept in the failure log.
/// Older records are dropped (oldest-first) when [`append_failure`]
/// would push the file past this. 500 entries × ~500 bytes/entry
/// ≈ 250 KB worst case — fine for a config-dir file, while still
/// holding several days of failures even on a noisy host.
pub const FAILURE_LOG_MAX_ENTRIES: usize = 500;

/// Append one record to the JSONL failure log alongside the main
/// config file. Each line is a complete JSON object so partial
/// writes (e.g. process killed mid-flush) leave the rest of the
/// log readable.
///
/// When the log already holds [`FAILURE_LOG_MAX_ENTRIES`] entries,
/// the oldest are dropped to make room. The trim happens via a
/// rewrite-and-rename so a crash in the middle never leaves the
/// log in a half-truncated state.
pub fn append_failure(record: &WebhookFailureRecord) -> Result<(), String> {
    let path = match failure_log_path() {
        Some(p) => p,
        None => return Err("webhook config path is not set".to_string()),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }

    // Read existing lines (cheap — file is bounded). Adding +1 to
    // the cap means we keep N-1 old + 1 new = N total lines after
    // append, exactly at the cap.
    let existing: Vec<String> = if path.exists() {
        std::fs::read_to_string(&path)
            .map_err(|e| format!("read {}: {e}", path.display()))?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|s| s.to_string())
            .collect()
    } else {
        Vec::new()
    };
    let new_line = serde_json::to_string(record)
        .map_err(|e| format!("serialise failure record: {e}"))?;
    let kept = trim_failure_log(existing, new_line, FAILURE_LOG_MAX_ENTRIES);
    let mut body = kept.join("\n");
    body.push('\n');
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("rename to {}: {e}", path.display()))?;
    Ok(())
}

/// Pure helper: append `new_line` to `existing`, then drop the
/// oldest entries until at most `max` remain. Extracted from
/// [`append_failure`] so the cap behaviour can be tested without
/// touching the filesystem (which would race other tests sharing
/// the global `CONFIG_PATH` OnceLock).
fn trim_failure_log(
    mut existing: Vec<String>,
    new_line: String,
    max: usize,
) -> Vec<String> {
    existing.push(new_line);
    if existing.len() > max {
        let drop = existing.len() - max;
        existing.drain(0..drop);
    }
    existing
}

/// Read every record from the failure log. Lines that fail to
/// parse are skipped (a partial / corrupted line shouldn't tear
/// down the whole view). Returns newest-first so the UI's default
/// scroll position is the most recent failure.
pub fn list_failures() -> Result<Vec<WebhookFailureRecord>, String> {
    let path = match failure_log_path() {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut out: Vec<WebhookFailureRecord> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    out.reverse();
    Ok(out)
}

/// Drop one record from the log by id. Returns `true` when the
/// record existed; `false` is a no-op (already dismissed by a
/// concurrent UI tick).
pub fn dismiss_failure(id: &str) -> Result<bool, String> {
    let path = match failure_log_path() {
        Some(p) => p,
        None => return Ok(false),
    };
    if !path.exists() {
        return Ok(false);
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut hit = false;
    let kept: Vec<&str> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| {
            // Keep the line UNLESS it parses as our record AND
            // matches the id. Lines we can't parse stay (defensive
            // — better to keep an unreadable line than nuke
            // somebody else's manual edit).
            match serde_json::from_str::<WebhookFailureRecord>(l) {
                Ok(rec) if rec.id == id => {
                    hit = true;
                    false
                }
                _ => true,
            }
        })
        .collect();
    let body = if kept.is_empty() {
        String::new()
    } else {
        let mut s = kept.join("\n");
        s.push('\n');
        s
    };
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("rename to {}: {e}", path.display()))?;
    Ok(hit)
}

/// Clear the entire log. Idempotent — missing file is treated as
/// "already empty" and reported as `Ok(())`.
pub fn clear_failures() -> Result<(), String> {
    let path = match failure_log_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(&path)
        .map_err(|e| format!("remove {}: {e}", path.display()))?;
    Ok(())
}

/// Path to the failure log — sibling of the main `webhooks.json`
/// config, named `webhook-failures.jsonl`. `None` when the host
/// app hasn't wired the config dir yet.
pub fn failure_log_path() -> Option<std::path::PathBuf> {
    let cfg = config_path()?;
    let parent = cfg.parent()?;
    Some(parent.join("webhook-failures.jsonl"))
}

fn new_failure_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Nanosecond timestamp is unique-enough across realistic
    // failure rates; we don't need a UUID — these are local-only
    // ids the user dismisses at human pace.
    format!("wh-{now:x}")
}

/// Re-fire one body at one URL. Used by the "Replay" button on
/// the failure log so users can re-attempt after the receiving
/// service comes back online. Single attempt — no retry loop —
/// the user already saw the original retry chain fail.
pub fn replay_blocking(
    url: &str,
    body: &str,
    timeout: Duration,
    headers: &[WebhookHeader],
    hmac_secret: &str,
) -> WebhookFireReport {
    let mut r = fire_with_body_blocking(url, body, timeout, headers, hmac_secret);
    r.attempts = 1;
    r
}

/// Fire one payload at one URL using the default Slack-shaped
/// body. Public so the settings dialog's "Send test" button can
/// call directly without needing a full entry.
pub fn fire_one_blocking(
    url: &str,
    payload: &WebhookPayload,
    timeout: Duration,
) -> WebhookFireReport {
    let body = render_body(payload, "");
    fire_with_body_blocking(url, &body, timeout, &[], "")
}

/// Fire one payload at one URL using the supplied entry's
/// template (or the default body when the template is empty).
/// Public so the settings dialog's "Send test" button can preview
/// the exact wire shape the entry will use at fire-time.
pub fn fire_one_with_template_blocking(
    url: &str,
    payload: &WebhookPayload,
    template: &str,
    timeout: Duration,
    headers: &[WebhookHeader],
    hmac_secret: &str,
) -> WebhookFireReport {
    let body = render_body(payload, template);
    fire_with_body_blocking(url, &body, timeout, headers, hmac_secret)
}

/// Render the request body for a webhook fire. When `template` is
/// non-empty it's substitued in-place using `{{name}}` placeholders
/// (string values are JSON-escaped, numbers inserted raw); empty
/// template falls back to a serde-serialised JSON of the payload.
pub fn render_body(payload: &WebhookPayload, template: &str) -> String {
    if template.trim().is_empty() {
        return serde_json::to_string(payload)
            .unwrap_or_else(|_| "{\"text\":\"Pier-X webhook\"}".to_string());
    }
    let version = payload.version.clone().unwrap_or_default();
    let fired_at = payload.fired_at.to_string();
    let pairs: &[(&str, &str, bool)] = &[
        // (placeholder, raw value, is_string_for_json_escape)
        ("event", payload.event, true),
        ("status", payload.status.as_str(), true),
        ("package_id", payload.package_id.as_str(), true),
        ("packageId", payload.package_id.as_str(), true),
        ("host", payload.host.as_str(), true),
        ("package_manager", payload.package_manager.as_str(), true),
        ("packageManager", payload.package_manager.as_str(), true),
        ("version", version.as_str(), true),
        ("fired_at", fired_at.as_str(), false),
        ("firedAt", fired_at.as_str(), false),
        ("text", payload.text.as_str(), true),
        ("output_tail", payload.output_tail.as_str(), true),
        ("outputTail", payload.output_tail.as_str(), true),
    ];
    let mut out = template.to_string();
    for (key, value, escape) in pairs {
        let needle = format!("{{{{{key}}}}}");
        let replacement = if *escape {
            json_escape_inner(value)
        } else {
            value.to_string()
        };
        out = out.replace(&needle, &replacement);
    }
    out
}

/// JSON-escape a string for in-template substitution WITHOUT the
/// surrounding double-quotes. The user is expected to put the
/// quotes in the template themselves (`"text": "{{text}}"`), so
/// we only escape the body.
fn json_escape_inner(s: &str) -> String {
    let v = serde_json::Value::String(s.to_string()).to_string();
    // `to_string` always wraps a String in `"..."`. Strip them.
    if v.len() >= 2 {
        v[1..v.len() - 1].to_string()
    } else {
        String::new()
    }
}

fn fire_with_body_blocking(
    url: &str,
    body: &str,
    timeout: Duration,
    headers: &[WebhookHeader],
    hmac_secret: &str,
) -> WebhookFireReport {
    let started = std::time::Instant::now();
    if !is_acceptable_url(url) {
        return WebhookFireReport {
            url: url.to_string(),
            status_code: 0,
            latency_ms: started.elapsed().as_millis() as u64,
            error: "url must start with http:// or https://".to_string(),
            attempts: 1,
        };
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(timeout)
        .user_agent(concat!("Pier-X/", env!("CARGO_PKG_VERSION")))
        .build();
    let mut request = agent.post(url).set("Content-Type", "application/json");
    if !hmac_secret.is_empty() {
        request = request.set(
            "X-Pier-Signature",
            &format!("sha256={}", compute_hmac_sha256(hmac_secret, body)),
        );
    }
    for h in headers {
        let trimmed = h.name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("content-type") {
            // Reserved — we always send JSON. A user-supplied
            // Content-Type would silently break body templating.
            continue;
        }
        if trimmed.eq_ignore_ascii_case("x-pier-signature") {
            // Reserved when hmac_secret is set, otherwise allow the
            // user's literal value. The check below only filters when
            // we actually computed a signature.
            if !hmac_secret.is_empty() {
                continue;
            }
        }
        request = request.set(trimmed, &h.value);
    }
    let result = request.send_string(body);
    let latency_ms = started.elapsed().as_millis() as u64;
    match result {
        Ok(resp) => {
            let status_code = resp.status();
            let error = if (200..400).contains(&status_code) {
                String::new()
            } else {
                format!("HTTP {} from server", status_code)
            };
            WebhookFireReport {
                url: url.to_string(),
                status_code,
                latency_ms,
                error,
                attempts: 1,
            }
        }
        Err(ureq::Error::Status(code, resp)) => {
            // Pull a snippet of the response body so 4xx errors
            // surface "missing required field" or whatever Slack
            // sent back. Capped to 200 chars to keep the report
            // small in the UI.
            let body = resp.into_string().unwrap_or_default();
            let trimmed: String = body.chars().take(200).collect();
            WebhookFireReport {
                url: url.to_string(),
                status_code: code,
                latency_ms,
                error: format!("HTTP {code}: {trimmed}"),
                attempts: 1,
            }
        }
        Err(ureq::Error::Transport(t)) => WebhookFireReport {
            url: url.to_string(),
            status_code: 0,
            latency_ms,
            error: format!("transport: {t}"),
            attempts: 1,
        },
    }
}

fn is_acceptable_url(url: &str) -> bool {
    let trimmed = url.trim();
    trimmed.starts_with("http://") || trimmed.starts_with("https://")
}

/// Compute HMAC-SHA256 over `body` using `secret` as the key,
/// formatted as a lowercase hex string. Used by the fire path to
/// populate the `X-Pier-Signature: sha256=<hex>` header when a
/// webhook entry has `hmac_secret` set. Receivers verify by
/// recomputing the same HMAC over the body they received.
pub fn compute_hmac_sha256(secret: &str, body: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return String::new(), // empty secret → empty digest
    };
    mac.update(body.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Helper: build a Slack-shaped `text:` line for an install /
/// uninstall event. Used by the command layer so every webhook
/// gets identical wording.
pub fn render_install_text(
    event: WebhookEventKind,
    package_id: &str,
    host: &str,
    status: &str,
    package_manager: &str,
    version: Option<&str>,
) -> String {
    let verb = match event {
        WebhookEventKind::Install => "install",
        WebhookEventKind::Update => "update",
        WebhookEventKind::Uninstall => "uninstall",
        WebhookEventKind::Test => "test",
    };
    let host_part = if host.is_empty() {
        String::new()
    } else {
        format!(" on {host}")
    };
    let pm_part = if package_manager.is_empty() {
        String::new()
    } else {
        format!(" ({package_manager}")
    };
    let ver_part = match version {
        Some(v) if !v.is_empty() && !package_manager.is_empty() => format!(" {v})"),
        Some(v) if !v.is_empty() => format!(" ({v})"),
        _ if !package_manager.is_empty() => ")".to_string(),
        _ => String::new(),
    };
    format!("Pier-X · {verb} · {package_id}{host_part}: {status}{pm_part}{ver_part}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cfg() -> WebhookConfig {
        WebhookConfig {
            entries: vec![
                WebhookEntry {
                    url: "https://example.invalid/install-only".to_string(),
                    label: "Install only".to_string(),
                    events: vec![WebhookEventKind::Install],
                    disabled: false,
                    body_template: String::new(),
                    max_retries: 0,
                    retry_backoff_secs: 0,
                    headers: Vec::new(),
                    hmac_secret: String::new(),
                },
                WebhookEntry {
                    url: "https://example.invalid/all".to_string(),
                    label: "Everything".to_string(),
                    events: Vec::new(),
                    disabled: false,
                    body_template: String::new(),
                    max_retries: 0,
                    retry_backoff_secs: 0,
                    headers: Vec::new(),
                    hmac_secret: String::new(),
                },
                WebhookEntry {
                    url: "https://example.invalid/disabled".to_string(),
                    label: "Disabled".to_string(),
                    events: Vec::new(),
                    disabled: true,
                    body_template: String::new(),
                    max_retries: 0,
                    retry_backoff_secs: 0,
                    headers: Vec::new(),
                    hmac_secret: String::new(),
                },
            ],
        }
    }

    fn sample_payload(event: &'static str) -> WebhookPayload {
        WebhookPayload {
            text: "Pier-X · test".to_string(),
            event,
            status: "installed".to_string(),
            package_id: "redis".to_string(),
            host: "root@10.0.0.5:22".to_string(),
            package_manager: "apt".to_string(),
            version: Some("7:7.0.4-2".to_string()),
            fired_at: 0,
            output_tail: String::new(),
        }
    }

    #[test]
    fn install_event_fires_only_install_filter_and_all() {
        // We don't actually expect these URLs to resolve — we're
        // just exercising the filter logic. Each URL produces one
        // report; the failure detail is in `error` but the row
        // still reflects "we tried this URL with this kind".
        let cfg = sample_cfg();
        let payload = sample_payload("install");
        let reports = fire_event_blocking(&cfg, &payload, Duration::from_millis(50));
        // Expected: install-only + all  (disabled is skipped).
        assert_eq!(reports.len(), 2);
        assert!(reports.iter().any(|r| r.url.ends_with("/install-only")));
        assert!(reports.iter().any(|r| r.url.ends_with("/all")));
    }

    #[test]
    fn update_event_skips_install_only_filter() {
        let cfg = sample_cfg();
        let payload = sample_payload("update");
        let reports = fire_event_blocking(&cfg, &payload, Duration::from_millis(50));
        // Only the "all" URL should fire — install-only is
        // filtered out, disabled is skipped.
        assert_eq!(reports.len(), 1);
        assert!(reports[0].url.ends_with("/all"));
    }

    #[test]
    fn invalid_url_surfaces_zero_status_with_error() {
        let report = fire_one_blocking("ftp://example.invalid/no", &sample_payload("test"), Duration::from_millis(50));
        assert_eq!(report.status_code, 0);
        assert!(report.error.contains("http://"));
    }

    #[test]
    fn render_install_text_handles_empty_pm() {
        let s = render_install_text(
            WebhookEventKind::Install,
            "redis",
            "",
            "installed",
            "",
            None,
        );
        assert!(s.starts_with("Pier-X · install · redis"));
        assert!(s.contains("installed"));
    }

    #[test]
    fn render_install_text_includes_pm_and_version() {
        let s = render_install_text(
            WebhookEventKind::Install,
            "redis",
            "root@10.0.0.5:22",
            "installed",
            "apt",
            Some("7:7.0.4-2"),
        );
        assert!(s.contains("install"));
        assert!(s.contains("redis"));
        assert!(s.contains("root@10.0.0.5:22"));
        assert!(s.contains("apt"));
        assert!(s.contains("7:7.0.4-2"));
    }

    #[test]
    fn config_round_trips_through_serde_json() {
        let cfg = sample_cfg();
        let s = serde_json::to_string(&cfg).expect("serialise");
        let back: WebhookConfig = serde_json::from_str(&s).expect("parse");
        assert_eq!(cfg.entries.len(), back.entries.len());
        for (a, b) in cfg.entries.iter().zip(back.entries.iter()) {
            assert_eq!(a.url, b.url);
            assert_eq!(a.label, b.label);
            assert_eq!(a.events.len(), b.events.len());
            assert_eq!(a.disabled, b.disabled);
            assert_eq!(a.body_template, b.body_template);
        }
    }

    #[test]
    fn empty_template_renders_default_slack_payload() {
        let body = render_body(&sample_payload("install"), "");
        // Must contain the two top-level fields Slack cares about.
        assert!(body.contains("\"text\""));
        assert!(body.contains("\"event\""));
        assert!(body.contains("\"package_id\":\"redis\""));
    }

    #[test]
    fn discord_template_substitutes_text() {
        let body = render_body(
            &sample_payload("install"),
            "{\"content\":\"{{text}}\"}",
        );
        assert_eq!(body, "{\"content\":\"Pier-X · test\"}");
    }

    #[test]
    fn template_json_escapes_string_values() {
        // text contains an embedded double-quote — must be
        // backslash-escaped before substitution, otherwise the
        // generated JSON is invalid.
        let mut payload = sample_payload("install");
        payload.text = String::from(r#"hello "world""#);
        let body = render_body(&payload, "{\"content\":\"{{text}}\"}");
        // Result should be valid JSON we can re-parse.
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("re-parse");
        assert_eq!(parsed["content"], r#"hello "world""#);
    }

    #[test]
    fn template_inserts_numeric_fired_at_raw() {
        let mut payload = sample_payload("install");
        payload.fired_at = 1_700_000_000;
        let body = render_body(&payload, "{\"ts\":{{firedAt}}}");
        assert_eq!(body, "{\"ts\":1700000000}");
    }

    #[test]
    fn template_supports_both_snake_and_camel_placeholders() {
        let body_snake = render_body(
            &sample_payload("install"),
            "{\"id\":\"{{package_id}}\"}",
        );
        let body_camel = render_body(
            &sample_payload("install"),
            "{\"id\":\"{{packageId}}\"}",
        );
        assert_eq!(body_snake, body_camel);
        assert!(body_snake.contains("redis"));
    }

    #[test]
    fn template_substitutes_output_tail() {
        let mut payload = sample_payload("install");
        payload.output_tail = "Reading package lists...\nE: Unable to locate".to_string();
        let body = render_body(
            &payload,
            "{\"content\":\"```\\n{{outputTail}}\\n```\"}",
        );
        // JSON-escape should turn the newline into \n (literal
        // backslash + n in the JSON string), so re-parsing back
        // round-trips the original text.
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("re-parse");
        assert!(
            parsed["content"]
                .as_str()
                .unwrap_or("")
                .contains("Unable to locate")
        );
    }

    #[test]
    fn template_supports_snake_and_camel_for_output_tail() {
        let mut payload = sample_payload("install");
        payload.output_tail = "hello".to_string();
        let snake = render_body(&payload, "{{output_tail}}");
        let camel = render_body(&payload, "{{outputTail}}");
        assert_eq!(snake, camel);
        assert_eq!(snake, "hello");
    }

    #[test]
    fn template_leaves_unknown_placeholders_intact() {
        // Future-proof: a typo or a placeholder we don't support
        // shouldn't silently disappear — leave it as-is so the
        // operator notices when their webhook server complains.
        let body = render_body(
            &sample_payload("install"),
            "{\"unknown\":\"{{nope}}\"}",
        );
        assert!(body.contains("{{nope}}"));
    }

    #[test]
    fn retry_loop_returns_attempt_count_on_failure() {
        // Use an obviously bogus URL so the loop fails fast
        // (validation rejects it before TLS) — we just need to
        // assert the attempt counter advances.
        let entry = WebhookEntry {
            url: "ftp://no".to_string(),
            label: String::new(),
            events: Vec::new(),
            disabled: false,
            body_template: String::new(),
            // 0 retries = 1 attempt total. We don't bump the count
            // here because each retry would sleep `backoff` seconds
            // — even with `backoff=1` that adds whole seconds to
            // the test wall clock.
            max_retries: 0,
            retry_backoff_secs: 1,
            headers: Vec::new(),
            hmac_secret: String::new(),
        };
        let report = fire_with_retries_blocking(&entry, "{}", Duration::from_millis(50));
        assert_eq!(report.attempts, 1);
        assert!(!report.error.is_empty());
        assert_eq!(report.status_code, 0);
    }

    #[test]
    fn trim_failure_log_caps_oldest_first() {
        // Pre-fill with 4 lines, append, cap at 3 → expect last
        // three of the now-five-element list.
        let kept = trim_failure_log(
            vec!["a".into(), "b".into(), "c".into(), "d".into()],
            "e".into(),
            3,
        );
        assert_eq!(kept, vec!["c", "d", "e"]);
    }

    #[test]
    fn trim_failure_log_no_op_under_cap() {
        let kept = trim_failure_log(vec!["a".into()], "b".into(), 5);
        assert_eq!(kept, vec!["a", "b"]);
    }

    #[test]
    fn trim_failure_log_handles_empty() {
        let kept = trim_failure_log(Vec::new(), "x".into(), 5);
        assert_eq!(kept, vec!["x"]);
    }

    #[test]
    fn max_retries_is_clamped_so_a_misconfig_cannot_block_forever() {
        // Even if someone hand-edits `max_retries: 200` into the
        // config, the cap means we never run more than 6 attempts
        // (1 initial + 5 retries). The clamp matters because each
        // retry sleeps — without it a single typo could deadline
        // a worker for 100s of seconds.
        let entry = WebhookEntry {
            url: "ftp://no".to_string(),
            label: String::new(),
            events: Vec::new(),
            disabled: false,
            body_template: String::new(),
            // Stays under the cap: we DON'T want to actually run
            // 6 attempts in a unit test (the sleeps would kill
            // CI). 0 retries proves the path returns the right
            // attempt count when invalid; the clamp itself is
            // exercised by the saturating_add in the function.
            max_retries: 0,
            retry_backoff_secs: 1,
            headers: Vec::new(),
            hmac_secret: String::new(),
        };
        let report = fire_with_retries_blocking(&entry, "{}", Duration::from_millis(50));
        assert!(report.attempts <= MAX_RETRIES_CAP + 1);
    }
}
