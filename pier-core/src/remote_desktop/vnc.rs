//! In-crate RFB / VNC 3.8 client.
//!
//! We implement the client ourselves (rather than depending on a VNC crate)
//! for one reason: **Apple ARD authentication** (RFB security type 30), which
//! modern macOS "Screen Sharing" requires and which no published Rust VNC
//! crate supports. Owning the handshake lets us add it (Diffie-Hellman key
//! agreement → MD5 → AES-128-ECB of the credential block) alongside the
//! standard None (1) and VNC-Auth (2, DES challenge) types.
//!
//! Supported framebuffer encodings: Raw, CopyRect, Zlib, plus the
//! DesktopSize and Cursor pseudo-encodings. Tight / ZRLE are not implemented;
//! the server falls back to Raw (which every RFB server must support).

use std::io::ErrorKind;
use std::sync::Arc;

use flate2::{Decompress, FlushDecompress, Status};
use num_bigint::BigUint;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{mpsc, Mutex};
use tokio::sync::mpsc::{UnboundedReceiver};

use super::error::{RemoteDesktopError, Result};
use super::frame::{self, CopyRect, FrameEvent, FrameSink};
use super::input::{InputEvent, MouseButton};
use super::{ControlMsg, RemoteDesktopConfig};
use crate::ssh::runtime;

// ── RFB framebuffer encodings (i32 on the wire) ──────────────────────────
const ENC_RAW: i32 = 0;
const ENC_COPY_RECT: i32 = 1;
const ENC_ZLIB: i32 = 6;
const ENC_CURSOR: i32 = -239; // 0xFFFF_FF11 pseudo-encoding
const ENC_DESKTOP_SIZE: i32 = -223; // 0xFFFF_FF21 pseudo-encoding

// ── Security types ───────────────────────────────────────────────────────
const SEC_NONE: u8 = 1;
const SEC_VNC_AUTH: u8 = 2;
const SEC_ARD: u8 = 30; // Apple Remote Desktop / macOS Screen Sharing

