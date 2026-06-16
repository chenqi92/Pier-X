//! FreeRDP-backed RDP client (feature `rdp-freerdp`, EXPERIMENTAL — WIP).
//!
//! Why this exists: IronRDP 0.x has **no H.264** at all (verified against the
//! pinned sources — the only `h264` token in the tree is a test fixture), and
//! its `ActiveStage` only ever emits decoded dirty-rects, never an encoded
//! bitstream. So neither in-Rust H.264 nor forwarding to WebCodecs is possible
//! on that stack. FreeRDP 3 implements the MS-RDPEGFX H.264 graphics pipeline
//! (AVC420 + AVC444) with **OS-native hardware decoders** — Media Foundation
//! (Windows), VideoToolbox (macOS 3.26+), OpenH264/FFmpeg (Linux) — which is the
//! only no-agent path to video-grade RDP smoothness.
//!
//! ## Integration seam
//!
//! This backend plugs into the exact same protocol-agnostic seam as
//! [`super::rdp`] (IronRDP) and [`super::vnc`]: it owns one connection, pushes
//! [`super::frame::FrameEvent`]s through the host's [`FrameSink`], and consumes
//! [`InputEvent`]s + [`ControlMsg`]s. **Nothing in `src-tauri` or the frontend
//! changes** — FreeRDP just becomes another producer of the same tile stream.
//!
//! ## Data path (target: "Phase 2")
//!
//! libfreerdp3 decodes H.264 internally via the OS-native codec (HW-accelerated
//! where available) into a BGRA surface; its `gdi` / `RdpgfxClientContext`
//! surface callbacks hand us the dirty regions, which we slice and emit as
//! [`FrameEvent::Tile`]s exactly like the IronRDP path. (A later optimisation
//! could intercept the raw AVC NALs before FreeRDP decodes and forward them to a
//! WebView `VideoDecoder`, but AVC444's dual-stream layout makes that a separate
//! research task — not the first milestone.)
//!
//! ## Build contract (target: "Phase 1")
//!
//! Behind this feature, a `build.rs` bindgens the libfreerdp3 subset we use and
//! links a `cmake`-built libfreerdp3 (`WITH_GFX_H264=ON`, OS-native H.264
//! backend, OpenSSL/zlib as needed). The C library + its codec deps are bundled
//! per target as Tauri resources. None of that is wired yet — this module is the
//! placeholder the FFI lands in so the seam compiles today.

use tokio::sync::mpsc::UnboundedReceiver;

use super::error::{RemoteDesktopError, Result};
use super::frame::FrameSink;
use super::input::InputEvent;
use super::{ControlMsg, RemoteDesktopConfig};

/// Drive one FreeRDP connection to completion.
///
/// Currently a stub: the FFI to libfreerdp3 is not wired yet, so this reports an
/// actionable error instead of silently doing nothing. Enabling the
/// `rdp-freerdp` feature today compiles the seam but does not yet connect.
pub(crate) async fn run(
    _config: RemoteDesktopConfig,
    _sink: FrameSink,
    _input_rx: UnboundedReceiver<InputEvent>,
    _control_rx: UnboundedReceiver<ControlMsg>,
) -> Result<()> {
    Err(RemoteDesktopError::Unsupported(
        "FreeRDP RDP backend is not yet wired (feature `rdp-freerdp` is work in progress; \
         use the default IronRDP build for RDP)"
            .to_string(),
    ))
}
