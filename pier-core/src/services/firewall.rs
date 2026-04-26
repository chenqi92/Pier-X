//! Firewall / network observability over SSH.
//!
//! Snapshots a remote host's firewall posture using only base tools that
//! ship by default on every mainstream Linux distro (no nethogs / iftop /
//! tcpdump): `ss` from iproute2 (universal), `/proc/net/dev`,
//! `iptables-save` (provided by both legacy `iptables` and the
//! `iptables-nft` shim on modern distros), `nft`, `ufw`, `firewall-cmd`.
//!
//! Detection precedence: firewalld → ufw → nftables → iptables. The
//! first one whose CLI exists *and* whose service/ruleset is active wins.
//! "iptables" is the catch-all; on RHEL 9 / Debian 12+ it's actually
//! `iptables-nft`, but the CLI surface and `iptables-save` output match
//! the legacy form, so the panel treats them as one backend.
//!
//! Reads are best-effort: a non-root SSH user may not see other users'
//! PIDs in `ss -p` output, and `iptables-save` will return empty for
//! unprivileged sessions. We surface whatever is readable rather than
//! erroring — the UI shows a "elevate via sudo to see full ruleset"
//! hint when the user lacks privileges.
//!
//! Writes are *not* in this module. They go via [`terminal_write`] from
//! the panel side, so commands run in the user's interactive PTY where
//! sudo prompts handle themselves.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ssh::error::{Result, SshError};
use crate::ssh::SshSession;

/// Which firewall stack the host is running. Detection precedence is
/// firewalld → ufw → nftables → iptables; the first whose CLI exists
/// *and* whose ruleset/service is active wins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FirewallBackend {
    /// `firewalld` daemon (RHEL/Fedora default).
    Firewalld,
    /// Ubuntu/Debian `ufw` frontend over iptables/nftables.
    Ufw,
    /// Native `nftables` ruleset (non-empty `nft list ruleset`).
    Nftables,
    /// Legacy `iptables` (or `iptables-nft` shim — same CLI surface).
    Iptables,
    /// No detectable firewall stack on the host.
    #[default]
    None,
}

/// One row from `ss -tulnp` — a port the host is currently listening on.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ListeningPort {
    /// "tcp" / "udp" (we collapse v4/v6 into one tag and rely on
    /// `local_addr` showing `::` to distinguish the family).
    pub proto: String,
    /// Bind address as `ss` reports it (`0.0.0.0`, `::`, `127.0.0.1`, …).
    pub local_addr: String,
    /// TCP/UDP port number.
    pub local_port: u16,
    /// "LISTEN" / "UNCONN". UDP rows from `ss` typically read UNCONN
    /// because UDP has no listen state — kept for completeness, the UI
    /// renders both as "open".
    pub state: String,
    /// Best-effort process name from `ss -p`. Empty when the SSH user
    /// can't see the owning process (cross-uid without root).
    pub process: String,
    /// Owning PID from `ss -p`, when readable.
    pub pid: Option<u32>,
}

/// One row from `/proc/net/dev` — cumulative RX/TX byte counters for a
/// single network interface. Diff two snapshots to derive throughput.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InterfaceCounter {
    /// Interface name (e.g. `eth0`, `wlan0`, `lo`, `docker0`).
    pub iface: String,
    /// Cumulative bytes received since boot.
    pub rx_bytes: u64,
    /// Cumulative bytes transmitted since boot.
    pub tx_bytes: u64,
}

