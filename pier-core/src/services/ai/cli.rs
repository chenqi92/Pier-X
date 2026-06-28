//! Local agent-CLI backend (PRODUCT-SPEC §5.14.8).
//!
//! Drives the user's already-installed, already-logged-in agent CLI
//! (Claude Code / Codex) as a subprocess, reusing their subscription
//! login — no API key, no proxy. Two modes:
//!
//!   * **M1 model backend** ([`CliMode::ModelBackend`], default): the
//!     CLI runs with its OWN tools DISABLED and only produces text, so
//!     Pier-X's risk-gated tool loop (`src-tauri/src/ai.rs`) is
//!     untouched. Maps to "ask / explain / suggest, insert-don't-run".
//!   * **M2a native agent** ([`CliMode::NativeAgent`]): the CLI runs
//!     its OWN loop + tools in the tab's LOCAL working dir and
//!     self-governs (its own sandbox / permission model). Pier-X
//!     renders the transcript read-only and executes nothing. Opt-in,
//!     local tab only (the SSH refusal lives in `ai.rs`).
//!
//! Either way the CLI's stdout JSON stream is adapted into a
//! [`TurnOutcome`] with EMPTY tool_calls, so `run_turn` never tries to
//! execute anything itself.
//!
//! Cancellation: a watcher thread kills the child when the
//! `CancellationToken` fires — abandoning the worker is not enough, the
//! subprocess would keep running.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::types::{
    AiError, ChatMessage, ChatRole, CliFlavor, CliMode, ProviderConfig, StopKind, ToolSpec,
    TurnOutcome,
};

fn flavor_of(cfg: &ProviderConfig) -> CliFlavor {
    cfg.cli_flavor.unwrap_or(CliFlavor::ClaudeCode)
}

fn default_bin(flavor: CliFlavor) -> &'static str {
    match flavor {
        CliFlavor::ClaudeCode => "claude",
        CliFlavor::Codex => "codex",
    }
}

fn resolve_bin(cfg: &ProviderConfig) -> String {
    cfg.cli_bin
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_bin(flavor_of(cfg)).to_string())
}

/// Build a `Command` for the CLI binary, wrapping a Windows PowerShell
/// (`.ps1`) or batch (`.cmd` / `.bat`) script in its interpreter so it
/// can be spawned directly — e.g. scoop installs Codex as `codex.ps1`.
/// Callers append the CLI's own args afterwards.
fn cli_command(bin: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        let lower = bin.to_ascii_lowercase();
        if lower.ends_with(".ps1") {
            let mut c = Command::new("powershell");
            c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", bin]);
            return c;
        }
        if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            let mut c = Command::new("cmd");
            c.args(["/C", bin]);
            return c;
        }
    }
    Command::new(bin)
}

/// Truncate to `max` chars (char-safe — JSON args may be multibyte).
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Argv for one turn, by `cfg.cli_mode`: M1 tool-less text, M2a native
/// agent (CLI self-governs), M2b gated agent (permission/MCP flags come
/// from `cfg.cli_extra_args`). Extra args are appended verbatim.
fn build_args(flavor: CliFlavor, cfg: &ProviderConfig) -> Vec<String> {
    let model = cfg.model.trim();
    let mode = cfg.cli_mode;
    let mut a = match flavor {
        CliFlavor::ClaudeCode => {
            let mut a = vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
                "--no-session-persistence".to_string(),
            ];
            match mode {
                CliMode::ModelBackend => {
                    // `--tools ""` disables ALL native tools => pure text
                    // (verified 2026-06-25: init event reports `tools:[]`).
                    a.push("--tools".to_string());
                    a.push(String::new());
                }
                CliMode::NativeAgent => {
                    // M2a: Claude runs its OWN tools, auto-accepting edits;
                    // Claude's own permission model governs.
                    a.push("--permission-mode".to_string());
                    a.push("acceptEdits".to_string());
                }
                CliMode::GatedAgent => {
                    // M2b: --permission-mode default + the MCP gate flags
                    // are supplied via cli_extra_args by the shell layer.
                }
            }
            if !model.is_empty() {
                a.push("--model".to_string());
                a.push(model.to_string());
            }
            a
        }
        CliFlavor::Codex => {
            // NOTE: codex `exec --json` event schema is NOT verified on a
            // live run (CLI version drift blocked it, see §5.14.8); the
            // parser below is best-effort. Prompt read from stdin (`-`).
            // Codex has no MCP permission tool wired here, so M2b ~ M2a.
            let mut a = vec![
                "exec".to_string(),
                "--json".to_string(),
                "--sandbox".to_string(),
            ];
            if mode == CliMode::ModelBackend {
                a.push("read-only".to_string());
            } else {
                a.push("workspace-write".to_string());
                a.push("--ask-for-approval".to_string());
                a.push("never".to_string());
            }
            a.push("--skip-git-repo-check".to_string());
            a.push("--color".to_string());
            a.push("never".to_string());
            if !model.is_empty() {
                a.push("-m".to_string());
                a.push(model.to_string());
            }
            a.push("-".to_string());
            a
        }
    };
    a.extend(cfg.cli_extra_args.iter().cloned());
    a
}

