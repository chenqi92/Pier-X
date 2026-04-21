//! Docker panel backend — containers, images, volumes over
//! an existing [`crate::ssh::SshSession`].
//!
//! ## Shape vs Redis
//!
//! Redis (M5a) owns its own connection: the `RedisClient`
//! wraps a `redis::aio::ConnectionManager` and the caller
//! just asks it questions. Docker is different — there is no
//! "docker protocol" we can hit; the canonical way to talk to
//! a remote dockerd without installing Docker SDKs on every
//! target is the `docker` CLI itself. Every operation in this
//! module is therefore implemented as a one-shot `exec_command`
//! against the underlying SSH session, parsing the stdout
//! that falls out.
//!
//! This module is deliberately stateless — it doesn't hold an
//! `SshSession` of its own. Callers pass `&SshSession` into
//! each function. The FFI layer owns the session, the Rust
//! layer owns the parsing. That keeps the test surface small
//! (the parsers are pure functions) and lets the session be
//! reused for other things the UI cares about (inspect,
//! stats, logs via [`crate::ssh::ExecStream`]).
//!
//! ## Why `--format '{{json .}}'`?
//!
//! Docker ships two JSON modes:
//!
//!   * `--format json` — one giant JSON array (newer CLI only)
//!   * `--format '{{json .}}'` — one JSON object per line, NDJSON-style
//!
//! The per-line form is supported in every docker ≥ 1.8 and
//! is what Compose, docker-py, and most GUIs use. We parse it
//! by splitting on `\n` and running `serde_json::from_str` on
//! each non-empty line. A single corrupt line doesn't break
//! the whole listing — we log a warning and skip it.
//!
//! ## Shell safety
//!
//! Every container id / name we pass to `docker <cmd> <id>`
//! has to survive interpolation into a shell command string
//! on the remote. We refuse any id that isn't
//! `^[A-Za-z0-9][A-Za-z0-9_.-]{0,254}$` — that's a superset
//! of Docker's own id + name grammar and blocks everything
//! that could possibly mean "shell metacharacter". Combined
//! with the fact that we only ever pass ids that came back
//! from a previous `docker ps` listing, that's a belt-and-
//! suspenders defense against shell injection.
//!
//! ## Not yet
//!
//! * Images / volumes listings. M5c ships containers only;
//!   the parsers here have room for siblings when a UI tab
//!   needs them.
//! * Pull / push / build. Those are long-running with live
//!   progress output and should flow through
//!   [`crate::ssh::ExecStream`], not this module.
//! * `docker stats` live feed. Same — belongs in the stream
//!   module.

use serde::{Deserialize, Serialize};

use crate::ssh::error::{Result, SshError};
use crate::ssh::SshSession;

/// One row from `docker ps --format '{{json .}}'`.
///
/// Fields are a deliberate subset of what docker returns —
/// we only keep what the UI actually renders. Adding a field
/// here is safe because `serde` defaults unknown fields to
/// `#[serde(default)]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Container {
    /// Short (12-char) container id.
    #[serde(rename(serialize = "id", deserialize = "ID"), alias = "id", default)]
    pub id: String,
    /// Image reference, e.g. `redis:7-alpine`.
    #[serde(
        rename(serialize = "image", deserialize = "Image"),
        alias = "image",
        default
    )]
    pub image: String,
    /// Friendly name assigned by the user or docker.
    #[serde(
        rename(serialize = "names", deserialize = "Names"),
        alias = "names",
        default
    )]
    pub names: String,
    /// Human status, e.g. `"Up 5 minutes"` / `"Exited (0) 3 hours ago"`.
    #[serde(
        rename(serialize = "status", deserialize = "Status"),
        alias = "status",
        default
    )]
    pub status: String,
    /// Low-level state: `"running"`, `"exited"`, `"paused"`,
    /// `"created"`, `"restarting"`, etc.
    #[serde(
        rename(serialize = "state", deserialize = "State"),
        alias = "state",
        default
    )]
    pub state: String,
    /// Freeform "X minutes ago" description of creation time.
    #[serde(
        rename(serialize = "created", deserialize = "CreatedAt"),
        alias = "created",
        default
    )]
    pub created: String,
    /// Port bindings, e.g. `"0.0.0.0:8080->80/tcp"`.
    #[serde(
        rename(serialize = "ports", deserialize = "Ports"),
        alias = "ports",
        default
    )]
    pub ports: String,
}

