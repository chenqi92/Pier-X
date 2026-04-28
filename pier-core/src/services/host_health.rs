//! Quick TCP-only reachability probe for the "Host Health" dashboard.
//!
//! ## Design
//!
//! The dashboard answers one question repeatedly across every saved
//! SSH connection: *is this host listening on its SSH port from
//! where I am right now?* That's a TCP `connect()` with a timeout —
//! nothing more.
//!
//! We deliberately do NOT do an SSH handshake here:
//!
//!  * Auth requires per-host paperwork (key files, agent, keyring).
//!    Replaying that on every refresh tick is paperwork the user
//!    didn't ask for.
//!  * `sshd` binds the listening port long before pre-auth banner
//!    exchange completes, so a TCP success is a strong "host is
//!    up, sshd is listening" signal even when auth would later
//!    reject a particular credential bundle.
//!  * The deeper view (uptime, distro, services) is one click away
//!    in a real tab — the dashboard is for triage, not deep dives.
//!
//! ## Concurrency
//!
//! Probes for many hosts run in parallel on the shared runtime.
//! `tokio::net::TcpStream::connect` yields cooperatively so even a
//! 100-host batch only consumes a handful of worker threads.

use std::io::ErrorKind;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::ssh::runtime;

/// One row of the dashboard. Returned in the same order as the
/// `indices` the caller passed in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostHealthReport {
    /// Echoes the saved-connection list index the caller asked
    /// about. Lets the frontend correlate against the
    /// `SavedSshConnection` it already has cached without a
    /// second lookup.
    pub saved_connection_index: usize,
    /// Coarse outcome — see [`HealthStatus`] for the full taxonomy.
    /// Serialised lower-case (`"online"`, `"offline"`, `"timeout"`,
    /// `"error"`) so the frontend can switch on it directly.
    pub status: HealthStatus,
    /// Round-trip latency of the TCP handshake, in milliseconds.
    /// Only meaningful when `status == Online`. `None` otherwise.
    pub latency_ms: Option<u64>,
    /// Best-effort short error string for the non-online cases.
    /// Empty when nothing useful to report.
    pub error_message: String,
    /// Unix epoch seconds at the moment the probe completed.
    pub checked_at: u64,
}

/// Coarse outcome of a single TCP probe. The dashboard maps each
/// variant onto a colored status dot — `Online` green, `Offline` /
/// `Timeout` red, `Error` neutral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// TCP handshake succeeded inside the timeout — host is up.
    Online,
    /// TCP refused / aborted / no route. The host or sshd is down,
    /// distinct from "didn't respond at all" which is `Timeout`.
    Offline,
    /// No response (or partial response) within the timeout.
    Timeout,
    /// Anything else — DNS resolution failure, permission denied,
    /// invalid configuration, etc. `error_message` carries detail.
    Error,
}

/// One target the caller wants checked. The probe needs only the
/// network endpoint — credentials are intentionally absent because
/// we never authenticate during a health probe.
#[derive(Debug, Clone)]
pub struct HostHealthTarget {
    /// Saved-connection index the report should echo back.
    pub saved_connection_index: usize,
    /// Hostname or IP. Trimmed before resolution; empty hosts are
    /// reported as `Error` rather than panicking.
    pub host: String,
    /// TCP port. Zero is treated as 22 (the SSH default).
    pub port: u16,
}

/// Probe `targets` in parallel and return one report per entry.
///
/// `timeout` is clamped to a sane window (200 ms ↔ 30 s) inside the
/// blocking wrapper — callers shouldn't pass anything tighter
/// because TLS-rejecting middleboxes routinely take 1-2 seconds to
/// drop a connection, and shouldn't pass anything looser because
/// the dashboard is supposed to be snappy.
pub async fn probe_many(
    targets: Vec<HostHealthTarget>,
    timeout: Duration,
) -> Vec<HostHealthReport> {
    let mut tasks = Vec::with_capacity(targets.len());
    for t in targets.into_iter() {
        tasks.push(tokio::spawn(probe_one(t, timeout)));
    }

    let mut out = Vec::with_capacity(tasks.len());
    for task in tasks {
        match task.await {
            Ok(r) => out.push(r),
            Err(e) => out.push(HostHealthReport {
                saved_connection_index: usize::MAX,
                status: HealthStatus::Error,
                latency_ms: None,
                error_message: format!("probe task panicked: {e}"),
                checked_at: now_epoch_secs(),
            }),
        }
    }
    out
}

