//! Minimal stub DNS resolver — sends an A query (and AAAA when no
//! IPv4 record exists) to a user-specified UDP DNS server, parses
//! the answer section, and returns the first IP literal.
//!
//! Hand-rolled to avoid pulling in a full DNS crate (`hickory-resolver`
//! is ~300KB of code) for one call site that only ever makes one
//! query at a time. RFC 1035 §4 defines the wire format.
//!
//! Used by [`super::EgressDns::Custom`] when `resolve_target`
//! needs to bypass the host's `/etc/resolv.conf` — typical case is
//! a profile pointing at a corporate DNS that resolves internal
//! names the host's resolver knows nothing about.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;

const QTYPE_A: u16 = 1;
const QTYPE_AAAA: u16 = 28;
const QCLASS_IN: u16 = 1;

const MAX_RESPONSE_BYTES: usize = 1232; // safe UDP DNS payload limit (EDNS0 friendly)
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolve `host` to an IP literal via `server` (`host:port`,
/// typically port 53). Tries A first, then AAAA. Returns the
/// first answer or an error if nothing comes back.
pub(super) async fn resolve_via(host: &str, server: &str) -> io::Result<IpAddr> {
    let server_addr: SocketAddr = parse_server(server)?;

    // Try A then AAAA. Most internal records are A; AAAA is the
    // graceful fallback for v6-only setups.
    if let Some(ip) = query_one(host, server_addr, QTYPE_A).await? {
        return Ok(ip);
    }
    if let Some(ip) = query_one(host, server_addr, QTYPE_AAAA).await? {
        return Ok(ip);
    }
    Err(io::Error::new(
        io::ErrorKind::AddrNotAvailable,
        format!("no DNS records for {host} via {server}"),
    ))
}

/// Parse a `host:port` server string. Accepts both IP literals and
/// hostnames (resolved via the host's resolver since the user has
/// to bootstrap somehow). Defaults to port 53 when omitted.
fn parse_server(server: &str) -> io::Result<SocketAddr> {
    // Try parsing as host:port first; then as bare IP / bare host.
    if let Ok(addr) = server.parse::<SocketAddr>() {
        return Ok(addr);
    }
    // Bare IP, attach :53.
    if let Ok(ip) = server.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, 53));
    }
    // Hostname[:port] — synchronous resolve via the host's resolver.
    // Tokio's lookup_host is async; use std for simplicity since this
    // bootstrap step is one-shot per profile.
    let with_port = if server.contains(':') {
        server.to_string()
    } else {
        format!("{server}:53")
    };
    let mut iter = std::net::ToSocketAddrs::to_socket_addrs(&with_port)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("DNS server {server}: {e}")))?;
    iter.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("DNS server {server} resolved to nothing"),
        )
    })
}

async fn query_one(host: &str, server: SocketAddr, qtype: u16) -> io::Result<Option<IpAddr>> {
    let query = build_query(host, qtype)?;
    let bind = if server.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let socket = UdpSocket::bind(bind).await?;
    socket.connect(server).await?;

    let send = socket.send(&query);
    tokio::time::timeout(QUERY_TIMEOUT, send)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "DNS send timed out"))??;

    let mut buf = vec![0u8; MAX_RESPONSE_BYTES];
    let recv = socket.recv(&mut buf);
    let n = tokio::time::timeout(QUERY_TIMEOUT, recv)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "DNS recv timed out"))??;
    buf.truncate(n);

    parse_first_ip(&buf, qtype)
}

/// Build a single-question DNS query packet (header + QNAME +
/// QTYPE + QCLASS). Random 16-bit transaction id.
fn build_query(host: &str, qtype: u16) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(40);
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u16)
        .unwrap_or(0);
    buf.extend_from_slice(&id.to_be_bytes());
    // Flags: standard query (RD=1).
    buf.extend_from_slice(&0x0100u16.to_be_bytes());
    // QDCOUNT, ANCOUNT, NSCOUNT, ARCOUNT.
    buf.extend_from_slice(&1u16.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());

    encode_name(host, &mut buf)?;
    buf.extend_from_slice(&qtype.to_be_bytes());
    buf.extend_from_slice(&QCLASS_IN.to_be_bytes());
    Ok(buf)
}

/// Encode `host` as a sequence of length-prefixed labels followed
/// by a zero terminator (`example.com` → `\x07example\x03com\x00`).
fn encode_name(host: &str, buf: &mut Vec<u8>) -> io::Result<()> {
    for label in host.split('.') {
        if label.is_empty() {
            continue;
        }
        if label.len() > 63 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("DNS label too long: {label:?}"),
            ));
        }
        buf.push(label.len() as u8);
        buf.extend_from_slice(label.as_bytes());
    }
    buf.push(0);
    Ok(())
}

