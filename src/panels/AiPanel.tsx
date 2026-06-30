// ── AI assistant panel (PRODUCT-SPEC §5.14) ────────────────────────
// Free-form chat + risk-gated tool execution against the CURRENT
// tab's host. The panel is render-only on the safety path: risk
// levels, allowlists, and red lines are computed and enforced in
// the backend — this file just draws what the backend reports and
// forwards the user's decision.

import { type RefObject, useEffect, useMemo, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  CircleStop,
  Copy,
  FileDown,
  Paperclip,
  SendHorizontal,
  ShieldAlert,
  ShieldX,
  Sparkles,
  SquareTerminal,
  Trash2,
  X,
} from "lucide-react";
import type { TabState, TerminalLine, TerminalSessionInfo, TerminalSnapshot } from "../lib/types";
import { effectiveSshTarget, isSshTargetReady } from "../lib/types";
import * as ai from "../lib/ai";
import type { AiRiskLevel, AiToolDecision } from "../lib/ai";
import * as cmd from "../lib/commands";
import { writeClipboardText } from "../lib/clipboard";
import { renderMarkdown } from "../lib/markdown";
import {
  ensureAiListener,
  useAiStore,
  type AiUiMessage,
} from "../stores/useAiStore";
import { useSettingsStore } from "../stores/useSettingsStore";
import { aiVendorById } from "../lib/aiVendors";
import { useDetectedServicesStore } from "../stores/useDetectedServicesStore";
import { useUiActionsStore } from "../stores/useUiActionsStore";
import { TERMINAL_THEMES, useThemeStore } from "../stores/useThemeStore";
import { useI18n } from "../i18n/useI18n";
import IconButton from "../components/IconButton";
import Select from "../components/Select";
import "../styles/ai-panel.css";

type Props = {
  tab: TabState | null;
  isActive: boolean;
};

const RISK_LABEL: Record<AiRiskLevel, string> = {
  l0: "L0",
  l1: "L1",
  l2: "L2",
  l3: "L3",
};

function riskClass(level: AiRiskLevel): string {
  switch (level) {
    case "l0":
      return "is-l0";
    case "l1":
      return "is-l1";
    case "l2":
      return "is-l2";
    case "l3":
      return "is-l3";
  }
}

const AI_NATIVE_CLI_COLS = 100;
const AI_NATIVE_CLI_ROWS = 24;

type TerminalEventPayload = {
  sessionId: string;
  kind: "data" | "exit";
  snapshot?: TerminalSnapshot | null;
};

// ── Assistant-text fence parsing ────────────────────────────────────
// Minimal markdown: only ``` fences are structured (they get copy /
// insert-into-terminal buttons, §5.14.5); everything else renders as
// plain pre-wrapped text. An unclosed fence (mid-stream) renders as a
// code block so the UI doesn't jump when the closing fence arrives.

type AssistSeg = { kind: "text"; text: string } | { kind: "code"; lang: string; code: string };

