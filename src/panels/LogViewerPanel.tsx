import {
  ChevronDown,
  ExternalLink,
  FolderTree,
  Play,
  RefreshCw,
  Server,
  Square,
  Terminal as TerminalIcon,
} from "lucide-react";
import type { ComponentType, SVGProps } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import * as cmd from "../lib/commands";
import { RIGHT_TOOL_META } from "../lib/rightToolMeta";
import {
  compileLogSource,
  compileLogSourceBackfill,
  describeLogSource,
  findPreset,
  isLogLikeFilename,
  LOG_SYSTEM_PRESETS,
  MODES,
} from "../lib/logSource";
import type { LogEventView, LogSource, LogSourceMode, SftpEntryView, TabState } from "../lib/types";
import { DEFAULT_LOG_SOURCE, effectiveSshTarget } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError, localizeRuntimeMessage } from "../i18n/localizeMessage";
import DismissibleNote from "../components/DismissibleNote";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";
import LogViewerDialog from "../shell/LogViewerDialog";
import { useTabStore } from "../stores/useTabStore";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";

type Props = { tab: TabState };
type IconType = ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;

const MAX_EVENTS = 600;

type LogLevel = "info" | "warn" | "error" | "debug";

type Enriched = {
  idx: number;
  kind: LogEventView["kind"];
  text: string;
  level: LogLevel;
  ts: string;
};

function detectLevel(kind: LogEventView["kind"], text: string): LogLevel {
  if (kind === "error") return "error";
  const upper = text.slice(0, 120).toUpperCase();
  if (/\b(ERROR|ERR|FATAL|PANIC)\b/.test(upper)) return "error";
  if (/\b(WARN|WARNING)\b/.test(upper)) return "warn";
  if (/\b(DEBUG|TRACE)\b/.test(upper)) return "debug";
  if (kind === "stderr") return "warn";
  return "info";
}

