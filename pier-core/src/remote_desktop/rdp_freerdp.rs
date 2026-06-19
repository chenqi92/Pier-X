//! FreeRDP-backed RDP client (feature `rdp-freerdp`).
//!
//! Why this exists: IronRDP 0.x has **no H.264** and its session stage only
//! emits decoded dirty-rects, so neither in-Rust H.264 nor WebCodecs forwarding
//! is possible on that stack. FreeRDP 3 implements the MS-RDPEGFX H.264 graphics
//! pipeline (AVC420 + AVC444) with OS-native decoders — Media Foundation
//! (Windows), VideoToolbox via FFmpeg (macOS), OpenH264/FFmpeg (Linux) — the
//! only no-agent path to video-grade RDP smoothness.
//!
//! ## Data path
//!
//! We drive a minimal embedding of `libfreerdp-client3`: a software GDI
//! (`gdi_init` with an RGBX framebuffer) plus the GFX→GDI graphics pipeline
//! (`gdi_graphics_pipeline_init`, wired on the RDPGFX channel-connected event).
//! FreeRDP decodes H.264 internally (HW where available) straight into
//! `gdi->primary_buffer`; our `EndPaint` callback slices the just-invalidated
//! rectangles out of that buffer and pushes them through the shared
//! [`FrameSink`] as [`FrameEvent::Tile`]s — the exact tile stream the IronRDP
//! and VNC backends already produce, so **nothing in `src-tauri` or the
//! frontend changes**.
//!
//! ## Threading
//!
//! libfreerdp is a blocking C event loop, so it runs on a dedicated
//! `spawn_blocking` thread. Viewer input / control arrive on tokio channels; a
//! small bridge task copies them onto std channels the blocking loop drains
//! each iteration, so every FreeRDP FFI call happens on the one owning thread.

use std::ffi::{CStr, CString};
use std::os::raw::c_void;
use std::ptr;
use std::sync::mpsc::{Receiver, TryRecvError};

use freerdp_sys as sys;
use tokio::sync::mpsc::UnboundedReceiver;

use super::error::{RemoteDesktopError, Result};
use super::frame::{self, FrameEvent, FrameSink};
use super::input::{InputEvent, MouseButton};
use super::{ControlMsg, RemoteDesktopConfig};

/// Our context: the FreeRDP `rdpContext` MUST come first so the allocator's
/// view and our `*mut PierxContext` casts of the `rdpContext*` agree.
#[repr(C)]
struct PierxContext {
    context: sys::rdpContext,
    state: *mut SharedState,
}

/// Rust-side session state reachable from the C callbacks via the context.
struct SharedState {
    sink: FrameSink,
    jpeg_threshold: u32,
    width: u16,
    height: u16,
}

fn b(v: bool) -> sys::BOOL {
    if v {
        1
    } else {
        0
    }
}

/// Drive one FreeRDP connection to completion.
pub(crate) async fn run(
    config: RemoteDesktopConfig,
    sink: FrameSink,
    mut input_rx: UnboundedReceiver<InputEvent>,
    mut control_rx: UnboundedReceiver<ControlMsg>,
) -> Result<()> {
    // Bridge the tokio receivers onto std channels the blocking loop can poll.
    let (in_tx, in_rx) = std::sync::mpsc::channel::<InputEvent>();
    let (ctl_tx, ctl_rx) = std::sync::mpsc::channel::<ControlMsg>();
    let bridge = tokio::spawn(async move {
        loop {
            tokio::select! {
                ev = input_rx.recv() => match ev {
                    Some(e) => { if in_tx.send(e).is_err() { break; } }
                    None => break,
                },
                c = control_rx.recv() => match c {
                    Some(c) => { if ctl_tx.send(c).is_err() { break; } }
                    None => break,
                },
            }
        }
    });

    let result = tokio::task::spawn_blocking(move || run_blocking(config, sink, in_rx, ctl_rx))
        .await
        .unwrap_or_else(|e| {
            Err(RemoteDesktopError::Protocol(format!(
                "freerdp worker thread panicked: {e}"
            )))
        });
    bridge.abort();
    result
}

