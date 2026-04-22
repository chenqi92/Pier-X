//! Database instance detection over SSH.
//!
//! Where [`super::service_detector`] answers "does this host
//! have MySQL installed at all?" with a boolean + version,
//! `db_detect` answers "what concrete DB instances can I reach
//! on this host right now, and on which port?". It emits
//! structured [`DetectedDbInstance`] rows that the right-side
//! DB panels pre-fill connection forms from.
//!
//! ## Strategy
//!
//! Three probes run concurrently via `tokio::join!`:
//!
//! 1. **Docker inventory** — `docker ps` parsed via
//!    [`crate::services::docker::list_containers`]. Images are
//!    classified against a small known-DB list. Host-bound
//!    ports (`0.0.0.0:3307->3306/tcp`) become reachable
//!    instances; internal-only containers are skipped.
//! 2. **Listening sockets** — `ss -tlnp` (with `netstat -tlnp`
//!    as fallback). Any listener on a DB default port
//!    (3306/3307, 5432/5433, 6379/6380, …) that wasn't already
//!    claimed by a docker container becomes a `direct` / `systemd`
//!    entry.
//! 3. **CLI capability** — `command -v mysql psql redis-cli sqlite3`
//!    returns which user-space clients exist; useful UX hint
//!    ("no sqlite3 on remote — downloads will use a local
//!    copy").
//!
//! ## Dedup
//!
//! Keyed by `(bind_host, host_port)`. Docker rows win over
//! `ss` rows on collision because their metadata (image name,
//! container id) is richer. A socket owned by `docker-proxy`
//! is always classified as the docker side, even if the
//! matching container row happens to be missing.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::session::SshSession;
use crate::services::docker;

/// Where a detected DB instance lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionSource {
    /// Managed by systemd / init — mysqld-style native install.
    Systemd,
    /// Running inside a Docker container on this host.
    Docker,
    /// Listening but we couldn't attribute it to systemd or
    /// docker (rare — standalone foreground process or unknown
    /// init system).
    Direct,
}

/// Which DB panel this instance maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetectedDbKind {
    /// MySQL / MariaDB / Percona.
    Mysql,
    /// PostgreSQL / TimescaleDB / Citus.
    Postgres,
    /// Redis / Valkey / KeyDB.
    Redis,
}

/// Optional metadata about a detection. Every field is
/// informational — the UI can show them, but the tunnel only
/// needs `(host, port)`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectedDbMetadata {
    /// Docker image ref when `source == Docker`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Docker container id (short) when `source == Docker`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    /// Version, if we could parse one cheaply (not always).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// PID of the listening process (from `ss -p`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// `comm` name for the PID, e.g. `mysqld`, `docker-proxy`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
}

/// One reachable database instance found on the remote host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectedDbInstance {
    /// How we found it.
    pub source: DetectionSource,
    /// Which panel it belongs to.
    pub kind: DetectedDbKind,
    /// Host to dial from the remote side — usually `127.0.0.1`
    /// because the tunnel forwards a remote-loopback port.
    pub host: String,
    /// Port on the remote side.
    pub port: u16,
    /// User-facing label (container name or `comm` fallback).
    pub label: String,
    /// Informational extras.
    #[serde(default)]
    pub metadata: DetectedDbMetadata,
    /// Stable dedupe key used by the UI to correlate a detection
    /// result with a saved credential (`source == Detected`).
    pub signature: String,
}

/// Which CLIs are available on the remote host. Used to decide
/// whether SQLite remote mode can use `sqlite3` over exec or
/// must fall back to SFTP-download.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteClis {
    /// `mysql` / `mariadb` CLI is on PATH.
    pub mysql: bool,
    /// `psql` CLI is on PATH.
    pub psql: bool,
    /// `redis-cli` is on PATH.
    pub redis_cli: bool,
    /// `sqlite3` CLI is on PATH.
    pub sqlite3: bool,
}