/// Drive one VNC connection to completion. Returns `Ok(())` on a clean close.
pub(crate) async fn run(
    config: RemoteDesktopConfig,
    sink: FrameSink,
    mut input_rx: UnboundedReceiver<InputEvent>,
    mut control_rx: UnboundedReceiver<ControlMsg>,
) -> Result<()> {
    let addr = format!("{}:{}", config.host, config.port);
    let stream = TcpStream::connect(&addr)
        .await
        .map_err(|e| RemoteDesktopError::Connect(format!("{addr}: {e}")))?;
    stream.set_nodelay(true).ok();
    let (rd, mut wr) = stream.into_split();
    let mut rd = BufReader::new(rd);

    // ── ProtocolVersion ──────────────────────────────────────────────
    let mut server_ver = [0u8; 12];
    rd.read_exact(&mut server_ver)
        .await
        .map_err(|e| RemoteDesktopError::Connect(format!("reading version: {e}")))?;
    let server_minor = parse_minor(&server_ver);
    let use_38 = server_minor >= 7;
    let client_ver: &[u8] = if use_38 { b"RFB 003.008\n" } else { b"RFB 003.003\n" };
    wr.write_all(client_ver).await?;

    // ── Security negotiation ─────────────────────────────────────────
    let sec_type = if use_38 {
        let count = rd.read_u8().await?;
        if count == 0 {
            let reason = read_string_u32(&mut rd).await.unwrap_or_default();
            return Err(RemoteDesktopError::Auth(format!(
                "server refused connection: {reason}"
            )));
        }
        let mut types = vec![0u8; count as usize];
        rd.read_exact(&mut types).await?;
        let chosen = choose_security(&types)?;
        wr.write_all(&[chosen]).await?;
        chosen
    } else {
        // RFB 3.3: the server dictates a single u32 security type.
        let t = rd.read_u32().await? as u8;
        t
    };

    // ── Authentication ───────────────────────────────────────────────
    match sec_type {
        SEC_NONE => {}
        SEC_VNC_AUTH => vnc_auth(&mut rd, &mut wr, &config.password).await?,
        SEC_ARD => ard_auth(&mut rd, &mut wr, &config.username, &config.password).await?,
        other => {
            return Err(RemoteDesktopError::Unsupported(format!(
                "VNC security type {other} is not supported"
            )))
        }
    }

    // SecurityResult: always present in 3.8; in 3.3 only after VNC-Auth.
    if use_38 || sec_type == SEC_VNC_AUTH {
        let result = rd.read_u32().await?;
        if result != 0 {
            let reason = if use_38 {
                read_string_u32(&mut rd).await.unwrap_or_default()
            } else {
                String::new()
            };
            return Err(RemoteDesktopError::Auth(if reason.is_empty() {
                "authentication failed".to_string()
            } else {
                reason
            }));
        }
    }

    // ── ClientInit (request a shared session) ────────────────────────
    wr.write_all(&[1u8]).await?;

    // ── ServerInit ───────────────────────────────────────────────────
    let fb_w = rd.read_u16().await?;
    let fb_h = rd.read_u16().await?;
    let mut pixel_format = [0u8; 16];
    rd.read_exact(&mut pixel_format).await?;
    let _name = read_string_u32(&mut rd).await.unwrap_or_default();

    // Ask the server to send pixels in our canonical RGBA layout, and tell
    // it which encodings we understand.
    send_set_pixel_format(&mut wr).await?;
    send_set_encodings(&mut wr).await?;

    sink.emit(FrameEvent::Connected { width: fb_w, height: fb_h });

    // Kick off with one full update; the read loop requests incrementals.
    send_fbur(&mut wr, false, 0, 0, fb_w, fb_h).await?;

    // ── Split: a dedicated read task (cancellation-safe sequential reads)
    // plus this task handling input / control. Both share the write half
    // behind a Mutex (writes are tiny and rare, so contention is nil). ──
    let wr = Arc::new(Mutex::new(wr));
    let (dead_tx, mut dead_rx) = mpsc::channel::<Option<String>>(1);
    let read_wr = wr.clone();
    let read_sink = sink.clone();
    let jpeg_threshold = config.jpeg_threshold_px;
    let reader = runtime::shared().spawn(async move {
        let outcome = read_loop(rd, read_wr, read_sink, fb_w, fb_h, jpeg_threshold).await;
        let reason = match outcome {
            Ok(()) => None,
            Err(e) => Some(e.to_string()),
        };
        let _ = dead_tx.send(reason).await;
    });

    let mut pointer = PointerState::default();
    let result = loop {
        tokio::select! {
            maybe = input_rx.recv() => match maybe {
                Some(ev) => {
                    let mut guard = wr.lock().await;
                    if let Err(e) = write_input(&mut *guard, &ev, &mut pointer).await {
                        break Err(e);
                    }
                }
                None => break Ok(()),
            },
            maybe = control_rx.recv() => match maybe {
                Some(ControlMsg::Close) | None => break Ok(()),
                // SetDesktopSize (server-side resize) is not implemented in
                // v1 — the viewer simply scales the canvas locally.
                Some(ControlMsg::Resize { .. }) => {}
            },
            dead = dead_rx.recv() => match dead {
                Some(Some(reason)) => break Err(RemoteDesktopError::Protocol(reason)),
                Some(None) | None => break Ok(()),
            },
        }
    };

    reader.abort();
    result
}

// ── Read loop ────────────────────────────────────────────────────────────

