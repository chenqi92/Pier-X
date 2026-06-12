//! Egress profiles — describe how an outgoing connection reaches its
//! target without touching the host's global routing.
//!
//! Each profile is a named, persisted entity (lives in
//! [`crate::connections::ConnectionStore`] alongside SSH connections).
//! A connection entry references at most one profile via
//! `egress_id`; absence of a reference means a plain TCP dial.
//!
//! See `docs/PRODUCT-SPEC.md` §3.4 (semantics, supported kinds, DNS
//! strategy) and §8.6 (this contract).
//!
//! ## Stage A scope
//!
//! Only `None`, `Socks5`, and `Http` (HTTP CONNECT) actually dial
//! today. `SshJump`, `Wireguard`, and `ExternalVpn` parse and
//! round-trip through serde so the store schema is forward-stable,
//! but [`resolve_tcp`] returns [`io::ErrorKind::Unsupported`] for
//! them. Stage B wires the runtime hookups for the remaining kinds.
//!
//! Nothing in this module touches the host routing table, system
//! DNS, or system proxy settings; doing so would violate
//! `PRODUCT-SPEC.md` §1.2.

mod dns;
pub mod forwarder;
mod http_connect;
mod none;
mod socks5;
pub mod vpn_subprocess;

pub use forwarder::EgressForwarder;
pub use vpn_subprocess::VpnProcess;

use std::future::Future;
use std::io;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

/// Bidirectional byte stream returned by [`resolve_tcp`]. Callers
/// treat it as an opaque socket — it might be a [`tokio::net::TcpStream`],
/// a SOCKS5-tunneled stream, or any future transport that satisfies
/// the bounds.
pub type EgressStream = Box<dyn ReadWriteUnpin + Send + Unpin>;

/// Trait alias so [`EgressStream`] stays one line. Blanket-impl'd
/// for every type that already satisfies the four bounds.
pub trait ReadWriteUnpin: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite + ?Sized> ReadWriteUnpin for T {}

/// Boxed future returned by [`EgressContext`] hooks. Pinned + boxed
/// so trait objects with async methods stay object-safe.
pub type EgressFuture<'a> = Pin<Box<dyn Future<Output = io::Result<EgressStream>> + Send + 'a>>;

/// External capabilities that egress kinds may need to dial through
/// other crate-level subsystems (SSH for ssh-jump today; reserved for
/// WireGuard / external-VPN later). The application layer (src-tauri)
/// supplies the concrete implementation; the egress module itself
/// stays free of any cross-module dependency on `crate::ssh`.
///
/// `resolve_tcp_with` calls into this trait only when the selected
/// profile actually needs it (e.g. `EgressKind::SshJump`). Profiles
/// that don't need a context — `None` / `Socks5` / `Http` — work
/// even when no context is provided.
pub trait EgressContext: Send + Sync {
    /// Dial `target_host:target_port` through a saved SSH connection
    /// referenced by `via_connection`. Implementations are expected
    /// to bind the lifetime of the underlying SSH transport to the
    /// returned [`EgressStream`] so the channel doesn't die mid-flight.
    fn ssh_jump_dial<'a>(
        &'a self,
        via_connection: &'a str,
        target_host: &'a str,
        target_port: u16,
    ) -> EgressFuture<'a>;
}

/// One egress profile. Persisted as part of the connections store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressProfile {
    /// Stable id referenced by `SshConfig::egress_id`.
    pub id: String,
    /// Human-readable label shown in the picker.
    pub name: String,
    /// What kind of tunnel this profile provides (and its parameters).
    #[serde(flatten)]
    pub kind: EgressKind,
    /// DNS strategy. Falls back to [`EgressKind::default_dns`] when
    /// omitted on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns: Option<EgressDns>,
}

impl EgressProfile {
    /// Resolved DNS strategy — explicit if set, else the kind default.
    pub fn effective_dns(&self) -> EgressDns {
        self.dns.clone().unwrap_or_else(|| self.kind.default_dns())
    }
}

