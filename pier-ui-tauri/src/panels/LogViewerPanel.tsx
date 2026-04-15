import { ScrollText } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import * as cmd from "../lib/commands";
import type { LogEventView, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { useTabStore } from "../stores/useTabStore";

type Props = { tab: TabState };

const MAX_EVENTS = 600;

function defaultCommand(tab: TabState) {
  return tab.logCommand.trim() || "tail -f /var/log/syslog";
}

export default function LogViewerPanel({ tab }: Props) {
  const { t } = useI18n();
  const updateTab = useTabStore((s) => s.updateTab);
  const [command, setCommand] = useState(defaultCommand(tab));
  const [streamId, setStreamId] = useState<string | null>(null);
  const [events, setEvents] = useState<LogEventView[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const outputRef = useRef<HTMLDivElement | null>(null);

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();

  useEffect(() => {
    const next = defaultCommand(tab);
    setCommand((current) => (current === next ? current : next));
  }, [tab.logCommand]);

  useEffect(() => {
    const viewport = outputRef.current;
    if (!viewport) {
      return;
    }
    viewport.scrollTop = viewport.scrollHeight;
  }, [events]);

  async function stopStream(targetId?: string | null) {
    const resolvedId = targetId ?? streamId;
    if (!resolvedId) {
      return;
    }
    await cmd.logStreamStop(resolvedId).catch(() => {});
    setStreamId((current) => (current === resolvedId ? null : current));
  }

  async function startStream() {
    if (!hasSsh || !command.trim()) {
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
        command: command.trim(),
      });
      updateTab(tab.id, { logCommand: command.trim() });
      setEvents([]);
      setStreamId(nextId);
      setNotice(t("Streaming remote command."));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    if (!streamId) {
      return;
    }

    let disposed = false;

    const drain = () => {
      cmd.logStreamDrain(streamId)
        .then((batch) => {
          if (disposed || batch.length === 0) {
            return;
          }

          setEvents((current) => [...current, ...batch].slice(-MAX_EVENTS));

          const terminalEvent = batch.find((entry) => entry.kind === "exit" || entry.kind === "error");
          if (terminalEvent) {
            if (terminalEvent.kind === "exit") {
              setNotice(t("Log stream exited with code {code}.", { code: terminalEvent.text }));
            } else {
              setError(terminalEvent.text || t("Log stream ended with an error."));
            }
            void stopStream(streamId);
          }
        })
        .catch((drainError) => {
          if (disposed) {
            return;
          }
          setError(String(drainError));
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

  return (
    <div className="panel-scroll">
      <section className="panel-section">
        <div className="panel-section__title"><ScrollText size={14} /><span>{t("Logs")}</span></div>
        <div className="form-stack">
          <label className="field-stack">
            <span className="field-label">{t("Remote command")}</span>
            <textarea
              className="field-textarea field-textarea--editor"
              onChange={(event) => setCommand(event.currentTarget.value)}
              rows={3}
              value={command}
            />
          </label>
          <div className="button-row">
            <button className="mini-button" disabled={!hasSsh || !command.trim() || busy} onClick={() => void startStream()} type="button">
              {busy ? t("Starting...") : streamId ? t("Restart Stream") : t("Start Stream")}
            </button>
            <button className="mini-button" disabled={!streamId} onClick={() => void stopStream()} type="button">
              {t("Stop Stream")}
            </button>
            <button className="mini-button" disabled={events.length === 0} onClick={() => setEvents([])} type="button">
              {t("Clear Output")}
            </button>
          </div>
          {!hasSsh && <div className="inline-note">{t("SSH connection required.")}</div>}
          {notice && <div className="status-note">{notice}</div>}
          {error && <div className="status-note status-note--error">{error}</div>}
        </div>
      </section>

      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Stream Output")}</span></div>
        {events.length > 0 ? (
          <div className="log-viewer" ref={outputRef}>
            {events.map((event, index) => (
              <div className={`log-line log-line--${event.kind}`} key={`${event.kind}-${index}-${event.text.slice(0, 32)}`}>
                <span className="log-line__kind">{event.kind}</span>
                <span className="log-line__text">
                  {event.kind === "exit"
                    ? t("Process exited with code {code}", { code: event.text })
                    : event.text}
                </span>
              </div>
            ))}
          </div>
        ) : (
          <div className="empty-note">{t("Start a remote log command to stream output.")}</div>
        )}
      </section>
    </div>
  );
}
