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

use std::io;
use std::sync::Arc;
use std::time::Duration;

use ironrdp::connector::{self, ClientConnector, Credentials, ServerName};
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::input::MouseButton as RdpMouseButton;
use ironrdp::input::{Database, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::geometry::InclusiveRectangle;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{split_tokio_framed, FramedWrite, TokioFramed};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::ServerName as TlsServerName;

use super::error::{RemoteDesktopError, Result};
use super::frame::{self, FrameEvent, FrameSink};
use super::input::{InputEvent, MouseButton};
use super::{ControlMsg, RemoteDesktopConfig};
use crate::ssh::runtime;

/// Upper bound on the whole RDP connect handshake (TCP → X.224 → TLS →
/// CredSSP/NLA → capability exchange). A reachable-but-silent host (firewall
/// drops the SYN instead of refusing) or a wedged NLA negotiation would
/// otherwise leave the task parked on an `.await` with no terminal event,
/// stranding the UI on "connecting" forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// Drive one RDP connection to completion.
pub(crate) async fn run(
    config: RemoteDesktopConfig,
    sink: FrameSink,
    mut input_rx: UnboundedReceiver<InputEvent>,
    mut control_rx: UnboundedReceiver<ControlMsg>,
    cert_prompt: Option<super::CertPromptCb>,
) -> Result<()> {
    let jpeg_threshold = config.jpeg_threshold_px;
    let server_name = config.host.clone();

    // The full connect handshake runs under one deadline so a stalled stage
    // becomes an error the host can show, not an indefinite hang.
    let handshake = async {
        // ── TCP + connector ──────────────────────────────────────────────
        let stream = TcpStream::connect((config.host.as_str(), config.port))
            .await
            .map_err(|e| {
                RemoteDesktopError::Connect(format!("{}:{}: {e}", config.host, config.port))
            })?;
        stream.set_nodelay(true).ok();
        let client_addr = stream
            .local_addr()
            .map_err(|e| RemoteDesktopError::Connect(format!("local addr: {e}")))?;
        let mut framed = TokioFramed::new(stream);

        let mut connector = ClientConnector::new(build_config(&config), client_addr);

        // ── X.224 negotiation up to the TLS boundary ─────────────────────
        let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector)
            .await
            .map_err(|e| {
                RemoteDesktopError::Connect(format!("RDP negotiation: {}", describe_err(&e)))
            })?;

        // ── TLS upgrade ──────────────────────────────────────────────────
        let initial_stream = framed.into_inner_no_leftover();
        let (upgraded_stream, server_public_key) = upgrade_tls(initial_stream, &server_name)
            .await
            .map_err(|e| RemoteDesktopError::Connect(format!("TLS upgrade: {e}")))?;

        // ── TOFU certificate pinning ─────────────────────────────────────
        // The TLS upgrade above does NOT verify the server certificate, so
        // pin the server public key (like an SSH host key) and check it
        // BEFORE CredSSP/NLA runs — credentials are never sent to a server
        // that fails verification. Without a prompt callback: accept-new on
        // first contact, hard-fail on a changed key (possible MITM).
        verify_server_key(&config, &server_public_key, &cert_prompt).await?;

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
            server_public_key,
            None,
        )
        .await
        .map_err(|e| RemoteDesktopError::Auth(format!("RDP activation: {}", describe_err(&e))))?;

        Ok::<_, RemoteDesktopError>((connection_result, upgraded_framed))
    };

    let (connection_result, upgraded_framed) = tokio::time::timeout(CONNECT_TIMEOUT, handshake)
        .await
        .map_err(|_| {
            RemoteDesktopError::Connect(format!(
                "{}:{}: connection timed out after {}s",
                config.host,
                config.port,
                CONNECT_TIMEOUT.as_secs()
            ))
        })??;

    let desktop = connection_result.desktop_size;
    let mut image = DecodedImage::new(PixelFormat::RgbA32, desktop.width, desktop.height);
    let mut active_stage = ActiveStage::new(connection_result);
    sink.emit(FrameEvent::Connected {
        width: desktop.width,
        height: desktop.height,
    });

    let (mut reader, mut writer) = split_tokio_framed(upgraded_framed);
    let mut input_db = Database::new();

    // ── Tile encoder off the read loop ───────────────────────────────
    // Dirty rects are copied out of the framebuffer on the read task, then
    // handed to a dedicated encoder task that does the (potentially costly)
    // JPEG re-encode via `spawn_blocking`. Without this, a large full-screen
    // JPEG encode runs inline in the `tokio::select!` read arm and blocks
    // `read_pdu` (stalling the TCP window) and input handling until it
    // finishes. The bounded channel makes `send().await` apply back-pressure
    // to the reader when the encoder falls behind; a single consumer keeps
    // tiles in arrival order.
    let (raw_tx, mut raw_rx) = mpsc::channel::<RawTile>(8);
    let encoder_sink = sink.clone();
    let encoder = runtime::shared().spawn(async move {
        while let Some(raw) = raw_rx.recv().await {
            let tile = tokio::task::spawn_blocking(move || {
                frame::encode_tile(
                    raw.x,
                    raw.y,
                    raw.width,
                    raw.height,
                    raw.rgba,
                    jpeg_threshold,
                )
            })
            .await;
            if let Ok(tile) = tile {
                encoder_sink.emit(FrameEvent::Tile(tile));
            }
        }
    });

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
                            // Copy the dirty rect out now; hand the owned RGBA
                            // to the encoder task. `send().await` back-pressures
                            // the reader if the encoder is behind.
                            if let Some(raw) = extract_region(&image, &region) {
                                if raw_tx.send(raw).await.is_err() {
                                    break 'session;
                                }
                            }
                        }
                        // Server pointer updates: forward the shape as a cursor
                        // event; the frontend overlay tracks the LOCAL mouse, so
                        // the position updates come for free. Hidden -> a 1×1
                        // transparent shape; Default -> an empty shape so the
                        // frontend falls back to the native OS arrow.
                        ActiveStageOutput::PointerBitmap(pointer) => {
                            sink.emit(FrameEvent::Cursor {
                                width: pointer.width,
                                height: pointer.height,
                                hot_x: pointer.hotspot_x,
                                hot_y: pointer.hotspot_y,
                                data: pointer.bitmap_data.clone(),
                            });
                        }
                        ActiveStageOutput::PointerHidden => {
                            sink.emit(FrameEvent::Cursor {
                                width: 1,
                                height: 1,
                                hot_x: 0,
                                hot_y: 0,
                                data: vec![0, 0, 0, 0],
                            });
                        }
                        ActiveStageOutput::PointerDefault => {
                            sink.emit(FrameEvent::Cursor {
                                width: 0,
                                height: 0,
                                hot_x: 0,
                                hot_y: 0,
                                data: Vec::new(),
                            });
                        }
                        // Position is driven client-side by the local mouse, so
                        // a server-reported position is not needed by the overlay.
                        ActiveStageOutput::PointerPosition { .. } => {}
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

    // Drop the sender so the encoder task drains its queue and exits, then wait
    // for it so every already-extracted tile reaches the sink before the
    // terminal `Disconnected` is emitted upstream.
    drop(raw_tx);
    let _ = encoder.await;
    Ok(())
}