/// Flatten system + history into one prompt string fed on stdin.
fn compose_prompt(system: &str, messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    if !system.trim().is_empty() {
        out.push_str(system.trim());
        out.push_str("\n\n");
    }
    for m in messages {
        match m.role {
            ChatRole::System => {
                out.push_str(&m.content);
                out.push_str("\n\n");
            }
            ChatRole::User => {
                out.push_str("User: ");
                out.push_str(&m.content);
                out.push_str("\n\n");
            }
            ChatRole::Assistant => {
                if !m.content.is_empty() {
                    out.push_str("Assistant: ");
                    out.push_str(&m.content);
                    out.push_str("\n\n");
                }
            }
            // M1 is tool-less; M2a folds tool steps into the assistant
            // transcript, so there are no separate tool turns to forward.
            ChatRole::Tool => {}
        }
    }
    out
}

#[derive(Default)]
struct CliAcc {
    text: String,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    error: Option<String>,
}

fn tool_result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn parse_claude_line(
    line: &str,
    on_delta: &mut dyn FnMut(&str),
    acc: &mut CliAcc,
    render_tools: bool,
) {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return, // tolerate non-JSON warning lines
    };
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "assistant" => {
            if let Some(content) = v.pointer("/message/content").and_then(Value::as_array) {
                for block in content {
                    match block.get("type").and_then(Value::as_str).unwrap_or("") {
                        "text" => {
                            if let Some(t) = block.get("text").and_then(Value::as_str) {
                                if !t.is_empty() {
                                    acc.text.push_str(t);
                                    on_delta(t);
                                }
                            }
                        }
                        // M2a: surface the CLI's own tool steps read-only.
                        "tool_use" if render_tools => {
                            let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                            let input = clip(
                                &block
                                    .get("input")
                                    .map(|i| i.to_string())
                                    .unwrap_or_default(),
                                200,
                            );
                            let l = format!("\n\n🔧 **{name}** `{input}`\n");
                            acc.text.push_str(&l);
                            on_delta(&l);
                        }
                        _ => {}
                    }
                }
            }
        }
        // M2a: tool results come back on a `user` event.
        "user" if render_tools => {
            if let Some(content) = v.pointer("/message/content").and_then(Value::as_array) {
                for block in content {
                    if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                        let text = clip(tool_result_text(block).trim(), 240);
                        if !text.is_empty() {
                            let l = format!("\n↳ {text}\n");
                            acc.text.push_str(&l);
                            on_delta(&l);
                        }
                    }
                }
            }
        }
        "result" => {
            if acc.text.is_empty() {
                if let Some(r) = v.get("result").and_then(Value::as_str) {
                    if !r.is_empty() {
                        acc.text.push_str(r);
                        on_delta(r);
                    }
                }
            }
            acc.input_tokens = v
                .pointer("/usage/input_tokens")
                .and_then(Value::as_u64)
                .or(acc.input_tokens);
            acc.output_tokens = v
                .pointer("/usage/output_tokens")
                .and_then(Value::as_u64)
                .or(acc.output_tokens);
            if v.get("is_error").and_then(Value::as_bool) == Some(true) {
                acc.error = Some(
                    v.get("result")
                        .and_then(Value::as_str)
                        .unwrap_or("cli backend reported an error")
                        .to_string(),
                );
            }
        }
        "error" => {
            acc.error = Some(
                v.get("error")
                    .and_then(Value::as_str)
                    .or_else(|| v.pointer("/error/message").and_then(Value::as_str))
                    .unwrap_or("cli backend stream error")
                    .to_string(),
            );
        }
        _ => {}
    }
}

