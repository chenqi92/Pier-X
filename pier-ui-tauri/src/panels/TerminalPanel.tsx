// ── Terminal Panel ───────────────────────────────────────────────
// Per-tab terminal with 80ms snapshot polling, keyboard I/O,
// scrollback, and session lifecycle management.

import { SquareTerminal } from "lucide-react";
import { startTransition, useEffect, useRef, useState } from "react";
import * as cmd from "../lib/commands";
import { controlKeyMap } from "../lib/commands";
import ContextMenu, { type ContextMenuItem } from "../components/ContextMenu";
import { useI18n } from "../i18n/useI18n";
import type { TabState, TerminalSessionInfo, TerminalSnapshot, TerminalSize } from "../lib/types";
import { useTabStore } from "../stores/useTabStore";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useThemeStore, TERMINAL_THEMES } from "../stores/useThemeStore";

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
};

export default function TerminalPanel({ tab, isActive }: Props) {
  const { t } = useI18n();
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
  const [terminalSize, setTerminalSize] = useState<TerminalSize>({ cols: 120, rows: 26 });
  const [scrollbackOffset, setScrollbackOffset] = useState(0);
  const [visualBellActive, setVisualBellActive] = useState(false);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number } | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const measureRef = useRef<HTMLSpanElement | null>(null);
  const startupAppliedRef = useRef<string | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const bellTimerRef = useRef<number | null>(null);

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
        }
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    }

    void create();
    return () => { cancelled = true; };
  }, [session, terminalSize.cols, terminalSize.rows, tab.backend, tab.sshHost]);

  // ── Resize session ──────────────────────────────────────────

  useEffect(() => {
    if (!session) return;
    cmd.terminalResize(session.sessionId, terminalSize.cols, terminalSize.rows).catch((e) =>
      setError(String(e)),
    );
  }, [session, terminalSize.cols, terminalSize.rows]);

  // ── Apply scrollback settings ───────────────────────────────

  useEffect(() => {
    if (!session) {
      return;
    }
    cmd.terminalSetScrollbackLimit(session.sessionId, scrollbackLines).catch((e) =>
      setError(String(e)),
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
        setError(String(e));
      });
  }, [session?.sessionId, tab.id, tab.startupCommand]);

  // ── Snapshot polling (80ms active, paused when hidden) ──────

  useEffect(() => {
    if (!session) return;
    let disposed = false;
    let inflight = false;

    const refresh = () => {
      if (inflight) return;
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
          if (!disposed) setError(String(e));
        })
        .finally(() => { inflight = false; });
    };

    refresh();
    const interval = isActive ? 80 : 1000;
    const id = window.setInterval(refresh, interval);
    return () => { disposed = true; window.clearInterval(id); };
  }, [session, scrollbackOffset, isActive]);

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

  async function sendInput(data: string) {
    if (!session || !data) return;
    try {
      await cmd.terminalWrite(session.sessionId, data);
      setScrollbackOffset(0);
    } catch (e) {
      setError(String(e));
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

  const surfaceStatus = snapshot?.alive ? t("Live") : session ? t("Exited") : t("Booting");

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
          <span className={`meta-pill ${surfaceStatus === "Live" ? "meta-pill--success" : ""}`}>
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
          <div className="terminal-placeholder terminal-placeholder--error">{error}</div>
        ) : snapshot ? (
          <div
            className={rowSeparators ? "terminal-screen terminal-screen--ruled" : "terminal-screen"}
            style={{
              fontFamily: `"${monoFont}", monospace`,
              fontSize: `${terminalFontSize}px`,
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