function splitFences(text: string): AssistSeg[] {
  const segs: AssistSeg[] = [];
  const lines = text.split("\n");
  let buf: string[] = [];
  let inCode = false;
  let lang = "";
  const flush = () => {
    if (inCode) {
      segs.push({ kind: "code", lang, code: buf.join("\n") });
    } else if (buf.join("\n").trim()) {
      segs.push({ kind: "text", text: buf.join("\n") });
    }
    buf = [];
  };
  for (const line of lines) {
    const fence = /^\s*```(.*)$/.exec(line);
    if (fence) {
      flush();
      if (!inCode) lang = fence[1].trim();
      inCode = !inCode;
      continue;
    }
    buf.push(line);
  }
  flush();
  return segs;
}

function isPierxProtocolFence(seg: AssistSeg): boolean {
  return (
    seg.kind === "code" &&
    ["pierx-run", "pierx"].includes(seg.lang.trim().toLowerCase())
  );
}

function hasVisibleAssistantContent(segs: AssistSeg[]): boolean {
  return segs.some((seg) =>
    seg.kind === "text" ? seg.text.trim().length > 0 : seg.code.trim().length > 0,
  );
}

function AiWait({ label }: { label: string }) {
  return (
    <div className="ai-wait" aria-live="polite">
      <span className="ai-wait__dots" aria-hidden="true">
        <span />
        <span />
        <span />
      </span>
      <span>{label}</span>
    </div>
  );
}

// ── User-bubble attachment parsing ──────────────────────────────────
// The backend composes attachments into the user message as
// `\n\n[attached: label]\n```\nbody\n````. Render them collapsed so a
// 60 KB log attach doesn't drown the conversation; expanding shows
// exactly what was sent (§5.14.2 visibility requirement).

function splitUserAttachments(text: string): { head: string; atts: { label: string; body: string }[] } {
  const atts: { label: string; body: string }[] = [];
  const re = /\n\n\[attached: ([^\]]*)\]\n```\n([\s\S]*?)\n```/g;
  let head = text;
  const first = text.search(/\n\n\[attached: /);
  if (first >= 0) head = text.slice(0, first);
  for (let m = re.exec(text); m !== null; m = re.exec(text)) {
    atts.push({ label: m[1], body: m[2] });
  }
  return { head, atts };
}

export default function AiPanel({ tab, isActive }: Props) {
  const { t } = useI18n();
  const conversationId = tab?.id ?? "no-tab";

  const settings = useSettingsStore();
  const requestOpenSettings = useUiActionsStore((s) => s.requestOpenSettings);
  const requestFocusTerminal = useUiActionsStore((s) => s.requestFocusTerminal);
  const detectedTools = useDetectedServicesStore((s) =>
    tab ? s.byTab[tab.id]?.tools : undefined,
  );

  const conv = useAiStore((s) => s.convs[conversationId]);
  const beginTurn = useAiStore((s) => s.beginTurn);
  const markIdle = useAiStore((s) => s.markIdle);
  const applyEvent = useAiStore((s) => s.applyEvent);
  const loadReplay = useAiStore((s) => s.loadReplay);
  const reset = useAiStore((s) => s.reset);
  const pendingAtts = useAiStore((s) => s.pending[conversationId]) ?? [];
  const removePendingAttachment = useAiStore((s) => s.removePendingAttachment);
  const clearPendingAttachments = useAiStore((s) => s.clearPendingAttachments);

  const [input, setInput] = useState("");
  const [note, setNote] = useState("");
  const [nativeSession, setNativeSession] = useState<TerminalSessionInfo | null>(null);
  const [nativeSnapshot, setNativeSnapshot] = useState<TerminalSnapshot | null>(null);
  const [nativeDraft, setNativeDraft] = useState("");
  const [nativeStarting, setNativeStarting] = useState(false);
  const [nativeError, setNativeError] = useState("");
  const noteTimer = useRef<number | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const nativeSessionRef = useRef<TerminalSessionInfo | null>(null);
  const nativeScreenRef = useRef<HTMLDivElement | null>(null);
  nativeSessionRef.current = nativeSession;

  const terminalThemeIndex = useThemeStore((s) => s.terminalThemeIndex);
  const resolvedDark = useThemeStore((s) => s.resolvedDark);
  const termTheme =
    TERMINAL_THEMES[terminalThemeIndex] ?? TERMINAL_THEMES[resolvedDark ? 0 : 1];
  const cliVendor = aiVendorById(settings.aiVendorId);
  const providerSettings = useMemo<ai.AiProviderSettings>(
    () => ({
      kind: settings.aiProviderKind,
      baseUrl: settings.aiBaseUrl,
      model: settings.aiModel,
      maxTokens: settings.aiMaxTokens > 0 ? settings.aiMaxTokens : null,
      secretId: settings.aiVendorId,
      cliFlavor: cliVendor.cliFlavor ?? null,
      cliBin: settings.aiCliBin || null,
      cliMode: settings.aiCliMode,
    }),
    [
      cliVendor.cliFlavor,
      settings.aiBaseUrl,
      settings.aiCliBin,
      settings.aiCliMode,
      settings.aiMaxTokens,
      settings.aiModel,
      settings.aiProviderKind,
      settings.aiVendorId,
    ],
  );

  const flashNote = (text: string) => {
    setNote(text);
    if (noteTimer.current !== null) window.clearTimeout(noteTimer.current);
    noteTimer.current = window.setTimeout(() => setNote(""), 4000);
  };

  // §5.14.5 "insert, don't execute": write the command into the PTY
  // WITHOUT a trailing newline so the user reviews and presses Enter.
  // Multi-line snippets only insert under bracketed paste (raw `\r`
  // separators would execute every line unreviewed); otherwise they
  // fall back to the clipboard.
  const terminalSessionId = tab?.terminalSessionId ?? null;
  const insertToTerminal = async (code: string) => {
    if (!terminalSessionId) return;
    const text = code.replace(/\s+$/, "");
    if (!text) return;
    try {
      if (!text.includes("\n")) {
        await cmd.terminalWrite(terminalSessionId, text);
        // Return keyboard focus to the terminal so Enter / Ctrl+C land
        // there — otherwise focus stays on this button and the terminal
        // reads as "stuck" until the user clicks it.
        requestFocusTerminal(terminalSessionId);
        flashNote(t("Inserted — review it in the terminal and press Enter to run."));
        return;
      }
      const snap = await cmd.terminalSnapshot(terminalSessionId, 0);
      if (snap.bracketedPaste) {
        await cmd.terminalWrite(
          terminalSessionId,
          "\x1b[200~" + text.replace(/\r?\n/g, "\r") + "\x1b[201~",
        );
        requestFocusTerminal(terminalSessionId);
        flashNote(t("Inserted — review it in the terminal and press Enter to run."));
        return;
      }
      await writeClipboardText(text);
      flashNote(t("Multi-line command copied to clipboard — paste it in the terminal yourself."));
    } catch {
      await writeClipboardText(text).catch(() => {});
      flashNote(t("Multi-line command copied to clipboard — paste it in the terminal yourself."));
    }
  };

  const copyCode = async (code: string) => {
    await writeClipboardText(code.replace(/\s+$/, "")).catch(() => {});
    flashNote(t("Copied."));
  };

  // Copy / export the whole assistant reply as raw markdown — `m.text`
  // is already the model's markdown source (fences and all), so it
  // round-trips cleanly to clipboard or a `.md` file.
  const copyMessage = async (text: string) => {
    await writeClipboardText(text.replace(/\s+$/, "")).catch(() => {});
    flashNote(t("Copied."));
  };

  const exportMarkdown = async (text: string) => {
    try {
      const dialog = await import("@tauri-apps/plugin-dialog");
      const picked = await dialog.save({
        title: t("Save reply as Markdown"),
        defaultPath: defaultMarkdownName(text),
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (typeof picked !== "string") return;
      await cmd.localWriteTextFile(picked, text.replace(/\s+$/, "") + "\n");
      flashNote(t("Exported to {path}", { path: picked }));
    } catch (e) {
      flashNote(String(e));
    }
  };

  const configured =
    settings.aiModel.trim().length > 0 || settings.aiProviderKind === "cli";
  const messages = conv?.messages ?? [];
  const running = conv?.running ?? false;
  const waitingForFirstResponse =
    running && messages[messages.length - 1]?.type === "user";

  // The execution target mirrors the rest of the right sidebar:
  // effective SSH addressing when present + ready, local otherwise.
  const target = tab ? effectiveSshTarget(tab) : null;
  const remoteReady = isSshTargetReady(target);
  const targetLabel = remoteReady && target ? `${target.user}@${target.host}` : t("local");
  const nativeCliLabel = providerSettings.cliFlavor === "codex" ? "Codex" : "Claude Code";
  const nativeCliEnabled = settings.aiProviderKind === "cli";
  const nativeCliStartDisabled = nativeStarting || !nativeCliEnabled || remoteReady;
  const nativeCliStartTitle = remoteReady
    ? t("Native CLI sessions run on this local machine only; SSH tabs are disabled.")
    : nativeCliEnabled
      ? t("Start native CLI terminal")
      : t("Choose Claude Code or Codex in Settings → AI first");

  useEffect(() => {
    ensureAiListener();
  }, []);

  useEffect(() => {
    setNativeSession(null);
    setNativeSnapshot(null);
    setNativeDraft("");
    setNativeError("");
    return () => {
      const session = nativeSessionRef.current;
      if (session) {
        cmd.terminalClose(session.sessionId).catch(() => {});
        nativeSessionRef.current = null;
      }
    };
  }, [conversationId]);

  // Replay the persisted transcript once per conversation id.
  useEffect(() => {
    if (conv?.loaded) return;
    let cancelled = false;
    ai.aiReplay(conversationId)
      .then((entries) => {
        if (!cancelled) loadReplay(conversationId, entries);
      })
      .catch(() => {
        if (!cancelled) loadReplay(conversationId, []);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conversationId, conv?.loaded]);

  useEffect(() => {
    if (!nativeSession) return;
    let disposed = false;
    let inflight = false;
    let dirty = false;
    let rafHandle: number | null = null;
    let pendingPush: TerminalSnapshot | null = null;
    let unlisten: UnlistenFn | undefined;

    const safeUnlisten = (fn: UnlistenFn | undefined) => {
      if (!fn) return;
      try {
        fn();
      } catch {
        // Late Tauri listener cleanup; already gone.
      }
    };

    const applySnapshot = (next: TerminalSnapshot) => {
      if (disposed) return;
      setNativeSnapshot(next);
    };

    const refresh = () => {
      if (disposed) return;
      if (inflight) {
        dirty = true;
        return;
      }
      dirty = false;
      inflight = true;
      cmd
        .terminalSnapshot(nativeSession.sessionId, 0)
        .then(applySnapshot)
        .catch((e) => {
          if (!disposed) setNativeError(String(e));
        })
        .finally(() => {
          inflight = false;
          if (dirty && !disposed) scheduleRefresh();
        });
    };

    const scheduleRefresh = () => {
      if (disposed) return;
      if (rafHandle !== null) return;
      rafHandle = window.requestAnimationFrame(() => {
        rafHandle = null;
        const pushed = pendingPush;
        pendingPush = null;
        if (pushed) applySnapshot(pushed);
        else refresh();
      });
    };

    refresh();
    void listen<TerminalEventPayload>("terminal:event", (event) => {
      if (disposed) return;
      if (event.payload.sessionId !== nativeSession.sessionId) return;
      if (event.payload.snapshot) pendingPush = event.payload.snapshot;
      scheduleRefresh();
    })
      .then((u) => {
        if (disposed) safeUnlisten(u);
        else unlisten = u;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      if (rafHandle !== null) window.cancelAnimationFrame(rafHandle);
      safeUnlisten(unlisten);
    };
  }, [nativeSession]);

  useEffect(() => {
    if (settings.aiProviderKind === "cli") return;
    const session = nativeSessionRef.current;
    if (!session) return;
    setNativeSession(null);
    setNativeSnapshot(null);
    setNativeDraft("");
    setNativeError("");
    cmd.terminalClose(session.sessionId).catch(() => {});
  }, [settings.aiProviderKind]);

  useEffect(() => {
    const el = nativeScreenRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [nativeSnapshot]);

  // Pin to bottom while streaming / on new messages.
  useEffect(() => {
    if (!isActive) return;
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, isActive]);

  const context = useMemo<ai.AiContextPayload | undefined>(() => {
    if (!settings.aiAutoContext) return { backend: tab?.backend ?? "none", locale: settings.locale };
    return {
      backend: tab?.backend ?? "none",
      host: target?.host ?? null,
      user: target?.user ?? null,
      cwd: tab?.lastCwd ?? null,
      services: detectedTools ? Array.from(detectedTools) : null,
      locale: settings.locale,
    };
  }, [settings.aiAutoContext, settings.locale, tab?.backend, tab?.lastCwd, target?.host, target?.user, detectedTools]);

  const startNativeCli = async () => {
    if (!nativeCliEnabled || nativeStarting) return;
    if (remoteReady) {
      flashNote(t("Native CLI sessions are local-only. Switch to a local tab to start one."));
      return;
    }
    const initialPrompt = input.trim();
    setNativeStarting(true);
    setNativeError("");
    try {
      const next = await cmd.terminalCreateAiCli({
        cols: AI_NATIVE_CLI_COLS,
        rows: AI_NATIVE_CLI_ROWS,
        provider: providerSettings,
        initialPrompt: initialPrompt || null,
        cwd: tab?.lastCwd ?? null,
      });
      setNativeSession(next);
      if (initialPrompt) setInput("");
      flashNote(t("Native CLI started. Its own approval model applies."));
    } catch (e) {
      setNativeError(String(e));
    } finally {
      setNativeStarting(false);
    }
  };

  const closeNativeCli = () => {
    const session = nativeSessionRef.current;
    setNativeSession(null);
    setNativeSnapshot(null);
    setNativeDraft("");
    setNativeError("");
    if (session) {
      cmd.terminalClose(session.sessionId).catch(() => {});
    }
  };

  const interruptNativeCli = () => {
    const session = nativeSessionRef.current;
    if (!session) return;
    cmd.terminalWrite(session.sessionId, "\u0003").catch((e) => setNativeError(String(e)));
  };

  const sendNativeInput = () => {
    const session = nativeSessionRef.current;
    if (!session) return;
    const payload =
      nativeDraft.length > 0 ? nativeDraft.replace(/\r?\n/g, "\r") + "\r" : "\r";
    setNativeDraft("");
    cmd.terminalWrite(session.sessionId, payload).catch((e) => setNativeError(String(e)));
  };

  const send = () => {
    const text = input.trim();
    if ((!text && pendingAtts.length === 0) || running || !configured) return;
    const attachments = pendingAtts;
    setInput("");
    clearPendingAttachments(conversationId);
    // Local bubble mirrors the backend's message composition so the
    // user sees exactly what went out (attachments render collapsed).
    let bubble = text;
    for (const a of attachments) {
      bubble += `\n\n[attached: ${a.label}]\n\`\`\`\n${a.content}\n\`\`\``;
    }
    beginTurn(conversationId, bubble);
    const req: ai.AiChatRequest = {
      conversationId,
      provider: providerSettings,
      userText: text,
      context,
      attachments,
      redact: settings.aiRedact,
      askReadOnly: settings.aiAskReadOnly,
      persistHistory: settings.aiPersistHistory,
      ssh:
        remoteReady && target
          ? {
              host: target.host,
              port: target.port,
              user: target.user,
              authMode: target.authMode,
              password: target.password,
              keyPath: target.keyPath,
              savedConnectionIndex: target.savedConnectionIndex,
            }
          : null,
    };
    ai.aiChatSend(req).catch((err) => {
      markIdle(conversationId);
      applyEvent({ conversationId, kind: "failed", message: String(err) });
    });
  };

  const stop = () => {
    void ai.aiChatCancel(conversationId).catch(() => {});
  };

  const clear = () => {
    void ai.aiClear(conversationId).catch(() => {});
    reset(conversationId);
  };

  const decide = (callId: string, decision: AiToolDecision) => {
    void ai.aiToolDecision(conversationId, callId, decision).catch(() => {});
  };

  if (!configured) {
    return (
      <div className="ai-panel">
        <div className="ai-guide">
          <div className="ai-guide__icon">
            <Sparkles size={22} strokeWidth={1.6} />
          </div>
          <div className="ai-guide__title">{t("AI assistant")}</div>
          <div className="ai-guide__subtitle">
            {t(
              "Ask in plain language; the assistant inspects and operates the current tab's host with per-action approval. Configure a model provider to enable it — nothing is sent anywhere until you do.",
            )}
          </div>
          <button type="button" className="btn" onClick={() => requestOpenSettings("Ai")}>
            {t("Open settings")}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="ai-panel">
      <div className="ai-panel__bar">
        <span className="ai-panel__target" title={t("Commands run on this host only")}>
          {targetLabel}
        </span>
        {nativeCliEnabled && (
          <IconButton
            variant="mini"
            title={nativeSession ? t("Native CLI is running") : nativeCliStartTitle}
            onClick={startNativeCli}
            disabled={nativeCliStartDisabled || nativeSession !== null}
            active={nativeSession !== null}
          >
            <SquareTerminal size={14} />
          </IconButton>
        )}
        {settings.aiProfiles.length > 0 && (
          <Select
            compact
            mono
            className="ai-profile-select"
            title={t("Switch model configuration")}
            value={settings.aiActiveProfileId ?? ""}
            onChange={(id) => {
              if (id) settings.activateAiProfile(id);
            }}
            items={[
              ...(settings.aiActiveProfileId === null
                ? [{ value: "", label: t("(draft)") }]
                : []),
              ...settings.aiProfiles.map((p) => ({ value: p.id, label: p.name })),
            ]}
          />
        )}
        <span className="ai-panel__usage">
          {(conv?.inputTokens ?? 0) > 0 || (conv?.outputTokens ?? 0) > 0
            ? `↑${conv?.inputTokens ?? 0} ↓${conv?.outputTokens ?? 0}`
            : ""}
        </span>
        <IconButton
          variant="mini"
          title={t("Clear conversation")}
          onClick={clear}
          disabled={running}
        >
          <Trash2 size={14} />
        </IconButton>
      </div>

      {(nativeSession || nativeStarting || nativeError) && (
        <NativeCliTerminal
          label={nativeCliLabel}
          cwd={tab?.lastCwd ?? null}
          snapshot={nativeSnapshot}
          draft={nativeDraft}
          error={nativeError}
          starting={nativeStarting}
          theme={termTheme}
          screenRef={nativeScreenRef}
          t={t}
          onDraftChange={setNativeDraft}
          onSend={sendNativeInput}
          onInterrupt={interruptNativeCli}
          onClose={closeNativeCli}
        />
      )}

      <div className="ai-panel__list ux-selectable" ref={listRef}>
        {messages.length === 0 && (
          <div className="ai-empty">
            {t("Ask about this host, paste an error to explain, or describe what you want done.")}
          </div>
        )}
        {messages.map((m, i) => (
          <Message
            key={messageKey(m, i)}
            m={m}
            t={t}
            onDecide={decide}
            canInsert={terminalSessionId !== null}
            onInsert={insertToTerminal}
            onCopy={copyCode}
            onCopyMessage={copyMessage}
            onExport={exportMarkdown}
          />
        ))}
        {waitingForFirstResponse && <AiWait label={t("Thinking…")} />}
      </div>

      {note && <div className="ai-flash">{note}</div>}

      {pendingAtts.length > 0 && (
        <div className="ai-attach-row">
          {pendingAtts.map((a, i) => (
            <details key={`${a.label}-${i}`} className="ai-attach-chip">
              <summary title={t("Click to preview what will be sent")}>
                <Paperclip size={11} />
                <span className="ai-attach-chip__label">{a.label}</span>
                <span className="ai-attach-chip__size">{a.content.length}</span>
                <button
                  type="button"
                  className="ai-attach-chip__x"
                  title={t("Remove attachment")}
                  onClick={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    removePendingAttachment(conversationId, i);
                  }}
                >
                  <X size={10} />
                </button>
              </summary>
              <pre className="ai-attach-chip__preview">{a.content}</pre>
            </details>
          ))}
        </div>
      )}

      <div className="ai-panel__composer">
        <textarea
          ref={inputRef}
          className="ai-input"
          rows={2}
          placeholder={t("Ask AI — Enter to send, Shift+Enter for newline")}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              send();
            }
          }}
        />
        {running ? (
          <button type="button" className="btn is-danger ai-send" onClick={stop} title={t("Stop")}>
            <CircleStop size={14} />
          </button>
        ) : (
          <button
            type="button"
            className="btn is-primary ai-send"
            onClick={send}
            disabled={!input.trim() && pendingAtts.length === 0}
            title={t("Send")}
          >
            <SendHorizontal size={14} />
          </button>
        )}
      </div>
    </div>
  );
}