/// Frees `freerdp_context_new` + `freerdp_new` allocations on any exit path.
struct InstanceGuard(*mut sys::freerdp);
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            sys::freerdp_context_free(self.0);
            sys::freerdp_free(self.0);
        }
    }
}

/// Frees the boxed [`SharedState`] stashed in the context.
struct StateGuard(*mut SharedState);
impl Drop for StateGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { drop(Box::from_raw(self.0)) };
        }
    }
}

fn run_blocking(
    config: RemoteDesktopConfig,
    sink: FrameSink,
    in_rx: Receiver<InputEvent>,
    ctl_rx: Receiver<ControlMsg>,
) -> Result<()> {
    unsafe {
        let instance = sys::freerdp_new();
        if instance.is_null() {
            return Err(RemoteDesktopError::Connect(
                "freerdp_new() returned null".into(),
            ));
        }
        (*instance).ContextSize = std::mem::size_of::<PierxContext>() as _;
        (*instance).PreConnect = Some(pre_connect);
        (*instance).PostConnect = Some(post_connect);
        (*instance).PostDisconnect = Some(post_disconnect);

        if sys::freerdp_context_new(instance) == 0 {
            sys::freerdp_free(instance);
            return Err(RemoteDesktopError::Connect(
                "freerdp_context_new() failed".into(),
            ));
        }
        // From here the context exists: this guard tears it down on every path.
        let _instance_guard = InstanceGuard(instance);

        let ctx = (*instance).context;
        let pierx = ctx as *mut PierxContext;
        let state = Box::into_raw(Box::new(SharedState {
            sink,
            jpeg_threshold: config.jpeg_threshold_px,
            width: config.width,
            height: config.height,
        }));
        (*pierx).state = state;
        let _state_guard = StateGuard(state);

        configure_settings(ctx, &config)?;

        // Wire the H.264/AVC444 GFX pipeline into the GDI surface when the
        // RDPGFX dynamic channel connects (mirrors libfreerdp-client's own
        // OnChannelConnected handler).
        sys::PubSub_Subscribe(
            (*ctx).pubSub,
            c"ChannelConnected".as_ptr(),
            on_channel_connected
                as unsafe extern "C" fn(*mut c_void, *const sys::ChannelConnectedEventArgs),
        );

        if sys::freerdp_connect(instance) == 0 {
            return Err(connect_error(ctx));
        }

        // ── Active session loop ──────────────────────────────────────────
        let mut handles: [sys::HANDLE; 64] = [ptr::null_mut(); 64];
        let loop_result = 'session: loop {
            match ctl_rx.try_recv() {
                Ok(ControlMsg::Close) | Err(TryRecvError::Disconnected) => break 'session Ok(()),
                Ok(ControlMsg::Resize { .. }) | Err(TryRecvError::Empty) => {}
            }
            if sys::freerdp_shall_disconnect_context(ctx) != 0 {
                break 'session Ok(());
            }
            loop {
                match in_rx.try_recv() {
                    Ok(ev) => send_input(ctx, &ev),
                    Err(_) => break,
                }
            }

            let n = sys::freerdp_get_event_handles(ctx, handles.as_mut_ptr(), handles.len() as u32);
            if n == 0 {
                break 'session Err(RemoteDesktopError::Protocol(
                    "freerdp_get_event_handles() returned no handles".into(),
                ));
            }
            // Wait up to ~20 ms so input/control stay responsive between frames.
            sys::WaitForMultipleObjects(n, handles.as_ptr(), b(false), 20);
            if sys::freerdp_check_event_handles(ctx) == 0 {
                let code = sys::freerdp_get_last_error(ctx);
                break 'session if code != 0 {
                    Err(RemoteDesktopError::Protocol(error_string(code)))
                } else {
                    Ok(())
                };
            }
        };

        sys::freerdp_disconnect(instance);
        loop_result
        // _state_guard then _instance_guard run here, in that order.
    }
}

