// ── AI assistant Tauri wrappers ────────────────────────────────────
// Typed `invoke()` wrappers for the AI commands surfaced by
// `src-tauri/src/ai.rs` (PRODUCT-SPEC §5.14). Per CLAUDE.md Rule 4,
// panels never call `invoke()` directly — these helpers are the only
// place the command names appear as raw strings.

import { invoke } from "@tauri-apps/api/core";

export type AiProviderKind = "anthropic" | "openai" | "ollama";

export type AiRiskLevel = "l0" | "l1" | "l2" | "l3";

export type AiRisk = {
  level: AiRiskLevel;
  reasons: string[];
  asRoot: boolean;
};

export type AiProviderSettings = {
  kind: AiProviderKind;
  baseUrl: string;
  model: string;
  maxTokens?: number | null;
  /** Vendor preset id — selects the keyring slot `pier-x.ai.<id>`
   *  so each vendor keeps its own API key. */
  secretId?: string | null;
};

export type AiSshCoords = {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex: number | null;
};

export type AiContextPayload = {
  backend: string;
  host?: string | null;
  user?: string | null;
  cwd?: string | null;
  os?: string | null;
  services?: string[] | null;
  locale?: string | null;
};

export type AiAttachment = { label: string; content: string };

export type AiChatRequest = {
  conversationId: string;
  provider: AiProviderSettings;
  userText: string;
  context?: AiContextPayload;
  attachments: AiAttachment[];
  redact: boolean;
  askReadOnly: boolean;
  ssh?: AiSshCoords | null;
  /** Append this conversation's events to the on-disk transcript;
   *  false = memory-only history (§5.14.1). */
  persistHistory: boolean;
};

export type AiToolDecision = "allow_once" | "allow_session" | "allow_always" | "deny";

export type AiWhitelistEntry = { host: string; prefix: string };

/** Payloads arriving on the `ai-chat` event channel (and from
 *  `aiReplay`, which returns the same shapes from the transcript). */
export type AiChatEvent = {
  conversationId: string;
  kind:
    | "delta"
    | "user"
    | "assistant"
    | "toolCall"
    | "toolResult"
    | "usage"
    | "scrub"
    | "done"
    | "failed";
  // delta / user / assistant
  text?: string;
  // toolCall
  callId?: string;
  name?: string;
  summary?: string;
  /** Model's one-line, plain-language description of the action. */
  explanation?: string;
  host?: string;
  risk?: AiRisk;
  status?: "awaiting" | "running" | "blocked";
  auto?: string;
  alwaysPrefix?: string;
  // toolResult
  exitCode?: number;
  output?: string;
  durationMs?: number;
  decision?: string;
  denyReason?: string;
  isError?: boolean;
  // usage
  inputTokens?: number | null;
  outputTokens?: number | null;
  // scrub
  hits?: string[];
  // done
  cancelled?: boolean;
  truncated?: boolean;
  // failed
  message?: string;
  // replay entries carry the append timestamp
  ts?: number;
};

export const AI_CHAT_EVENT = "ai-chat";

export const aiChatSend = (req: AiChatRequest) => invoke<void>("ai_chat_send", { req });

export const aiChatCancel = (conversationId: string) =>
  invoke<void>("ai_chat_cancel", { conversationId });

export const aiToolDecision = (
  conversationId: string,
  callId: string,
  decision: AiToolDecision,
  denyReason?: string,
) => invoke<void>("ai_tool_decision", { conversationId, callId, decision, denyReason });

export const aiSecretSet = (id: string, value: string) =>
  invoke<void>("ai_secret_set", { id, value });

export const aiSecretStatus = (id: string) => invoke<boolean>("ai_secret_status", { id });

export const aiTestConnection = (provider: AiProviderSettings) =>
  invoke<string>("ai_test_connection", { provider });

/** Enumerate the models the configured endpoint serves
 *  (`GET /models`). Vendors without a listing endpoint reject —
 *  the caller degrades to manual model entry. */
export const aiListModels = (provider: AiProviderSettings) =>
  invoke<string[]>("ai_list_models", { provider });

export const aiWhitelistList = () => invoke<AiWhitelistEntry[]>("ai_whitelist_list");

export const aiWhitelistRemove = (host: string, prefix: string) =>
  invoke<void>("ai_whitelist_remove", { host, prefix });

export const aiReplay = (conversationId: string) =>
  invoke<AiChatEvent[]>("ai_replay", { conversationId });

export const aiClear = (conversationId: string) => invoke<void>("ai_clear", { conversationId });