function messageKey(m: AiUiMessage, index: number): string {
  return m.type === "tool" ? `tool-${m.callId}` : `${m.type}-${index}`;
}

// Suggested filename for an exported reply: a slug from the first
// non-empty line (heading markers stripped), capped, with a stable
// fallback so the save dialog always opens with something sensible.
function defaultMarkdownName(text: string): string {
  const firstLine =
    text
      .split("\n")
      .map((l) => l.replace(/^#+\s*/, "").trim())
      .find((l) => l.length > 0) ?? "";
  const slug = firstLine
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40)
    .replace(/-+$/g, "");
  return (slug || "ai-reply") + ".md";
}

type AiTerminalTheme = (typeof TERMINAL_THEMES)[number];

function NativeCliTerminal({
  label,
  cwd,
  snapshot,
  draft,
  error,
  starting,
  theme,
  screenRef,
  t,
  onDraftChange,
  onSend,
  onInterrupt,
  onClose,
}: {
  label: string;
  cwd: string | null;
  snapshot: TerminalSnapshot | null;
  draft: string;
  error: string;
  starting: boolean;
  theme: AiTerminalTheme;
  screenRef: RefObject<HTMLDivElement | null>;
  t: (s: string) => string;
  onDraftChange: (value: string) => void;
  onSend: () => void;
  onInterrupt: () => void;
  onClose: () => void;
}) {
  const alive = snapshot?.alive ?? !error;
  return (
    <section className="ai-native" aria-label={t("Native CLI terminal")}>
      <div className="ai-native__head">
        <div className="ai-native__title">
          <SquareTerminal size={13} />
          <span>{label}</span>
        </div>
        <span className="ai-native__cwd" title={cwd ?? t("local")}>
          {cwd ?? t("local")}
        </span>
        <span className={"ai-native__state" + (alive ? "" : " is-exited")}>
          {starting ? t("starting…") : alive ? t("CLI approvals") : t("exited")}
        </span>
        <IconButton variant="mini" title={t("Send Ctrl+C")} onClick={onInterrupt} disabled={!snapshot}>
          <CircleStop size={13} />
        </IconButton>
        <IconButton variant="mini" title={t("Close native CLI")} onClick={onClose}>
          <X size={13} />
        </IconButton>
      </div>
      <div
        className="ai-native__screen ux-selectable"
        ref={screenRef}
        style={{ background: theme.bg, color: theme.fg }}
      >
        {snapshot ? (
          snapshot.lines.map((line, i) => (
            <NativeTerminalLine key={`${line.hash}-${i}`} line={line} theme={theme} />
          ))
        ) : (
          <div className="ai-native__placeholder">
            {starting ? t("Starting native CLI…") : t("Waiting for terminal output…")}
          </div>
        )}
      </div>
      {error && <div className="ai-native__error">{error}</div>}
      <div className="ai-native__input-row">
        <textarea
          className="ai-native__input"
          rows={1}
          placeholder={t("Type for the CLI — Enter sends, Shift+Enter adds a line")}
          value={draft}
          onChange={(e) => onDraftChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              onSend();
            }
          }}
        />
        <button type="button" className="btn is-primary is-compact" onClick={onSend}>
          <SendHorizontal size={13} />
        </button>
      </div>
    </section>
  );
}