function clockStamp(d: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

const LOG_ICON = RIGHT_TOOL_META.log.icon;

const MODE_ICONS: Record<LogSourceMode, IconType> = {
  file: FolderTree,
  system: Server,
  custom: TerminalIcon,
};

export default function LogViewerPanel(props: Props) {
  const ready = useDeferredMount();
  return (
    <div className="panel-stage">
      {ready ? <LogViewerPanelBody {...props} /> : <PanelSkeleton variant="rows" rows={8} />}
    </div>
  );
}

function LogViewerPanelBody({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const updateTab = useTabStore((s) => s.updateTab);

  const source: LogSource = tab.logSource ?? DEFAULT_LOG_SOURCE;
  const preset = source.mode === "system" ? findPreset(source.systemPresetId) : undefined;

  const [streamId, setStreamId] = useState<string | null>(null);
  const [events, setEvents] = useState<Enriched[]>([]);
  const [busy, setBusy] = useState(false);
  const [backfillBusy, setBackfillBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [follow, setFollow] = useState(true);
  /** When true, the rich LogViewerDialog renders alongside the inline tail.
   *  Filters / search / line detail live in the dialog — the panel itself is
   *  a thin tail viewer. */
  const [dialogOpen, setDialogOpen] = useState(false);

  // File-mode draft state: dir input + fetched entries.
  const [fileDirDraft, setFileDirDraft] = useState(source.fileDir || "/var/log");
  const [fileList, setFileList] = useState<SftpEntryView[]>([]);
  const [scanBusy, setScanBusy] = useState(false);
  const [scanError, setScanError] = useState("");

  // Custom-mode draft textarea — only applied when the user clicks Apply.
  const [customDraft, setCustomDraft] = useState(source.customCommand || "");

  const outputRef = useRef<HTMLDivElement | null>(null);
  const counter = useRef(0);
  // Lines/sec EMA — driven from each non-empty drain. We weight the
  // most recent sample at 30% so spikes show up quickly but fast
  // enough to settle on idle. `lastDrainAt` is the wall clock of
  // the previous successful drain; the gap is the EMA's denominator.
  const lastDrainAt = useRef<number | null>(null);
  const rateEma = useRef(0);
  const [linesPerSecond, setLinesPerSecond] = useState(0);

  // Accept SSH context inferred from a local terminal that ran `ssh
  // user@host` or from a nested-ssh overlay on top of a real SSH tab.
  const sshTarget = effectiveSshTarget(tab);
  const hasSsh = sshTarget !== null;
  const sshArgs = {
    host: sshTarget?.host ?? "",
    port: sshTarget?.port ?? 22,
    user: sshTarget?.user ?? "",
    authMode: sshTarget?.authMode ?? "password",
    password: sshTarget?.password ?? "",
    keyPath: sshTarget?.keyPath ?? "",
    savedConnectionIndex: sshTarget?.savedConnectionIndex ?? null,
  };

  useEffect(() => {
    setFileDirDraft(source.fileDir || "/var/log");
  }, [source.fileDir]);
  useEffect(() => {
    setCustomDraft(source.customCommand || "");
  }, [source.customCommand]);

  useEffect(() => {
    if (!follow) return;
    const viewport = outputRef.current;
    if (!viewport) return;
    viewport.scrollTop = viewport.scrollHeight;
  }, [events, follow]);

  function patchSource(patch: Partial<LogSource>) {
    updateTab(tab.id, { logSource: { ...source, ...patch } });
  }

  function setMode(mode: LogSourceMode) {
    if (source.mode === mode) return;
    patchSource({ mode });
  }

  async function stopStream(targetId?: string | null) {
    const resolvedId = targetId ?? streamId;
    if (!resolvedId) return;
    await cmd.logStreamStop(resolvedId).catch(() => {});
    setStreamId((current) => (current === resolvedId ? null : current));
  }

  /** Run a one-shot historical fetch for the last `windowMinutes`.
   *  Stops the live stream (if any), starts a new finite stream with
   *  the back-fill command, and lets the existing drain loop pull the
   *  events. The exit event from the finite command will naturally
   *  flip `streaming` to false at the end. We don't auto-restart the
   *  live stream — the user backfilled to inspect, restarting tail -F
   *  would scroll them away from what they just loaded. */
  async function runBackfill(windowMinutes: number) {
    if (!hasSsh || backfillBusy) return;
    const command = compileLogSourceBackfill(source, windowMinutes);
    if (!command) {
      setError(t("This source can't be back-filled."));
      return;
    }
    setBackfillBusy(true);
    setError("");
    setNotice("");
    if (streamId) {
      await stopStream(streamId);
    }
    try {
      const nextId = await cmd.logStreamStart({
        host: sshArgs.host,
        port: sshArgs.port,
        user: sshArgs.user,
        authMode: sshArgs.authMode,
        password: sshArgs.password,
        keyPath: sshArgs.keyPath,
        command,
        savedConnectionIndex: sshArgs.savedConnectionIndex,
      });
      setEvents([]);
      counter.current = 0;
      rateEma.current = 0;
      lastDrainAt.current = null;
      setLinesPerSecond(0);
      setStreamId(nextId);
      setNotice(
        t("Backfilling {n} min of history.", { n: windowMinutes }),
      );
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBackfillBusy(false);
    }
  }

  async function startStream() {
    if (!hasSsh) return;
    const command = compileLogSource(source);
    if (!command) {
      setError(t("Select a log source before starting."));
      return;
    }
    setBusy(true);
    setError("");
    setNotice("");
    if (streamId) {
      await stopStream(streamId);
    }
    try {
      const nextId = await cmd.logStreamStart({
        host: sshArgs.host,
        port: sshArgs.port,
        user: sshArgs.user,
        authMode: sshArgs.authMode,
        password: sshArgs.password,
        keyPath: sshArgs.keyPath,
        command,
        savedConnectionIndex: sshArgs.savedConnectionIndex,
      });
      updateTab(tab.id, { logCommand: command });
      setEvents([]);
      counter.current = 0;
      rateEma.current = 0;
      lastDrainAt.current = null;
      setLinesPerSecond(0);
      setStreamId(nextId);
      setNotice(t("Streaming remote command."));
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    if (!streamId) return;

    // Adaptive drain cadence. A busy stream (new bytes on every tick) stays
    // at 200ms for low latency; an idle stream backs off exponentially to
    // 2s so we don't burn IPC on silence. Each non-empty drain resets to
    // the fast tier — the loop keeps feel snappy once logs start flowing.
    const MIN_MS = 200;
    const MAX_MS = 2000;

    let disposed = false;
    let timerId: number | null = null;
    let delay = MIN_MS;

    const schedule = () => {
      if (disposed) return;
      timerId = window.setTimeout(run, delay);
    };

    const run = () => {
      cmd.logStreamDrain(streamId)
        .then((batch) => {
          if (disposed) return;

          if (batch.length === 0) {
            delay = Math.min(delay * 2, MAX_MS);
            schedule();
            // Decay the rate so a quiet stream's chip eventually
            // drops to zero — otherwise a 10-line burst sticks at
            // its peak forever.
            rateEma.current *= 0.7;
            setLinesPerSecond(rateEma.current);
            return;
          }
          delay = MIN_MS;
          // Update the lines/sec EMA. `intervalMs` is the wall-clock
          // gap since the last non-empty drain (or this drain's
          // interval if this is the first).
          const drainAt = Date.now();
          const intervalMs = lastDrainAt.current
            ? Math.max(50, drainAt - lastDrainAt.current)
            : delay;
          lastDrainAt.current = drainAt;
          const sample = (batch.length * 1000) / intervalMs;
          rateEma.current = rateEma.current * 0.7 + sample * 0.3;
          setLinesPerSecond(rateEma.current);

          const now = clockStamp();
          setEvents((current) => {
            const appended = batch.map<Enriched>((b) => ({
              idx: ++counter.current,
              kind: b.kind,
              text: b.text,
              level: detectLevel(b.kind, b.text),
              ts: now,
            }));
            return [...current, ...appended].slice(-MAX_EVENTS);
          });

          const terminalEvent = batch.find((entry) => entry.kind === "exit" || entry.kind === "error");
          if (terminalEvent) {
            if (terminalEvent.kind === "exit") {
              setNotice(t("Log stream exited with code {code}.", { code: terminalEvent.text }));
            } else {
              setError(localizeRuntimeMessage(terminalEvent.text || t("Log stream ended with an error."), t));
            }
            void stopStream(streamId);
            return;
          }
          schedule();
        })
        .catch((drainError) => {
          if (disposed) return;
          setError(formatError(drainError));
          void stopStream(streamId);
        });
    };

    run();
    return () => {
      disposed = true;
      if (timerId !== null) window.clearTimeout(timerId);
    };
  }, [streamId]);

  useEffect(() => () => {
    if (streamId) {
      void cmd.logStreamStop(streamId).catch(() => {});
    }
  }, [streamId]);

  async function scanFileDir() {
    if (!hasSsh) return;
    const dir = fileDirDraft.trim() || "/var/log";
    setScanBusy(true);
    setScanError("");
    try {
      const result = await cmd.sftpBrowse({
        host: sshArgs.host,
        port: sshArgs.port,
        user: sshArgs.user,
        authMode: sshArgs.authMode,
        password: sshArgs.password,
        keyPath: sshArgs.keyPath,
        path: dir,
        savedConnectionIndex: sshArgs.savedConnectionIndex,
      });
      const logs = result.entries
        .filter((e) => !e.isDir && isLogLikeFilename(e.name))
        .sort((a, b) => a.name.localeCompare(b.name));
      setFileList(logs);
      patchSource({ fileDir: result.currentPath });
      if (logs.length === 0) {
        setScanError(t("No log-like files in {dir}.", { dir: result.currentPath }));
      }
    } catch (e) {
      setScanError(formatError(e));
      setFileList([]);
    } finally {
      setScanBusy(false);
    }
  }

  /** The inline panel always shows the most recent 200 lines unfiltered.
   *  Heavy filtering / search / line detail live in LogViewerDialog. */
  const tailEvents = useMemo(() => events.slice(-200), [events]);

  const streaming = !!streamId;
  const compiled = compileLogSource(source);
  const canStart = hasSsh && compiled.length > 0;

  const headerMeta = streaming
    ? linesPerSecond >= 0.5
      ? t("{count} lines · {rate} l/s", {
          count: events.length,
          rate: linesPerSecond < 10
            ? linesPerSecond.toFixed(1)
            : Math.round(linesPerSecond),
        })
      : t("{count} lines · streaming", { count: events.length })
    : events.length > 0
      ? t("{count} lines", { count: events.length })
      : undefined;

  const SourceIcon = MODE_ICONS[source.mode];

  return (
    <>
      <PanelHeader icon={LOG_ICON} title={t("Logs")} meta={headerMeta} />
      <div className="lg">
        {/* Primary picker row: mode segment + streaming indicator + Start/Stop */}
        <div className="lg-picker">
          <div className="lg-seg" role="tablist" aria-label={t("SOURCE")}>
            {MODES.map((m) => {
              const Icon = MODE_ICONS[m.id];
              const on = source.mode === m.id;
              return (
                <button
                  key={m.id}
                  type="button"
                  role="tab"
                  aria-selected={on}
                  className={"lg-seg-item" + (on ? " on" : "")}
                  onClick={() => setMode(m.id)}
                  title={t(m.label)}
                >
                  <Icon size={11} />
                  <span>{t(m.label)}</span>
                </button>
              );
            })}
          </div>

          <div className="lg-picker-spacer" />

          <span className={"lg-status " + (streaming ? "on" : "off")}>
            <StatusDot tone={streaming ? "pos" : "off"} />
            {streaming ? t("streaming") : t("paused")}
          </span>
          <button
            type="button"
            className={"btn is-compact" + (streaming ? " is-danger" : " is-primary")}
            disabled={!streaming && (!canStart || busy)}
            onClick={() => (streaming ? void stopStream() : void startStream())}
          >
            {streaming ? <Square size={10} /> : <Play size={10} />}
            {streaming ? t("Stop") : busy ? t("Starting...") : t("Start")}
          </button>
        </div>

        {/* Backfill row — one-shot historical fetch over the chosen
            window. Disabled when the source can't sensibly back-fill
            (custom command or empty file path). */}
        {(() => {
          const canBackfill =
            !!hasSsh &&
            !backfillBusy &&
            compileLogSourceBackfill(source, 1).length > 0;
          return (
            <div className="lg-backfill mono">
              <span className="muted">{t("Backfill")}:</span>
              {[
                { mins: 1, label: "1m" },
                { mins: 15, label: "15m" },
                { mins: 60, label: "1h" },
                { mins: 1440, label: "24h" },
              ].map((b) => (
                <button
                  key={b.label}
                  type="button"
                  className="btn is-ghost is-compact"
                  disabled={!canBackfill}
                  onClick={() => void runBackfill(b.mins)}
                  title={t("Run a one-shot historical fetch for the last {n}", {
                    n: b.label,
                  })}
                >
                  {b.label}
                </button>
              ))}
              {backfillBusy && (
                <span className="muted">{t("Backfilling…")}</span>
              )}
            </div>
          );
        })()}

        {/* Secondary row — changes by mode */}
        {source.mode === "file" && (
          <div className="lg-picker lg-picker--sub">
            <div className="lg-pick">
              <label>{t("DIR")}</label>
              <div className="lg-sel lg-sel--input">
                <SourceIcon size={11} />
                <input
                  type="text"
                  value={fileDirDraft}
                  onChange={(e) => setFileDirDraft(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void scanFileDir();
                  }}
                  placeholder="/var/log"
                  spellCheck={false}
                />
              </div>
            </div>
            <button
              type="button"
              className="btn is-ghost is-compact"
              onClick={() => void scanFileDir()}
              disabled={!hasSsh || scanBusy}
              title={t("Scan directory")}
            >
              <RefreshCw size={10} />
              {scanBusy ? t("Scanning...") : t("Scan")}
            </button>
            <div className="lg-pick lg-pick--grow">
              <label>{t("FILE")}</label>
              <div className="lg-sel">
                <select
                  className="lg-sel-native"
                  value={source.filePath}
                  onChange={(e) => patchSource({ filePath: e.currentTarget.value })}
                >
                  <option value="">
                    {fileList.length === 0 ? t("(scan to list files)") : t("(choose a file)")}
                  </option>
                  {fileList.map((f) => (
                    <option key={f.path} value={f.path}>
                      {f.name}
                    </option>
                  ))}
                </select>
                <ChevronDown size={10} />
              </div>
            </div>
          </div>
        )}

        {source.mode === "system" && (
          <div className="lg-picker lg-picker--sub">
            <div className="lg-pick lg-pick--grow">
              <label>{t("PRESET")}</label>
              <div className="lg-sel">
                <SourceIcon size={11} />
                <select
                  className="lg-sel-native"
                  value={source.systemPresetId}
                  onChange={(e) => patchSource({ systemPresetId: e.currentTarget.value, systemArg: "" })}
                >
                  {LOG_SYSTEM_PRESETS.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.label}
                    </option>
                  ))}
                </select>
                <ChevronDown size={10} />
              </div>
            </div>
            {preset?.argLabel && (
              <div className="lg-pick lg-pick--grow">
                <label>{preset.argLabel}</label>
                <div className="lg-sel lg-sel--input">
                  <input
                    type="text"
                    value={source.systemArg}
                    onChange={(e) => patchSource({ systemArg: e.currentTarget.value })}
                    placeholder={preset.argPlaceholder || ""}
                    spellCheck={false}
                  />
                </div>
              </div>
            )}
            <span className="lg-picker-hint mono" title={compiled || t("(incomplete)")}>
              {compiled || t("(incomplete)")}
            </span>
          </div>
        )}

        {source.mode === "custom" && (
          <div className="lg-cmd-editor">
            <textarea
              className="field-textarea field-textarea--editor"
              rows={2}
              value={customDraft}
              onChange={(e) => setCustomDraft(e.currentTarget.value)}
              placeholder="tail -F /var/log/syslog"
              spellCheck={false}
            />
            <button
              type="button"
              className="btn is-ghost is-compact"
              disabled={customDraft.trim() === (source.customCommand || "").trim()}
              onClick={() => patchSource({ customCommand: customDraft.trim() })}
            >
              {t("Apply")}
            </button>
          </div>
        )}

        {/* Slim toolbar — wrap toggle + open-in-dialog button. Filters /
            search / line detail moved to LogViewerDialog. */}
        <div className="lg-filters lg-filters--slim">
          {hasSsh && events.length > 0 && (
            <span className="lg-counts mono">
              {t("{count} lines buffered", { count: events.length })}
            </span>
          )}
          <span className="lg-picker-spacer" />
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={() => setDialogOpen(true)}
            disabled={events.length === 0}
            title={t("Open in dialog")}
          >
            <ExternalLink size={11} />
            {t("Open in dialog")}
          </button>
        </div>

        <div
          className={"lg-body mono wrap"}
          ref={outputRef}
          onScroll={(e) => {
            const el = e.currentTarget;
            const atBottom = el.scrollHeight - el.clientHeight - el.scrollTop < 4;
            if (atBottom !== follow) setFollow(atBottom);
          }}
        >
          {!hasSsh && (
            <div className="lg-note">{t("SSH connection required.")}</div>
          )}
          {hasSsh && !compiled && (
            <div className="lg-note">{t("Pick a source above, then press Start.")}</div>
          )}
          {hasSsh && source.mode === "file" && scanError && (
            <DismissibleNote onDismiss={() => setScanError("")}>{scanError}</DismissibleNote>
          )}
          {hasSsh && compiled && events.length === 0 && !streaming && (
            <div className="lg-note mono">
              <span className="text-muted">{t("ready:")} </span>
              {compiled}
            </div>
          )}
          {notice && (
            <DismissibleNote onDismiss={() => setNotice("")}>{notice}</DismissibleNote>
          )}
          {error && (
            <DismissibleNote tone="error" onDismiss={() => setError("")}>
              {error}
            </DismissibleNote>
          )}

          {tailEvents.map((e) => (
            <div key={e.idx} className={"lg-line lv-" + e.level}>
              <span className="lg-n">{String(e.idx).padStart(4, " ")}</span>
              <span className="lg-t">{e.ts}</span>
              <span className={"lg-lvl " + e.level}>{e.level.toUpperCase()}</span>
              <span className="lg-msg">{e.kind === "exit" ? t("Process exited with code {code}", { code: e.text }) : e.text}</span>
            </div>
          ))}
          {streaming && tailEvents.length > 0 && (
            <div className="lg-line lg-line--cursor">
              <span className="lg-n">{String(counter.current + 1).padStart(4, " ")}</span>
              <span className="lg-cursor" />
            </div>
          )}
        </div>

        <div className="lg-foot">
          <span className="mono lg-foot-src" title={compiled || describeLogSource(source)}>
            <SourceIcon size={10} />
            {describeLogSource(source)}
          </span>
          <span className="lg-picker-spacer" />
          <button
            type="button"
            className={"lg-foot-pin mono" + (follow ? " active" : "")}
            onClick={() => {
              setFollow(true);
              const el = outputRef.current;
              if (el) el.scrollTop = el.scrollHeight;
            }}
          >
            ↓ {t("follow")}
          </button>
        </div>
      </div>

      <LogViewerDialog
        open={dialogOpen}
        onClose={() => setDialogOpen(false)}
        events={events}
        source={source}
        hostLabel={hasSsh ? `${sshArgs.user}@${sshArgs.host}` : ""}
        streaming={streaming}
        onToggleStreaming={() => (streaming ? void stopStream() : void startStream())}
        onClear={() => setEvents([])}
        compiledCommand={compiled}
      />
    </>
  );
}