/// Supported tunnel kinds. The serde tag is `kind` so the on-disk
/// shape is `{ "id": "...", "name": "...", "kind": "socks5", "host": ..., "port": ... }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EgressKind {
    /// No tunnel — equivalent to absence of a profile reference.
    /// Useful as an explicit "Direct" entry in pickers.
    None,
    /// SOCKS5 proxy (RFC 1928). NoAuth and Username/Password (RFC 1929).
    Socks5 {
        /// Proxy host (IP literal or hostname).
        host: String,
        /// Proxy TCP port.
        port: u16,
        /// Optional credential ref. Keyring blob is `"user\npassword"`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<EgressAuthRef>,
    },
    /// HTTP CONNECT proxy (RFC 7231 §4.3.6).
    Http {
        /// Proxy host (IP literal or hostname).
        host: String,
        /// Proxy TCP port.
        port: u16,
        /// Optional credential ref for HTTP Basic. Same blob shape
        /// as `Socks5`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<EgressAuthRef>,
    },
    /// Jump through an existing saved SSH connection. Stage B.
    SshJump {
        /// References an [`crate::ssh::SshConfig`] by name.
        ///
        /// Serialized as `viaConnection` — the profile crosses the
        /// Tauri IPC boundary verbatim and the frontend speaks
        /// camelCase. The snake_case alias keeps any pre-existing
        /// on-disk store entries readable.
        #[serde(rename = "viaConnection", alias = "via_connection")]
        via_connection: String,
    },
    /// WireGuard via system `wg-quick` subprocess. The actual peer
    /// config (Interface PrivateKey, Peer PublicKey, AllowedIPs,
    /// Endpoint, …) lives in a standard wg-quick `.conf` file the
    /// user supplies — Pier-X never sees the private key, just
    /// hands the path to `wg-quick up`.
    ///
    /// `conf_path` empty → falls back to
    /// `~/.config/pier-x/egress/<profile-id>.conf` (legacy default,
    /// useful if you want one profile per app-managed file).
    Wireguard {
        /// Absolute path to a wg-quick compatible `.conf` file.
        /// Empty / omitted → the default per-profile path under
        /// the app data dir.
        ///
        /// Serialized as `confPath` (camelCase over IPC); the
        /// snake_case alias keeps old on-disk entries readable.
        #[serde(default, rename = "confPath", alias = "conf_path")]
        conf_path: String,
    },
    /// External VPN binary (openvpn / openconnect). Stage B+.
    /// Cargo feature `egress-external-vpn` will gate the actual
    /// implementation; today the variant only parses for round-trip.
    ExternalVpn {
        /// Which engine to invoke.
        engine: ExternalVpnEngine,
        /// Path to the config (`.ovpn` for OpenVPN, hostname or `.xml`
        /// for OpenConnect).
        config: String,
        /// OpenConnect protocol flavour, mapped to `--protocol=<x>`.
        /// `None` means the binary default (AnyConnect). Ignored for
        /// the OpenVPN engine.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        protocol: Option<OpenConnectProtocol>,
    },
}

/// External VPN engine flavor. See PRODUCT-SPEC §3.4 last row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalVpnEngine {
    /// `openvpn` binary.
    OpenVpn,
    /// `openconnect` binary (handles AnyConnect / Pulse subset).
    OpenConnect,
}

/// WebVPN dialect passed to `openconnect --protocol=<x>`. Variant
/// serde names match the binary's accepted values exactly, so
/// [`OpenConnectProtocol::flag_value`] and the wire format agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenConnectProtocol {
    /// Cisco AnyConnect / ocserv (binary default).
    Anyconnect,
    /// Juniper Network Connect.
    Nc,
    /// Palo Alto GlobalProtect.
    Gp,
    /// Pulse Connect Secure / Ivanti.
    Pulse,
    /// F5 BIG-IP.
    F5,
    /// Fortinet FortiGate.
    Fortinet,
    /// Array Networks AG.
    Array,
}

impl OpenConnectProtocol {
    /// The value accepted by `openconnect --protocol=`.
    pub fn flag_value(self) -> &'static str {
        match self {
            OpenConnectProtocol::Anyconnect => "anyconnect",
            OpenConnectProtocol::Nc => "nc",
            OpenConnectProtocol::Gp => "gp",
            OpenConnectProtocol::Pulse => "pulse",
            OpenConnectProtocol::F5 => "f5",
            OpenConnectProtocol::Fortinet => "fortinet",
            OpenConnectProtocol::Array => "array",
        }
    }
}

/// Reference to a credential stored in the OS keyring under the
/// `pier-x.egress.*` namespace. The blob convention is one logical
/// secret string; for username/password it's `"user\npassword"`,
/// for WG it's the base64 private key alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressAuthRef {
    /// Keyring key, conventionally `pier-x.egress.<profile-id>`.
    /// camelCase over IPC; snake_case alias for old on-disk entries.
    #[serde(rename = "credentialId", alias = "credential_id")]
    pub credential_id: String,
}

