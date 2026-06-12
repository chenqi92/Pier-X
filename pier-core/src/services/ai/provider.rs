//! Streaming chat clients (PRODUCT-SPEC §8.7).
//!
//! Two wire protocols cover all three configured kinds:
//!
//!   * **Anthropic** — `POST {base}/v1/messages`, SSE event stream.
//!   * **OpenAI-compatible** — `POST {base}/chat/completions`, SSE
//!     `data:` chunks. Used directly for the `openai` kind and for
//!     **Ollama** (which serves the OpenAI protocol at `/v1`).
//!
//! Blocking + callback style (§8.3 convention): `stream_chat` blocks
//! the calling thread, invokes `on_delta` for every text fragment,
//! and returns the assembled [`TurnOutcome`] (final text, tool calls,
//! usage, stop reason). HTTP goes through `ureq` — the same blocking
//! client the webhook fan-out uses — so callers run this on a worker
//! thread, never on a UI / IPC thread.
//!
//! Cancellation: the blocking HTTP work (connect, headers, first
//! byte, SSE reads) runs on a worker thread; [`stream_chat`] polls
//! the `CancellationToken` every 100 ms while relaying deltas, so a
//! stop request takes effect immediately even while the socket is
//! stuck connecting or waiting for the first byte. An abandoned
//! worker dies on its own at the connect/read timeout.

use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::types::{
    AiError, ChatMessage, ChatRole, ProviderConfig, ProviderKind, StopKind, ToolCall, ToolSpec,
    TurnOutcome,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Watchdog between SSE reads. Generations emit sub-second deltas,
/// so this only fires when the provider truly stalled.
const READ_TIMEOUT: Duration = Duration::from_secs(180);

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .build()
}

/// Run one streamed model turn. `on_delta` receives assistant text
/// fragments as they arrive; tool calls are assembled internally and
/// returned on the outcome (the caller gates them through the risk
/// classifier before anything executes).
///
/// The HTTP request runs on a worker thread so cancellation is
/// honoured DURING connect / header / first-byte waits, not just
/// between SSE lines — without this, a black-holed endpoint pinned
/// the stop button until the read timeout (3 min) fired.
pub fn stream_chat(
    cfg: &ProviderConfig,
    system: &str,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    on_delta: &mut dyn FnMut(&str),
    cancel: &CancellationToken,
) -> Result<TurnOutcome, AiError> {
    enum Ev {
        Delta(String),
        Done(Box<Result<TurnOutcome, AiError>>),
    }
    let (tx, rx) = mpsc::channel::<Ev>();
    let delta_tx = tx.clone();
    let cfg = cfg.clone();
    let system = system.to_string();
    let messages = messages.to_vec();
    let tools = tools.to_vec();
    let worker_cancel = cancel.clone();
    std::thread::spawn(move || {
        let mut send = |s: &str| {
            let _ = delta_tx.send(Ev::Delta(s.to_string()));
        };
        let result = match cfg.kind {
            ProviderKind::Anthropic => {
                stream_anthropic(&cfg, &system, &messages, &tools, &mut send, &worker_cancel)
            }
            ProviderKind::Openai | ProviderKind::Ollama => {
                stream_openai(&cfg, &system, &messages, &tools, &mut send, &worker_cancel)
            }
        };
        // The receiver is gone after a cancel — that's fine.
        let _ = tx.send(Ev::Done(Box::new(result)));
    });
    loop {
        if cancel.is_cancelled() {
            // Abandon the worker: its next send fails silently and the
            // socket dies at its own timeout.
            return Err(AiError::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ev::Delta(s)) => on_delta(&s),
            Ok(Ev::Done(result)) => return *result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(AiError::Http("provider worker exited unexpectedly".into()));
            }
        }
    }
}

