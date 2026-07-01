//! Tauri bridge for the remote-desktop (RDP / VNC) backends.
//!
//! Thin glue only: it owns no protocol logic. Each session lives in
//! `pier_core::remote_desktop`; here we
//!   * translate the frontend's connect request into a `RemoteDesktopConfig`,
//!   * install a [`FrameSink`] that packs each [`FrameEvent`] into a compact
//!     binary packet and ships it over a Tauri [`Channel`] as raw bytes
//!     (delivered to JS as an `ArrayBuffer`, never base64),
//!   * forward input / resize / close commands to the live session,
//!   * expose a loopback WebSocket-to-TCP proxy so the WebView can run
//!     noVNC directly against ordinary VNC servers.
//!
//! Wire format of one frame packet (all integers little-endian):
//! ```text
//! kind=1 Connected   : u16 width, u16 height
//! kind=2 Resize      : u16 width, u16 height
//! kind=3 Tile (RGBA) : u16 x, u16 y, u16 w, u16 h, [w*h*4 RGBA bytes]
//! kind=4 Tile (JPEG) : u16 x, u16 y, u16 w, u16 h, [JPEG bytes]
//! kind=5 CopyRect    : u16 sx, u16 sy, u16 dx, u16 dy, u16 w, u16 h
//! kind=6 Cursor      : u16 w, u16 h, u16 hotX, u16 hotY, [w*h*4 RGBA bytes]
//! kind=7 Disconnected: u8 hasReason, [reason UTF-8 bytes]
//! kind=8 Clipboard   : [text UTF-8 bytes]
//! ```

use std::error::Error;
use std::sync::atomic::Ordering;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use pier_core::remote_desktop::{
    CopyRect, FrameEvent, FrameSink, FrameTile, InputEvent, MouseButton, RemoteDesktopConfig,
    RemoteDesktopSession, RemoteProtocol, TileEncoding,
};

use crate::AppState;

/// A local proxy process for one noVNC viewer. Dropping it cancels the accept
/// loop and aborts any pending listener task.
pub(crate) struct VncWebSocketProxy {
    stop: CancellationToken,
    task: tauri::async_runtime::JoinHandle<()>,
}

impl Drop for VncWebSocketProxy {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VncProxyInfo {
    id: String,
    url: String,
}

/// One input action from the viewer canvas. Tagged union matching the
/// frontend's `RemoteInput` type.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub(crate) enum RdInput {
    PointerMove {
        x: u16,
        y: u16,
    },
    PointerButton {
        x: u16,
        y: u16,
        button: u8,
        pressed: bool,
    },
    PointerScroll {
        x: u16,
        y: u16,
        dx: i16,
        dy: i16,
    },
    Key {
        keysym: u32,
        scancode: u16,
        extended: bool,
        pressed: bool,
    },
    KeyUnicode {
        codepoint: u32,
        pressed: bool,
    },
    SetClipboard {
        text: String,
    },
}

impl RdInput {
    fn into_event(self) -> Option<InputEvent> {
        Some(match self {
            RdInput::PointerMove { x, y } => InputEvent::PointerMove { x, y },
            RdInput::PointerButton {
                x,
                y,
                button,
                pressed,
            } => InputEvent::PointerButton {
                x,
                y,
                button: match button {
                    1 => MouseButton::Middle,
                    2 => MouseButton::Right,
                    _ => MouseButton::Left,
                },
                pressed,
            },
            RdInput::PointerScroll { x, y, dx, dy } => InputEvent::PointerScroll { x, y, dx, dy },
            RdInput::Key {
                keysym,
                scancode,
                extended,
                pressed,
            } => InputEvent::Key {
                keysym,
                scancode,
                extended,
                pressed,
            },
            RdInput::KeyUnicode { codepoint, pressed } => InputEvent::KeyUnicode {
                ch: char::from_u32(codepoint)?,
                pressed,
            },
            RdInput::SetClipboard { text } => InputEvent::SetClipboard(text),
        })
    }
}