/// Pull the first A or AAAA record out of an answer section. Walks
/// past the question section, then iterates answers and matches
/// `qtype`. Pointer compression in NAME fields is supported via
/// `skip_name`.
fn parse_first_ip(buf: &[u8], qtype: u16) -> io::Result<Option<IpAddr>> {
    if buf.len() < 12 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "DNS reply too short"));
    }
    let flags = u16::from_be_bytes([buf[2], buf[3]]);
    let rcode = flags & 0x000F;
    if rcode != 0 {
        // 0 = NOERROR, 3 = NXDOMAIN. Treat anything non-zero as "no answer".
        return Ok(None);
    }
    let qd = u16::from_be_bytes([buf[4], buf[5]]);
    let an = u16::from_be_bytes([buf[6], buf[7]]);
    let mut pos = 12usize;

    // Walk past the question section: NAME + QTYPE(2) + QCLASS(2).
    for _ in 0..qd {
        pos = skip_name(buf, pos)?;
        if pos + 4 > buf.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "DNS reply truncated"));
        }
        pos += 4;
    }

    for _ in 0..an {
        pos = skip_name(buf, pos)?;
        if pos + 10 > buf.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "DNS reply truncated"));
        }
        let rtype = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
        // skip class(2) + ttl(4)
        let rdlen = u16::from_be_bytes([buf[pos + 8], buf[pos + 9]]) as usize;
        pos += 10;
        if pos + rdlen > buf.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "DNS reply truncated"));
        }
        if rtype == qtype {
            if qtype == QTYPE_A && rdlen == 4 {
                let ip = std::net::Ipv4Addr::new(buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]);
                return Ok(Some(IpAddr::V4(ip)));
            }
            if qtype == QTYPE_AAAA && rdlen == 16 {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&buf[pos..pos + 16]);
                return Ok(Some(IpAddr::V6(std::net::Ipv6Addr::from(octets))));
            }
        }
        pos += rdlen;
    }

    Ok(None)
}

/// Walk past a NAME field, handling RFC 1035 §4.1.4 pointer
/// compression. Returns the offset of the byte right after the name.
fn skip_name(buf: &[u8], mut pos: usize) -> io::Result<usize> {
    loop {
        if pos >= buf.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "DNS name overruns reply"));
        }
        let len = buf[pos];
        if len == 0 {
            return Ok(pos + 1);
        }
        if len & 0xC0 == 0xC0 {
            // Compression pointer occupies 2 bytes; we don't have to
            // follow it — the offset right after is the next field.
            return Ok(pos + 2);
        }
        if len & 0xC0 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("DNS name unsupported label type: {len:#04x}"),
            ));
        }
        pos += 1 + len as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_query_round_trips_header_and_question() {
        let q = build_query("example.com", QTYPE_A).unwrap();
        // Header = 12 bytes; QNAME = 1+7+1+3+1 = 13; QTYPE = 2; QCLASS = 2 → 29 total.
        assert_eq!(q.len(), 29);
        // QDCOUNT == 1
        assert_eq!(u16::from_be_bytes([q[4], q[5]]), 1);
        // QNAME matches
        assert_eq!(&q[12..20], b"\x07example");
        assert_eq!(&q[20..24], b"\x03com");
        assert_eq!(q[24], 0);
        // QTYPE = A, QCLASS = IN
        assert_eq!(u16::from_be_bytes([q[25], q[26]]), QTYPE_A);
        assert_eq!(u16::from_be_bytes([q[27], q[28]]), QCLASS_IN);
    }

    #[test]
    fn parse_extracts_a_record_with_pointer_compression() {
        // Hand-build a DNS reply: 1 question (example.com A IN),
        // 1 answer (example.com A IN 60 1.2.3.4) using pointer
        // compression for the answer's NAME field.
        let mut buf = Vec::new();
        // Header
        buf.extend_from_slice(&0xCAFEu16.to_be_bytes()); // id
        buf.extend_from_slice(&0x8180u16.to_be_bytes()); // flags: response, recursion available, NOERROR
        buf.extend_from_slice(&1u16.to_be_bytes()); // qd
        buf.extend_from_slice(&1u16.to_be_bytes()); // an
        buf.extend_from_slice(&0u16.to_be_bytes()); // ns
        buf.extend_from_slice(&0u16.to_be_bytes()); // ar
        // Question
        encode_name("example.com", &mut buf).unwrap();
        buf.extend_from_slice(&QTYPE_A.to_be_bytes());
        buf.extend_from_slice(&QCLASS_IN.to_be_bytes());
        // Answer NAME = pointer to offset 12 (start of question)
        buf.extend_from_slice(&0xC00Cu16.to_be_bytes());
        // TYPE A, CLASS IN, TTL 60, RDLENGTH 4, RDATA 1.2.3.4
        buf.extend_from_slice(&QTYPE_A.to_be_bytes());
        buf.extend_from_slice(&QCLASS_IN.to_be_bytes());
        buf.extend_from_slice(&60u32.to_be_bytes());
        buf.extend_from_slice(&4u16.to_be_bytes());
        buf.extend_from_slice(&[1, 2, 3, 4]);

        let ip = parse_first_ip(&buf, QTYPE_A).unwrap().unwrap();
        assert_eq!(ip, IpAddr::V4(std::net::Ipv4Addr::new(1, 2, 3, 4)));
    }

    #[test]
    fn parse_returns_none_on_nxdomain() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&0u16.to_be_bytes());
        // RCODE = 3 (NXDOMAIN)
        buf.extend_from_slice(&0x8183u16.to_be_bytes());
        buf.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        let result = parse_first_ip(&buf, QTYPE_A).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_server_accepts_bare_ip() {
        let s = parse_server("8.8.8.8").unwrap();
        assert_eq!(s.port(), 53);
        let s = parse_server("1.1.1.1:5353").unwrap();
        assert_eq!(s.port(), 5353);
    }
}