impl Container {
    /// True when `state == "running"` — the UI uses this to
    /// decide which action buttons to enable.
    pub fn is_running(&self) -> bool {
        self.state.eq_ignore_ascii_case("running")
    }
}

/// List every container on the remote. When `all` is true,
/// also include stopped / exited containers (`docker ps -a`).
pub async fn list_containers(session: &SshSession, all: bool) -> Result<Vec<Container>> {
    // `--format '{{json .}}'` emits one object per line. We
    // deliberately do NOT wrap this in a shell — the SSH
    // server's login shell handles it. No arguments from
    // untrusted input go into this command.
    let cmd = if all {
        "docker ps --all --no-trunc --format '{{json .}}'"
    } else {
        "docker ps --no-trunc --format '{{json .}}'"
    };
    let (exit, stdout) = session.exec_command(cmd).await?;
    if exit != 0 {
        return Err(SshError::InvalidConfig(format!(
            "docker ps exited {exit}: {}",
            stdout.lines().next().unwrap_or("").trim()
        )));
    }
    Ok(parse_ps_lines(&stdout))
}

/// Blocking wrapper for [`list_containers`].
pub fn list_containers_blocking(session: &SshSession, all: bool) -> Result<Vec<Container>> {
    crate::ssh::runtime::shared().block_on(list_containers(session, all))
}

/// Start a stopped container.
pub async fn start(session: &SshSession, id: &str) -> Result<()> {
    run_simple_action(session, "start", id, false).await
}

/// Stop a running container.
pub async fn stop(session: &SshSession, id: &str) -> Result<()> {
    run_simple_action(session, "stop", id, false).await
}

/// Restart a container (stop then start).
pub async fn restart(session: &SshSession, id: &str) -> Result<()> {
    run_simple_action(session, "restart", id, false).await
}

/// Remove a container. Pass `force = true` for `rm -f` which
/// also kills running containers — the UI should always
/// confirm with the user before passing `true`.
pub async fn remove(session: &SshSession, id: &str, force: bool) -> Result<()> {
    run_simple_action(session, "rm", id, force).await
}

/// Inspect a single container and return the raw JSON array
/// emitted by `docker inspect`.
pub async fn inspect_container(session: &SshSession, id: &str) -> Result<String> {
    if !is_safe_id(id) {
        return Err(SshError::InvalidConfig(format!(
            "refusing unsafe docker id {id:?}"
        )));
    }
    let cmd = format!("docker inspect --type container {id}");
    let (exit, stdout) = session.exec_command(&cmd).await?;
    if exit != 0 {
        return Err(SshError::InvalidConfig(format!(
            "docker inspect exited {exit}: {}",
            stdout.lines().next().unwrap_or("").trim()
        )));
    }
    Ok(stdout)
}

/// Run `docker <args...>` on the remote and return the raw
/// `(exit_code, stdout)` pair without interpreting success.
///
/// This is used for the richer UI flows (inspect image,
/// pull/prune, run, network create, etc.) that don't fit the
/// narrow container-only action API above.
pub async fn exec(session: &SshSession, args: &[String]) -> Result<(i32, String)> {
    let cmd = if args.is_empty() {
        String::from("docker")
    } else {
        format!(
            "docker {}",
            join_shell_args(args.iter().map(String::as_str))
        )
    };
    session.exec_command(&cmd).await
}

/// Blocking wrapper for [`exec`].
pub fn exec_blocking(session: &SshSession, args: &[String]) -> Result<(i32, String)> {
    crate::ssh::runtime::shared().block_on(exec(session, args))
}

