//! Frame / event types pushed from a backend up to the host (src-tauri).
//!
//! A backend never talks to a UI. It pushes [`FrameEvent`]s through a
//! [`FrameSink`] callback; the Tauri layer turns those into a binary
//! `Channel` stream for the WebView. Pixel tiles are delivered already in a
//! WebView-friendly form: RGBA (drop-in for `canvas.putImageData`) or JPEG
//! (decode with `createImageBitmap`).

use std::sync::Arc;

/// Pixel encoding of a [`FrameTile`] payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileEncoding {
    /// Raw little-endian RGBA, 4 bytes/pixel, `width*height*4` bytes,
    /// row-major, alpha forced to 255. Maps 1:1 onto canvas `ImageData`.
    Rgba,
    /// A complete baseline JPEG of the tile. Opaque; decode with the
    /// platform image decoder (`createImageBitmap`).
    Jpeg,
}

/// One dirty rectangle of the remote framebuffer, ready to paint.
#[derive(Debug, Clone)]
pub struct FrameTile {
    /// Left edge in the remote framebuffer.
    pub x: u16,
    /// Top edge in the remote framebuffer.
    pub y: u16,
    /// Tile width in pixels.
    pub width: u16,
    /// Tile height in pixels.
    pub height: u16,
    /// How `data` is encoded.
    pub encoding: TileEncoding,
    /// The encoded pixels.
    pub data: Vec<u8>,
}

/// A "copy existing region" op (VNC CopyRect / scroll). No pixel payload —
/// the frontend blits the already-painted source rect to the destination.
#[derive(Debug, Clone, Copy)]
pub struct CopyRect {
    /// Source left.
    pub src_x: u16,
    /// Source top.
    pub src_y: u16,
    /// Destination left.
    pub dst_x: u16,
    /// Destination top.
    pub dst_y: u16,
    /// Region width.
    pub width: u16,
    /// Region height.
    pub height: u16,
}

/// Events emitted by a running session, in arrival order.
#[derive(Debug, Clone)]
pub enum FrameEvent {
    /// Handshake done; the framebuffer size is known. Sent once, before
    /// any tiles. The frontend allocates its canvas from this.
    Connected {
        /// Desktop width in pixels.
        width: u16,
        /// Desktop height in pixels.
        height: u16,
    },
    /// The desktop was resized (VNC DesktopSize / RDP reactivation).
    Resize {
        /// New width in pixels.
        width: u16,
        /// New height in pixels.
        height: u16,
    },
    /// A dirty-rect tile to paint.
    Tile(FrameTile),
    /// Blit an on-screen region elsewhere (scroll / window move).
    Copy(CopyRect),
    /// Cursor shape update. `data` is RGBA (`width*height*4`); `(hot_x,
    /// hot_y)` is the click hotspot inside the bitmap.
    Cursor {
        /// Cursor bitmap width.
        width: u16,
        /// Cursor bitmap height.
        height: u16,
        /// Hotspot X within the bitmap.
        hot_x: u16,
        /// Hotspot Y within the bitmap.
        hot_y: u16,
        /// RGBA pixels, `width*height*4` bytes.
        data: Vec<u8>,
    },
    /// Server clipboard text is available (the user can paste it locally).
    Clipboard(String),
    /// The session ended. `None` = clean close; `Some(reason)` = error.
    Disconnected(Option<String>),
}

/// A cloneable, thread-safe sink the host installs to receive
/// [`FrameEvent`]s. Keeps `pier-core` UI-agnostic: it never sees Tauri.
#[derive(Clone)]
pub struct FrameSink(Arc<dyn Fn(FrameEvent) + Send + Sync + 'static>);

impl FrameSink {
    /// Wrap a callback as a sink.
    pub fn new(f: impl Fn(FrameEvent) + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    /// Deliver one event to the host.
    pub fn emit(&self, event: FrameEvent) {
        (self.0)(event);
    }
}

impl std::fmt::Debug for FrameSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FrameSink(..)")
    }
}

/// JPEG quality (0–100) used when a backend re-encodes a raw tile.
pub const JPEG_QUALITY: u8 = 80;

/// Encode a raw RGBA tile to a [`FrameTile`]. Tiles whose pixel area is at
/// or above `jpeg_threshold_px` (and ≥ 16×16) are JPEG-compressed to keep
/// the IPC stream small; smaller rects ship as raw RGBA. `jpeg_threshold_px
/// == 0` forces raw.
///
/// `rgba` must be `width*height*4` bytes with alpha already set to 255.
pub fn encode_tile(
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    rgba: Vec<u8>,
    jpeg_threshold_px: u32,
) -> FrameTile {
    let area = u32::from(width) * u32::from(height);
    let want_jpeg = jpeg_threshold_px != 0 && area >= jpeg_threshold_px && width >= 16 && height >= 16;
    if want_jpeg {
        if let Some(jpeg) = rgba_to_jpeg(&rgba, width, height, JPEG_QUALITY) {
            return FrameTile { x, y, width, height, encoding: TileEncoding::Jpeg, data: jpeg };
        }
    }
    FrameTile { x, y, width, height, encoding: TileEncoding::Rgba, data: rgba }
}

/// Compress an RGBA buffer to a baseline JPEG (alpha dropped). Returns
/// `None` if encoding fails (the caller falls back to raw RGBA).
fn rgba_to_jpeg(rgba: &[u8], width: u16, height: u16, quality: u8) -> Option<Vec<u8>> {
    let w = u32::from(width);
    let h = u32::from(height);
    let expected = (w as usize) * (h as usize) * 4;
    if rgba.len() < expected {
        return None;
    }
    // Pack RGB (JPEG has no alpha).
    let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for px in rgba[..expected].chunks_exact(4) {
        rgb.push(px[0]);
        rgb.push(px[1]);
        rgb.push(px[2]);
    }
    let mut out: Vec<u8> = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    encoder
        .encode(&rgb, w, h, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(out)
}