fn parse_codex_line(
    line: &str,
    on_delta: &mut dyn FnMut(&str),
    acc: &mut CliAcc,
    render_tools: bool,
) {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return,
    };
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "item.completed" => {
            let item = v.get("item");
            let itype = item
                .and_then(|i| i.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if itype.contains("message") {
                if let Some(t) = item.and_then(|i| {
                    i.get("text")
                        .or_else(|| i.get("content"))
                        .and_then(Value::as_str)
                }) {
                    if !t.is_empty() {
                        acc.text.push_str(t);
                        on_delta(t);
                    }
                }
            } else if render_tools
                && (itype.contains("command") || itype.contains("exec") || itype.contains("shell"))
            {
                // M2a best-effort: surface the executed command.
                if let Some(c) = item.and_then(|i| {
                    i.get("command")
                        .or_else(|| i.get("text"))
                        .and_then(Value::as_str)
                }) {
                    let l = format!("\n\n🔧 {}\n", clip(c, 200));
                    acc.text.push_str(&l);
                    on_delta(&l);
                }
            }
        }
        "turn.completed" => {
            acc.input_tokens = v
                .pointer("/usage/input_tokens")
                .and_then(Value::as_u64)
                .or(acc.input_tokens);
            acc.output_tokens = v
                .pointer("/usage/output_tokens")
                .and_then(Value::as_u64)
                .or(acc.output_tokens);
        }
        "error" | "turn.failed" => {
            acc.error = Some(
                v.get("message")
                    .and_then(Value::as_str)
                    .or_else(|| v.pointer("/error/message").and_then(Value::as_str))
                    .unwrap_or("codex error")
                    .to_string(),
            );
        }
        _ => {}
    }
}

/// Run one turn via the agent CLI. M1 (tool-less) or M2a (native agent)
/// per `cfg.cli_mode`. Returns text + EMPTY tool_calls — the CLI does
/// its own work (M2a) or only chats (M1); Pier-X never executes here.
pub fn stream_cli(
    cfg: &ProviderConfig,
    system: &str,
    messages: &[ChatMessage],
    _tools: &[ToolSpec],
    on_delta: &mut dyn FnMut(&str),
    cancel: &CancellationToken,
) -> Result<TurnOutcome, AiError> {
    // M1 keeps Pier-X's own risk-gated tools, so `_tools` is not
    // forwarded; M2a lets the CLI use its own tools (rendered read-only).
    let flavor = flavor_of(cfg);
    let native = cfg.cli_mode != CliMode::ModelBackend;
    let bin = resolve_bin(cfg);
    let prompt = compose_prompt(system, messages);

    let mut cmd = cli_command(&bin);
    cmd.args(build_args(flavor, cfg));
    if native {
        if let Some(dir) = cfg
            .cli_cwd
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            cmd.current_dir(dir);
        }
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::process_util::configure_background_command(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| AiError::Http(format!("failed to launch {bin}: {e}")))?;

    // Feed the prompt on a dedicated thread so a large prompt can't
    // deadlock against the child's stdout we're about to read.
    if let Some(mut stdin) = child.stdin.take() {
        std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
            // stdin dropped here -> EOF
        });
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AiError::Protocol("cli backend: no stdout pipe".into()))?;

    // Drain stderr off-thread (avoid pipe-buffer deadlock; surfaced on error).
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    let stderr_handle = child.stderr.take().map(|mut e| {
        let buf = stderr_buf.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            if let Ok(mut b) = buf.lock() {
                *b = s;
            }
        })
    });

    let child = Arc::new(Mutex::new(child));
    let done = Arc::new(AtomicBool::new(false));
    let killer = {
        let child = child.clone();
        let cancel = cancel.clone();
        let done = done.clone();
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if cancel.is_cancelled() {
                    if let Ok(mut c) = child.lock() {
                        let _ = c.kill();
                    }
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    };

    let mut acc = CliAcc::default();
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        if cancel.is_cancelled() {
            break;
        }
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        match flavor {
            CliFlavor::ClaudeCode => parse_claude_line(&line, on_delta, &mut acc, native),
            CliFlavor::Codex => parse_codex_line(&line, on_delta, &mut acc, native),
        }
    }

    // Stop the killer, then ensure the child is reaped (and killed on cancel
    // — the killer has already joined, so no lock contention with wait()).
    done.store(true, Ordering::Relaxed);
    let _ = killer.join();
    if cancel.is_cancelled() {
        if let Ok(mut c) = child.lock() {
            let _ = c.kill();
        }
    }
    let status = child.lock().ok().and_then(|mut c| c.wait().ok());
    if let Some(h) = stderr_handle {
        let _ = h.join();
    }

    if cancel.is_cancelled() {
        return Err(AiError::Cancelled);
    }
    if let Some(err) = acc.error.take() {
        return Err(AiError::Protocol(err));
    }
    let ok = status.map(|s| s.success()).unwrap_or(false);
    if !ok && acc.text.is_empty() {
        let mut msg = stderr_buf
            .lock()
            .ok()
            .map(|b| b.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{bin} exited without output"));
        msg.truncate(800);
        return Err(AiError::Http(msg));
    }

    Ok(TurnOutcome {
        text: acc.text,
        tool_calls: Vec::new(),
        input_tokens: acc.input_tokens,
        output_tokens: acc.output_tokens,
        stop: StopKind::EndTurn,
    })
}

