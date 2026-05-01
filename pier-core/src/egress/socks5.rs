//! SOCKS5 client (RFC 1928) with optional Username/Password auth
//! (RFC 1929). Hand-rolled to avoid pulling in a SOCKS crate for
//! ~150 lines of protocol.

use std::io;
use std::net::IpAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::EgressStream;

const VER: u8 = 0x05;
const CMD_CONNECT: u8 = 0x01;
const RSV: u8 = 0x00;

const METHOD_NO_AUTH: u8 = 0x00;
const METHOD_USER_PASS: u8 = 0x02;
const METHOD_NONE_ACCEPTABLE: u8 = 0xFF;

const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

/// SOCKS5 reply codes per RFC 1928 §6.
fn reply_text(code: u8) -> &'static str {
    match code {
        0x00 => "succeeded",
        0x01 => "general SOCKS server failure",
        0x02 => "connection not allowed by ruleset",
        0x03 => "network unreachable",
        0x04 => "host unreachable",
        0x05 => "connection refused",
        0x06 => "TTL expired",
        0x07 => "command not supported",
        0x08 => "address type not supported",
        _ => "unknown reply code",
    }
}

/// Dial `target_host:target_port` through a SOCKS5 proxy at
/// `proxy_host:proxy_port`. `creds = Some((user, pass))` enables
/// username/password authentication.
pub(super) async fn dial(
    proxy_host: &str,
    proxy_port: u16,
    target_host: &str,
    target_port: u16,
    creds: Option<&(String, String)>,
) -> io::Result<EgressStream> {
    let mut stream = TcpStream::connect((proxy_host, proxy_port)).await?;

    // ----- greeting -----
    let methods: &[u8] = if creds.is_some() {
        &[METHOD_NO_AUTH, METHOD_USER_PASS]
    } else {
        &[METHOD_NO_AUTH]
    };
    let mut greeting = Vec::with_capacity(2 + methods.len());
    greeting.push(VER);
    greeting.push(methods.len() as u8);
    greeting.extend_from_slice(methods);
    stream.write_all(&greeting).await?;

    let mut server_choice = [0u8; 2];
    stream.read_exact(&mut server_choice).await?;
    if server_choice[0] != VER {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("SOCKS5 greeting: unexpected version {}", server_choice[0]),
        ));
    }
    match server_choice[1] {
        METHOD_NO_AUTH => { /* nothing more to do */ }
        METHOD_USER_PASS => {
            let (user, pass) = creds.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "SOCKS5 server requires user/pass but no credentials configured",
                )
            })?;
            do_user_pass_auth(&mut stream, user, pass).await?;
        }
        METHOD_NONE_ACCEPTABLE => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "SOCKS5 server rejected all offered auth methods",
            ));
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("SOCKS5 server picked unsupported method {other:#04x}"),
            ));
        }
    }

    // ----- CONNECT request -----
    let mut req = Vec::with_capacity(7 + target_host.len());
    req.push(VER);
    req.push(CMD_CONNECT);
    req.push(RSV);
    push_addr(&mut req, target_host)?;
    req.extend_from_slice(&target_port.to_be_bytes());
    stream.write_all(&req).await?;

    // ----- CONNECT reply -----
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await?;
    if head[0] != VER {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("SOCKS5 reply: unexpected version {}", head[0]),
        ));
    }
    if head[1] != 0x00 {
        return Err(io::Error::other(format!(
            "SOCKS5 CONNECT failed: {} ({:#04x})",
            reply_text(head[1]),
            head[1]
        )));
    }
    // Drain the bound-address tail so the stream is positioned at
    // the start of the relayed payload.
    match head[3] {
        ATYP_IPV4 => {
            let mut buf = [0u8; 4 + 2];
            stream.read_exact(&mut buf).await?;
        }
        ATYP_IPV6 => {
            let mut buf = [0u8; 16 + 2];
            stream.read_exact(&mut buf).await?;
        }
        ATYP_DOMAIN => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut buf = vec![0u8; len[0] as usize + 2];
            stream.read_exact(&mut buf).await?;
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("SOCKS5 reply: unknown ATYP {other:#04x}"),
            ));
        }
    }

    Ok(Box::new(stream))
}

async fn do_user_pass_auth(stream: &mut TcpStream, user: &str, pass: &str) -> io::Result<()> {
    if user.len() > 255 || pass.len() > 255 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "SOCKS5 user/pass must be <= 255 bytes each",
        ));
    }
    let mut req = Vec::with_capacity(3 + user.len() + pass.len());
    req.push(0x01); // sub-version
    req.push(user.len() as u8);
    req.extend_from_slice(user.as_bytes());
    req.push(pass.len() as u8);
    req.extend_from_slice(pass.as_bytes());
    stream.write_all(&req).await?;

    let mut resp = [0u8; 2];
    stream.read_exact(&mut resp).await?;
    if resp[0] != 0x01 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("SOCKS5 auth: unexpected sub-version {}", resp[0]),
        ));
    }
    if resp[1] != 0x00 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("SOCKS5 user/pass auth rejected (status {})", resp[1]),
        ));
    }
    Ok(())
}