/// Full detection report for one host.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DbDetectionReport {
    /// Reachable DB instances, sorted stably (kind → source → port).
    pub instances: Vec<DetectedDbInstance>,
    /// Which DB client CLIs are installed on the remote host.
    pub clis: RemoteClis,
}

// ─────────────────────────────────────────────────────────
// Public entry points
// ─────────────────────────────────────────────────────────

/// Run the full detection pipeline.
pub async fn detect(session: &SshSession) -> DbDetectionReport {
    let (docker_probe, listen_probe, clis_probe) = tokio::join!(
        probe_docker(session),
        probe_listening_sockets(session),
        probe_clis(session),
    );

    let mut map: HashMap<(String, u16), DetectedDbInstance> = HashMap::new();

    // Docker first — its metadata is richer.
    for inst in docker_probe {
        map.insert((inst.host.clone(), inst.port), inst);
    }
    // Fill in `ss` results that don't collide with a docker entry.
    for inst in listen_probe {
        map.entry((inst.host.clone(), inst.port)).or_insert(inst);
    }

    let mut instances: Vec<DetectedDbInstance> = map.into_values().collect();
    instances.sort_by(|a, b| {
        let k = |i: &DetectedDbInstance| {
            (
                kind_sort_key(i.kind),
                source_sort_key(i.source),
                i.port,
                i.host.clone(),
            )
        };
        k(a).cmp(&k(b))
    });

    DbDetectionReport {
        instances,
        clis: clis_probe,
    }
}

/// Sync convenience for [`detect`].
pub fn detect_blocking(session: &SshSession) -> DbDetectionReport {
    super::runtime::shared().block_on(detect(session))
}

// ─────────────────────────────────────────────────────────
// Individual probes
// ─────────────────────────────────────────────────────────

