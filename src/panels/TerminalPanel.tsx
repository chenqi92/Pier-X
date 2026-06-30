// ── Terminal Panel ───────────────────────────────────────────────
// Per-tab terminal: event-driven snapshot refresh (with a slow safety
// interval), keyboard I/O, scrollback, and session lifecycle management.

import { KeyRound, SquareTerminal } from "lucide-react";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import * as cmd from "../lib/commands";
import { controlKeyMap } from "../lib/commands";
import ContextMenu, { type ContextMenuItem } from "../components/ContextMenu";
import {
  loadSnippets,
  saveSnippets,
  snippetDisplayLabel,
  makeSnippetId,
  type TerminalSnippet,
} from "../lib/terminalSnippets";
import TerminalSyntaxOverlay from "../components/TerminalSyntaxOverlay";
import CompletionPopover from "../components/CompletionPopover";
import ManPagePopover from "../components/ManPagePopover";
import { TerminalRow, type TerminalRowEnv } from "../components/TerminalRow";
import {
  terminalCompletions,
  terminalCompletionsRemote,
  terminalManSynopsis,
  type Completion,
  type ManSynopsis,
} from "../lib/terminalSmart";
import {
  useTerminalHistoryStore,
  suggestFromHistory,
} from "../stores/useTerminalHistoryStore";
import { useI18n } from "../i18n/useI18n";
import { shakeDialogOverlay } from "../lib/dialogShake";
import { isMissingKeychainError, localizeError } from "../i18n/localizeMessage";
import type {
  TabState,
  TerminalLine,
  TerminalSessionInfo,
  TerminalSnapshot,
  TerminalSize,
} from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import { useTabStore } from "../stores/useTabStore";
import { useAiStore } from "../stores/useAiStore";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useSudoStore } from "../stores/useSudoStore";
import { useStatusStore } from "../stores/useStatusStore";
import { useThemeStore, TERMINAL_THEMES } from "../stores/useThemeStore";
import { parseSshCommand } from "../lib/parseSshCommand";
import { readClipboardText, writeClipboardText } from "../lib/clipboard";
import { textCols } from "../lib/textCols";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useUiActionsStore } from "../stores/useUiActionsStore";
import { hasPendingHostKeyPrompts } from "../stores/useHostKeyPromptStore";
import { logEvent } from "../lib/logger";
import "../styles/terminal-panel.css";

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

// Shells only split on ASCII whitespace. Commands copied from web pages,
// IMEs, or macOS Option-Space can contain lookalike spaces such as NBSP
// or full-width space; normalize those at human text ingress so `ps -ef |`
// does not become one invalid `-ef<nbsp>|` argument.
const SHELL_COMPAT_SPACE_RE = /[\u00a0\u1680\u2000-\u200a\u202f\u205f\u3000]/g;
const TERMINAL_CREATE_RETRY_DELAYS_MS = [1200, 2500, 5000, 8000, 13000, 21000];

function normalizeTerminalCommandText(text: string): string {
  return text.replace(SHELL_COMPAT_SPACE_RE, " ");
}

