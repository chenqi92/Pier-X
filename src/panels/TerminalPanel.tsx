// ── Terminal Panel ───────────────────────────────────────────────
// Per-tab terminal: event-driven snapshot refresh (with a slow safety
// interval), keyboard I/O, scrollback, and session lifecycle management.

import { KeyRound, SquareTerminal } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import * as cmd from "../lib/commands";
import { controlKeyMap } from "../lib/commands";
import ContextMenu, { type ContextMenuItem } from "../components/ContextMenu";
import TerminalSyntaxOverlay from "../components/TerminalSyntaxOverlay";
import CompletionPopover from "../components/CompletionPopover";
import ManPagePopover from "../components/ManPagePopover";
import {
  terminalCompletions,
  terminalManSynopsis,
  type Completion,
  type ManSynopsis,
} from "../lib/terminalSmart";
import {
  useTerminalHistoryStore,
  suggestFromHistory,
} from "../stores/useTerminalHistoryStore";
import { useI18n } from "../i18n/useI18n";
import { isMissingKeychainError, localizeError } from "../i18n/localizeMessage";
import type { TabState, TerminalSessionInfo, TerminalSnapshot, TerminalSize } from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import { useTabStore } from "../stores/useTabStore";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useStatusStore } from "../stores/useStatusStore";
import { useThemeStore, TERMINAL_THEMES } from "../stores/useThemeStore";
import { parseSshCommand } from "../lib/parseSshCommand";
import { readClipboardText, writeClipboardText } from "../lib/clipboard";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useUiActionsStore } from "../stores/useUiActionsStore";
import { logEvent } from "../lib/logger";

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

/** Payload shape for the backend's `terminal:ssh-state` event. Emitted
 *  whenever the SSH-child watcher sees the set of `ssh` clients in the
 *  PTY descendant tree change. `target === null` means no ssh is
 *  currently running under this terminal — the right panel should go
 *  idle. */
type TerminalSshStatePayload = {
  sessionId: string;
  target: TerminalSshStateTarget | null;
};

type TerminalSshStateTarget = {
  host: string;
  user: string;
  port: number;
  /** `-i <path>` from the argv; empty string when absent. */
  identityPath: string;
};

type Props = {
  tab: TabState;
  isActive: boolean;
  /** Open the saved-connection editor when the keychain has lost the
   *  password for this tab's saved connection. */
  onEditConnection?: (index: number) => void;
};

