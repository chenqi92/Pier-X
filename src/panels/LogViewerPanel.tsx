import { AlignJustify, Download, Pause, Play, Scroll, Search, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import * as cmd from "../lib/commands";
import type { LogEventView, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError, localizeRuntimeMessage } from "../i18n/localizeMessage";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";
import { useTabStore } from "../stores/useTabStore";

type Props = { tab: TabState };

const MAX_EVENTS = 600;

type LogLevel = "info" | "warn" | "error" | "debug";

type Enriched = {
  idx: number;
  kind: LogEventView["kind"];
  text: string;
  level: LogLevel;
  ts: string;
};

function defaultCommand(tab: TabState) {
  return tab.logCommand.trim() || "tail -f /var/log/syslog";
}

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

export default function LogViewerPanel({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const updateTab = useTabStore((s) => s.updateTab);
  const [command, setCommand] = useState(defaultCommand(tab));
  const [streamId, setStreamId] = useState<string | null>(null);
  const [events, setEvents] = useState<Enriched[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [wrap, setWrap] = useState(false);
  const [search, setSearch] = useState("");
  const [activeLevels, setActiveLevels] = useState<Record<LogLevel, boolean>>({
    info: true,
    warn: true,
    error: true,
    debug: false,
  });
  const [follow, setFollow] = useState(true);
  const [showCommand, setShowCommand] = useState(false);
  const outputRef = useRef<HTMLDivElement | null>(null);
  const counter = useRef(0);

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();

  useEffect(() => {
    const next = defaultCommand(tab);
    setCommand((current) => (current === next ? current : next));
  }, [tab.logCommand]);

  useEffect(() => {
    if (!follow) return;
    const viewport = outputRef.current;
    if (!viewport) return;
    viewport.scrollTop = viewport.scrollHeight;
  }, [events, follow]);

  async function stopStream(targetId?: string | null) {
    const resolvedId = targetId ?? streamId;
    if (!resolvedId) return;
    await cmd.logStreamStop(resolvedId).catch(() => {});
    setStreamId((current) => (current === resolvedId ? null : current));
  }

  async function startStream() {
    if (!hasSsh || !command.trim()) return;
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
        command: command.trim(),
      });
      updateTab(tab.id, { logCommand: command.trim() });
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

  const counts = useMemo(() => {
    const acc: Record<LogLevel, number> = { info: 0, warn: 0, error: 0, debug: 0 };
    for (const e of events) acc[e.level]++;
    return acc;
  }, [events]);

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return events.filter((e) => {
      if (!activeLevels[e.level]) return false;
      if (needle && !e.text.toLowerCase().includes(needle)) return false;
      return true;
    });
  }, [events, activeLevels, search]);

  const streaming = !!streamId;
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

  return (
    <>
      <PanelHeader icon={Scroll} title={t("Logs")} meta={headerMeta} />
      <div className="lg">
        <div className="lg-source">
          <Scroll size={12} />
          <span className="mono lg-source-cmd" onClick={() => setShowCommand((v) => !v)} title={t("Edit command")}>
            {command || t("(no command)")}
          </span>
          <div style={{ flex: 1 }} />
          <span className={"lg-status " + (streaming ? "on" : "off")}>
            <StatusDot tone={streaming ? "pos" : "off"} />
            {streaming ? t("streaming") : t("paused")}
          </span>
        </div>

        {showCommand && (
          <div className="lg-cmd-editor">
            <textarea
              className="field-textarea field-textarea--editor"
              rows={2}
              value={command}
              onChange={(e) => setCommand(e.currentTarget.value)}
            />
            <button
              type="button"
              className="btn is-primary is-compact"
              disabled={!hasSsh || !command.trim() || busy}
              onClick={() => { void startStream(); setShowCommand(false); }}
            >
              {busy ? t("Starting...") : streaming ? t("Restart") : t("Start")}
            </button>
          </div>
        )}

        <div className="lg-toolbar">
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
              value={search}
              onChange={(e) => setSearch(e.currentTarget.value)}
            />
            {search ? (
              <button className="lg-x" type="button" onClick={() => setSearch("")}>
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
            className={"lg-ic" + (streaming ? " on" : "")}
            title={streaming ? t("Pause") : t("Resume")}
            onClick={() => streaming ? void stopStream() : void startStream()}
            disabled={!streaming && (!hasSsh || !command.trim())}
          >
            {streaming ? <Pause size={11} /> : <Play size={11} />}
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
          {hasSsh && events.length === 0 && !streaming && (
            <div className="lg-note">{t("Start a remote log command to stream output.")}</div>
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
