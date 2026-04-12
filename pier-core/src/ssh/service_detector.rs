//! Service detection over SSH.
//!
//! Probes a remote server for installed services — MySQL,
//! Redis, PostgreSQL, Docker — by running lightweight shell
//! commands through `SshSession::exec_command`. Each probe
//! checks `which`, extracts a version string from `--version`
//! output, and reports a running / stopped / installed status
//! based on a combination of `systemctl is-active`, `pgrep`,
//! and service-specific health commands like `redis-cli ping`
//! or `docker info`.
//!
//! ## Why this lives in pier-core
//!
//! This is the backbone of the "pier moment": after the SSH
//! handshake completes, pier-x runs `detect_all(session)` and
//! lights up the right-panel tabs for whichever services the
//! remote actually has. The detection logic is pure
//! shell-out-and-parse — no crates beyond what SshSession
//! already pulls in — so it drops into the Rust side of the
//! stack with no new dependencies.
//!
//! ## Upstream parity
//!
//! Ported from upstream Pier's `pier-core/src/ssh/service_detector.rs`
//! with minor refactors:
//!
//!  * Uses our unified `SshError` / `Result` types instead of
//!    `anyhow::Error`, so the UI layer gets typed errors.
//!  * Uses our `tokio::join!` on the shared runtime so all four
//!    probes run concurrently.
//!  * `parse_version` drops a dead `_tool` parameter.

use serde::{Deserialize, Serialize};

use super::session::SshSession;

/// Running state of a detected service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceStatus {
    /// The service binary is installed AND a health probe
    /// (systemctl / pgrep / service-specific ping) reports it
    /// as active.
    Running,
    /// The binary is installed but no health probe reported
    /// it as active.
    Stopped,
    /// The binary is installed but we couldn't determine its
    /// running state at all (e.g. `systemctl` not available
    /// and `pgrep` returned nothing meaningful).
    Installed,
}

/// A service we detected on a remote host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectedService {
    /// Short stable identifier. One of `mysql`, `redis`,
    /// `postgresql`, `docker` today; more land alongside
    /// new probes.
    pub name: String,
    /// Version string extracted from the tool's `--version`
    /// output. Free-form text on parse failure.
    pub version: String,
    /// Running state.
    pub status: ServiceStatus,
    /// Default TCP port the service listens on, used later by
    /// the tunnel manager to pick a local forward target.
    /// `0` for services that don't expose a network port
    /// (e.g. docker talks over a Unix socket).
    pub port: u16,
}

/// Detect every known service on the remote host.
///
/// Runs the individual detectors concurrently via
/// `tokio::join!` — a typical LAN detection of four services
/// finishes in well under a second because the latency-
/// dominant steps (TCP already established, one short SSH
/// exec per probe) can overlap.
///
/// Returns a `Vec` sorted in the order probes fired, which is
/// stable across runs so the UI doesn't shuffle tabs.
pub async fn detect_all(session: &SshSession) -> Vec<DetectedService> {
    let (mysql, redis, postgres, docker) = tokio::join!(
        detect_mysql(session),
        detect_redis(session),
        detect_postgresql(session),
        detect_docker(session),
    );

    let mut services = Vec::new();
    if let Some(s) = mysql {
        services.push(s);
    }
    if let Some(s) = redis {
        services.push(s);
    }
    if let Some(s) = postgres {
        services.push(s);
    }
    if let Some(s) = docker {
        services.push(s);
    }
    log::info!("detected {} services on remote host", services.len());
    services
}

/// Sync convenience for [`detect_all`].
pub fn detect_all_blocking(session: &SshSession) -> Vec<DetectedService> {
    super::runtime::shared().block_on(detect_all(session))
}

// ─────────────────────────────────────────────────────────
// Individual detectors
// ─────────────────────────────────────────────────────────

async fn detect_mysql(session: &SshSession) -> Option<DetectedService> {
    let (code, _) = session
        .exec_command("which mysql 2>/dev/null || which mysqld 2>/dev/null")
        .await
        .ok()?;
    if code != 0 {
        return None;
    }

    let (_, version_out) = session
        .exec_command("mysql --version 2>/dev/null")
        .await
        .unwrap_or((-1, String::new()));
    let version = parse_version(&version_out);

    let status = check_service_status(
        session,
        &[
            "systemctl is-active mysql 2>/dev/null || systemctl is-active mysqld 2>/dev/null || systemctl is-active mariadb 2>/dev/null",
            "pgrep -x mysqld >/dev/null 2>&1 && echo active",
        ],
    )
    .await;

    Some(DetectedService {
        name: "mysql".to_string(),
        version,
        status,
        port: 3306,
    })
}

async fn detect_redis(session: &SshSession) -> Option<DetectedService> {
    let (code, _) = session
        .exec_command("which redis-server 2>/dev/null || which redis-cli 2>/dev/null")
        .await
        .ok()?;
    if code != 0 {
        return None;
    }

    let (_, version_out) = session
        .exec_command("redis-cli --version 2>/dev/null")
        .await
        .unwrap_or((-1, String::new()));
    let version = parse_version(&version_out);

    // Try ping first — most direct health check.
    let (ping_code, ping_out) = session
        .exec_command("redis-cli ping 2>/dev/null")
        .await
        .unwrap_or((-1, String::new()));
    let status = if ping_code == 0 && ping_out.contains("PONG") {
        ServiceStatus::Running
    } else {
        check_service_status(
            session,
            &[
                "systemctl is-active redis 2>/dev/null || systemctl is-active redis-server 2>/dev/null",
                "pgrep -x redis-server >/dev/null 2>&1 && echo active",
            ],
        )
        .await
    };

    Some(DetectedService {
        name: "redis".to_string(),
        version,
        status,
        port: 6379,
    })
}