/// Synchronous wrapper around [`probe_many`] that hops onto
/// [`runtime::shared`] and applies the soft clamp on `timeout`.
/// Intended for the Tauri command layer which is sync-shaped.
pub fn probe_many_blocking(
    targets: Vec<HostHealthTarget>,
    timeout_ms: u32,
) -> Vec<HostHealthReport> {
    let clamped = Duration::from_millis(timeout_ms.clamp(200, 30_000) as u64);
    runtime::shared().block_on(probe_many(targets, clamped))
}

async fn probe_one(target: HostHealthTarget, timeout: Duration) -> HostHealthReport {
    let checked_at = now_epoch_secs();
    let host = target.host.trim();
    if host.is_empty() {
        return HostHealthReport {
            saved_connection_index: target.saved_connection_index,
            status: HealthStatus::Error,
            latency_ms: None,
            error_message: "host is empty".to_string(),
            checked_at,
        };
    }
    let port = if target.port == 0 { 22 } else { target.port };
    let addr = format!("{host}:{port}");
    let started = Instant::now();
    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&addr)).await {
        Ok(Ok(_stream)) => HostHealthReport {
            saved_connection_index: target.saved_connection_index,
            status: HealthStatus::Online,
            latency_ms: Some(started.elapsed().as_millis() as u64),
            error_message: String::new(),
            checked_at,
        },
        Ok(Err(e)) => {
            // The error kind is the most stable cross-platform
            // signal — Windows localises the messages themselves.
            let status = match e.kind() {
                ErrorKind::ConnectionRefused
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted => HealthStatus::Offline,
                ErrorKind::TimedOut => HealthStatus::Timeout,
                ErrorKind::HostUnreachable
                | ErrorKind::NetworkUnreachable
                | ErrorKind::AddrNotAvailable => HealthStatus::Offline,
                _ => HealthStatus::Error,
            };
            HostHealthReport {
                saved_connection_index: target.saved_connection_index,
                status,
                latency_ms: None,
                error_message: e.to_string(),
                checked_at,
            }
        }
        Err(_) => HostHealthReport {
            saved_connection_index: target.saved_connection_index,
            status: HealthStatus::Timeout,
            latency_ms: None,
            error_message: format!("no response within {} ms", timeout.as_millis()),
            checked_at,
        },
    }
}

fn now_epoch_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── Deep probe ──────────────────────────────────────────────────
//
// Opt-in companion to the TCP-only probe. The user clicks a per-row
// "Deep probe" button; the command layer hands an existing
// `SshSession` (only ever a cached session — we never authenticate
// during a deep probe) and we run two cheap one-shot commands:
//
//   * `uptime` — load averages and human-readable uptime
//   * `df -hP /` — root filesystem usage
//
// Both outputs are captured verbatim and parsed defensively. Lines
// we can't parse just leave the corresponding field as `None` so a
// rare distro shape doesn't take down the whole row.

/// Result of a deep probe run. Populated fields are best-effort —
/// any parser miss yields `None`, never an error.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDeepProbeReport {
    /// Echoes the saved-connection index the caller asked about.
    pub saved_connection_index: usize,
    /// `uptime`'s "up X days, Y:Z" portion. None when parsing
    /// failed or the command produced empty output.
    pub uptime: Option<String>,
    /// `uptime`'s `load average:` triplet rendered as a single
    /// comma-separated string, e.g. `"0.12, 0.34, 0.45"`.
    pub load_avg: Option<String>,
    /// Root filesystem use percentage from `df -hP /`. e.g. `"78%"`.
    pub disk_root_use: Option<String>,
    /// Root filesystem available space, e.g. `"12G"`.
    pub disk_root_avail: Option<String>,
    /// `/etc/os-release` PRETTY_NAME when readable.
    pub distro: Option<String>,
    /// Unix epoch seconds when the probe finished.
    pub checked_at: u64,
}