/// DNS resolution strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum EgressDns {
    /// Resolve target hostname locally, send the resulting IP through
    /// the tunnel. Default for SOCKS5 / HTTP / None.
    Passthrough,
    /// Send the hostname through the tunnel; the far side resolves.
    /// Default for SshJump / WireGuard / ExternalVpn.
    Tunnel,
    /// Stub-resolve via a custom DNS server, then tunnel the IP.
    /// Stage B; resolve_tcp currently degrades this to `Passthrough`.
    Custom {
        /// `host:port` of the DNS server (UDP).
        server: String,
    },
}

impl EgressKind {
    /// The sensible default DNS strategy for this kind when the
    /// profile omits an explicit `dns`.
    pub fn default_dns(&self) -> EgressDns {
        match self {
            EgressKind::None
            | EgressKind::Socks5 { .. }
            | EgressKind::Http { .. } => EgressDns::Passthrough,
            EgressKind::SshJump { .. }
            | EgressKind::Wireguard { .. }
            | EgressKind::ExternalVpn { .. } => EgressDns::Tunnel,
        }
    }
}

/// Dial `target_host:target_port` through `profile` without an
/// `EgressContext`. Convenience over [`resolve_tcp_with`] for kinds
/// that don't need cross-module hooks (`None` / `Socks5` / `Http`).
/// `SshJump` will fail with [`io::ErrorKind::Unsupported`] here; use
/// [`resolve_tcp_with`] and supply an [`EgressContext`].
pub async fn resolve_tcp(
    profile: Option<&EgressProfile>,
    target_host: &str,
    target_port: u16,
) -> io::Result<EgressStream> {
    resolve_tcp_with(profile, target_host, target_port, None).await
}

/// Dial `target_host:target_port` through `profile`, optionally
/// using `ctx` for kinds that need outside help (SSH-jump today).
///
/// `profile = None` is a plain TCP dial. For SOCKS5 / HTTP, the
/// caller does not need to pre-resolve `target_host`; the proxy
/// either gets a hostname (when `dns == Tunnel`) or a pre-resolved
/// IP literal (when `dns == Passthrough`, the default).
pub async fn resolve_tcp_with(
    profile: Option<&EgressProfile>,
    target_host: &str,
    target_port: u16,
    ctx: Option<&dyn EgressContext>,
) -> io::Result<EgressStream> {
    let Some(profile) = profile else {
        return none::dial_direct(target_host, target_port).await;
    };

    let target = resolve_target(profile, target_host).await?;

    match &profile.kind {
        EgressKind::None => none::dial_direct(&target, target_port).await,
        EgressKind::Socks5 { host, port, auth } => {
            let creds = resolve_auth(auth)?;
            socks5::dial(host, *port, &target, target_port, creds.as_ref()).await
        }
        EgressKind::Http { host, port, auth } => {
            let creds = resolve_auth(auth)?;
            http_connect::dial(host, *port, &target, target_port, creds.as_ref()).await
        }
        EgressKind::SshJump { via_connection } => {
            let Some(ctx) = ctx else {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "ssh-jump egress requires an EgressContext implementation",
                ));
            };
            ctx.ssh_jump_dial(via_connection, &target, target_port).await
        }
        EgressKind::Wireguard { .. } | EgressKind::ExternalVpn { .. } => {
            // Subprocess VPN model — see PRODUCT-SPEC §3.4 and
            // [`vpn_subprocess`]. Lifecycle of the spawned VPN
            // client is owned by the application layer (src-tauri
            // starts the process when the profile is enabled, drops
            // it when the profile is removed). At dial time we
            // assume the OS tun + routes are already in place, so
            // the actual TCP connect is byte-for-byte identical to
            // a direct dial — the kernel transparently routes
            // through whatever interface the VPN client installed.
            none::dial_direct(&target, target_port).await
        }
    }
}

/// Resolve `target_host` per the profile's DNS strategy.
///
/// `Passthrough` does a local lookup so the proxy sees an IP.
/// `Tunnel` returns the hostname unchanged so the far side resolves.
/// `Custom` sends a single A/AAAA query to the user-specified server
/// via [`dns::resolve_via`]; on failure it falls back to the system
/// resolver so the connection doesn't dead-end on a flaky DNS box.
async fn resolve_target(profile: &EgressProfile, target_host: &str) -> io::Result<String> {
    // IP literals never need resolution regardless of mode.
    if target_host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(target_host.to_string());
    }
    match profile.effective_dns() {
        EgressDns::Tunnel => Ok(target_host.to_string()),
        EgressDns::Passthrough => system_resolve(target_host).await,
        EgressDns::Custom { server } => match dns::resolve_via(target_host, &server).await {
            Ok(ip) => Ok(ip.to_string()),
            Err(e) => {
                log::warn!(
                    "egress custom DNS server '{server}' failed for {target_host}: {e} — falling back to system resolver"
                );
                system_resolve(target_host).await
            }
        },
    }
}