/// Enumerate the models the configured endpoint serves
/// (`GET {base}/models`, Anthropic: `GET {base}/v1/models`).
///
/// Powers the settings dialog's "fetch models" button. Vendors that
/// don't implement the listing endpoint surface their HTTP error
/// here — the UI treats that as "type the model name manually", not
/// as a blocker. Result is sorted and de-duplicated.
pub fn list_models(cfg: &ProviderConfig) -> Result<Vec<String>, AiError> {
    let base = cfg.effective_base_url();
    let url = match cfg.kind {
        ProviderKind::Anthropic => format!("{base}/v1/models"),
        ProviderKind::Openai | ProviderKind::Ollama => format!("{base}/models"),
    };
    let mut req = agent().get(&url);
    match cfg.kind {
        ProviderKind::Anthropic => {
            let key = cfg.api_key.as_deref().ok_or(AiError::MissingKey)?;
            req = req.set("x-api-key", key).set("anthropic-version", "2023-06-01");
        }
        ProviderKind::Openai | ProviderKind::Ollama => {
            if let Some(key) = cfg.api_key.as_deref().filter(|k| !k.trim().is_empty()) {
                req = req.set("authorization", &format!("Bearer {key}"));
            }
        }
    }
    let resp = req.call().map_err(map_ureq_error)?;
    let body: Value = resp
        .into_json()
        .map_err(|e| AiError::Protocol(format!("models response: {e}")))?;
    // Standard shape: `{"data": [{"id": …}, …]}`. A few compat
    // servers use `{"models": [{"name"|"id"|"model": …}]}` (Ollama
    // native, some gateways) — accept both rather than failing.
    let entries = body
        .get("data")
        .or_else(|| body.get("models"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut models: Vec<String> = entries
        .iter()
        .filter_map(|m| {
            m.get("id")
                .or_else(|| m.get("name"))
                .or_else(|| m.get("model"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| m.as_str().map(str::to_string))
        })
        .filter(|s| !s.is_empty())
        .collect();
    models.sort();
    models.dedup();
    Ok(models)
}

/// Reachability + auth + model probe for the settings dialog: a
/// real 1-token, non-streaming chat round-trip through the SAME
/// endpoint the panel uses. `GET /models` is not a sufficient probe
/// — it can succeed while `POST /chat/completions` is unreachable
/// (regional blocks, partial proxies) or rejects the model id.
pub fn test_connection(cfg: &ProviderConfig) -> Result<String, AiError> {
    if cfg.model.trim().is_empty() {
        return Err(AiError::Protocol("no model configured".into()));
    }
    let base = cfg.effective_base_url();
    let started = Instant::now();
    match cfg.kind {
        ProviderKind::Anthropic => {
            let key = cfg.api_key.as_deref().ok_or(AiError::MissingKey)?;
            let body = json!({
                "model": cfg.model,
                "max_tokens": 1,
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": "ping" }] }],
            });
            agent()
                .post(&format!("{base}/v1/messages"))
                .set("content-type", "application/json")
                .set("x-api-key", key)
                .set("anthropic-version", "2023-06-01")
                .send_string(&body.to_string())
                .map_err(map_ureq_error)?;
        }
        ProviderKind::Openai | ProviderKind::Ollama => {
            let body = json!({
                "model": cfg.model,
                "max_tokens": 1,
                "messages": [{ "role": "user", "content": "ping" }],
            });
            let mut req = agent()
                .post(&format!("{base}/chat/completions"))
                .set("content-type", "application/json");
            if let Some(key) = cfg.api_key.as_deref().filter(|k| !k.trim().is_empty()) {
                req = req.set("authorization", &format!("Bearer {key}"));
            }
            req.send_string(&body.to_string()).map_err(map_ureq_error)?;
        }
    }
    let ms = started.elapsed().as_millis();
    Ok(format!("ok · {} · {ms} ms", cfg.model))
}

fn map_ureq_error(err: ureq::Error) -> AiError {
    match err {
        ureq::Error::Status(status, resp) => {
            let mut body = resp.into_string().unwrap_or_default();
            // Surface the provider's JSON error message when present.
            if let Ok(v) = serde_json::from_str::<Value>(&body) {
                if let Some(msg) = v
                    .pointer("/error/message")
                    .or_else(|| v.pointer("/message"))
                    .and_then(|m| m.as_str())
                {
                    body = msg.to_string();
                }
            }
            body.truncate(500);
            AiError::Api { status, message: body }
        }
        ureq::Error::Transport(t) => AiError::Http(t.to_string()),
    }
}

// ── OpenAI-compatible ──────────────────────────────────────────────

fn openai_messages(system: &str, messages: &[ChatMessage]) -> Vec<Value> {
    let mut out = Vec::with_capacity(messages.len() + 1);
    if !system.trim().is_empty() {
        out.push(json!({ "role": "system", "content": system }));
    }
    for m in messages {
        match m.role {
            ChatRole::System => out.push(json!({ "role": "system", "content": m.content })),
            ChatRole::User => out.push(json!({ "role": "user", "content": m.content })),
            ChatRole::Assistant => {
                let mut msg = json!({ "role": "assistant" });
                if m.content.is_empty() && !m.tool_calls.is_empty() {
                    msg["content"] = Value::Null;
                } else {
                    msg["content"] = Value::String(m.content.clone());
                }
                if !m.tool_calls.is_empty() {
                    msg["tool_calls"] = Value::Array(
                        m.tool_calls
                            .iter()
                            .map(|tc| {
                                json!({
                                    "id": tc.id,
                                    "type": "function",
                                    "function": { "name": tc.name, "arguments": tc.arguments },
                                })
                            })
                            .collect(),
                    );
                }
                out.push(msg);
            }
            ChatRole::Tool => out.push(json!({
                "role": "tool",
                "tool_call_id": m.tool_call_id.clone().unwrap_or_default(),
                "content": m.content,
            })),
        }
    }
    out
}

fn stream_openai(
    cfg: &ProviderConfig,
    system: &str,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    on_delta: &mut dyn FnMut(&str),
    cancel: &CancellationToken,
) -> Result<TurnOutcome, AiError> {
    let base = cfg.effective_base_url();
    let url = format!("{base}/chat/completions");

    let mut body = json!({
        "model": cfg.model,
        "messages": openai_messages(system, messages),
        "stream": true,
        "max_tokens": cfg.effective_max_tokens(),
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.schema,
                        },
                    })
                })
                .collect(),
        );
    }

    let mut req = agent()
        .post(&url)
        .set("content-type", "application/json")
        .set("accept", "text/event-stream");
    if let Some(key) = cfg.api_key.as_deref().filter(|k| !k.trim().is_empty()) {
        req = req.set("authorization", &format!("Bearer {key}"));
    }
    let resp = req.send_string(&body.to_string()).map_err(map_ureq_error)?;

    let reader = BufReader::new(resp.into_reader());
    let mut text = String::new();
    // tool-call accumulation keyed by stream `index`.
    let mut calls: Vec<(String, String, String)> = Vec::new(); // (id, name, args)
    let mut stop = StopKind::EndTurn;
    let mut input_tokens = None;
    let mut output_tokens = None;

    for line in reader.lines() {
        if cancel.is_cancelled() {
            return Err(AiError::Cancelled);
        }
        let line = line.map_err(|e| AiError::Http(format!("stream read: {e}")))?;
        let line = line.trim_end_matches('\r');
        let Some(data) = line.strip_prefix("data:").map(str::trim_start) else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        let chunk: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue, // tolerate keep-alive noise from compat servers
        };
        if let Some(usage) = chunk.get("usage").filter(|u| !u.is_null()) {
            input_tokens = usage.get("prompt_tokens").and_then(Value::as_u64).or(input_tokens);
            output_tokens = usage.get("completion_tokens").and_then(Value::as_u64).or(output_tokens);
        }
        let Some(choice) = chunk.pointer("/choices/0") else { continue };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            stop = match reason {
                "stop" => StopKind::EndTurn,
                "tool_calls" | "function_call" => StopKind::ToolUse,
                "length" => StopKind::MaxTokens,
                _ => StopKind::Other,
            };
        }
        let Some(delta) = choice.get("delta") else { continue };
        if let Some(content) = delta.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                text.push_str(content);
                on_delta(content);
            }
        }
        if let Some(tool_deltas) = delta.get("tool_calls").and_then(Value::as_array) {
            for td in tool_deltas {
                let index = td.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                while calls.len() <= index {
                    calls.push((String::new(), String::new(), String::new()));
                }
                let slot = &mut calls[index];
                if let Some(id) = td.get("id").and_then(Value::as_str) {
                    slot.0 = id.to_string();
                }
                if let Some(name) = td.pointer("/function/name").and_then(Value::as_str) {
                    slot.1.push_str(name);
                }
                if let Some(args) = td.pointer("/function/arguments").and_then(Value::as_str) {
                    slot.2.push_str(args);
                }
            }
        }
    }

    let tool_calls: Vec<ToolCall> = calls
        .into_iter()
        .enumerate()
        .filter(|(_, (_, name, _))| !name.is_empty())
        .map(|(i, (id, name, args))| ToolCall {
            id: if id.is_empty() { format!("call_{i}") } else { id },
            name,
            arguments: if args.is_empty() { "{}".into() } else { args },
        })
        .collect();
    if !tool_calls.is_empty() {
        stop = StopKind::ToolUse;
    }

    Ok(TurnOutcome { text, tool_calls, input_tokens, output_tokens, stop })
}