/// Blocking wrappers for each action. Explicit instead of a
/// macro so clippy / docs can see them.
pub fn start_blocking(session: &SshSession, id: &str) -> Result<()> {
    crate::ssh::runtime::shared().block_on(start(session, id))
}
/// Blocking wrapper for [`stop`].
pub fn stop_blocking(session: &SshSession, id: &str) -> Result<()> {
    crate::ssh::runtime::shared().block_on(stop(session, id))
}
/// Blocking wrapper for [`restart`].
pub fn restart_blocking(session: &SshSession, id: &str) -> Result<()> {
    crate::ssh::runtime::shared().block_on(restart(session, id))
}
/// Blocking wrapper for [`remove`].
pub fn remove_blocking(session: &SshSession, id: &str, force: bool) -> Result<()> {
    crate::ssh::runtime::shared().block_on(remove(session, id, force))
}
/// Blocking wrapper for [`inspect_container`].
pub fn inspect_container_blocking(session: &SshSession, id: &str) -> Result<String> {
    crate::ssh::runtime::shared().block_on(inspect_container(session, id))
}

// ═══════════════════════════════════════════════════════════
// Container stats (CPU / memory snapshot)
// ═══════════════════════════════════════════════════════════

/// One row from `docker stats --no-stream --format '{{json .}}'`.
///
/// Every field arrives as a pre-formatted string — docker does not
/// expose raw bytes/percentages through `docker stats` when a format
/// template is used, so we keep them as strings and let the UI
/// decide how to render them.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerStat {
    /// Short container id, matches [`Container::id`].
    #[serde(rename(serialize = "id", deserialize = "ID"), alias = "id", default)]
    pub id: String,
    /// Container name, matches [`Container::names`].
    #[serde(
        rename(serialize = "name", deserialize = "Name"),
        alias = "name",
        default
    )]
    pub name: String,
    /// CPU usage as a pre-formatted percentage string, e.g. `"1.23%"`.
    #[serde(
        rename(serialize = "cpuPerc", deserialize = "CPUPerc"),
        alias = "cpuPerc",
        default
    )]
    pub cpu_perc: String,
    /// Memory usage / limit, e.g. `"48.5MiB / 1.94GiB"`.
    #[serde(
        rename(serialize = "memUsage", deserialize = "MemUsage"),
        alias = "memUsage",
        default
    )]
    pub mem_usage: String,
    /// Memory usage as a percentage of the container limit, e.g. `"2.44%"`.
    #[serde(
        rename(serialize = "memPerc", deserialize = "MemPerc"),
        alias = "memPerc",
        default
    )]
    pub mem_perc: String,
}

/// Snapshot of per-container CPU / memory usage from `docker stats`.
///
/// Uses `--no-stream` so we get one sample and exit. This is deliberate —
/// live streaming should flow through [`crate::ssh::ExecStream`], not
/// this module.
pub async fn list_container_stats(session: &SshSession) -> Result<Vec<ContainerStat>> {
    let cmd = "docker stats --no-stream --format '{{json .}}'";
    let (exit, stdout) = session.exec_command(cmd).await?;
    if exit != 0 {
        return Err(SshError::InvalidConfig(format!(
            "docker stats exited {exit}: {}",
            stdout.lines().next().unwrap_or("").trim()
        )));
    }
    Ok(parse_ndjson(&stdout))
}

/// Blocking wrapper for [`list_container_stats`].
pub fn list_container_stats_blocking(session: &SshSession) -> Result<Vec<ContainerStat>> {
    crate::ssh::runtime::shared().block_on(list_container_stats(session))
}

// ═══════════════════════════════════════════════════════════
// Images
// ═══════════════════════════════════════════════════════════

/// One row from `docker images`.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerImage {
    #[serde(rename(serialize = "id", deserialize = "ID"), alias = "id", default)]
    pub id: String,
    #[serde(
        rename(serialize = "repository", deserialize = "Repository"),
        alias = "repository",
        default
    )]
    pub repository: String,
    #[serde(rename(serialize = "tag", deserialize = "Tag"), alias = "tag", default)]
    pub tag: String,
    #[serde(
        rename(serialize = "size", deserialize = "Size"),
        alias = "size",
        default
    )]
    pub size: String,
    #[serde(
        rename(serialize = "created", deserialize = "CreatedAt"),
        alias = "created",
        default
    )]
    pub created: String,
}

/// List images.
pub async fn list_images(session: &SshSession) -> Result<Vec<DockerImage>> {
    let cmd = "docker images --format '{{json .}}'";
    let (exit, stdout) = session.exec_command(cmd).await?;
    if exit != 0 {
        return Err(SshError::InvalidConfig(format!(
            "docker images exited {exit}"
        )));
    }
    Ok(parse_ndjson::<DockerImage>(&stdout))
}

