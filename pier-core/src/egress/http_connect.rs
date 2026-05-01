//! HTTP CONNECT proxy client (RFC 7231 §4.3.6) with optional
//! HTTP Basic auth (RFC 7617). Hand-rolled — base64 is the only
//! awkward bit, and we have a 30-line standard table.

use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::EgressStream;

/// Dial `target_host:target_port` through an HTTP proxy at
/// `proxy_host:proxy_port`. `creds = Some((user, pass))` enables
/// HTTP Basic authentication.
pub(super) async fn dial(
    proxy_host: &str,
    proxy_port: u16,
    target_host: &str,
    target_port: u16,
    creds: Option<&(String, String)>,
) -> io::Result<EgressStream> {
    let mut stream = TcpStream::connect((proxy_host, proxy_port)).await?;

    // Build the CONNECT request. Using HTTP/1.1 and a Host header
    // is the common form; some proxies (older Squid, some appliance
    // boxes) reject 1.0 CONNECTs.
    let mut req = String::with_capacity(128);
    req.push_str(&format!("CONNECT {target_host}:{target_port} HTTP/1.1\r\n"));
    req.push_str(&format!("Host: {target_host}:{target_port}\r\n"));
    if let Some((user, pass)) = creds {
        let token = base64_encode(format!("{user}:{pass}").as_bytes());
        req.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    req.push_str("Proxy-Connection: keep-alive\r\n");
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).await?;

    // Read the response head (status line + headers + blank line).
    // CONNECT responses don't have a body before the tunnel begins,
    // so anything after the blank line is already relayed payload.
    let head = read_until_double_crlf(&mut stream).await?;
    let head_str = std::str::from_utf8(&head)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    let status = parse_status_line(head_str)?;
    if !(200..300).contains(&status) {
        return Err(io::Error::other(format!(
            "HTTP CONNECT failed: status {status}"
        )));
    }

    Ok(Box::new(stream))
}

/// Pull bytes from `stream` until we see the canonical `\r\n\r\n`
/// header terminator. Bounded at 16 KiB to keep a hostile proxy
/// from making us allocate forever.
async fn read_until_double_crlf(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(512);
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "HTTP CONNECT: proxy closed connection before responding",
            ));
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            return Ok(buf);
        }
        if buf.len() > 16 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP CONNECT: response head exceeded 16KiB without terminator",
            ));
        }
    }
}

/// Parse the numeric status code out of `HTTP/1.1 200 Connection established\r\n…`.
fn parse_status_line(head: &str) -> io::Result<u16> {
    let line = head.split("\r\n").next().unwrap_or("");
    let mut parts = line.split_ascii_whitespace();
    let _version = parts.next();
    let code = parts.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("HTTP CONNECT: malformed status line {line:?}"),
        )
    })?;
    code.parse::<u16>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("HTTP CONNECT: non-numeric status code {code:?}"),
        )
    })
}

/// Standard base64 encode (RFC 4648 §4) — about 30 lines, kept here
/// to dodge a separate `base64` crate dependency for one call site.
fn base64_encode(bytes: &[u8]) -> String {
    const TBL: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    let mut chunks = bytes.chunks_exact(3);
    for chunk in chunks.by_ref() {
        let b0 = chunk[0] as u32;
        let b1 = chunk[1] as u32;
        let b2 = chunk[2] as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TBL[((n >> 18) & 0x3F) as usize] as char);
        out.push(TBL[((n >> 12) & 0x3F) as usize] as char);
        out.push(TBL[((n >> 6) & 0x3F) as usize] as char);
        out.push(TBL[(n & 0x3F) as usize] as char);
    }
    let rem = chunks.remainder();
    match rem.len() {
        1 => {
            let n = (rem[0] as u32) << 16;
            out.push(TBL[((n >> 18) & 0x3F) as usize] as char);
            out.push(TBL[((n >> 12) & 0x3F) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let n = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
            out.push(TBL[((n >> 18) & 0x3F) as usize] as char);
            out.push(TBL[((n >> 12) & 0x3F) as usize] as char);
            out.push(TBL[((n >> 6) & 0x3F) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn base64_matches_known_vectors() {
        // RFC 4648 §10 test vectors.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // The classic Basic-auth test pair "Aladdin:open sesame".
        assert_eq!(
            base64_encode(b"Aladdin:open sesame"),
            "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
    }

    #[test]
    fn parse_status_picks_off_the_code() {
        assert_eq!(
            parse_status_line("HTTP/1.1 200 Connection established\r\n").unwrap(),
            200
        );
        assert_eq!(parse_status_line("HTTP/1.0 407 Proxy auth\r\n").unwrap(), 407);
        assert!(parse_status_line("garbage").is_err());
    }

    #[tokio::test]
    async fn http_connect_success_round_trip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // Read the request head ourselves so we can assert on it.
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                let _ = sock.read_exact(&mut byte).await.unwrap();
                buf.push(byte[0]);
                if buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let req = std::str::from_utf8(&buf).unwrap();
            assert!(req.starts_with("CONNECT example.com:443 HTTP/1.1\r\n"));
            assert!(req.contains("Host: example.com:443\r\n"));
            assert!(!req.contains("Proxy-Authorization"));

            sock.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await
                .unwrap();

            // Echo what the client sends.
            let mut payload = [0u8; 2];
            sock.read_exact(&mut payload).await.unwrap();
            assert_eq!(&payload, b"hi");
            sock.write_all(b"ok").await.unwrap();
        });

        let mut tunneled = dial("127.0.0.1", proxy_addr.port(), "example.com", 443, None)
            .await
            .expect("dial");
        tunneled.write_all(b"hi").await.unwrap();
        let mut got = [0u8; 2];
        tunneled.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"ok");

        server.await.unwrap();
    }

    #[tokio::test]
    async fn http_connect_emits_basic_auth_header() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                sock.read_exact(&mut byte).await.unwrap();
                buf.push(byte[0]);
                if buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let req = std::str::from_utf8(&buf).unwrap();
            assert!(req.contains("Proxy-Authorization: Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==\r\n"));
            sock.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        });

        let _ = dial(
            "127.0.0.1",
            proxy_addr.port(),
            "example.com",
            22,
            Some(&("Aladdin".to_string(), "open sesame".to_string())),
        )
        .await
        .expect("dial");

        server.await.unwrap();
    }

    #[tokio::test]
    async fn http_connect_407_surfaces_status() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                sock.read_exact(&mut byte).await.unwrap();
                buf.push(byte[0]);
                if buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            sock.write_all(
                b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic\r\n\r\n",
            )
            .await
            .unwrap();
        });

        let result = dial("127.0.0.1", proxy_addr.port(), "example.com", 443, None).await;
        let err = match result {
            Ok(_) => panic!("dial unexpectedly succeeded"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("407"), "unexpected: {err}");

        server.await.unwrap();
    }
}
