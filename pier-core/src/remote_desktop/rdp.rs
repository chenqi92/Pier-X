//! RDP client backed by IronRDP (Devolutions).
//!
//! IronRDP is a headless protocol stack: we drive the connection + session
//! state machines, it software-decodes graphics into a flat RGBA
//! framebuffer ([`DecodedImage`]), and we forward dirty rectangles to the
//! host. Network Level Authentication (NLA) runs over CredSSP/NTLM in-band,
//! so the `sspi` network client is only needed for Kerberos KDC traffic —
//! we pass a stub that errors if invoked, keeping the dependency tree small.
//! Username/password (NTLM) auth therefore works; domain-Kerberos is a
//! follow-up.

use ironrdp::connector::{self, ClientConnector, Credentials, ServerName};
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::input::{Database, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp::input::MouseButton as RdpMouseButton;
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::geometry::InclusiveRectangle;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{FramedWrite, TokioFramed, split_tokio_framed};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedReceiver;

use super::error::{RemoteDesktopError, Result};
use super::frame::{self, FrameEvent, FrameSink};
use super::input::{InputEvent, MouseButton};
use super::{ControlMsg, RemoteDesktopConfig};

/// Drive one RDP connection to completion.
pub(crate) async fn run(
    config: RemoteDesktopConfig,
    sink: FrameSink,
    mut input_rx: UnboundedReceiver<InputEvent>,
    mut control_rx: UnboundedReceiver<ControlMsg>,
) -> Result<()> {
    let jpeg_threshold = config.jpeg_threshold_px;
    let server_name = config.host.clone();

    // ── TCP + connector ──────────────────────────────────────────────
    let stream = TcpStream::connect((config.host.as_str(), config.port))
        .await
        .map_err(|e| RemoteDesktopError::Connect(format!("{}:{}: {e}", config.host, config.port)))?;
    stream.set_nodelay(true).ok();
    let client_addr = stream
        .local_addr()
        .map_err(|e| RemoteDesktopError::Connect(format!("local addr: {e}")))?;
    let mut framed = TokioFramed::new(stream);

    let mut connector = ClientConnector::new(build_config(&config), client_addr);

    // ── X.224 negotiation up to the TLS boundary ─────────────────────
    let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector)
        .await
        .map_err(|e| RemoteDesktopError::Connect(format!("RDP negotiation: {e}")))?;

    // ── TLS upgrade ──────────────────────────────────────────────────
    let initial_stream = framed.into_inner_no_leftover();
    let (upgraded_stream, tls_cert) = ironrdp_tls::upgrade(initial_stream, &server_name)
        .await
        .map_err(|e| RemoteDesktopError::Connect(format!("TLS upgrade: {e}")))?;
    let server_public_key = ironrdp_tls::extract_tls_server_public_key(&tls_cert)
        .ok_or_else(|| RemoteDesktopError::Connect("unable to extract TLS server public key".to_string()))?;
    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
    let mut upgraded_framed = TokioFramed::new(upgraded_stream);

    // ── Finalize: CredSSP/NLA, MCS, capabilities, licensing ──────────
    let mut network_client = ReqwestNetworkClient::new();
    let connection_result = ironrdp_tokio::connect_finalize(
        upgraded,
        connector,
        &mut upgraded_framed,
        &mut network_client,
        ServerName::new(&server_name),
        server_public_key.to_owned(),
        None,
    )
    .await
    .map_err(|e| RemoteDesktopError::Auth(format!("RDP activation: {e}")))?;

    let desktop = connection_result.desktop_size;
    let mut image = DecodedImage::new(PixelFormat::RgbA32, desktop.width, desktop.height);
    let mut active_stage = ActiveStage::new(connection_result);
    sink.emit(FrameEvent::Connected {
        width: desktop.width,
        height: desktop.height,
    });

    let (mut reader, mut writer) = split_tokio_framed(upgraded_framed);
    let mut input_db = Database::new();

    // ── Active session loop ──────────────────────────────────────────
    'session: loop {
        tokio::select! {
            frame = reader.read_pdu() => {
                let (action, payload) = frame
                    .map_err(|e| RemoteDesktopError::Protocol(format!("read pdu: {e}")))?;
                let outputs = active_stage
                    .process(&mut image, action, &payload)
                    .map_err(|e| RemoteDesktopError::Protocol(format!("process: {e}")))?;
                for out in outputs {
                    match out {
                        ActiveStageOutput::ResponseFrame(frame) => writer
                            .write_all(&frame)
                            .await
                            .map_err(|e| RemoteDesktopError::Protocol(format!("write: {e}")))?,
                        ActiveStageOutput::GraphicsUpdate(region) => {
                            emit_region(&sink, &image, &region, jpeg_threshold);
                        }
                        ActiveStageOutput::Terminate(_reason) => break 'session,
                        // Display reactivation (server-side resize) requires
                        // replaying the activation sequence — not wired in
                        // v1; ask the user to reconnect.
                        ActiveStageOutput::DeactivateAll(_) => {
                            return Err(RemoteDesktopError::Unsupported(
                                "the server changed the display mode (resize); please reconnect"
                                    .to_string(),
                            ));
                        }
                        _ => {}
                    }
                }
            }
            maybe = input_rx.recv() => match maybe {
                Some(ev) => {
                    let ops = to_operations(&ev);
                    if ops.is_empty() {
                        continue;
                    }
                    let events = input_db.apply(ops);
                    if events.is_empty() {
                        continue;
                    }
                    let outputs = active_stage
                        .process_fastpath_input(&mut image, &events)
                        .map_err(|e| RemoteDesktopError::Protocol(format!("input: {e}")))?;
                    for out in outputs {
                        if let ActiveStageOutput::ResponseFrame(frame) = out {
                            writer
                                .write_all(&frame)
                                .await
                                .map_err(|e| RemoteDesktopError::Protocol(format!("write: {e}")))?;
                        }
                    }
                }
                None => break 'session,
            },
            maybe = control_rx.recv() => match maybe {
                Some(ControlMsg::Close) | None => break 'session,
                // DisplayControl dynamic resize is a follow-up.
                Some(ControlMsg::Resize { .. }) => {}
            },
        }
    }

    Ok(())
}

