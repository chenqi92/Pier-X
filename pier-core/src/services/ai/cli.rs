//! Local agent-CLI backend (PRODUCT-SPEC §5.14.8).
//!
//! Drives the user's already-installed, already-logged-in agent CLI
//! (Claude Code / Codex) as a subprocess, reusing their subscription
//! login — no API key, no proxy. This is the **M1 "model backend"**
//! mode: the CLI runs with its OWN tools DISABLED and only produces
//! text, so Pier-X's risk-gated tool loop (`src-tauri/src/ai.rs`) is
//! untouched. The CLI's stdout JSON stream is adapted into a
//! [`TurnOutcome`], matching the shape `stream_chat` expects from the
//! HTTP providers.
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
    AiError, ChatMessage, ChatRole, CliFlavor, ProviderConfig, StopKind, ToolSpec, TurnOutcome,
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

/// Argv for a single tool-less completion turn (M1).
fn build_args(flavor: CliFlavor, cfg: &ProviderConfig) -> Vec<String> {
    let model = cfg.model.trim();
    match flavor {
        CliFlavor::ClaudeCode => {
            // `--tools ""` disables ALL native tools => pure text (verified
            // 2026-06-25: the init event reports `tools:[]`). The prompt is
            // fed on stdin; `--no-session-persistence` keeps it stateless.
            let mut a = vec![
                "-p".to_string(),
                "--tools".to_string(),
                String::new(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
                "--no-session-persistence".to_string(),
            ];
            if !model.is_empty() {
                a.push("--model".to_string());
                a.push(model.to_string());
            }
            a
        }
        CliFlavor::Codex => {
            // NOTE: codex `exec --json` event schema is NOT verified on a
            // live run (CLI version drift blocked it, see §5.14.8); the
            // parser below is best-effort. `read-only` keeps it side-effect
            // free; `-` reads the prompt from stdin.
            let mut a = vec![
                "exec".to_string(),
                "--json".to_string(),
                "--sandbox".to_string(),
                "read-only".to_string(),
                "--skip-git-repo-check".to_string(),
                "--color".to_string(),
                "never".to_string(),
            ];
            if !model.is_empty() {
                a.push("-m".to_string());
                a.push(model.to_string());
            }
            a.push("-".to_string());
            a
        }
    }
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
            // M1 is tool-less: there are no tool-result turns to forward.
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

fn parse_claude_line(line: &str, on_delta: &mut dyn FnMut(&str), acc: &mut CliAcc) {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return, // tolerate non-JSON warning lines
    };
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "assistant" => {
            if let Some(content) = v.pointer("/message/content").and_then(Value::as_array) {
                for block in content {
                    if block.get("type").and_then(Value::as_str) == Some("text") {
                        if let Some(t) = block.get("text").and_then(Value::as_str) {
                            if !t.is_empty() {
                                acc.text.push_str(t);
                                on_delta(t);
                            }
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

fn parse_codex_line(line: &str, on_delta: &mut dyn FnMut(&str), acc: &mut CliAcc) {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return,
    };
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        // Best-effort: assistant text arrives as a message item. Only emit
        // on item.completed to avoid duplicating partial updates.
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

/// Run one tool-less completion turn via the agent CLI.
pub fn stream_cli(
    cfg: &ProviderConfig,
    system: &str,
    messages: &[ChatMessage],
    _tools: &[ToolSpec],
    on_delta: &mut dyn FnMut(&str),
    cancel: &CancellationToken,
) -> Result<TurnOutcome, AiError> {
    // M1: Pier-X keeps its OWN risk-gated tools; the CLI runs tool-less,
    // so `_tools` is intentionally not forwarded.
    let flavor = flavor_of(cfg);
    let bin = resolve_bin(cfg);
    let prompt = compose_prompt(system, messages);

    let mut cmd = Command::new(&bin);
    cmd.args(build_args(flavor, cfg));
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
            CliFlavor::ClaudeCode => parse_claude_line(&line, on_delta, &mut acc),
            CliFlavor::Codex => parse_codex_line(&line, on_delta, &mut acc),
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
            vec!["opus".to_string(), "sonnet".to_string(), "haiku".to_string()]
        }
        CliFlavor::Codex => vec!["gpt-5.1-codex".to_string(), "gpt-5-codex".to_string()],
    }
}

/// Probe the CLI via `<bin> --version` (cheap; no tokens, no network).
pub fn test_connection(cfg: &ProviderConfig) -> Result<String, AiError> {
    let flavor = flavor_of(cfg);
    let bin = resolve_bin(cfg);
    let mut cmd = Command::new(&bin);
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
        return Err(AiError::Http(format!("{bin} --version failed: {}", err.trim())));
    }
    let ver_raw = String::from_utf8_lossy(&out.stdout);
    let ver = ver_raw.lines().next().unwrap_or("").trim();
    Ok(format!("ok · {} · {ver}", default_bin(flavor)))
}

// ── Tests ──────────────────────────────────────────────────────────
// Sample lines are real shapes captured from `claude … stream-json` and
// `codex exec --json` on 2026-06-25 (PRODUCT-SPEC §5.14.8).

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(lines: &[&str], flavor: CliFlavor) -> (String, CliAcc) {
        let mut acc = CliAcc::default();
        let mut out = String::new();
        {
            let mut on_delta = |s: &str| out.push_str(s);
            for l in lines {
                match flavor {
                    CliFlavor::ClaudeCode => parse_claude_line(l, &mut on_delta, &mut acc),
                    CliFlavor::Codex => parse_codex_line(l, &mut on_delta, &mut acc),
                }
            }
        }
        (out, acc)
    }

    #[test]
    fn claude_streams_text_and_captures_usage() {
        let lines = [
            r#"{"type":"system","subtype":"init","tools":[]}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"x"},{"type":"text","text":"PIERX_OK"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"PIERX_OK","usage":{"input_tokens":3,"output_tokens":8}}"#,
        ];
        let (streamed, acc) = collect(&lines, CliFlavor::ClaudeCode);
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
        let (streamed, acc) = collect(&lines, CliFlavor::ClaudeCode);
        assert_eq!(streamed, "hi");
        assert_eq!(acc.text, "hi");
    }

    #[test]
    fn claude_error_result_sets_error() {
        let lines = [r#"{"type":"result","subtype":"error","is_error":true,"result":"boom"}"#];
        let (_streamed, acc) = collect(&lines, CliFlavor::ClaudeCode);
        assert_eq!(acc.error.as_deref(), Some("boom"));
    }

    #[test]
    fn ignores_non_json_warning_lines() {
        let lines = [
            "Warning: no stdin data received in 3s, proceeding without it.",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"ok"}]}}"#,
        ];
        let (streamed, acc) = collect(&lines, CliFlavor::ClaudeCode);
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
        let (streamed, acc) = collect(&ok, CliFlavor::Codex);
        assert_eq!(streamed, "hello");
        assert_eq!(acc.output_tokens, Some(4));

        let err = [r#"{"type":"turn.failed","error":{"message":"nope"}}"#];
        let (_s, acc2) = collect(&err, CliFlavor::Codex);
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
    fn claude_args_disable_tools() {
        let cfg = ProviderConfig {
            kind: super::super::types::ProviderKind::Cli,
            base_url: String::new(),
            api_key: None,
            model: "sonnet".into(),
            max_tokens: None,
            cli_flavor: Some(CliFlavor::ClaudeCode),
            cli_bin: None,
        };
        let args = build_args(CliFlavor::ClaudeCode, &cfg);
        // `--tools ""` (a flag immediately followed by an empty arg) is the
        // tool-less switch — its presence is the M1 safety guarantee.
        let i = args.iter().position(|a| a == "--tools").expect("has --tools");
        assert_eq!(args[i + 1], "");
        assert!(args.iter().any(|a| a == "stream-json"));
        assert!(args.windows(2).any(|w| w[0] == "--model" && w[1] == "sonnet"));
    }
}