async fn system_resolve(host: &str) -> io::Result<String> {
    let lookup = format!("{host}:0");
    let mut iter = tokio::net::lookup_host(lookup).await?;
    let first = iter.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("no DNS records for {host}"),
        )
    })?;
    Ok(first.ip().to_string())
}

#[cfg_attr(not(test), allow(dead_code))]
fn kind_label(kind: &EgressKind) -> &'static str {
    match kind {
        EgressKind::None => "none",
        EgressKind::Socks5 { .. } => "socks5",
        EgressKind::Http { .. } => "http",
        EgressKind::SshJump { .. } => "ssh-jump",
        EgressKind::Wireguard { .. } => "wireguard",
        EgressKind::ExternalVpn { .. } => "external-vpn",
    }
}

/// Resolve `(user, password)` from a keyring credential ref. The blob
/// is split on the first `\n`; everything after is the password.
fn resolve_auth(auth: &Option<EgressAuthRef>) -> io::Result<Option<(String, String)>> {
    let Some(auth) = auth else {
        return Ok(None);
    };
    match crate::credentials::get(&auth.credential_id) {
        Ok(Some(blob)) => {
            let mut parts = blob.splitn(2, '\n');
            let user = parts.next().unwrap_or_default().to_string();
            let pass = parts.next().unwrap_or_default().to_string();
            Ok(Some((user, pass)))
        }
        Ok(None) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("egress credential missing in keyring: {}", auth.credential_id),
        )),
        Err(e) => Err(io::Error::other(e.to_string())),
    }
}

/// Look up a profile by id. Returns `None` when `id` is `None` or the
/// profile no longer exists — the UI is expected to auto-degrade a
/// dangling reference to direct, so callers can treat the result as
/// "what to dial through, possibly nothing".
pub fn lookup<'a>(profiles: &'a [EgressProfile], id: Option<&str>) -> Option<&'a EgressProfile> {
    let id = id?;
    profiles.iter().find(|p| p.id == id)
}

/// Outcome of [`probe_tcp`] — measured TCP-handshake latency or a
/// typed error wrapped into a string.
#[derive(Debug, Clone)]
pub struct ProbeOutcome {
    /// Round-trip duration of the dial. Always populated — even on
    /// failure, this is the time spent before erroring out, which
    /// helps tell "instant refusal" from "slow timeout" apart.
    pub elapsed: std::time::Duration,
    /// `Ok(())` on success; `Err(message)` on failure. The byte
    /// stream is dropped immediately — this is a reachability
    /// probe, not a real connection.
    pub result: Result<(), String>,
}

/// Reachability probe: dial `host:port` through `profile` (or
/// directly when `profile = None`), wait at most `timeout`, then
/// drop the resulting stream. Used by the "Test connection" button
/// in the egress profile editor.
pub async fn probe_tcp(
    profile: Option<&EgressProfile>,
    target_host: &str,
    target_port: u16,
    timeout: std::time::Duration,
    ctx: Option<&dyn EgressContext>,
) -> ProbeOutcome {
    let started = std::time::Instant::now();
    let dial = resolve_tcp_with(profile, target_host, target_port, ctx);
    let outcome = match tokio::time::timeout(timeout, dial).await {
        Ok(Ok(_stream)) => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!("probe deadline ({timeout:?}) exceeded")),
    };
    ProbeOutcome {
        elapsed: started.elapsed(),
        result: outcome,
    }
}