/// Run a deep probe over an existing session. Async; the command
/// layer wraps it in `spawn_blocking` like other deep operations.
pub async fn deep_probe(
    saved_connection_index: usize,
    session: &crate::ssh::SshSession,
) -> HostDeepProbeReport {
    let mut report = HostDeepProbeReport {
        saved_connection_index,
        checked_at: now_epoch_secs(),
        ..Default::default()
    };

    // `uptime` is universally available. We use it instead of
    // poking `/proc/uptime` so macOS-targeted SSH sessions work too.
    if let Ok((_code, out)) = session.exec_command("uptime 2>/dev/null").await {
        let (up, la) = parse_uptime(&out);
        report.uptime = up;
        report.load_avg = la;
    }

    // `df -hP /` keeps a stable column shape across BSD/Linux. The
    // `-P` flag forces POSIX one-line-per-fs output even when the
    // mountpoint hangs off a long device path.
    if let Ok((_code, out)) = session.exec_command("df -hP / 2>/dev/null").await {
        let (use_pct, avail) = parse_df_root(&out);
        report.disk_root_use = use_pct;
        report.disk_root_avail = avail;
    }

    // PRETTY_NAME is the user-facing distro string ("Ubuntu 22.04.4
    // LTS"). Falls back to `ID=` when PRETTY_NAME is missing
    // (Alpine, some embedded distros).
    if let Ok((_code, out)) = session
        .exec_command(". /etc/os-release 2>/dev/null && echo \"$PRETTY_NAME\"; true")
        .await
    {
        let trimmed = out.trim();
        if !trimmed.is_empty() {
            report.distro = Some(trimmed.to_string());
        }
    }

    report
}

/// Parse a single `uptime` output line. Returns `(uptime, load_avg)`.
/// Both `None` when the line doesn't match the expected shape.
///
/// The canonical line is:
/// ` 12:34:56 up 5 days,  3:42,  2 users,  load average: 0.12, 0.34, 0.45`
/// We anchor on the `load average:` and ` up ` substrings because
/// the spacing in between is GNU-coreutils-vs-busybox-flavoured
/// (one or two spaces, sometimes tabs); using `find` instead of a
/// fixed split string lets us tolerate either flavour.
fn parse_uptime(s: &str) -> (Option<String>, Option<String>) {
    let line = s.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return (None, None);
    }
    let after_up = line.split(" up ").nth(1).unwrap_or("");
    let (pre, post) = match after_up.find("load average") {
        Some(i) => {
            // Trim trailing whitespace + comma off the head.
            let head = after_up[..i]
                .trim_end_matches(|c: char| c.is_whitespace() || c == ',');
            // Skip "load average" and any colon/whitespace after.
            let tail = after_up[i + "load average".len()..]
                .trim_start_matches(|c: char| c == ':' || c.is_whitespace());
            (head.to_string(), Some(tail.to_string()))
        }
        None => (after_up.trim().to_string(), None),
    };
    // `pre` looks like `5 days,  3:42,  2 users` — strip the
    // trailing "X users"/"X user" segment by chopping at the last
    // comma. When pre has no comma at all (e.g. `47 min` after
    // GNU's odd "X user" lines that lack the days+HH:MM segment)
    // there's nothing to chop and the whole thing is the uptime.
    let uptime = match pre.rsplit_once(',') {
        Some((head, tail)) if tail.contains("user") => {
            Some(head.trim().to_string())
        }
        _ => Some(pre.trim().to_string()),
    }
    .filter(|s| !s.is_empty());
    (uptime, post)
}