/// Blocking wrapper.
pub fn list_images_blocking(session: &SshSession) -> Result<Vec<DockerImage>> {
    crate::ssh::runtime::shared().block_on(list_images(session))
}

/// Remove an image.
pub async fn remove_image(session: &SshSession, id: &str, force: bool) -> Result<()> {
    if !is_safe_id(id) {
        return Err(SshError::InvalidConfig(format!("unsafe image id {id:?}")));
    }
    let cmd = if force {
        format!("docker rmi --force {id}")
    } else {
        format!("docker rmi {id}")
    };
    let (exit, stdout) = session.exec_command(&cmd).await?;
    if exit != 0 {
        return Err(SshError::InvalidConfig(format!(
            "docker rmi exited {exit}: {}",
            stdout.lines().next().unwrap_or("").trim()
        )));
    }
    Ok(())
}

/// Blocking wrapper.
pub fn remove_image_blocking(session: &SshSession, id: &str, force: bool) -> Result<()> {
    crate::ssh::runtime::shared().block_on(remove_image(session, id, force))
}

// ═══════════════════════════════════════════════════════════
// Volumes
// ═══════════════════════════════════════════════════════════

/// One row from `docker volume ls`.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerVolume {
    #[serde(
        rename(serialize = "name", deserialize = "Name"),
        alias = "name",
        default
    )]
    pub name: String,
    #[serde(
        rename(serialize = "driver", deserialize = "Driver"),
        alias = "driver",
        default
    )]
    pub driver: String,
    #[serde(
        rename(serialize = "mountpoint", deserialize = "Mountpoint"),
        alias = "mountpoint",
        default
    )]
    pub mountpoint: String,
}

/// List volumes.
pub async fn list_volumes(session: &SshSession) -> Result<Vec<DockerVolume>> {
    let cmd = "docker volume ls --format '{{json .}}'";
    let (exit, stdout) = session.exec_command(cmd).await?;
    if exit != 0 {
        return Err(SshError::InvalidConfig(format!(
            "docker volume ls exited {exit}"
        )));
    }
    Ok(parse_ndjson::<DockerVolume>(&stdout))
}

/// Blocking wrapper.
pub fn list_volumes_blocking(session: &SshSession) -> Result<Vec<DockerVolume>> {
    crate::ssh::runtime::shared().block_on(list_volumes(session))
}

/// Per-volume disk usage from `docker system df -v`.
///
/// Docker's own `system df -v` is the correct source for per-volume
/// size — it accounts for shared layers and reflects what actually
/// lives under the volume directory. `du -sh` against the mountpoint
/// would need root on most hosts and would double-count hardlinks.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeDiskUsage {
    /// Volume name, matches [`DockerVolume::name`].
    #[serde(
        rename(serialize = "name", deserialize = "Name"),
        alias = "name",
        default
    )]
    pub name: String,
    /// Pre-formatted size string, e.g. `"1.234GB"` or `"0B"`.
    #[serde(
        rename(serialize = "size", deserialize = "Size"),
        alias = "size",
        default
    )]
    pub size: String,
    /// Number of containers referencing this volume.
    #[serde(
        rename(serialize = "links", deserialize = "Links"),
        alias = "links",
        default
    )]
    pub links: i64,
}

/// `docker system df -v` emits a single JSON object that bundles every
/// category (images / containers / volumes / buildcache). We only care
/// about `Volumes`; this wrapper exists purely to carve that array out.
#[derive(Deserialize)]
struct SystemDfVerbose {
    #[serde(alias = "Volumes", default)]
    volumes: Vec<VolumeDiskUsage>,
}

/// Parser for `docker system df -v --format '{{json .}}'`. Accepts either
/// the single-object form (current docker) or a first-line JSON with
/// trailing noise (older docker CLI).
pub fn parse_volume_df(stdout: &str) -> Vec<VolumeDiskUsage> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<SystemDfVerbose>(trimmed) {
        Ok(df) => df.volumes,
        Err(_) => {
            // Some docker versions emit one line per section instead of
            // one big object. Walk lines looking for one that parses.
            for line in trimmed.lines() {
                let line = line.trim();
                if let Ok(df) = serde_json::from_str::<SystemDfVerbose>(line) {
                    return df.volumes;
                }
            }
            Vec::new()
        }
    }
}