/// Open a remote-desktop connection. Returns a session id used by the other
/// commands. Frames stream over `on_frame` as raw binary packets.
#[tauri::command]
pub fn remote_desktop_connect(
    state: tauri::State<'_, AppState>,
    protocol: String,
    host: String,
    port: u16,
    username: String,
    password: String,
    domain: Option<String>,
    width: u16,
    height: u16,
    on_frame: Channel<Response>,
) -> Result<String, String> {
    let protocol = match protocol.as_str() {
        "rdp" => RemoteProtocol::Rdp,
        "vnc" => RemoteProtocol::Vnc,
        other => return Err(format!("unknown remote-desktop protocol: {other}")),
    };
    let default_port = match protocol {
        RemoteProtocol::Rdp => 3389,
        RemoteProtocol::Vnc => 5900,
    };
    let config = RemoteDesktopConfig {
        protocol,
        host,
        port: if port == 0 { default_port } else { port },
        username,
        password,
        domain: domain.filter(|d| !d.is_empty()),
        width: width.max(640),
        height: height.max(480),
        jpeg_threshold_px: match protocol {
            RemoteProtocol::Rdp => RemoteDesktopConfig::DEFAULT_RDP_JPEG_THRESHOLD_PX,
            RemoteProtocol::Vnc => RemoteDesktopConfig::DEFAULT_VNC_JPEG_THRESHOLD_PX,
        },
    };

    // Pack every frame event into the binary wire format and push it down
    // the channel. Send failures (closed channel) are ignored — the session
    // task winds itself down when the viewer goes away.
    let channel = on_frame;
    let sink = FrameSink::new(move |event| {
        let _ = channel.send(Response::new(encode_packet(&event)));
    });

    // For RDP, route unknown / changed server certificates through the same
    // "trust this host?" dialog as SSH host keys (TOFU pinning). VNC has no
    // TLS layer, so it gets no prompt.
    let cert_prompt = match protocol {
        RemoteProtocol::Rdp => crate::host_key_prompt_cb(),
        RemoteProtocol::Vnc => None,
    };
    let session =
        RemoteDesktopSession::connect(config, sink, cert_prompt).map_err(|e| e.to_string())?;

    let id = format!(
        "rd-{}",
        state.next_remote_desktop_id.fetch_add(1, Ordering::SeqCst)
    );
    state
        .remote_desktops
        .lock()
        .map_err(|_| "remote desktop registry poisoned".to_string())?
        .insert(id.clone(), session);
    Ok(id)
}

/// Forward one input event to a live session.
#[tauri::command]
pub fn remote_desktop_input(
    state: tauri::State<'_, AppState>,
    session_id: String,
    event: RdInput,
) -> Result<(), String> {
    let Some(event) = event.into_event() else {
        return Ok(());
    };
    let sessions = state
        .remote_desktops
        .lock()
        .map_err(|_| "remote desktop registry poisoned".to_string())?;
    if let Some(session) = sessions.get(&session_id) {
        session.send_input(event);
    }
    Ok(())
}

/// Forward a batch of input events to a live session, in arrival order. The
/// viewer coalesces a frame's worth of pointer / keyboard events into one call
/// so high-rate input does not pay a per-event main-thread hop (these commands
/// are synchronous). A single ordered batch also avoids the reordering a naive
/// concurrent-async command stream would cause.
#[tauri::command]
pub fn remote_desktop_input_batch(
    state: tauri::State<'_, AppState>,
    session_id: String,
    events: Vec<RdInput>,
) -> Result<(), String> {
    let sessions = state
        .remote_desktops
        .lock()
        .map_err(|_| "remote desktop registry poisoned".to_string())?;
    if let Some(session) = sessions.get(&session_id) {
        for event in events {
            if let Some(event) = event.into_event() {
                session.send_input(event);
            }
        }
    }
    Ok(())
}

/// Request a new desktop size (best-effort; protocol-dependent).
#[tauri::command]
pub fn remote_desktop_resize(
    state: tauri::State<'_, AppState>,
    session_id: String,
    width: u16,
    height: u16,
) -> Result<(), String> {
    let sessions = state
        .remote_desktops
        .lock()
        .map_err(|_| "remote desktop registry poisoned".to_string())?;
    if let Some(session) = sessions.get(&session_id) {
        session.resize(width, height);
    }
    Ok(())
}

/// Tear a session down and free its connection.
#[tauri::command]
pub fn remote_desktop_close(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    let mut sessions = state
        .remote_desktops
        .lock()
        .map_err(|_| "remote desktop registry poisoned".to_string())?;
    // Dropping the session sends Close + aborts its task.
    sessions.remove(&session_id);
    Ok(())
}

/// Start a loopback WebSocket proxy for noVNC. noVNC speaks RFB over
/// WebSocket; most VNC servers still expose plain TCP, so the desktop shell
/// provides the narrow bridge locally.
#[tauri::command]
pub async fn remote_desktop_vnc_proxy_start(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
) -> Result<VncProxyInfo, String> {
    let target_port = if port == 0 { 5900 } else { port };
    let target = format!("{host}:{target_port}");
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("bind local VNC proxy: {e}"))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| format!("read local VNC proxy address: {e}"))?;
    let id = format!(
        "vnc-proxy-{}",
        state
            .next_remote_desktop_proxy_id
            .fetch_add(1, Ordering::SeqCst)
    );
    let stop = CancellationToken::new();
    let task_stop = stop.clone();
    let task = tauri::async_runtime::spawn(async move {
        run_vnc_websocket_proxy(listener, target, task_stop).await;
    });

    state
        .remote_desktop_proxies
        .lock()
        .map_err(|_| "remote desktop proxy registry poisoned".to_string())?
        .insert(id.clone(), VncWebSocketProxy { stop, task });

    Ok(VncProxyInfo {
        id,
        url: format!("ws://{local_addr}"),
    })
}