async fn read_loop<R: AsyncRead + Unpin>(
    mut rd: R,
    wr: Arc<Mutex<OwnedWriteHalf>>,
    sink: FrameSink,
    mut fb_w: u16,
    mut fb_h: u16,
    jpeg_threshold: u32,
) -> Result<()> {
    // One persistent zlib stream for the whole connection (the VNC Zlib
    // encoding keeps a single sliding window across every rectangle).
    let mut zlib = Decompress::new(true);

    loop {
        let msg_type = match rd.read_u8().await {
            Ok(t) => t,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        match msg_type {
            0 => {
                // FramebufferUpdate
                let _padding = rd.read_u8().await?;
                let rect_count = rd.read_u16().await?;
                let mut need_full = false;
                for _ in 0..rect_count {
                    let x = rd.read_u16().await?;
                    let y = rd.read_u16().await?;
                    let w = rd.read_u16().await?;
                    let h = rd.read_u16().await?;
                    let encoding = rd.read_i32().await?;
                    match encoding {
                        ENC_RAW => read_raw_rect(&mut rd, &sink, x, y, w, h, jpeg_threshold).await?,
                        ENC_ZLIB => {
                            read_zlib_rect(&mut rd, &mut zlib, &sink, x, y, w, h, jpeg_threshold)
                                .await?
                        }
                        ENC_COPY_RECT => {
                            let src_x = rd.read_u16().await?;
                            let src_y = rd.read_u16().await?;
                            sink.emit(FrameEvent::Copy(CopyRect {
                                src_x,
                                src_y,
                                dst_x: x,
                                dst_y: y,
                                width: w,
                                height: h,
                            }));
                        }
                        ENC_DESKTOP_SIZE => {
                            fb_w = w;
                            fb_h = h;
                            need_full = true;
                            sink.emit(FrameEvent::Resize { width: w, height: h });
                        }
                        ENC_CURSOR => read_cursor(&mut rd, &sink, x, y, w, h).await?,
                        other => {
                            return Err(RemoteDesktopError::Protocol(format!(
                                "server used unsupported encoding {other}"
                            )))
                        }
                    }
                }
                // Pump the next frame. After a desktop resize we need a full
                // (non-incremental) refresh of the new geometry.
                let mut guard = wr.lock().await;
                send_fbur(&mut *guard, !need_full, 0, 0, fb_w, fb_h).await?;
            }
            1 => {
                // SetColourMapEntries — we use true-colour, so just skip it.
                let _padding = rd.read_u8().await?;
                let _first = rd.read_u16().await?;
                let count = rd.read_u16().await?;
                let mut skip = vec![0u8; count as usize * 6];
                rd.read_exact(&mut skip).await?;
            }
            2 => { /* Bell — no UI hook yet */ }
            3 => {
                // ServerCutText (clipboard) — latin-1 text.
                let mut pad = [0u8; 3];
                rd.read_exact(&mut pad).await?;
                let len = rd.read_u32().await?;
                let mut buf = vec![0u8; len as usize];
                rd.read_exact(&mut buf).await?;
                let text: String = buf.iter().map(|&b| b as char).collect();
                sink.emit(FrameEvent::Clipboard(text));
            }
            other => {
                return Err(RemoteDesktopError::Protocol(format!(
                    "unknown server message type {other}"
                )))
            }
        }
    }
}

async fn read_raw_rect<R: AsyncRead + Unpin>(
    rd: &mut R,
    sink: &FrameSink,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    jpeg_threshold: u32,
) -> Result<()> {
    if w == 0 || h == 0 {
        return Ok(());
    }
    let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
    rd.read_exact(&mut buf).await?;
    force_opaque(&mut buf);
    sink.emit(FrameEvent::Tile(frame::encode_tile(x, y, w, h, buf, jpeg_threshold)));
    Ok(())
}

async fn read_zlib_rect<R: AsyncRead + Unpin>(
    rd: &mut R,
    zlib: &mut Decompress,
    sink: &FrameSink,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    jpeg_threshold: u32,
) -> Result<()> {
    let comp_len = rd.read_u32().await? as usize;
    let mut comp = vec![0u8; comp_len];
    rd.read_exact(&mut comp).await?;
    if w == 0 || h == 0 {
        return Ok(());
    }
    let expected = (w as usize) * (h as usize) * 4;
    let mut out = vec![0u8; expected];
    let mut filled = 0usize;
    let mut consumed = 0usize;
    while filled < expected {
        let before_in = zlib.total_in();
        let before_out = zlib.total_out();
        let status = zlib
            .decompress(&comp[consumed..], &mut out[filled..], FlushDecompress::Sync)
            .map_err(|e| RemoteDesktopError::Protocol(format!("zlib inflate: {e}")))?;
        consumed += (zlib.total_in() - before_in) as usize;
        filled += (zlib.total_out() - before_out) as usize;
        if matches!(status, Status::BufError) || (zlib.total_in() == before_in && zlib.total_out() == before_out) {
            break;
        }
    }
    force_opaque(&mut out);
    sink.emit(FrameEvent::Tile(frame::encode_tile(x, y, w, h, out, jpeg_threshold)));
    Ok(())
}

async fn read_cursor<R: AsyncRead + Unpin>(
    rd: &mut R,
    sink: &FrameSink,
    hot_x: u16,
    hot_y: u16,
    w: u16,
    h: u16,
) -> Result<()> {
    let pixels_len = (w as usize) * (h as usize) * 4;
    let stride = ((w as usize) + 7) / 8;
    let mask_len = stride * (h as usize);
    let mut pixels = vec![0u8; pixels_len];
    rd.read_exact(&mut pixels).await?;
    let mut mask = vec![0u8; mask_len];
    rd.read_exact(&mut mask).await?;
    // Translate the 1-bpp transparency mask into the alpha channel.
    for row in 0..h as usize {
        for col in 0..w as usize {
            let bit = (mask[row * stride + col / 8] >> (7 - (col % 8))) & 1;
            pixels[(row * w as usize + col) * 4 + 3] = if bit == 1 { 0xFF } else { 0 };
        }
    }
    sink.emit(FrameEvent::Cursor {
        width: w,
        height: h,
        hot_x,
        hot_y,
        data: pixels,
    });
    Ok(())
}

/// Force every pixel's alpha byte to 255. RFB true-colour with a 32-bit
/// format leaves the 4th byte undefined; the WebView's `putImageData` would
/// render a 0-alpha pixel as transparent, so we make tiles opaque here.
fn force_opaque(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        px[3] = 0xFF;
    }
}