/// Model ids offered in the settings dropdown (free-text entry also works).
pub fn known_models(cfg: &ProviderConfig) -> Vec<String> {
    match flavor_of(cfg) {
        CliFlavor::ClaudeCode => {
            vec![
                "opus".to_string(),
                "sonnet".to_string(),
                "haiku".to_string(),
            ]
        }
        CliFlavor::Codex => vec!["gpt-5.1-codex".to_string(), "gpt-5-codex".to_string()],
    }
}

/// Probe the CLI via `<bin> --version` (cheap; no tokens, no network).
pub fn test_connection(cfg: &ProviderConfig) -> Result<String, AiError> {
    let flavor = flavor_of(cfg);
    let bin = resolve_bin(cfg);
    let mut cmd = cli_command(&bin);
    cmd.arg("--version");
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::process_util::configure_background_command(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| AiError::Http(format!("failed to launch {bin}: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(AiError::Http(format!(
            "{bin} --version failed: {}",
            err.trim()
        )));
    }
    let ver_raw = String::from_utf8_lossy(&out.stdout);
    let ver = ver_raw.lines().next().unwrap_or("").trim();
    Ok(format!("ok · {} · {ver}", default_bin(flavor)))
}

// ── Detection (settings "Detect" button, §5.14.8) ──────────────────

/// Result of probing for an installed agent CLI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliDetect {
    /// Whether a working binary was found.
    pub found: bool,
    /// The binary string that responded to `--version` (bare name if it
    /// resolved on PATH, else an absolute path).
    pub path: String,
    /// First line of `--version` output.
    pub version: String,
}

fn home_dir() -> Option<String> {
    std::env::var("USERPROFILE")
        .ok()
        .or_else(|| std::env::var("HOME").ok())
        .filter(|s| !s.is_empty())
}

/// Resolve a bare name to full paths via the OS resolver, so `.cmd` /
/// `.ps1` / `.exe` shims a bare `Command::new` would miss on Windows
/// (e.g. scoop's `codex.cmd` / `codex.ps1`) are still found.
fn resolve_on_path(name: &str) -> Vec<String> {
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("where");
        c.arg(name);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(format!("command -v {name}"));
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::process_util::configure_background_command(&mut cmd);
    let Ok(out) = cmd.output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut paths: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    // Prefer fast, non-stalling shims: .exe, then .cmd/.bat, then .ps1
    // (nested PowerShell is slow / can hang), then extension-less last
    // (unrunnable as a bare Windows process).
    paths.sort_by_key(|p| {
        let l = p.to_ascii_lowercase();
        if l.ends_with(".exe") {
            0
        } else if l.ends_with(".cmd") || l.ends_with(".bat") {
            1
        } else if l.ends_with(".ps1") {
            2
        } else {
            3
        }
    });
    paths
}