// ── Anthropic ──────────────────────────────────────────────────────

fn anthropic_messages(messages: &[ChatMessage]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(messages.len());
    for m in messages {
        match m.role {
            // System content travels in the top-level `system` field;
            // a stray system message in history degrades to user text.
            ChatRole::System | ChatRole::User => {
                out.push(json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": m.content }],
                }));
            }
            ChatRole::Assistant => {
                let mut blocks: Vec<Value> = Vec::new();
                if !m.content.is_empty() {
                    blocks.push(json!({ "type": "text", "text": m.content }));
                }
                for tc in &m.tool_calls {
                    let input: Value =
                        serde_json::from_str(&tc.arguments).unwrap_or_else(|_| json!({}));
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": input,
                    }));
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": "" }));
                }
                out.push(json!({ "role": "assistant", "content": blocks }));
            }
            ChatRole::Tool => {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": m.tool_call_id.clone().unwrap_or_default(),
                    "content": m.content,
                });
                // Anthropic requires strict user/assistant alternation:
                // consecutive tool results merge into one user turn.
                let merged = out
                    .last_mut()
                    .filter(|last| {
                        last.get("role").and_then(Value::as_str) == Some("user")
                            && last
                                .pointer("/content/0/type")
                                .and_then(Value::as_str)
                                == Some("tool_result")
                    })
                    .map(|last| {
                        last["content"].as_array_mut().unwrap().push(block.clone());
                    })
                    .is_some();
                if !merged {
                    out.push(json!({ "role": "user", "content": [block] }));
                }
            }
        }
    }
    out
}