async fn probe_docker(session: &SshSession) -> Vec<DetectedDbInstance> {
    // Skip entirely if docker isn't present — saves a failed
    // exec round-trip on non-docker hosts.
    let docker_here = session
        .exec_command("command -v docker >/dev/null 2>&1 && echo yes")
        .await
        .map(|(code, out)| code == 0 && out.trim() == "yes")
        .unwrap_or(false);
    if !docker_here {
        return Vec::new();
    }

    let containers = match docker::list_containers(session, false).await {
        Ok(c) => c,
        Err(e) => {
            log::debug!("db_detect: docker list_containers failed: {e}");
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for c in containers {
        let Some(kind) = classify_image(&c.image) else {
            continue;
        };
        for mapping in parse_port_mappings(&c.ports) {
            if mapping.container_port != default_port(kind) {
                // If the container exposes an unusual internal port,
                // still accept it — some deployments run mysql on
                // 3307 internally. The host_port is always what we
                // dial, so we never mis-tunnel.
            }
            let host = normalize_bind_host(&mapping.host);
            let label = container_label(&c.names, &c.image);
            let signature = format!(
                "docker://{}/{}:{}",
                short_container_id(&c.id),
                host,
                mapping.host_port,
            );
            out.push(DetectedDbInstance {
                source: DetectionSource::Docker,
                kind: to_cred_kind(kind),
                host,
                port: mapping.host_port,
                label,
                metadata: DetectedDbMetadata {
                    image: Some(c.image.clone()),
                    container_id: Some(short_container_id(&c.id)),
                    ..Default::default()
                },
                signature,
            });
        }
    }
    out
}

async fn probe_listening_sockets(session: &SshSession) -> Vec<DetectedDbInstance> {
    // `ss -tlnp` is the modern path. `netstat -tlnp` is the
    // fallback for old distros. `2>/dev/null` swallows
    // permission warnings from `ss -p` when run as a non-root
    // user — the process column will just be empty, which is
    // fine; we still get host+port.
    let cmd = "(ss -tlnp 2>/dev/null; netstat -tlnp 2>/dev/null) | head -n 500";
    let stdout = match session.exec_command(cmd).await {
        Ok((_, s)) => s,
        Err(e) => {
            log::debug!("db_detect: ss/netstat failed: {e}");
            return Vec::new();
        }
    };
    parse_listen_lines(&stdout)
}

async fn probe_clis(session: &SshSession) -> RemoteClis {
    // Single shell run returning a bitmap string, so we don't
    // pay four round-trips. `command -v` is POSIX-guaranteed
    // where `which` is not.
    let cmd = "\
        for b in mysql psql redis-cli sqlite3; do \
            if command -v $b >/dev/null 2>&1; then echo \"$b:1\"; else echo \"$b:0\"; fi; \
        done";
    let mut out = RemoteClis::default();
    let Ok((_, stdout)) = session.exec_command(cmd).await else {
        return out;
    };
    for line in stdout.lines() {
        let Some((name, flag)) = line.split_once(':') else {
            continue;
        };
        let present = flag.trim() == "1";
        match name.trim() {
            "mysql" => out.mysql = present,
            "psql" => out.psql = present,
            "redis-cli" => out.redis_cli = present,
            "sqlite3" => out.sqlite3 = present,
            _ => {}
        }
    }
    out
}

// ─────────────────────────────────────────────────────────
// Parsing helpers — public for unit testing
// ─────────────────────────────────────────────────────────

/// Image kind after lookup. Kept separate from
/// [`DetectedDbKind`] so we can add internal-only variants
/// without breaking the public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageKind {
    Mysql,
    Postgres,
    Redis,
}

fn to_cred_kind(k: ImageKind) -> DetectedDbKind {
    match k {
        ImageKind::Mysql => DetectedDbKind::Mysql,
        ImageKind::Postgres => DetectedDbKind::Postgres,
        ImageKind::Redis => DetectedDbKind::Redis,
    }
}

fn default_port(k: ImageKind) -> u16 {
    match k {
        ImageKind::Mysql => 3306,
        ImageKind::Postgres => 5432,
        ImageKind::Redis => 6379,
    }
}

/// Classify a docker image reference. Returns `None` for
/// non-DB images. Strips registry + namespace prefixes so
/// `docker.io/library/mysql:8` and `bitnami/postgresql:16`
/// both resolve.
fn classify_image(image: &str) -> Option<ImageKind> {
    let image = image.trim();
    if image.is_empty() {
        return None;
    }
    // Take the final path component (registry/namespace/name) …
    let tail = image.rsplit('/').next().unwrap_or(image);
    // … drop the :tag / @digest suffix …
    let name = tail
        .split_once([':', '@'])
        .map(|(n, _)| n)
        .unwrap_or(tail)
        .to_ascii_lowercase();

    match name.as_str() {
        "mysql" | "mariadb" | "percona" | "percona-server" | "percona-mysql" => Some(ImageKind::Mysql),
        "postgres" | "postgresql" | "timescaledb" | "citus" => Some(ImageKind::Postgres),
        "redis" | "redis-stack" | "redis-stack-server" | "valkey" | "keydb" => Some(ImageKind::Redis),
        _ => None,
    }
}

/// One entry out of the `Ports` column of `docker ps`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortMapping {
    /// Host bind address (e.g. `0.0.0.0`, `::`, `127.0.0.1`).
    pub host: String,
    /// Port on the host.
    pub host_port: u16,
    /// Port inside the container.
    pub container_port: u16,
    /// `tcp` / `udp`. Only `tcp` is emitted by this module.
    pub proto: String,
}

