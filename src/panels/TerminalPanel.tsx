// ── Terminal Panel ───────────────────────────────────────────────
// Per-tab terminal: event-driven snapshot refresh (with a slow safety
// interval), keyboard I/O, scrollback, and session lifecycle management.

import { KeyRound, SquareTerminal } from "lucide-react";
import { startTransition, useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import * as cmd from "../lib/commands";
import { controlKeyMap } from "../lib/commands";
import ContextMenu, { type ContextMenuItem } from "../components/ContextMenu";
import { useI18n } from "../i18n/useI18n";
import { isMissingKeychainError, localizeError } from "../i18n/localizeMessage";
import type { TabState, TerminalSessionInfo, TerminalSnapshot, TerminalSize } from "../lib/types";
import { useTabStore } from "../stores/useTabStore";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useStatusStore } from "../stores/useStatusStore";
import { useThemeStore, TERMINAL_THEMES } from "../stores/useThemeStore";
import { parseSshCommand } from "../lib/parseSshCommand";
import { useConnectionStore } from "../stores/useConnectionStore";

/**
 * Resolve a backend-emitted color tag against the user's selected terminal
 * theme palette.
 *
 * Backend tags (see `render_terminal_color` in `src-tauri/src/lib.rs`):
 * - `""` → default fg/bg (returns `undefined` to inherit from the
 *   parent `<div className="terminal-screen">` which is painted with
 *   `termTheme.fg` / `termTheme.bg`).
 * - `"ansi:N"` → N in 0..=15 maps directly to `termTheme.ansi[N]`.
 *   N in 16..=231 is the 6×6×6 color cube, N in 232..=255 is grayscale —
 *   both get calculated hex values (themes don't re-skin these).
 * - `"#rrggbb"` → truecolor from ANSI SGR 38/48;2;r;g;b — pass through.
 */
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
      return `rgb(${r},${g},${b})`;
    }
    if (n >= 232 && n <= 255) {
      const shade = 8 + (n - 232) * 10;
      return `rgb(${shade},${shade},${shade})`;
    }
    return undefined;
  }
  return tag;
}

type Props = {
  tab: TabState;
  isActive: boolean;
  /** Open the saved-connection editor when the keychain has lost the
   *  password for this tab's saved connection. */
  onEditConnection?: (index: number) => void;
};