/// Push all connection settings (target, credentials, GFX H.264, perf trims).
unsafe fn configure_settings(
    ctx: *mut sys::rdpContext,
    config: &RemoteDesktopConfig,
) -> Result<()> {
    let s = (*ctx).settings;

    let set_str = |id, val: &str| -> Result<()> {
        let c = CString::new(val).map_err(|_| {
            RemoteDesktopError::Connect("connection field contained a NUL byte".into())
        })?;
        if sys::freerdp_settings_set_string(s, id, c.as_ptr()) == 0 {
            return Err(RemoteDesktopError::Connect(
                "failed to set an RDP setting".into(),
            ));
        }
        Ok(())
    };
    let set_u32 = |id, val: u32| {
        sys::freerdp_settings_set_uint32(s, id, val);
    };
    let set_bool = |id, val: bool| {
        sys::freerdp_settings_set_bool(s, id, b(val));
    };

    set_str(sys::FreeRDP_ServerHostname, &config.host)?;
    set_u32(sys::FreeRDP_ServerPort, u32::from(config.port));
    set_str(sys::FreeRDP_Username, &config.username)?;
    set_str(sys::FreeRDP_Password, &config.password)?;
    if let Some(domain) = config.domain.as_deref() {
        set_str(sys::FreeRDP_Domain, domain)?;
    }
    set_u32(sys::FreeRDP_DesktopWidth, u32::from(config.width));
    set_u32(sys::FreeRDP_DesktopHeight, u32::from(config.height));
    set_u32(sys::FreeRDP_ColorDepth, 32);

    // Software GDI: FreeRDP decodes (incl. H.264) into gdi->primary_buffer,
    // which our EndPaint reads. The GFX pipeline carries AVC420/AVC444.
    set_bool(sys::FreeRDP_SoftwareGdi, true);
    set_bool(sys::FreeRDP_SupportGraphicsPipeline, true);
    set_bool(sys::FreeRDP_GfxH264, true);
    set_bool(sys::FreeRDP_GfxAVC444, true);
    set_bool(sys::FreeRDP_GfxAVC444v2, true);
    set_bool(sys::FreeRDP_RemoteFxCodec, true);

    // Security: offer TLS + NLA, never legacy Standard RDP Security (RC4) —
    // same stance as the IronRDP path. Accept self-signed certs (the universal
    // Windows default); host-key TOFU is a follow-up.
    set_bool(sys::FreeRDP_NlaSecurity, true);
    set_bool(sys::FreeRDP_TlsSecurity, true);
    set_bool(sys::FreeRDP_RdpSecurity, false);
    set_bool(sys::FreeRDP_IgnoreCertificate, true);
    set_bool(sys::FreeRDP_AutoLogonEnabled, true);

    // Trim redundant redraws (wallpaper, drag outlines, menu fades) — same
    // performance flags the IronRDP path sets.
    set_bool(sys::FreeRDP_DisableWallpaper, true);
    set_bool(sys::FreeRDP_DisableFullWindowDrag, true);
    set_bool(sys::FreeRDP_DisableMenuAnims, true);

    Ok(())
}

// ── C callbacks ────────────────────────────────────────────────────────────

unsafe extern "C" fn pre_connect(instance: *mut sys::freerdp) -> sys::BOOL {
    let ctx = (*instance).context;
    if ctx.is_null() {
        return b(false);
    }
    // Load the dynamic-channel addins the settings ask for (drdynvc + rdpgfx).
    sys::freerdp_client_load_addins((*ctx).channels, (*ctx).settings)
}

unsafe extern "C" fn post_connect(instance: *mut sys::freerdp) -> sys::BOOL {
    let ctx = (*instance).context;
    if ctx.is_null() {
        return b(false);
    }
    // RGBX framebuffer: memory byte order R,G,B,X — drop-in for the WebView's
    // RGBA canvas tile (we force the 4th byte to 0xFF on copy).
    if sys::gdi_init(instance, sys::PIERX_PIXEL_FORMAT_RGBX32 as u32) == 0 {
        return b(false);
    }

    let update = (*ctx).update;
    if !update.is_null() {
        (*update).EndPaint = Some(end_paint);
        (*update).DesktopResize = Some(desktop_resize);
    }

    let st = state_of(ctx);
    let gdi = (*ctx).gdi;
    if !st.is_null() && !gdi.is_null() {
        let w = (*gdi).width.max(1) as u16;
        let h = (*gdi).height.max(1) as u16;
        (*st).width = w;
        (*st).height = h;
        (*st).sink.emit(FrameEvent::Connected {
            width: w,
            height: h,
        });
    }
    b(true)
}