export default function TerminalPanel({ tab, isActive, onEditConnection }: Props) {
  const { t, locale } = useI18n();
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
  const copyOnSelect = useSettingsStore((s) => s.terminalCopyOnSelect);
  const smartMode = useSettingsStore((s) => s.terminalSmartMode);
  const termThemeIdx = useThemeStore((s) => s.terminalThemeIndex);
  const termTheme = TERMINAL_THEMES[termThemeIdx] ?? TERMINAL_THEMES[0];
  const [session, setSession] = useState<TerminalSessionInfo | null>(null);
  const [snapshot, setSnapshot] = useState<TerminalSnapshot | null>(null);
  const [error, setError] = useState("");
  const [needsPasswordRecovery, setNeedsPasswordRecovery] = useState(false);
  const requestEditConnection = useUiActionsStore((s) => s.requestEditConnection);
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

  // Smart-mode mirror buffer — fish-style autosuggest, syntax-highlight
  // and Tab popover (M2+) all need a frontend-side view of the line the
  // user is currently typing. M1 just maintains the buffer; later
  // milestones layer UI overlays on top. Tracking is gated on:
  //   * smartMode setting being on (per PRODUCT-SPEC §4.2.1, opt-in)
  //   * the emulator having seen an OSC 133;B (`promptEnd != null`)
  //   * `awaitingInput` (between B and C)
  //   * not in the alt screen (vim/htop) and not mid-bracketed-paste
  // When any condition flips off, the buffer is cleared so a re-arm
  // starts from empty. Reads/writes from event handlers go through
  // `smartActiveRef` so the latest value is visible without a closure
  // re-bind; render-time consumers use the derived `smartActive`
  // boolean below.
  const smartLineBufferRef = useRef("");
  const smartActiveRef = useRef(false);

  // Render-driven mirror of `smartLineBufferRef`. The ref keeps event
  // handlers fast and stale-closure-free; this state forces React to
  // re-render the syntax overlay on every keystroke. We deliberately
  // accept the extra render pass — the overlay is tiny (a few hundred
  // chars max) and ANSI/lexer cost is negligible compared with the
  // existing per-snapshot render of the full grid.
  const [smartLineBufferText, setSmartLineBufferText] = useState("");

  // Cell metrics (charWidth, rowHeight) derived from the live font
  // measurement in the resize-observer effect below. Used by the
  // syntax overlay to position itself onto the right grid cell.
  const [cellMetrics, setCellMetrics] = useState<{
    charWidth: number;
    rowHeight: number;
  }>({ charWidth: 7.8, rowHeight: 19 });

  // M4: Tab-completion popover state. `open` flips on the Tab
  // keypress and off when the user accepts / dismisses / leaves the
  // smart-active gate. `items` is the full set returned from the
  // backend; `filtered` is what the popover currently renders after
  // local prefix filtering as the user keeps typing. The DOM anchor
  // is kept in a ref because the upstream Popover component takes
  // an HTMLElement.
  const cursorAnchorRef = useRef<HTMLDivElement | null>(null);
  type CompletionState = {
    open: boolean;
    items: Completion[];
    filtered: Completion[];
    selectedIndex: number;
  };
  const [completion, setCompletion] = useState<CompletionState>({
    open: false,
    items: [],
    filtered: [],
    selectedIndex: 0,
  });

  // M6: man-page popover. Triggered by Ctrl+Shift+M in smart mode.
  // The popover shows SYNOPSIS / DESCRIPTION / OPTIONS for the
  // command name at the cursor (or the first word when no clear
  // cursor word is available). Result + loading + error live in a
  // single state object so the component re-renders coherently.
  type ManState = {
    open: boolean;
    command: string;
    data: ManSynopsis | null;
    loading: boolean;
    errorMessage: string | null;
  };
  const [manState, setManState] = useState<ManState>({
    open: false,
    command: "",
    data: null,
    loading: false,
    errorMessage: null,
  });

  // M5: autosuggestion. The history ring is global (one across the
  // whole app) so a command run in tab A is suggestible from tab B.
  // We track the previous `awaitingInput` value to detect the
  // edge-trigger when the shell emits OSC 133;C — that's our signal
  // that the line was submitted and should land in the ring.
  const historyRing = useTerminalHistoryStore((s) => s.ring);
  const pushHistory = useTerminalHistoryStore((s) => s.push);
  const hydrateHistory = useTerminalHistoryStore((s) => s.hydrate);
  const historyPersist = useSettingsStore((s) => s.terminalHistoryPersist);
  // The suggestion suffix itself is computed alongside the other
  // smart-active-derived state below, after `smartActive` exists.
  // The ref is filled there too; declared up here so event handlers
  // can read it without a forward-ref dance.
  const suggestionSuffixRef = useRef("");

  // Prompt-anchored capture window. Armed when the backend PTY
  // reader sees the canonical OpenSSH `<user>@<host>'s password:` /
  // `Enter passphrase for key` shape in the output stream and fires
  // `terminal:ssh-password-prompt`. The very next Enter-terminated
  // line the user types (with echo disabled by ssh, so we only see
  // it because pier-x forwards raw keystrokes to the PTY) is mirrored
  // into `tab.sshPassword` for the right-side russh session. After
  // one capture the window disarms; a second wrong attempt re-fires
  // the prompt event from the backend, which re-arms us cleanly. The
  // 60s deadline is a safety net so a stale arm doesn't grab an
  // unrelated line if the user walked away.
  //
  // Fully deterministic compared with the previous keystroke-shape
  // heuristic: `sudo` prompts, local `passwd`, and post-login
  // single-word commands (`ls`, `pwd`) can no longer be mistaken for
  // the ssh password because they don't emit the specific OpenSSH
  // prompt pattern the backend is matching on.
  const pendingPasswordCaptureRef = useRef<
    { deadline: number; kind: "password" | "passphrase" } | null
  >(null);

  // Sync session ID to tab store
  useEffect(() => {
    if (session && tab.terminalSessionId !== session.sessionId) {
      updateTab(tab.id, { terminalSessionId: session.sessionId });
    }
  }, [session?.sessionId]);

  // ── SSH session pre-warm ────────────────────────────────────────
  // The real ssh the user launched (local `ssh user@host`, or nested
  // ssh inside an ssh tab) has its own TCP connection that lives in a
  // subprocess we can't reuse. To keep the "all panels reuse one
  // session" promise, open a parallel russh connection in the
  // background the moment we have enough credentials, and seed the
  // shared `sftp_sessions` cache under the same key the panels look
  // up. By the time the user clicks Docker / Monitor / Log / DB, the
  // cache is warm and their first call skips the handshake.
  //
  // Fires only when the credential shape actually changes — re-
  // rendering the tab for an unrelated reason (resize, scroll) does
  // not retrigger the prewarm.
  const prewarmFingerprintRef = useRef<string>("");
  useEffect(() => {
    const target = effectiveSshTarget(tab);
    if (!target) {
      prewarmFingerprintRef.current = "";
      return;
    }
    // For real SSH-backend tabs without a nested overlay, the terminal
    // creation path already seeded the cache via
    // `create_ssh_terminal_from_config`. Skip so we don't open a
    // redundant second russh connection on top of it.
    if (tab.backend === "ssh" && !tab.nestedSshTarget) return;

    // We need at least one credential path with a chance of succeeding.
    // `auto` and `agent` self-authenticate via the SSH agent / default
    // identity files, so they're always worth trying; `key` needs a
    // path; `password` needs the captured / keychain-resolved secret;
    // a saved-index alone is enough because the on-disk config carries
    // its own auth. Skip until one of these holds — otherwise the
    // prewarm would just fail and waste a handshake.
    const hasCredential =
      target.savedConnectionIndex !== null
      || target.authMode === "agent"
      || target.authMode === "auto"
      || (target.authMode === "key" && target.keyPath.length > 0)
      || (target.authMode === "password" && target.password.length > 0);
    if (!hasCredential) return;

    const fingerprint = [
      target.host,
      target.port,
      target.user,
      target.authMode,
      target.keyPath,
      target.savedConnectionIndex ?? "",
      target.password.length > 0 ? "pw" : "no-pw",
    ].join("|");
    if (fingerprint === prewarmFingerprintRef.current) return;
    prewarmFingerprintRef.current = fingerprint;

    cmd
      .sshSessionPrewarm({
        host: target.host,
        port: target.port,
        user: target.user,
        authMode: target.authMode,
        password: target.password,
        keyPath: target.keyPath,
        savedConnectionIndex: target.savedConnectionIndex,
      })
      .catch(() => {
        // Backend already swallows errors; this catch guards against
        // invoke-layer failures (dev reload, missing command) — not
        // worth surfacing to the user for an optimization path.
      });
  }, [
    tab.backend,
    tab.nestedSshTarget?.host,
    tab.nestedSshTarget?.port,
    tab.nestedSshTarget?.user,
    tab.nestedSshTarget?.authMode,
    tab.nestedSshTarget?.keyPath,
    tab.nestedSshTarget?.savedConnectionIndex,
    (tab.nestedSshTarget?.password.length ?? 0) > 0,
    tab.sshHost,
    tab.sshPort,
    tab.sshUser,
    tab.sshAuthMode,
    tab.sshKeyPath,
    tab.sshSavedConnectionIndex,
    (tab.sshPassword?.length ?? 0) > 0,
  ]);

  // ── Measure terminal grid dimensions ────────────────────────

  useEffect(() => {
    const viewport = viewportRef.current;
    const measure = measureRef.current;
    if (!viewport || !measure) return;

    const recalculate = () => {
      const measureBox = measure.getBoundingClientRect();
      const charWidth = measureBox.width / 10 || 7.8;
      // Match the px row height used by the renderer (--terminal-row-h is set
      // via Math.ceil(fontSize * 1.45) inline). Using the fractional measured
      // height here would let `rows × ceil_row_h` exceed the viewport, clipping
      // the bottom of the cursor row under `overflow: hidden`.
      const rowH = Math.ceil(terminalFontSize * 1.45);
      const cols = Math.max(48, Math.min(220, Math.floor((viewport.clientWidth - 24) / charWidth)));
      const rows = Math.max(14, Math.min(72, Math.floor((viewport.clientHeight - 20) / rowH)));
      setTerminalSize((prev) =>
        prev.cols === cols && prev.rows === rows ? prev : { cols, rows },
      );
      // Smart-mode overlay needs the same metrics to align coloured
      // spans with the underlying terminal cells. Stored as state so
      // the overlay re-renders when font size or container width
      // changes; the threshold avoids needless re-renders on
      // sub-pixel jitter from the ResizeObserver.
      setCellMetrics((prev) =>
        Math.abs(prev.charWidth - charWidth) < 0.05 && prev.rowHeight === rowH
          ? prev
          : { charWidth, rowHeight: rowH },
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
          // Smart mode injects an OSC 133-emitting init script into the
          // local shell. We only enable it for local PTYs — SSH sessions
          // can't currently push the rcfile to the remote host, and the
          // emulator's prompt-zone tracking would silently no-op. Per
          // PRODUCT-SPEC §4.2.1 this is opt-in via Settings.
          next = await cmd.terminalCreate(
            terminalSize.cols,
            terminalSize.rows,
            undefined,
            smartMode,
          );
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
    // `tab.sshPassword` is in the deps so a tab whose first
    // create() rejected with "saved password missing in keychain"
    // automatically retries once the user re-enters the password
    // via the recovery dialog (App.tsx propagates the new password
    // into matching tabs and nulls `terminalSessionId`, which we
    // mirror into local `session` state below).
  }, [session, terminalSize.cols, terminalSize.rows, tab.backend, tab.sshHost, tab.sshPassword]);

  // When App.tsx clears `tab.terminalSessionId` (e.g. as part of
  // the post-recovery propagation), drop the local session state
  // so the create-effect above re-runs against the fresh
  // credentials. Skipped for the steady-state case where the IDs
  // already match — that just means the session-id sync ran once
  // after our own creation and there's nothing to do.
  useEffect(() => {
    if (tab.terminalSessionId !== null) return;
    if (!session) return;
    setSession(null);
    setSnapshot(null);
    setError("");
    setNeedsPasswordRecovery(false);
  }, [tab.terminalSessionId, session]);

  // Pull keyboard focus onto the terminal viewport the moment the
  // session is ready, and again whenever the tab becomes visible.
  // Without this, creating a fresh local tab leaves focus on the
  // previous UI element (or nothing at all) — users have to click
  // the terminal before typing works, which reads as "the app ate
  // my keystrokes". We keep the existing onMouseDown handler for
  // the recovery path, but proactive focus on session-ready is the
  // default interaction a shell should offer.
  useEffect(() => {
    if (!session) return;
    if (!isActive) return;
    const viewport = viewportRef.current;
    if (!viewport) return;
    // Defer to the next paint: the viewport is `display: none` when
    // the tab isn't active and focus() on a hidden element no-ops.
    // requestAnimationFrame ensures the layout commit from
    // `display: flex` has happened before we call focus().
    const raf = window.requestAnimationFrame(() => {
      if (document.activeElement === viewport) return;
      viewport.focus({ preventScroll: true });
    });
    return () => window.cancelAnimationFrame(raf);
  }, [session, isActive]);

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

  // Copy-on-select (iTerm-style). Listen for mouseup on the viewport
  // and, if the resulting selection lives inside it and is non-empty,
  // ship it to the clipboard. Only active when the setting is on so
  // existing users keep the explicit ⌘C behavior.
  useEffect(() => {
    if (!copyOnSelect) return;
    const viewport = viewportRef.current;
    if (!viewport) return;
    const handler = () => {
      const sel = window.getSelection();
      if (!sel || sel.rangeCount === 0 || sel.isCollapsed) return;
      const anchor = sel.anchorNode;
      const focus = sel.focusNode;
      if (!anchor || !focus) return;
      if (!viewport.contains(anchor) || !viewport.contains(focus)) return;
      const text = sel.toString();
      if (!text) return;
      void writeClipboardText(text);
    };
    viewport.addEventListener("mouseup", handler);
    return () => viewport.removeEventListener("mouseup", handler);
  }, [copyOnSelect, session]);

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
    let safety: number | null = null;

    // The safety timer fires only after 1500ms of quiet — any
    // event-driven refresh re-arms it, so we no longer get the
    // double-fetch that happened when an event arrived ~100ms before
    // a fixed-interval tick.
    const armSafety = () => {
      if (safety !== null) window.clearTimeout(safety);
      safety = window.setTimeout(() => {
        safety = null;
        refresh();
      }, 1500);
    };

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
          // Direct setSnapshot — terminal feedback MUST paint on the
          // next frame. Wrapping in startTransition let React defer
          // the update when anything else was in-flight, which
          // compounded with the old backend throttle to make casual
          // typing feel seconds-delayed.
          setSnapshot(next);
          setError("");
        })
        .catch((e) => {
          if (!disposed) setError(formatError(e));
        })
        .finally(() => {
          inflight = false;
          if (dirty && !disposed) {
            refresh();
          } else if (!disposed) {
            armSafety();
          }
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

    // Subscribe to the SSH-child state event. The backend watcher
    // polls this terminal's PTY descendant tree once a second and
    // fires whenever the set of live `ssh` clients changes — nested
    // ssh in, ssh out, DNS failure reaping the child, all of it.
    // We're the authoritative source for tab.sshHost / nestedSshTarget
    // on local-backend tabs; input parsing only arms password capture.
    let unlistenSshState: UnlistenFn | undefined;
    void listen<TerminalSshStatePayload>("terminal:ssh-state", (event) => {
      if (disposed) return;
      if (event.payload.sessionId !== session.sessionId) return;
      applySshStateFromWatcher(event.payload.target);
    }).then((u) => {
      if (disposed) u();
      else unlistenSshState = u;
    });

    // Subscribe to the one-shot password-prompt event. The PTY
    // reader fires this when it sees the canonical OpenSSH prompt
    // shape in the output bytes — which is the only moment at which
    // "the next typed line is the password" is actually true. Arming
    // from keystroke parsing was fundamentally heuristic (missed
    // history-edited / pasted `ssh` lines, and couldn't distinguish
    // a post-login single-word command from a second password
    // attempt); arming from the prompt itself is precise.
    let unlistenSshPrompt: UnlistenFn | undefined;
    void listen<{ sessionId: string }>("terminal:ssh-password-prompt", (event) => {
      if (disposed) return;
      if (event.payload.sessionId !== session.sessionId) return;
      pendingPasswordCaptureRef.current = {
        deadline: Date.now() + 60_000,
        kind: "password",
      };
      logEvent("INFO", "ssh.capture", `tab=${tab.id} armed capture on OpenSSH password prompt`);
    }).then((u) => {
      if (disposed) u();
      else unlistenSshPrompt = u;
    });

    // Separate event for `Enter passphrase for key '<path>':` — the
    // captured value is a key passphrase, not a server password, so
    // we route it to a different backend slot. Crossing the two
    // (passing a passphrase as a server password, or vice versa)
    // costs the user a wrong auth attempt and surfaces as a
    // confusing "auth rejected" error on the right side.
    let unlistenSshPassphrasePrompt: UnlistenFn | undefined;
    void listen<{ sessionId: string }>("terminal:ssh-passphrase-prompt", (event) => {
      if (disposed) return;
      if (event.payload.sessionId !== session.sessionId) return;
      pendingPasswordCaptureRef.current = {
        deadline: Date.now() + 60_000,
        kind: "passphrase",
      };
      logEvent(
        "INFO",
        "ssh.capture",
        `tab=${tab.id} armed capture on OpenSSH key passphrase prompt`,
      );
    }).then((u) => {
      if (disposed) u();
      else unlistenSshPassphrasePrompt = u;
    });

    return () => {
      disposed = true;
      if (safety !== null) window.clearTimeout(safety);
      if (unlisten) unlisten();
      if (unlistenSshState) unlistenSshState();
      if (unlistenSshPrompt) unlistenSshPrompt();
      if (unlistenSshPassphrasePrompt) unlistenSshPassphrasePrompt();
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

  // ── Smart-mode mirror state ────────────────────────────────

  // We deliberately don't gate on OSC 133 sentinels here: remote
  // shells reached over `ssh` don't emit them, and the byte-stream
  // mirror buffer already self-resets on CR/LF, so smart mode stays
  // correct without prompt-end signals.
  //
  // We also no longer gate on `bracketedPaste`. That field tracks
  // DECSET 2004 — i.e. whether the shell has *enabled* bracketed-
  // paste mode — which bash/zsh do for the entire interactive prompt
  // lifetime. So gating on it as if it meant "paste in flight"
  // silently disabled smart mode on every normal prompt. Detecting
  // an actual paste needs the `\e[200~`/`\e[201~` start/end markers,
  // which the emulator doesn't surface separately yet — TODO when
  // we want to mute the lexer for huge pastes.
  //
  // Alt-screen is still a real signal — vim/htop genuinely take over
  // the screen and the smart UI must hide. Snapshot may be null on
  // first mount; that's fine, we activate eagerly.
  const smartActive = smartMode && snapshot?.altScreen !== true;

  useEffect(() => {
    smartActiveRef.current = smartActive;
    if (!smartActive) {
      // Disabling the tracker drops whatever was buffered so we
      // never resurrect stale typing on the next prompt — the
      // buffer is re-armed empty by the prompt-end effect below
      // when conditions return to active.
      smartLineBufferRef.current = "";
      setSmartLineBufferText("");
    }
  }, [smartActive]);

  // Reset the mirror buffer at every fresh prompt-end. Encoding
  // the position into a string keeps the dep array primitive-only
  // so React can shallow-compare.
  const promptEndKey = snapshot?.promptEnd
    ? `${snapshot.promptEnd[0]},${snapshot.promptEnd[1]}`
    : "";
  useEffect(() => {
    if (!smartActive) return;
    smartLineBufferRef.current = "";
    setSmartLineBufferText("");
  }, [smartActive, promptEndKey]);

  // M5: compute the autosuggestion suffix on every render where
  // smart mode is active. Cheap — `suggestFromHistory` is an O(n)
  // walk of at most 500 strings.
  const suggestionSuffix = smartActive
    ? suggestFromHistory(historyRing, smartLineBufferText)
    : "";
  suggestionSuffixRef.current = suggestionSuffix;

  // History persistence: hydrate the ring with the on-disk file
  // for this session's shell on first mount. The store dedups so
  // calling this for every tab open is safe — only the first one
  // per shell actually issues an invoke.
  useEffect(() => {
    if (!session?.shell) return;
    if (!historyPersist) return;
    void hydrateHistory(session.shell, historyPersist);
  }, [session?.shell, historyPersist, hydrateHistory]);

  // M4: refilter the open completion popover on every keystroke so
  // the candidate list narrows as the user types more characters.
  // We re-derive the prefix from the live mirror buffer ref rather
  // than the state mirror to keep filtering in lock-step with what
  // the user actually typed (the state lags by one render).
  useEffect(() => {
    setCompletion((s) => {
      if (!s.open) return s;
      const line = smartLineBufferRef.current;
      const cursor = line.length;
      const wordStart = findWordStart(line, cursor);
      const prefix = line.slice(wordStart, cursor);
      const filtered = s.items.filter((it) => it.value.startsWith(prefix));
      const selectedIndex =
        filtered.length === 0
          ? 0
          : Math.min(s.selectedIndex, filtered.length - 1);
      return { ...s, filtered, selectedIndex };
    });
  }, [smartLineBufferText]);

  // Close the popover whenever the smart-mode gate flips off — alt
  // screen, bracketed paste, scroll into history, or smart toggle
  // off all dismiss the popover so the user is never stranded with
  // a stale list over a TUI.
  useEffect(() => {
    if (!smartActive) {
      closeCompletion();
      closeMan();
    }
  }, [smartActive]);

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
   * Walk through the bytes about to be sent to the PTY and keep the
   * "currently typing" buffer in sync. Returns any complete lines
   * that just got submitted (Enter pressed, or pasted text with an
   * embedded newline) so the caller can offer them to the SSH
   * command parser.
   *
   * Models just enough shell line-editing semantics to cover the
   * common cases: printable bytes append, backspace removes, Enter
   * completes a line, Ctrl+C / Ctrl+U / Esc clear, and CSI/SS3
   * escape sequences (arrow keys, function keys) are recognized so
   * they reset the buffer cleanly instead of polluting it with the
   * raw `[A` / `OB` payload.
   *
   * Heuristic by design — if the user navigates history with arrows
   * or edits mid-line we may miss an `ssh` command, but we won't
   * misattribute one. False negatives the user can retry; false
   * positives would route the right panel to the wrong host.
   */
  function captureCompletedCommands(data: string): string[] {
    const CR = 13;
    const LF = 10;
    const ESC = 27;
    const BS = 8;
    const DEL = 127;
    const ETX = 3;   // Ctrl+C
    const NAK = 21;  // Ctrl+U
    const completed: string[] = [];
    for (let i = 0; i < data.length; i++) {
      const code = data.charCodeAt(i);
      if (code === CR || code === LF) {
        completed.push(commandBufferRef.current);
        commandBufferRef.current = "";
        if (code === CR && data.charCodeAt(i + 1) === LF) i += 1;
        continue;
      }
      if (code === ESC) {
        // Escape sequence: reset the buffer and consume the rest.
        // Two shapes are emitted by handleKeyDown today:
        //   CSI: ESC [ <params> <final letter A-Z, a-z, ~>
        //   SS3: ESC O <letter>
        commandBufferRef.current = "";
        const next = data[i + 1];
        if (next === "[") {
          i += 1;
          while (i + 1 < data.length) {
            i += 1;
            if (/[A-Za-z~]/.test(data[i])) break;
          }
        } else if (next === "O") {
          i += 2;
        }
        continue;
      }
      if (code === DEL || code === BS) {
        commandBufferRef.current = commandBufferRef.current.slice(0, -1);
        continue;
      }
      if (code === ETX || code === NAK) {
        commandBufferRef.current = "";
        continue;
      }
      if (code < 0x20 || code === 0x7f) {
        // Other unmodelled control byte — reset to avoid carrying
        // stale state into the next Enter.
        commandBufferRef.current = "";
        continue;
      }
      commandBufferRef.current += data[i];
    }
    return completed;
  }

  /**
   * Smart-mode line mirror — applies the same line-edit emulation
   * as `captureCompletedCommands` to `smartLineBufferRef` so the
   * frontend keeps a precise view of the line the user is typing
   * for M2+ overlays (autosuggest, syntax-highlight, Tab popover).
   *
   * Only invoked when `smartActiveRef.current` is true at the
   * moment of the keystroke; the snapshot-driven effect above
   * resets the buffer on every fresh prompt-end and clears it
   * when conditions flip off (alt screen, bracketed paste,
   * command running, smart toggle disabled).
   */
  function updateSmartLineBuffer(data: string) {
    if (!smartActiveRef.current) return;
    const CR = 13;
    const LF = 10;
    const ESC = 27;
    const BS = 8;
    const DEL = 127;
    const ETX = 3;   // Ctrl+C
    const NAK = 21;  // Ctrl+U
    let buf = smartLineBufferRef.current;
    for (let i = 0; i < data.length; i++) {
      const code = data.charCodeAt(i);
      if (code === CR || code === LF) {
        // M5 history capture: push the just-submitted line into the
        // ring before clearing. We used to capture this on the
        // OSC 133;C edge (`awaiting_input` true → false), but remote
        // shells reached over `ssh` don't emit OSC 133, so that
        // path missed every command typed inside SSH. The byte
        // stream sees Enter regardless of where the shell lives.
        const submitted = buf.trim();
        if (submitted) {
          pushHistory(submitted, {
            shell: session?.shell,
            persist: historyPersist,
          });
        }
        buf = "";
        if (code === CR && data.charCodeAt(i + 1) === LF) i += 1;
        continue;
      }
      if (code === ESC) {
        // M1 doesn't model arrow-key navigation inside the mirror —
        // any escape sequence resets, matching the SSH-capture
        // buffer's conservative stance. M4+ will need richer
        // line-editor semantics to cover Ctrl+W, Ctrl+A, etc.
        buf = "";
        const next = data[i + 1];
        if (next === "[") {
          i += 1;
          while (i + 1 < data.length) {
            i += 1;
            if (/[A-Za-z~]/.test(data[i])) break;
          }
        } else if (next === "O") {
          i += 2;
        }
        continue;
      }
      if (code === DEL || code === BS) {
        buf = buf.slice(0, -1);
        continue;
      }
      if (code === ETX || code === NAK) {
        buf = "";
        continue;
      }
      if (code < 0x20 || code === 0x7f) {
        buf = "";
        continue;
      }
      buf += data[i];
    }
    if (buf !== smartLineBufferRef.current) {
      smartLineBufferRef.current = buf;
      // Push the new value into render state so the syntax overlay
      // re-tokenises with the latest text. React batches sequential
      // setState calls inside event handlers, so a paste of N chars
      // still produces only one re-render.
      setSmartLineBufferText(buf);
    }
  }


  /**
   * If the previous Enter was an `ssh user@host` invocation, take
   * the line the user just submitted and treat it as the password
   * they typed at the ssh password prompt. Mirroring that into
   * `tab.sshPassword` lets the right-side russh session
   * authenticate against the same target without making the user
   * re-enter the password in our own dialog.
   *
   * Best-effort and conservative — if we'd be writing into a slot
   * that's already populated (saved-keychain resolve raced ahead),
   * if the line doesn't look password-shaped (whitespace, way too
   * long), or if the deadline passed, we just clear the
   * single-shot flag and move on. A wrong capture only costs the
   * right-side panels one failed authentication, which is no worse
   * than the previous "saved password missing" surface.
   */
  function maybeCapturePasswordFromLine(line: string): void {
    const pending = pendingPasswordCaptureRef.current;
    if (!pending) {
      return;
    }
    if (Date.now() > pending.deadline) {
      logEvent("DEBUG", "ssh.capture", `tab=${tab.id} capture window expired`);
      pendingPasswordCaptureRef.current = null;
      return;
    }

    // One-shot: disarm immediately. If the remote rejects the
    // password, the PTY reader will see another OpenSSH prompt and
    // re-fire `terminal:ssh-password-prompt`, which re-arms us.
    const captureKind = pending.kind;
    pendingPasswordCaptureRef.current = null;

    const trimmed = line.trim();
    // Empty Enter at the prompt means the user submitted nothing —
    // ssh re-prompts, the backend fires the event again, and we'll
    // arm ourselves fresh.
    if (!trimmed) return;
    // Pathologically long values are almost certainly not a password;
    // drop silently.
    if (trimmed.length > 256) return;

    const current = useTabStore.getState().tabs.find((t) => t.id === tab.id);
    if (!current) return;

    // Resolve the target this capture belongs to. Local-backend tabs
    // use the primary ssh* fields; an SSH-backend tab nesting another
    // ssh uses `nestedSshTarget`. Either way, `host/port/user` is what
    // we key the process-level credential cache on.
    const targetHost =
      tab.backend === "local"
        ? current.sshHost
        : current.nestedSshTarget?.host ?? "";
    const targetPort =
      tab.backend === "local"
        ? current.sshPort
        : current.nestedSshTarget?.port ?? 22;
    const targetUser =
      tab.backend === "local"
        ? current.sshUser
        : current.nestedSshTarget?.user ?? "";

    if (captureKind === "passphrase") {
      // Key passphrase — does NOT belong in `tab.sshPassword`. We
      // sync only to the process-level credential cache; the right-
      // side russh AutoChain will read it from there when loading
      // the explicit / default key files.
      logEvent(
        "INFO",
        "ssh.capture",
        `tab=${tab.id} captured key passphrase (len=${trimmed.length}) for ${targetUser}@${targetHost}:${targetPort}`,
      );
      if (targetHost && targetUser) {
        cmd
          .sshCredCachePutPassphrase({
            host: targetHost,
            port: targetPort,
            user: targetUser,
            passphrase: trimmed,
          })
          .catch((err) => {
            logEvent(
              "WARN",
              "ssh.capture",
              `tab=${tab.id} cred-cache passphrase put failed: ${err}`,
            );
          });
      }
      return;
    }

    // Server password path — keep the existing tab-state mirror so
    // panels that already read `tab.sshPassword` keep working,
    // AND sync to the process-level cache so multi-tab / new-tab
    // reuse works without re-prompting.
    if (tab.backend === "local") {
      if (current.sshPassword === trimmed) return;
      logEvent(
        "INFO",
        "ssh.capture",
        `tab=${tab.id} captured password (len=${trimmed.length}, overwrote=${current.sshPassword ? "yes" : "no"}) for ${current.sshUser}@${current.sshHost}:${current.sshPort}`,
      );
      updateTab(tab.id, { sshPassword: trimmed });
    } else if (current.nestedSshTarget) {
      if (current.nestedSshTarget.password === trimmed) return;
      logEvent(
        "INFO",
        "ssh.capture",
        `tab=${tab.id} captured nested password (overwrote=${current.nestedSshTarget.password ? "yes" : "no"}) for ${current.nestedSshTarget.user}@${current.nestedSshTarget.host}:${current.nestedSshTarget.port}`,
      );
      updateTab(tab.id, {
        nestedSshTarget: { ...current.nestedSshTarget, password: trimmed },
      });
    }

    if (targetHost && targetUser) {
      cmd
        .sshCredCachePutPassword({
          host: targetHost,
          port: targetPort,
          user: targetUser,
          password: trimmed,
        })
        .catch((err) => {
          logEvent(
            "WARN",
            "ssh.capture",
            `tab=${tab.id} cred-cache password put failed: ${err}`,
          );
        });
    }
  }

  /**
   * Apply an SSH state update pushed from the backend watcher.
   *
   * This is the authoritative path for local-backend tabs: the
   * backend looks at the PTY's child process tree, finds any live
   * `ssh` client, extracts its argv, and pushes the target here. If
   * the user typed a typo that failed, the ssh process exits within
   * a second and we receive `target: null` — the right panel goes
   * idle instead of latching onto the dead target. If they retry
   * with the correct host (whether freshly typed, pasted, or
   * edited via shell history), the new ssh process is picked up
   * automatically. Nested ssh inside a still-live session surfaces
   * as the innermost target.
   *
   * SSH-backend tabs (terminal_create_ssh / _saved) never spawn a
   * local child and the watcher is disabled for them — handling
   * them here would be a no-op, so we skip. Nested ssh on those
   * tabs is still driven by the input parser for now.
   */
  function applySshStateFromWatcher(target: TerminalSshStateTarget | null): void {
    if (tab.backend !== "local") return;
    const current = useTabStore.getState().tabs.find((t) => t.id === tab.id);
    if (!current) return;

    if (!target) {
      // No ssh running under this terminal — clear any SSH context
      // so the right panel drops back to "local" / no connection.
      // We only touch fields when they're currently populated so we
      // don't spam zustand with no-op updates while idle.
      if (
        current.sshHost
        || current.sshUser
        || current.sshPassword
        || current.sshSavedConnectionIndex !== null
        || current.nestedSshTarget !== null
      ) {
        logEvent(
          "INFO",
          "ssh.watcher",
          `tab=${tab.id} ssh child exited; clearing cached ${current.sshUser}@${current.sshHost}:${current.sshPort} (had password=${current.sshPassword ? "yes" : "no"})`,
        );
        updateTab(tab.id, {
          sshHost: "",
          sshPort: 22,
          sshUser: "",
          sshAuthMode: "password",
          sshKeyPath: "",
          sshSavedConnectionIndex: null,
          sshPassword: "",
          nestedSshTarget: null,
        });
      }
      return;
    }

    const conns = useConnectionStore.getState().connections;
    const port = target.port > 0 ? target.port : 22;
    const hostLc = target.host.trim().toLowerCase();
    const userLc = target.user.trim().toLowerCase();
    const sameHostUser = (c: { host: string; user: string }) =>
      c.host.trim().toLowerCase() === hostLc
      && (userLc === "" || c.user.trim().toLowerCase() === userLc);
    const matched =
      conns.find((c) => sameHostUser(c) && (c.port || 22) === port)
      ?? conns.find((c) => sameHostUser(c))
      ?? conns.find((c) => c.host.trim().toLowerCase() === hostLc);

    // Auth-mode inference order:
    //   1. A saved connection match wins — the user already decided
    //      which mode this host uses.
    //   2. Explicit `-i <keyfile>` on the ssh argv → `key` mode
    //      against that exact path.
    //   3. Everything else (including plain `ssh user@host` that
    //      authenticated via SSH agent or a default `~/.ssh/id_*`
    //      file) → `auto`. The backend chains agent + conventional
    //      default identity files so a passwordless key login on the
    //      terminal side lets the right-side russh session reach the
    //      same host without us having a credential to carry. The
    //      old default here was `password`, which guaranteed the
    //      monitor probe would fail with "SSH auth rejected" the
    //      moment the user used a public key.
    const authMode: "password" | "agent" | "key" | "auto" =
      matched?.authKind ?? (target.identityPath ? "key" : "auto");
    const keyPath = target.identityPath || matched?.keyPath || "";
    const savedConnectionIndex = matched ? matched.index : null;

    // Preserve an in-flight password (captured from the ssh prompt
    // or resolved from the keychain) across flaps of the watcher,
    // but wipe it when the actual target changed — a stale wrong
    // password would only cause the right-side russh session to
    // fail loudly.
    const sameTarget =
      savedConnectionIndex === current.sshSavedConnectionIndex
      && current.sshHost.trim().toLowerCase() === hostLc
      && current.sshUser.trim().toLowerCase() === target.user.toLowerCase()
      && current.sshPort === port;

    logEvent(
      "INFO",
      "ssh.watcher",
      `tab=${tab.id} ssh child detected: ${target.user}@${target.host}:${port} authMode=${authMode} savedIdx=${savedConnectionIndex ?? "-"} sameTarget=${sameTarget} passwordRetained=${sameTarget && !!current.sshPassword}`,
    );
    updateTab(tab.id, {
      sshHost: target.host,
      sshPort: port,
      sshUser: target.user,
      sshAuthMode: authMode,
      sshKeyPath: keyPath,
      sshSavedConnectionIndex: savedConnectionIndex,
      sshPassword: sameTarget ? current.sshPassword : "",
      nestedSshTarget: null,
      rightTool: "monitor",
    });

    // Saved password match — prime the password from the keychain
    // so the first probe doesn't surface a "saved password missing"
    // error just to recover immediately.
    if (matched && matched.authKind === "password") {
      cmd
        .sshConnectionResolvePassword(matched.index)
        .then((password) => {
          if (!password) return;
          const latest = useTabStore.getState().tabs.find((t) => t.id === tab.id);
          if (!latest) return;
          if (
            latest.sshSavedConnectionIndex === matched.index
            && !latest.sshPassword
          ) {
            useTabStore.getState().updateTab(tab.id, { sshPassword: password });
          }
        })
        .catch(() => {});
    }
  }

  /**
   * Inspect a freshly-submitted shell line for credentials-relevant
   * side effects:
   *
   * 1. If it's an `ssh user@host` invocation, arm the one-shot
   *    password-capture window so the next line the user types
   *    (ssh's silent password prompt response) lands in
   *    `tab.sshPassword`. The host/user/port themselves are NOT
   *    written to tab state from here — the backend SSH watcher
   *    ({@link TERMINAL_SSH_STATE_EVENT}) is the authoritative
   *    source for "what target is the terminal actually connected
   *    to right now". Input parsing can't see history-edited or
   *    copy-pasted commands reliably; the process watcher can.
   *
   * 2. If the line is NOT an ssh invocation and we have a pending
   *    password-capture armed, it probably is the password — mirror
   *    it into tab state so the right-side russh session can
   *    authenticate against the same target without a second prompt.
   *
   * For SSH-backend tabs (nested ssh), the watcher cannot see inside
   * a remote shell, so we still fall back to input parsing to set
   * `nestedSshTarget`. Ideal long-term fix is remote `ps -ef`
   * polling over the existing session; input parsing remains the
   * stop-gap there.
   */
  function applySshContextFromCommand(line: string): void {
    const parsed = parseSshCommand(line);
    if (!parsed) {
      maybeCapturePasswordFromLine(line);
      return;
    }
    const conns = useConnectionStore.getState().connections;
    const port = parsed.port > 0 ? parsed.port : 22;
    const sameHostUser = (c: { host: string; user: string }) =>
      c.host.trim().toLowerCase() === parsed.host.toLowerCase()
      && (parsed.user === "" || c.user.trim().toLowerCase() === parsed.user.toLowerCase());
    const matched =
      conns.find((c) => sameHostUser(c) && (c.port || 22) === port)
      ?? conns.find((c) => sameHostUser(c))
      ?? conns.find((c) => c.host.trim().toLowerCase() === parsed.host.toLowerCase());

    const inferredUser = parsed.user || matched?.user || "";
    if (!inferredUser) return;

    // Arm the one-shot password capture only when the ssh client is
    // about to prompt interactively: no `-i`, and either no saved
    // match or a saved match whose auth kind is `password` (so the
    // keychain might still be empty). 60s window covers banner +
    // typing + Enter.
    const expectsInteractivePassword =
      !parsed.identityPath
      && (matched === undefined || matched.authKind === "password");
    // NOTE: we no longer arm the capture here. The backend PTY
    // reader fires `terminal:ssh-password-prompt` when it sees the
    // actual OpenSSH prompt in the output stream, and the listener
    // in this component arms the capture one line ahead of the
    // user's keystrokes. That's more precise than guessing from the
    // `ssh …` command line — it works for history-edited invocations,
    // pasted commands, and nested ssh; and it doesn't fire for
    // remote `sudo` / local `passwd` whose prompt shapes differ.
    // `expectsInteractivePassword` is retained only to suppress the
    // capture when we know a saved key/agent is already handling
    // auth — without a prompt from ssh there's nothing to capture.
    if (!expectsInteractivePassword) {
      pendingPasswordCaptureRef.current = null;
    }

    // Nested ssh inside a real SSH-backend tab: the backend watcher
    // on this tab's PTY won't see anything (the PTY is a remote ssh
    // channel, not a local process), so we still have to update tab
    // state from the input scan. Local-backend tabs defer entirely
    // to the watcher event.
    if (tab.backend === "ssh") {
      const authMode: "password" | "agent" | "key" | "auto" =
        matched?.authKind ?? (parsed.identityPath ? "key" : "auto");
      const keyPath = parsed.identityPath || matched?.keyPath || "";
      const savedConnectionIndex = matched ? matched.index : null;

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

      if (matched && matched.authKind === "password") {
        cmd
          .sshConnectionResolvePassword(matched.index)
          .then((password) => {
            if (!password) return;
            const current = useTabStore.getState().tabs.find((t) => t.id === tab.id);
            if (current?.nestedSshTarget && current.nestedSshTarget.savedConnectionIndex === matched.index) {
              useTabStore.getState().updateTab(tab.id, {
                nestedSshTarget: { ...current.nestedSshTarget, password },
              });
            }
          })
          .catch(() => {});
      }
    }
  }

  async function sendInput(data: string) {
    if (!session || !data) return;
    // Capture any complete lines BEFORE writing to the PTY so the
    // command buffer reflects the post-Enter state. The captured
    // lines are scanned for `ssh ...` after the write succeeds.
    const completed = captureCompletedCommands(data);
    // Mirror the same bytes into the smart-mode line buffer when
    // tracking is active. Done unconditionally — the helper itself
    // gates on `smartActiveRef`. M1 has no UI consumer; M2+ will
    // hand this buffer to the syntax-highlight overlay and Tab
    // popover.
    updateSmartLineBuffer(data);
    try {
      await cmd.terminalWrite(session.sessionId, data);
      setScrollbackOffset(0);
    } catch (e) {
      setError(formatError(e));
      return;
    }
    for (const line of completed) {
      const trimmed = line.trim();
      if (trimmed) applySshContextFromCommand(trimmed);
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

  // ── M4: Tab completion popover ─────────────────────────────────

  /**
   * Mirror of `pier-core::terminal::completions::find_word_start`.
   * Walks back from `cursor` while the char is part of a word
   * (anything not in our small set of operators / whitespace) and
   * returns the byte offset where the word begins.
   */
  function findWordStart(line: string, cursor: number): number {
    let i = Math.min(cursor, line.length);
    while (i > 0) {
      const ch = line[i - 1];
      if (
        ch === " " ||
        ch === "\t" ||
        ch === "\n" ||
        ch === "|" ||
        ch === "&" ||
        ch === ";" ||
        ch === ">" ||
        ch === "<"
      ) {
        break;
      }
      i -= 1;
    }
    return i;
  }

  function closeCompletion() {
    setCompletion((s) => (s.open ? { ...s, open: false } : s));
  }

  /**
   * Insert the diff between the candidate and the current word at
   * cursor. The shell's echo of those bytes drives the standard
   * mirror-buffer update path, so highlight + caret stay in sync
   * for free. A candidate shorter than the typed prefix is treated
   * as a no-op for M4 — backspace-and-replace lands in M5+ once we
   * have richer line-editor semantics.
   */
  function insertCompletion(item: Completion) {
    const line = smartLineBufferRef.current;
    const cursor = line.length;
    const wordStart = findWordStart(line, cursor);
    const currentWord = line.slice(wordStart, cursor);
    if (item.value.startsWith(currentWord)) {
      const diff = item.value.slice(currentWord.length);
      if (diff) void sendInput(diff);
    }
    closeCompletion();
  }

  /**
   * M6: extract the command name the user is currently typing.
   * Returns the first whitespace-delimited word of the mirror
   * buffer — that's the command position even when the cursor has
   * moved past the first argument. Empty string when there's
   * nothing to look up.
   */
  function commandAtCursor(): string {
    const line = smartLineBufferRef.current.trim();
    if (!line) return "";
    const space = line.search(/\s/);
    return space === -1 ? line : line.slice(0, space);
  }

  function closeMan() {
    setManState((s) => (s.open ? { ...s, open: false } : s));
  }

  /**
   * Ctrl+Shift+M handler. Opens the man popover immediately in a
   * loading state so it snaps into position even when the backend
   * spawn takes a few hundred ms; populates with the parsed result
   * (or an empty / error message) once `terminal_man_synopsis`
   * resolves.
   */
  async function openMan() {
    const command = commandAtCursor();
    if (!command) return;
    setManState({
      open: true,
      command,
      data: null,
      loading: true,
      errorMessage: null,
    });
    try {
      const data = await terminalManSynopsis(command);
      setManState((s) =>
        s.open && s.command === command
          ? { ...s, data, loading: false, errorMessage: null }
          : s,
      );
    } catch (e) {
      setManState((s) =>
        s.open && s.command === command
          ? {
              ...s,
              loading: false,
              data: null,
              errorMessage: formatError(e),
            }
          : s,
      );
    }
  }

  /**
   * Tab handler: ask the backend for completion candidates against
   * the current mirror buffer + cursor, then either auto-insert
   * (single match) or open the popover.
   */
  async function openCompletion() {
    if (!session) return;
    const line = smartLineBufferRef.current;
    const cursor = line.length;
    let cwd: string | null = null;
    try {
      cwd = (await cmd.terminalCurrentCwd(session.sessionId)) ?? null;
    } catch {
      cwd = null;
    }
    let items: Completion[] = [];
    try {
      items = await terminalCompletions(line, cursor, cwd, locale);
    } catch {
      // Fall through with `items` empty — history rows below may
      // still produce a useful popover even when the backend
      // completer fails (e.g. transient IPC blip).
    }

    // Prepend up-to-10 history rows that strictly extend the current
    // line. We slice from the user's word-start so `insertCompletion`'s
    // existing word-diff logic injects the right tail into the PTY.
    // Most-recent-first ordering is preserved by `historyRing`'s
    // insertion semantics in `useTerminalHistoryStore`.
    const historyRows: Completion[] = [];
    if (line.length > 0) {
      const wordStart = findWordStart(line, cursor);
      const seen = new Set<string>();
      for (const entry of historyRing) {
        if (historyRows.length >= 10) break;
        if (entry.length <= line.length) continue;
        if (!entry.startsWith(line)) continue;
        if (seen.has(entry)) continue;
        seen.add(entry);
        historyRows.push({
          kind: "history",
          value: entry.slice(wordStart),
          display: entry,
          hint: null,
          description: null,
        });
      }
    }
    items = [...historyRows, ...items];
    if (items.length === 0) return;
    // Always show the popover — even for a single match. The old
    // "auto-insert when exactly one candidate" path silently
    // consumed the first Tab when (e.g.) `cd Doc<Tab>` had a
    // unique completion, which made the popover feel unreliable
    // (users would press Tab a second time to see anything).
    // Visible feedback on every Tab > the half-second saved by
    // skipping the popover for one candidate.
    setCompletion({
      open: true,
      items,
      filtered: items,
      selectedIndex: 0,
    });
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    const mod = event.ctrlKey || event.metaKey;
    const selText = getSelectionText();

    // M4: completion popover key handling. Highest precedence — we
    // own arrow / Enter / Tab / Esc while the popover is open, so
    // those bytes never reach the underlying shell readline (which
    // would otherwise scroll its history or submit the line).
    if (completion.open) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setCompletion((s) =>
          s.filtered.length === 0
            ? s
            : {
                ...s,
                selectedIndex: Math.min(
                  s.selectedIndex + 1,
                  s.filtered.length - 1,
                ),
              },
        );
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setCompletion((s) => ({
          ...s,
          selectedIndex: Math.max(s.selectedIndex - 1, 0),
        }));
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        closeCompletion();
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        const sel = completion.filtered[completion.selectedIndex];
        if (sel) insertCompletion(sel);
        else closeCompletion();
        return;
      }
      // Any other key — fall through to the normal handler so the
      // user can keep typing and the popover refilters in real time.
    }

    // M4: Tab in smart mode pops the completion menu. SSH tabs
    // intentionally fall through to the existing transparent-Tab
    // path, since smart mode auto-bypasses there.
    if (
      smartActiveRef.current &&
      !completion.open &&
      event.key === "Tab" &&
      !event.shiftKey
    ) {
      event.preventDefault();
      void openCompletion();
      return;
    }

    // M6: Ctrl+Shift+M opens the man popover for the command at
    // cursor. Intercepted here (before the generic Ctrl-letter
    // path below) so the keystroke never reaches shell readline,
    // which would otherwise insert a literal carriage return for
    // Ctrl+M.
    if (
      smartActiveRef.current &&
      event.ctrlKey &&
      event.shiftKey &&
      !event.altKey &&
      !event.metaKey &&
      event.key.toLowerCase() === "m"
    ) {
      event.preventDefault();
      void openMan();
      return;
    }

    // M5: accept the live autosuggestion. ArrowRight matches fish's
    // accept-suggestion behaviour at end-of-line; Ctrl+E mirrors
    // zsh-autosuggestions. Both fall through when there's no
    // suggestion to accept, so the underlying shell readline still
    // receives them as cursor / end-of-line.
    if (smartActiveRef.current && suggestionSuffixRef.current) {
      const isAccept =
        event.key === "ArrowRight" ||
        (event.ctrlKey &&
          !event.altKey &&
          !event.metaKey &&
          event.key.toLowerCase() === "e");
      if (isAccept) {
        event.preventDefault();
        void sendInput(suggestionSuffixRef.current);
        return;
      }
    }

    if (mod && !event.altKey && event.key.toLowerCase() === "v") {
      event.preventDefault();
      void readClipboardText().then((text) => {
        if (text) void sendInput(text.replace(/\r?\n/g, "\r"));
      });
      return;
    }

    if (mod && !event.altKey && event.key.toLowerCase() === "c" && selText) {
      event.preventDefault();
      void writeClipboardText(selText);
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
    await writeClipboardText(sel);
  }

  async function pasteClipboard() {
    if (!session) return;
    const text = await readClipboardText();
    if (text) {
      try {
        await cmd.terminalWrite(session.sessionId, text);
      } catch {
        /* PTY write blocked */
      }
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
          {smartMode ? (
            <span
              className={`meta-pill ${smartActive ? "meta-pill--success" : ""}`}
              title={
                smartActive
                  ? t("Smart mode is intercepting Tab / autosuggest in this tab")
                  : t("Smart mode bypassed: alt-screen app (vim / htop / tmux)")
              }
            >
              {smartActive ? t("Smart") : t("Smart \u00b7 alt")}
            </span>
          ) : null}
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
            {needsPasswordRecovery && tab.sshSavedConnectionIndex !== null && (
              <button
                type="button"
                // Custom class — `.mini-button` styling is tuned for
                // light/neutral panel chrome and doesn't read well on
                // the terminal's dark background. The terminal-aware
                // variant in pier-x.css uses the negative palette
                // tokens that already match the surrounding error
                // text so the affordance feels native.
                className="terminal-recovery-btn"
                onClick={(event) => {
                  // Stop propagation so the parent terminal viewport's
                  // mousedown-focus handler doesn't steal focus before
                  // the click completes against the button.
                  event.stopPropagation();
                  const index = tab.sshSavedConnectionIndex;
                  if (index === null) return;
                  requestEditConnection(index);
                  onEditConnection?.(index);
                }}
                onMouseDown={(event) => event.stopPropagation()}
              >
                <KeyRound size={12} /> {t("Re-enter password")}
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
            {snapshot.lines.map((line, i) => {
              const usedCols = line.segments.reduce((n, s) => n + s.text.length, 0);
              const padCols = Math.max(0, snapshot.cols - usedCols);
              return (
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
                  {padCols > 0 && (
                    <span className="terminal-segment terminal-segment--filler" aria-hidden>
                      {" ".repeat(padCols)}
                    </span>
                  )}
                </div>
              );
            })}
            {smartActive &&
              snapshot.promptEnd &&
              (smartLineBufferText || suggestionSuffix) && (
                <TerminalSyntaxOverlay
                  text={smartLineBufferText}
                  promptEnd={snapshot.promptEnd}
                  charWidth={cellMetrics.charWidth}
                  rowHeight={cellMetrics.rowHeight}
                  bgColor={termTheme.bg}
                  suggestionSuffix={suggestionSuffix}
                />
              )}
            {smartActive && snapshot.promptEnd && (
              <div
                ref={cursorAnchorRef}
                aria-hidden
                style={{
                  position: "absolute",
                  // Anchor sits at the cursor cell so Popover can
                  // place the menu just below the row. Width 0 with
                  // `pointer-events: none` keeps it invisible to
                  // text selection and mouse events.
                  top:
                    snapshot.promptEnd[0] * cellMetrics.rowHeight,
                  left:
                    (snapshot.promptEnd[1] + smartLineBufferText.length) *
                    cellMetrics.charWidth,
                  width: 0,
                  height: cellMetrics.rowHeight,
                  pointerEvents: "none",
                }}
              />
            )}
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

      <CompletionPopover
        open={completion.open}
        anchor={cursorAnchorRef.current}
        items={completion.filtered}
        selectedIndex={completion.selectedIndex}
        onHighlight={(idx) =>
          setCompletion((s) => ({ ...s, selectedIndex: idx }))
        }
        onSelect={(item) => insertCompletion(item)}
        onClose={() => closeCompletion()}
      />

      <ManPagePopover
        open={manState.open}
        anchor={cursorAnchorRef.current}
        command={manState.command}
        data={manState.data}
        loading={manState.loading}
        errorMessage={manState.errorMessage}
        onClose={() => closeMan()}
      />
    </section>
  );
}