async fn detect_postgresql(session: &SshSession) -> Option<DetectedService> {
    let (code, _) = session.exec_command("which psql 2>/dev/null").await.ok()?;
    if code != 0 {
        return None;
    }

    let (_, version_out) = session
        .exec_command("psql --version 2>/dev/null")
        .await
        .unwrap_or((-1, String::new()));
    let version = parse_version(&version_out);

    let status = check_service_status(
        session,
        &[
            "systemctl is-active postgresql 2>/dev/null",
            "pgrep -x postgres >/dev/null 2>&1 && echo active",
        ],
    )
    .await;

    Some(DetectedService {
        name: "postgresql".to_string(),
        version,
        status,
        port: 5432,
    })
}

async fn detect_docker(session: &SshSession) -> Option<DetectedService> {
    let (code, _) = session
        .exec_command("which docker 2>/dev/null")
        .await
        .ok()?;
    if code != 0 {
        return None;
    }

    let (_, version_out) = session
        .exec_command("docker --version 2>/dev/null")
        .await
        .unwrap_or((-1, String::new()));
    let version = parse_version(&version_out);

    // `docker info` succeeds only if the daemon is running
    // and the user has permission to talk to it, which is the
    // health signal we want.
    let (info_code, _) = session
        .exec_command("docker info >/dev/null 2>&1")
        .await
        .unwrap_or((-1, String::new()));
    let status = if info_code == 0 {
        ServiceStatus::Running
    } else {
        check_service_status(session, &["systemctl is-active docker 2>/dev/null"]).await
    };

    Some(DetectedService {
        name: "docker".to_string(),
        version,
        status,
        port: 0, // Docker doesn't tunnel over a fixed TCP port.
    })
}

/// Walk `commands` until one reports `active` on stdout,
/// returning `Running`. If none do, return `Stopped` — we
/// already know the binary is installed (that's the caller's
/// precondition) so we never fall through to `Installed`.
///
/// The `Installed` variant exists for the case where a probe
/// finds the binary but has no way to ask if it's running
/// (e.g. a stripped-down container without systemctl or
/// pgrep). That's rare enough that we don't construct it here
/// — the enum variant is reserved for future probes that
/// explicitly detect this case.
async fn check_service_status(session: &SshSession, commands: &[&str]) -> ServiceStatus {
    for cmd in commands {
        if let Ok((code, output)) = session.exec_command(cmd).await {
            if code == 0 && output.contains("active") {
                return ServiceStatus::Running;
            }
        }
    }
    ServiceStatus::Stopped
}

/// Extract a version-looking token from a `--version`-style
/// output string.
///
/// The heuristic is "first whitespace-separated token that
/// starts with a digit and contains a `.`". That matches:
///
///  * `"mysql  Ver 8.0.35 Distrib 8.0.35, for Linux ..."` → `8.0.35`
///  * `"redis-cli 7.0.11"` → `7.0.11`
///  * `"psql (PostgreSQL) 15.4"` → `15.4`
///  * `"Docker version 24.0.5, build ced0996"` → `24.0.5`
///
/// On failure it returns the first line of the output or
/// `"unknown"` when the output is empty.
fn parse_version(output: &str) -> String {
    for word in output.split_whitespace() {
        let trimmed = word.trim_end_matches([',', ';', '.']);
        if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) && trimmed.contains('.') {
            return trimmed.to_string();
        }
    }
    output.lines().next().unwrap_or("unknown").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_mysql() {
        assert_eq!(
            parse_version("mysql  Ver 8.0.35 Distrib 8.0.35, for Linux on x86_64"),
            "8.0.35",
        );
    }

    #[test]
    fn parse_version_redis() {
        assert_eq!(parse_version("redis-cli 7.0.11"), "7.0.11");
    }

    #[test]
    fn parse_version_psql() {
        assert_eq!(parse_version("psql (PostgreSQL) 15.4"), "15.4");
    }

    #[test]
    fn parse_version_docker() {
        assert_eq!(
            parse_version("Docker version 24.0.5, build ced0996"),
            "24.0.5"
        );
    }

    #[test]
    fn parse_version_fallback_first_line() {
        // Output with no numeric tokens → fall back to first line.
        let out = "hello\nworld";
        assert_eq!(parse_version(out), "hello");
    }

    #[test]
    fn parse_version_empty_input() {
        assert_eq!(parse_version(""), "unknown");
    }

    #[test]
    fn detected_service_json_round_trip() {
        let s = DetectedService {
            name: "mysql".to_string(),
            version: "8.0.35".to_string(),
            status: ServiceStatus::Running,
            port: 3306,
        };
        let json = serde_json::to_string(&s).unwrap();
        // serde rename_all=lowercase pins the status tag.
        assert!(
            json.contains("\"status\":\"running\""),
            "status rename_all pin broken: {json}",
        );
        let back: DetectedService = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn service_status_serde_uses_lowercase_variants() {
        assert_eq!(
            serde_json::to_string(&ServiceStatus::Running).unwrap(),
            "\"running\"",
        );
        assert_eq!(
            serde_json::to_string(&ServiceStatus::Stopped).unwrap(),
            "\"stopped\"",
        );
        assert_eq!(
            serde_json::to_string(&ServiceStatus::Installed).unwrap(),
            "\"installed\"",
        );
    }
}
