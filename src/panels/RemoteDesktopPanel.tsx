// ── Remote Desktop panel (RDP / VNC) ─────────────────────────────────────
// Renders one remote-desktop session onto a <canvas>. Frames arrive as
// binary packets over a Tauri Channel (see lib/remoteDesktop.ts); input is
// captured here and forwarded to the backend. Kept mounted like the terminal
// panel (display:none when inactive) so the live session survives tab
// switches. Takes over the full center+right width via App.tsx's layout
// modifier — this component only paints its own region.

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Monitor, RotateCw } from "lucide-react";
import type { TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import {
  keyToInput,
  remoteDesktopClose,
  remoteDesktopConnect,
  remoteDesktopInput,
  type RdPacket,
  type RemoteInput,
} from "../lib/remoteDesktop";
import "../styles/remote-desktop.css";

type Props = {
  tab: TabState;
  isActive: boolean;
};

type Status = "connecting" | "connected" | "error" | "disconnected";

function RemoteDesktopPanel({ tab, isActive }: Props) {
  const i18n = useI18n();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const ctxRef = useRef<CanvasRenderingContext2D | null>(null);
  const sessionIdRef = useRef<string | null>(null);
  const remoteSizeRef = useRef<{ w: number; h: number }>({ w: 0, h: 0 });

  const [status, setStatus] = useState<Status>("connecting");
  const [errorMsg, setErrorMsg] = useState<string>("");
  const [connectAttempt, setConnectAttempt] = useState(0);

  const protocolLabel = tab.rdProtocol === "rdp" ? "RDP" : "VNC";

  // ── Fit the canvas element to the container, preserving aspect ratio so
  // pointer mapping stays a simple linear scale (no letterboxing math). ──
  const fitCanvas = useCallback(() => {
    const stage = stageRef.current;
    const canvas = canvasRef.current;
    const { w, h } = remoteSizeRef.current;
    if (!stage || !canvas || w === 0 || h === 0) return;
    const cw = stage.clientWidth;
    const ch = stage.clientHeight;
    if (cw === 0 || ch === 0) return;
    const scale = Math.min(cw / w, ch / h);
    canvas.style.width = `${Math.round(w * scale)}px`;
    canvas.style.height = `${Math.round(h * scale)}px`;
  }, []);

  // ── Apply one decoded packet to the canvas. ──
  const onPacket = useCallback(
    (packet: RdPacket) => {
      const canvas = canvasRef.current;
      const ctx = ctxRef.current;
      switch (packet.kind) {
        case "connected":
        case "resize": {
          if (canvas) {
            canvas.width = packet.width;
            canvas.height = packet.height;
          }
          remoteSizeRef.current = { w: packet.width, h: packet.height };
          fitCanvas();
          if (packet.kind === "connected") setStatus("connected");
          break;
        }
        case "tile": {
          if (!ctx) break;
          const len = packet.width * packet.height * 4;
          if (packet.encoding === "rgba") {
            if (packet.data.byteLength < len) break;
            const clamped = new Uint8ClampedArray(packet.data.buffer, packet.data.byteOffset, len);
            // Copy out of the channel buffer — ImageData keeps a reference.
            const img = new ImageData(new Uint8ClampedArray(clamped), packet.width, packet.height);
            ctx.putImageData(img, packet.x, packet.y);
          } else {
            const { x, y, data } = packet;
            const blob = new Blob([data.slice()], { type: "image/jpeg" });
            void createImageBitmap(blob)
              .then((bmp) => {
                ctxRef.current?.drawImage(bmp, x, y);
                bmp.close();
              })
              .catch(() => {});
          }
          break;
        }
        case "copy": {
          if (!ctx || !canvas) break;
          ctx.drawImage(
            canvas,
            packet.srcX,
            packet.srcY,
            packet.width,
            packet.height,
            packet.dstX,
            packet.dstY,
            packet.width,
            packet.height,
          );
          break;
        }
        case "cursor":
          // Cursor is rendered into the framebuffer by the server (we don't
          // advertise the VNC cursor pseudo-encoding), so nothing to do.
          break;
        case "clipboard":
          // Remote clipboard mirroring is a follow-up.
          break;
        case "disconnected": {
          sessionIdRef.current = null;
          setStatus("disconnected");
          setErrorMsg(packet.reason ?? "");
          break;
        }
      }
    },
    [fitCanvas],
  );

  // ── Connect on mount / reconnect; close on unmount. ──
  useEffect(() => {
    if (!tab.rdHost) {
      setStatus("error");
      setErrorMsg(i18n.t("No host configured for this connection."));
      return;
    }
    let cancelled = false;
    let pendingId: string | null = null;
    setStatus("connecting");
    setErrorMsg("");

    const initW = Math.min(1920, Math.max(800, Math.round(window.innerWidth)));
    const initH = Math.min(1200, Math.max(600, Math.round(window.innerHeight)));

    remoteDesktopConnect(
      {
        protocol: tab.rdProtocol,
        host: tab.rdHost,
        port: tab.rdPort,
        username: tab.rdUser,
        password: tab.rdPassword,
        domain: tab.rdDomain || null,
        width: initW,
        height: initH,
      },
      onPacket,
    )
      .then((id) => {
        if (cancelled) {
          void remoteDesktopClose(id).catch(() => {});
          return;
        }
        pendingId = id;
        sessionIdRef.current = id;
      })
      .catch((err) => {
        if (cancelled) return;
        setStatus("error");
        setErrorMsg(String(err));
      });

    return () => {
      cancelled = true;
      const id = sessionIdRef.current ?? pendingId;
      if (id) {
        void remoteDesktopClose(id).catch(() => {});
      }
      sessionIdRef.current = null;
    };
    // Reconnect when the user hits retry; tab addressing is otherwise fixed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectAttempt, tab.rdHost, tab.rdPort, tab.rdProtocol]);

  // Grab the 2D context once the canvas mounts.
  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (canvas) {
      ctxRef.current = canvas.getContext("2d", { alpha: false });
    }
  }, []);

  // Re-fit on container resize.
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    const ro = new ResizeObserver(() => fitCanvas());
    ro.observe(stage);
    return () => ro.disconnect();
  }, [fitCanvas]);

  // Focus the surface when this tab becomes active so keys flow immediately.
  useEffect(() => {
    if (!isActive) return;
    const id = window.requestAnimationFrame(() => {
      containerRef.current?.focus();
      fitCanvas();
    });
    return () => window.cancelAnimationFrame(id);
  }, [isActive, fitCanvas]);

  // ── Input forwarding ──
  const send = useCallback((event: RemoteInput) => {
    const id = sessionIdRef.current;
    if (id) void remoteDesktopInput(id, event).catch(() => {});
  }, []);

  const toRemote = useCallback((clientX: number, clientY: number) => {
    const canvas = canvasRef.current;
    const { w, h } = remoteSizeRef.current;
    if (!canvas || w === 0 || h === 0) return { x: 0, y: 0 };
    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return { x: 0, y: 0 };
    const x = Math.max(0, Math.min(w - 1, Math.round(((clientX - rect.left) * w) / rect.width)));
    const y = Math.max(0, Math.min(h - 1, Math.round(((clientY - rect.top) * h) / rect.height)));
    return { x, y };
  }, []);

  // Coalesce pointer-move spam to one send per animation frame.
  const moveRef = useRef<{ x: number; y: number } | null>(null);
  const moveScheduled = useRef(false);
  const flushMove = useCallback(() => {
    moveScheduled.current = false;
    const m = moveRef.current;
    if (m) send({ kind: "pointerMove", x: m.x, y: m.y });
  }, [send]);

  const onMouseMove = useCallback(
    (e: React.MouseEvent) => {
      moveRef.current = toRemote(e.clientX, e.clientY);
      if (!moveScheduled.current) {
        moveScheduled.current = true;
        window.requestAnimationFrame(flushMove);
      }
    },
    [toRemote, flushMove],
  );

  const onMouseButton = useCallback(
    (e: React.MouseEvent, pressed: boolean) => {
      if (e.button > 2) return;
      e.preventDefault();
      containerRef.current?.focus();
      const { x, y } = toRemote(e.clientX, e.clientY);
      send({ kind: "pointerButton", x, y, button: e.button, pressed });
    },
    [toRemote, send],
  );

  const onWheel = useCallback(
    (e: React.WheelEvent) => {
      const { x, y } = toRemote(e.clientX, e.clientY);
      send({
        kind: "pointerScroll",
        x,
        y,
        dx: Math.sign(e.deltaX),
        dy: Math.sign(e.deltaY),
      });
    },
    [toRemote, send],
  );

  const onKey = useCallback(
    (e: React.KeyboardEvent, pressed: boolean) => {
      const input = keyToInput(e.nativeEvent, pressed);
      if (!input) return;
      e.preventDefault();
      send(input);
    },
    [send],
  );

  const reconnect = useCallback(() => setConnectAttempt((n) => n + 1), []);

  return (
    <section
      className="rd-panel"
      style={{ display: isActive ? "flex" : "none" }}
      ref={containerRef}
      tabIndex={0}
      onKeyDown={(e) => onKey(e, true)}
      onKeyUp={(e) => onKey(e, false)}
    >
      <div className="rd-panel__header">
        <div className="rd-panel__title">
          <Monitor size={15} />
          <span>{tab.rdUser ? `${tab.rdUser}@${tab.rdHost}` : tab.rdHost}</span>
          <span className="rd-panel__proto">{protocolLabel}</span>
        </div>
        <div className="rd-panel__meta">
          <span className={`meta-pill ${status === "connected" ? "meta-pill--success" : ""}`}>
            {status === "connected"
              ? i18n.t("Connected")
              : status === "connecting"
                ? i18n.t("Connecting…")
                : status === "disconnected"
                  ? i18n.t("Disconnected")
                  : i18n.t("Error")}
          </span>
          {(status === "error" || status === "disconnected") && (
            <button type="button" className="btn" onClick={reconnect}>
              <RotateCw size={13} />
              {i18n.t("Reconnect")}
            </button>
          )}
        </div>
      </div>

      <div
        className="rd-panel__stage"
        ref={stageRef}
        onMouseMove={onMouseMove}
        onMouseDown={(e) => onMouseButton(e, true)}
        onMouseUp={(e) => onMouseButton(e, false)}
        onWheel={onWheel}
        onContextMenu={(e) => e.preventDefault()}
      >
        <canvas className="rd-canvas" ref={canvasRef} />
        {status !== "connected" && (
          <div className="rd-panel__overlay">
            {status === "connecting" && (
              <div className="rd-panel__overlay-card">
                <div className="rd-spinner" />
                <span>
                  {i18n.t("Connecting to {host}…", { host: tab.rdHost })}
                </span>
              </div>
            )}
            {(status === "error" || status === "disconnected") && (
              <div className="rd-panel__overlay-card">
                <Monitor size={26} />
                <span className="rd-panel__overlay-title">
                  {status === "error"
                    ? i18n.t("Connection failed")
                    : i18n.t("Session ended")}
                </span>
                {errorMsg && <span className="rd-panel__overlay-detail">{errorMsg}</span>}
                <button type="button" className="btn is-primary" onClick={reconnect}>
                  <RotateCw size={14} />
                  {i18n.t("Reconnect")}
                </button>
              </div>
            )}
          </div>
        )}
      </div>
    </section>
  );
}

export default RemoteDesktopPanel;