/// Parse `df -hP /` output. Returns `(use_pct, avail)`. The first
/// non-header line carries the values; columns are
/// `Filesystem Size Used Avail Use% Mounted on`.
fn parse_df_root(s: &str) -> (Option<String>, Option<String>) {
    let mut lines = s.lines().filter(|l| !l.trim().is_empty());
    // Skip the header.
    let _ = lines.next();
    let row = match lines.next() {
        Some(r) => r,
        None => return (None, None),
    };
    let cols: Vec<&str> = row.split_whitespace().collect();
    // Columns: Filesystem Size Used Avail Use% Mounted-on
    let avail = cols.get(3).map(|s| s.to_string());
    let use_pct = cols.get(4).map(|s| s.to_string());
    (use_pct, avail)
}

#[cfg(test)]
mod deep_probe_tests {
    use super::*;

    #[test]
    fn parse_uptime_handles_canonical_line() {
        let s =
            " 12:34:56 up 5 days,  3:42,  2 users,  load average: 0.12, 0.34, 0.45\n";
        let (up, la) = parse_uptime(s);
        assert_eq!(up.as_deref(), Some("5 days,  3:42"));
        assert_eq!(la.as_deref(), Some("0.12, 0.34, 0.45"));
    }

    #[test]
    fn parse_uptime_handles_short_uptime() {
        // Hosts that just booted: "12:00:01 up 47 min, ..."
        let s = " 12:00:01 up 47 min,  1 user,  load average: 0.04, 0.12, 0.10\n";
        let (up, la) = parse_uptime(s);
        assert_eq!(up.as_deref(), Some("47 min"));
        assert!(la.unwrap().contains("0.04"));
    }

    #[test]
    fn parse_uptime_handles_empty() {
        assert_eq!(parse_uptime(""), (None, None));
        assert_eq!(parse_uptime("   \n"), (None, None));
    }

    #[test]
    fn parse_df_root_extracts_avail_and_use() {
        let s = "Filesystem      Size  Used Avail Use% Mounted on\n\
                 /dev/sda1        50G   38G   12G  78% /\n";
        let (use_pct, avail) = parse_df_root(s);
        assert_eq!(use_pct.as_deref(), Some("78%"));
        assert_eq!(avail.as_deref(), Some("12G"));
    }

    #[test]
    fn parse_df_root_returns_none_on_empty() {
        assert_eq!(parse_df_root(""), (None, None));
        assert_eq!(parse_df_root("Filesystem ...\n"), (None, None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn probe_online_when_listener_is_bound() {
        // Bind a real listener on an ephemeral port so the test
        // works on every CI box without privileged ports / known
        // services. Drop it after the probe — the OS will release
        // the port for reuse.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        let port = listener.local_addr().unwrap().port();

        let reports = probe_many_blocking(
            vec![HostHealthTarget {
                saved_connection_index: 7,
                host: "127.0.0.1".to_string(),
                port,
            }],
            1500,
        );

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].saved_connection_index, 7);
        assert_eq!(reports[0].status, HealthStatus::Online);
        assert!(reports[0].latency_ms.is_some());
        drop(listener);
    }

    #[test]
    fn probe_not_online_for_unbound_port() {
        // Bind + immediately drop so the OS reclaims the port; then
        // probe it. Most kernels surface this as ConnectionRefused
        // (→ `Offline`) but Windows can occasionally let the SYN
        // get swallowed, surfacing as a timeout. Both outcomes
        // satisfy the dashboard's invariant: "not online means no
        // latency". We don't pin a specific kind to keep the test
        // stable across CI runners.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let reports = probe_many_blocking(
            vec![HostHealthTarget {
                saved_connection_index: 0,
                host: "127.0.0.1".to_string(),
                port,
            }],
            1500,
        );
        assert_ne!(reports[0].status, HealthStatus::Online);
        assert!(reports[0].latency_ms.is_none());
    }

    #[test]
    fn probe_error_for_empty_host() {
        let reports = probe_many_blocking(
            vec![HostHealthTarget {
                saved_connection_index: 3,
                host: "  ".to_string(),
                port: 22,
            }],
            1500,
        );
        assert_eq!(reports[0].status, HealthStatus::Error);
        assert_eq!(reports[0].saved_connection_index, 3);
    }
}
