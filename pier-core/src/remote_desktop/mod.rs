//! Remote-desktop clients (RDP + VNC).
//!
//! UI-agnostic, mirroring the `terminal` / `ssh` modules: a
//! [`RemoteDesktopSession`] owns a long-lived task on the shared async
//! runtime that drives one protocol connection. The task pushes
//! [`FrameEvent`]s out through a [`FrameSink`] callback the host installs,
//! and receives [`InputEvent`]s + control messages through channels. No
//! Tauri / UI types appear anywhere in this module.
//!
//! * **VNC** — implemented in-crate ([`vnc`]) as an RFB 3.8 client so we
//!   own the security handshake, including Apple ARD (RFB security type 30)
//!   used by modern macOS Screen Sharing.
//! * **RDP** — backed by IronRDP ([`rdp`], behind the `rdp` feature).

mod error;
mod frame;
mod input;
#[cfg(feature = "rdp")]
pub mod rdp;
#[cfg(feature = "rdp-freerdp")]
pub mod rdp_freerdp;
pub mod vnc;

pub use error::{RemoteDesktopError, Result};
pub use frame::{CopyRect, FrameEvent, FrameSink, FrameTile, TileEncoding};
pub use input::{InputEvent, MouseButton};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use crate::ssh::runtime;

/// Which wire protocol a session speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteProtocol {
    /// Microsoft Remote Desktop Protocol (Windows hosts).
    Rdp,
    /// RFB / VNC (Linux, macOS Screen Sharing, cross-platform servers).
    Vnc,
}

/// Everything needed to open one remote-desktop connection.
#[derive(Debug, Clone)]
pub struct RemoteDesktopConfig {
    /// Protocol to speak.
    pub protocol: RemoteProtocol,
    /// Host / IP (no scheme, no port).
    pub host: String,
    /// TCP port (RDP default 3389, VNC default 5900).
    pub port: u16,
    /// Login user. For VNC standard auth this may be empty (password only);
    /// for VNC ARD auth and for RDP it is the account name.
    pub username: String,
    /// Password / secret. Resolved from the keyring by the caller.
    pub password: String,
    /// RDP only: optional Windows domain / NetBIOS name.
    pub domain: Option<String>,
    /// Initial desktop width to request (RDP). VNC takes the server size.
    pub width: u16,
    /// Initial desktop height to request (RDP).
    pub height: u16,
    /// Tiles whose area (w*h) is >= this are JPEG-compressed before they hit
    /// the IPC channel; smaller rects ship raw RGBA. `0` = always raw.
    pub jpeg_threshold_px: u32,
}

impl RemoteDesktopConfig {
    /// Only re-encode genuinely large dirty rects to JPEG; everything below
    /// this area ships as lossless RGBA. The tile stream crosses a local IPC
    /// channel (not a network), so the win from JPEG is bounded while its cost
    /// — lossy text on every medium UI/scroll update — is not. A 256×256 floor
    /// keeps typical UI repaints and full-width text-line scrolls
    /// (e.g. 1920×~34) crisp, leaving JPEG for full-screen / photo updates.
    pub const DEFAULT_RDP_JPEG_THRESHOLD_PX: u32 = 256 * 256;
    /// VNC zlib/raw rectangles are often small and frequent. Keeping those
    /// raw avoids a costly inflate -> JPEG encode -> WebView JPEG decode loop.
    pub const DEFAULT_VNC_JPEG_THRESHOLD_PX: u32 = 256 * 256;
    /// Legacy default for callers that do not distinguish protocols.
    pub const DEFAULT_JPEG_THRESHOLD_PX: u32 = Self::DEFAULT_RDP_JPEG_THRESHOLD_PX;
}

/// Control messages the host sends to a live session.
pub(crate) enum ControlMsg {
    /// Viewer canvas resized — ask the server for a new desktop size where
    /// the protocol supports it (RDP DisplayControl; VNC SetDesktopSize).
    /// Server-side resize is a follow-up, so the fields are carried but not
    /// yet consumed by the backends.
    #[allow(dead_code)]
    Resize {
        /// Requested width in pixels.
        width: u16,
        /// Requested height in pixels.
        height: u16,
    },
    /// Tear the session down.
    Close,
}

/// A live remote-desktop session. Dropping it closes the connection.
pub struct RemoteDesktopSession {
    input_tx: UnboundedSender<InputEvent>,
    control_tx: UnboundedSender<ControlMsg>,
    width: u16,
    height: u16,
    task: Option<JoinHandle<()>>,
}

impl RemoteDesktopSession {
    /// Open a connection and start streaming. The protocol task runs on the
    /// shared runtime; `sink` receives every [`FrameEvent`] including the
    /// terminal [`FrameEvent::Disconnected`] when the task ends.
    pub fn connect(config: RemoteDesktopConfig, sink: FrameSink) -> Result<Self> {
        let (input_tx, input_rx) = unbounded_channel();
        let (control_tx, control_rx) = unbounded_channel();
        let width = config.width.max(1);
        let height = config.height.max(1);

        let task = runtime::shared().spawn(run_session(config, sink, input_rx, control_rx));

        Ok(Self {
            input_tx,
            control_tx,
            width,
            height,
            task: Some(task),
        })
    }

    /// Forward an input event. No-op if the session has already ended.
    pub fn send_input(&self, event: InputEvent) {
        let _ = self.input_tx.send(event);
    }

    /// Request a new desktop size (best-effort).
    pub fn resize(&self, width: u16, height: u16) {
        let _ = self.control_tx.send(ControlMsg::Resize { width, height });
    }

    /// Initial desktop dimensions requested at connect time.
    pub fn dimensions(&self) -> (u16, u16) {
        (self.width, self.height)
    }
}

impl Drop for RemoteDesktopSession {
    fn drop(&mut self) {
        // Ask the task to wind down, then abort as a backstop so a wedged
        // protocol read can't leak the connection.
        let _ = self.control_tx.send(ControlMsg::Close);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// The session entry point: dispatch to the protocol backend, then always
/// emit a terminal `Disconnected` so the host can update UI state.
async fn run_session(
    config: RemoteDesktopConfig,
    sink: FrameSink,
    input_rx: UnboundedReceiver<InputEvent>,
    control_rx: UnboundedReceiver<ControlMsg>,
) {
    let result = match config.protocol {
        RemoteProtocol::Vnc => vnc::run(config, sink.clone(), input_rx, control_rx).await,
        // FreeRDP supersedes IronRDP for the RDP protocol when its (experimental)
        // feature is compiled in — it adds the real H.264/AVC444 path IronRDP lacks.
        #[cfg(feature = "rdp-freerdp")]
        RemoteProtocol::Rdp => rdp_freerdp::run(config, sink.clone(), input_rx, control_rx).await,
        #[cfg(all(feature = "rdp", not(feature = "rdp-freerdp")))]
        RemoteProtocol::Rdp => rdp::run(config, sink.clone(), input_rx, control_rx).await,
        #[cfg(all(not(feature = "rdp"), not(feature = "rdp-freerdp")))]
        RemoteProtocol::Rdp => Err(RemoteDesktopError::Unsupported(
            "RDP support was not compiled into this build".to_string(),
        )),
    };

    let reason = match result {
        Ok(()) => None,
        Err(err) => Some(err.to_string()),
    };
    sink.emit(FrameEvent::Disconnected(reason));
}