function NativeTerminalLine({ line, theme }: { line: TerminalLine; theme: AiTerminalTheme }) {
  if (line.segments.length === 0) {
    return <div className="ai-native__line">&nbsp;</div>;
  }
  return (
    <div className="ai-native__line">
      {line.segments.map((seg, i) => {
        const fg = resolveTerminalColor(seg.fg, theme.ansi);
        const bg = resolveTerminalColor(seg.bg, theme.ansi);
        return (
          <span
            key={i}
            className={seg.cursor ? "ai-native__segment is-cursor" : "ai-native__segment"}
            style={{
              color: seg.cursor ? theme.bg : fg,
              backgroundColor: seg.cursor ? theme.fg : bg,
              fontWeight: seg.bold ? 600 : undefined,
              textDecoration: seg.underline ? "underline" : undefined,
            }}
          >
            {seg.text || " "}
          </span>
        );
      })}
    </div>
  );
}

function resolveTerminalColor(tag: string, ansi: string[]): string | undefined {
  if (!tag) return undefined;
  if (tag.startsWith("ansi:")) {
    const n = Number.parseInt(tag.slice(5), 10);
    if (!Number.isFinite(n)) return undefined;
    if (n >= 0 && n < 16 && ansi[n]) return ansi[n];
    if (n >= 16 && n <= 231) {
      const value = n - 16;
      const steps = [0, 95, 135, 175, 215, 255];
      const r = steps[Math.floor(value / 36) % 6];
      const g = steps[Math.floor(value / 6) % 6];
      const b = steps[value % 6];
      return toHexColor(r, g, b);
    }
    if (n >= 232 && n <= 255) {
      const shade = 8 + (n - 232) * 10;
      return toHexColor(shade, shade, shade);
    }
    return undefined;
  }
  return tag;
}

