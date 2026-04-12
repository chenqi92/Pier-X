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
    #[serde(rename = "ID", default)]
    pub id: String,
    /// Image reference, e.g. `redis:7-alpine`.
    #[serde(rename = "Image", default)]
    pub image: String,
    /// Friendly name assigned by the user or docker.
    #[serde(rename = "Names", default)]
    pub names: String,
    /// Human status, e.g. `"Up 5 minutes"` / `"Exited (0) 3 hours ago"`.
    #[serde(rename = "Status", default)]
    pub status: String,
    /// Low-level state: `"running"`, `"exited"`, `"paused"`,
    /// `"created"`, `"restarting"`, etc.
    #[serde(rename = "State", default)]
    pub state: String,
    /// Freeform "X minutes ago" description of creation time.
    #[serde(rename = "CreatedAt", default)]
    pub created: String,
    /// Port bindings, e.g. `"0.0.0.0:8080->80/tcp"`.
    #[serde(rename = "Ports", default)]
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

/// Internal: run `docker <verb> [--force] <id>`, returning an
/// error if the id fails the safety check or if docker exits
/// non-zero.
async fn run_simple_action(
    session: &SshSession,
    verb: &str,
    id: &str,
    force: bool,
) -> Result<()> {
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

/// Parse an NDJSON-style `docker ps` stdout into a vec of
/// containers. Malformed lines are logged and skipped rather
/// than failing the whole listing, because one corrupt entry
/// shouldn't hide every other container from the user.
fn parse_ps_lines(stdout: &str) -> Vec<Container> {
    let mut out = Vec::new();
    for raw in stdout.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Container>(line) {
            Ok(c) => out.push(c),
            Err(e) => log::warn!("docker ps: skipping malformed line: {e}"),
        }
    }
    out
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
        assert!(is_safe_id("abc123"));              // short id
        assert!(is_safe_id("0123456789abcdef0123456789abcdef01234567")); // 40-char full id
        assert!(is_safe_id("my_service"));          // compose-style
        assert!(is_safe_id("my-service.1"));        // swarm task name
        assert!(is_safe_id("Z9"));                  // 2 chars
    }

    #[test]
    fn is_safe_id_rejects_shell_metacharacters() {
        for evil in [
            "", "a b", "a;rm -rf /", "a|b", "a&&b", "a$PATH",
            "a`whoami`", "a\nb", "a\"b", "a'b", "a/b", "a\\b",
            "-flag", ".leading-dot", "_leading-under",
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
}
