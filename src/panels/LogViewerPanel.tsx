import {
  AlignJustify,
  ChevronDown,
  Download,
  FolderTree,
  Play,
  RefreshCw,
  Search,
  Server,
  Square,
  Terminal as TerminalIcon,
  Trash2,
  X,
} from "lucide-react";
import type { ComponentType, SVGProps } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import * as cmd from "../lib/commands";
import { RIGHT_TOOL_META } from "../lib/rightToolMeta";
import {
  compileLogSource,
  describeLogSource,
  findPreset,
  isLogLikeFilename,
  LOG_SYSTEM_PRESETS,
  MODES,
} from "../lib/logSource";
import type { LogEventView, LogSource, LogSourceMode, SftpEntryView, TabState } from "../lib/types";
import { DEFAULT_LOG_SOURCE } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError, localizeRuntimeMessage } from "../i18n/localizeMessage";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";
import { useTabStore } from "../stores/useTabStore";

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

const LEVELS: { key: LogLevel; label: string }[] = [
  { key: "info", label: "INFO" },
  { key: "warn", label: "WARN" },
  { key: "error", label: "ERROR" },
  { key: "debug", label: "DEBUG" },
];

const LOG_ICON = RIGHT_TOOL_META.log.icon;

const MODE_ICONS: Record<LogSourceMode, IconType> = {
  file: FolderTree,
  system: Server,
  custom: TerminalIcon,
};

export default function LogViewerPanel({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const updateTab = useTabStore((s) => s.updateTab);

  const source: LogSource = tab.logSource ?? DEFAULT_LOG_SOURCE;
  const preset = source.mode === "system" ? findPreset(source.systemPresetId) : undefined;

  const [streamId, setStreamId] = useState<string | null>(null);
  const [events, setEvents] = useState<Enriched[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [wrap, setWrap] = useState(false);
  const [searchText, setSearchText] = useState("");
  const [activeLevels, setActiveLevels] = useState<Record<LogLevel, boolean>>({
    info: true,
    warn: true,
    error: true,
    debug: false,
  });
  const [follow, setFollow] = useState(true);

  // File-mode draft state: dir input + fetched entries.
  const [fileDirDraft, setFileDirDraft] = useState(source.fileDir || "/var/log");
  const [fileList, setFileList] = useState<SftpEntryView[]>([]);
  const [scanBusy, setScanBusy] = useState(false);
  const [scanError, setScanError] = useState("");

  // Custom-mode draft textarea — only applied when the user clicks Apply.
  const [customDraft, setCustomDraft] = useState(source.customCommand || "");

  const outputRef = useRef<HTMLDivElement | null>(null);
  const counter = useRef(0);

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();

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
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        command,
        savedConnectionIndex: tab.sshSavedConnectionIndex,
      });
      updateTab(tab.id, { logCommand: command });
      setEvents([]);
      counter.current = 0;
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

    let disposed = false;

    const drain = () => {
      cmd.logStreamDrain(streamId)
        .then((batch) => {
          if (disposed || batch.length === 0) return;

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
          }
        })
        .catch((drainError) => {
          if (disposed) return;
          setError(formatError(drainError));
          void stopStream(streamId);
        });
    };

    drain();
    const intervalId = window.setInterval(drain, 200);
    return () => {
      disposed = true;
      window.clearInterval(intervalId);
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
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
        path: dir,
        savedConnectionIndex: tab.sshSavedConnectionIndex,
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

  const counts = useMemo(() => {
    const acc: Record<LogLevel, number> = { info: 0, warn: 0, error: 0, debug: 0 };
    for (const e of events) acc[e.level]++;
    return acc;
  }, [events]);

  const filtered = useMemo(() => {
    const needle = searchText.trim().toLowerCase();
    return events.filter((e) => {
      if (!activeLevels[e.level]) return false;
      if (needle && !e.text.toLowerCase().includes(needle)) return false;
      return true;
    });
  }, [events, activeLevels, searchText]);

  const streaming = !!streamId;
  const compiled = compileLogSource(source);
  const canStart = hasSsh && compiled.length > 0;

  const headerMeta = streaming
    ? t("{count} lines · streaming", { count: events.length })
    : events.length > 0
      ? t("{count} lines", { count: events.length })
      : undefined;

  function downloadLog() {
    const lines = events.map((e) => `${e.ts} [${e.level.toUpperCase()}] ${e.text}`).join("\n");
    const blob = new Blob([lines], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `pier-log-${Date.now()}.log`;
    a.click();
    URL.revokeObjectURL(url);
  }

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

        {/* Filter toolbar */}
        <div className="lg-filters">
          <div className="lg-levels">
            {LEVELS.map((lv) => (
              <button
                key={lv.key}
                type="button"
                className={"lg-chip lv-" + lv.key + (activeLevels[lv.key] ? " on" : "")}
                onClick={() => setActiveLevels((prev) => ({ ...prev, [lv.key]: !prev[lv.key] }))}
              >
                <span className="lg-chip-dot" />
                {t(lv.label)}
                <span className="lg-chip-n">{counts[lv.key] || 0}</span>
              </button>
            ))}
          </div>

          <div className="lg-search">
            <Search size={10} />
            <input
              placeholder={t("Filter…")}
              value={searchText}
              onChange={(e) => setSearchText(e.currentTarget.value)}
            />
            {searchText ? (
              <button className="lg-x" type="button" onClick={() => setSearchText("")}>
                <X size={10} />
              </button>
            ) : null}
          </div>

          <button
            type="button"
            className={"lg-ic" + (wrap ? " on" : "")}
            title={t("Wrap lines")}
            onClick={() => setWrap((v) => !v)}
          >
            <AlignJustify size={11} />
          </button>
          <button
            type="button"
            className="lg-ic"
            title={t("Clear")}
            onClick={() => setEvents([])}
            disabled={events.length === 0}
          >
            <Trash2 size={11} />
          </button>
          <button
            type="button"
            className="lg-ic"
            title={t("Download")}
            onClick={downloadLog}
            disabled={events.length === 0}
          >
            <Download size={11} />
          </button>
        </div>

        <div
          className={"lg-body mono" + (wrap ? " wrap" : "")}
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
            <div className="lg-note">{scanError}</div>
          )}
          {hasSsh && compiled && events.length === 0 && !streaming && (
            <div className="lg-note mono">
              <span className="text-muted">{t("ready:")} </span>
              {compiled}
            </div>
          )}
          {notice && <div className="lg-note">{notice}</div>}
          {error && <div className="lg-note lg-note--error">{error}</div>}

          {filtered.map((e) => (
            <div key={e.idx} className={"lg-line lv-" + e.level}>
              <span className="lg-n">{String(e.idx).padStart(4, " ")}</span>
              <span className="lg-t">{e.ts}</span>
              <span className={"lg-lvl " + e.level}>{e.level.toUpperCase()}</span>
              <span className="lg-msg">{e.kind === "exit" ? t("Process exited with code {code}", { code: e.text }) : e.text}</span>
            </div>
          ))}
          {streaming && filtered.length > 0 && (
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
          <span className="mono">
            <span className="lg-foot-muted">{t("showing")} </span>{filtered.length}
            <span className="lg-foot-muted"> / {events.length} {t("lines")}</span>
          </span>
          <div style={{ flex: 1 }} />
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
    </>
  );
}