/// Sync wrapper for [`probe_tcp`]. Runs on the shared runtime so
/// the caller doesn't have to spin up its own.
pub fn probe_tcp_blocking(
    profile: Option<&EgressProfile>,
    target_host: &str,
    target_port: u16,
    timeout: std::time::Duration,
    ctx: Option<&dyn EgressContext>,
) -> ProbeOutcome {
    crate::ssh::runtime::shared()
        .block_on(probe_tcp(profile, target_host, target_port, timeout, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dns_picks_kind_appropriate_strategy() {
        let socks = EgressProfile {
            id: "p".into(),
            name: "n".into(),
            kind: EgressKind::Socks5 {
                host: "127.0.0.1".into(),
                port: 1080,
                auth: None,
            },
            dns: None,
        };
        assert!(matches!(socks.effective_dns(), EgressDns::Passthrough));

        let wg = EgressProfile {
            id: "w".into(),
            name: "wg".into(),
            kind: EgressKind::Wireguard {
                conf_path: String::new(),
            },
            dns: None,
        };
        assert!(matches!(wg.effective_dns(), EgressDns::Tunnel));
    }

    #[test]
    fn explicit_dns_wins_over_default() {
        let p = EgressProfile {
            id: "p".into(),
            name: "n".into(),
            kind: EgressKind::Http {
                host: "proxy".into(),
                port: 8080,
                auth: None,
            },
            dns: Some(EgressDns::Tunnel),
        };
        assert!(matches!(p.effective_dns(), EgressDns::Tunnel));
    }

    #[test]
    fn round_trip_socks5_no_auth() {
        let p = EgressProfile {
            id: "p1".into(),
            name: "Office SOCKS".into(),
            kind: EgressKind::Socks5 {
                host: "10.0.0.1".into(),
                port: 1080,
                auth: None,
            },
            dns: None,
        };
        let json = serde_json::to_string(&p).expect("serialize");
        // Sanity: tagged enum should flatten into the top object.
        assert!(json.contains("\"kind\":\"socks5\""));
        assert!(json.contains("\"host\":\"10.0.0.1\""));
        let parsed: EgressProfile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, p);
    }

    #[test]
    fn round_trip_external_vpn_keeps_engine_and_protocol() {
        let p = EgressProfile {
            id: "p2".into(),
            name: "AnyConnect".into(),
            kind: EgressKind::ExternalVpn {
                engine: ExternalVpnEngine::OpenConnect,
                config: "vpn.corp.example.com".into(),
                protocol: Some(OpenConnectProtocol::Gp),
            },
            dns: None,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"protocol\":\"gp\""));
        let parsed: EgressProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, p);
    }

    #[test]
    fn multi_word_fields_serialize_camel_case() {
        // The profile crosses the Tauri IPC boundary verbatim, and
        // the frontend types (src/lib/types.ts) speak camelCase.
        let jump = EgressProfile {
            id: "j".into(),
            name: "jump".into(),
            kind: EgressKind::SshJump {
                via_connection: "prod-bastion".into(),
            },
            dns: None,
        };
        let json = serde_json::to_string(&jump).unwrap();
        assert!(json.contains("\"viaConnection\":\"prod-bastion\""));

        let wg = EgressProfile {
            id: "w".into(),
            name: "wg".into(),
            kind: EgressKind::Wireguard {
                conf_path: "/etc/wireguard/wg0.conf".into(),
            },
            dns: None,
        };
        let json = serde_json::to_string(&wg).unwrap();
        assert!(json.contains("\"confPath\""));

        let socks = EgressProfile {
            id: "s".into(),
            name: "s".into(),
            kind: EgressKind::Socks5 {
                host: "10.0.0.1".into(),
                port: 1080,
                auth: Some(EgressAuthRef {
                    credential_id: "pier-x.egress.s".into(),
                }),
            },
            dns: None,
        };
        let json = serde_json::to_string(&socks).unwrap();
        assert!(json.contains("\"credentialId\""));
    }

    #[test]
    fn snake_case_aliases_still_deserialize() {
        // Pre-rename on-disk entries used snake_case field names.
        let parsed: EgressProfile = serde_json::from_str(
            r#"{"id":"j","name":"jump","kind":"ssh_jump","via_connection":"prod-bastion"}"#,
        )
        .unwrap();
        assert!(matches!(
            parsed.kind,
            EgressKind::SshJump { ref via_connection } if via_connection == "prod-bastion"
        ));

        let parsed: EgressProfile = serde_json::from_str(
            r#"{"id":"w","name":"wg","kind":"wireguard","conf_path":"/x/wg0.conf"}"#,
        )
        .unwrap();
        assert!(matches!(
            parsed.kind,
            EgressKind::Wireguard { ref conf_path } if conf_path == "/x/wg0.conf"
        ));

        let parsed: EgressProfile = serde_json::from_str(
            r#"{"id":"s","name":"s","kind":"socks5","host":"h","port":1080,"auth":{"credential_id":"pier-x.egress.s"}}"#,
        )
        .unwrap();
        assert!(matches!(
            parsed.kind,
            EgressKind::Socks5 { auth: Some(ref a), .. } if a.credential_id == "pier-x.egress.s"
        ));
    }

    #[test]
    fn lookup_missing_id_returns_none() {
        let profiles = vec![EgressProfile {
            id: "a".into(),
            name: "A".into(),
            kind: EgressKind::None,
            dns: None,
        }];
        assert!(lookup(&profiles, Some("a")).is_some());
        assert!(lookup(&profiles, Some("nope")).is_none());
        assert!(lookup(&profiles, None).is_none());
    }
}