/// Pull per-volume sizes from the remote via `docker system df -v`.
pub async fn list_volume_sizes(session: &SshSession) -> Result<Vec<VolumeDiskUsage>> {
    let cmd = "docker system df -v --format '{{json .}}'";
    let (exit, stdout) = session.exec_command(cmd).await?;
    if exit != 0 {
        return Err(SshError::InvalidConfig(format!(
            "docker system df exited {exit}: {}",
            stdout.lines().next().unwrap_or("").trim()
        )));
    }
    Ok(parse_volume_df(&stdout))
}

/// Blocking wrapper for [`list_volume_sizes`].
pub fn list_volume_sizes_blocking(session: &SshSession) -> Result<Vec<VolumeDiskUsage>> {
    crate::ssh::runtime::shared().block_on(list_volume_sizes(session))
}

/// Parse `"1.234GB"` / `"0B"` / `"48.5MiB"` into raw bytes so the UI can
/// sort by size. Returns `0` on anything unparseable — sort stability is
/// more important than perfect fidelity when docker's own string is
/// already lossy.
pub fn parse_size_to_bytes(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    let (num_end, unit) = match s.find(|c: char| c.is_ascii_alphabetic()) {
        Some(i) => (i, s[i..].trim().to_ascii_lowercase()),
        None => (s.len(), String::new()),
    };
    let num: f64 = s[..num_end].trim().parse().unwrap_or(0.0);
    let mult: f64 = match unit.as_str() {
        "" | "b" => 1.0,
        "k" | "kb" => 1_000.0,
        "ki" | "kib" => 1_024.0,
        "m" | "mb" => 1_000_000.0,
        "mi" | "mib" => 1_024.0 * 1_024.0,
        "g" | "gb" => 1_000_000_000.0,
        "gi" | "gib" => 1_024.0 * 1_024.0 * 1_024.0,
        "t" | "tb" => 1_000_000_000_000.0,
        "ti" | "tib" => 1_024.0_f64.powi(4),
        _ => 1.0,
    };
    (num * mult) as u64
}

/// Remove a volume.
pub async fn remove_volume(session: &SshSession, name: &str) -> Result<()> {
    run_simple_action(session, "volume rm", name, false).await
}

/// Blocking wrapper.
pub fn remove_volume_blocking(session: &SshSession, name: &str) -> Result<()> {
    crate::ssh::runtime::shared().block_on(remove_volume(session, name))
}

// ═══════════════════════════════════════════════════════════
// Networks
// ═══════════════════════════════════════════════════════════

/// One row from `docker network ls`.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerNetwork {
    #[serde(rename(serialize = "id", deserialize = "ID"), alias = "id", default)]
    pub id: String,
    #[serde(
        rename(serialize = "name", deserialize = "Name"),
        alias = "name",
        default
    )]
    pub name: String,
    #[serde(
        rename(serialize = "driver", deserialize = "Driver"),
        alias = "driver",
        default
    )]
    pub driver: String,
    #[serde(
        rename(serialize = "scope", deserialize = "Scope"),
        alias = "scope",
        default
    )]
    pub scope: String,
}

/// List networks.
pub async fn list_networks(session: &SshSession) -> Result<Vec<DockerNetwork>> {
    let cmd = "docker network ls --format '{{json .}}'";
    let (exit, stdout) = session.exec_command(cmd).await?;
    if exit != 0 {
        return Err(SshError::InvalidConfig(format!(
            "docker network ls exited {exit}"
        )));
    }
    Ok(parse_ndjson::<DockerNetwork>(&stdout))
}

/// Blocking wrapper.
pub fn list_networks_blocking(session: &SshSession) -> Result<Vec<DockerNetwork>> {
    crate::ssh::runtime::shared().block_on(list_networks(session))
}

/// Remove a network.
pub async fn remove_network(session: &SshSession, name: &str) -> Result<()> {
    run_simple_action(session, "network rm", name, false).await
}

