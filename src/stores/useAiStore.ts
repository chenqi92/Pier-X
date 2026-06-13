// ── AI conversation UI state ───────────────────────────────────────
// One conversation per tab (PRODUCT-SPEC §5.14.1), keyed by tab id
// ("no-tab" on the welcome view). The store is a pure reducer over
// `ai-chat` events — the SAME payload shapes come from the live event
// channel and from `aiReplay` (transcript), so reopening the app
// re-renders history through one code path.
//
// UI state only (CLAUDE.md Rule 3): risk gating, allowlists, and
// execution all live in the backend; this store just renders what
// the backend reports.

import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { AI_CHAT_EVENT, type AiAttachment, type AiChatEvent, type AiRisk } from "../lib/ai";

export type AiToolStatus =
  | "awaiting"
  | "running"
  | "blocked"
  | "done"
  | "denied"
  | "error";

export type AiUiMessage =
  | { type: "user"; text: string }
  | { type: "assistant"; text: string; streaming: boolean }
  | {
      type: "tool";
      callId: string;
      name: string;
      summary: string;
      explanation?: string;
      host: string;
      risk: AiRisk;
      status: AiToolStatus;
      auto?: string;
      alwaysPrefix?: string;
      exitCode?: number;
      output?: string;
      durationMs?: number;
      decision?: string;
      denyReason?: string;
    }
  | { type: "notice"; tone: "info" | "error"; text: string };

export type AiConversation = {
  messages: AiUiMessage[];
  running: boolean;
  /** Σ token usage reported by the provider for this conversation. */
  inputTokens: number;
  outputTokens: number;
  /** Transcript replay finished (or attempted) for this id. */
  loaded: boolean;
};

const EMPTY: AiConversation = {
  messages: [],
  running: false,
  inputTokens: 0,
  outputTokens: 0,
  loaded: false,
};

type AiState = {
  convs: Record<string, AiConversation>;
  /** Attachments staged for the NEXT message, per conversation —
   *  filled by "Ask AI" entries elsewhere in the app (terminal
   *  selection / screen output) and consumed by the panel's send.
   *  §5.14.2: what goes out must be visible, so the panel renders
   *  these as removable chips before anything is sent. */
  pending: Record<string, AiAttachment[]>;
  /** Apply one live event or one replayed transcript entry. */
  applyEvent: (ev: AiChatEvent, opts?: { replay?: boolean }) => void;
  beginTurn: (conversationId: string, userText: string) => void;
  markIdle: (conversationId: string) => void;
  loadReplay: (conversationId: string, entries: AiChatEvent[]) => void;
  reset: (conversationId: string) => void;
  addPendingAttachment: (conversationId: string, att: AiAttachment) => void;
  removePendingAttachment: (conversationId: string, index: number) => void;
  clearPendingAttachments: (conversationId: string) => void;
};

function conv(state: AiState, id: string): AiConversation {
  return state.convs[id] ?? EMPTY;
}

function withConv(
  state: AiState,
  id: string,
  next: (c: AiConversation) => AiConversation,
): Pick<AiState, "convs"> {
  return { convs: { ...state.convs, [id]: next(conv(state, id)) } };
}