fn detect_candidates(flavor: CliFlavor) -> Vec<String> {
    let name = default_bin(flavor);
    let mut v = vec![name.to_string()];
    v.extend(resolve_on_path(name));
    let Some(home) = home_dir() else {
        return v;
    };
    let sep = std::path::MAIN_SEPARATOR;
    let join = |parts: &[&str]| -> String {
        let mut p = home.clone();
        for part in parts {
            p.push(sep);
            p.push_str(part);
        }
        p
    };
    if cfg!(target_os = "windows") {
        match flavor {
            CliFlavor::ClaudeCode => {
                v.push(join(&[".local", "bin", "claude.exe"]));
                v.push(join(&["AppData", "Roaming", "npm", "claude.cmd"]));
                v.push(join(&["scoop", "shims", "claude.exe"]));
            }
            CliFlavor::Codex => {
                v.push(join(&["scoop", "shims", "codex.exe"]));
                v.push(join(&["scoop", "shims", "codex.cmd"]));
                v.push(join(&["scoop", "shims", "codex.ps1"]));
                v.push(join(&["AppData", "Roaming", "npm", "codex.cmd"]));
            }
        }
    } else {
        v.push(join(&[".local", "bin", name]));
        v.push(format!("/usr/local/bin/{name}"));
        v.push(format!("/opt/homebrew/bin/{name}"));
        v.push(join(&[".npm-global", "bin", name]));
        if matches!(flavor, CliFlavor::ClaudeCode) {
            v.push(join(&[".claude", "local", "claude"]));
        }
    }
    v
}

fn probe_version(bin: &str) -> Option<String> {
    let mut cmd = cli_command(bin);
    cmd.arg("--version");
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::process_util::configure_background_command(&mut cmd);
    let mut child = cmd.spawn().ok()?;
    // Bound the probe — some Windows shims (.ps1 via PowerShell) can
    // stall; never let detection hang on one candidate.
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) => {
                if start.elapsed() > Duration::from_secs(6) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
    // `--version` output is tiny, so reading after exit can't deadlock.
    let mut s = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut s);
    }
    let line = s.lines().next().unwrap_or("").trim().to_string();
    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}

/// Probe PATH + common install locations for the flavor's binary. Stops
/// at the first candidate that answers `--version`.
pub fn detect(flavor: CliFlavor) -> CliDetect {
    for cand in detect_candidates(flavor) {
        if let Some(version) = probe_version(&cand) {
            return CliDetect {
                found: true,
                path: cand,
                version,
            };
        }
    }
    CliDetect {
        found: false,
        path: String::new(),
        version: String::new(),
    }
}

