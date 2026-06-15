// ── Remote-desktop IPC wrappers ──────────────────────────────────────────
// Typed bridge to the `remote_desktop_*` Tauri commands. Frames arrive over a
// binary Channel as compact packets (see src-tauri/src/remote_desktop.rs for
// the wire format); we parse them here and hand structured packets to the
// panel. Input goes back as `invoke("remote_desktop_input", …)`.

import { Channel, invoke } from "@tauri-apps/api/core";
import { resolveKey } from "./remoteDesktopKeys";

export type RemoteDesktopProtocol = "rdp" | "vnc";

export type RemoteDesktopConnectParams = {
  protocol: RemoteDesktopProtocol;
  host: string;
  port: number;
  username: string;
  password: string;
  domain?: string | null;
  width: number;
  height: number;
};

export type VncProxyInfo = {
  id: string;
  url: string;
};

/** A parsed frame packet pushed from the backend. */
export type RdPacket =
  | { kind: "connected"; width: number; height: number }
  | { kind: "resize"; width: number; height: number }
  | {
      kind: "tile";
      x: number;
      y: number;
      width: number;
      height: number;
      encoding: "rgba" | "jpeg";
      data: Uint8Array;
    }
  | {
      kind: "copy";
      srcX: number;
      srcY: number;
      dstX: number;
      dstY: number;
      width: number;
      height: number;
    }
  | {
      kind: "cursor";
      width: number;
      height: number;
      hotX: number;
      hotY: number;
      data: Uint8Array;
    }
  | { kind: "disconnected"; reason: string | null }
  | { kind: "clipboard"; text: string };

/** Input action sent to the backend. Field `kind` is the serde tag. */
export type RemoteInput =
  | { kind: "pointerMove"; x: number; y: number }
  | { kind: "pointerButton"; x: number; y: number; button: number; pressed: boolean }
  | { kind: "pointerScroll"; x: number; y: number; dx: number; dy: number }
  | { kind: "key"; keysym: number; scancode: number; extended: boolean; pressed: boolean }
  | { kind: "keyUnicode"; codepoint: number; pressed: boolean }
  | { kind: "setClipboard"; text: string };

const td = new TextDecoder();

/** Normalise whatever the Channel delivered (ArrayBuffer for large payloads,
 *  a typed-array or number[] for tiny ones) into a `Uint8Array`. */
function toBytes(msg: unknown): Uint8Array | null {
  if (msg instanceof ArrayBuffer) return new Uint8Array(msg);
  if (ArrayBuffer.isView(msg)) return new Uint8Array(msg.buffer, msg.byteOffset, msg.byteLength);
  if (Array.isArray(msg)) return new Uint8Array(msg as number[]);
  return null;
}

/** Parse one binary frame packet. Returns `null` for an unknown tag. */
export function parseRemoteDesktopPacket(bytes: Uint8Array): RdPacket | null {
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tag = dv.getUint8(0);
  let o = 1;
  const u16 = () => {
    const v = dv.getUint16(o, true);
    o += 2;
    return v;
  };
  switch (tag) {
    case 1:
      return { kind: "connected", width: u16(), height: u16() };
    case 2:
      return { kind: "resize", width: u16(), height: u16() };
    case 3:
    case 4: {
      const x = u16();
      const y = u16();
      const width = u16();
      const height = u16();
      return {
        kind: "tile",
        x,
        y,
        width,
        height,
        encoding: tag === 3 ? "rgba" : "jpeg",
        data: bytes.subarray(o),
      };
    }
    case 5:
      return {
        kind: "copy",
        srcX: u16(),
        srcY: u16(),
        dstX: u16(),
        dstY: u16(),
        width: u16(),
        height: u16(),
      };
    case 6: {
      const width = u16();
      const height = u16();
      const hotX = u16();
      const hotY = u16();
      return { kind: "cursor", width, height, hotX, hotY, data: bytes.subarray(o) };
    }
    case 7: {
      const hasReason = dv.getUint8(o);
      o += 1;
      const reason = hasReason ? td.decode(bytes.subarray(o)) : null;
      return { kind: "disconnected", reason };
    }
    case 8:
      return { kind: "clipboard", text: td.decode(bytes.subarray(o)) };
    default:
      return null;
  }
}

/** Open a remote-desktop session. Packets are delivered to `onPacket`; the
 *  returned promise resolves to the session id used by the other calls. */
export async function remoteDesktopConnect(
  params: RemoteDesktopConnectParams,
  onPacket: (packet: RdPacket) => void,
): Promise<string> {
  const channel = new Channel<unknown>();
  channel.onmessage = (msg) => {
    const bytes = toBytes(msg);
    if (!bytes || bytes.byteLength === 0) return;
    const packet = parseRemoteDesktopPacket(bytes);
    if (packet) onPacket(packet);
  };
  return invoke<string>("remote_desktop_connect", {
    protocol: params.protocol,
    host: params.host,
    port: params.port,
    username: params.username,
    password: params.password,
    domain: params.domain ?? null,
    width: params.width,
    height: params.height,
    onFrame: channel,
  });
}

export const remoteDesktopInput = (sessionId: string, event: RemoteInput) =>
  invoke<void>("remote_desktop_input", { sessionId, event });

export const remoteDesktopResize = (sessionId: string, width: number, height: number) =>
  invoke<void>("remote_desktop_resize", { sessionId, width, height });

export const remoteDesktopClose = (sessionId: string) =>
  invoke<void>("remote_desktop_close", { sessionId });

export const remoteDesktopVncProxyStart = (params: { host: string; port: number }) =>
  invoke<VncProxyInfo>("remote_desktop_vnc_proxy_start", params);

export const remoteDesktopVncProxyStop = (proxyId: string) =>
  invoke<void>("remote_desktop_vnc_proxy_stop", { proxyId });

/** Build a key input from a DOM `KeyboardEvent`, or `null` if unmappable. */
export function keyToInput(e: KeyboardEvent, pressed: boolean): RemoteInput | null {
  const resolved = resolveKey(e);
  if (!resolved) return null;
  return {
    kind: "key",
    keysym: resolved.keysym,
    scancode: resolved.scancode,
    extended: resolved.extended,
    pressed,
  };
}
