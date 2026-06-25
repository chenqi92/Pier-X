//! Shared types for the AI assistant backend.
//!
//! Everything that crosses the IPC boundary serialises with
//! camelCase field names (house convention for Tauri payloads);
//! enums that the frontend switches on use lowercase string tags.

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Risk model ─────────────────────────────────────────────────────

/// Risk tier for an AI-proposed action (PRODUCT-SPEC §5.14.4).
///
/// Ordering is meaningful: compound commands take the max of their
/// segments, so `L0 < L1 < L2 < L3` must hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// Read-only, no side effects — auto-executes (tool card stays visible).
    L0,
    /// Recoverable write — per-action approval card; allow-listable.
    L1,
    /// Hard-to-reverse / wide blast radius — strong confirm; never allow-listable.
    L2,
    /// Red line — the execution channel is closed. No button is rendered.
    L3,
}

/// Output of [`crate::services::ai::risk::classify_command`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskAssessment {
    /// The resolved tier (max across compound segments).
    pub level: RiskLevel,
    /// Human-readable reasons (already terse; frontend localises around them).
    pub reasons: Vec<String>,
    /// `sudo` / `doas` prefix detected — approval card must flag root execution.
    pub as_root: bool,
}

impl RiskAssessment {
    /// A bare assessment at `level` with no reasons attached.
    pub fn new(level: RiskLevel) -> Self {
        Self { level, reasons: Vec::new(), as_root: false }
    }
}

// ── Chat model ─────────────────────────────────────────────────────

/// Who authored a [`ChatMessage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    /// Operator instructions (normally carried out-of-band as the
    /// top-level system prompt; tolerated in history).
    System,
    /// The human.
    User,
    /// The model.
    Assistant,
    /// A tool result being fed back to the model. `tool_call_id`
    /// links it to the originating [`ToolCall`].
    Tool,
}

/// One tool invocation requested by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    /// Provider-assigned call id; echoed back on the result.
    pub id: String,
    /// Tool name as declared in the [`ToolSpec`] list.
    pub name: String,
    /// Raw JSON argument string exactly as the model produced it.
    pub arguments: String,
}

/// One turn in the conversation history sent to the provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    /// Author of this turn.
    pub role: ChatRole,
    /// Text body (may be empty on tool-calling assistant turns).
    pub content: String,
    /// Set on assistant messages that requested tools.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Set on `Tool` messages: which call this result answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    /// A plain user turn.
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: ChatRole::User, content: content.into(), tool_calls: Vec::new(), tool_call_id: None }
    }
    /// An assistant turn (text and/or tool calls).
    pub fn assistant(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self { role: ChatRole::Assistant, content: content.into(), tool_calls, tool_call_id: None }
    }
    /// A tool-result turn answering `call_id`.
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
        }
    }
}

/// A tool the model is allowed to request. `schema` is a JSON Schema
/// object (provider impls re-wrap it into their own envelope).
#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    /// Tool name the model calls.
    pub name: String,
    /// Prescriptive description (when to call it, not just what it does).
    pub description: String,
    /// JSON Schema for the arguments object.
    pub schema: serde_json::Value,
}

// ── Provider configuration ─────────────────────────────────────────

/// Which wire protocol / vendor a [`ProviderConfig`] targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Anthropic Messages API (`/v1/messages`).
    Anthropic,
    /// Any OpenAI-compatible `/chat/completions` endpoint
    /// (OpenAI, DeepSeek, Qwen, OpenRouter, LM Studio, vLLM, …).
    Openai,
    /// Local Ollama. Speaks the OpenAI-compatible protocol against
    /// `http://localhost:11434/v1` by default; no API key.
    Ollama,
    /// Local agent CLI driven as a subprocess (PRODUCT-SPEC §5.14.8):
    /// reuses the user's logged-in CLI (Claude Code / Codex) — no API
    /// key. Run tool-less (M1 model-backend mode); its stdout JSON
    /// stream is adapted into a [`TurnOutcome`].
    Cli,
}