// ── Tests ──────────────────────────────────────────────────────────
// Sample lines are real shapes captured from `claude … stream-json` and
// `codex exec --json` on 2026-06-25 (PRODUCT-SPEC §5.14.8).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ai::types::ProviderKind;

    fn collect(lines: &[&str], flavor: CliFlavor, render_tools: bool) -> (String, CliAcc) {
        let mut acc = CliAcc::default();
        let mut out = String::new();
        {
            let mut on_delta = |s: &str| out.push_str(s);
            for l in lines {
                match flavor {
                    CliFlavor::ClaudeCode => {
                        parse_claude_line(l, &mut on_delta, &mut acc, render_tools)
                    }
                    CliFlavor::Codex => parse_codex_line(l, &mut on_delta, &mut acc, render_tools),
                }
            }
        }
        (out, acc)
    }

    fn cli_cfg(flavor: CliFlavor, mode: CliMode) -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::Cli,
            base_url: String::new(),
            api_key: None,
            model: "sonnet".into(),
            max_tokens: None,
            cli_flavor: Some(flavor),
            cli_bin: None,
            cli_mode: mode,
            cli_cwd: None,
            cli_extra_args: Vec::new(),
        }
    }

    #[test]
    fn claude_streams_text_and_captures_usage() {
        let lines = [
            r#"{"type":"system","subtype":"init","tools":[]}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"x"},{"type":"text","text":"PIERX_OK"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"PIERX_OK","usage":{"input_tokens":3,"output_tokens":8}}"#,
        ];
        let (streamed, acc) = collect(&lines, CliFlavor::ClaudeCode, false);
        assert_eq!(streamed, "PIERX_OK");
        assert_eq!(acc.text, "PIERX_OK");
        assert_eq!(acc.input_tokens, Some(3));
        assert_eq!(acc.output_tokens, Some(8));
        assert!(acc.error.is_none());
    }

    #[test]
    fn claude_falls_back_to_result_text() {
        let lines =
            [r#"{"type":"result","subtype":"success","result":"hi","usage":{"output_tokens":1}}"#];
        let (streamed, acc) = collect(&lines, CliFlavor::ClaudeCode, false);
        assert_eq!(streamed, "hi");
        assert_eq!(acc.text, "hi");
    }

    #[test]
    fn claude_error_result_sets_error() {
        let lines = [r#"{"type":"result","subtype":"error","is_error":true,"result":"boom"}"#];
        let (_streamed, acc) = collect(&lines, CliFlavor::ClaudeCode, false);
        assert_eq!(acc.error.as_deref(), Some("boom"));
    }

    #[test]
    fn ignores_non_json_warning_lines() {
        let lines = [
            "Warning: no stdin data received in 3s, proceeding without it.",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"ok"}]}}"#,
        ];
        let (streamed, acc) = collect(&lines, CliFlavor::ClaudeCode, false);
        assert_eq!(streamed, "ok");
        assert_eq!(acc.text, "ok");
    }

    #[test]
    fn codex_message_item_and_failure() {
        let ok = [
            r#"{"type":"thread.started","thread_id":"t"}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"hello"}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":2,"output_tokens":4}}"#,
        ];
        let (streamed, acc) = collect(&ok, CliFlavor::Codex, false);
        assert_eq!(streamed, "hello");
        assert_eq!(acc.output_tokens, Some(4));

        let err = [r#"{"type":"turn.failed","error":{"message":"nope"}}"#];
        let (_s, acc2) = collect(&err, CliFlavor::Codex, false);
        assert_eq!(acc2.error.as_deref(), Some("nope"));
    }

    #[test]
    fn compose_prompt_carries_system_and_user() {
        let msgs = [ChatMessage::user("hello there")];
        let p = compose_prompt("SYS", &msgs);
        assert!(p.contains("SYS"));
        assert!(p.contains("User: hello there"));
    }

    #[test]
    fn m1_tools_off_m2a_accept_m2b_extra_args() {
        let m1 = build_args(
            CliFlavor::ClaudeCode,
            &cli_cfg(CliFlavor::ClaudeCode, CliMode::ModelBackend),
        );
        let i = m1
            .iter()
            .position(|a| a == "--tools")
            .expect("M1 has --tools");
        assert_eq!(m1[i + 1], "");
        assert!(m1.iter().any(|a| a == "stream-json"));
        assert!(m1.windows(2).any(|w| w[0] == "--model" && w[1] == "sonnet"));

        let m2a = build_args(
            CliFlavor::ClaudeCode,
            &cli_cfg(CliFlavor::ClaudeCode, CliMode::NativeAgent),
        );
        assert!(!m2a.iter().any(|a| a == "--tools"));
        assert!(m2a
            .windows(2)
            .any(|w| w[0] == "--permission-mode" && w[1] == "acceptEdits"));

        // M2b: no acceptEdits/--tools; extra args (set by the shell layer)
        // are appended verbatim.
        let mut cfg = cli_cfg(CliFlavor::ClaudeCode, CliMode::GatedAgent);
        cfg.cli_extra_args = vec![
            "--permission-prompt-tool".into(),
            "mcp__pierx__approve".into(),
        ];
        let m2b = build_args(CliFlavor::ClaudeCode, &cfg);
        assert!(!m2b.iter().any(|a| a == "--tools"));
        assert!(!m2b.iter().any(|a| a == "acceptEdits"));
        assert!(m2b
            .windows(2)
            .any(|w| w[0] == "--permission-prompt-tool" && w[1] == "mcp__pierx__approve"));
    }

    #[test]
    fn m2a_renders_tool_steps_m1_hides_them() {
        let lines = [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t","name":"Read","input":{"file_path":"a.txt"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"tool_use_id":"t","type":"tool_result","content":"alpha bravo"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"done"}]}}"#,
        ];
        let (m2a, _) = collect(&lines, CliFlavor::ClaudeCode, true);
        assert!(m2a.contains("Read"));
        assert!(m2a.contains("alpha bravo"));
        assert!(m2a.contains("done"));

        let (m1, _) = collect(&lines, CliFlavor::ClaudeCode, false);
        assert!(!m1.contains("Read"));
        assert!(!m1.contains("alpha bravo"));
        assert!(m1.contains("done"));
    }
}