/// Push `ATYP + ADDR` for `host` into `buf`. IP literals use the
/// IPv4 / IPv6 form; everything else is sent as `DOMAINNAME`, which
/// the proxy resolves remotely (DNS-over-tunnel for free).
fn push_addr(buf: &mut Vec<u8>, host: &str) -> io::Result<()> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(v4) => {
                buf.push(ATYP_IPV4);
                buf.extend_from_slice(&v4.octets());
            }
            IpAddr::V6(v6) => {
                buf.push(ATYP_IPV6);
                buf.extend_from_slice(&v6.octets());
            }
        }
    } else {
        let bytes = host.as_bytes();
        if bytes.len() > 255 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SOCKS5 domain name must be <= 255 bytes",
            ));
        }
        buf.push(ATYP_DOMAIN);
        buf.push(bytes.len() as u8);
        buf.extend_from_slice(bytes);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Drive a fake SOCKS5 server on a loopback listener, accept a
    /// NoAuth CONNECT to 1.2.3.4:443, then echo "ok" through the
    /// tunnel. Verifies framing on both sides without needing an
    /// actual SOCKS proxy.
    #[tokio::test]
    async fn socks5_no_auth_connect_round_trip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();

            let mut greeting = [0u8; 3];
            sock.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting[0], VER);
            assert_eq!(greeting[1], 1);
            assert_eq!(greeting[2], METHOD_NO_AUTH);

            sock.write_all(&[VER, METHOD_NO_AUTH]).await.unwrap();

            let mut head = [0u8; 4];
            sock.read_exact(&mut head).await.unwrap();
            assert_eq!(head[0], VER);
            assert_eq!(head[1], CMD_CONNECT);
            assert_eq!(head[3], ATYP_IPV4);

            let mut ip = [0u8; 4];
            sock.read_exact(&mut ip).await.unwrap();
            assert_eq!(ip, [1, 2, 3, 4]);

            let mut port = [0u8; 2];
            sock.read_exact(&mut port).await.unwrap();
            assert_eq!(u16::from_be_bytes(port), 443);

            // BND.ADDR = 0.0.0.0, BND.PORT = 0 — typical for a CONNECT
            // reply since the bound address rarely matters to clients.
            sock.write_all(&[VER, 0x00, RSV, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();

            // Relay: read 2 bytes from client, echo "ok".
            let mut payload = [0u8; 2];
            sock.read_exact(&mut payload).await.unwrap();
            assert_eq!(&payload, b"hi");
            sock.write_all(b"ok").await.unwrap();
        });

        let mut tunneled = dial(
            "127.0.0.1",
            proxy_addr.port(),
            &Ipv4Addr::new(1, 2, 3, 4).to_string(),
            443,
            None,
        )
        .await
        .expect("dial");
        tunneled.write_all(b"hi").await.unwrap();
        let mut got = [0u8; 2];
        tunneled.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"ok");

        server.await.unwrap();
    }

    #[tokio::test]
    async fn socks5_user_pass_auth_round_trip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();

            // Greeting: client offers both methods.
            let mut greeting = [0u8; 4];
            sock.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting[0], VER);
            assert_eq!(greeting[1], 2);
            assert_eq!(&greeting[2..4], &[METHOD_NO_AUTH, METHOD_USER_PASS]);

            // We pick user/pass.
            sock.write_all(&[VER, METHOD_USER_PASS]).await.unwrap();

            // Auth: read sub-version + ULEN + USER + PLEN + PASS.
            let mut hdr = [0u8; 2];
            sock.read_exact(&mut hdr).await.unwrap();
            assert_eq!(hdr[0], 0x01);
            assert_eq!(hdr[1], 4);
            let mut user = [0u8; 4];
            sock.read_exact(&mut user).await.unwrap();
            assert_eq!(&user, b"abcd");
            let mut plen = [0u8; 1];
            sock.read_exact(&mut plen).await.unwrap();
            assert_eq!(plen[0], 5);
            let mut pass = [0u8; 5];
            sock.read_exact(&mut pass).await.unwrap();
            assert_eq!(&pass, b"hunte");

            sock.write_all(&[0x01, 0x00]).await.unwrap();

            // CONNECT — domain form, since target is a hostname.
            let mut head = [0u8; 4];
            sock.read_exact(&mut head).await.unwrap();
            assert_eq!(head[3], ATYP_DOMAIN);
            let mut dlen = [0u8; 1];
            sock.read_exact(&mut dlen).await.unwrap();
            let mut name = vec![0u8; dlen[0] as usize];
            sock.read_exact(&mut name).await.unwrap();
            assert_eq!(name, b"example.com");
            let mut port = [0u8; 2];
            sock.read_exact(&mut port).await.unwrap();
            assert_eq!(u16::from_be_bytes(port), 22);

            sock.write_all(&[VER, 0x00, RSV, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
        });

        let _ = dial(
            "127.0.0.1",
            proxy_addr.port(),
            "example.com",
            22,
            Some(&("abcd".to_string(), "hunte".to_string())),
        )
        .await
        .expect("dial");

        server.await.unwrap();
    }

    #[tokio::test]
    async fn socks5_failed_reply_surfaces_typed_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut greeting = [0u8; 3];
            sock.read_exact(&mut greeting).await.unwrap();
            sock.write_all(&[VER, METHOD_NO_AUTH]).await.unwrap();
            let mut head = [0u8; 4];
            sock.read_exact(&mut head).await.unwrap();
            // skip address tail (IPv4 + port)
            let mut tail = [0u8; 6];
            sock.read_exact(&mut tail).await.unwrap();
            // Reject with "host unreachable".
            sock.write_all(&[VER, 0x04, RSV, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
        });

        let result = dial(
            "127.0.0.1",
            proxy_addr.port(),
            &Ipv4Addr::new(1, 2, 3, 4).to_string(),
            443,
            None,
        )
        .await;
        let err = match result {
            Ok(_) => panic!("dial unexpectedly succeeded"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("host unreachable"),
            "unexpected error: {err}"
        );

        server.await.unwrap();
    }
}