unsafe extern "C" fn post_disconnect(instance: *mut sys::freerdp) {
    if !(*instance).context.is_null() {
        sys::gdi_free(instance);
    }
}

/// On RDPGFX connect, bind the GFX pipeline to the GDI surface so decoded
/// H.264/AVC444 frames land in `gdi->primary_buffer`.
unsafe extern "C" fn on_channel_connected(
    context: *mut c_void,
    e: *const sys::ChannelConnectedEventArgs,
) {
    if context.is_null() || e.is_null() || (*e).name.is_null() {
        return;
    }
    let name = CStr::from_ptr((*e).name);
    if name.to_bytes_with_nul() == sys::RDPGFX_DVC_CHANNEL_NAME {
        let ctx = context as *mut sys::rdpContext;
        let gfx = (*e).pInterface as *mut sys::RdpgfxClientContext;
        sys::gdi_graphics_pipeline_init((*ctx).gdi, gfx);
    }
}

unsafe extern "C" fn desktop_resize(context: *mut sys::rdpContext) -> sys::BOOL {
    let st = state_of(context);
    if st.is_null() || (*context).gdi.is_null() {
        return b(false);
    }
    let s = (*context).settings;
    let w = sys::freerdp_settings_get_uint32(s, sys::FreeRDP_DesktopWidth);
    let h = sys::freerdp_settings_get_uint32(s, sys::FreeRDP_DesktopHeight);
    if sys::gdi_resize((*context).gdi, w, h) == 0 {
        return b(false);
    }
    (*st).width = w as u16;
    (*st).height = h as u16;
    (*st).sink.emit(FrameEvent::Resize {
        width: w as u16,
        height: h as u16,
    });
    b(true)
}

/// Slice every just-invalidated rectangle out of the framebuffer and emit it.
unsafe extern "C" fn end_paint(context: *mut sys::rdpContext) -> sys::BOOL {
    let st = state_of(context);
    if st.is_null() {
        return b(true);
    }
    let gdi = (*context).gdi;
    if gdi.is_null() || (*gdi).primary.is_null() {
        return b(true);
    }
    let hdc = (*(*gdi).primary).hdc;
    if hdc.is_null() {
        return b(true);
    }
    let hwnd = (*hdc).hwnd;
    if hwnd.is_null() {
        return b(true);
    }
    let ninvalid = (*hwnd).ninvalid;
    if ninvalid < 1 || (*hwnd).cinvalid.is_null() {
        return b(true);
    }

    let cinvalid = (*hwnd).cinvalid;
    for i in 0..ninvalid as isize {
        let rgn = cinvalid.offset(i);
        emit_tile(&*st, gdi, (*rgn).x, (*rgn).y, (*rgn).w, (*rgn).h);
    }

    // Reset the invalid region for the next frame (mirrors the mainline clients).
    if !(*hwnd).invalid.is_null() {
        (*(*hwnd).invalid).null = b(true);
    }
    (*hwnd).ninvalid = 0;
    b(true)
}

/// Copy one dirty rect from `gdi->primary_buffer` (RGBX) into a tight RGBA
/// tile (alpha forced opaque) and push it through the sink.
unsafe fn emit_tile(st: &SharedState, gdi: *mut sys::rdpGdi, x: i32, y: i32, w: i32, h: i32) {
    let buf = (*gdi).primary_buffer;
    if buf.is_null() {
        return;
    }
    let fb_w = (*gdi).width;
    let fb_h = (*gdi).height;
    let stride = (*gdi).stride as usize;

    let x = x.clamp(0, fb_w);
    let y = y.clamp(0, fb_h);
    let w = w.min(fb_w - x);
    let h = h.min(fb_h - y);
    if w <= 0 || h <= 0 {
        return;
    }
    let (xs, ys, ws, hs) = (x as usize, y as usize, w as usize, h as usize);
    let row_bytes = ws * 4;
    let mut tile = vec![0u8; ws * hs * 4];
    for row in 0..hs {
        let src_off = (ys + row) * stride + xs * 4;
        let src = std::slice::from_raw_parts(buf.add(src_off), row_bytes);
        let dst = &mut tile[row * row_bytes..row * row_bytes + row_bytes];
        dst.copy_from_slice(src);
    }
    // RGBX → force opaque alpha for the canvas.
    for px in tile.chunks_exact_mut(4) {
        px[3] = 0xFF;
    }

    st.sink.emit(FrameEvent::Tile(frame::encode_tile(
        x as u16,
        y as u16,
        w as u16,
        h as u16,
        tile,
        st.jpeg_threshold,
    )));
}