async fn upgrade_tls<S>(
    stream: S,
    server_name: &str,
) -> io::Result<(tokio_rustls::client::TlsStream<S>, Vec<u8>)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let mut config = rustls::client::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
        .with_no_client_auth();

    config.resumption = rustls::client::Resumption::disabled();

    let domain = TlsServerName::try_from(server_name.to_owned()).map_err(io::Error::other)?;
    let mut tls_stream = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(domain, stream)
        .await?;
    tls_stream.flush().await?;

    let cert_der = tls_stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .ok_or_else(|| io::Error::other("peer certificate is missing"))?;
    let server_public_key = {
        use x509_cert::der::{Decode as _, Encode as _};
        let cert = x509_cert::Certificate::from_der(cert_der).map_err(io::Error::other)?;
        cert.tbs_certificate
            .subject_public_key_info
            .to_der()
            .map_err(|e| {
                io::Error::other(format!("unable to extract TLS server public key: {e}"))
            })?
    };

    Ok((tls_stream, server_public_key))
}

#[derive(Debug)]
struct NoCertificateVerification;

impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA1,
            rustls::SignatureScheme::ECDSA_SHA1_Legacy,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ED448,
        ]
    }
}

/// One dirty rectangle copied out of the framebuffer, ready to be encoded off
/// the read task.
struct RawTile {
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    rgba: Vec<u8>,
}

/// Slice the updated rectangle out of the full RGBA framebuffer into an owned
/// buffer. `region` is inclusive on all edges. Returns `None` for a degenerate
/// rect. The (costly) tile encoding is done later on the encoder task, not here.
fn extract_region(image: &DecodedImage, region: &InclusiveRectangle) -> Option<RawTile> {
    let img_w = image.width() as usize;
    let left = region.left as usize;
    let top = region.top as usize;
    let right = region.right as usize;
    let bottom = region.bottom as usize;
    if right < left || bottom < top {
        return None;
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
            let dst_row = &mut tile[dst..dst + row_bytes];
            dst_row.copy_from_slice(&data[src..src + row_bytes]);
            // IronRDP's RgbA32 leaves alpha undefined; force opaque for the
            // canvas as part of the row copy (no second pass over the tile).
            for px in dst_row.chunks_exact_mut(4) {
                px[3] = 0xFF;
            }
        }
    }
    Some(RawTile {
        x: left as u16,
        y: top as u16,
        width: w as u16,
        height: h as u16,
        rgba: tile,
    })
}

