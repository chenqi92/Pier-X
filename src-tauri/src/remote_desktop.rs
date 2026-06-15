//! Tauri bridge for the remote-desktop (RDP / VNC) backends.
//!
//! Thin glue only: it owns no protocol logic. Each session lives in
//! `pier_core::remote_desktop`; here we
//!   * translate the frontend's connect request into a `RemoteDesktopConfig`,
//!   * install a [`FrameSink`] that packs each [`FrameEvent`] into a compact
//!     binary packet and ships it over a Tauri [`Channel`] as raw bytes
//!     (delivered to JS as an `ArrayBuffer`, never base64),
//!   * forward input / resize / close commands to the live session.
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

use std::sync::atomic::Ordering;

use serde::Deserialize;
use tauri::ipc::{Channel, Response};

use pier_core::remote_desktop::{
    CopyRect, FrameEvent, FrameSink, FrameTile, InputEvent, MouseButton, RemoteDesktopConfig,
    RemoteDesktopSession, RemoteProtocol, TileEncoding,
};

use crate::AppState;

/// One input action from the viewer canvas. Tagged union matching the
/// frontend's `RemoteInput` type.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub(crate) enum RdInput {
    PointerMove { x: u16, y: u16 },
    PointerButton { x: u16, y: u16, button: u8, pressed: bool },
    PointerScroll { x: u16, y: u16, dx: i16, dy: i16 },
    Key { keysym: u32, scancode: u16, extended: bool, pressed: bool },
    KeyUnicode { codepoint: u32, pressed: bool },
    SetClipboard { text: String },
}

impl RdInput {
    fn into_event(self) -> Option<InputEvent> {
        Some(match self {
            RdInput::PointerMove { x, y } => InputEvent::PointerMove { x, y },
            RdInput::PointerButton { x, y, button, pressed } => InputEvent::PointerButton {
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
            RdInput::Key { keysym, scancode, extended, pressed } => InputEvent::Key {
                keysym,
                scancode,
                extended,
                pressed,
            },
            RdInput::KeyUnicode { codepoint, pressed } => {
                InputEvent::KeyUnicode {
                    ch: char::from_u32(codepoint)?,
                    pressed,
                }
            }
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
        jpeg_threshold_px: RemoteDesktopConfig::DEFAULT_JPEG_THRESHOLD_PX,
    };

    // Pack every frame event into the binary wire format and push it down
    // the channel. Send failures (closed channel) are ignored — the session
    // task winds itself down when the viewer goes away.
    let channel = on_frame;
    let sink = FrameSink::new(move |event| {
        let _ = channel.send(Response::new(encode_packet(&event)));
    });

    let session = RemoteDesktopSession::connect(config, sink).map_err(|e| e.to_string())?;

    let id = format!("rd-{}", state.next_remote_desktop_id.fetch_add(1, Ordering::SeqCst));
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

// ── Binary packing ───────────────────────────────────────────────────────

fn put_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn encode_packet(event: &FrameEvent) -> Vec<u8> {
    let mut buf = Vec::new();
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
        FrameEvent::Tile(FrameTile { x, y, width, height, encoding, data }) => {
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
        FrameEvent::Copy(CopyRect { src_x, src_y, dst_x, dst_y, width, height }) => {
            buf.push(5);
            put_u16(&mut buf, *src_x);
            put_u16(&mut buf, *src_y);
            put_u16(&mut buf, *dst_x);
            put_u16(&mut buf, *dst_y);
            put_u16(&mut buf, *width);
            put_u16(&mut buf, *height);
        }
        FrameEvent::Cursor { width, height, hot_x, hot_y, data } => {
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