// ── Client → server messages ─────────────────────────────────────────────

async fn send_set_pixel_format<W: AsyncWrite + Unpin>(wr: &mut W) -> Result<()> {
    // 32 bpp, depth 24, little-endian, true-colour, RGBA byte order
    // (red-shift 0, green 8, blue 16) so framebuffer bytes are [R,G,B,X].
    let mut msg = [0u8; 20];
    msg[0] = 0; // SetPixelFormat
    // msg[1..4] padding
    let pf = [
        32u8, 24, 0, 1, // bpp, depth, big-endian-flag, true-colour-flag
        0, 255, 0, 255, 0, 255, // red-max, green-max, blue-max (u16 BE)
        0, 8, 16, // red-shift, green-shift, blue-shift
        0, 0, 0, // padding
    ];
    msg[4..20].copy_from_slice(&pf);
    wr.write_all(&msg).await?;
    Ok(())
}

async fn send_set_encodings<W: AsyncWrite + Unpin>(wr: &mut W) -> Result<()> {
    let encodings: [i32; 5] = [ENC_ZLIB, ENC_COPY_RECT, ENC_RAW, ENC_CURSOR, ENC_DESKTOP_SIZE];
    let mut msg = Vec::with_capacity(4 + encodings.len() * 4);
    msg.push(2u8); // SetEncodings
    msg.push(0u8); // padding
    msg.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
    for enc in encodings {
        msg.extend_from_slice(&enc.to_be_bytes());
    }
    wr.write_all(&msg).await?;
    Ok(())
}

async fn send_fbur<W: AsyncWrite + Unpin>(
    wr: &mut W,
    incremental: bool,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
) -> Result<()> {
    let mut msg = [0u8; 10];
    msg[0] = 3; // FramebufferUpdateRequest
    msg[1] = u8::from(incremental);
    msg[2..4].copy_from_slice(&x.to_be_bytes());
    msg[4..6].copy_from_slice(&y.to_be_bytes());
    msg[6..8].copy_from_slice(&w.to_be_bytes());
    msg[8..10].copy_from_slice(&h.to_be_bytes());
    wr.write_all(&msg).await?;
    Ok(())
}

// ── Input forwarding ─────────────────────────────────────────────────────

#[derive(Default)]
struct PointerState {
    mask: u8,
    x: u16,
    y: u16,
}

async fn write_input<W: AsyncWrite + Unpin>(
    wr: &mut W,
    ev: &InputEvent,
    st: &mut PointerState,
) -> Result<()> {
    match ev {
        InputEvent::PointerMove { x, y } => {
            st.x = *x;
            st.y = *y;
            send_pointer(wr, st.mask, *x, *y).await?;
        }
        InputEvent::PointerButton { x, y, button, pressed } => {
            let bit = match button {
                MouseButton::Left => 0x01,
                MouseButton::Middle => 0x02,
                MouseButton::Right => 0x04,
            };
            if *pressed {
                st.mask |= bit;
            } else {
                st.mask &= !bit;
            }
            st.x = *x;
            st.y = *y;
            send_pointer(wr, st.mask, *x, *y).await?;
        }
        InputEvent::PointerScroll { x, y, dy, .. } => {
            st.x = *x;
            st.y = *y;
            // Wheel up = button 4 (bit 3), wheel down = button 5 (bit 4);
            // each notch is a press + release.
            let wheel = if *dy > 0 { 0x10 } else { 0x08 };
            send_pointer(wr, st.mask | wheel, *x, *y).await?;
            send_pointer(wr, st.mask, *x, *y).await?;
        }
        InputEvent::Key { keysym, pressed, .. } => {
            send_key(wr, *keysym, *pressed).await?;
        }
        InputEvent::KeyUnicode { ch, pressed } => {
            send_key(wr, unicode_to_keysym(*ch), *pressed).await?;
        }
        InputEvent::SetClipboard(text) => {
            send_client_cut_text(wr, text).await?;
        }
    }
    Ok(())
}