/// Which agent CLI a [`ProviderKind::Cli`] config drives. Selects the
/// argv template and the stdout-event parser (PRODUCT-SPEC §5.14.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CliFlavor {
    /// Anthropic Claude Code (`claude -p --output-format stream-json`).
    ClaudeCode,
    /// OpenAI Codex (`codex exec --json`).
    Codex,
}

/// Resolved provider settings for one chat turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// Protocol / vendor selector.
    pub kind: ProviderKind,
    /// Endpoint base. Empty string selects the kind's default
    /// (`https://api.anthropic.com`, `https://api.openai.com/v1`,
    /// `http://localhost:11434/v1`).
    #[serde(default)]
    pub base_url: String,
    /// Resolved by the shell from the OS keyring (`pier-x.ai.<kind>`)
    /// just before the call — never persisted in config files.
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    /// Model id, verbatim.
    pub model: String,
    /// Per-turn output cap. Defaults to 4096 (Anthropic requires an
    /// explicit value; OpenAI-compatible endpoints accept it too).
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// `ProviderKind::Cli` only: which agent CLI to drive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_flavor: Option<CliFlavor>,
    /// `ProviderKind::Cli` only: absolute path to the CLI binary (from
    /// detection / the settings picker). Empty → resolve the flavor's
    /// default name on PATH.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_bin: Option<String>,
}

impl ProviderConfig {
    /// `base_url` with the kind's default substituted when empty,
    /// trailing slash trimmed.
    pub fn effective_base_url(&self) -> String {
        let trimmed = self.base_url.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
        match self.kind {
            ProviderKind::Anthropic => "https://api.anthropic.com".into(),
            ProviderKind::Openai => "https://api.openai.com/v1".into(),
            ProviderKind::Ollama => "http://localhost:11434/v1".into(),
            // CLI backends are subprocess, not HTTP — no base URL.
            ProviderKind::Cli => String::new(),
        }
    }

    /// `max_tokens` with the default applied and clamped to sane bounds.
    pub fn effective_max_tokens(&self) -> u32 {
        self.max_tokens.unwrap_or(4096).clamp(256, 64_000)
    }
}

// ── Turn outcome ───────────────────────────────────────────────────

/// Why the model stopped producing this turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopKind {
    /// Natural end of the assistant message.
    EndTurn,
    /// The model wants tool results before continuing.
    ToolUse,
    /// Output was truncated by the max-token cap.
    MaxTokens,
    /// Provider-specific other reason (kept for diagnostics).
    Other,
}

/// The finished result of one streamed model turn.
#[derive(Debug, Clone)]
pub struct TurnOutcome {
    /// Full assistant text (the same content already streamed as deltas).
    pub text: String,
    /// Tool calls the model requested this turn (empty when none).
    pub tool_calls: Vec<ToolCall>,
    /// Prompt tokens, when the provider reported usage.
    pub input_tokens: Option<u64>,
    /// Completion tokens, when the provider reported usage.
    pub output_tokens: Option<u64>,
    /// Stop reason mapped to a provider-neutral enum.
    pub stop: StopKind,
}

// ── Errors ─────────────────────────────────────────────────────────

/// Errors surfaced by the provider client.
#[derive(Debug, Error)]
pub enum AiError {
    /// Transport-level failure (DNS, TLS, connect, mid-stream read).
    #[error("network error: {0}")]
    Http(String),
    /// Non-2xx response from the provider.
    #[error("provider returned HTTP {status}: {message}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Trimmed error message extracted from the response body.
        message: String,
    },
    /// The caller's `CancellationToken` fired.
    #[error("cancelled")]
    Cancelled,
    /// The stream violated the expected wire protocol.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// The provider requires an API key and none is configured.
    #[error("no API key configured for this provider")]
    MissingKey,
}