export default function TerminalPanel({ tab, isActive, onEditConnection }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const updateTab = useTabStore((s) => s.updateTab);
  const terminalFontSize = useSettingsStore((s) => s.terminalFontSize);
  const monoFont = useSettingsStore((s) => s.monoFontFamily);
  const cursorStyle = useSettingsStore((s) => s.cursorStyle);
  const cursorBlink = useSettingsStore((s) => s.cursorBlink);
  const scrollbackLines = useSettingsStore((s) => s.scrollbackLines);
  const visualBell = useSettingsStore((s) => s.visualBell);
  const audioBell = useSettingsStore((s) => s.audioBell);
  const rowSeparators = useSettingsStore((s) => s.terminalRowSeparators);
  const termThemeIdx = useThemeStore((s) => s.terminalThemeIndex);
  const termTheme = TERMINAL_THEMES[termThemeIdx] ?? TERMINAL_THEMES[0];
  const [session, setSession] = useState<TerminalSessionInfo | null>(null);
  const [snapshot, setSnapshot] = useState<TerminalSnapshot | null>(null);
  const [error, setError] = useState("");
  const [needsPasswordRecovery, setNeedsPasswordRecovery] = useState(false);
  const [terminalSize, setTerminalSize] = useState<TerminalSize>({ cols: 120, rows: 26 });
  const setStatusTerminalSize = useStatusStore((s) => s.setTerminalSize);
  const [scrollbackOffset, setScrollbackOffset] = useState(0);
  const [visualBellActive, setVisualBellActive] = useState(false);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number } | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const measureRef = useRef<HTMLSpanElement | null>(null);
  const startupAppliedRef = useRef<string | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const bellTimerRef = useRef<number | null>(null);
  const pendingResizeRef = useRef(false);
  const latestSizeRef = useRef(terminalSize);
  latestSizeRef.current = terminalSize;

  // Mirror of the user's currently-being-typed line so we can
  // recognize `ssh user@host` and resync the right sidebar to that
  // target. Reset on Enter / Ctrl+C / Ctrl+U. Tracks visible
  // characters only — escape sequences for arrow keys, ESC, and
  // function keys are ignored, so an ssh line that's been edited
  // mid-stream may be missed but a freshly typed one is captured
  // accurately. This covers the local-terminal case (`ssh foo@bar`)
  // as well as nested ssh inside an existing SSH session — both
  // funnel through `sendInput`, so the same buffer logic catches
  // both transitions.
  const commandBufferRef = useRef("");

  // Sync session ID to tab store
  useEffect(() => {
    if (session && tab.terminalSessionId !== session.sessionId) {
      updateTab(tab.id, { terminalSessionId: session.sessionId });
    }
  }, [session?.sessionId]);

  // ── Measure terminal grid dimensions ────────────────────────

  useEffect(() => {
    const viewport = viewportRef.current;
    const measure = measureRef.current;
    if (!viewport || !measure) return;

    const recalculate = () => {
      const measureBox = measure.getBoundingClientRect();
      const charWidth = measureBox.width / 10 || 7.8;
      const charHeight = measureBox.height || 19;
      const cols = Math.max(48, Math.min(220, Math.floor((viewport.clientWidth - 24) / charWidth)));
      const rows = Math.max(14, Math.min(72, Math.floor((viewport.clientHeight - 20) / charHeight)));
      setTerminalSize((prev) =>
        prev.cols === cols && prev.rows === rows ? prev : { cols, rows },
      );
    };

    recalculate();
    const observer = new ResizeObserver(recalculate);
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [terminalFontSize, monoFont]);

  // ── Create session ──────────────────────────────────────────

  useEffect(() => {
    if (session) return;
    let cancelled = false;

    async function create() {
      try {
        let next: TerminalSessionInfo;
        if (tab.backend === "ssh") {
          if (tab.sshSavedConnectionIndex !== null) {
            // Saved connection — backend resolves password from secure store
            next = await cmd.terminalCreateSshSaved(
              terminalSize.cols,
              terminalSize.rows,
              tab.sshSavedConnectionIndex,
            );
          } else {
            next = await cmd.terminalCreateSsh({
              cols: terminalSize.cols,
              rows: terminalSize.rows,
              host: tab.sshHost,
              port: tab.sshPort,
              user: tab.sshUser,
              authMode: tab.sshAuthMode,
              password: tab.sshPassword,
              keyPath: tab.sshKeyPath,
            });
          }
        } else {
          next = await cmd.terminalCreate(terminalSize.cols, terminalSize.rows);
        }
        if (!cancelled) {
          setSession(next);
          setError("");
          setNeedsPasswordRecovery(false);
        }
      } catch (e) {
        if (!cancelled) {
          setError(formatError(e));
          if (isMissingKeychainError(e)) setNeedsPasswordRecovery(true);
        }
      }
    }

    void create();
    return () => { cancelled = true; };
  }, [session, terminalSize.cols, terminalSize.rows, tab.backend, tab.sshHost]);

  // ── Resize session (trigger-based) ──────────────────────────
  //
  // Dragging a resize handle compresses the terminal viewport many
  // times per frame. Sending SIGWINCH on every tick makes the shell
  // reflow at intermediate (often min-clamped) widths, and any
  // content wrapped at that narrower width can't un-wrap when the
  // viewport grows back — so text appears to vanish after a drag.
  //
  // Instead: while a resize handle is actively being dragged
  // (document.body.is-resizing, set by ResizeHandle), record that a
  // resize is pending and skip the PTY call. When the drag releases,
  // the global mouseup listener below fires exactly one SIGWINCH
  // with the final size.
  useEffect(() => {
    if (!session) return;
    if (document.body.classList.contains("is-resizing")) {
      pendingResizeRef.current = true;
      return;
    }
    pendingResizeRef.current = false;
    cmd.terminalResize(session.sessionId, terminalSize.cols, terminalSize.rows).catch((e) =>
      setError(formatError(e)),
    );
  }, [session, terminalSize.cols, terminalSize.rows]);

  useEffect(() => {
    if (!session) return;
    const onMouseUp = () => {
      if (!pendingResizeRef.current) return;
      // ResizeHandle clears the is-resizing class in its own mouseup
      // listener; defer to a microtask so that runs first regardless
      // of listener registration order.
      queueMicrotask(() => {
        if (!pendingResizeRef.current) return;
        if (document.body.classList.contains("is-resizing")) return;
        pendingResizeRef.current = false;
        const size = latestSizeRef.current;
        cmd.terminalResize(session.sessionId, size.cols, size.rows).catch((e) =>
          setError(formatError(e)),
        );
      });
    };
    window.addEventListener("mouseup", onMouseUp);
    return () => window.removeEventListener("mouseup", onMouseUp);
  }, [session]);

  useEffect(() => {
    setStatusTerminalSize(terminalSize.cols, terminalSize.rows);
    return () => setStatusTerminalSize(null, null);
  }, [terminalSize.cols, terminalSize.rows, setStatusTerminalSize]);

  // ── Apply scrollback settings ───────────────────────────────

  useEffect(() => {
    if (!session) {
      return;
    }
    cmd.terminalSetScrollbackLimit(session.sessionId, scrollbackLines).catch((e) =>
      setError(formatError(e)),
    );
  }, [session?.sessionId, scrollbackLines]);

  // ── Run startup command once per created session ─────────────

  useEffect(() => {
    if (!session || !tab.startupCommand.trim()) {
      return;
    }

    const startupKey = `${tab.id}:${session.sessionId}:${tab.startupCommand}`;
    if (startupAppliedRef.current === startupKey) {
      return;
    }
    startupAppliedRef.current = startupKey;

    cmd.terminalWrite(session.sessionId, `${tab.startupCommand}\r`)
      .then(() => {
        updateTab(tab.id, { startupCommand: "" });
      })
      .catch((e) => {
        setError(formatError(e));
      });
  }, [session?.sessionId, tab.id, tab.startupCommand]);

  // ── Snapshot refresh (event-driven + slow safety interval) ──
  //
  // Backend emits `terminal:event` via PierTerminal's notify callback
  // whenever output arrives (coalesced to ≤16ms in Rust). We fetch a
  // fresh snapshot on each event; the `inflight` guard plus `dirty`
  // bit ensures bursty output still only takes one in-flight IPC at a
  // time and guarantees a trailing refresh so we don't miss the final
  // frame. The 1500ms interval is a safety net for dropped events
  // (tab-background throttling, throttled bursts).

  useEffect(() => {
    if (!session) return;
    let disposed = false;
    let inflight = false;
    let dirty = false;

    const refresh = () => {
      if (disposed) return;
      if (inflight) { dirty = true; return; }
      dirty = false;
      inflight = true;
      cmd
        .terminalSnapshot(session.sessionId, scrollbackOffset)
        .then((next) => {
          if (disposed) return;
          if (scrollbackOffset > next.scrollbackLen) {
            setScrollbackOffset(next.scrollbackLen);
          }
          startTransition(() => setSnapshot(next));
          setError("");
        })
        .catch((e) => {
          if (!disposed) setError(formatError(e));
        })
        .finally(() => {
          inflight = false;
          if (dirty && !disposed) refresh();
        });
    };

    refresh();

    // Listen for backend-pushed events. Each TerminalPanel subscribes;
    // the payload carries `sessionId` so we filter other tabs out.
    let unlisten: UnlistenFn | undefined;
    type TerminalEventPayload = { sessionId: string; kind: "data" | "exit" };
    void listen<TerminalEventPayload>("terminal:event", (event) => {
      if (disposed) return;
      if (event.payload.sessionId !== session.sessionId) return;
      refresh();
    }).then((u) => {
      if (disposed) u();
      else unlisten = u;
    });

    // Safety interval: catches any event we might miss (backend throttle,
    // webview backgrounding, burst overflow). 1500ms is much cheaper than
    // the old 80ms polling but still keeps the UI eventually-consistent.
    const safety = window.setInterval(refresh, 1500);

    return () => {
      disposed = true;
      window.clearInterval(safety);
      if (unlisten) unlisten();
    };
  }, [session, scrollbackOffset]);

  // ── Cleanup on unmount ──────────────────────────────────────

  useEffect(() => {
    return () => {
      if (bellTimerRef.current !== null) {
        window.clearTimeout(bellTimerRef.current);
      }
      if (audioContextRef.current) {
        void audioContextRef.current.close().catch(() => {});
        audioContextRef.current = null;
      }
      if (session) {
        cmd.terminalClose(session.sessionId).catch(() => {});
      }
    };
  }, [session]);

  // ── Bell handling ───────────────────────────────────────────

  useEffect(() => {
    if (!snapshot?.bellPending) {
      return;
    }

    if (visualBell) {
      setVisualBellActive(true);
      if (bellTimerRef.current !== null) {
        window.clearTimeout(bellTimerRef.current);
      }
      bellTimerRef.current = window.setTimeout(() => {
        setVisualBellActive(false);
        bellTimerRef.current = null;
      }, 140);
    }

    if (audioBell) {
      try {
        const AudioCtx = window.AudioContext || (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
        if (AudioCtx) {
          if (!audioContextRef.current) {
            audioContextRef.current = new AudioCtx();
          }
          const context = audioContextRef.current;
          if (context.state === "suspended") {
            void context.resume().catch(() => {});
          }
          const oscillator = context.createOscillator();
          const gain = context.createGain();
          oscillator.type = "sine";
          oscillator.frequency.value = 880;
          gain.gain.value = 0.035;
          oscillator.connect(gain);
          gain.connect(context.destination);
          const now = context.currentTime;
          oscillator.start(now);
          oscillator.stop(now + 0.08);
        }
      } catch {
        // Ignore audio failures; visual bell still covers the event.
      }
    }
  }, [snapshot?.bellPending, visualBell, audioBell]);

  // ── Input handlers ──────────────────────────────────────────

  /**
   * Update the "currently typing" buffer by interpreting the bytes
   * we are about to send to the PTY. Only meaningful for line-edit
   * sequences we can model cheaply: printable text, backspace,
   * carriage return, Ctrl+U (line kill), Ctrl+C / Ctrl+D (cancel).
   * Anything else (arrow keys, escape sequences, big paste blobs)
   * resets the buffer rather than risk drifting out of sync with
   * what the shell actually has in its prompt.
   */
  function updateCommandBuffer(data: string): void {
    // Bracketed-paste payloads, control sequences, multi-line input —
    // bail rather than try to model them.
    if (data.length > 1 && !data.endsWith("\r") && !data.endsWith("\n")) {
      commandBufferRef.current = "";
      return;
    }
    if (data === "\r" || data === "\n" || data === "\r\n") {
      // Caller fires the parser separately; here we just clear so the
      // next prompt starts fresh.
      commandBufferRef.current = "";
      return;
    }
    if (data === "" || data === "\b") {
      commandBufferRef.current = commandBufferRef.current.slice(0, -1);
      return;
    }
    if (data === "" /* ^C */ || data === "" /* ^U */ || data === "" /* ESC */) {
      commandBufferRef.current = "";
      return;
    }
    if (data.charCodeAt(0) < 0x20) {
      // Other control bytes — could be a Ctrl+something we don't model.
      // Reset to avoid wrong attribution on the next Enter.
      commandBufferRef.current = "";
      return;
    }
    if (data.length === 1) {
      commandBufferRef.current += data;
    }
  }

  /**
   * Apply a freshly parsed `ssh user@host` invocation to this tab so
   * the right sidebar (Server Monitor, Detected Services, …) reflects
   * the host the user is connecting to. Matches against saved
   * connections by host+user+port — when one matches, we set the
   * saved-connection index so panel commands can resolve the
   * keychain password automatically.
   *
   * For local-backend tabs we update the primary `ssh*` fields
   * directly: nothing else on the tab is reading them, and panels
   * pick them up via `effectiveSshTarget`. For SSH-backend tabs
   * (nested ssh) we instead write to `nestedSshTarget` so the live
   * PTY session, tab title, and any MySQL/PG/Redis tunnels stay
   * pinned to the original host while the right panel monitors the
   * nested target.
   */
  function applySshContextFromCommand(line: string): void {
    const parsed = parseSshCommand(line);
    if (!parsed) return;
    const conns = useConnectionStore.getState().connections;
    const port = parsed.port > 0 ? parsed.port : 22;
    // Match priority: exact host+user+port → host+user → host alone.
    const sameHostUser = (c: { host: string; user: string }) =>
      c.host.trim().toLowerCase() === parsed.host.toLowerCase()
      && (parsed.user === "" || c.user.trim().toLowerCase() === parsed.user.toLowerCase());
    const matched =
      conns.find((c) => sameHostUser(c) && (c.port || 22) === port)
      ?? conns.find((c) => sameHostUser(c))
      ?? conns.find((c) => c.host.trim().toLowerCase() === parsed.host.toLowerCase());

    const inferredUser = parsed.user || matched?.user || "";
    if (!inferredUser) return; // Without a user we can't probe meaningfully.

    // When no saved connection matches, try the SSH agent rather than
    // attempting password auth with an empty string — the agent will
    // either authenticate cleanly or fail with a typed AuthRejected
    // error, both of which surface a clearer message than the
    // empty-DirectPassword "host, user, port and auth must all be set".
    const authMode: "password" | "agent" | "key" =
      matched?.authKind ?? (parsed.identityPath ? "key" : "agent");
    const keyPath = parsed.identityPath || matched?.keyPath || "";
    const savedConnectionIndex = matched ? matched.index : null;

    if (tab.backend === "local") {
      updateTab(tab.id, {
        sshHost: parsed.host,
        sshPort: port,
        sshUser: inferredUser,
        sshAuthMode: authMode,
        sshKeyPath: keyPath,
        sshSavedConnectionIndex: savedConnectionIndex,
        // Don't clobber a stored password — but if we're switching to
        // a new host we don't have credentials for, blank it so stale
        // creds from a previous target don't leak into the new probe.
        sshPassword:
          savedConnectionIndex === tab.sshSavedConnectionIndex
            ? tab.sshPassword
            : "",
        nestedSshTarget: null,
        rightTool: "monitor",
      });
    } else {
      // Nested ssh on a real SSH tab — keep primary fields intact so
      // the original session / tunnels keep working; only overlay
      // the new target for monitoring.
      updateTab(tab.id, {
        nestedSshTarget: {
          host: parsed.host,
          user: inferredUser,
          port,
          authMode,
          password: "",
          keyPath,
          savedConnectionIndex,
        },
        rightTool: "monitor",
      });
    }

    // Saved connection: prime the in-memory password (same flow as
    // openSshSaved). Without this the very first probe / detect for
    // the new target would be the one to surface "saved password
    // missing" — we'd rather front-load that and give the recovery
    // button a chance to render before the user notices.
    if (matched && matched.authKind === "password") {
      cmd
        .sshConnectionResolvePassword(matched.index)
        .then((password) => {
          if (!password) return;
          if (tab.backend === "local") {
            useTabStore.getState().updateTab(tab.id, { sshPassword: password });
          } else {
            const current = useTabStore.getState().tabs.find((t) => t.id === tab.id);
            if (current?.nestedSshTarget && current.nestedSshTarget.savedConnectionIndex === matched.index) {
              useTabStore.getState().updateTab(tab.id, {
                nestedSshTarget: { ...current.nestedSshTarget, password },
              });
            }
          }
        })
        .catch(() => {});
    }
  }

  async function sendInput(data: string) {
    if (!session || !data) return;
    const isSubmit = data === "\r" || data === "\n" || data === "\r\n";
    const lineToParse = isSubmit ? commandBufferRef.current.trim() : "";
    updateCommandBuffer(data);
    try {
      await cmd.terminalWrite(session.sessionId, data);
      setScrollbackOffset(0);
    } catch (e) {
      setError(formatError(e));
      return;
    }
    if (isSubmit && lineToParse) {
      applySshContextFromCommand(lineToParse);
    }
  }

  function getSelectionText(): string {
    const viewport = viewportRef.current;
    const sel = window.getSelection();
    if (!viewport || !sel || sel.rangeCount === 0 || sel.isCollapsed) return "";
    const anchor = sel.anchorNode;
    const focus = sel.focusNode;
    if (!anchor || !focus || !viewport.contains(anchor) || !viewport.contains(focus)) return "";
    return sel.toString();
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    const mod = event.ctrlKey || event.metaKey;
    const selText = getSelectionText();

    if (mod && !event.altKey && event.key.toLowerCase() === "v") {
      event.preventDefault();
      navigator.clipboard.readText().then((text) => {
        if (text) void sendInput(text.replace(/\r?\n/g, "\r"));
      }).catch(() => {});
      return;
    }

    if (mod && !event.altKey && event.key.toLowerCase() === "c" && selText) {
      event.preventDefault();
      navigator.clipboard.writeText(selText).catch(() => {});
      return;
    }

    let payload = "";

    if (event.ctrlKey && !event.altKey && !event.metaKey) {
      if (event.key.length === 1) {
        const upper = event.key.toUpperCase();
        if (upper >= "A" && upper <= "Z") {
          payload = String.fromCharCode(upper.charCodeAt(0) - 64);
        } else if (upper in controlKeyMap) {
          payload = controlKeyMap[upper];
        }
      }
    } else if (event.key === "Enter") {
      payload = "\r";
    } else if (event.key === "Backspace") {
      payload = "\u007f";
    } else if (event.key === "Tab") {
      payload = event.shiftKey ? "\u001b[Z" : "\t";
    } else if (event.key === "Escape") {
      payload = "\u001b";
    } else if (event.key === "ArrowUp") {
      payload = "\u001b[A";
    } else if (event.key === "ArrowDown") {
      payload = "\u001b[B";
    } else if (event.key === "ArrowRight") {
      payload = "\u001b[C";
    } else if (event.key === "ArrowLeft") {
      payload = "\u001b[D";
    } else if (event.key === "Home") {
      payload = "\u001b[H";
    } else if (event.key === "End") {
      payload = "\u001b[F";
    } else if (!event.metaKey && !event.ctrlKey && event.key.length === 1) {
      payload = event.key;
    }

    if (!payload) return;
    event.preventDefault();
    void sendInput(payload);
  }

  function handleWheel(event: React.WheelEvent<HTMLDivElement>) {
    if (!snapshot?.scrollbackLen) return;
    event.preventDefault();
    const step = Math.max(1, Math.round(Math.abs(event.deltaY) / 36));
    setScrollbackOffset((prev) =>
      event.deltaY < 0
        ? Math.min(prev + step, snapshot.scrollbackLen)
        : Math.max(prev - step, 0),
    );
  }

  async function restartTerminal() {
    if (session) {
      await cmd.terminalClose(session.sessionId).catch(() => {});
    }
    setSession(null);
    setSnapshot(null);
    setScrollbackOffset(0);
  }

  async function copySelection() {
    const sel = window.getSelection?.()?.toString() ?? "";
    if (!sel) return;
    try {
      await navigator.clipboard.writeText(sel);
    } catch {
      /* clipboard blocked — silently skip */
    }
  }

  async function pasteClipboard() {
    if (!session) return;
    try {
      const text = await navigator.clipboard.readText();
      if (text) await cmd.terminalWrite(session.sessionId, text);
    } catch {
      /* clipboard blocked */
    }
  }

  function selectAllInTerminal() {
    const screen = viewportRef.current?.querySelector(".terminal-screen");
    if (!screen) return;
    const range = document.createRange();
    range.selectNodeContents(screen);
    const sel = window.getSelection();
    sel?.removeAllRanges();
    sel?.addRange(range);
  }

  async function clearTerminal() {
    if (!session) return;
    // Send form-feed / "clear" sequence (xterm CSI 3 J erases scrollback, \x1b[H\x1b[2J clears screen).
    await cmd.terminalWrite(session.sessionId, "\x1b[H\x1b[2J\x1b[3J").catch(() => {});
  }

  const surfaceLive = snapshot?.alive ?? false;
  const surfaceStatus = surfaceLive ? t("Live") : session ? t("Exited") : t("Booting");

  return (
    <section
      className="terminal-panel"
      style={{ display: isActive ? "flex" : "none" }}
    >
      <div className="terminal-panel__header">
        <div className="terminal-panel__title">
          <SquareTerminal size={15} />
          <span>
            {tab.backend === "ssh"
              ? `${tab.sshUser}@${tab.sshHost}`
              : session?.shell ?? t("Terminal")}
          </span>
        </div>
        <div className="terminal-panel__meta">
          <span className={`meta-pill ${surfaceLive ? "meta-pill--success" : ""}`}>
            {surfaceStatus}
          </span>
          <span className="meta-pill">
            {snapshot
              ? `${snapshot.cols} \u00d7 ${snapshot.rows}`
              : `${terminalSize.cols} \u00d7 ${terminalSize.rows}`}
          </span>
          {scrollbackOffset > 0 ? (
            <button
              className="mini-button"
              onClick={() => setScrollbackOffset(0)}
              type="button"
            >
              {t("Follow Live")}
            </button>
          ) : null}
          <button className="mini-button" onClick={() => void restartTerminal()} type="button">
            {t("Restart")}
          </button>
        </div>
      </div>

      <div
        onKeyDown={handleKeyDown}
        onMouseDown={(e) => e.currentTarget.focus()}
        onWheel={handleWheel}
        onContextMenu={(e) => {
          e.preventDefault();
          setCtxMenu({ x: e.clientX, y: e.clientY });
        }}
        ref={viewportRef}
        className={visualBellActive ? "terminal-viewport terminal-viewport--bell" : "terminal-viewport"}
        style={{ background: termTheme.bg }}
        tabIndex={0}
      >
        <span
          aria-hidden
          className="terminal-measure"
          ref={measureRef}
          style={{ fontFamily: `"${monoFont}", monospace`, fontSize: `${terminalFontSize}px` }}
        >
          MMMMMMMMMM
        </span>

        {error ? (
          <div className="terminal-placeholder terminal-placeholder--error">
            <span>{error}</span>
            {needsPasswordRecovery
              && tab.sshSavedConnectionIndex !== null
              && onEditConnection && (
                <button
                  type="button"
                  className="mini-button"
                  onClick={() => {
                    if (tab.sshSavedConnectionIndex !== null) {
                      onEditConnection(tab.sshSavedConnectionIndex);
                    }
                  }}
                  style={{ marginLeft: "var(--sp-2)" }}
                >
                  <KeyRound size={11} /> {t("Re-enter password")}
                </button>
              )}
          </div>
        ) : snapshot ? (
          <div
            className={rowSeparators ? "terminal-screen terminal-screen--ruled" : "terminal-screen"}
            style={{
              fontFamily: `"${monoFont}", monospace`,
              fontSize: `${terminalFontSize}px`,
              lineHeight: `${Math.ceil(terminalFontSize * 1.45)}px`,
              ["--terminal-row-h" as string]: `${Math.ceil(terminalFontSize * 1.45)}px`,
              background: termTheme.bg,
              color: termTheme.fg,
            }}
          >
            {snapshot.lines.map((line, i) => (
              <div className="terminal-row" key={`line-${i}`} style={{ color: termTheme.fg }}>
                {line.segments.map((seg, j) => {
                  const isCursor = seg.cursor;
                  // Cursor style: 0=block (default), 1=beam, 2=underline
                  const cursorClass = isCursor
                    ? cursorStyle === 1
                      ? "terminal-segment terminal-segment--cursor-beam"
                      : cursorStyle === 2
                        ? "terminal-segment terminal-segment--cursor-underline"
                        : "terminal-segment terminal-segment--cursor"
                    : "terminal-segment";
                  const segBg = isCursor
                    ? undefined
                    : resolveTerminalColor(seg.bg, termTheme.ansi);
                  const segFg = isCursor
                    ? undefined
                    : resolveTerminalColor(seg.fg, termTheme.ansi);
                  return (
                    <span
                      className={cursorClass}
                      key={`seg-${i}-${j}`}
                      style={{
                        backgroundColor: segBg,
                        color: segFg,
                        fontWeight: seg.bold ? 510 : 400,
                        textDecoration: seg.underline ? "underline" : "none",
                        animation: isCursor && cursorBlink ? "cursor-blink 1s step-end infinite" : undefined,
                      }}
                    >
                      {seg.text}
                    </span>
                  );
                })}
              </div>
            ))}
          </div>
        ) : (
          <div className="terminal-placeholder">{t("Launching shell...")}</div>
        )}
      </div>

      {ctxMenu && (() => {
        const hasSelection = (window.getSelection?.()?.toString() ?? "").length > 0;
        const isMac = navigator.platform.includes("Mac");
        const mod = isMac ? "\u2318" : "Ctrl+";
        const items: ContextMenuItem[] = [
          {
            label: t("Copy"),
            shortcut: `${mod}C`,
            disabled: !hasSelection,
            action: () => void copySelection(),
          },
          {
            label: t("Paste"),
            shortcut: `${mod}V`,
            disabled: !session,
            action: () => void pasteClipboard(),
          },
          { divider: true },
          {
            label: t("Select All"),
            shortcut: `${mod}A`,
            action: selectAllInTerminal,
          },
          {
            label: t("Clear terminal"),
            shortcut: `${mod}K`,
            disabled: !session,
            action: () => void clearTerminal(),
          },
          { divider: true },
          {
            label: t("Restart terminal"),
            action: () => void restartTerminal(),
          },
        ];
        return (
          <ContextMenu
            x={ctxMenu.x}
            y={ctxMenu.y}
            items={items}
            onClose={() => setCtxMenu(null)}
          />
        );
      })()}
    </section>
  );
}