function toHexColor(r: number, g: number, b: number): string {
  const hex = (n: number) => Math.max(0, Math.min(255, n)).toString(16).padStart(2, "0");
  return `#${hex(r)}${hex(g)}${hex(b)}`;
}

function Message({
  m,
  t,
  onDecide,
  canInsert,
  onInsert,
  onCopy,
  onCopyMessage,
  onExport,
}: {
  m: AiUiMessage;
  t: (s: string) => string;
  onDecide: (callId: string, decision: AiToolDecision) => void;
  canInsert: boolean;
  onInsert: (code: string) => void;
  onCopy: (code: string) => void;
  onCopyMessage: (text: string) => void;
  onExport: (text: string) => void;
}) {
  if (m.type === "user") {
    const { head, atts } = splitUserAttachments(m.text);
    return (
      <div className="ai-msg is-user">
        {head}
        {atts.map((a, i) => (
          <details key={i} className="ai-att">
            <summary>
              <Paperclip size={11} /> {a.label}
            </summary>
            <pre>{a.body}</pre>
          </details>
        ))}
      </div>
    );
  }
  if (m.type === "assistant") {
    const segs = splitFences(m.text).filter((seg) => !isPierxProtocolFence(seg));
    const hasVisibleContent = hasVisibleAssistantContent(segs);
    if (!hasVisibleContent && m.streaming) return <AiWait label={t("Preparing action…")} />;
    if (!hasVisibleContent && !m.streaming) return null;
    return (
      <div className="ai-msg is-assistant">
        {!m.streaming && hasVisibleContent && (
          <div className="ai-msg__actions ux-chrome">
            <button
              type="button"
              className="ai-msg__btn"
              title={t("Copy message")}
              onClick={() => onCopyMessage(m.text)}
            >
              <Copy size={12} />
            </button>
            <button
              type="button"
              className="ai-msg__btn"
              title={t("Export as Markdown")}
              onClick={() => onExport(m.text)}
            >
              <FileDown size={12} />
            </button>
          </div>
        )}
        {segs.map((seg, i) =>
          seg.kind === "text" ? (
            // Markdown for the prose between top-level code fences:
            // headings, bold/italic, inline `code`, lists,
            // blockquotes, links. Top-level fences are handled by the
            // code branch below so they keep the copy / insert
            // buttons; a fence nested in a blockquote stays inside
            // this markdown render (no buttons — it's an aside, not a
            // run-this suggestion). `renderMarkdown` escapes every
            // leaf, same trust level as the Markdown file preview.
            <div
              key={i}
              className="ai-md"
              dangerouslySetInnerHTML={{ __html: renderMarkdown(seg.text) }}
            />
          ) : (
            <div key={i} className="ai-code">
              <div className="ai-code__bar">
                <span className="ai-code__lang">{seg.lang || "code"}</span>
                <button
                  type="button"
                  className="ai-code__btn"
                  title={t("Copy")}
                  onClick={() => onCopy(seg.code)}
                >
                  <Copy size={11} />
                </button>
                <button
                  type="button"
                  className="ai-code__btn"
                  title={
                    canInsert
                      ? t("Insert into terminal (does not run — you press Enter)")
                      : t("Open the terminal for this tab first")
                  }
                  disabled={!canInsert}
                  onClick={() => onInsert(seg.code)}
                >
                  <SquareTerminal size={11} />
                </button>
              </div>
              <pre>{seg.code}</pre>
            </div>
          ),
        )}
        {m.streaming && <span className="ai-caret" />}
      </div>
    );
  }
  if (m.type === "notice") {
    // Connectivity failures (TCP timeouts, refused, DNS) get a hint —
    // "model list worked but chat times out" usually means the chat
    // endpoint is blocked on this network, not a wrong base URL.
    const isConnErr =
      m.tone === "error" && /10060|10061|timed? ?out|connect|unreachable|dns/i.test(m.text);
    return (
      <div className={"ai-notice" + (m.tone === "error" ? " is-error" : "")}>
        {m.text}
        {isConnErr && (
          <div className="ai-notice__hint">
            {t("Endpoint unreachable from this network — check proxy/firewall, or run Settings → AI → Test connection.")}
          </div>
        )}
      </div>
    );
  }
  return <ToolCard m={m} t={t} onDecide={onDecide} />;
}