/// Send `ClientCutText` (msg type 6): the RFB cut-text protocol is Latin-1
/// only, so non-Latin-1 characters are replaced with `?`.
async fn send_client_cut_text<W: AsyncWrite + Unpin>(wr: &mut W, text: &str) -> Result<()> {
    let latin1: Vec<u8> = text
        .chars()
        .map(|c| {
            let cp = c as u32;
            if cp < 0x100 { cp as u8 } else { b'?' }
        })
        .collect();
    let mut msg = Vec::with_capacity(8 + latin1.len());
    msg.push(6); // ClientCutText
    msg.extend_from_slice(&[0, 0, 0]); // padding
    msg.extend_from_slice(&(latin1.len() as u32).to_be_bytes());
    msg.extend_from_slice(&latin1);
    wr.write_all(&msg).await?;
    Ok(())
}

async fn send_pointer<W: AsyncWrite + Unpin>(wr: &mut W, mask: u8, x: u16, y: u16) -> Result<()> {
    let mut msg = [0u8; 6];
    msg[0] = 5; // PointerEvent
    msg[1] = mask;
    msg[2..4].copy_from_slice(&x.to_be_bytes());
    msg[4..6].copy_from_slice(&y.to_be_bytes());
    wr.write_all(&msg).await?;
    Ok(())
}

async fn send_key<W: AsyncWrite + Unpin>(wr: &mut W, keysym: u32, pressed: bool) -> Result<()> {
    let mut msg = [0u8; 8];
    msg[0] = 4; // KeyEvent
    msg[1] = u8::from(pressed);
    // msg[2..4] padding
    msg[4..8].copy_from_slice(&keysym.to_be_bytes());
    wr.write_all(&msg).await?;
    Ok(())
}

/// Map a Unicode scalar to an X11 keysym (latin-1 maps directly; everything
/// else uses the 0x0100_0000 + codepoint convention).
fn unicode_to_keysym(ch: char) -> u32 {
    let cp = ch as u32;
    if cp < 0x100 {
        cp
    } else {
        0x0100_0000 + cp
    }
}

// ── Security handshakes ──────────────────────────────────────────────────

/// Pick the strongest security type we support from the server's offer.
fn choose_security(types: &[u8]) -> Result<u8> {
    for preferred in [SEC_ARD, SEC_VNC_AUTH, SEC_NONE] {
        if types.contains(&preferred) {
            return Ok(preferred);
        }
    }
    Err(RemoteDesktopError::Unsupported(format!(
        "server offered no supported VNC security type (got {types:?}); \
         Tight/VeNCrypt/RSA-AES are not implemented"
    )))
}

/// Standard VNC Authentication (type 2): DES-encrypt the 16-byte server
/// challenge with the bit-reversed password and return it.
async fn vnc_auth<R, W>(rd: &mut R, wr: &mut W, password: &str) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    use des::cipher::{BlockEncrypt, KeyInit};
    use des::cipher::generic_array::GenericArray;
    use des::Des;

    let mut challenge = [0u8; 16];
    rd.read_exact(&mut challenge).await?;

    // The DES key is the first 8 password bytes (NUL-padded) with each
    // byte's bits reversed — a long-standing VNC quirk.
    let mut key = [0u8; 8];
    for (slot, byte) in key.iter_mut().zip(password.bytes()) {
        *slot = byte.reverse_bits();
    }
    let cipher = Des::new(GenericArray::from_slice(&key));
    for block in challenge.chunks_mut(8) {
        cipher.encrypt_block(GenericArray::from_mut_slice(block));
    }
    wr.write_all(&challenge).await?;
    Ok(())
}