/// Stop a noVNC loopback proxy.
#[tauri::command]
pub fn remote_desktop_vnc_proxy_stop(
    state: tauri::State<'_, AppState>,
    proxy_id: String,
) -> Result<(), String> {
    state
        .remote_desktop_proxies
        .lock()
        .map_err(|_| "remote desktop proxy registry poisoned".to_string())?
        .remove(&proxy_id);
    Ok(())
}

async fn run_vnc_websocket_proxy(listener: TcpListener, target: String, stop: CancellationToken) {
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            accepted = listener.accept() => {
                let Ok((stream, _peer)) = accepted else {
                    break;
                };
                let target = target.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = bridge_vnc_websocket(stream, target).await;
                });
            }
        }
    }
}

async fn bridge_vnc_websocket(
    stream: TcpStream,
    target: String,
) -> std::result::Result<(), Box<dyn Error + Send + Sync>> {
    stream.set_nodelay(true).ok();
    let websocket = accept_async(stream).await?;
    let remote = TcpStream::connect(target).await?;
    remote.set_nodelay(true).ok();

    let (mut ws_tx, mut ws_rx) = websocket.split();
    let (mut remote_rx, mut remote_tx) = remote.into_split();

    let websocket_to_remote = async {
        while let Some(message) = ws_rx.next().await {
            match message? {
                Message::Binary(data) => remote_tx.write_all(data.as_ref()).await?,
                Message::Text(text) => remote_tx.write_all(text.as_bytes()).await?,
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
        let _ = remote_tx.shutdown().await;
        Ok::<(), Box<dyn Error + Send + Sync>>(())
    };

    let remote_to_websocket = async {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            let n = remote_rx.read(&mut buf).await?;
            if n == 0 {
                let _ = ws_tx.send(Message::Close(None)).await;
                break;
            }
            ws_tx
                .send(Message::Binary(buf[..n].to_vec().into()))
                .await?;
        }
        Ok::<(), Box<dyn Error + Send + Sync>>(())
    };

    tokio::select! {
        result = websocket_to_remote => result,
        result = remote_to_websocket => result,
    }
}

// ── Binary packing ───────────────────────────────────────────────────────

fn put_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn encode_packet(event: &FrameEvent) -> Vec<u8> {
    // Pre-size for the big payloads (tile / cursor RGBA, clipboard text) so the
    // final extend does not grow a zero-capacity buffer through several reallocs.
    let hint = match event {
        FrameEvent::Tile(FrameTile { data, .. }) => data.len() + 9,
        FrameEvent::Cursor { data, .. } => data.len() + 9,
        FrameEvent::Clipboard(text) => text.len() + 1,
        _ => 16,
    };
    let mut buf = Vec::with_capacity(hint);
    match event {
        FrameEvent::Connected { width, height } => {
            buf.push(1);
            put_u16(&mut buf, *width);
            put_u16(&mut buf, *height);
        }
        FrameEvent::Resize { width, height } => {
            buf.push(2);
            put_u16(&mut buf, *width);
            put_u16(&mut buf, *height);
        }
        FrameEvent::Tile(FrameTile {
            x,
            y,
            width,
            height,
            encoding,
            data,
        }) => {
            buf.push(match encoding {
                TileEncoding::Rgba => 3,
                TileEncoding::Jpeg => 4,
            });
            put_u16(&mut buf, *x);
            put_u16(&mut buf, *y);
            put_u16(&mut buf, *width);
            put_u16(&mut buf, *height);
            buf.extend_from_slice(data);
        }
        FrameEvent::Copy(CopyRect {
            src_x,
            src_y,
            dst_x,
            dst_y,
            width,
            height,
        }) => {
            buf.push(5);
            put_u16(&mut buf, *src_x);
            put_u16(&mut buf, *src_y);
            put_u16(&mut buf, *dst_x);
            put_u16(&mut buf, *dst_y);
            put_u16(&mut buf, *width);
            put_u16(&mut buf, *height);
        }
        FrameEvent::Cursor {
            width,
            height,
            hot_x,
            hot_y,
            data,
        } => {
            buf.push(6);
            put_u16(&mut buf, *width);
            put_u16(&mut buf, *height);
            put_u16(&mut buf, *hot_x);
            put_u16(&mut buf, *hot_y);
            buf.extend_from_slice(data);
        }
        FrameEvent::Disconnected(reason) => {
            buf.push(7);
            match reason {
                Some(r) => {
                    buf.push(1);
                    buf.extend_from_slice(r.as_bytes());
                }
                None => buf.push(0),
            }
        }
        FrameEvent::Clipboard(text) => {
            buf.push(8);
            buf.extend_from_slice(text.as_bytes());
        }
    }
    buf
}