/// One full pass over the host's firewall + network state. Cheap enough
/// to call repeatedly (sub-second on most hosts); the panel polls it
/// every couple of seconds while the Traffic tab is visible so the
/// frontend can derive RX/TX rates by diffing two snapshots.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FirewallSnapshot {
    /// Detected firewall stack on the host.
    pub backend: FirewallBackend,
    /// True if the detected backend appears to be running. For
    /// firewalld/ufw this is the `is-active` check; for nftables it
    /// means the ruleset is non-empty; for iptables it's "the binary
    /// exists" (always active in some sense).
    pub backend_active: bool,
    /// True when the SSH user is root (uid == 0). Drives whether the
    /// frontend offers write actions inline or routes them to the
    /// terminal with a `sudo` prefix.
    pub root: bool,
    /// SSH login name (`whoami` on the remote host).
    pub user: String,
    /// `uname -srm` output from the remote host (kernel + arch banner).
    pub uname: String,
    /// All currently listening TCP/UDP ports.
    pub listening: Vec<ListeningPort>,
    /// Per-interface byte counters; pair two snapshots to derive rates.
    pub interfaces: Vec<InterfaceCounter>,
    /// Server-side wall clock at capture, ms since epoch. Pair two
    /// snapshots and divide by `(t1 - t0)` for byte/sec rates that are
    /// independent of network round-trip jitter between client and
    /// host.
    pub captured_at_ms: u64,
    /// Raw `iptables-save -c` for the filter + nat tables, joined.
    /// Empty when iptables wasn't readable (non-root + no setuid).
    pub rules_v4: String,
    /// Raw `ip6tables-save -c` (filter only — we don't push v6 NAT in
    /// the UI; rare and nftables-only on most setups).
    pub rules_v6: String,
    /// Raw `iptables-save -t nat -c` so the frontend can extract
    /// `-A DOCKER ...` lines for the Mappings tab without re-running.
    pub nat_v4: String,
    /// Default policies per filter chain ("INPUT" → "ACCEPT"). Useful
    /// to surface "default-deny vs default-accept" at a glance.
    pub default_policies: BTreeMap<String, String>,
    /// First non-empty line from the backend's status command (for
    /// firewalld: `firewall-cmd --state`; for ufw: the `Status:` line).
    /// Helps the panel show backend status without re-running.
    pub backend_status: String,
}

const PROBE: &str = "LC_ALL=C; export LC_ALL; \
    echo '---ID---'; id -u 2>/dev/null; \
    echo '---WHOAMI---'; whoami 2>/dev/null; \
    echo '---UNAME---'; uname -srm 2>/dev/null; \
    echo '---NOW---'; date +%s%3N 2>/dev/null; \
    echo '---HAS_FIREWALLD---'; ( command -v firewall-cmd >/dev/null 2>&1 && firewall-cmd --state 2>/dev/null ) || echo no; \
    echo '---HAS_UFW---'; ( command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | head -1 ) || echo no; \
    echo '---HAS_NFT---'; ( command -v nft >/dev/null 2>&1 && nft list ruleset 2>/dev/null | wc -l ) || echo 0; \
    echo '---HAS_IPT---'; ( command -v iptables >/dev/null 2>&1 && echo yes ) || echo no; \
    echo '---LISTEN---'; ss -H -tulnp 2>/dev/null; \
    echo '---NETDEV---'; cat /proc/net/dev 2>/dev/null; \
    echo '---IPT_FILTER---'; iptables-save -c -t filter 2>/dev/null; \
    echo '---IPT_NAT---'; iptables-save -c -t nat 2>/dev/null; \
    echo '---IPT6_FILTER---'; ip6tables-save -c -t filter 2>/dev/null";

/// Run the firewall probe and return one snapshot.
pub async fn snapshot(session: &SshSession) -> Result<FirewallSnapshot> {
    let (exit, stdout) = session.exec_command(PROBE).await?;
    if exit != 0 && stdout.is_empty() {
        return Err(SshError::InvalidConfig(format!(
            "firewall probe exited {exit} with empty output"
        )));
    }
    Ok(parse_snapshot(&stdout))
}

/// Synchronous wrapper around [`snapshot`] for callers outside an async
/// context (Tauri command threads, FFI bridges).
pub fn snapshot_blocking(session: &SshSession) -> Result<FirewallSnapshot> {
    crate::ssh::runtime::shared().block_on(snapshot(session))
}