/// Slice the updated rectangle out of the full RGBA framebuffer and emit it
/// as a tile. `region` is inclusive on all edges.
fn emit_region(sink: &FrameSink, image: &DecodedImage, region: &InclusiveRectangle, jpeg: u32) {
    let img_w = image.width() as usize;
    let left = region.left as usize;
    let top = region.top as usize;
    let right = region.right as usize;
    let bottom = region.bottom as usize;
    if right < left || bottom < top {
        return;
    }
    let w = right - left + 1;
    let h = bottom - top + 1;
    let data = image.data();
    let row_bytes = w * 4;
    let mut tile = vec![0u8; w * h * 4];
    for row in 0..h {
        let src = ((top + row) * img_w + left) * 4;
        let dst = row * row_bytes;
        if src + row_bytes <= data.len() {
            tile[dst..dst + row_bytes].copy_from_slice(&data[src..src + row_bytes]);
        }
    }
    // IronRDP's RgbA32 leaves alpha undefined; force opaque for the canvas.
    for px in tile.chunks_exact_mut(4) {
        px[3] = 0xFF;
    }
    sink.emit(FrameEvent::Tile(frame::encode_tile(
        left as u16,
        top as u16,
        w as u16,
        h as u16,
        tile,
        jpeg,
    )));
}

/// Translate one viewer input event into IronRDP input operations.
fn to_operations(ev: &InputEvent) -> Vec<Operation> {
    match ev {
        InputEvent::PointerMove { x, y } => {
            vec![Operation::MouseMove(MousePosition { x: *x, y: *y })]
        }
        InputEvent::PointerButton { x, y, button, pressed } => {
            let btn = match button {
                MouseButton::Left => RdpMouseButton::Left,
                MouseButton::Middle => RdpMouseButton::Middle,
                MouseButton::Right => RdpMouseButton::Right,
            };
            vec![
                Operation::MouseMove(MousePosition { x: *x, y: *y }),
                if *pressed {
                    Operation::MouseButtonPressed(btn)
                } else {
                    Operation::MouseButtonReleased(btn)
                },
            ]
        }
        InputEvent::PointerScroll { dy, .. } => {
            // RDP wheel: positive rotation = up, negative = down (units of
            // ~120 per notch).
            let rotation_units: i16 = if *dy > 0 { -120 } else { 120 };
            vec![Operation::WheelRotations(WheelRotations {
                is_vertical: true,
                rotation_units,
            })]
        }
        InputEvent::Key { scancode, extended, pressed, .. } => {
            let sc = Scancode::from_u8(*extended, *scancode as u8);
            vec![if *pressed {
                Operation::KeyPressed(sc)
            } else {
                Operation::KeyReleased(sc)
            }]
        }
        InputEvent::KeyUnicode { ch, pressed } => {
            vec![if *pressed {
                Operation::UnicodeKeyPressed(*ch)
            } else {
                Operation::UnicodeKeyReleased(*ch)
            }]
        }
    }
}

/// Build the IronRDP connector config from our protocol-agnostic config.
fn build_config(config: &RemoteDesktopConfig) -> connector::Config {
    connector::Config {
        desktop_size: connector::DesktopSize {
            width: config.width,
            height: config.height,
        },
        desktop_scale_factor: 0,
        enable_tls: false,
        enable_credssp: true,
        credentials: Credentials::UsernamePassword {
            username: config.username.clone(),
            password: config.password.clone(),
        },
        domain: config.domain.clone(),
        client_build: 0,
        client_name: "Pier-X".to_owned(),
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        keyboard_layout: 0,
        ime_file_name: String::new(),
        bitmap: None,
        dig_product_id: String::new(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        platform: MajorPlatformType::WINDOWS,
        hardware_id: None,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        performance_flags: PerformanceFlags::default(),
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        compression_type: None,
        // Let the server render the cursor into the framebuffer so we don't
        // need a separate cursor channel in v1.
        enable_server_pointer: false,
        pointer_software_rendering: true,
        multitransport_flags: None,
    }
}
