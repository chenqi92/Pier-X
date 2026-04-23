import { AlignJustify, ArrowDown, ExternalLink, Pause, Play, Scroll, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import IconButton from "../components/IconButton";
import { useDraggableDialog } from "../components/useDraggableDialog";
import * as cmd from "../lib/commands";
import { quoteCommandArg } from "../lib/commands";
import type { LogEventView, TabState } from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError, localizeRuntimeMessage } from "../i18n/localizeMessage";

type Props = {
  open: boolean;
  tab: TabState;
  containerId: string;
  containerName?: string;
  onClose: () => void;
  /** Switch the right-side tool to the Log panel and stream the
   *  container's logs there. Shown as an icon button in the header
   *  so the user can get the same content without a modal blocking
   *  the rest of the UI. */
  onOpenInLogPanel?: () => void;
};

type Line = {
  idx: number;
  kind: LogEventView["kind"];
  text: string;
};

const MAX_LINES = 2000;

export default function ContainerLogsDialog({ open, tab, containerId, containerName, onClose, onOpenInLogPanel }: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);
  const { dialogStyle, handleProps } = useDraggableDialog(open);

  const [streamId, setStreamId] = useState<string | null>(null);
  const [lines, setLines] = useState<Line[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [wrap, setWrap] = useState(true);
  const [follow, setFollow] = useState(true);
  const counter = useRef(0);
  const outputRef = useRef<HTMLDivElement | null>(null);

  const sshTarget = effectiveSshTarget(tab);
  const hasSsh = sshTarget !== null;

  async function stopStream(target?: string | null) {
    const id = target ?? streamId;
    if (!id) return;
    await cmd.logStreamStop(id).catch(() => {});
    setStreamId((cur) => (cur === id ? null : cur));
  }

  async function startStream() {
    if (!hasSsh || !containerId) return;
    setBusy(true);
    setError("");
    setNotice("");
    if (streamId) await stopStream(streamId);
    try {
      const command = `docker logs -f --tail 500 ${quoteCommandArg(containerId)} 2>&1`;
      const nextId = await cmd.logStreamStart({
        host: sshTarget!.host,
        port: sshTarget!.port,
        user: sshTarget!.user,
        authMode: sshTarget!.authMode,
        password: sshTarget!.password,
        keyPath: sshTarget!.keyPath,
        command,
        savedConnectionIndex: sshTarget!.savedConnectionIndex,
      });
      setLines([]);
      counter.current = 0;
      setStreamId(nextId);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  // Open/close lifecycle: start the stream when the dialog opens for a
  // fresh container, and stop it cleanly on close so the backend doesn't
  // keep the SSH exec channel open.
  useEffect(() => {
    if (!open) return;
    if (!hasSsh) {
      setError(t("SSH connection required for log streaming."));
      return;
    }
    void startStream();
    return () => {
      void stopStream();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, containerId]);

  // Drain loop — adaptive cadence mirrors LogViewerPanel. Fast tier keeps
  // flowing logs snappy, slow tier avoids burning IPC on quiet streams.
  useEffect(() => {
    if (!streamId) return;
    const MIN_MS = 200;
    const MAX_MS = 1500;
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
            return;
          }
          delay = MIN_MS;
          setLines((cur) => {
            const appended = batch.map<Line>((b) => ({
              idx: ++counter.current,
              kind: b.kind,
              text: b.text,
            }));
            return [...cur, ...appended].slice(-MAX_LINES);
          });
          const terminal = batch.find((b) => b.kind === "exit" || b.kind === "error");
          if (terminal) {
            if (terminal.kind === "exit") {
              setNotice(t("Stream exited ({code}).", { code: terminal.text }));
            } else {
              setError(localizeRuntimeMessage(terminal.text || t("Stream ended."), t));
            }
            void stopStream(streamId);
            return;
          }
          schedule();
        })
        .catch((err) => {
          if (disposed) return;
          setError(formatError(err));
          void stopStream(streamId);
        });
    };
    run();
    return () => {
      disposed = true;
      if (timerId !== null) window.clearTimeout(timerId);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streamId]);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  useEffect(() => {
    if (!follow) return;
    const el = outputRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [lines, follow]);

  const streaming = streamId !== null;
  const title = useMemo(
    () => containerName || containerId.slice(0, 12),
    [containerName, containerId],
  );

  if (!open) return null;

  return createPortal(
    <div className="cmdp-overlay" onClick={onClose}>
      <div
        className="dlg dlg--logs"
        style={dialogStyle}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <Scroll size={12} />
            {t("Logs")} · <span className="mono">{title}</span>
          </span>
          <div style={{ flex: 1 }} />
          <button
            type="button"
            className={"mini-btn" + (streaming ? " is-stop" : " is-start")}
            title={streaming ? t("Pause") : t("Resume")}
            disabled={busy || !hasSsh}
            onClick={() => (streaming ? void stopStream() : void startStream())}
          >
            {streaming ? <Pause size={11} /> : <Play size={11} />}
          </button>
          <button
            type="button"
            className={"mini-btn" + (wrap ? " is-info" : "")}
            title={t("Wrap lines")}
            onClick={() => setWrap((v) => !v)}
          >
            <AlignJustify size={11} />
          </button>
          <button
            type="button"
            className="mini-btn"
            title={t("Clear")}
            disabled={lines.length === 0}
            onClick={() => {
              setLines([]);
              counter.current = 0;
            }}
          >
            <Trash2 size={11} />
          </button>
          {onOpenInLogPanel && (
            <button
              type="button"
              className="mini-btn"
              title={t("Open in Log panel")}
              onClick={() => {
                onOpenInLogPanel();
                onClose();
              }}
            >
              <ExternalLink size={11} />
            </button>
          )}
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>
        <div
          className={"dlg-logs-body mono" + (wrap ? " wrap" : "")}
          ref={outputRef}
          onScroll={(e) => {
            const el = e.currentTarget;
            const atBottom =
              el.scrollHeight - el.clientHeight - el.scrollTop < 4;
            if (atBottom !== follow) setFollow(atBottom);
          }}
        >
          {!hasSsh && (
            <div className="lg-note">{t("SSH connection required for log streaming.")}</div>
          )}
          {lines.length === 0 && !error && hasSsh && (
            <div className="lg-note">{busy ? t("Connecting…") : t("Waiting for output…")}</div>
          )}
          {lines.map((l) => (
            <div
              key={l.idx}
              className={"dlg-logs-line" + (l.kind === "stderr" ? " is-err" : "")}
            >
              <span className="dlg-logs-n">{l.idx}</span>
              <span className="dlg-logs-msg">{l.text}</span>
            </div>
          ))}
        </div>
        <div className="dlg-foot">
          <span className="mono text-muted" style={{ fontSize: "var(--size-micro)" }}>
            {streaming ? t("● streaming") : t("○ paused")}
            {"  ·  "}
            {t("{count} lines", { count: lines.length })}
          </span>
          <div style={{ flex: 1 }} />
          {notice && <span className="mono text-muted" style={{ fontSize: "var(--size-micro)" }}>{notice}</span>}
          {error && <span className="mono" style={{ fontSize: "var(--size-micro)", color: "var(--neg)" }}>{error}</span>}
          <button
            type="button"
            className={"mini-btn" + (follow ? " is-info" : "")}
            title={t("Follow tail")}
            onClick={() => {
              setFollow(true);
              const el = outputRef.current;
              if (el) el.scrollTop = el.scrollHeight;
            }}
          >
            <ArrowDown size={11} />
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