// ── Input ──────────────────────────────────────────────────────────────────

unsafe fn send_input(ctx: *mut sys::rdpContext, ev: &InputEvent) {
    let input = (*ctx).input;
    if input.is_null() {
        return;
    }
    match ev {
        InputEvent::PointerMove { x, y } => {
            sys::freerdp_input_send_mouse_event(input, sys::PTR_FLAGS_MOVE as u16, *x, *y);
        }
        InputEvent::PointerButton {
            x,
            y,
            button,
            pressed,
        } => {
            let mut flags = match button {
                MouseButton::Left => sys::PTR_FLAGS_BUTTON1,
                MouseButton::Right => sys::PTR_FLAGS_BUTTON2,
                MouseButton::Middle => sys::PTR_FLAGS_BUTTON3,
            };
            if *pressed {
                flags |= sys::PTR_FLAGS_DOWN;
            }
            sys::freerdp_input_send_mouse_event(input, flags as u16, *x, *y);
        }
        InputEvent::PointerScroll { x, y, dy, .. } => {
            // One notch (120) per event; PTR_FLAGS_WHEEL_NEGATIVE = scroll down.
            let mut flags = sys::PTR_FLAGS_WHEEL | (120 & sys::WheelRotationMask);
            if *dy > 0 {
                flags |= sys::PTR_FLAGS_WHEEL_NEGATIVE;
            }
            sys::freerdp_input_send_mouse_event(input, flags as u16, *x, *y);
        }
        InputEvent::Key {
            scancode,
            extended,
            pressed,
            ..
        } => {
            let rdp_scancode = (u32::from(*scancode) & 0xFF)
                | if *extended {
                    sys::PIERX_KBDEXT as u32
                } else {
                    0
                };
            sys::freerdp_input_send_keyboard_event_ex(input, b(*pressed), b(false), rdp_scancode);
        }
        InputEvent::KeyUnicode { ch, pressed } => {
            let flags = if *pressed {
                0
            } else {
                sys::KBD_FLAGS_RELEASE as u16
            };
            sys::freerdp_input_send_unicode_keyboard_event(input, flags, *ch as u16);
        }
        // RDP clipboard runs over CLIPRDR — a follow-up.
        InputEvent::SetClipboard(_) => {}
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

unsafe fn state_of(context: *mut sys::rdpContext) -> *mut SharedState {
    if context.is_null() {
        return ptr::null_mut();
    }
    (*(context as *mut PierxContext)).state
}

/// Map a failed `freerdp_connect` to an error, splitting credential failures
/// (so the frontend diagnoses them as a sign-in problem) from the rest.
unsafe fn connect_error(ctx: *mut sys::rdpContext) -> RemoteDesktopError {
    let code = sys::freerdp_get_last_error(ctx);
    let msg = error_string(code);
    let lower = msg.to_ascii_lowercase();
    if lower.contains("auth")
        || lower.contains("logon")
        || lower.contains("credential")
        || lower.contains("password")
        || lower.contains("access denied")
    {
        RemoteDesktopError::Auth(msg)
    } else {
        RemoteDesktopError::Connect(msg)
    }
}

unsafe fn error_string(code: u32) -> String {
    let s = sys::freerdp_get_last_error_string(code);
    if s.is_null() {
        format!("RDP error 0x{code:08x}")
    } else {
        CStr::from_ptr(s).to_string_lossy().into_owned()
    }
}