fn stream_anthropic(
    cfg: &ProviderConfig,
    system: &str,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    on_delta: &mut dyn FnMut(&str),
    cancel: &CancellationToken,
) -> Result<TurnOutcome, AiError> {
    let key = cfg.api_key.as_deref().ok_or(AiError::MissingKey)?;
    let base = cfg.effective_base_url();
    let url = format!("{base}/v1/messages");

    let mut body = json!({
        "model": cfg.model,
        "max_tokens": cfg.effective_max_tokens(),
        "messages": anthropic_messages(messages),
        "stream": true,
    });
    if !system.trim().is_empty() {
        body["system"] = Value::String(system.to_string());
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.schema,
                    })
                })
                .collect(),
        );
    }

    let resp = agent()
        .post(&url)
        .set("content-type", "application/json")
        .set("accept", "text/event-stream")
        .set("x-api-key", key)
        .set("anthropic-version", "2023-06-01")
        .send_string(&body.to_string())
        .map_err(map_ureq_error)?;

    let reader = BufReader::new(resp.into_reader());
    let mut text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    // index → position in tool_calls for in-flight tool_use blocks.
    let mut open_tool_blocks: Vec<(usize, usize)> = Vec::new();
    let mut stop = StopKind::EndTurn;
    let mut input_tokens = None;
    let mut output_tokens = None;

    for line in reader.lines() {
        if cancel.is_cancelled() {
            return Err(AiError::Cancelled);
        }
        let line = line.map_err(|e| AiError::Http(format!("stream read: {e}")))?;
        let line = line.trim_end_matches('\r');
        let Some(data) = line.strip_prefix("data:").map(str::trim_start) else {
            continue;
        };
        let event: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match event.get("type").and_then(Value::as_str).unwrap_or("") {
            "message_start" => {
                input_tokens = event
                    .pointer("/message/usage/input_tokens")
                    .and_then(Value::as_u64)
                    .or(input_tokens);
            }
            "content_block_start" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                if event.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use")
                {
                    let id = event
                        .pointer("/content_block/id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let name = event
                        .pointer("/content_block/name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    tool_calls.push(ToolCall { id, name, arguments: String::new() });
                    open_tool_blocks.push((index, tool_calls.len() - 1));
                }
            }
            "content_block_delta" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                match event.pointer("/delta/type").and_then(Value::as_str).unwrap_or("") {
                    "text_delta" => {
                        if let Some(t) = event.pointer("/delta/text").and_then(Value::as_str) {
                            text.push_str(t);
                            on_delta(t);
                        }
                    }
                    "input_json_delta" => {
                        if let Some(part) =
                            event.pointer("/delta/partial_json").and_then(Value::as_str)
                        {
                            if let Some((_, slot)) =
                                open_tool_blocks.iter().find(|(i, _)| *i == index)
                            {
                                tool_calls[*slot].arguments.push_str(part);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = event.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    stop = match reason {
                        "end_turn" => StopKind::EndTurn,
                        "tool_use" => StopKind::ToolUse,
                        "max_tokens" => StopKind::MaxTokens,
                        _ => StopKind::Other,
                    };
                }
                output_tokens = event
                    .pointer("/usage/output_tokens")
                    .and_then(Value::as_u64)
                    .or(output_tokens);
            }
            "message_stop" => break,
            "error" => {
                let msg = event
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("provider stream error")
                    .to_string();
                return Err(AiError::Protocol(msg));
            }
            _ => {}
        }
    }

    for tc in &mut tool_calls {
        if tc.arguments.is_empty() {
            tc.arguments = "{}".into();
        }
    }
    if !tool_calls.is_empty() && stop == StopKind::EndTurn {
        stop = StopKind::ToolUse;
    }

    Ok(TurnOutcome { text, tool_calls, input_tokens, output_tokens, stop })
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_message_mapping_carries_tool_calls() {
        let msgs = vec![
            ChatMessage::user("list files"),
            ChatMessage::assistant(
                "",
                vec![ToolCall { id: "call_1".into(), name: "run_command".into(), arguments: "{\"command\":\"ls\"}".into() }],
            ),
            ChatMessage::tool_result("call_1", "a.txt\nb.txt"),
        ];
        let mapped = openai_messages("sys", &msgs);
        assert_eq!(mapped[0]["role"], "system");
        assert_eq!(mapped[2]["tool_calls"][0]["function"]["name"], "run_command");
        assert_eq!(mapped[3]["role"], "tool");
        assert_eq!(mapped[3]["tool_call_id"], "call_1");
    }

    #[test]
    fn anthropic_merges_consecutive_tool_results() {
        let msgs = vec![
            ChatMessage::user("q"),
            ChatMessage::assistant(
                "",
                vec![
                    ToolCall { id: "a".into(), name: "x".into(), arguments: "{}".into() },
                    ToolCall { id: "b".into(), name: "y".into(), arguments: "{}".into() },
                ],
            ),
            ChatMessage::tool_result("a", "ra"),
            ChatMessage::tool_result("b", "rb"),
        ];
        let mapped = anthropic_messages(&msgs);
        // user, assistant, ONE merged user turn with two tool_result blocks.
        assert_eq!(mapped.len(), 3);
        assert_eq!(mapped[2]["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn default_base_urls() {
        let mk = |kind| ProviderConfig {
            kind,
            base_url: String::new(),
            api_key: None,
            model: "m".into(),
            max_tokens: None,
        };
        assert_eq!(mk(ProviderKind::Anthropic).effective_base_url(), "https://api.anthropic.com");
        assert_eq!(mk(ProviderKind::Openai).effective_base_url(), "https://api.openai.com/v1");
        assert_eq!(mk(ProviderKind::Ollama).effective_base_url(), "http://localhost:11434/v1");
    }
}
