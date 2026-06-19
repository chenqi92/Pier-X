// ── Remote Desktop panel (RDP / VNC) ─────────────────────────────────────
// Renders one remote-desktop session. RDP uses our existing canvas packet
// stream; VNC uses noVNC via a local WebSocket-to-TCP proxy. Kept mounted
// like the terminal panel (display:none when inactive) so live sessions
// survive tab switches. Takes over the full center+right width via App.tsx's
// layout modifier — this component only paints its own region.

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Activity, Image, Maximize2, Monitor, RotateCw, Zap } from "lucide-react";
import RFB, {
  type RFBClipboardEvent,
  type RFBDisconnectEvent,
  type RFBSecurityFailureEvent,
} from "@novnc/novnc";
import type { TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import {
  diagnoseRemoteDesktopError,
  keyToInput,
  remoteDesktopClose,
  remoteDesktopConnect,
  remoteDesktopInput,
  remoteDesktopVncProxyStart,
  remoteDesktopVncProxyStop,
  type RdPacket,
  type RemoteInput,
} from "../lib/remoteDesktop";
import { readClipboardText, writeClipboardText } from "../lib/clipboard";
import "../styles/remote-desktop.css";

type Props = {
  tab: TabState;
  isActive: boolean;
};

type RemoteKeyInput = Extract<RemoteInput, { kind: "key" }>;

type Status = "connecting" | "connected" | "error" | "disconnected";
type VncRenderMode = "latency" | "quality";
type VncStats = {
  fps: number;
  frames: number;
  remoteSize: string;
  mode: VncRenderMode;
  resizeRemote: boolean;
  idleMs: number | null;
};
type NoVncRfbInternals = RFB & {
  _display?: {
    flip?: () => void;
  };
  _fbWidth?: number;
  _fbHeight?: number;
};

const VNC_RENDER_MODE_KEY = "pierx:vnc-render-mode";
const VNC_REMOTE_RESIZE_KEY = "pierx:vnc-remote-resize";

function readVncRenderMode(): VncRenderMode {
  try {
    return localStorage.getItem(VNC_RENDER_MODE_KEY) === "quality" ? "quality" : "latency";
  } catch {
    return "latency";
  }
}

function readVncRemoteResize() {
  try {
    return localStorage.getItem(VNC_REMOTE_RESIZE_KEY) === "1";
  } catch {
    return false;
  }
}

function applyVncPerformance(rfb: RFB, mode: VncRenderMode, resizeRemote: boolean) {
  rfb.scaleViewport = true;
  rfb.resizeSession = resizeRemote;
  rfb.clipViewport = false;
  rfb.focusOnClick = true;
  if (mode === "latency") {
    rfb.compressionLevel = 0;
    rfb.qualityLevel = 4;
  } else {
    rfb.compressionLevel = 2;
    rfb.qualityLevel = 8;
  }
}

type CursorState = {
  width: number;
  height: number;
  hotX: number;
  hotY: number;
  x: number;
  y: number;
  inside: boolean;
  hasShape: boolean;
};

function RemoteDesktopPanel(props: Props) {
  if (props.tab.rdProtocol === "vnc") {
    return <NoVncRemoteDesktopPanel {...props} />;
  }
  return <CanvasRemoteDesktopPanel {...props} />;
}

function CanvasRemoteDesktopPanel({ tab, isActive }: Props) {
  const i18n = useI18n();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const cursorCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const ctxRef = useRef<CanvasRenderingContext2D | null>(null);
  // Ordered paint queue. JPEG tiles decode asynchronously (createImageBitmap),
  // so without serialisation a slow decode can land *after* a newer RGBA/JPEG
  // tile for the same region and overwrite it — visible as tearing or stale
  // rectangles under motion. Every canvas write goes through this chain so it
  // is applied in arrival order; `paintGenRef` invalidates work queued by a
  // previous connection on reconnect.
  const paintTailRef = useRef<Promise<void>>(Promise.resolve());
  const paintGenRef = useRef(0);
  const sessionIdRef = useRef<string | null>(null);
  const remoteSizeRef = useRef<{ w: number; h: number }>({ w: 0, h: 0 });
  const cursorStateRef = useRef<CursorState>({
    width: 0,
    height: 0,
    hotX: 0,
    hotY: 0,
    x: 0,
    y: 0,
    inside: false,
    hasShape: false,
  });
  const isActiveRef = useRef(isActive);
  // Last clipboard text synced in either direction — dedupes so we don't
  // echo a value we just received back to the side it came from.
  const lastClipboardRef = useRef<string>("");
  // Currently-held keys (by physical `code`), so we can release them if the
  // surface loses focus mid-keypress — otherwise a modifier can stick down
  // on the remote after the user tabs away.
  const pressedKeysRef = useRef<Map<string, RemoteKeyInput>>(new Map());

  const [status, setStatus] = useState<Status>("connecting");
  const [errorMsg, setErrorMsg] = useState<string>("");
  const [connectAttempt, setConnectAttempt] = useState(0);
  const [hasRemoteCursor, setHasRemoteCursor] = useState(false);

  const protocolLabel = tab.rdProtocol === "rdp" ? "RDP" : "VNC";

  const positionCursorOverlay = useCallback(() => {
    const cursorCanvas = cursorCanvasRef.current;
    const canvas = canvasRef.current;
    const stage = stageRef.current;
    const { w, h } = remoteSizeRef.current;
    const cursor = cursorStateRef.current;

    if (!cursorCanvas) return;
    if (!canvas || !stage || !cursor.hasShape || !cursor.inside || w === 0 || h === 0) {
      cursorCanvas.style.display = "none";
      return;
    }

    const canvasRect = canvas.getBoundingClientRect();
    const stageRect = stage.getBoundingClientRect();
    if (canvasRect.width === 0 || canvasRect.height === 0) {
      cursorCanvas.style.display = "none";
      return;
    }

    const scaleX = canvasRect.width / w;
    const scaleY = canvasRect.height / h;
    const x = Math.max(0, Math.min(w - 1, cursor.x));
    const y = Math.max(0, Math.min(h - 1, cursor.y));
    const left = canvasRect.left - stageRect.left + x * scaleX - cursor.hotX * scaleX;
    const top = canvasRect.top - stageRect.top + y * scaleY - cursor.hotY * scaleY;

    cursorCanvas.style.display = "block";
    cursorCanvas.style.width = `${cursor.width * scaleX}px`;
    cursorCanvas.style.height = `${cursor.height * scaleY}px`;
    cursorCanvas.style.transform = `translate(${left}px, ${top}px)`;
  }, []);

  const resetCursorOverlay = useCallback(() => {
    cursorStateRef.current = {
      width: 0,
      height: 0,
      hotX: 0,
      hotY: 0,
      x: 0,
      y: 0,
      inside: false,
      hasShape: false,
    };
    const cursorCanvas = cursorCanvasRef.current;
    if (cursorCanvas) {
      cursorCanvas.width = 0;
      cursorCanvas.height = 0;
      cursorCanvas.style.display = "none";
    }
    setHasRemoteCursor(false);
  }, []);

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
    positionCursorOverlay();
  }, [positionCursorOverlay]);

  const isOverCanvas = useCallback((clientX: number, clientY: number) => {
    const canvas = canvasRef.current;
    if (!canvas) return false;
    const rect = canvas.getBoundingClientRect();
    return clientX >= rect.left && clientX <= rect.right && clientY >= rect.top && clientY <= rect.bottom;
  }, []);

  // Append one canvas op to the ordered paint chain. `gen` is captured at
  // enqueue time; ops from a superseded connection are skipped.
  const enqueuePaint = useCallback((gen: number, op: () => void | Promise<void>) => {
    paintTailRef.current = paintTailRef.current
      .then(() => {
        if (gen === paintGenRef.current) return op();
      })
      .catch(() => {});
  }, []);

  // ── Apply one decoded packet to the canvas. ──
  const onPacket = useCallback(
    (packet: RdPacket) => {
      const canvas = canvasRef.current;
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
          const gen = paintGenRef.current;
          const { x, y, width, height } = packet;
          if (packet.encoding === "rgba") {
            const len = width * height * 4;
            if (packet.data.byteLength < len) break;
            // Copy out of the channel buffer now (it is reused next turn);
            // ImageData keeps the reference, so the queued paint is safe.
            const img = new ImageData(new Uint8ClampedArray(packet.data.subarray(0, len)), width, height);
            enqueuePaint(gen, () => {
              ctxRef.current?.putImageData(img, x, y);
            });
          } else {
            // Copy the JPEG bytes now; decode + paint on the queue so a slow
            // decode can't overwrite a newer tile.
            const blob = new Blob([packet.data.slice()], { type: "image/jpeg" });
            enqueuePaint(gen, async () => {
              const bmp = await createImageBitmap(blob);
              ctxRef.current?.drawImage(bmp, x, y);
              bmp.close();
            });
          }
          break;
        }
        case "copy": {
          const gen = paintGenRef.current;
          const { srcX, srcY, width, height, dstX, dstY } = packet;
          enqueuePaint(gen, () => {
            const c = canvasRef.current;
            const cx = ctxRef.current;
            if (c && cx) cx.drawImage(c, srcX, srcY, width, height, dstX, dstY, width, height);
          });
          break;
        }
        case "cursor":
          {
            const cursorCanvas = cursorCanvasRef.current;
            const expected = packet.width * packet.height * 4;
            const hasShape = packet.width > 0 && packet.height > 0 && packet.data.byteLength >= expected;

            cursorStateRef.current = {
              ...cursorStateRef.current,
              width: packet.width,
              height: packet.height,
              hotX: packet.hotX,
              hotY: packet.hotY,
              hasShape,
            };
            setHasRemoteCursor(hasShape);

            if (!cursorCanvas || !hasShape) {
              positionCursorOverlay();
              break;
            }

            cursorCanvas.width = packet.width;
            cursorCanvas.height = packet.height;
            const cursorCtx = cursorCanvas.getContext("2d");
            if (!cursorCtx) {
              cursorStateRef.current.hasShape = false;
              setHasRemoteCursor(false);
              positionCursorOverlay();
              break;
            }
            const pixels = new Uint8ClampedArray(packet.data.subarray(0, expected));
            cursorCtx.clearRect(0, 0, packet.width, packet.height);
            cursorCtx.putImageData(new ImageData(pixels, packet.width, packet.height), 0, 0);
            positionCursorOverlay();
          }
          break;
        case "clipboard":
          // Mirror the remote clipboard into the local one, but only for the
          // tab the user is actually looking at (every session streams).
          if (isActiveRef.current && packet.text) {
            lastClipboardRef.current = packet.text;
            void writeClipboardText(packet.text);
          }
          break;
        case "disconnected": {
          sessionIdRef.current = null;
          setStatus("disconnected");
          setErrorMsg(packet.reason ?? "");
          break;
        }
      }
    },
    [fitCanvas, positionCursorOverlay, enqueuePaint],
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
    resetCursorOverlay();
    // Invalidate any paints still queued from a previous session and start a
    // fresh ordered chain for this connection.
    paintGenRef.current += 1;
    paintTailRef.current = Promise.resolve();

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

  // Keep a ref of the active flag for callbacks captured at connect time.
  useEffect(() => {
    isActiveRef.current = isActive;
  }, [isActive]);

  // ── Input forwarding ──
  const send = useCallback((event: RemoteInput) => {
    const id = sessionIdRef.current;
    if (id) void remoteDesktopInput(id, event).catch(() => {});
  }, []);

  // Focus the surface when this tab becomes active so keys flow immediately,
  // and push the local clipboard to the remote so a paste there sees it.
  useEffect(() => {
    if (!isActive) return;
    const id = window.requestAnimationFrame(() => {
      containerRef.current?.focus();
      fitCanvas();
    });
    void readClipboardText().then((text) => {
      if (text && text !== lastClipboardRef.current) {
        lastClipboardRef.current = text;
        send({ kind: "setClipboard", text });
      }
    });
    return () => window.cancelAnimationFrame(id);
  }, [isActive, fitCanvas, send]);

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

  const updateCursorPoint = useCallback(
    (clientX: number, clientY: number) => {
      const point = toRemote(clientX, clientY);
      const cursor = cursorStateRef.current;
      cursor.x = point.x;
      cursor.y = point.y;
      cursor.inside = isOverCanvas(clientX, clientY);
      positionCursorOverlay();
      return point;
    },
    [toRemote, isOverCanvas, positionCursorOverlay],
  );

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
      moveRef.current = updateCursorPoint(e.clientX, e.clientY);
      if (!moveScheduled.current) {
        moveScheduled.current = true;
        window.requestAnimationFrame(flushMove);
      }
    },
    [updateCursorPoint, flushMove],
  );

  const onMouseButton = useCallback(
    (e: React.MouseEvent, pressed: boolean) => {
      if (e.button > 2) return;
      e.preventDefault();
      containerRef.current?.focus();
      const { x, y } = updateCursorPoint(e.clientX, e.clientY);
      send({ kind: "pointerButton", x, y, button: e.button, pressed });
    },
    [updateCursorPoint, send],
  );

  const onWheel = useCallback(
    (e: React.WheelEvent) => {
      const { x, y } = updateCursorPoint(e.clientX, e.clientY);
      send({
        kind: "pointerScroll",
        x,
        y,
        dx: Math.sign(e.deltaX),
        dy: Math.sign(e.deltaY),
      });
    },
    [updateCursorPoint, send],
  );

  const onMouseLeave = useCallback(() => {
    cursorStateRef.current.inside = false;
    positionCursorOverlay();
  }, [positionCursorOverlay]);

  const onKey = useCallback(
    (e: React.KeyboardEvent, pressed: boolean) => {
      const input = keyToInput(e.nativeEvent, pressed);
      if (!input || input.kind !== "key") return;
      e.preventDefault();
      if (pressed) pressedKeysRef.current.set(e.code, input);
      else pressedKeysRef.current.delete(e.code);
      send(input);
    },
    [send],
  );

  // Release every held key — on blur, or when the tab goes inactive.
  const releaseAllKeys = useCallback(() => {
    const held = pressedKeysRef.current;
    for (const input of held.values()) send({ ...input, pressed: false });
    held.clear();
  }, [send]);

  useEffect(() => {
    if (!isActive) releaseAllKeys();
  }, [isActive, releaseAllKeys]);

  const reconnect = useCallback(() => setConnectAttempt((n) => n + 1), []);

  // Friendly, actionable line derived from the raw backend error; the raw
  // string is still shown beneath it as the technical detail.
  const errorHint = errorMsg ? diagnoseRemoteDesktopError(errorMsg) : null;

  return (
    <section
      className="rd-panel"
      style={{ display: isActive ? "flex" : "none" }}
      ref={containerRef}
      tabIndex={0}
      onKeyDown={(e) => onKey(e, true)}
      onKeyUp={(e) => onKey(e, false)}
      onBlur={releaseAllKeys}
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
        onMouseLeave={onMouseLeave}
        onWheel={onWheel}
        onContextMenu={(e) => e.preventDefault()}
      >
        <canvas
          className={
            "rd-canvas" +
            (tab.rdProtocol === "rdp" || hasRemoteCursor ? " rd-canvas--hide-native-cursor" : "")
          }
          ref={canvasRef}
        />
        <canvas className="rd-cursor" ref={cursorCanvasRef} aria-hidden="true" />
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
                {errorHint && <span className="rd-panel__overlay-hint">{i18n.t(errorHint)}</span>}
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

function NoVncRemoteDesktopPanel({ tab, isActive }: Props) {
  const i18n = useI18n();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const rfbRef = useRef<RFB | null>(null);
  const proxyIdRef = useRef<string | null>(null);
  const isActiveRef = useRef(isActive);
  const lastClipboardRef = useRef<string>("");
  const frameCountRef = useRef(0);
  const lastFrameAtRef = useRef<number | null>(null);
  const restoreFrameProbeRef = useRef<(() => void) | null>(null);
  const [vncMode, setVncMode] = useState<VncRenderMode>(readVncRenderMode);
  const [resizeRemote, setResizeRemote] = useState(readVncRemoteResize);
  const [showStats, setShowStats] = useState(false);
  const [stats, setStats] = useState<VncStats>({
    fps: 0,
    frames: 0,
    remoteSize: "—",
    mode: vncMode,
    resizeRemote,
    idleMs: null,
  });
  const vncSettingsRef = useRef({ mode: vncMode, resizeRemote });

  const [status, setStatus] = useState<Status>("connecting");
  const [errorMsg, setErrorMsg] = useState<string>("");
  const [connectAttempt, setConnectAttempt] = useState(0);

  const reconnect = useCallback(() => setConnectAttempt((n) => n + 1), []);

  const installFrameProbe = useCallback((rfb: RFB) => {
    restoreFrameProbeRef.current?.();
    restoreFrameProbeRef.current = null;
    frameCountRef.current = 0;
    lastFrameAtRef.current = null;

    const internals = rfb as NoVncRfbInternals;
    const display = internals._display;
    const originalFlip = display?.flip;
    if (!display || typeof originalFlip !== "function") return;

    const patchedFlip: typeof originalFlip = function patchedFlip() {
      frameCountRef.current += 1;
      lastFrameAtRef.current = performance.now();
      return originalFlip.call(display);
    };

    display.flip = patchedFlip;
    restoreFrameProbeRef.current = () => {
      if (display.flip === patchedFlip) {
        display.flip = originalFlip;
      }
    };
  }, []);

  useEffect(() => {
    const stage = stageRef.current;
    if (!stage || !tab.rdHost) {
      setStatus("error");
      setErrorMsg(i18n.t("No host configured for this connection."));
      return;
    }

    let cancelled = false;
    let activeProxyId: string | null = null;
    setStatus("connecting");
    setErrorMsg("");
    stage.replaceChildren();

    remoteDesktopVncProxyStart({ host: tab.rdHost, port: tab.rdPort })
      .then((proxy) => {
        if (cancelled) {
          void remoteDesktopVncProxyStop(proxy.id).catch(() => {});
          return;
        }

        activeProxyId = proxy.id;
        proxyIdRef.current = proxy.id;
        const credentials = {
          username: tab.rdUser || undefined,
          password: tab.rdPassword || undefined,
        };
        const rfb = new RFB(stage, proxy.url, {
          credentials,
          shared: true,
        });
        rfbRef.current = rfb;
        applyVncPerformance(
          rfb,
          vncSettingsRef.current.mode,
          vncSettingsRef.current.resizeRemote,
        );
        installFrameProbe(rfb);

        const onConnect = () => {
          if (!cancelled) setStatus("connected");
        };
        const onDisconnect = (event: RFBDisconnectEvent) => {
          if (cancelled) return;
          restoreFrameProbeRef.current?.();
          restoreFrameProbeRef.current = null;
          rfbRef.current = null;
          proxyIdRef.current = null;
          setStatus("disconnected");
          setErrorMsg(event.detail.clean ? "" : i18n.t("Connection lost."));
          if (activeProxyId) {
            void remoteDesktopVncProxyStop(activeProxyId).catch(() => {});
            activeProxyId = null;
          }
        };
        const onSecurityFailure = (event: RFBSecurityFailureEvent) => {
          if (cancelled) return;
          setStatus("error");
          setErrorMsg(event.detail.reason || i18n.t("Security negotiation failed."));
        };
        const onCredentialsRequired = () => {
          rfb.sendCredentials(credentials);
        };
        const onClipboard = (event: RFBClipboardEvent) => {
          if (!isActiveRef.current || !event.detail.text) return;
          lastClipboardRef.current = event.detail.text;
          void writeClipboardText(event.detail.text);
        };

        rfb.addEventListener("connect", onConnect);
        rfb.addEventListener("disconnect", onDisconnect);
        rfb.addEventListener("securityfailure", onSecurityFailure);
        rfb.addEventListener("credentialsrequired", onCredentialsRequired);
        rfb.addEventListener("clipboard", onClipboard);

        if (isActiveRef.current) {
          window.requestAnimationFrame(() => rfb.focus());
        }
      })
      .catch((err) => {
        if (cancelled) return;
        setStatus("error");
        setErrorMsg(String(err));
      });

    return () => {
      cancelled = true;
      restoreFrameProbeRef.current?.();
      restoreFrameProbeRef.current = null;
      const rfb = rfbRef.current;
      rfbRef.current = null;
      if (rfb) rfb.disconnect();
      const proxyId = proxyIdRef.current ?? activeProxyId;
      proxyIdRef.current = null;
      if (proxyId) {
        void remoteDesktopVncProxyStop(proxyId).catch(() => {});
      }
      stage.replaceChildren();
    };
  }, [connectAttempt, installFrameProbe, tab.rdHost, tab.rdPassword, tab.rdPort, tab.rdUser]);

  useEffect(() => {
    isActiveRef.current = isActive;
  }, [isActive]);

  useEffect(() => {
    vncSettingsRef.current = { mode: vncMode, resizeRemote };
    try {
      localStorage.setItem(VNC_RENDER_MODE_KEY, vncMode);
      localStorage.setItem(VNC_REMOTE_RESIZE_KEY, resizeRemote ? "1" : "0");
    } catch {
      // Non-critical: private windows or storage blockers should not break VNC.
    }
    const rfb = rfbRef.current;
    if (rfb) applyVncPerformance(rfb, vncMode, resizeRemote);
  }, [vncMode, resizeRemote]);

  useEffect(() => {
    if (!showStats) return;

    let lastCount = frameCountRef.current;
    let lastSampleAt = performance.now();
    const sampleStats = () => {
      const now = performance.now();
      const frameCount = frameCountRef.current;
      const elapsedSeconds = Math.max((now - lastSampleAt) / 1000, 0.001);
      const rfb = rfbRef.current as NoVncRfbInternals | null;
      const width = rfb?._fbWidth ?? 0;
      const height = rfb?._fbHeight ?? 0;
      setStats({
        fps: (frameCount - lastCount) / elapsedSeconds,
        frames: frameCount,
        remoteSize: width > 0 && height > 0 ? `${width}×${height}` : "—",
        mode: vncSettingsRef.current.mode,
        resizeRemote: vncSettingsRef.current.resizeRemote,
        idleMs: lastFrameAtRef.current == null ? null : now - lastFrameAtRef.current,
      });
      lastCount = frameCount;
      lastSampleAt = now;
    };

    sampleStats();
    const interval = window.setInterval(sampleStats, 1000);
    return () => window.clearInterval(interval);
  }, [showStats]);

  useEffect(() => {
    const rfb = rfbRef.current;
    if (!rfb) return;
    if (!isActive) {
      rfb.blur();
      return;
    }

    const frame = window.requestAnimationFrame(() => {
      containerRef.current?.focus();
      rfb.focus();
    });
    void readClipboardText().then((text) => {
      if (text && text !== lastClipboardRef.current) {
        lastClipboardRef.current = text;
        rfb.clipboardPasteFrom(text);
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [isActive, status]);

  const errorHint = errorMsg ? diagnoseRemoteDesktopError(errorMsg) : null;

  return (
    <section
      className="rd-panel"
      style={{ display: isActive ? "flex" : "none" }}
      ref={containerRef}
      tabIndex={0}
    >
      <div className="rd-panel__header">
        <div className="rd-panel__title">
          <Monitor size={15} />
          <span>{tab.rdUser ? `${tab.rdUser}@${tab.rdHost}` : tab.rdHost}</span>
          <span className="rd-panel__proto">VNC</span>
        </div>
        <div className="rd-panel__meta">
          <div className="rd-panel__tools" role="group" aria-label={i18n.t("VNC performance")}>
            <button
              type="button"
              className={`mini-btn${vncMode === "latency" ? " is-active" : ""}`}
              title={i18n.t("Low latency mode")}
              aria-label={i18n.t("Low latency mode")}
              aria-pressed={vncMode === "latency"}
              onClick={() => setVncMode("latency")}
            >
              <Zap size={12} />
            </button>
            <button
              type="button"
              className={`mini-btn${vncMode === "quality" ? " is-active" : ""}`}
              title={i18n.t("Quality mode")}
              aria-label={i18n.t("Quality mode")}
              aria-pressed={vncMode === "quality"}
              onClick={() => setVncMode("quality")}
            >
              <Image size={12} />
            </button>
            <button
              type="button"
              className={`mini-btn${resizeRemote ? " is-active" : ""}`}
              title={i18n.t("Resize remote desktop to viewer")}
              aria-label={i18n.t("Resize remote desktop to viewer")}
              aria-pressed={resizeRemote}
              onClick={() => setResizeRemote((value) => !value)}
            >
              <Maximize2 size={12} />
            </button>
            <button
              type="button"
              className={`mini-btn${showStats ? " is-active" : ""}`}
              title={i18n.t(showStats ? "Hide VNC stats" : "Show VNC stats")}
              aria-label={i18n.t(showStats ? "Hide VNC stats" : "Show VNC stats")}
              aria-pressed={showStats}
              onClick={() => setShowStats((value) => !value)}
            >
              <Activity size={12} />
            </button>
          </div>
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

      <div className="rd-panel__stage">
        <div className="rd-novnc" ref={stageRef} />
        {showStats && (
          <div className="rd-stats" aria-hidden="true">
            <div className="rd-stats__row">
              <span>fps</span>
              <span className="rd-stats__value">{stats.fps.toFixed(1)}</span>
            </div>
            <div className="rd-stats__row">
              <span>remote</span>
              <span className="rd-stats__value">{stats.remoteSize}</span>
            </div>
            <div className="rd-stats__row">
              <span>mode</span>
              <span className="rd-stats__value">{stats.mode}</span>
            </div>
            <div className="rd-stats__row">
              <span>resize</span>
              <span className="rd-stats__value">{stats.resizeRemote ? "on" : "off"}</span>
            </div>
            <div className="rd-stats__row">
              <span>idle</span>
              <span className="rd-stats__value">
                {stats.idleMs == null ? "—" : `${Math.round(stats.idleMs)}ms`}
              </span>
            </div>
            <div className="rd-stats__row">
              <span>frames</span>
              <span className="rd-stats__value">{stats.frames}</span>
            </div>
          </div>
        )}
        {status !== "connected" && (
          <div className="rd-panel__overlay">
            {status === "connecting" && (
              <div className="rd-panel__overlay-card">
                <div className="rd-spinner" />
                <span>{i18n.t("Connecting to {host}…", { host: tab.rdHost })}</span>
              </div>
            )}
            {(status === "error" || status === "disconnected") && (
              <div className="rd-panel__overlay-card">
                <Monitor size={26} />
                <span className="rd-panel__overlay-title">
                  {status === "error" ? i18n.t("Connection failed") : i18n.t("Session ended")}
                </span>
                {errorHint && <span className="rd-panel__overlay-hint">{i18n.t(errorHint)}</span>}
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