fn parse_snapshot(stdout: &str) -> FirewallSnapshot {
    let mut snap = FirewallSnapshot::default();
    let sections = split_sections(stdout);

    if let Some(s) = sections.get("ID") {
        snap.root = s.lines().next().unwrap_or("").trim() == "0";
    }
    if let Some(s) = sections.get("WHOAMI") {
        snap.user = s.lines().next().unwrap_or("").trim().to_string();
    }
    if let Some(s) = sections.get("UNAME") {
        snap.uname = s.lines().next().unwrap_or("").trim().to_string();
    }
    if let Some(s) = sections.get("NOW") {
        snap.captured_at_ms = s
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .parse::<u64>()
            .unwrap_or(0);
    }
    let firewalld = sections
        .get("HAS_FIREWALLD")
        .map(|s| s.lines().next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    let ufw_first = sections
        .get("HAS_UFW")
        .map(|s| s.lines().next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    let nft_lines = sections
        .get("HAS_NFT")
        .and_then(|s| s.lines().next())
        .and_then(|line| line.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let has_iptables = sections
        .get("HAS_IPT")
        .map(|s| s.lines().next().unwrap_or("").trim() == "yes")
        .unwrap_or(false);

    // Detection precedence: firewalld first (RHEL/Fedora default), ufw
    // next (Debian/Ubuntu default), then nftables-active, then iptables
    // catch-all. "Active" means the daemon/ruleset is doing something —
    // `firewall-cmd --state` returns "running"; `ufw status` first line
    // is "Status: active"; nft is active when ruleset is non-empty.
    if firewalld == "running" {
        snap.backend = FirewallBackend::Firewalld;
        snap.backend_active = true;
        snap.backend_status = firewalld;
    } else if ufw_first.starts_with("Status: active") {
        snap.backend = FirewallBackend::Ufw;
        snap.backend_active = true;
        snap.backend_status = ufw_first;
    } else if ufw_first.starts_with("Status:") {
        // ufw installed but inactive — still inform the UI so it can
        // show "ufw is installed but inactive; use the iptables view".
        snap.backend = FirewallBackend::Iptables;
        snap.backend_active = has_iptables;
        snap.backend_status = ufw_first;
    } else if nft_lines > 0 {
        snap.backend = FirewallBackend::Nftables;
        snap.backend_active = true;
    } else if has_iptables {
        snap.backend = FirewallBackend::Iptables;
        snap.backend_active = true;
    } else {
        snap.backend = FirewallBackend::None;
        snap.backend_active = false;
    }

    if let Some(s) = sections.get("LISTEN") {
        snap.listening = parse_listening(s);
    }
    if let Some(s) = sections.get("NETDEV") {
        snap.interfaces = parse_netdev(s);
    }
    if let Some(s) = sections.get("IPT_FILTER") {
        snap.rules_v4 = s.trim().to_string();
        for (chain, policy) in extract_default_policies(s) {
            snap.default_policies.insert(chain, policy);
        }
    }
    if let Some(s) = sections.get("IPT_NAT") {
        snap.nat_v4 = s.trim().to_string();
        // Append nat to the rules dump so the frontend has one
        // combined v4 view per `iptables-save` semantics.
        if !snap.rules_v4.is_empty() && !snap.nat_v4.is_empty() {
            let combined = format!("{}\n{}", snap.rules_v4, snap.nat_v4);
            snap.rules_v4 = combined;
        }
    }
    if let Some(s) = sections.get("IPT6_FILTER") {
        snap.rules_v6 = s.trim().to_string();
    }

    snap
}

fn split_sections(stdout: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let mut current_key = String::new();
    let mut current_buf = String::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(key) = trimmed
            .strip_prefix("---")
            .and_then(|s| s.strip_suffix("---"))
        {
            if !current_key.is_empty() {
                out.insert(current_key.clone(), current_buf.clone());
            }
            current_key = key.to_string();
            current_buf.clear();
        } else if !current_key.is_empty() {
            if !current_buf.is_empty() {
                current_buf.push('\n');
            }
            current_buf.push_str(line);
        }
    }
    if !current_key.is_empty() {
        out.insert(current_key, current_buf);
    }
    out
}

/// Parse `ss -H -tulnp` rows. `-H` strips the header but per-distro
/// column counts vary — newer iproute2 includes `Process` as one
/// field, older ones break `users:(...)` onto its own column. We split
/// on whitespace and locate fields by content rather than position.
fn parse_listening(text: &str) -> Vec<ListeningPort> {
    let mut out = Vec::new();
    for line in text.lines() {
        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }
        let mut fields = raw.split_whitespace();
        // Recv-Q / Send-Q are present in all `ss` versions, so the
        // first token is the protocol family. Some ss builds prefix
        // with `Netid` column even with -H — accept both.
        let proto = fields.next().unwrap_or("").to_string();
        if proto.is_empty() {
            continue;
        }
        // skip state if present (`ss -H -tul` puts state second)
        let mut state = String::new();
        let mut local: Option<String> = None;
        let mut peer: Option<String> = None;
        let mut process_blob: Option<String> = None;
        let mut seen_numbers = 0u8; // recv-q / send-q
        for token in fields {
            if token.starts_with("users:") {
                process_blob = Some(token.to_string());
                continue;
            }
            // State tokens can appear before queue counters (newer ss
            // emits `tcp LISTEN 0 4096 …`) or after them; check state
            // first so it doesn't get misclassified as `local_addr`.
            if state.is_empty()
                && (token == "LISTEN"
                    || token == "UNCONN"
                    || token == "ESTAB"
                    || token == "CLOSE-WAIT"
                    || token == "TIME-WAIT")
            {
                state = token.to_string();
                continue;
            }
            if token.parse::<u32>().is_ok() && seen_numbers < 2 {
                seen_numbers += 1;
                continue;
            }
            if local.is_none() {
                local = Some(token.to_string());
                continue;
            }
            if peer.is_none() {
                peer = Some(token.to_string());
                continue;
            }
        }
        // Some `ss -H` builds emit state before the queue counters.
        // If we recognised a state token but it's empty, fall back to
        // the proto-derived default.
        if state.is_empty() {
            state = if proto.starts_with("tcp") {
                "LISTEN".into()
            } else {
                "UNCONN".into()
            };
        }
        let local_str = local.unwrap_or_default();
        let _ = peer;
        let (local_addr, local_port) = split_addr_port(&local_str);
        let (process, pid) = parse_users_blob(process_blob.as_deref().unwrap_or(""));
        out.push(ListeningPort {
            proto,
            local_addr,
            local_port,
            state,
            process,
            pid,
        });
    }
    out
}

/// `addr:port` or `[v6]:port` or `wildcard *:port`.
fn split_addr_port(s: &str) -> (String, u16) {
    if let Some(idx) = s.rfind(':') {
        let (a, p) = s.split_at(idx);
        let port = p.trim_start_matches(':');
        let port_n = port.parse::<u16>().unwrap_or(0);
        let addr = a.trim_matches(|c| c == '[' || c == ']');
        return (addr.to_string(), port_n);
    }
    (s.to_string(), 0)
}

/// `users:(("sshd",pid=948,fd=3))` → ("sshd", Some(948)). Only the
/// first process is reported even though the blob can carry several
/// (rare for listeners — fd inheritance via systemd-socket-activate
/// is the typical case where it happens).
fn parse_users_blob(blob: &str) -> (String, Option<u32>) {
    if blob.is_empty() {
        return (String::new(), None);
    }
    // strip leading `users:(` and trailing `)`
    let inner = blob
        .trim_start_matches("users:")
        .trim_start_matches('(')
        .trim_end_matches(')');
    // first record `("name",pid=N,fd=N)`
    let first = inner.split("),(").next().unwrap_or(inner);
    let first = first.trim_start_matches('(').trim_end_matches(')');
    let parts: Vec<&str> = first.split(',').collect();
    let name = parts
        .first()
        .map(|p| p.trim_matches('"').to_string())
        .unwrap_or_default();
    let pid = parts
        .iter()
        .find_map(|p| p.trim().strip_prefix("pid="))
        .and_then(|p| p.parse::<u32>().ok());
    (name, pid)
}

/// `/proc/net/dev` columns:
///     Inter-|   Receive                                                |  Transmit
///      face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop ...
/// Skips the two header lines and the loopback interface; loopback
/// counts inflate "current bandwidth" without telling the user
/// anything useful.
fn parse_netdev(text: &str) -> Vec<InterfaceCounter> {
    let mut out = Vec::new();
    for line in text.lines().skip(2) {
        let mut parts = line.split_whitespace();
        let name = match parts.next() {
            Some(s) => s.trim_end_matches(':'),
            None => continue,
        };
        if name == "lo" {
            continue;
        }
        let rx_bytes = parts
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        // skip rx_packets, rx_errs, rx_drop, rx_fifo, rx_frame, rx_compressed, rx_multicast (7 fields)
        for _ in 0..7 {
            parts.next();
        }
        let tx_bytes = parts
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        out.push(InterfaceCounter {
            iface: name.to_string(),
            rx_bytes,
            tx_bytes,
        });
    }
    out
}

/// Pull `:CHAIN POLICY [pkts:bytes]` lines out of an `iptables-save`
/// dump. The format is `:INPUT ACCEPT [123:456]` for built-in chains
/// or `:DOCKER -` for user chains (no policy). User chains are skipped.
fn extract_default_policies(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(':') {
            let mut parts = rest.split_whitespace();
            let chain = parts.next().unwrap_or("").to_string();
            let policy = parts.next().unwrap_or("").to_string();
            if chain.is_empty() || policy == "-" {
                continue;
            }
            out.push((chain, policy));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_addr_port_v4() {
        assert_eq!(split_addr_port("0.0.0.0:22"), ("0.0.0.0".into(), 22));
        assert_eq!(
            split_addr_port("127.0.0.1:5432"),
            ("127.0.0.1".into(), 5432)
        );
    }

    #[test]
    fn parses_addr_port_v6() {
        assert_eq!(split_addr_port("[::]:80"), ("::".into(), 80));
        assert_eq!(split_addr_port("[::1]:8080"), ("::1".into(), 8080));
    }

    #[test]
    fn parses_users_blob_simple() {
        assert_eq!(
            parse_users_blob("users:((\"sshd\",pid=948,fd=3))"),
            ("sshd".into(), Some(948))
        );
    }

    #[test]
    fn parses_users_blob_empty() {
        assert_eq!(parse_users_blob(""), ("".into(), None));
    }

    #[test]
    fn extracts_default_policies() {
        let dump = "*filter\n:INPUT ACCEPT [12:345]\n:FORWARD DROP [0:0]\n:OUTPUT ACCEPT [1:2]\n:DOCKER - [0:0]\nCOMMIT\n";
        let pols = extract_default_policies(dump);
        assert!(pols.iter().any(|(c, p)| c == "INPUT" && p == "ACCEPT"));
        assert!(pols.iter().any(|(c, p)| c == "FORWARD" && p == "DROP"));
        assert!(!pols.iter().any(|(c, _)| c == "DOCKER"));
    }

    #[test]
    fn parses_netdev_skips_lo() {
        let raw = "Inter-|   Receive                                                |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n    lo: 1234 56 0 0 0 0 0 0 5678 9 0 0 0 0 0 0\n  eth0: 1000 5 0 0 0 0 0 0 2000 7 0 0 0 0 0 0\n";
        let v = parse_netdev(raw);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].iface, "eth0");
        assert_eq!(v[0].rx_bytes, 1000);
        assert_eq!(v[0].tx_bytes, 2000);
    }

    #[test]
    fn parses_listening_basic() {
        let raw = "tcp   LISTEN 0      4096        0.0.0.0:22       0.0.0.0:*    users:((\"sshd\",pid=948,fd=3))\n";
        let v = parse_listening(raw);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].proto, "tcp");
        assert_eq!(v[0].local_port, 22);
        assert_eq!(v[0].process, "sshd");
        assert_eq!(v[0].pid, Some(948));
    }
}