function ToolCard({
  m,
  t,
  onDecide,
}: {
  m: Extract<AiUiMessage, { type: "tool" }>;
  t: (s: string) => string;
  onDecide: (callId: string, decision: AiToolDecision) => void;
}) {
  const [confirmText, setConfirmText] = useState("");
  const skipL2Confirm = useSettingsStore((s) => s.aiSkipL2Confirm);
  const level = m.risk.level;
  const headToken = m.summary.trim().split(/\s+/)[0] ?? "";
  const l2Unlocked = level !== "l2" || skipL2Confirm || confirmText.trim() === headToken;

  return (
    <div className={`ai-tool ${riskClass(level)}`}>
      <div className="ai-tool__head">
        <span className={`ai-risk ${riskClass(level)}`}>{RISK_LABEL[level]}</span>
        <span className="ai-tool__name">{m.name}</span>
        <span className="ai-tool__host">@{m.host}</span>
        <span className="ai-tool__status">
          {m.status === "awaiting" && t("waiting for approval")}
          {m.status === "running" && t("running…")}
          {m.status === "blocked" && t("blocked")}
          {m.status === "denied" && t("denied")}
          {m.status === "error" && t("error")}
          {m.status === "done" &&
            (m.exitCode === 0 ? t("done") : `${t("exit")} ${m.exitCode ?? "?"}`)}
          {m.auto === "whitelisted" && ` · ${t("allow-listed")}`}
          {m.auto === "session" && ` · ${t("session grant")}`}
        </span>
      </div>

      {m.explanation && <div className="ai-tool__explain">{m.explanation}</div>}

      {m.summary && <pre className="ai-tool__cmd">{m.summary}</pre>}

      {m.risk.asRoot && (
        <div className="ai-tool__root">
          <ShieldAlert size={12} /> {t("Will run as root")}
        </div>
      )}

      {m.risk.reasons.length > 0 && m.status !== "done" && (
        <div className="ai-tool__reasons">{m.risk.reasons.join(" · ")}</div>
      )}

      {m.status === "blocked" && (
        <div className="ai-tool__blocked">
          <ShieldX size={12} />
          {t("Red line: the AI execution channel is closed for this command. Run it yourself in the terminal if you really need it.")}
        </div>
      )}

      {m.status === "awaiting" && level === "l1" && (
        <div className="ai-tool__actions">
          <button type="button" className="btn is-compact is-primary" onClick={() => onDecide(m.callId, "allow_once")}>
            {t("Allow once")}
          </button>
          {/* Standing grants are offered only when the backend supplies a
              grant key — absent for interpreter/wrapper heads (sh/sudo
              wrappers/…), where a grant would blanket-bypass the classifier. */}
          {m.alwaysPrefix && (
            <>
              <button type="button" className="btn is-compact" onClick={() => onDecide(m.callId, "allow_session")}>
                {t("Allow this session")}
              </button>
              <button
                type="button"
                className="btn is-compact"
                title={`${t("Always allow")}: ${m.alwaysPrefix}`}
                onClick={() => onDecide(m.callId, "allow_always")}
              >
                {t("Always allow")}
                {` “${m.alwaysPrefix}”`}
              </button>
            </>
          )}
          <button type="button" className="btn is-compact is-danger" onClick={() => onDecide(m.callId, "deny")}>
            {t("Deny")}
          </button>
        </div>
      )}

      {m.status === "awaiting" && (level === "l2" || level === "l0") && (
        <div className="ai-tool__actions">
          {level === "l2" && !skipL2Confirm && (
            <input
              className="ai-confirm-input"
              placeholder={`${t("Enter the first word to unlock:")} ${headToken}`}
              value={confirmText}
              onChange={(e) => setConfirmText(e.target.value)}
            />
          )}
          <button
            type="button"
            className={"btn is-compact " + (level === "l2" ? "is-danger" : "is-primary")}
            disabled={!l2Unlocked}
            onClick={() => onDecide(m.callId, "allow_once")}
          >
            {level === "l2" ? t("Execute (high risk)") : t("Allow once")}
          </button>
          <button type="button" className="btn is-compact" onClick={() => onDecide(m.callId, "deny")}>
            {t("Deny")}
          </button>
        </div>
      )}

      {(m.status === "done" || m.status === "error") && m.output && (
        <details className="ai-tool__out">
          <summary>
            {t("Output")}
            {typeof m.durationMs === "number" ? ` · ${m.durationMs} ms` : ""}
          </summary>
          <pre>{m.output}</pre>
        </details>
      )}

      {m.status === "denied" && m.denyReason && (
        <div className="ai-tool__reasons">{m.denyReason}</div>
      )}
    </div>
  );
}