function reduceEvent(c: AiConversation, ev: AiChatEvent, replay: boolean): AiConversation {
  switch (ev.kind) {
    case "user":
      return { ...c, messages: [...c.messages, { type: "user", text: ev.text ?? "" }] };
    case "delta": {
      const messages = [...c.messages];
      const last = messages[messages.length - 1];
      if (last && last.type === "assistant" && last.streaming) {
        messages[messages.length - 1] = { ...last, text: last.text + (ev.text ?? "") };
      } else {
        messages.push({ type: "assistant", text: ev.text ?? "", streaming: true });
      }
      return { ...c, messages, running: true };
    }
    case "assistant": {
      // Replay path: full assistant text in one entry. Live path uses
      // deltas, so skip duplicates when a streaming bubble exists.
      if (!replay) return c;
      return {
        ...c,
        messages: [...c.messages, { type: "assistant", text: ev.text ?? "", streaming: false }],
      };
    }
    case "toolCall": {
      const callId = ev.callId ?? "";
      const status: AiToolStatus = ev.status === "blocked"
        ? "blocked"
        : ev.status === "awaiting"
          ? "awaiting"
          : "running";
      const messages = [...c.messages];
      // Close any streaming assistant bubble first — the model's
      // preamble text for this tool call is complete.
      const lastIdx = messages.length - 1;
      const last = messages[lastIdx];
      if (last && last.type === "assistant" && last.streaming) {
        messages[lastIdx] = { ...last, streaming: false };
      }
      const existing = messages.findIndex((m) => m.type === "tool" && m.callId === callId);
      const base = {
        type: "tool" as const,
        callId,
        name: ev.name ?? "",
        summary: ev.summary ?? "",
        explanation: ev.explanation,
        host: ev.host ?? "",
        risk: ev.risk ?? { level: "l2", reasons: [], asRoot: false },
        status,
        auto: ev.auto,
        alwaysPrefix: ev.alwaysPrefix,
      };
      if (existing >= 0) {
        const prev = messages[existing];
        if (prev.type === "tool") {
          messages[existing] = { ...prev, ...base, status };
        }
      } else {
        messages.push(base);
      }
      return { ...c, messages };
    }
    case "toolResult": {
      const callId = ev.callId ?? "";
      const messages = c.messages.map((m) => {
        if (m.type !== "tool" || m.callId !== callId) return m;
        const status: AiToolStatus = ev.decision === "deny"
          ? "denied"
          : ev.isError
            ? "error"
            : "done";
        return {
          ...m,
          status,
          exitCode: ev.exitCode,
          output: ev.output,
          durationMs: ev.durationMs,
          decision: ev.decision,
          denyReason: ev.denyReason,
        };
      });
      return { ...c, messages };
    }
    case "usage":
      return {
        ...c,
        inputTokens: c.inputTokens + (ev.inputTokens ?? 0),
        outputTokens: c.outputTokens + (ev.outputTokens ?? 0),
      };
    case "scrub": {
      const hits = (ev.hits ?? []).join(", ");
      return {
        ...c,
        messages: [
          ...c.messages,
          { type: "notice", tone: "info", text: `redacted: ${hits}` },
        ],
      };
    }
    case "done": {
      const messages = c.messages.map((m) =>
        m.type === "assistant" && m.streaming ? { ...m, streaming: false } : m,
      );
      if (ev.cancelled && !replay) {
        messages.push({ type: "notice", tone: "info", text: "cancelled" });
      }
      return { ...c, messages, running: false };
    }
    case "failed": {
      const messages = c.messages.map((m) =>
        m.type === "assistant" && m.streaming ? { ...m, streaming: false } : m,
      );
      messages.push({ type: "notice", tone: "error", text: ev.message ?? "request failed" });
      return { ...c, messages, running: false };
    }
    default:
      return c;
  }
}

export const useAiStore = create<AiState>((set) => ({
  convs: {},
  pending: {},
  applyEvent: (ev, opts) =>
    set((state) => {
      if (!ev.conversationId) return state;
      return withConv(state, ev.conversationId, (c) => reduceEvent(c, ev, opts?.replay ?? false));
    }),
  beginTurn: (conversationId, userText) =>
    set((state) =>
      withConv(state, conversationId, (c) => ({
        ...c,
        running: true,
        messages: [...c.messages, { type: "user", text: userText }],
      })),
    ),
  markIdle: (conversationId) =>
    set((state) => withConv(state, conversationId, (c) => ({ ...c, running: false }))),
  loadReplay: (conversationId, entries) =>
    set((state) => {
      let c: AiConversation = { ...EMPTY, loaded: true };
      for (const ev of entries) {
        c = reduceEvent(c, { ...ev, conversationId }, true);
      }
      c.running = false;
      // Pending approvals do not survive a restart — the backend's
      // decision channel is gone, so render them as denied-by-restart.
      c.messages = c.messages.map((m) =>
        m.type === "tool" && (m.status === "awaiting" || m.status === "running")
          ? { ...m, status: "denied" as const, denyReason: "app restarted" }
          : m,
      );
      return { convs: { ...state.convs, [conversationId]: c } };
    }),
  reset: (conversationId) =>
    set((state) => ({
      convs: { ...state.convs, [conversationId]: { ...EMPTY, loaded: true } },
    })),
  addPendingAttachment: (conversationId, att) =>
    set((state) => ({
      pending: {
        ...state.pending,
        [conversationId]: [...(state.pending[conversationId] ?? []), att],
      },
    })),
  removePendingAttachment: (conversationId, index) =>
    set((state) => ({
      pending: {
        ...state.pending,
        [conversationId]: (state.pending[conversationId] ?? []).filter((_, i) => i !== index),
      },
    })),
  clearPendingAttachments: (conversationId) =>
    set((state) => ({
      pending: { ...state.pending, [conversationId]: [] },
    })),
}));

// ── Global event listener (singleton) ──────────────────────────────
// Registered once per webview so events keep landing while the panel
// is hidden (keep-alive slot) or the user is on another tab.

let listenerStarted = false;

export function ensureAiListener() {
  if (listenerStarted) return;
  listenerStarted = true;
  void listen<AiChatEvent>(AI_CHAT_EVENT, (event) => {
    useAiStore.getState().applyEvent(event.payload);
  });
}