/// Blocking wrapper.
pub fn remove_network_blocking(session: &SshSession, name: &str) -> Result<()> {
    crate::ssh::runtime::shared().block_on(remove_network(session, name))
}

/// Internal: run `docker <verb> [--force] <id>`, returning an
/// error if the id fails the safety check or if docker exits
/// non-zero.
async fn run_simple_action(session: &SshSession, verb: &str, id: &str, force: bool) -> Result<()> {
    if !is_safe_id(id) {
        return Err(SshError::InvalidConfig(format!(
            "refusing unsafe docker id {id:?}"
        )));
    }
    let cmd = if force {
        format!("docker {verb} --force {id}")
    } else {
        format!("docker {verb} {id}")
    };
    let (exit, stdout) = session.exec_command(&cmd).await?;
    if exit != 0 {
        return Err(SshError::InvalidConfig(format!(
            "docker {verb} exited {exit}: {}",
            stdout.lines().next().unwrap_or("").trim()
        )));
    }
    Ok(())
}

/// Generic NDJSON parser — one JSON object per line.
pub fn parse_ndjson<T: serde::de::DeserializeOwned>(stdout: &str) -> Vec<T> {
    let mut out = Vec::new();
    for raw in stdout.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<T>(line) {
            Ok(v) => out.push(v),
            Err(e) => log::warn!("docker: skipping malformed line: {e}"),
        }
    }
    out
}

/// Parse an NDJSON-style `docker ps` stdout into containers.
pub fn parse_ps_lines(stdout: &str) -> Vec<Container> {
    parse_ndjson(stdout)
}

/// Single-quote a shell argument for safe interpolation into a
/// POSIX-compatible shell command string.
pub fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'/' | b':' | b'.' | b'-' | b'@' | b'=' | b'+' | b','))
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

