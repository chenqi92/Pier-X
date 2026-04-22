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
import { readClipboardText, writeClipboardText } from "../lib/clipboard";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useUiActionsStore } from "../stores/useUiActionsStore";

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

  // Single-shot "the next typed line is probably the password ssh is
  // prompting for" tracker. Armed by `applySshContextFromCommand`
  // when the user types `ssh user@host` without `-i` and without
  // matching saved key/agent credentials, since in that case the
  // local ssh.exe child process is about to ask for a password and
  // we want to mirror that into `tab.sshPassword` so the right-side
  // russh session can authenticate the same way without a second
  // prompt. Cleared after one capture or after the deadline, and
  // skipped entirely if `tab.sshPassword` is already populated
  // (e.g. resolved from the keychain).
  const pendingPasswordCaptureRef = useRef<{ deadline: number } | null>(null);

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
    if (!pending) return;
    pendingPasswordCaptureRef.current = null; // single-shot regardless
    if (Date.now() > pending.deadline) return;

    const trimmed = line.trim();
    if (!trimmed) return;
    if (trimmed.length > 256) return;
    if (/\s/.test(trimmed)) return;

    // Don't overwrite a working password resolved from the keychain
    // or set by a previous capture — the user is just running their
    // first command at the new shell prompt, not re-entering creds.
    const current = useTabStore.getState().tabs.find((t) => t.id === tab.id);
    if (!current) return;
    if (tab.backend === "local") {
      if (current.sshPassword) return;
      updateTab(tab.id, { sshPassword: trimmed });
    } else if (current.nestedSshTarget) {
      if (current.nestedSshTarget.password) return;
      updateTab(tab.id, {
        nestedSshTarget: { ...current.nestedSshTarget, password: trimmed },
      });
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
    if (!parsed) {
      // Not an `ssh ...` line — but it might be the password the user
      // is typing in response to ssh's "password:" prompt. If we
      // recently saw an ssh command and have no working password yet,
      // mirror the captured line into the tab so the right-side russh
      // session can authenticate against the same target the local
      // ssh.exe is connecting to. One-shot: cleared after each call.
      maybeCapturePasswordFromLine(line);
      return;
    }
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

    // Default to `password` for unmatched / no-`-i` invocations: the
    // user is typing `ssh user@host` interactively, which means
    // ssh.exe is about to prompt for a password. We arm the
    // password-capture sweep below so the next line they type
    // (silently echoed by the terminal) lands in `tab.sshPassword`
    // and the right-side russh session can authenticate without a
    // second prompt. `-i` overrides to key auth, and a saved
    // connection's auth kind always wins.
    const authMode: "password" | "agent" | "key" =
      matched?.authKind ?? (parsed.identityPath ? "key" : "password");
    const keyPath = parsed.identityPath || matched?.keyPath || "";
    const savedConnectionIndex = matched ? matched.index : null;

    // Arm the one-shot password capture only if we don't expect
    // credentials to come from elsewhere: no key file, and either no
    // saved match or a saved match whose auth kind is `password`
    // (in which case the keychain might still be empty). 60s window
    // is generous enough for the user to read the welcome banner,
    // type their password, and hit Enter.
    const expectsInteractivePassword =
      !parsed.identityPath
      && (matched === undefined || matched.authKind === "password");
    pendingPasswordCaptureRef.current = expectsInteractivePassword
      ? { deadline: Date.now() + 60_000 }
      : null;

    // Re-typing the same `ssh user@host` after a disconnect should
    // keep any previously-resolved/captured password so the right
    // side reconnects without prompting again. A *different* target
    // (or a different saved-connection slot) gets a clean slate so
    // a stale wrong capture doesn't poison the next attempt.
    const sameTarget =
      savedConnectionIndex === tab.sshSavedConnectionIndex
      && tab.sshHost.trim().toLowerCase() === parsed.host.toLowerCase()
      && tab.sshUser.trim().toLowerCase() === inferredUser.toLowerCase()
      && tab.sshPort === port;

    if (tab.backend === "local") {
      updateTab(tab.id, {
        sshHost: parsed.host,
        sshPort: port,
        sshUser: inferredUser,
        sshAuthMode: authMode,
        sshKeyPath: keyPath,
        sshSavedConnectionIndex: savedConnectionIndex,
        sshPassword: sameTarget ? tab.sshPassword : "",
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
    // Capture any complete lines BEFORE writing to the PTY so the
    // command buffer reflects the post-Enter state. The captured
    // lines are scanned for `ssh ...` after the write succeeds.
    const completed = captureCompletedCommands(data);
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

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    const mod = event.ctrlKey || event.metaKey;
    const selText = getSelectionText();

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
                className="mini-button"
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