function terminalCreateErrorString(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

// True when a post-session terminal command (resize / write / scrollback)
// failed only because the channel had already exited. Surfacing this as
// the big red error banner blanks the whole terminal; the "Exited" status
// chip + Restart button (driven by snapshot.alive === false) is the right
// recovery affordance, so callers swallow these instead of painting the
// banner over the last screen contents.
function isChannelExitedError(error: unknown): boolean {
  const message = terminalCreateErrorString(error).toLowerCase();
  return (
    message.includes("ssh channel task has exited") ||
    message.includes("ssh channel closed") ||
    message.includes("channel task has exited")
  );
}

function isTransientSshChannelOpenError(error: unknown): boolean {
  const message = terminalCreateErrorString(error).toLowerCase();
  // Permanent server-side rejections — retrying the channel open
  // cannot succeed, so don't burn the retry ladder on them.
  if (
    message.includes("administratively prohibited") ||
    message.includes("unknown channel type")
  ) {
    return false;
  }
  return (
    message.includes("failed to open channel") ||
    message.includes("connectfailed") ||
    message.includes("channel open")
  );
}

function terminalLineText(snapshot: TerminalSnapshot, row: number): string {
  const line = snapshot.lines[row];
  if (!line) return "";
  return line.segments.map((segment) => segment.text).join("");
}

function inferPromptUser(snapshot: TerminalSnapshot): string {
  const line = terminalLineText(snapshot, snapshot.cursorY).trimEnd();
  if (!/[#$%>]\s*$/.test(line)) return "";
  const matches = Array.from(line.matchAll(/(?:^|[\s([{<])([A-Za-z_][A-Za-z0-9_.-]*)@[A-Za-z0-9_.-]+(?=[\s)\]}>:/~-])/g));
  const match = matches[matches.length - 1];
  return match?.[1] ?? "";
}

type TerminalSelectionPoint = {
  row: number;
  col: number;
};

type TerminalSelectionModel = {
  anchor: TerminalSelectionPoint;
  focus: TerminalSelectionPoint;
};

function compareTerminalPoints(a: TerminalSelectionPoint, b: TerminalSelectionPoint): number {
  if (a.row !== b.row) return a.row - b.row;
  return a.col - b.col;
}

function normalizeTerminalSelection(
  selection: TerminalSelectionModel | null,
): { start: TerminalSelectionPoint; end: TerminalSelectionPoint } | null {
  if (!selection) return null;
  if (compareTerminalPoints(selection.anchor, selection.focus) <= 0) {
    return { start: selection.anchor, end: selection.focus };
  }
  return { start: selection.focus, end: selection.anchor };
}

function terminalSelectionHasText(selection: TerminalSelectionModel | null): boolean {
  const normalized = normalizeTerminalSelection(selection);
  if (!normalized) return false;
  return compareTerminalPoints(normalized.start, normalized.end) !== 0;
}

function terminalCellWidth(char: string): number {
  const cp = char.codePointAt(0) ?? 0;
  if (cp === 0) return 0;
  if (
    (cp >= 0x0300 && cp <= 0x036f) ||
    (cp >= 0x1ab0 && cp <= 0x1aff) ||
    (cp >= 0x1dc0 && cp <= 0x1dff) ||
    (cp >= 0xfe00 && cp <= 0xfe0f) ||
    (cp >= 0xfe20 && cp <= 0xfe2f)
  ) {
    return 0;
  }
  if (
    cp >= 0x1100 &&
    (cp <= 0x115f ||
      cp === 0x2329 ||
      cp === 0x232a ||
      (cp >= 0x2e80 && cp <= 0xa4cf && cp !== 0x303f) ||
      (cp >= 0xac00 && cp <= 0xd7a3) ||
      (cp >= 0xf900 && cp <= 0xfaff) ||
      (cp >= 0xfe10 && cp <= 0xfe19) ||
      (cp >= 0xfe30 && cp <= 0xfe6f) ||
      (cp >= 0xff00 && cp <= 0xff60) ||
      (cp >= 0xffe0 && cp <= 0xffe6) ||
      (cp >= 0x20000 && cp <= 0x3fffd))
  ) {
    return 2;
  }
  return 1;
}

function stringOffsetForTerminalCell(text: string, targetCell: number): number {
  if (targetCell <= 0) return 0;
  let cells = 0;
  let offset = 0;
  for (const char of Array.from(text)) {
    const width = terminalCellWidth(char);
    if (cells + width > targetCell) return offset;
    cells += width;
    offset += char.length;
    if (cells >= targetCell) return offset;
  }
  return text.length;
}

function sliceTextByTerminalCells(text: string, startCell: number, endCell: number): string {
  const start = stringOffsetForTerminalCell(text, startCell);
  const end = stringOffsetForTerminalCell(text, Math.max(startCell, endCell));
  return text.slice(start, end);
}

function terminalLineCells(line: TerminalLine): number {
  return line.segments.reduce((total, segment) => total + segment.cells, 0);
}

function sliceTerminalLine(line: TerminalLine, startCol: number, endCol: number): string {
  const safeStart = Math.max(0, startCol);
  const safeEnd = Math.max(safeStart, endCol);
  let text = "";
  let col = 0;
  for (const segment of line.segments) {
    const segmentStart = col;
    const segmentEnd = segmentStart + segment.cells;
    if (segmentEnd <= safeStart) {
      col = segmentEnd;
      continue;
    }
    if (segmentStart >= safeEnd) break;
    text += sliceTextByTerminalCells(
      segment.text,
      Math.max(0, safeStart - segmentStart),
      Math.min(segment.cells, safeEnd - segmentStart),
    );
    col = segmentEnd;
  }
  return text;
}

function copySliceTerminalLine(line: TerminalLine, startCol: number, endCol: number): string {
  return sliceTerminalLine(line, startCol, endCol).replace(/[ \t]+$/g, "");
}

function TerminalPanel({ tab, isActive, onEditConnection }: Props) {
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
  const sessionRef = useRef<TerminalSessionInfo | null>(null);
  const snapshotRef = useRef<TerminalSnapshot | null>(null);
  sessionRef.current = session;
  snapshotRef.current = snapshot;
  const [error, setError] = useState("");
  const [createAttempt, setCreateAttempt] = useState(0);
  const createRetryCountRef = useRef(0);
  const [needsPasswordRecovery, setNeedsPasswordRecovery] = useState(false);
  const requestEditConnection = useUiActionsStore((s) => s.requestEditConnection);
  const focusTerminalSeq = useUiActionsStore((s) => s.focusTerminalSeq);
  const focusTerminalSessionId = useUiActionsStore((s) => s.focusTerminalSessionId);
  const [terminalSize, setTerminalSize] = useState<TerminalSize>({ cols: 120, rows: 26 });
  const setStatusTerminalSize = useStatusStore((s) => s.setTerminalSize);
  const [scrollbackOffset, setScrollbackOffset] = useState(0);
  const [snapshotViewOffset, setSnapshotViewOffset] = useState(0);
  const snapshotViewOffsetRef = useRef(0);
  snapshotViewOffsetRef.current = snapshotViewOffset;
  const [visualBellActive, setVisualBellActive] = useState(false);
  const [selectingInTerminal, setSelectingInTerminal] = useState(false);
  const [terminalSelection, setTerminalSelectionState] =
    useState<TerminalSelectionModel | null>(null);
  const terminalSelectionRef = useRef<TerminalSelectionModel | null>(null);
  const selectionDragRef = useRef<TerminalSelectionModel | null>(null);
  const selectionDragCleanupRef = useRef<(() => void) | null>(null);
  const screenRef = useRef<HTMLDivElement | null>(null);
  const setTerminalSelection = (next: TerminalSelectionModel | null) => {
    terminalSelectionRef.current = next;
    setTerminalSelectionState(next);
    setSelectingInTerminal(terminalSelectionHasText(next));
  };
  // Brief gate after `isActive` flips on. While the panel was
  // display:none, the viewport had no box and the ResizeObserver
  // didn't measure it; the moment we re-show it the observer fires,
  // `terminalSize` updates, a SIGWINCH ships off, and the snapshot
  // re-flows to the new row count. Because the viewport is
  // bottom-anchored (`justify-content: flex-end`), the existing
  // grid visibly slides up while the new (taller) snapshot fills in
  // — that's the "上一个 tab 的内容从底部瞬间上去" the user reported.
  // We hide the screen for the first paint after activation so the
  // reflow happens behind the curtain.
  const [activating, setActivating] = useState(false);
  const wasActiveRef = useRef(isActive);
  // Snapshot present when activation masking starts. We keep the mask
  // until a different snapshot arrives at the post-resize row count.
  const activationSnapshotRef = useRef<TerminalSnapshot | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number } | null>(null);
  // Persistent user snippets surfaced in the right-click menu. Loaded
  // once on mount; the manager dialog re-saves on each commit.
  const [snippets, setSnippets] = useState<TerminalSnippet[]>(() =>
    loadSnippets(),
  );
  const [snippetsDialogOpen, setSnippetsDialogOpen] = useState(false);

  /** Paste a snippet's command into the active session. Honors the
   *  snippet's `runOnPaste` flag — when set, appends a newline so the
   *  shell submits without the user having to press Enter. Otherwise
   *  the line lands at the prompt for review. */
  async function pasteSnippet(s: TerminalSnippet) {
    if (!session) return;
    const text = normalizeTerminalCommandText(s.runOnPaste ? `${s.command}\n` : s.command);
    try {
      await cmd.terminalWrite(session.sessionId, text);
    } catch {
      /* PTY write blocked */
    }
  }

  /** Save snippet edits and persist. Used by the manager dialog. */
  function commitSnippets(next: TerminalSnippet[]) {
    setSnippets(next);
    saveSnippets(next);
  }
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const measureRef = useRef<HTMLSpanElement | null>(null);
  const startupAppliedRef = useRef<string | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const bellTimerRef = useRef<number | null>(null);
  const pendingResizeRef = useRef(false);
  const latestSizeRef = useRef(terminalSize);
  latestSizeRef.current = terminalSize;
  const lastShellUserRef = useRef(tab.currentShellUser);
  // The original duplicate-create concern (ResizeObserver re-firing
  // the effect mid-handshake and double-injecting the smart-mode
  // hook) is solved by keeping `terminalSize` out of the create
  // effect's dep array — see the comment on that effect below. We
  // intentionally do NOT add a `sessionCreatingRef` guard here: an
  // earlier version did, and it deadlocked the panel on
  // "Launching shell..." whenever React 19's strict-mode double
  // mount fired (or any of the actual deps changed mid-flight) —
  // cleanup set the local `cancelled` flag, the next effect run
  // bailed on `ref.current === true`, the in-flight create's
  // finally cleared the ref but nothing re-triggered React, and
  // `session` stayed null forever. The `cancelled` closure flag
  // is enough on its own — when it fires for an in-flight create,
  // the resolved session is closed via `terminalClose` instead of
  // setSession'd, so we don't leak handles.

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

  // Referentially-stable per-row environment for the memoized TerminalRow.
  // Changes only on theme / cursor-setting / column-count / cell-metric
  // changes, so a pushed snapshot re-renders only the rows whose content
  // hash differs.
  const rowEnv = useMemo<TerminalRowEnv>(
    () => ({
      cursorStyle,
      cursorBlink,
      ansi: termTheme.ansi,
      fg: termTheme.fg,
      cols: snapshot?.cols ?? 0,
      cellWidth: cellMetrics.charWidth,
    }),
    [cursorStyle, cursorBlink, termTheme, snapshot?.cols, cellMetrics.charWidth],
  );

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
    /** Snapshot of the mirror buffer at the moment the popover
     *  opened (after the longest-common-prefix auto-complete was
     *  applied — so this is what the user has *committed to* by
     *  pressing Tab). Used to compute the extra prefix the user
     *  types while the popover is open (for live filtering) and
     *  the remaining tail to inject on Enter. */
    basePrefix: string;
    /** Cached `findWordStart` of `basePrefix`. Each candidate's
     *  append-suffix is recomputed from this without walking the
     *  buffer on every render. */
    baseWordStart: number;
  };
  const [completion, setCompletion] = useState<CompletionState>({
    open: false,
    items: [],
    filtered: [],
    selectedIndex: 0,
    basePrefix: "",
    baseWordStart: 0,
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

  // Suppress-only window for generic secret prompts (remote sudo,
  // local passwd / su / login, 2FA). Armed by the backend's
  // `terminal:secret-prompt` event; while set, the next Enter-
  // terminated line is kept out of the history ring AND persistence.
  // Distinct from `pendingPasswordCaptureRef` because we never route
  // the value anywhere — these prompts must not feed the russh slot.
  const suppressHistoryRef = useRef<{ deadline: number } | null>(null);

  // Elevation-follow capture. `sudoCmdSeenAtRef` records when the user
  // last ran `sudo` or `su`; when a secret prompt fires shortly after AND
  // `followTerminalSudo` is on, `pendingSudoCaptureRef` arms so the next
  // typed line (the user's own password for `sudo`, or root's for `su`)
  // is mirrored into the session-only store — letting the right-side
  // panels follow the terminal's elevation with no second prompt. The
  // backend tries `sudo` then `su` with the captured secret, so either
  // kind works. Captured values stay in memory for the session only —
  // never the keychain.
  const sudoCmdSeenAtRef = useRef(0);
  const pendingSudoCaptureRef = useRef<{ deadline: number } | null>(null);

  // Bracketed-paste tracking for the smart line mirror. `active` is
  // true between `\e[200~` and `\e[201~`; `tainted` marks the current
  // line as containing pasted bytes so it stays out of history (a
  // pasted block can be a whole .env / a secret on its own line, and
  // its embedded newlines are content, not command submissions).
  const pasteActiveRef = useRef(false);
  const pasteTaintedRef = useRef(false);

  // Mirror DESYNC flag. The mirror conservatively resets its buffer
  // on bytes it can't model (history-nav escapes, a forwarded Tab,
  // unknown control chars) — but the shell's REAL line still has the
  // earlier content. Rendering the overlay from a reset buffer then
  // paints a wrong fragment on top of the line (e.g. type `cd `, Tab
  // → forwarded, type `/etc` → overlay shows `/etc` as a red
  // "missing command" over the grid's `cd /etc`). While desynced the
  // whole smart UI (overlay, autosuggest, Tab popover) stays silent;
  // the flag clears at the next prompt-end or Enter, when the line
  // state is knowable again.
  const mirrorDesyncedRef = useRef(false);
  const [mirrorDesynced, setMirrorDesyncedState] = useState(false);
  function setMirrorDesync(v: boolean) {
    if (mirrorDesyncedRef.current !== v) {
      mirrorDesyncedRef.current = v;
      setMirrorDesyncedState(v);
    }
  }

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
    if (!isActive) return;
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

    const doPrewarm = () =>
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

    // For a nested hop, register the jump parent FIRST and only then
    // prewarm — otherwise the prewarm would dial the inner address
    // directly (it's the effective target) and cache a session that
    // bypasses the jump. The dedicated effect below owns the mapping's
    // lifecycle (incl. cleanup); this redundant set just orders the
    // backend so the prewarm itself tunnels.
    const nested = tab.nestedSshTarget;
    if (nested) {
      const parentHost = tab.sshHost.trim();
      const parentUser = tab.sshUser.trim();
      if (!parentHost || !parentUser) return;
      cmd
        .sshSetHostVia({
          host: target.host,
          port: target.port,
          user: target.user,
          authMode: target.authMode,
          via: {
            host: parentHost,
            port: tab.sshPort,
            user: parentUser,
            authMode: tab.sshAuthMode,
            savedConnectionIndex: tab.sshSavedConnectionIndex,
          },
        })
        .then(doPrewarm)
        .catch(doPrewarm);
    } else {
      void doPrewarm();
    }
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
    isActive,
  ]);

  // ── Nested-SSH jump registration ────────────────────────────────
  // When this tab is inside a nested `ssh user@inner` (the overlay is
  // set), tell the backend to tunnel every right-side probe for the
  // inner host through this tab's PRIMARY (jump) session, instead of
  // dialing the — usually internal, unroutable — inner address
  // directly. The parent is this tab's primary ssh* identity, which the
  // watcher / saved connection populated for the outer hop. Cleared on
  // change / unmount so a later direct connect to the same address
  // doesn't ride an orphaned tunnel. Single-hop only: a deeper chain
  // overwrites the one overlay, so `via` always names the outermost
  // (primary) session — fine for the common bastion → inner case.
  useEffect(() => {
    const nested = tab.nestedSshTarget;
    if (!nested) return;
    const parentHost = tab.sshHost.trim();
    const parentUser = tab.sshUser.trim();
    if (!parentHost || !parentUser) return;
    const target = {
      host: nested.host,
      port: nested.port,
      user: nested.user,
      authMode: nested.authMode,
    };
    const via = {
      host: parentHost,
      port: tab.sshPort,
      user: parentUser,
      authMode: tab.sshAuthMode,
      savedConnectionIndex: tab.sshSavedConnectionIndex,
    };
    cmd.sshSetHostVia({ ...target, via }).catch(() => {});
    return () => {
      // Match-aware clear: pass the same `via` so a sibling tab that
      // re-registered a different jump parent for this inner host isn't
      // torn down by our cleanup.
      cmd.sshSetHostVia({ ...target, via, remove: true }).catch(() => {});
    };
  }, [
    tab.nestedSshTarget?.host,
    tab.nestedSshTarget?.port,
    tab.nestedSshTarget?.user,
    tab.nestedSshTarget?.authMode,
    tab.sshHost,
    tab.sshPort,
    tab.sshUser,
    tab.sshAuthMode,
    tab.sshSavedConnectionIndex,
  ]);

  // ── Measure terminal grid dimensions ────────────────────────

  useEffect(() => {
    const viewport = viewportRef.current;
    const measure = measureRef.current;
    if (!viewport || !measure) return;

    let resizeDebounce: number | null = null;

    const recalculate = () => {
      if (!isActive) return;
      // Don't reflow the terminal grid mid-drag. A live splitter drag transiently
      // makes the viewport very narrow; refitting to that width would clamp to the
      // 48-column floor and visibly deform the content. Defer (re-arming) until the
      // drag settles — the pane then snaps to either hidden or a comfortable width,
      // and we fit to that final size.
      if (document.body.classList.contains("is-resizing")) {
        if (resizeDebounce !== null) window.clearTimeout(resizeDebounce);
        resizeDebounce = window.setTimeout(recalculate, 100);
        return;
      }
      if (viewport.clientWidth <= 0 || viewport.clientHeight <= 0) return;
      const measureBox = measure.getBoundingClientRect();
      const charWidth = measureBox.width / 10 || 7.8;
      const style = window.getComputedStyle(viewport);
      const horizontalPadding =
        Number.parseFloat(style.paddingLeft) + Number.parseFloat(style.paddingRight);
      const verticalPadding =
        Number.parseFloat(style.paddingTop) + Number.parseFloat(style.paddingBottom);
      // Match the px row height used by the renderer (--terminal-row-h is set
      // via Math.ceil(fontSize * 1.45) inline). Using the fractional measured
      // height here would let `rows × ceil_row_h` exceed the viewport, clipping
      // the bottom of the cursor row under `overflow: hidden`.
      const rowH = Math.ceil(terminalFontSize * 1.45);
      const cols = Math.max(
        48,
        Math.min(220, Math.floor((viewport.clientWidth - horizontalPadding) / charWidth)),
      );
      const rows = Math.max(
        14,
        Math.min(72, Math.floor((viewport.clientHeight - verticalPadding) / rowH)),
      );
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
    // Collapse ResizeObserver bursts — the per-frame width changes during a
    // sidebar / right-panel open-close slide or a live drag-resize — into a
    // single trailing recalculate once motion settles. Without this, every
    // animation frame ran getComputedStyle + recomputed cols + setState + a
    // PTY resize, which is the real source of the toggle/drag jank. The
    // direct recalculate() above keeps mount / tab-switch / font changes
    // responsive (this effect re-runs on those via its deps).
    const observer = new ResizeObserver(() => {
      if (resizeDebounce !== null) window.clearTimeout(resizeDebounce);
      resizeDebounce = window.setTimeout(() => {
        resizeDebounce = null;
        recalculate();
      }, 100);
    });
    observer.observe(viewport);
    return () => {
      if (resizeDebounce !== null) window.clearTimeout(resizeDebounce);
      observer.disconnect();
    };
  }, [isActive, terminalFontSize, monoFont]);

  // ── Create session ──────────────────────────────────────────

  useEffect(() => {
    if (session) return;
    if (!isActive) return;
    let cancelled = false;
    let retryTimer: number | null = null;

    async function create() {
      try {
        // On an automatic channel-open retry (retry count > 0) keep
        // the existing error on screen — blanking it per attempt makes
        // the banner flicker. It's cleared on success below, and the
        // manual Restart path clears it itself.
        if (createRetryCountRef.current === 0) setError("");
        setNeedsPasswordRecovery(false);
        // Read the size from the ref so a ResizeObserver tick that
        // arrives mid-flight doesn't re-fire this effect (terminalSize
        // is intentionally not a dep — that's what kept us from
        // double-creating sessions originally). The session is created
        // at whatever size we have at call time; a follow-up SIGWINCH
        // from the resize effect adjusts once the session settles.
        const size = latestSizeRef.current;
        let next: TerminalSessionInfo;
        if (tab.backend === "ssh") {
          if (tab.sshSavedConnectionIndex !== null) {
            // Saved connection — backend resolves password from secure store
            next = await cmd.terminalCreateSshSaved(
              size.cols,
              size.rows,
              tab.sshSavedConnectionIndex,
            );
          } else {
            next = await cmd.terminalCreateSsh({
              cols: size.cols,
              rows: size.rows,
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
            size.cols,
            size.rows,
            undefined,
            smartMode,
          );
        }
        if (!cancelled) {
          setSession(next);
          setError("");
          createRetryCountRef.current = 0;
          setNeedsPasswordRecovery(false);
        } else {
          // The component / tab credentials changed before our backend
          // call returned — we own a live session that nobody is going
          // to consume. Close it so we don't leak a PTY (and, for SSH,
          // a shell channel) per cancelled attempt.
          cmd.terminalClose(next.sessionId).catch(() => {});
        }
      } catch (e) {
        if (!cancelled) {
          const missingKeychain = isMissingKeychainError(e);
          setError(formatError(e));
          if (missingKeychain) setNeedsPasswordRecovery(true);
          if (
            tab.backend === "ssh" &&
            !missingKeychain &&
            isTransientSshChannelOpenError(e) &&
            createRetryCountRef.current < TERMINAL_CREATE_RETRY_DELAYS_MS.length
          ) {
            const delay = TERMINAL_CREATE_RETRY_DELAYS_MS[createRetryCountRef.current];
            createRetryCountRef.current += 1;
            retryTimer = window.setTimeout(() => {
              retryTimer = null;
              if (!cancelled) setCreateAttempt((attempt) => attempt + 1);
            }, delay);
          }
        }
      }
    }

    void create();
    return () => {
      cancelled = true;
      if (retryTimer !== null) window.clearTimeout(retryTimer);
    };
    // `tab.sshPassword` is in the deps so a tab whose first
    // create() rejected with "saved password missing in keychain"
    // automatically retries once the user re-enters the password
    // via the recovery dialog (App.tsx propagates the new password
    // into matching tabs and nulls `terminalSessionId`, which we
    // mirror into local `session` state below).
  }, [session, isActive, tab.backend, tab.sshHost, tab.sshPassword, createAttempt]);

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

  // Mask activation until the resize round-trip has produced a fresh snapshot.
  // A fixed two-frame wait can still expose the old, bottom-anchored grid.
  useLayoutEffect(() => {
    const wasActive = wasActiveRef.current;
    wasActiveRef.current = isActive;
    if (!isActive || wasActive) {
      if (activating) setActivating(false);
      return;
    }
    activationSnapshotRef.current = snapshot;
    setActivating(true);
    const timeout = window.setTimeout(() => setActivating(false), 600);
    return () => window.clearTimeout(timeout);
    // `activating` and `snapshot` are intentionally not deps — we only react
    // to the isActive edge; the unmask effect below watches the snapshot.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isActive]);

  useEffect(() => {
    if (!activating) return;
    if (!snapshot) return;
    if (snapshot === activationSnapshotRef.current) return;
    if (snapshot.rows !== terminalSize.rows) return;
    const raf = window.requestAnimationFrame(() => setActivating(false));
    return () => window.cancelAnimationFrame(raf);
  }, [activating, snapshot, terminalSize.rows]);

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

  // Honor cross-component focus requests (AI panel "insert into
  // terminal"). The request carries the target session id; only the
  // matching active panel grabs focus, so a command inserted into tab
  // A's terminal doesn't yank focus to tab B.
  useEffect(() => {
    if (focusTerminalSeq === 0) return;
    if (!isActive) return;
    if (!session || focusTerminalSessionId !== session.sessionId) return;
    const viewport = viewportRef.current;
    if (!viewport) return;
    const raf = window.requestAnimationFrame(() => {
      viewport.focus({ preventScroll: true });
    });
    return () => window.cancelAnimationFrame(raf);
  }, [focusTerminalSeq, focusTerminalSessionId, session, isActive]);

  // Selection is fully model-driven: it is painted as overlay rects from
  // `terminalSelection` (see the selection layer in render), not by
  // borrowing the browser's native text selection. The old approach
  // mirrored the model onto a DOM Range every drag tick
  // (removeAllRanges/addRange); on Chromium-based WebViews that fought
  // the in-progress native drag and produced the directional /
  // flickering selection bugs. With model-only painting there is no
  // native range to reconcile — we only drop the model selection when
  // the tab goes inactive so a backgrounded tab keeps no stale highlight.
  useEffect(() => {
    if (isActive) return;
    terminalSelectionRef.current = null;
    setTerminalSelectionState(null);
    setSelectingInTerminal(false);
  }, [isActive]);

  // Visible-window selection rectangles derived from the model. Rows
  // scrolled out of the current snapshot window are simply not painted
  // (the model still carries them for copy). One rect per selected row:
  // the first row starts at the anchor column, the last ends at the
  // focus column, and rows in between fill to the right margin so a
  // multi-line selection reads as a continuous block.
  const selectionRects = useMemo(() => {
    const view = snapshot;
    const normalized = normalizeTerminalSelection(terminalSelection);
    if (!view || !normalized) return [] as Array<{
      row: number;
      top: number;
      left: number;
      width: number;
      height: number;
    }>;
    const { charWidth, rowHeight } = cellMetrics;
    if (charWidth <= 0 || rowHeight <= 0) return [];
    const visibleStart = visibleStartForSnapshot(view, snapshotViewOffset);
    const visibleEnd = visibleStart + view.lines.length - 1;
    const startRow = Math.max(normalized.start.row, visibleStart);
    const endRow = Math.min(normalized.end.row, visibleEnd);
    if (startRow > endRow) return [];
    const rects: Array<{ row: number; top: number; left: number; width: number; height: number }> = [];
    for (let row = startRow; row <= endRow; row += 1) {
      const fromCol = row === normalized.start.row ? normalized.start.col : 0;
      const toCol = row === normalized.end.row ? normalized.end.col : view.cols;
      const width = (Math.max(fromCol, toCol) - fromCol) * charWidth;
      if (width <= 0) continue;
      rects.push({
        row,
        top: (row - visibleStart) * rowHeight,
        left: fromCol * charWidth,
        width,
        height: rowHeight,
      });
    }
    return rects;
  }, [snapshot, terminalSelection, snapshotViewOffset, cellMetrics]);

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
    if (!isActive) return;
    if (document.body.classList.contains("is-resizing")) {
      pendingResizeRef.current = true;
      return;
    }
    pendingResizeRef.current = false;
    // A dead PTY can't resize — the exited surface (status chip +
    // Restart) is the recovery path; painting the BrokenPipe error
    // over the last screen contents would just hide it.
    if (snapshotRef.current?.alive === false) return;
    cmd.terminalResize(session.sessionId, terminalSize.cols, terminalSize.rows).catch((e) => {
      // The channel can die in the window between create and the first
      // snapshot (before the alive=false guard above can catch it). Let
      // the Exited surface handle that instead of the red banner.
      if (isChannelExitedError(e)) return;
      setError(formatError(e));
    });
  }, [session, isActive, terminalSize.cols, terminalSize.rows]);

  useEffect(() => {
    if (!session) return;
    if (!isActive) return;
    const onMouseUp = () => {
      if (!pendingResizeRef.current) return;
      // ResizeHandle clears the is-resizing class in its own mouseup
      // listener; defer to a microtask so that runs first regardless
      // of listener registration order.
      queueMicrotask(() => {
        if (!pendingResizeRef.current) return;
        if (document.body.classList.contains("is-resizing")) return;
        pendingResizeRef.current = false;
        if (snapshotRef.current?.alive === false) return;
        const size = latestSizeRef.current;
        cmd.terminalResize(session.sessionId, size.cols, size.rows).catch((e) => {
          if (isChannelExitedError(e)) return;
          setError(formatError(e));
        });
      });
    };
    window.addEventListener("mouseup", onMouseUp);
    return () => window.removeEventListener("mouseup", onMouseUp);
  }, [session, isActive]);

  // Copy-on-select (iTerm-style). Listen for mouseup on the viewport
  // and, if the resulting selection lives inside it and is non-empty,
  // ship it to the clipboard. Only active when the setting is on so
  // existing users keep the explicit ⌘C behavior.
  useEffect(() => {
    if (!isActive) return;
    if (!copyOnSelect) return;
    const viewport = viewportRef.current;
    if (!viewport) return;
    const handler = () => {
      void getSelectionTextForCopy().then((text) => {
        if (text) void writeClipboardText(text);
      });
    };
    viewport.addEventListener("mouseup", handler);
    return () => viewport.removeEventListener("mouseup", handler);
  }, [copyOnSelect, session, isActive]);

  useEffect(() => {
    if (!isActive) return;
    setStatusTerminalSize(terminalSize.cols, terminalSize.rows);
    return () => setStatusTerminalSize(null, null);
  }, [isActive, terminalSize.cols, terminalSize.rows, setStatusTerminalSize]);

  // ── Apply scrollback settings ───────────────────────────────

  useEffect(() => {
    if (!session) {
      return;
    }
    cmd.terminalSetScrollbackLimit(session.sessionId, scrollbackLines).catch((e) => {
      if (isChannelExitedError(e)) return;
      setError(formatError(e));
    });
  }, [session?.sessionId, scrollbackLines]);

  // ── Persist last cwd so a restart re-cd's the new session ────
  //
  // The shell's OSC 7 / OSC 9;9 cwd rides along on every snapshot
  // (see `next.currentCwd` in the snapshot refresh below), so we
  // read it from there and skip the dedicated cwd poll. A `cd`
  // followed by Enter triggers a DataReady → snapshot refresh, so
  // the cwd update is event-driven in practice.
  const lastCwdSampledRef = useRef<string | null>(tab.lastCwd ?? null);

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

    cmd.terminalWrite(
      session.sessionId,
      normalizeTerminalCommandText(`${tab.startupCommand}\r`),
    )
      .then(() => {
        updateTab(tab.id, { startupCommand: "" });
      })
      .catch((e) => {
        if (isChannelExitedError(e)) return;
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
    if (!isActive) return;
    let disposed = false;
    let inflight = false;
    let dirty = false;
    let safety: number | null = null;
    let rafHandle: number | null = null;
    // Snapshot carried on the most recent terminal:event payload this
    // frame; applied (instead of pulling) when the rAF coalescer fires.
    let pendingPush: TerminalSnapshot | null = null;

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

    // Coalesce bursts of `terminal:event` (one per ≤16ms backend frame)
    // into at most one IPC + setSnapshot per animation frame. Without
    // this, a `docker logs -f` flood drives setSnapshot well above
    // 60Hz and can race React's `useSyncExternalStore` snapshot
    // detection into "infinite loop" lock-up.
    const scheduleRefresh = () => {
      if (disposed) return;
      if (rafHandle !== null) return;
      rafHandle = window.requestAnimationFrame(() => {
        rafHandle = null;
        // A snapshot that rode in on a terminal:event payload wins — apply
        // it directly and skip the terminal_snapshot round-trip. Pull only
        // when no snapshot arrived this frame (history view, resize events,
        // or a frame the backend attach-throttle skipped).
        const pushed = pendingPush;
        pendingPush = null;
        if (pushed) {
          applySnapshot(pushed);
          if (!channelExited && !disposed) armSafety();
        } else {
          refresh();
        }
      });
    };

    // True once we've observed the backend reporting alive=false for
    // this session. After that we stop arming the safety timer and
    // stop scheduling new refreshes — a dead PTY isn't going to
    // produce new bytes, and on a flapping SSH link the per-1.5s
    // polling stacks IPC pressure on top of whatever the rest of the
    // app is already doing. The Restart button (which clears
    // `session`) is the supported recovery path.
    let channelExited = false;

    // Shared post-processing for a fresh snapshot, whether it arrived on a
    // pushed terminal:event payload or via a terminal_snapshot pull.
    const applySnapshot = (next: TerminalSnapshot) => {
      if (disposed) return;
      if (scrollbackOffset > next.scrollbackLen) {
        setScrollbackOffset(next.scrollbackLen);
      }
      setSnapshotViewOffset(Math.min(scrollbackOffset, next.scrollbackLen));
      const shellUser = inferPromptUser(next) || next.currentUser.trim();
      if (shellUser && shellUser !== lastShellUserRef.current) {
        lastShellUserRef.current = shellUser;
        updateTab(tab.id, { currentShellUser: shellUser });
        // Mirror the terminal's effective OS user to the backend so the
        // whole right side follows a `sudo -i` / `su root` (and de-follows
        // on `exit`) — including NOPASSWD hosts where no prompt fired and
        // there was nothing to capture.
        syncEffectiveUserElevation(shellUser);
      }
      // Persist last-seen cwd onto the tab record so the rehydrate path
      // can prepend a `cd …` startup command on restart.
      const nextCwd = next.currentCwd;
      if (nextCwd && nextCwd !== lastCwdSampledRef.current) {
        lastCwdSampledRef.current = nextCwd;
        updateTab(tab.id, { lastCwd: nextCwd });
      }
      setSnapshot(next);
      setError("");
      if (!next.alive) {
        channelExited = true;
      }
    };

    const refresh = () => {
      if (disposed) return;
      if (channelExited) return;
      // Pause refresh while a host-key TOFU dialog is open so we don't
      // pile snapshot fetches onto the stalled SSH gate. The next
      // backend event (keystroke, channel data) will re-fire refresh
      // once the user decides.
      if (hasPendingHostKeyPrompts()) return;
      if (inflight) { dirty = true; return; }
      dirty = false;
      inflight = true;
      cmd
        .terminalSnapshot(session.sessionId, scrollbackOffset)
        .then((next) => {
          applySnapshot(next);
        })
        .catch((e) => {
          if (!disposed) setError(formatError(e));
        })
        .finally(() => {
          inflight = false;
          if (channelExited || disposed) return;
          if (dirty) {
            scheduleRefresh();
          } else {
            armSafety();
          }
        });
    };

    refresh();

    // Tauri's `_unlisten` looks up `listeners[eventId]` on the JS side and
    // throws `TypeError: undefined is not an object (evaluating
    // 'listeners[eventId].handlerId')` if the entry was already removed —
    // which happens when this effect's cleanup races with the late
    // resolution of `listen()`. Swallow it so it never escapes as an
    // unhandledrejection.
    const safeUnlisten = (fn: UnlistenFn | undefined) => {
      if (!fn) return;
      try {
        fn();
      } catch {
        // Already-unregistered handler; nothing to do.
      }
    };

    // Listen for backend-pushed events. Each TerminalPanel subscribes;
    // the payload carries `sessionId` so we filter other tabs out.
    let unlisten: UnlistenFn | undefined;
    type TerminalEventPayload = {
      sessionId: string;
      kind: "data" | "exit";
      snapshot?: TerminalSnapshot | null;
    };
    void listen<TerminalEventPayload>("terminal:event", (event) => {
      if (disposed) return;
      if (event.payload.sessionId !== session.sessionId) return;
      // The backend attaches the live (offset 0) snapshot to data events
      // so we paint without a follow-up terminal_snapshot pull. While
      // scrolled into history (offset > 0) we ignore the push and pull the
      // offset view instead. The rAF coalescer applies the freshest push,
      // or pulls when none arrived this frame.
      const pushed = event.payload.snapshot;
      if (pushed && scrollbackOffset === 0) {
        pendingPush = pushed;
      }
      scheduleRefresh();
    })
      .then((u) => {
        if (disposed) safeUnlisten(u);
        else unlisten = u;
      })
      .catch(() => {});

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
    })
      .then((u) => {
        if (disposed) safeUnlisten(u);
        else unlistenSshState = u;
      })
      .catch(() => {});

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
    })
      .then((u) => {
        if (disposed) safeUnlisten(u);
        else unlistenSshPrompt = u;
      })
      .catch(() => {});

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
    })
      .then((u) => {
        if (disposed) safeUnlisten(u);
        else unlistenSshPassphrasePrompt = u;
      })
      .catch(() => {});

    // Generic secret-entry prompt (sudo / passwd / su / 2FA). Arms a
    // suppress-only window so the next typed line never reaches the
    // history ring or persistence. We do NOT capture or route the
    // value — these prompts are not SSH server auth.
    let unlistenSecretPrompt: UnlistenFn | undefined;
    void listen<{ sessionId: string }>("terminal:secret-prompt", (event) => {
      if (disposed) return;
      if (event.payload.sessionId !== session.sessionId) return;
      suppressHistoryRef.current = { deadline: Date.now() + 60_000 };
      // If this secret prompt closely follows a `sudo`/`su` command and
      // the user opted into following the terminal's elevation, arm a
      // one-shot capture of the next line into the session-only store.
      // Gated on a recent `sudo`/`su` so `passwd` / 2FA prompts (which
      // don't yield a panel-usable elevation password) are never captured.
      const sawSudoRecently = Date.now() - sudoCmdSeenAtRef.current < 15_000;
      if (sawSudoRecently && useSettingsStore.getState().followTerminalSudo) {
        pendingSudoCaptureRef.current = { deadline: Date.now() + 60_000 };
      }
    })
      .then((u) => {
        if (disposed) safeUnlisten(u);
        else unlistenSecretPrompt = u;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      if (safety !== null) window.clearTimeout(safety);
      if (rafHandle !== null) {
        window.cancelAnimationFrame(rafHandle);
        rafHandle = null;
      }
      safeUnlisten(unlisten);
      safeUnlisten(unlistenSshState);
      safeUnlisten(unlistenSshPrompt);
      safeUnlisten(unlistenSshPassphrasePrompt);
      safeUnlisten(unlistenSecretPrompt);
    };
  }, [session, isActive, scrollbackOffset]);

  // ── Cleanup on unmount ──────────────────────────────────────

  useEffect(() => {
    return () => {
      selectionDragCleanupRef.current?.();
      selectionDragCleanupRef.current = null;
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
      setMirrorDesync(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
    // A fresh prompt-end means the line is empty and knowable again.
    setMirrorDesync(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [smartActive, promptEndKey]);

  // True when the snapshot's caret sits at the end of the mirrored
  // line buffer, accounting for line wrap. We gate the live
  // autosuggestion (display + ArrowRight accept) on this so backing
  // up mid-line with ←/→ — or click-positioning the caret — never
  // dangles a misleading gray ghost past the cursor that ArrowRight
  // would inject into the PTY and overwrite later content.
  const cursorAtBufferEnd = (() => {
    if (!snapshot || !snapshot.promptEnd) return false;
    const [startRow, startCol] = snapshot.promptEnd;
    const cols = snapshot.cols || 1;
    // Grid columns, not chars — CJK glyphs occupy 2 cells each.
    const totalCols = startCol + textCols(smartLineBufferText);
    const endRow = startRow + Math.floor(totalCols / cols);
    const endCol = totalCols % cols;
    return snapshot.cursorY === endRow && snapshot.cursorX === endCol;
  })();
  const cursorAtBufferEndRef = useRef(false);
  cursorAtBufferEndRef.current = cursorAtBufferEnd;

  // M5: compute the autosuggestion suffix on every render where
  // smart mode is active. Cheap — `suggestFromHistory` is an O(n)
  // walk of at most 500 strings. Suppressed when the caret isn't
  // at end-of-line so a mid-line edit doesn't see ghost text.
  const suggestionSuffix = smartActive && cursorAtBufferEnd && !mirrorDesynced
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

  // Live narrow the popover as the user keeps typing past `basePrefix`.
  // The buffer at popover-open is `basePrefix` (after the LCP auto-
  // complete on Tab); each subsequent keystroke extends it. We filter
  // candidates whose append-suffix still starts with what's been
  // typed beyond `basePrefix`, mirroring fish / warp's narrowing
  // behaviour. When the user backspaces past the open-time base
  // (or filters down to zero), we just close — they can Tab again
  // for a fresh list.
  useEffect(() => {
    setCompletion((s) => {
      if (!s.open) return s;
      const line = smartLineBufferRef.current;
      if (!line.startsWith(s.basePrefix)) {
        return { ...s, open: false };
      }
      const extra = line.slice(s.basePrefix.length);
      const filtered = s.items.filter((it) => {
        const sfx = appendSuffixFor(it, s.basePrefix, s.baseWordStart);
        return sfx !== null && sfx.startsWith(extra) && sfx.length > extra.length;
      });
      if (filtered.length === 0) {
        return { ...s, open: false };
      }
      const selectedIndex =
        s.selectedIndex < filtered.length ? s.selectedIndex : 0;
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
        // Escape sequence: consume the whole CSI / SS3 then decide
        // whether to drop the buffer. Cursor-left / cursor-right just
        // move the caret without rewriting the line, so we preserve
        // the in-progress capture across them — otherwise click-to-
        // position and ←/→ edits would invalidate the ssh-command
        // detection buffer for any line the user edits before Enter.
        const next = data[i + 1];
        if (next === "[") {
          let j = i + 2;
          while (j < data.length) {
            if (/[A-Za-z~]/.test(data[j])) break;
            j += 1;
          }
          const final = j < data.length ? data[j] : "";
          if (final !== "C" && final !== "D") {
            commandBufferRef.current = "";
          }
          i = j;
        } else if (next === "O") {
          const j = i + 2;
          const final = j < data.length ? data[j] : "";
          if (final !== "C" && final !== "D") {
            commandBufferRef.current = "";
          }
          i = j;
        } else {
          commandBufferRef.current = "";
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
        // A newline arriving inside a bracketed paste is pasted
        // content, not a command submission — don't record it and
        // don't reset the line buffer.
        if (pasteActiveRef.current) {
          if (code === CR && data.charCodeAt(i + 1) === LF) i += 1;
          continue;
        }
        // M5 history capture: push the just-submitted line into the
        // ring before clearing. We used to capture this on the
        // OSC 133;C edge (`awaiting_input` true → false), but remote
        // shells reached over `ssh` don't emit OSC 133, so that
        // path missed every command typed inside SSH. The byte
        // stream sees Enter regardless of where the shell lives.
        const submitted = buf.trim();
        // A line assembled from a paste never enters history (secrets
        // / config blocks are commonly pasted). One-shot per submit.
        const pasteTainted = pasteTaintedRef.current;
        pasteTaintedRef.current = false;
        // Never record a line typed at a secret prompt — neither in
        // the in-memory ring (autosuggest would resurface it) nor on
        // disk. `pendingPasswordCaptureRef` covers the OpenSSH
        // password / key-passphrase prompts (also routed to the russh
        // slot); `suppressHistoryRef` covers the broad suppress-only
        // prompts (sudo / passwd / su / 2FA). This runs before the
        // post-write scan that disarms `pendingPasswordCaptureRef`, so
        // both refs are still set here.
        const now = Date.now();
        const inPasswordWindow =
          (pendingPasswordCaptureRef.current !== null
            && now <= pendingPasswordCaptureRef.current.deadline)
          || (suppressHistoryRef.current !== null
            && now <= suppressHistoryRef.current.deadline);
        if (suppressHistoryRef.current !== null) {
          // One-shot: a secret prompt arms exactly one line.
          suppressHistoryRef.current = null;
        }
        // A desynced buffer is a fragment of the real line — never
        // record it as history (`cd /etc` typed around a forwarded
        // Tab would otherwise save as `/etc`).
        if (submitted && !inPasswordWindow && !pasteTainted && !mirrorDesyncedRef.current) {
          pushHistory(submitted, {
            shell: session?.shell,
            persist: historyPersist,
          });
        }
        buf = "";
        // Enter starts a fresh, fully-knowable line — re-arm.
        setMirrorDesync(false);
        if (code === CR && data.charCodeAt(i + 1) === LF) i += 1;
        continue;
      }
      if (code === ESC) {
        // We don't fully model arrow-key navigation inside the mirror,
        // but cursor-left / cursor-right just move the caret without
        // changing line content — preserving the buffer keeps the
        // syntax overlay & autosuggest stable when the user backs up
        // to fix a typo. Any other escape sequence (history nav,
        // function keys, …) still resets, matching the previous
        // conservative stance.
        const next = data[i + 1];
        if (next === "[") {
          let j = i + 2;
          while (j < data.length) {
            if (/[A-Za-z~]/.test(data[j])) break;
            j += 1;
          }
          const final = j < data.length ? data[j] : "";
          const params = data.slice(i + 2, j);
          // Bracketed-paste boundaries: `\e[200~` start / `\e[201~` end.
          if (final === "~" && (params === "200" || params === "201")) {
            if (params === "200") {
              pasteActiveRef.current = true;
              pasteTaintedRef.current = true;
            } else {
              pasteActiveRef.current = false;
            }
            i = j;
            continue;
          }
          if (final !== "C" && final !== "D") {
            // History nav / function keys — the shell will rewrite
            // the line in ways we can't model. Buffer is no longer
            // the line; mute the smart UI until the next prompt.
            buf = "";
            setMirrorDesync(true);
          }
          i = j;
        } else if (next === "O") {
          // SS3: cursor keys can also arrive as ESC O C / ESC O D.
          const j = i + 2;
          const final = j < data.length ? data[j] : "";
          if (final !== "C" && final !== "D") {
            buf = "";
            setMirrorDesync(true);
          }
          i = j;
        } else {
          buf = "";
          setMirrorDesync(true);
        }
        continue;
      }
      if (code === DEL || code === BS) {
        buf = buf.slice(0, -1);
        continue;
      }
      if (code === ETX || code === NAK) {
        // Ctrl+C / Ctrl+U genuinely empty the shell's line — the
        // reset buffer is ACCURATE, not desynced.
        buf = "";
        continue;
      }
      if (code < 0x20 || code === 0x7f) {
        // Tab (forwarded to the shell's own completion), Ctrl+R, …
        // — the shell may extend or rewrite the line invisibly to
        // the mirror. Mute the smart UI until the next prompt.
        buf = "";
        setMirrorDesync(true);
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
   * Mirror the password typed at a `sudo`/`su` prompt into the
   * **session-only** store so the right-side panels follow the terminal's
   * elevation with no second prompt. Armed by the secret-prompt listener
   * only after a recent `sudo`/`su` (never `passwd` / 2FA) and only when
   * `followTerminalSudo` is on. The backend's `exec_as_effective` tries
   * `sudo` then `su` with this secret, so a `sudo` (own) password or a
   * `su` (root) password both work. Stored via `set` (in-memory for the
   * session), never `setPersistent` (keychain). A wrong capture
   * self-heals: the panel's permission-denied flow re-prompts + overwrites.
   */
  function maybeCaptureSudoFromLine(line: string): void {
    const pending = pendingSudoCaptureRef.current;
    if (!pending) return;
    pendingSudoCaptureRef.current = null; // one-shot
    if (Date.now() > pending.deadline) return;
    const trimmed = line.trim();
    if (!trimmed || trimmed.length > 256) return;
    const current = useTabStore.getState().tabs.find((t) => t.id === tab.id);
    if (!current) return;
    // The host the panels target — primary fields, or the nested hop.
    const host = tab.backend === "local" ? current.sshHost : current.nestedSshTarget?.host ?? "";
    const port = tab.backend === "local" ? current.sshPort : current.nestedSshTarget?.port ?? 22;
    const user = tab.backend === "local" ? current.sshUser : current.nestedSshTarget?.user ?? "";
    if (!host || !user) return;
    const authMode =
      (tab.backend === "local" ? current.sshAuthMode : current.nestedSshTarget?.authMode) ??
      "password";
    // Session-only: NEVER the keychain — this is a captured credential.
    useSudoStore.getState().set(
      { host, port, user, authMode, password: "", keyPath: "", savedConnectionIndex: null },
      trimmed,
    );
    logEvent(
      "INFO",
      "elevation.capture",
      `tab=${tab.id} captured sudo password from terminal (len=${trimmed.length}) for ${user}@${host}:${port}`,
    );
  }

  /**
   * Mirror the terminal's current effective OS user (`tab.currentShellUser`)
   * to the backend host-elevation state so every right-side panel follows
   * the terminal's `sudo -i` / `su root` — and drops back on `exit`.
   *
   * Unlike {@link maybeCaptureSudoFromLine}, this does NOT need a captured
   * password: on a NOPASSWD / cached-credentials host `sudo -i` fires no
   * prompt, so there is nothing to capture, yet the operator is genuinely
   * root. Sending the effective user arms the backend session for a
   * passwordless `sudo -n`, so detection / monitor / DB probes run (and are
   * labeled) as root. Gated on `followTerminalSudo`; when off, or when the
   * shell is back at the login user, it clears the arming (`effectiveUser:
   * null`). Uses {@link effectiveSshTarget} so the host key matches the one
   * the panels target.
   */
  function syncEffectiveUserElevation(shellUser: string): void {
    const current = useTabStore.getState().tabs.find((t) => t.id === tab.id);
    if (!current) return;
    const target = effectiveSshTarget(current);
    if (!target) return;
    const follow = useSettingsStore.getState().followTerminalSudo;
    const trimmed = shellUser.trim();
    const elevated = follow && !!trimmed && trimmed !== target.user;
    void cmd
      .sshSetHostEffectiveUser({
        host: target.host,
        port: target.port,
        user: target.user,
        authMode: target.authMode,
        password: target.password ?? "",
        keyPath: target.keyPath ?? "",
        savedConnectionIndex: target.savedConnectionIndex ?? null,
        effectiveUser: elevated ? trimmed : null,
      })
      .catch(() => {});
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
          currentShellUser: "",
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
      currentShellUser: target.user,
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
    // Note an interactive elevation (`sudo` or `su`) so a following
    // secret prompt can arm the follow-capture. Skip `sudo -n` (never
    // prompts). The captured value is whatever the user types — their
    // own password (sudo) or root's (su); the backend tries `sudo` then
    // `su`, so either works.
    const lead = line.trimStart();
    if (/^(sudo|su)(\s|$)/.test(lead) && !/^sudo\s+-n\b/.test(lead)) {
      sudoCmdSeenAtRef.current = Date.now();
    }
    const parsed = parseSshCommand(line);
    if (!parsed) {
      maybeCapturePasswordFromLine(line);
      maybeCaptureSudoFromLine(line);
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

    // Decide whether this `ssh` line is a nested hop we have to record
    // ourselves. Two cases:
    //   - SSH-backend tab: the PTY is a remote channel, the watcher
    //     can't see the inner process — input parsing is the only
    //     signal.
    //   - Local-backend tab whose primary SSH fields are already
    //     populated: the watcher has confirmed the outer `ssh` and
    //     can't see the second hop typed inside it. Without this
    //     branch the right side stays pinned to the first host.
    // The first ssh on a fresh local tab still defers to the watcher
    // (primary is empty here, so the gate skips us) — letting the
    // watcher remain authoritative for argv → connection matching.
    const isNestedOnSshTab = tab.backend === "ssh";
    const isNestedOnLocalTab =
      tab.backend === "local"
      && tab.sshHost.trim().length > 0
      && tab.sshUser.trim().length > 0;
    if (isNestedOnSshTab || isNestedOnLocalTab) {
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
        currentShellUser: inferredUser,
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

  function visibleStartForSnapshot(view: TerminalSnapshot, viewOffset: number): number {
    return Math.max(0, view.scrollbackLen - Math.min(viewOffset, view.scrollbackLen));
  }

  function terminalPointFromClient(
    clientX: number,
    clientY: number,
    viewOffset = snapshotViewOffsetRef.current,
  ): TerminalSelectionPoint | null {
    // Read the snapshot / view offset through refs: the selection-drag
    // listeners on `window` capture this closure at mousedown, and a
    // mid-drag wheel scroll or streaming output must map rows against
    // the live view, not the render the drag started in. cellMetrics
    // stays render-scoped — it only changes on font remeasure, which
    // can't happen while the pointer is held down.
    const view = snapshotRef.current;
    const screen = screenRef.current;
    if (!screen || !view) return null;
    const rowHeight = cellMetrics.rowHeight;
    const charWidth = cellMetrics.charWidth;
    if (rowHeight <= 0 || charWidth <= 0) return null;

    const rect = screen.getBoundingClientRect();
    const row = Math.max(
      0,
      Math.min(view.lines.length - 1, Math.floor((clientY - rect.top) / rowHeight)),
    );
    const col = Math.max(0, Math.min(view.cols, Math.floor((clientX - rect.left) / charWidth)));
    return {
      row: visibleStartForSnapshot(view, viewOffset) + row,
      col,
    };
  }

  // Word boundaries for double-click selection: the maximal run of
  // non-whitespace cells around the clicked cell. Returns null when the
  // click lands on whitespace or past the line content. Works in cell
  // space (CJK-aware) by mapping the clicked column to a string offset
  // and back through textCols.
  function wordSelectionAt(clientX: number, clientY: number): TerminalSelectionModel | null {
    const point = terminalPointFromClient(clientX, clientY);
    const view = snapshotRef.current;
    if (!point || !view) return null;
    const line = view.lines[point.row - visibleStartForSnapshot(view, snapshotViewOffsetRef.current)];
    if (!line) return null;
    const text = line.segments.map((s) => s.text).join("");
    if (!text) return null;
    const clickChar = stringOffsetForTerminalCell(text, point.col);
    const isWord = (ch: string | undefined) => ch !== undefined && ch !== " " && ch !== "\t";
    if (!isWord(text[clickChar])) return null;
    let startChar = clickChar;
    while (startChar > 0 && isWord(text[startChar - 1])) startChar -= 1;
    let endChar = clickChar;
    while (endChar < text.length && isWord(text[endChar])) endChar += 1;
    return {
      anchor: { row: point.row, col: textCols(text.slice(0, startChar)) },
      focus: { row: point.row, col: textCols(text.slice(0, endChar)) },
    };
  }

  // Whole visible row for triple-click selection (column 0 → line
  // content end). Returns null for an empty row.
  function lineSelectionAt(clientX: number, clientY: number): TerminalSelectionModel | null {
    const point = terminalPointFromClient(clientX, clientY);
    const view = snapshotRef.current;
    if (!point || !view) return null;
    const line = view.lines[point.row - visibleStartForSnapshot(view, snapshotViewOffsetRef.current)];
    if (!line) return null;
    const end = terminalLineCells(line);
    if (end <= 0) return null;
    return { anchor: { row: point.row, col: 0 }, focus: { row: point.row, col: end } };
  }

  function hasCopyableSelection(): boolean {
    return terminalSelectionHasText(terminalSelectionRef.current);
  }

  async function getTerminalSelectionText(): Promise<string> {
    const session = sessionRef.current;
    const currentSnapshot = snapshotRef.current;
    const normalized = normalizeTerminalSelection(terminalSelectionRef.current);
    if (!session || !currentSnapshot || !normalized) return "";
    if (compareTerminalPoints(normalized.start, normalized.end) === 0) return "";

    const lines = new Map<number, TerminalLine>();
    const addSnapshotLines = (view: TerminalSnapshot, viewOffset: number) => {
      const visibleStart = visibleStartForSnapshot(view, viewOffset);
      view.lines.forEach((line, index) => {
        lines.set(visibleStart + index, line);
      });
    };

    addSnapshotLines(currentSnapshot, snapshotViewOffsetRef.current);
    // Each fetch returns a viewport-height window, so cap the total:
    // a huge selection must not queue thousands of sequential IPC
    // round-trips on mouseup. When the cap hits we truncate to the
    // rows we actually fetched instead of blank-filling the rest.
    const maxFetches = 64;
    let cursor = normalized.start.row;
    let fetches = 0;
    let retriedCursor = -1;
    while (cursor <= normalized.end.row && fetches < maxFetches) {
      if (lines.has(cursor)) {
        while (cursor <= normalized.end.row && lines.has(cursor)) {
          cursor += 1;
        }
        continue;
      }

      const latest = snapshotRef.current ?? currentSnapshot;
      const offset = Math.max(0, Math.min(latest.scrollbackLen, latest.scrollbackLen - cursor));
      fetches += 1;
      try {
        const view = await cmd.terminalSnapshot(session.sessionId, offset);
        addSnapshotLines(view, offset);
        if (!lines.has(cursor)) {
          // The window raced output growth and missed the cursor row;
          // retry it once against the fresh scrollbackLen, then give
          // this row up (it stays a blank line in the copy).
          if (retriedCursor !== cursor) {
            retriedCursor = cursor;
            continue;
          }
          cursor += 1;
        }
      } catch {
        break;
      }
    }

    // Fetch cap or IPC failure: emit only up to the last row we
    // resolved rather than padding the remainder with empty lines.
    const lastRow = cursor > normalized.end.row ? normalized.end.row : cursor - 1;
    const parts: string[] = [];
    for (let row = normalized.start.row; row <= lastRow; row += 1) {
      const line = lines.get(row);
      if (!line) {
        parts.push("");
        continue;
      }
      const lineEnd = terminalLineCells(line);
      if (normalized.start.row === normalized.end.row) {
        parts.push(copySliceTerminalLine(line, normalized.start.col, normalized.end.col));
      } else if (row === normalized.start.row) {
        parts.push(copySliceTerminalLine(line, normalized.start.col, lineEnd));
      } else if (row === normalized.end.row) {
        parts.push(copySliceTerminalLine(line, 0, normalized.end.col));
      } else {
        parts.push(copySliceTerminalLine(line, 0, lineEnd));
      }
    }
    return parts.join("\n");
  }

  async function getSelectionTextForCopy(): Promise<string> {
    return getTerminalSelectionText();
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

  /**
   * Compute what we'd need to *append* to `basePrefix` to land on
   * `item.value`. Pure — used by both LCP-on-Tab and accept-on-Enter.
   * Returns `null` when the candidate doesn't share the current word
   * prefix (rare but possible with stale popover candidates).
   */
  function appendSuffixFor(
    item: Completion,
    basePrefix: string,
    baseWordStart: number,
  ): string | null {
    const baseWord = basePrefix.slice(baseWordStart);
    if (!item.value.startsWith(baseWord)) return null;
    return item.value.slice(baseWord.length);
  }

  /** Longest string `p` such that every entry in `xs` starts with `p`.
   *  Empty string when the inputs disagree from the first character. */
  function longestCommonPrefix(xs: string[]): string {
    if (xs.length === 0) return "";
    let p = xs[0];
    for (let i = 1; i < xs.length; i += 1) {
      while (!xs[i].startsWith(p)) {
        p = p.slice(0, -1);
        if (!p) return "";
      }
    }
    return p;
  }

  /** Cycle the popover highlight (visual only — PTY isn't touched).
   *  Wraps modulo `filtered.length`. */
  function cycleSelection(direction: 1 | -1) {
    setCompletion((s) => {
      if (!s.open || s.filtered.length === 0) return s;
      const next = (s.selectedIndex + direction + s.filtered.length) %
        s.filtered.length;
      return { ...s, selectedIndex: next };
    });
  }

  /** Close the popover without touching the line. The line content
   *  past `basePrefix` is the longest-common-prefix we already auto-
   *  completed on Tab plus any chars the user typed afterwards —
   *  both are theirs to keep. */
  function closeCompletion() {
    setCompletion((s) => (s.open ? { ...s, open: false } : s));
  }

  /** Enter / click handler. Inject the rest of the highlighted
   *  candidate that isn't already in the buffer (basePrefix +
   *  whatever the user typed in the popover) and dismiss. */
  function acceptCompletion() {
    setCompletion((s) => {
      if (!s.open) return s;
      const item = s.filtered[s.selectedIndex];
      if (item) {
        const fullSuffix = appendSuffixFor(item, s.basePrefix, s.baseWordStart);
        if (fullSuffix !== null) {
          const alreadyTyped = smartLineBufferRef.current.slice(s.basePrefix.length);
          if (fullSuffix.startsWith(alreadyTyped)) {
            const remaining = fullSuffix.slice(alreadyTyped.length);
            if (remaining.length > 0) void sendInput(remaining);
          }
        }
      }
      return { ...s, open: false };
    });
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
   * Tab handler: open the popover with backend + history candidates,
   * and *eagerly* insert the first one into the PTY so the user sees
   * the highlighted candidate reflected in the line immediately
   * (warp / fish menu-complete pattern). Subsequent Tabs cycle the
   * highlight + replace the inserted text.
   */
  async function openCompletion() {
    if (!session) return;
    if (mirrorDesyncedRef.current) {
      // We don't know what the line actually contains — completing
      // against the mirror would target the wrong word. Hand the Tab
      // to the shell's own completion instead.
      await sendInput("\t");
      return;
    }
    const line = smartLineBufferRef.current;
    // Backend slices `line` by byte offset, but JS `.length` returns
    // UTF-16 code units — those disagree the moment the line has any
    // multi-byte char (Chinese punctuation `。`, emoji, etc.) and the
    // resulting mismatch panics inside `complete_with_library`. Send
    // the UTF-8 byte length so backend slicing always lands on a
    // char boundary.
    const cursor = new TextEncoder().encode(line).length;
    let cwd: string | null = null;
    try {
      cwd = (await cmd.terminalCurrentCwd(session.sessionId)) ?? null;
    } catch {
      cwd = null;
    }
    let items: Completion[] = [];
    try {
      // For russh tabs, route file rows through SFTP so `cd /mnt/da`
      // + Tab on a remote host actually lists *that host's* /mnt/.
      // We pass the same `(host, port, user, authMode)` quadruple
      // the SFTP cache is keyed by; backend falls back to local
      // readdir if the cache hasn't seen the session yet (e.g. tab
      // still authenticating). Library + history rows stay client-
      // side either way.
      if (tab.backend === "ssh" && tab.sshHost && tab.sshUser) {
        items = await terminalCompletionsRemote(
          line,
          cursor,
          cwd,
          locale,
          tab.sshHost,
          tab.sshPort,
          tab.sshUser,
          tab.sshAuthMode,
        );
      } else {
        items = await terminalCompletions(line, cursor, cwd, locale);
      }
    } catch {
      // Fall through with `items` empty — history rows below may
      // still produce a useful popover even when the backend
      // completer fails (e.g. transient IPC blip).
    }

    // Prepend up-to-10 history rows that strictly extend the current
    // line. We slice from the user's word-start so the cycle/insert
    // logic injects the right tail into the PTY.
    const wordStart = findWordStart(line, cursor);
    const historyRows: Completion[] = [];
    if (line.length > 0) {
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
    if (items.length === 0) {
      // No client-side candidates — forward the Tab to the shell so
      // its own completion still answers (remote bash in SSH tabs;
      // before this, a zero-candidate Tab was swallowed entirely).
      // The forwarded control byte marks the mirror DESYNCED, which
      // mutes the overlay / autosuggest until the next prompt — the
      // shell may rewrite the line in ways the mirror can't see.
      await sendInput("\t");
      return;
    }

    // Mainstream Tab semantics:
    //   1. Compute the longest common append-suffix among all
    //      candidates and write *that* into the line (auto-complete
    //      to the unambiguous part — exactly what bash does on the
    //      first Tab).
    //   2. If the LCP collapses every candidate to a unique match,
    //      we're done — don't open the popover at all.
    //   3. Otherwise open the popover so the user can pick. The
    //      popover only highlights visually; cycling Tabs / arrows
    //      doesn't touch the PTY. Enter / click commits the selected
    //      candidate's remaining tail.
    const suffixes: string[] = [];
    for (const it of items) {
      const s = appendSuffixFor(it, line, wordStart);
      if (s !== null) suffixes.push(s);
    }
    if (suffixes.length === 0) return;
    const lcp = longestCommonPrefix(suffixes);

    if (lcp.length > 0) await sendInput(lcp);
    const newBase = line + lcp;

    // Filter to candidates whose suffix is strictly longer than the
    // LCP — those are the ones still worth picking. If only one
    // remains and we already injected its full tail via LCP, we're
    // done; close silently.
    const remaining = items.filter((it) => {
      const s = appendSuffixFor(it, line, wordStart);
      return s !== null && s.length > lcp.length;
    });
    if (remaining.length === 0) return;

    setCompletion({
      open: true,
      items,
      filtered: remaining,
      selectedIndex: 0,
      basePrefix: newBase,
      baseWordStart: wordStart,
    });
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    const mod = event.ctrlKey || event.metaKey;
    const hasSelection = hasCopyableSelection();

    // M4: completion popover key handling. Highest precedence — we
    // own arrow / Enter / Tab / Esc while the popover is open, so
    // those bytes never reach the underlying shell readline (which
    // would otherwise scroll its history or submit the line).
    //
    // Tab here cycles the highlight (warp / fish menu-complete) AND
    // injects the highlighted candidate's bytes into the PTY so the
    // line tracks the selection in real time. Shift+Tab cycles back.
    // Tab / arrows cycle the popover visually only — PTY isn't
    // touched until Enter. Esc closes without rolling back (LCP
    // we auto-completed on Tab and any chars the user typed
    // afterwards both stay).
    if (completion.open) {
      if (event.key === "ArrowDown" || (event.key === "Tab" && !event.shiftKey)) {
        event.preventDefault();
        cycleSelection(1);
        return;
      }
      if (event.key === "ArrowUp" || (event.key === "Tab" && event.shiftKey)) {
        event.preventDefault();
        cycleSelection(-1);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        closeCompletion();
        return;
      }
      if (event.key === "Enter") {
        event.preventDefault();
        acceptCompletion();
        return;
      }
      // Any other key — fall through to the normal handler so the
      // user can keep typing and the popover narrows in real time.
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
    //
    // We additionally require the caret to actually be at end-of-line
    // (`cursorAtBufferEndRef`). Without this, a user who used ←/→ to
    // step back into the line and intends ArrowRight as "move caret
    // right" would instead inject the gray ghost text into the PTY at
    // the mid-line position — overwriting whatever they typed past
    // that point. Caret mid-line ⇒ pass through as cursor movement.
    if (
      smartActiveRef.current
      && suggestionSuffixRef.current
      && cursorAtBufferEndRef.current
    ) {
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
        if (text) void sendInput(normalizeTerminalCommandText(text.replace(/\r?\n/g, "\r")));
      });
      return;
    }

    if (mod && !event.altKey && event.key.toLowerCase() === "c" && hasSelection) {
      event.preventDefault();
      void copySelection();
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
      payload = normalizeTerminalCommandText(event.key);
    }

    if (!payload) return;
    event.preventDefault();
    void sendInput(payload);
  }

  function updateTerminalSelectionFocus(clientX: number, clientY: number, viewOffset = snapshotViewOffsetRef.current) {
    const active = selectionDragRef.current;
    if (!active) return;
    const focus = terminalPointFromClient(clientX, clientY, viewOffset);
    if (!focus) return;
    const next = { anchor: active.anchor, focus };
    selectionDragRef.current = next;
    setTerminalSelection(next);
  }

  // Start a model-driven selection drag. `anchor` is the fixed end;
  // `focus` is the initial moving end (equal to anchor for a fresh
  // click, or the clicked point for a Shift-extend that preserves the
  // existing anchor). Window-level capture listeners keep the drag alive
  // when the pointer leaves the screen box.
  function beginTerminalSelectionDrag(
    anchor: TerminalSelectionPoint,
    focus: TerminalSelectionPoint,
  ) {
    selectionDragCleanupRef.current?.();
    const initial = { anchor, focus };
    selectionDragRef.current = initial;
    setTerminalSelection(terminalSelectionHasText(initial) ? initial : null);

    let cleanup = () => {};
    const onMouseMove = (moveEvent: MouseEvent) => {
      if ((moveEvent.buttons & 1) === 0) {
        onMouseUp(moveEvent);
        return;
      }
      updateTerminalSelectionFocus(moveEvent.clientX, moveEvent.clientY);
    };
    const onMouseUp = (upEvent: MouseEvent) => {
      updateTerminalSelectionFocus(upEvent.clientX, upEvent.clientY);
      const finalSelection = selectionDragRef.current;
      cleanup();
      selectionDragRef.current = null;
      setTerminalSelection(terminalSelectionHasText(finalSelection) ? finalSelection : null);
    };
    cleanup = () => {
      window.removeEventListener("mousemove", onMouseMove, true);
      window.removeEventListener("mouseup", onMouseUp, true);
      if (selectionDragCleanupRef.current === cleanup) {
        selectionDragCleanupRef.current = null;
      }
    };
    selectionDragCleanupRef.current = cleanup;
    window.addEventListener("mousemove", onMouseMove, true);
    window.addEventListener("mouseup", onMouseUp, true);
  }

  function handleTerminalSelectionMouseDown(event: React.MouseEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    // Modifiers other than Shift are reserved (cursor-position click,
    // future column-select) — leave the selection untouched.
    if (event.metaKey || event.ctrlKey || event.altKey) return;

    // Shift+click / Shift+drag extends the current selection: keep the
    // existing anchor (or start one at the click point) and move focus.
    if (event.shiftKey) {
      const point = terminalPointFromClient(event.clientX, event.clientY);
      if (!point) return;
      const existing = terminalSelectionRef.current;
      beginTerminalSelectionDrag(existing ? existing.anchor : point, point);
      return;
    }

    // Double-click selects the word under the cursor; triple-click the
    // whole row. Both are terminal-modeled (native selection is disabled
    // on the screen), so there is nothing to defer to the browser.
    if (event.detail === 2) {
      const word = wordSelectionAt(event.clientX, event.clientY);
      if (word) setTerminalSelection(word);
      return;
    }
    if (event.detail >= 3) {
      const line = lineSelectionAt(event.clientX, event.clientY);
      if (line) setTerminalSelection(line);
      return;
    }

    const anchor = terminalPointFromClient(event.clientX, event.clientY);
    if (!anchor) return;
    beginTerminalSelectionDrag(anchor, anchor);
  }

  function handleWheel(event: React.WheelEvent<HTMLDivElement>) {
    if (!snapshot?.scrollbackLen) return;
    event.preventDefault();
    const step = Math.max(1, Math.round(Math.abs(event.deltaY) / 36));
    const { clientX, clientY, deltaY } = event;
    // Functional updater: two wheel events can land between renders,
    // and a render-captured offset would lose the first step.
    setScrollbackOffset((prev) => {
      const scrollbackLen = snapshotRef.current?.scrollbackLen ?? 0;
      const next =
        deltaY < 0
          ? Math.min(prev + step, scrollbackLen)
          : Math.max(prev - step, 0);
      if (selectionDragRef.current) {
        updateTerminalSelectionFocus(clientX, clientY, next);
      }
      return next;
    });
  }

  /**
   * Click-to-position-cursor on the current input line. Translates
   * the click into a cell delta and emits enough cursor-left /
   * cursor-right escapes to land readline / fish / zsh on the chosen
   * column — sparing the user the "hold ←/→ to walk back" dance.
   *
   * Works whether OSC 133 prompt-end has been emitted yet or not
   * (russh sessions and shells whose first prompt has not yet drawn
   * are common cases). Bails inside TUI alt-screens so vim / htop /
   * less still own the mouse, and bails on a non-empty selection so
   * drag-to-copy still ships text to the clipboard untouched.
   */
  function handleScreenClick(event: React.MouseEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    if (event.shiftKey || event.metaKey || event.ctrlKey || event.altKey) return;
    if (!snapshot) return;
    // TUI alt-screens (vim, htop, less) own mouse events themselves —
    // don't synthesize keystrokes that would compete with their input.
    if (snapshot.altScreen) return;
    // A live model selection means the user just dragged/clicked to
    // select — don't also reposition the shell cursor on this click.
    if (terminalSelectionHasText(terminalSelectionRef.current)) return;

    const screen = event.currentTarget;
    const rect = screen.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;

    const { charWidth, rowHeight } = cellMetrics;
    if (charWidth <= 0 || rowHeight <= 0) return;

    // Round to the nearest cell boundary so a click between two glyphs
    // lands on the closer one (matches the IME / native textfield feel).
    const targetCol = Math.max(0, Math.round(x / charWidth));
    const targetRow = Math.floor(y / rowHeight);

    const cols = snapshot.cols || 1;

    // Two paths:
    //   * Precise — OSC 133;B told us where input begins, so we can
    //     map any cell on the input row(s) back to a buffer offset
    //     and handle wrapped multi-row prompts.
    //   * Fallback — no prompt-end (russh, shells without smart-mode
    //     rcfile, fresh prompt before first OSC 133;B fires). Honour
    //     clicks on the same row as the live cursor and use cursorX
    //     as the anchor; readline clamps overshoot at the prompt
    //     boundary so we don't have to know exactly where input begins.
    let delta: number;
    if (snapshot.promptEnd) {
      const [startRow, startCol] = snapshot.promptEnd;
      if (targetRow < startRow) return;
      if (targetRow === startRow && targetCol < startCol) return;
      const targetOff = targetRow === startRow
        ? targetCol - startCol
        : (cols - startCol) + (targetRow - startRow - 1) * cols + targetCol;
      const currentOff = snapshot.cursorY === startRow
        ? Math.max(0, snapshot.cursorX - startCol)
        : (cols - startCol) + (snapshot.cursorY - startRow - 1) * cols + snapshot.cursorX;
      delta = targetOff - currentOff;
    } else {
      if (targetRow !== snapshot.cursorY) return;
      delta = targetCol - snapshot.cursorX;
    }

    if (delta === 0) return;

    event.preventDefault();
    const seq = delta > 0 ? "\u001b[C".repeat(delta) : "\u001b[D".repeat(-delta);
    void sendInput(seq);
  }

  async function restartTerminal() {
    if (session) {
      await cmd.terminalClose(session.sessionId).catch(() => {});
    }
    setSession(null);
    setSnapshot(null);
    setError("");
    setNeedsPasswordRecovery(false);
    setScrollbackOffset(0);
    setSnapshotViewOffset(0);
    setTerminalSelection(null);
    createRetryCountRef.current = 0;
    setCreateAttempt((attempt) => attempt + 1);
  }

  async function copySelection() {
    const sel = await getSelectionTextForCopy();
    if (!sel) return;
    await writeClipboardText(sel);
  }

  async function pasteClipboard() {
    if (!session) return;
    const text = await readClipboardText();
    if (text) {
      try {
        await sendInput(normalizeTerminalCommandText(text.replace(/\r?\n/g, "\r")));
      } catch {
        /* PTY write blocked */
      }
    }
  }

  function selectAllInTerminal() {
    const view = snapshotRef.current;
    if (!view || view.lines.length === 0) return;
    const visibleStart = visibleStartForSnapshot(view, snapshotViewOffsetRef.current);
    setTerminalSelection({
      anchor: { row: visibleStart, col: 0 },
      focus: { row: visibleStart + view.lines.length - 1, col: view.cols },
    });
  }

  // ── "Ask AI" bridge (§5.14.5) ────────────────────────────────
  // Stage terminal text as a pending attachment on this tab's AI
  // conversation and switch the right tool to the assistant. The
  // attachment is visible (and removable) in the panel before
  // anything is sent.

  function openAiWithAttachment(label: string, content: string) {
    if (!content.trim()) return;
    useAiStore.getState().addPendingAttachment(tab.id, { label, content });
    useTabStore.getState().setTabRightTool(tab.id, "ai");
  }

  async function askAiAboutSelection() {
    const text = await getSelectionTextForCopy();
    openAiWithAttachment(t("terminal selection"), text);
  }

  function askAiAboutScreen() {
    const snap = snapshotRef.current;
    if (!snap) return;
    const text = snap.lines
      .map((l) => copySliceTerminalLine(l, 0, terminalLineCells(l)))
      .join("\n")
      .trimEnd();
    openAiWithAttachment(t("terminal output (visible screen)"), text);
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
              ? `${tab.currentShellUser || tab.sshUser}@${tab.sshHost}`
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
        className={[
          "terminal-viewport",
          visualBellActive ? "terminal-viewport--bell" : "",
          activating ? "terminal-viewport--activating" : "",
          selectingInTerminal ? "terminal-viewport--selecting" : "",
        ].filter(Boolean).join(" ")}
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
            onClick={handleScreenClick}
            onMouseDown={handleTerminalSelectionMouseDown}
            ref={screenRef}
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
              <TerminalRow key={`line-${i}`} line={line} env={rowEnv} rowIndex={i} />
            ))}
            {selectionRects.length > 0 && (
              <div className="terminal-selection-layer" aria-hidden>
                {selectionRects.map((rect) => (
                  <div
                    key={`sel-${rect.row}`}
                    className="terminal-selection-rect"
                    style={{
                      top: rect.top,
                      left: rect.left,
                      width: rect.width,
                      height: rect.height,
                    }}
                  />
                ))}
              </div>
            )}
            {smartActive &&
              !mirrorDesynced &&
              snapshot.promptEnd &&
              (smartLineBufferText || suggestionSuffix) && (
                <TerminalSyntaxOverlay
                  text={smartLineBufferText}
                  promptEnd={snapshot.promptEnd}
                  charWidth={cellMetrics.charWidth}
                  rowHeight={cellMetrics.rowHeight}
                  bgColor={termTheme.bg}
                  suggestionSuffix={suggestionSuffix}
                  fontFamily={`"${monoFont}", monospace`}
                />
              )}
            {smartActive && (() => {
              // Prefer the OSC 133 prompt-end + mirror buffer length
              // when available — that's the most precise (it tracks
              // wrapped prompts correctly). Fall back to the live
              // cursor coords from the snapshot so russh tabs and
              // any other shell that doesn't emit OSC 133 still get
              // the popover anchored at the cursor instead of
              // floating in the middle of the viewport.
              const [row, col] = snapshot.promptEnd
                ? [
                    snapshot.promptEnd[0],
                    // Columns, not chars — CJK input occupies 2 cells.
                    snapshot.promptEnd[1] + textCols(smartLineBufferText),
                  ]
                : [snapshot.cursorY, snapshot.cursorX];
              return (
                <div
                  ref={cursorAnchorRef}
                  aria-hidden
                  style={{
                    position: "absolute",
                    top: row * cellMetrics.rowHeight,
                    left: col * cellMetrics.charWidth,
                    width: 0,
                    height: cellMetrics.rowHeight,
                    pointerEvents: "none",
                  }}
                />
              );
            })()}
          </div>
        ) : (
          <div className="terminal-placeholder">{t("Launching shell...")}</div>
        )}
      </div>

      {ctxMenu && (() => {
        const hasSelection = hasCopyableSelection();
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
            label: t("Ask AI about selection"),
            disabled: !hasSelection,
            action: () => void askAiAboutSelection(),
          },
          {
            label: t("Ask AI about screen output"),
            disabled: !session,
            action: askAiAboutScreen,
          },
          { divider: true },
          {
            label: t("Restart terminal"),
            action: () => void restartTerminal(),
          },
        ];
        // Cap the inline snippet count to keep the menu navigable —
        // anything beyond the first 12 lives behind the manager.
        const SNIPPETS_INLINE_CAP = 12;
        const visible = snippets.slice(0, SNIPPETS_INLINE_CAP);
        if (visible.length > 0 || snippets.length === 0) {
          items.push({ divider: true });
          items.push({ section: t("Snippets") });
          for (const s of visible) {
            items.push({
              label: snippetDisplayLabel(s),
              disabled: !session,
              action: () => void pasteSnippet(s),
            });
          }
          if (snippets.length > SNIPPETS_INLINE_CAP) {
            items.push({
              label: t("(+{n} more — open manager)", {
                n: snippets.length - SNIPPETS_INLINE_CAP,
              }),
              action: () => setSnippetsDialogOpen(true),
            });
          }
          items.push({
            label:
              snippets.length === 0
                ? t("Add snippet…")
                : t("Manage snippets…"),
            action: () => setSnippetsDialogOpen(true),
          });
        }
        return (
          <ContextMenu
            x={ctxMenu.x}
            y={ctxMenu.y}
            items={items}
            onClose={() => setCtxMenu(null)}
          />
        );
      })()}

      {snippetsDialogOpen && (
        <TerminalSnippetsDialog
          snippets={snippets}
          onClose={() => setSnippetsDialogOpen(false)}
          onChange={commitSnippets}
        />
      )}

      <CompletionPopover
        open={completion.open}
        anchor={cursorAnchorRef.current}
        items={completion.filtered}
        selectedIndex={completion.selectedIndex}
        onHighlight={(idx) =>
          setCompletion((s) => ({ ...s, selectedIndex: idx }))
        }
        onSelect={(_item, idx) => {
          // Click = highlight this row + commit (same as Enter on it).
          setCompletion((s) => ({ ...s, selectedIndex: idx }));
          // Defer to next tick so the highlight state lands before
          // accept reads it.
          queueMicrotask(() => acceptCompletion());
        }}
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

/** Per-user manager for terminal snippets. Lives behind the
 *  context-menu "Manage snippets…" item. CRUD over the localStorage-
 *  persisted list — no SSH, no IPC, just React state. Closes via the
 *  overlay click / Esc / explicit Done button. */
function TerminalSnippetsDialog({
  snippets,
  onChange,
  onClose,
}: {
  snippets: TerminalSnippet[];
  onChange: (next: TerminalSnippet[]) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();

  function update(idx: number, patch: Partial<TerminalSnippet>) {
    onChange(snippets.map((s, i) => (i === idx ? { ...s, ...patch } : s)));
  }
  function remove(idx: number) {
    onChange(snippets.filter((_, i) => i !== idx));
  }
  function add() {
    onChange([
      ...snippets,
      { id: makeSnippetId(), label: "", command: "" },
    ]);
  }

  return (
    <div className="dlg-overlay" onMouseDown={shakeDialogOverlay}>
      <div className="dlg" onClick={(e) => e.stopPropagation()}>
        <div className="dlg-head">
          <span className="dlg-title">{t("Terminal snippets")}</span>
          <div style={{ flex: 1 }} />
          <span className="muted mono" style={{ fontSize: "var(--size-micro)" }}>
            {t(
              "Saved per-user. Right-click the terminal to paste a snippet.",
            )}
          </span>
        </div>
        <div className="dlg-body dlg-body--form">
          {snippets.length === 0 && (
            <div className="status-note mono">
              {t(
                "No snippets yet. Add one — e.g. `journalctl -u nginx -f` or `docker compose logs -f --tail=200`.",
              )}
            </div>
          )}
          {snippets.map((s, i) => (
            <div key={s.id} className="term-snip-row">
              <div className="term-snip-row__head">
                <input
                  className="dlg-input"
                  placeholder={t("Label (optional)")}
                  value={s.label}
                  onChange={(e) => update(i, { label: e.currentTarget.value })}
                />
                <label className="term-snip-row__flag mono">
                  <input
                    type="checkbox"
                    checked={!!s.runOnPaste}
                    onChange={(e) =>
                      update(i, { runOnPaste: e.currentTarget.checked })
                    }
                  />
                  <span>{t("Run on paste")}</span>
                </label>
                <button
                  type="button"
                  className="btn is-ghost is-compact"
                  onClick={() => remove(i)}
                  title={t("Remove")}
                >
                  ×
                </button>
              </div>
              <textarea
                className="term-snip-row__cmd mono"
                value={s.command}
                spellCheck={false}
                rows={2}
                placeholder={t("Command body")}
                onChange={(e) => update(i, { command: e.currentTarget.value })}
              />
            </div>
          ))}
          <div className="term-snip-add">
            <button
              type="button"
              className="btn is-ghost is-compact"
              onClick={add}
            >
              + {t("Add snippet")}
            </button>
          </div>
        </div>
        <div className="dlg-foot">
          <button
            type="button"
            className="btn is-primary is-compact"
            onClick={onClose}
          >
            {t("Done")}
          </button>
        </div>
      </div>
    </div>
  );
}

export default memo(TerminalPanel);