/// Join shell arguments into one safely-quoted command tail.
pub fn join_shell_args<'a>(args: impl IntoIterator<Item = &'a str>) -> String {
    args.into_iter()
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Strict allowlist for docker identifiers. Matches the
/// superset of Docker's own `name` and `id` grammars:
///
///   * Must start with `[A-Za-z0-9]`
///   * Follow-up chars from `[A-Za-z0-9_.-]`
///   * 1..=255 characters total
///
/// Anything outside that set is rejected up-front — we never
/// try to quote-and-escape because escaping is error-prone
/// and the legitimate grammar has no overlap with shell
/// metacharacters anyway.
pub fn is_safe_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 255 {
        return false;
    }
    let mut chars = id.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ps_lines_happy_path() {
        let stdout = r#"{"ID":"abc123","Image":"redis:7","Names":"cache","Status":"Up 5 minutes","State":"running","CreatedAt":"2025-01-01 10:00:00 +0000 UTC","Ports":"0.0.0.0:6379->6379/tcp"}
{"ID":"def456","Image":"postgres:16","Names":"db","Status":"Exited (0) 2 hours ago","State":"exited","CreatedAt":"2025-01-01 08:00:00 +0000 UTC","Ports":""}
"#;
        let containers = parse_ps_lines(stdout);
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0].id, "abc123");
        assert_eq!(containers[0].names, "cache");
        assert_eq!(containers[0].state, "running");
        assert!(containers[0].is_running());
        assert_eq!(containers[1].id, "def456");
        assert_eq!(containers[1].state, "exited");
        assert!(!containers[1].is_running());
    }

    #[test]
    fn parse_ps_lines_skips_blanks_and_malformed() {
        let stdout = r#"
{"ID":"aaa","Image":"nginx","Names":"web","Status":"Up 1d","State":"running","CreatedAt":"x","Ports":""}

not-json-at-all
{"ID":"bbb","Image":"alpine","Names":"sh","Status":"Exited","State":"exited","CreatedAt":"x","Ports":""}
"#;
        let containers = parse_ps_lines(stdout);
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0].id, "aaa");
        assert_eq!(containers[1].id, "bbb");
    }

    #[test]
    fn parse_ps_lines_missing_fields_default_to_empty() {
        // Older docker versions omitted some fields. serde's
        // `#[serde(default)]` should fill them in as empty
        // strings rather than erroring.
        let stdout = r#"{"ID":"xyz","Image":"busybox"}"#;
        let containers = parse_ps_lines(stdout);
        assert_eq!(containers.len(), 1);
        assert_eq!(containers[0].id, "xyz");
        assert_eq!(containers[0].image, "busybox");
        assert_eq!(containers[0].names, "");
        assert_eq!(containers[0].state, "");
    }

    #[test]
    fn parse_ps_lines_empty_stdout_returns_empty_vec() {
        assert!(parse_ps_lines("").is_empty());
        assert!(parse_ps_lines("   \n\n").is_empty());
    }

    #[test]
    fn is_safe_id_accepts_canonical_forms() {
        assert!(is_safe_id("abc123")); // short id
        assert!(is_safe_id("0123456789abcdef0123456789abcdef01234567")); // 40-char full id
        assert!(is_safe_id("my_service")); // compose-style
        assert!(is_safe_id("my-service.1")); // swarm task name
        assert!(is_safe_id("Z9")); // 2 chars
    }

    #[test]
    fn is_safe_id_rejects_shell_metacharacters() {
        for evil in [
            "",
            "a b",
            "a;rm -rf /",
            "a|b",
            "a&&b",
            "a$PATH",
            "a`whoami`",
            "a\nb",
            "a\"b",
            "a'b",
            "a/b",
            "a\\b",
            "-flag",
            ".leading-dot",
            "_leading-under",
        ] {
            assert!(!is_safe_id(evil), "{evil:?} must be rejected");
        }
    }

    #[test]
    fn is_safe_id_rejects_overlong() {
        let long = "a".repeat(256);
        assert!(!is_safe_id(&long));
        let max = "a".repeat(255);
        assert!(is_safe_id(&max));
    }

    #[test]
    fn container_round_trips_through_json() {
        let c = Container {
            id: "abc".into(),
            image: "nginx:stable".into(),
            names: "web".into(),
            status: "Up 10m".into(),
            state: "running".into(),
            created: "2025-01-01".into(),
            ports: "80/tcp".into(),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Container = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn parse_container_stats_happy_path() {
        let stdout = r#"{"ID":"abc","Name":"cache","CPUPerc":"1.23%","MemUsage":"48.5MiB / 1.94GiB","MemPerc":"2.44%"}
{"ID":"def","Name":"db","CPUPerc":"0.01%","MemUsage":"220MiB / 2GiB","MemPerc":"11.00%"}
"#;
        let stats = parse_ndjson::<ContainerStat>(stdout);
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].id, "abc");
        assert_eq!(stats[0].cpu_perc, "1.23%");
        assert_eq!(stats[0].mem_usage, "48.5MiB / 1.94GiB");
        assert_eq!(stats[1].name, "db");
    }

    #[test]
    fn parse_volume_df_extracts_volumes_array() {
        let stdout = r#"{"Images":[],"Containers":[],"Volumes":[{"Name":"warehouse_db","Size":"4.2GB","Links":1},{"Name":"redis_data","Size":"186MB","Links":1}],"BuildCache":[]}"#;
        let volumes = parse_volume_df(stdout);
        assert_eq!(volumes.len(), 2);
        assert_eq!(volumes[0].name, "warehouse_db");
        assert_eq!(volumes[0].size, "4.2GB");
        assert_eq!(volumes[0].links, 1);
        assert_eq!(volumes[1].name, "redis_data");
    }

    #[test]
    fn parse_volume_df_empty_stdout_returns_empty_vec() {
        assert!(parse_volume_df("").is_empty());
        assert!(parse_volume_df("   \n").is_empty());
    }

    #[test]
    fn parse_size_to_bytes_covers_common_units() {
        assert_eq!(parse_size_to_bytes("0B"), 0);
        assert_eq!(parse_size_to_bytes("186MB"), 186_000_000);
        assert_eq!(parse_size_to_bytes("186MiB"), 186 * 1024 * 1024);
        assert_eq!(parse_size_to_bytes("1.5GB"), 1_500_000_000);
        assert_eq!(parse_size_to_bytes("48.5MiB"), (48.5_f64 * 1024.0 * 1024.0) as u64);
        assert_eq!(parse_size_to_bytes(""), 0);
        assert_eq!(parse_size_to_bytes("garbage"), 0);
    }
}