/// Translate one viewer input event into IronRDP input operations.
fn to_operations(ev: &InputEvent) -> Vec<Operation> {
    match ev {
        InputEvent::PointerMove { x, y } => {
            vec![Operation::MouseMove(MousePosition { x: *x, y: *y })]
        }
        InputEvent::PointerButton {
            x,
            y,
            button,
            pressed,
        } => {
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
        InputEvent::Key {
            scancode,
            extended,
            pressed,
            ..
        } => {
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
        // RDP clipboard runs over the CLIPRDR virtual channel, not the input
        // PDU stream — a follow-up. Ignore for now.
        InputEvent::SetClipboard(_) => Vec::new(),
    }
}

/// Flatten an error's `source()` chain into one string so the leaf cause
/// (a connection reset / unexpected EOF / decode failure behind IronRDP's
/// generic "custom error") reaches the user.
fn describe_err<E: std::error::Error>(err: &E) -> String {
    let mut out = err.to_string();
    let mut src = err.source();
    while let Some(inner) = src {
        out.push_str(" :: ");
        out.push_str(&inner.to_string());
        src = inner.source();
    }
    out
}

/// TOFU-verify the server's public key against the pin store, consulting
/// the prompt callback for unknown / changed keys. Errors (rejecting the
/// connection) when the key is untrusted, so the caller aborts before
/// CredSSP/NLA sends credentials.
async fn verify_server_key(
    config: &RemoteDesktopConfig,
    server_public_key: &[u8],
    cert_prompt: &Option<super::CertPromptCb>,
) -> Result<()> {
    use super::cert_pins::{self, PinCheck};
    use crate::ssh::{HostKeyDecision, HostKeyPromptKind, HostKeyPromptRequest};

    let fingerprint = cert_pins::fingerprint(server_public_key);
    let kind = match cert_pins::check(&config.host, config.port, &fingerprint) {
        PinCheck::Match => return Ok(()),
        PinCheck::Unknown => HostKeyPromptKind::Unknown,
        PinCheck::Mismatch => HostKeyPromptKind::Changed,
    };

    // With a prompt wired, ask the user; otherwise default to accept-new on
    // first contact and reject a changed key (possible MITM).
    let decision = match cert_prompt {
        Some(cb) => {
            let req = HostKeyPromptRequest {
                host: config.host.clone(),
                port: config.port,
                key_type: "RDP TLS certificate".to_string(),
                fingerprint: fingerprint.clone(),
                kind,
            };
            cb(req).await
        }
        None => match kind {
            HostKeyPromptKind::Unknown => HostKeyDecision::Accept,
            HostKeyPromptKind::Changed => HostKeyDecision::Reject,
        },
    };

    match decision {
        HostKeyDecision::Accept => {
            cert_pins::save(&config.host, config.port, &fingerprint);
            Ok(())
        }
        HostKeyDecision::Reject => Err(RemoteDesktopError::Auth(match kind {
            HostKeyPromptKind::Unknown => format!(
                "RDP server certificate for {}:{} was not trusted",
                config.host, config.port
            ),
            HostKeyPromptKind::Changed => format!(
                "RDP server certificate for {}:{} changed and was not trusted (possible MITM)",
                config.host, config.port
            ),
        })),
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
        // Offer BOTH plain TLS (SSL) and NLA/CredSSP (HYBRID) in the X.224
        // security negotiation, and let the server choose. Requesting HYBRID
        // alone makes a TLS-only server (NLA disabled / "less secure" mode)
        // close the connection during negotiation.
        enable_tls: true,
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
        // On top of the IronRDP defaults (no full-window-drag, no menu
        // animations, font smoothing on) also tell the server to skip the
        // desktop wallpaper and cursor shadow — the two biggest sources of
        // redundant screen bitmaps. Themes are left on so the remote desktop
        // still looks normal; this only trims eye-candy redraws.
        performance_flags: PerformanceFlags::default()
            | PerformanceFlags::DISABLE_WALLPAPER
            | PerformanceFlags::DISABLE_CURSOR_SHADOW,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        compression_type: None,
        // Process server pointer updates and surface them as `PointerBitmap`
        // outputs (rather than compositing a software cursor into the
        // framebuffer). With `enable_server_pointer: false` IronRDP drops every
        // pointer PDU and no cursor is drawn anywhere; with software rendering
        // the cursor is baked at the *server*-reported position, which does not
        // track the local mouse. Accelerated rendering hands us the cursor
        // shape as non-premultiplied RGBA, which the frontend overlays at the
        // local mouse position (see FrameEvent::Cursor handling).
        enable_server_pointer: true,
        pointer_software_rendering: false,
        multitransport_flags: None,
    }
}