/// Parse Docker's `Ports` column, keeping only host-bound TCP
/// entries. Input looks like
/// `"0.0.0.0:3307->3306/tcp, :::3307->3306/tcp, 5432/tcp"`.
pub fn parse_port_mappings(ports: &str) -> Vec<PortMapping> {
    let mut out = Vec::new();
    let mut seen: HashSet<(String, u16)> = HashSet::new();
    for raw in ports.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let Some((left, right)) = raw.split_once("->") else {
            // No `->` → not a host-bound mapping.
            continue;
        };
        // left = "0.0.0.0:3307" or "[::]:3307"
        // right = "3306/tcp"
        let (proto_ok, container_port) = match right.split_once('/') {
            Some((cp, proto)) => (proto.eq_ignore_ascii_case("tcp"), cp.parse::<u16>().ok()),
            None => (true, right.parse::<u16>().ok()),
        };
        if !proto_ok {
            continue;
        }
        let Some(container_port) = container_port else {
            continue;
        };
        let (raw_host, host_port) = match split_host_port(left) {
            Some(v) => v,
            None => continue,
        };
        // Fold `0.0.0.0` and `::` to a single canonical key so
        // the IPv4+IPv6 dual-bind dockerd emits dedupes.
        let host = normalize_bind_host(&raw_host);
        let key = (host.clone(), host_port);
        if seen.insert(key) {
            out.push(PortMapping {
                host,
                host_port,
                container_port,
                proto: "tcp".into(),
            });
        }
    }
    out
}

/// Parse `"[::]:3307"` / `"0.0.0.0:3307"` / `"127.0.0.1:3307"`
/// into `(host, port)`.
fn split_host_port(s: &str) -> Option<(String, u16)> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('[') {
        // `[::]:3307` — IPv6 bracket form.
        let (host, tail) = rest.split_once(']')?;
        let port = tail.strip_prefix(':')?.parse::<u16>().ok()?;
        return Some((host.to_string(), port));
    }
    let (host, port) = s.rsplit_once(':')?;
    Some((host.to_string(), port.parse::<u16>().ok()?))
}

/// Fold bind addresses so "all interfaces" variants dedupe.
/// `0.0.0.0`, `::`, `[::]` → `0.0.0.0`. Loopback stays as-is.
fn normalize_bind_host(host: &str) -> String {
    match host.trim() {
        "" | "*" | "0.0.0.0" | "::" | "[::]" => "0.0.0.0".to_string(),
        other => other.to_string(),
    }
}

fn short_container_id(id: &str) -> String {
    if id.len() > 12 {
        id[..12].to_string()
    } else {
        id.to_string()
    }
}

fn container_label(names: &str, image: &str) -> String {
    let name = names.trim_start_matches('/');
    if !name.is_empty() {
        name.split(',').next().unwrap_or(name).to_string()
    } else {
        image.to_string()
    }
}

fn kind_sort_key(k: DetectedDbKind) -> u8 {
    match k {
        DetectedDbKind::Mysql => 0,
        DetectedDbKind::Postgres => 1,
        DetectedDbKind::Redis => 2,
    }
}

fn source_sort_key(s: DetectionSource) -> u8 {
    match s {
        DetectionSource::Docker => 0,
        DetectionSource::Systemd => 1,
        DetectionSource::Direct => 2,
    }
}