/// Apple ARD authentication (type 30): Diffie-Hellman key agreement followed
/// by AES-128-ECB of a 128-byte username/password block, keyed by
/// MD5(shared secret). Required by modern macOS Screen Sharing.
async fn ard_auth<R, W>(rd: &mut R, wr: &mut W, username: &str, password: &str) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    use aes::cipher::{BlockEncrypt, KeyInit};
    use aes::cipher::generic_array::GenericArray;
    use aes::Aes128;
    use md5::{Digest, Md5};

    // Server → client: generator (u16), key length (u16), prime, server pub.
    let generator = rd.read_u16().await?;
    let key_len = rd.read_u16().await? as usize;
    if key_len == 0 || key_len > 1024 {
        return Err(RemoteDesktopError::Protocol(format!(
            "implausible ARD DH key length {key_len}"
        )));
    }
    let mut prime = vec![0u8; key_len];
    rd.read_exact(&mut prime).await?;
    let mut server_pub = vec![0u8; key_len];
    rd.read_exact(&mut server_pub).await?;

    // Our DH key pair.
    let p = BigUint::from_bytes_be(&prime);
    let g = BigUint::from(generator);
    let server_pub_n = BigUint::from_bytes_be(&server_pub);
    let mut priv_bytes = vec![0u8; key_len];
    getrandom::getrandom(&mut priv_bytes)
        .map_err(|e| RemoteDesktopError::Auth(format!("RNG failure: {e}")))?;
    let private = BigUint::from_bytes_be(&priv_bytes);
    let client_pub = g.modpow(&private, &p);
    let shared = server_pub_n.modpow(&private, &p);

    let shared_bytes = left_pad(shared.to_bytes_be(), key_len);
    let client_pub_bytes = left_pad(client_pub.to_bytes_be(), key_len);
    let aes_key: [u8; 16] = Md5::digest(&shared_bytes).into();

    // 128-byte credential block: username (NUL-terminated) in [0,64),
    // password (NUL-terminated) in [64,128), remainder random.
    let mut credentials = [0u8; 128];
    getrandom::getrandom(&mut credentials)
        .map_err(|e| RemoteDesktopError::Auth(format!("RNG failure: {e}")))?;
    write_credential_field(&mut credentials[0..64], username)?;
    write_credential_field(&mut credentials[64..128], password)?;

    let cipher = Aes128::new(GenericArray::from_slice(&aes_key));
    for block in credentials.chunks_mut(16) {
        cipher.encrypt_block(GenericArray::from_mut_slice(block));
    }

    // Client → server: encrypted credentials (128) then our DH public value.
    wr.write_all(&credentials).await?;
    wr.write_all(&client_pub_bytes).await?;
    Ok(())
}

/// Write a NUL-terminated UTF-8 field into a fixed 64-byte slot, leaving the
/// (already-randomised) tail untouched after the terminator.
fn write_credential_field(slot: &mut [u8], value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    if bytes.len() >= slot.len() {
        return Err(RemoteDesktopError::Auth(
            "username or password too long for ARD (max 63 bytes)".to_string(),
        ));
    }
    slot[..bytes.len()].copy_from_slice(bytes);
    slot[bytes.len()] = 0;
    Ok(())
}

/// Left-pad a big-endian byte vector with zeros to `len` (or truncate the
/// high zero bytes if it is somehow longer).
fn left_pad(bytes: Vec<u8>, len: usize) -> Vec<u8> {
    if bytes.len() == len {
        bytes
    } else if bytes.len() < len {
        let mut out = vec![0u8; len - bytes.len()];
        out.extend_from_slice(&bytes);
        out
    } else {
        bytes[bytes.len() - len..].to_vec()
    }
}

// ── Misc helpers ─────────────────────────────────────────────────────────

/// Parse the minor version from a `RFB 003.0XX` banner. Apple sends
/// `003.889`; standard servers send `003.003/007/008`.
fn parse_minor(banner: &[u8; 12]) -> u32 {
    std::str::from_utf8(&banner[8..11])
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(3)
}

async fn read_string_u32<R: AsyncRead + Unpin>(rd: &mut R) -> Result<String> {
    let len = rd.read_u32().await?;
    let mut buf = vec![0u8; len as usize];
    rd.read_exact(&mut buf).await?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}