/// Parse `ss -tlnp` / `netstat -tlnp` output into DB instance
/// rows. Recognizes any LISTEN line whose local port matches
/// a known DB default (extended with common alternates).
pub fn parse_listen_lines(stdout: &str) -> Vec<DetectedDbInstance> {
    let mut out = Vec::new();
    let mut seen: HashSet<(String, u16)> = HashSet::new();

    for line in stdout.lines() {
        let line = line.trim();
        // Skip headers / non-LISTEN rows. Both tools include
        // the literal "LISTEN" on the relevant row.
        if !line.contains("LISTEN") {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        // `ss -tlnp`: Netid State Recv-Q Send-Q Local Peer [Process]
        // `netstat -tlnp`: Proto Recv-Q Send-Q Local Foreign State [PID/Program]
        // Find the first field that parses as host:port — that's the local bind.
        let Some(local) = fields.iter().find_map(|f| split_host_port(f)) else {
            continue;
        };
        let (host, port) = local;
        let Some(kind) = classify_port(port) else {
            continue;
        };
        let host = normalize_bind_host(&host);
        if host == "127.0.0.1" || host == "::1" {
            // Loopback-only — reachable via tunnel but we still
            // record it.
        }
        if !seen.insert((host.clone(), port)) {
            continue;
        }

        // Extract `comm` + pid from the trailing `users:(("comm",pid=123,fd=4))`
        // (ss) or `1234/mysqld` (netstat) blob.
        let (process_name, pid) = extract_process_info(line);
        let is_docker_proxy = process_name.as_deref() == Some("docker-proxy");
        let source = if is_docker_proxy {
            DetectionSource::Docker
        } else if process_name.is_some() {
            DetectionSource::Systemd
        } else {
            DetectionSource::Direct
        };
        let label = process_name
            .clone()
            .unwrap_or_else(|| format!("{host}:{port}"));
        let signature = format!("listen://{host}:{port}");

        out.push(DetectedDbInstance {
            source,
            kind,
            host,
            port,
            label,
            metadata: DetectedDbMetadata {
                pid,
                process_name,
                ..Default::default()
            },
            signature,
        });
    }
    out
}

fn classify_port(port: u16) -> Option<DetectedDbKind> {
    match port {
        3306 | 3307 | 3308 | 13306 => Some(DetectedDbKind::Mysql),
        5432 | 5433 | 5434 | 15432 => Some(DetectedDbKind::Postgres),
        6379 | 6380 | 6381 | 16379 => Some(DetectedDbKind::Redis),
        _ => None,
    }
}

/// Pull `(process_name, pid)` out of a listen line.
/// Handles the two common shapes:
///   * `ss`: `users:(("mysqld",pid=1234,fd=20))`
///   * `netstat`: `1234/mysqld`
fn extract_process_info(line: &str) -> (Option<String>, Option<u32>) {
    // ss shape first — the inner literal is more specific.
    if let Some(users_idx) = line.find("users:((") {
        let tail = &line[users_idx + "users:((".len()..];
        // `"mysqld",pid=1234,fd=20))`
        if let Some(stripped) = tail.strip_prefix('"') {
            if let Some(end_quote) = stripped.find('"') {
                let name = stripped[..end_quote].to_string();
                let after = &stripped[end_quote + 1..];
                let pid = after
                    .split(',')
                    .find_map(|p| p.trim().strip_prefix("pid="))
                    .and_then(|s| s.parse::<u32>().ok());
                return (Some(name), pid);
            }
        }
    }
    // netstat shape. The program column is the LAST whitespace
    // field; `-` when we lack permission.
    if let Some(last) = line.split_whitespace().last() {
        if last != "-" {
            if let Some((pid_str, name)) = last.split_once('/') {
                if let Ok(pid) = pid_str.parse::<u32>() {
                    return (Some(name.to_string()), Some(pid));
                }
            }
        }
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_image_handles_registries_and_tags() {
        assert_eq!(classify_image("mysql:8"), Some(ImageKind::Mysql));
        assert_eq!(classify_image("mariadb:10.11"), Some(ImageKind::Mysql));
        assert_eq!(classify_image("percona:8"), Some(ImageKind::Mysql));
        assert_eq!(classify_image("postgres:16"), Some(ImageKind::Postgres));
        assert_eq!(classify_image("redis:7-alpine"), Some(ImageKind::Redis));
        assert_eq!(classify_image("valkey/valkey:8"), Some(ImageKind::Redis));
        assert_eq!(classify_image("bitnami/postgresql:16"), Some(ImageKind::Postgres));
        assert_eq!(
            classify_image("docker.m.daocloud.io/library/mysql:8"),
            Some(ImageKind::Mysql),
        );
        assert_eq!(
            classify_image("quay.io/citus/citus:12"),
            Some(ImageKind::Postgres),
        );
    }

    #[test]
    fn classify_image_rejects_non_db_images() {
        assert_eq!(classify_image(""), None);
        assert_eq!(classify_image("nginx:latest"), None);
        assert_eq!(classify_image("rediscommander/redis-commander"), None);
        assert_eq!(classify_image("myredis/myapp"), None);
    }

    #[test]
    fn classify_image_is_case_insensitive() {
        assert_eq!(classify_image("MySQL:8"), Some(ImageKind::Mysql));
        assert_eq!(classify_image("POSTGRES:16"), Some(ImageKind::Postgres));
    }

    #[test]
    fn parse_port_mappings_keeps_host_bound_tcp_only() {
        let ports = "0.0.0.0:3307->3306/tcp, :::3307->3306/tcp, 5432/tcp";
        let got = parse_port_mappings(ports);
        assert_eq!(got.len(), 1, "ipv4+ipv6 should dedupe; unbound skipped: {got:?}");
        assert_eq!(got[0].host_port, 3307);
        assert_eq!(got[0].container_port, 3306);
        assert_eq!(got[0].proto, "tcp");
    }

    #[test]
    fn parse_port_mappings_ipv6_bracket_form() {
        let ports = "[::1]:5433->5432/tcp";
        let got = parse_port_mappings(ports);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].host, "::1");
        assert_eq!(got[0].host_port, 5433);
    }

    #[test]
    fn parse_port_mappings_skips_udp() {
        let ports = "0.0.0.0:53->53/udp";
        assert!(parse_port_mappings(ports).is_empty());
    }

    #[test]
    fn parse_port_mappings_empty_for_no_host_binding() {
        assert!(parse_port_mappings("").is_empty());
        assert!(parse_port_mappings("3306/tcp").is_empty());
        assert!(parse_port_mappings("   ").is_empty());
    }

    #[test]
    fn parse_listen_lines_ss_shape() {
        let stdout = "\
State    Recv-Q Send-Q Local Address:Port Peer Address:Port Process
LISTEN   0      128    0.0.0.0:22        0.0.0.0:*         users:((\"sshd\",pid=1234,fd=3))
LISTEN   0      70     127.0.0.1:3306    0.0.0.0:*         users:((\"mysqld\",pid=2345,fd=20))
LISTEN   0      511    *:6379            *:*               users:((\"redis-server\",pid=3456,fd=6))
";
        let got = parse_listen_lines(stdout);
        // sshd on 22 is not a DB port → skipped.
        assert_eq!(got.len(), 2, "expected mysql + redis, got {got:?}");
        let mysql = got.iter().find(|i| i.kind == DetectedDbKind::Mysql).unwrap();
        assert_eq!(mysql.host, "127.0.0.1");
        assert_eq!(mysql.port, 3306);
        assert_eq!(mysql.metadata.process_name.as_deref(), Some("mysqld"));
        assert_eq!(mysql.metadata.pid, Some(2345));
        assert_eq!(mysql.source, DetectionSource::Systemd);

        let redis = got.iter().find(|i| i.kind == DetectedDbKind::Redis).unwrap();
        assert_eq!(redis.port, 6379);
    }

    #[test]
    fn parse_listen_lines_docker_proxy_flagged_as_docker() {
        let stdout = "\
LISTEN 0 128 0.0.0.0:3307 0.0.0.0:* users:((\"docker-proxy\",pid=4321,fd=4))
";
        let got = parse_listen_lines(stdout);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, DetectionSource::Docker);
        assert_eq!(got[0].port, 3307);
        assert_eq!(got[0].metadata.process_name.as_deref(), Some("docker-proxy"));
    }

    #[test]
    fn parse_listen_lines_netstat_shape() {
        let stdout = "\
Proto Recv-Q Send-Q Local Address        Foreign Address  State   PID/Program
tcp   0      0      127.0.0.1:5432       0.0.0.0:*        LISTEN  1111/postgres
";
        let got = parse_listen_lines(stdout);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, DetectedDbKind::Postgres);
        assert_eq!(got[0].metadata.process_name.as_deref(), Some("postgres"));
        assert_eq!(got[0].metadata.pid, Some(1111));
    }

    #[test]
    fn parse_listen_lines_skips_non_listen() {
        let stdout = "\
LISTEN 0 128 127.0.0.1:3306 0.0.0.0:* users:((\"mysqld\",pid=1,fd=1))
ESTAB  0 0   127.0.0.1:51234 127.0.0.1:3306 users:((\"mysql\",pid=2,fd=2))
";
        let got = parse_listen_lines(stdout);
        assert_eq!(got.len(), 1, "ESTAB row must be ignored: {got:?}");
    }

    #[test]
    fn parse_listen_lines_no_process_info_is_direct() {
        // `ss -tln` without `-p` (or netstat without -p) omits
        // the process column.
        let stdout = "LISTEN 0 128 127.0.0.1:6379 0.0.0.0:*\n";
        let got = parse_listen_lines(stdout);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, DetectionSource::Direct);
        assert_eq!(got[0].metadata.pid, None);
    }

    #[test]
    fn normalize_bind_host_folds_wildcards() {
        assert_eq!(normalize_bind_host("0.0.0.0"), "0.0.0.0");
        assert_eq!(normalize_bind_host("::"), "0.0.0.0");
        assert_eq!(normalize_bind_host("[::]"), "0.0.0.0");
        assert_eq!(normalize_bind_host("*"), "0.0.0.0");
        assert_eq!(normalize_bind_host("127.0.0.1"), "127.0.0.1");
    }

    #[test]
    fn split_host_port_parses_ipv4_and_ipv6_forms() {
        assert_eq!(split_host_port("0.0.0.0:3307"), Some(("0.0.0.0".into(), 3307)));
        assert_eq!(split_host_port("[::1]:5432"), Some(("::1".into(), 5432)));
        assert_eq!(split_host_port("127.0.0.1:6379"), Some(("127.0.0.1".into(), 6379)));
        assert_eq!(split_host_port("no-port"), None);
    }

    #[test]
    fn short_container_id_truncates_long_ids() {
        assert_eq!(short_container_id("abcdef0123456789"), "abcdef012345");
        assert_eq!(short_container_id("short"), "short");
    }

    #[test]
    fn extract_process_info_handles_both_shapes() {
        let (name, pid) = extract_process_info(
            "LISTEN 0 128 0.0.0.0:3306 0.0.0.0:* users:((\"mysqld\",pid=99,fd=20))",
        );
        assert_eq!(name.as_deref(), Some("mysqld"));
        assert_eq!(pid, Some(99));

        let (name, pid) =
            extract_process_info("tcp 0 0 0.0.0.0:5432 0.0.0.0:* LISTEN 42/postgres");
        assert_eq!(name.as_deref(), Some("postgres"));
        assert_eq!(pid, Some(42));

        let (name, pid) = extract_process_info("LISTEN 0 128 0.0.0.0:6379 0.0.0.0:*");
        assert_eq!(name, None);
        assert_eq!(pid, None);
    }

    #[test]
    fn container_label_prefers_first_name() {
        assert_eq!(container_label("/my-mysql", "mysql:8"), "my-mysql");
        assert_eq!(container_label("/a,/b", "x:1"), "a");
        assert_eq!(container_label("", "mysql:8"), "mysql:8");
    }

    #[test]
    fn detected_db_instance_round_trips_through_json() {
        let inst = DetectedDbInstance {
            source: DetectionSource::Docker,
            kind: DetectedDbKind::Mysql,
            host: "0.0.0.0".into(),
            port: 3307,
            label: "my-mysql".into(),
            metadata: DetectedDbMetadata {
                image: Some("mysql:8".into()),
                container_id: Some("abcdef012345".into()),
                ..Default::default()
            },
            signature: "docker://abcdef012345/0.0.0.0:3307".into(),
        };
        let json = serde_json::to_string(&inst).unwrap();
        let back: DetectedDbInstance = serde_json::from_str(&json).unwrap();
        assert_eq!(inst, back);
        assert!(json.contains("\"source\":\"docker\""));
        assert!(json.contains("\"kind\":\"mysql\""));
    }
}
