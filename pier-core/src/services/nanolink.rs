//! NanoLink integration (detect role + control agent + query server).
//!
//! [NanoLink](https://github.com/chenqi92/NanoLink) is a lightweight
//! server-monitoring + remote-management platform. A host can run two
//! independent processes:
//!
//!   * **Agent** (`nanolink-agent`, Rust) — the *client*: dials OUTBOUND
//!     gRPC to one or more servers, exposes no inbound port (an optional
//!     loopback management API on `127.0.0.1:9101` is off by default).
//!   * **Server** (`nanolink-server`, Go) — the *collector*: LISTENS on
//!     `:8080` (REST + embedded dashboard), `:9100` (dashboard WS) and a
//!     config-driven gRPC port (default `:9200`, but agent docs use
//!     `:39100` — so we never assume it, we read `/api/server-info`).
//!
//! The two are different binaries / units / ports, so "which is
//! installed" is the reliable role signal — not a port scan (the agent
//! listens for nothing). A single host can be **both**.
//!
//! ## How this module talks to a host
//!
//! Everything runs over the already-established [`SshSession`]:
//!
//!   * Role / status / agent control → shell-out (`which`, `systemctl`,
//!     `nanolink-agent …`).
//!   * Server data → `curl http://localhost:<port>/…` *on the host*
//!     (the chosen architecture: the JWT never leaves the box and we
//!     don't depend on the server's ports being reachable from Pier-X).
//!
//! All functions return plain serde types (UI-agnostic, per the
//! pier-core contract); the camelCase wire shape is pinned here with
//! `rename_all` so `src-tauri` can return them directly.
//!
//! ## Upstream uncertainty
//!
//! Some NanoLink CLI/REST shapes (exact `server list` output, the
//! `/api/agents` envelope, whether `--version` exists) are not pinned by
//! the upstream docs. The probes below are written leniently: they fall
//! back to raw text and tolerate missing JSON fields rather than failing
//! the whole call.

use serde::{Deserialize, Serialize};

use crate::ssh::error::{Result, SshError};
use crate::ssh::SshSession;

/// Default REST/dashboard HTTP port a standalone server listens on.
pub const DEFAULT_HTTP_PORT: u16 = 8080;

// ─────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────

/// Detected NanoLink role + running state on a host.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NanoLinkStatus {
    /// `has_agent || has_server`.
    pub installed: bool,
    /// `nanolink-agent` present (binary or config or active unit).
    pub has_agent: bool,
    /// `nanolink-server` present (binary, active unit, or live REST).
    pub has_server: bool,
    /// Agent systemd/launchd unit is active.
    pub agent_running: bool,
    /// Server process answers `/api/health` or its unit is active.
    pub server_running: bool,
    /// `nanolink-agent --version` token (empty if unknown).
    pub agent_version: String,
    /// `nanolink-server --version` token (empty if unknown).
    pub server_version: String,
    /// One of `none` | `client` | `server` | `both`.
    pub role: String,
    /// First existing agent config path (`/etc/nanolink/nanolink.yaml`…),
    /// empty when none found.
    pub agent_config_path: String,
    /// Server REST/HTTP port (default 8080; 0 when no server).
    pub http_port: u16,
    /// Server gRPC port from `/api/server-info` (0 when unknown).
    pub grpc_port: u16,
    /// Server dashboard WS port from `/api/server-info` (0 when unknown).
    pub ws_port: u16,
    /// Whether the server requires auth (from `/api/server-info`).
    pub auth_enabled: bool,
}

/// Generic redacted result of a control command (service action,
/// server add/remove).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandReport {
    /// `ok` | `failed` | `sudo-requires-password`.
    pub status: String,
    /// Process exit code (-1 if it could not be determined).
    pub exit_code: i32,
    /// Last lines of combined stdout/stderr, secrets scrubbed.
    pub output: String,
}

/// Cluster summary served by a NanoLink server (`/api/summary`).
/// Parsed leniently — every field defaults to 0 when absent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerSummary {
    /// Number of currently-connected agents.
    pub connected_agents: u32,
    /// Mean CPU usage across agents, percent.
    pub avg_cpu_percent: f64,
    /// Mean memory usage across agents, percent.
    pub memory_percent: f64,
    /// Mean disk usage across agents, percent.
    pub disk_percent: f64,
    /// Aggregate physical memory across agents, bytes.
    pub total_memory: u64,
    /// Aggregate disk capacity across agents, bytes.
    pub total_disk: u64,
}

/// One agent row as a server reports it (`/api/agents`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerAgent {
    /// Stable agent id assigned by the server.
    pub id: String,
    /// Reported hostname.
    pub hostname: String,
    /// Operating system string.
    pub os: String,
    /// CPU architecture string.
    pub arch: String,
    /// Agent version.
    pub version: String,
    /// Permission level granted to this agent (0–3).
    pub permission_level: u8,
    /// Connection timestamp (server-formatted string).
    pub connected_at: String,
    /// Last heartbeat timestamp (server-formatted string).
    pub last_heartbeat: String,
    /// Whether the agent is currently connected.
    pub online: bool,
}

// ─────────────────────────────────────────────────────────
// Status / role detection
// ─────────────────────────────────────────────────────────

/// Probe a host for NanoLink and classify its role.
#[allow(clippy::field_reassign_with_default)]
pub async fn status(session: &SshSession) -> Result<NanoLinkStatus> {
    let mut s = NanoLinkStatus::default();

    // ── Agent presence ───────────────────────────────────
    s.has_agent = run(
        session,
        "command -v nanolink-agent >/dev/null 2>&1 && echo yes",
    )
    .await
    .1
    .contains("yes");
    s.agent_config_path = run(
        session,
        "for p in /etc/nanolink/nanolink.yaml /etc/nanolink.yaml /etc/nanolink/nanolink.toml \
         /etc/nanolink/config.yaml; do [ -f \"$p\" ] && { echo \"$p\"; break; }; done",
    )
    .await
    .1
    .trim()
    .to_string();
    if !s.agent_config_path.is_empty() {
        s.has_agent = true;
    }
    s.agent_running = unit_active(session, "nanolink-agent").await;
    if s.agent_running {
        s.has_agent = true;
    }
    if s.has_agent {
        s.agent_version = version_of(session, "nanolink-agent").await;
    }

    // ── Server presence ──────────────────────────────────
    s.has_server = run(
        session,
        "command -v nanolink-server >/dev/null 2>&1 && echo yes",
    )
    .await
    .1
    .contains("yes");
    let unit_server = unit_active(session, "nanolink-server").await;
    if unit_server {
        s.has_server = true;
    }
    // Live REST is the most authoritative "server is here AND running".
    s.http_port = DEFAULT_HTTP_PORT;
    let health = run(
        session,
        &format!(
            "curl -fsS -m 3 http://localhost:{}/api/health >/dev/null 2>&1 && echo yes",
            DEFAULT_HTTP_PORT
        ),
    )
    .await
    .1
    .contains("yes");
    if health {
        s.has_server = true;
    }
    s.server_running = unit_server || health;
    if s.has_server {
        s.server_version = version_of(session, "nanolink-server").await;
        // Pull the real ports + auth flag from the public server-info.
        if let Some(info) = server_info_value(session, s.http_port).await {
            s.http_port = u16_field(&info, "httpPort").unwrap_or(s.http_port);
            s.grpc_port = u16_field(&info, "grpcPort").unwrap_or(0);
            s.ws_port = u16_field(&info, "wsPort").unwrap_or(0);
            s.auth_enabled = info
                .get("authEnabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        }
    } else {
        s.http_port = 0;
    }

    s.installed = s.has_agent || s.has_server;
    s.role = match (s.has_server, s.has_agent) {
        (true, true) => "both",
        (true, false) => "server",
        (false, true) => "client",
        (false, false) => "none",
    }
    .to_string();

    log::info!(
        "nanolink status: role={} agent_running={} server_running={}",
        s.role,
        s.agent_running,
        s.server_running
    );
    Ok(s)
}

/// Blocking twin of [`status`].
pub fn status_blocking(session: &SshSession) -> Result<NanoLinkStatus> {
    crate::ssh::runtime::shared().block_on(status(session))
}

// ─────────────────────────────────────────────────────────
// Agent (client) control — SSH exec of the CLI / systemctl
// ─────────────────────────────────────────────────────────

/// Human-readable agent status text for the panel (raw, not parsed):
/// prefers `nanolink-agent status`, falls back to `systemctl status`.
pub async fn agent_status_text(session: &SshSession) -> Result<String> {
    let (_, out) = run(
        session,
        "nanolink-agent status 2>&1 || (systemctl status nanolink-agent --no-pager 2>&1 | head -n 40)",
    )
    .await;
    Ok(out)
}

/// Blocking twin of [`agent_status_text`].
pub fn agent_status_text_blocking(session: &SshSession) -> Result<String> {
    crate::ssh::runtime::shared().block_on(agent_status_text(session))
}

/// One server upstream the agent is configured to dial, parsed from
/// `nanolink-agent server list`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentServer {
    /// Server host (no port).
    pub host: String,
    /// Server gRPC port.
    pub port: u16,
    /// Permission level granted on this link (0–3).
    pub permission: u8,
    /// Human name for `permission` (e.g. `SERVICE_CONTROL`).
    pub permission_name: String,
    /// Whether TLS is enabled on the link.
    pub tls_enabled: bool,
    /// Whether the server certificate is verified.
    pub tls_verify: bool,
}

/// List the agent's configured server upstreams as structured rows
/// (`nanolink-agent server list`). An empty result means the output had
/// no parseable rows (e.g. `(none)`), so the panel can fall back to text.
pub async fn agent_servers(session: &SshSession) -> Result<Vec<AgentServer>> {
    let (_, out) = run(session, "nanolink-agent server list 2>&1").await;
    Ok(parse_agent_servers(&out))
}

/// Blocking twin of [`agent_servers`].
pub fn agent_servers_blocking(session: &SshSession) -> Result<Vec<AgentServer>> {
    crate::ssh::runtime::shared().block_on(agent_servers(session))
}

/// Parse `nanolink-agent server list` output. Upstream format, per server:
///   `  {i}. {host}:{port}`
///   `     Permission: {N} ({NAME})`
///   `     TLS: {bool}, Verify: {bool}`
/// Lines that don't match are ignored.
fn parse_agent_servers(text: &str) -> Vec<AgentServer> {
    let mut servers: Vec<AgentServer> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        // "{i}. {host}:{port}"
        if let Some((idx, rest)) = line.split_once(". ") {
            if !idx.is_empty() && idx.bytes().all(|b| b.is_ascii_digit()) {
                if let Some((host, port)) = rest.rsplit_once(':') {
                    if let Ok(p) = port.trim().parse::<u16>() {
                        servers.push(AgentServer {
                            host: host.trim().to_string(),
                            port: p,
                            ..Default::default()
                        });
                        continue;
                    }
                }
            }
        }
        if let Some(rest) = line.strip_prefix("Permission:") {
            if let Some(srv) = servers.last_mut() {
                let rest = rest.trim();
                let (num, name) = match rest.split_once(' ') {
                    Some((n, r)) => (n, r.trim().trim_start_matches('(').trim_end_matches(')')),
                    None => (rest, ""),
                };
                srv.permission = num.trim().parse().unwrap_or(0);
                srv.permission_name = name.to_string();
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("TLS:") {
            if let Some(srv) = servers.last_mut() {
                let mut parts = rest.splitn(2, ',');
                srv.tls_enabled = parts
                    .next()
                    .map(|s| s.trim().eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                if let Some(v) = parts.next() {
                    let vv = v.trim().trim_start_matches("Verify:").trim();
                    srv.tls_verify = vv.eq_ignore_ascii_case("true");
                }
            }
        }
    }
    servers
}

/// `start` | `stop` | `restart` the agent service (needs sudo).
pub async fn agent_service_action(session: &SshSession, action: &str) -> Result<CommandReport> {
    let act = match action {
        "start" | "stop" | "restart" => action,
        _ => return Err(SshError::InvalidConfig(format!("无效的服务操作: {action}"))),
    };
    // systemd by default; on OpenWRT (procd) drive the init.d script.
    let cmd = format!(
        "if command -v systemctl >/dev/null 2>&1; then systemctl {act} nanolink-agent 2>&1; \
         elif [ -x /etc/init.d/nanolink-agent ]; then /etc/init.d/nanolink-agent {act} 2>&1; \
         else echo '找不到 systemctl 或 /etc/init.d/nanolink-agent'; false; fi; echo __rc=$?"
    );
    let (_, out) = session.exec_with_sudo(&cmd).await?;
    Ok(rc_report(&out, None))
}

/// Blocking twin of [`agent_service_action`].
pub fn agent_service_action_blocking(session: &SshSession, action: &str) -> Result<CommandReport> {
    crate::ssh::runtime::shared().block_on(agent_service_action(session, action))
}

/// Add a server upstream to the agent (`nanolink-agent server add`).
/// `host`/`token` are guarded to a safe token charset and the token is
/// scrubbed from the returned output. Needs sudo (writes config).
pub async fn agent_add_server(
    session: &SshSession,
    host: &str,
    port: u16,
    token: &str,
    permission: u8,
    no_tls: bool,
) -> Result<CommandReport> {
    guard_host(host)?;
    guard_token(token)?;
    if permission > 3 {
        return Err(SshError::InvalidConfig("权限级别必须是 0-3".into()));
    }
    // The agent CLI has no `--no-tls`; TLS is `--tls-verify <bool>`
    // (passing `--no-tls` would be a clap "unexpected argument" error).
    let cmd = format!(
        "nanolink-agent server add --host {} --port {} --token {} --permission {} --tls-verify {} \
         2>&1; echo __rc=$?",
        sq(host),
        port,
        sq(token),
        permission,
        if no_tls { "false" } else { "true" },
    );
    let (_, out) = session.exec_with_sudo(&cmd).await?;
    Ok(rc_report(&out, Some(token)))
}

/// Blocking twin of [`agent_add_server`].
pub fn agent_add_server_blocking(
    session: &SshSession,
    host: &str,
    port: u16,
    token: &str,
    permission: u8,
    no_tls: bool,
) -> Result<CommandReport> {
    crate::ssh::runtime::shared().block_on(agent_add_server(
        session, host, port, token, permission, no_tls,
    ))
}

/// Remove a server upstream (`nanolink-agent server remove`). Needs sudo.
pub async fn agent_remove_server(
    session: &SshSession,
    host: &str,
    port: u16,
) -> Result<CommandReport> {
    guard_host(host)?;
    let cmd = format!(
        "nanolink-agent server remove --host {} --port {} 2>&1; echo __rc=$?",
        sq(host),
        port,
    );
    let (_, out) = session.exec_with_sudo(&cmd).await?;
    Ok(rc_report(&out, None))
}

/// Blocking twin of [`agent_remove_server`].
pub fn agent_remove_server_blocking(
    session: &SshSession,
    host: &str,
    port: u16,
) -> Result<CommandReport> {
    crate::ssh::runtime::shared().block_on(agent_remove_server(session, host, port))
}

// ─────────────────────────────────────────────────────────
// Server (collector) query — `curl localhost` over SSH
// ─────────────────────────────────────────────────────────

/// Log in to the server's REST API and return a bearer JWT. Username /
/// password are JSON-encoded then single-quoted for the shell so they
/// can't break out. Errors when no token is found in the response.
pub async fn server_login(
    session: &SshSession,
    port: u16,
    username: &str,
    password: &str,
) -> Result<String> {
    let body = serde_json::json!({ "username": username, "password": password }).to_string();
    // NanoLink's login returns the JWT in an HttpOnly cookie
    // `nanolink_session` (Set-Cookie header), NOT in the JSON body — so
    // capture response headers with `-i` and read the cookie out. The
    // bearer header we send on later calls is still accepted by the
    // server's middleware.
    let cmd = format!(
        "curl -sS -i -m 8 -X POST http://localhost:{port}/api/auth/login \
         -H 'Content-Type: application/json' -d {} 2>&1",
        sq(&body),
    );
    let (_, out) = run(session, &cmd).await;
    if let Some(jwt) = find_session_cookie(&out) {
        return Ok(jwt);
    }
    // No cookie → surface the server's error (last non-empty body line).
    let msg = scrub(&out, Some(password));
    let tail = msg
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    Err(SshError::InvalidConfig(format!(
        "登录失败（未拿到 nanolink_session）: {tail}"
    )))
}

/// Blocking twin of [`server_login`].
pub fn server_login_blocking(
    session: &SshSession,
    port: u16,
    username: &str,
    password: &str,
) -> Result<String> {
    crate::ssh::runtime::shared().block_on(server_login(session, port, username, password))
}

/// Fetch the cluster summary (`/api/summary`). `jwt` may be empty when
/// the server has auth disabled.
pub async fn server_summary(session: &SshSession, port: u16, jwt: &str) -> Result<ServerSummary> {
    let v = server_get_json(session, port, "/api/summary", jwt).await?;
    // Some builds wrap the payload under `data`.
    let v = v.get("data").cloned().unwrap_or(v);
    serde_json::from_value(v)
        .map_err(|e| SshError::InvalidConfig(format!("解析 summary 失败: {e}")))
}

/// Blocking twin of [`server_summary`].
pub fn server_summary_blocking(
    session: &SshSession,
    port: u16,
    jwt: &str,
) -> Result<ServerSummary> {
    crate::ssh::runtime::shared().block_on(server_summary(session, port, jwt))
}

/// Fetch the agent list (`/api/agents`). Tolerates either a bare array
/// or an envelope (`{agents:[…]}` / `{data:[…]}`).
pub async fn server_agents(session: &SshSession, port: u16, jwt: &str) -> Result<Vec<ServerAgent>> {
    let v = server_get_json(session, port, "/api/agents", jwt).await?;
    let arr = if v.is_array() {
        v
    } else {
        v.get("agents")
            .or_else(|| v.get("data"))
            .or_else(|| v.get("items"))
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]))
    };
    let mut agents: Vec<ServerAgent> = serde_json::from_value(arr)
        .map_err(|e| SshError::InvalidConfig(format!("解析 agents 失败: {e}")))?;
    // The server exposes no per-agent `online` flag — any agent returned by
    // /api/agents is currently connected, so mark them all online rather
    // than letting serde default the missing field to false.
    for a in &mut agents {
        a.online = true;
    }
    Ok(agents)
}

/// Blocking twin of [`server_agents`].
pub fn server_agents_blocking(
    session: &SshSession,
    port: u16,
    jwt: &str,
) -> Result<Vec<ServerAgent>> {
    crate::ssh::runtime::shared().block_on(server_agents(session, port, jwt))
}

// ─────────────────────────────────────────────────────────
// Server: provision new agents + dispatch commands
// ─────────────────────────────────────────────────────────

/// Result of `/api/config/generate` — the agent config plus ready-to-run
/// per-platform install commands.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GenerateConfigResult {
    /// The full agent YAML config.
    pub config_yaml: String,
    /// One-line Linux/macOS install command (curl … | bash).
    pub install_command_unix: String,
    /// One-line Windows install command (PowerShell).
    pub install_command_windows: String,
    /// Token the server minted when none was supplied (else empty).
    pub generated_token: String,
    /// Short hash identifying this server endpoint.
    pub server_id: String,
}

/// Ask the server to mint a config + install command for a new agent
/// (`POST /api/config/generate`, **admin-only**). `token` empty → the
/// server generates and persists one and returns it in `generated_token`.
pub async fn server_generate_config(
    session: &SshSession,
    port: u16,
    jwt: &str,
    server_url: &str,
    token: &str,
    permission: u8,
    tls_verify: bool,
    hostname: &str,
) -> Result<GenerateConfigResult> {
    if permission > 3 {
        return Err(SshError::InvalidConfig("权限级别必须是 0-3".into()));
    }
    let body = serde_json::json!({
        "serverUrl": server_url,
        "token": token,
        "permission": permission,
        "tlsVerify": tls_verify,
        "hostname": hostname,
    })
    .to_string();
    let (code, out) = server_curl(
        session,
        "POST",
        port,
        "/api/config/generate",
        jwt,
        Some(&body),
    )
    .await?;
    if code == 200 {
        serde_json::from_str(&out)
            .map_err(|e| SshError::InvalidConfig(format!("解析配置生成响应失败: {e}")))
    } else {
        Err(SshError::InvalidConfig(curl_error(code, &out)))
    }
}

/// Blocking twin of [`server_generate_config`].
#[allow(clippy::too_many_arguments)]
pub fn server_generate_config_blocking(
    session: &SshSession,
    port: u16,
    jwt: &str,
    server_url: &str,
    token: &str,
    permission: u8,
    tls_verify: bool,
    hostname: &str,
) -> Result<GenerateConfigResult> {
    crate::ssh::runtime::shared().block_on(server_generate_config(
        session, port, jwt, server_url, token, permission, tls_verify, hostname,
    ))
}

/// Outcome of dispatching a command to an agent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CommandDispatch {
    /// `sent` on success.
    pub status: String,
    /// Id to poll for the structured result.
    pub command_id: String,
}

/// Dispatch a command to a connected agent
/// (`POST /api/agents/:id/command`). `cmd_type` is a NanoLink CommandType
/// enum name (e.g. `SERVICE_RESTART`, `PROCESS_LIST`); `target` is the
/// command's subject (a service/process name) or empty.
pub async fn server_send_command(
    session: &SshSession,
    port: u16,
    jwt: &str,
    agent_id: &str,
    cmd_type: &str,
    target: &str,
) -> Result<CommandDispatch> {
    guard_id(agent_id)?;
    guard_cmd_type(cmd_type)?;
    let body = serde_json::json!({ "type": cmd_type, "target": target, "params": {} }).to_string();
    let path = format!("/api/agents/{}/command", agent_id);
    let (code, out) = server_curl(session, "POST", port, &path, jwt, Some(&body)).await?;
    if code == 200 {
        serde_json::from_str(&out)
            .map_err(|e| SshError::InvalidConfig(format!("解析命令响应失败: {e}")))
    } else {
        Err(SshError::InvalidConfig(curl_error(code, &out)))
    }
}

/// Blocking twin of [`server_send_command`].
pub fn server_send_command_blocking(
    session: &SshSession,
    port: u16,
    jwt: &str,
    agent_id: &str,
    cmd_type: &str,
    target: &str,
) -> Result<CommandDispatch> {
    crate::ssh::runtime::shared().block_on(server_send_command(
        session, port, jwt, agent_id, cmd_type, target,
    ))
}

/// One poll of a dispatched command's result
/// (`GET /api/agents/:id/command/:commandId/result`). `status` is
/// `pending` (HTTP 202) or `done` (HTTP 200); `json` carries the raw
/// structured result body once done.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CommandResult {
    /// `pending` | `done`.
    pub status: String,
    /// Raw result JSON when `status == "done"`, else empty.
    pub json: String,
}

/// Poll a dispatched command's result once. See [`CommandResult`].
pub async fn server_command_result(
    session: &SshSession,
    port: u16,
    jwt: &str,
    agent_id: &str,
    command_id: &str,
) -> Result<CommandResult> {
    guard_id(agent_id)?;
    guard_id(command_id)?;
    let path = format!("/api/agents/{}/command/{}/result", agent_id, command_id);
    let (code, out) = server_curl(session, "GET", port, &path, jwt, None).await?;
    match code {
        202 => Ok(CommandResult {
            status: "pending".into(),
            json: String::new(),
        }),
        200 => Ok(CommandResult {
            status: "done".into(),
            json: out,
        }),
        _ => Err(SshError::InvalidConfig(curl_error(code, &out))),
    }
}

/// Blocking twin of [`server_command_result`].
pub fn server_command_result_blocking(
    session: &SshSession,
    port: u16,
    jwt: &str,
    agent_id: &str,
    command_id: &str,
) -> Result<CommandResult> {
    crate::ssh::runtime::shared().block_on(server_command_result(
        session, port, jwt, agent_id, command_id,
    ))
}

// ─────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────

/// Run a read-only command over SSH (no sudo), swallowing transport
/// errors into `(-1, "")` the way the detector does.
async fn run(session: &SshSession, cmd: &str) -> (i32, String) {
    session
        .exec_command(cmd)
        .await
        .unwrap_or((-1, String::new()))
}

/// True when a service is running. systemd first; on OpenWRT (procd) fall
/// back to the init.d `running` action, then to `pgrep`, so a procd-managed
/// agent still reads as running.
async fn unit_active(session: &SshSession, unit: &str) -> bool {
    let cmd = format!(
        "if command -v systemctl >/dev/null 2>&1; then systemctl is-active {unit} >/dev/null 2>&1 && echo yes; \
         elif [ -x /etc/init.d/{unit} ]; then /etc/init.d/{unit} running >/dev/null 2>&1 && echo yes; \
         else pgrep -f {unit} >/dev/null 2>&1 && echo yes; fi"
    );
    run(session, &cmd).await.1.contains("yes")
}

/// Extract a version token from `<bin> --version`. Empty if the binary
/// has no such flag or isn't there.
async fn version_of(session: &SshSession, bin: &str) -> String {
    let (code, out) = run(session, &format!("{bin} --version 2>/dev/null")).await;
    if code != 0 {
        return String::new();
    }
    for word in out.split_whitespace() {
        let t = word.trim_start_matches('v').trim_end_matches([',', ';']);
        if t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains('.') {
            return t.to_string();
        }
    }
    String::new()
}

/// GET `/api/server-info` (public) and parse it as JSON.
async fn server_info_value(session: &SshSession, port: u16) -> Option<serde_json::Value> {
    let cmd = format!("curl -fsS -m 3 http://localhost:{port}/api/server-info 2>/dev/null");
    let (code, out) = run(session, &cmd).await;
    if code != 0 {
        return None;
    }
    serde_json::from_str(&out).ok()
}

/// GET a server endpoint with an optional bearer token, returning parsed
/// JSON. The token is single-quoted; the URL/path are static.
async fn server_get_json(
    session: &SshSession,
    port: u16,
    path: &str,
    jwt: &str,
) -> Result<serde_json::Value> {
    let auth = if jwt.is_empty() {
        String::new()
    } else {
        guard_token(jwt)?;
        format!("-H 'Authorization: Bearer {}' ", jwt)
    };
    let cmd = format!(
        "curl -fsS -m 8 {auth}http://localhost:{port}{path} 2>&1",
        auth = auth,
        port = port,
        path = path,
    );
    let (code, out) = run(session, &cmd).await;
    if code != 0 {
        return Err(SshError::InvalidConfig(format!(
            "请求 {path} 失败: {}",
            out.lines().last().unwrap_or("").trim()
        )));
    }
    serde_json::from_str(&out)
        .map_err(|e| SshError::InvalidConfig(format!("解析 {path} 响应失败: {e}")))
}

/// `curl` an endpoint capturing the HTTP status code via `-w`. Returns
/// `(code, body)`. The bearer header, the JSON `body` (serde-escaped),
/// and the URL are all single-quoted, so no value can break the shell.
async fn server_curl(
    session: &SshSession,
    method: &str,
    port: u16,
    path: &str,
    jwt: &str,
    body: Option<&str>,
) -> Result<(u16, String)> {
    let auth = if jwt.is_empty() {
        String::new()
    } else {
        guard_token(jwt)?;
        format!("-H 'Authorization: Bearer {}' ", jwt)
    };
    let data = match body {
        Some(b) => format!("-H 'Content-Type: application/json' -d {} ", sq(b)),
        None => String::new(),
    };
    let url = format!("http://localhost:{port}{path}");
    let cmd = format!(
        "curl -sS -m 10 -X {method} {auth}{data}-w '\\n%{{http_code}}' {url} 2>&1",
        method = method,
        auth = auth,
        data = data,
        url = sq(&url),
    );
    let (_, out) = run(session, &cmd).await;
    Ok(split_http_code(&out))
}

/// Split `curl -w '\n%{http_code}'` output into `(code, body)`. Falls
/// back to `(0, whole-output)` when no trailing code is present (e.g. a
/// transport error printed by `curl -sS`).
fn split_http_code(out: &str) -> (u16, String) {
    let trimmed = out.trim_end_matches('\n');
    match trimmed.rsplit_once('\n') {
        Some((body, code)) => (code.trim().parse().unwrap_or(0), body.to_string()),
        None => trimmed
            .trim()
            .parse()
            .map(|c| (c, String::new()))
            .unwrap_or((0, trimmed.to_string())),
    }
}

/// Build a human-readable error from a non-2xx curl result, pulling the
/// gin `{"error":…}` message out of the body when present.
fn curl_error(code: u16, body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(msg) = v.get("error").and_then(|m| m.as_str()) {
            return format!("HTTP {code}: {msg}");
        }
    }
    format!("HTTP {code}: {}", body.lines().last().unwrap_or("").trim())
}

/// Allow ids/uuids/hostnames that go into a URL path — rejects shell and
/// URL metacharacters (spaces, `/`, quotes, `?`, `#`, …).
fn guard_id(s: &str) -> Result<()> {
    let ok = !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_' || b == b':');
    if ok {
        Ok(())
    } else {
        Err(SshError::InvalidConfig("非法 id".into()))
    }
}

/// CommandType enum names are `A-Z` + `_` only.
fn guard_cmd_type(s: &str) -> Result<()> {
    let ok = !s.is_empty() && s.bytes().all(|b| b.is_ascii_uppercase() || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(SshError::InvalidConfig("非法命令类型".into()))
    }
}

/// Read a u16 port from a JSON object field (number or numeric string).
fn u16_field(v: &serde_json::Value, key: &str) -> Option<u16> {
    let f = v.get(key)?;
    f.as_u64()
        .or_else(|| f.as_str().and_then(|s| s.parse::<u64>().ok()))
        .map(|n| n as u16)
}

/// Pull the `nanolink_session` cookie value out of `curl -i` response
/// headers. The cookie name is a fixed lowercase ASCII literal, so a
/// plain substring scan over each line is enough (header-name case and
/// other Set-Cookie attributes don't matter).
fn find_session_cookie(raw: &str) -> Option<String> {
    const KEY: &str = "nanolink_session=";
    for line in raw.lines() {
        if let Some(idx) = line.find(KEY) {
            let val = line[idx + KEY.len()..]
                .split(';')
                .next()
                .unwrap_or("")
                .trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Find a `token` string anywhere in a (possibly nested) JSON response.
#[allow(dead_code)]
fn find_token(raw: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    fn walk(v: &serde_json::Value) -> Option<String> {
        match v {
            serde_json::Value::Object(map) => {
                for k in ["token", "accessToken", "jwt"] {
                    if let Some(serde_json::Value::String(s)) = map.get(k) {
                        if !s.is_empty() {
                            return Some(s.clone());
                        }
                    }
                }
                map.values().find_map(walk)
            }
            _ => None,
        }
    }
    walk(&v)
}

/// Build a [`CommandReport`] from output ending in `__rc=N`, scrubbing
/// `secret` if present.
fn rc_report(out: &str, secret: Option<&str>) -> CommandReport {
    let mut exit_code = -1;
    let mut body_lines: Vec<&str> = Vec::new();
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("__rc=") {
            exit_code = rest.trim().parse::<i32>().unwrap_or(-1);
        } else {
            body_lines.push(line);
        }
    }
    let tail = body_lines
        .iter()
        .rev()
        .take(30)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let output = scrub(&tail, secret);
    let status = if exit_code == 0 {
        "ok"
    } else if looks_like_sudo_prompt(&output) {
        "sudo-requires-password"
    } else {
        "failed"
    };
    CommandReport {
        status: status.to_string(),
        exit_code,
        output,
    }
}

/// Replace any occurrence of `secret` in `text` with `***`.
fn scrub(text: &str, secret: Option<&str>) -> String {
    match secret {
        Some(s) if !s.is_empty() => text.replace(s, "***"),
        _ => text.to_string(),
    }
}

/// Heuristic: did the output indicate sudo wanted a password?
fn looks_like_sudo_prompt(out: &str) -> bool {
    let l = out.to_lowercase();
    l.contains("password is required")
        || l.contains("a password is required")
        || l.contains("sudo: a terminal is required")
        || l.contains("incorrect password attempt")
}

/// Single-quote a value for POSIX `sh`.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Allow only host-name characters (letters, digits, `.`/`-`). Rejects
/// shell metacharacters, whitespace, and `:` (the port is separate).
fn guard_host(s: &str) -> Result<()> {
    let ok = !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(SshError::InvalidConfig(
            "主机名非法（仅允许字母、数字、. -）".into(),
        ))
    }
}

/// Allow only token characters (letters, digits, `. _ -`). Used for both
/// agent tokens and the JWT we forward to `curl`.
fn guard_token(s: &str) -> Result<()> {
    let ok = !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.');
    if ok {
        Ok(())
    } else {
        Err(SshError::InvalidConfig(
            "token 非法（仅允许字母、数字、. _ -）".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_classification_roundtrip() {
        // The wire shape must stay camelCase for the frontend.
        let s = NanoLinkStatus {
            installed: true,
            has_agent: true,
            has_server: true,
            role: "both".into(),
            http_port: 8080,
            ..Default::default()
        };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"hasAgent\":true"), "{j}");
        assert!(j.contains("\"httpPort\":8080"), "{j}");
    }

    #[test]
    fn find_token_nested() {
        assert_eq!(
            find_token(r#"{"data":{"token":"abc.def"}}"#).as_deref(),
            Some("abc.def")
        );
        assert_eq!(
            find_token(r#"{"accessToken":"zzz"}"#).as_deref(),
            Some("zzz")
        );
        assert_eq!(find_token(r#"{"nope":1}"#), None);
    }

    #[test]
    fn find_session_cookie_from_headers() {
        let headers = "HTTP/1.1 200 OK\r\n\
            Content-Type: application/json\r\n\
            Set-Cookie: nanolink_session=eyJhbGci.payload.sig; Path=/; HttpOnly; SameSite=Lax\r\n\
            \r\n\
            {\"user\":{\"username\":\"admin\"}}";
        assert_eq!(
            find_session_cookie(headers).as_deref(),
            Some("eyJhbGci.payload.sig")
        );
        // No cookie (e.g. 401 body) → None.
        assert_eq!(
            find_session_cookie("HTTP/1.1 401\r\n\r\n{\"error\":\"bad creds\"}"),
            None
        );
    }

    #[test]
    fn rc_report_parses_exit_and_scrubs() {
        let r = rc_report("added ok\nsecret-tok used\n__rc=0", Some("secret-tok"));
        assert_eq!(r.status, "ok");
        assert_eq!(r.exit_code, 0);
        assert!(r.output.contains("***"), "{}", r.output);
        assert!(!r.output.contains("secret-tok"));
    }

    #[test]
    fn rc_report_failed() {
        let r = rc_report("boom\n__rc=1", None);
        assert_eq!(r.status, "failed");
        assert_eq!(r.exit_code, 1);
    }

    #[test]
    fn guards_reject_metacharacters() {
        assert!(guard_host("1.2.3.4").is_ok());
        assert!(guard_host("srv-01.example.com").is_ok());
        assert!(guard_host("a;rm -rf /").is_err());
        assert!(guard_host("host:9200").is_err());
        assert!(guard_token("abc.DEF_123-x").is_ok());
        assert!(guard_token("a b").is_err());
        assert!(guard_token("a'b").is_err());
    }

    #[test]
    fn u16_field_handles_number_and_string() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"httpPort":8080,"grpcPort":"9200"}"#).unwrap();
        assert_eq!(u16_field(&v, "httpPort"), Some(8080));
        assert_eq!(u16_field(&v, "grpcPort"), Some(9200));
        assert_eq!(u16_field(&v, "missing"), None);
    }

    #[test]
    fn split_http_code_parses_trailing_code() {
        assert_eq!(
            split_http_code("{\"ok\":true}\n200"),
            (200, "{\"ok\":true}".to_string())
        );
        assert_eq!(split_http_code("\n202"), (202, "".to_string()));
        // Transport error with no code → (0, body).
        assert_eq!(
            split_http_code("curl: (7) connection refused"),
            (0, "curl: (7) connection refused".to_string())
        );
    }

    #[test]
    fn curl_error_pulls_gin_message() {
        assert_eq!(
            curl_error(403, r#"{"error":"insufficient permissions"}"#),
            "HTTP 403: insufficient permissions"
        );
        assert_eq!(curl_error(500, "boom"), "HTTP 500: boom");
    }

    #[test]
    fn parse_agent_servers_format() {
        let out = "Configured servers:\n  1. 192.168.1.10:39100\n     Permission: 2 (SERVICE_CONTROL)\n     TLS: true, Verify: false\n  2. host.example.com:9200\n     Permission: 0 (READ_ONLY)\n     TLS: false, Verify: false\n";
        let s = parse_agent_servers(out);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].host, "192.168.1.10");
        assert_eq!(s[0].port, 39100);
        assert_eq!(s[0].permission, 2);
        assert_eq!(s[0].permission_name, "SERVICE_CONTROL");
        assert!(s[0].tls_enabled);
        assert!(!s[0].tls_verify);
        assert_eq!(s[1].host, "host.example.com");
        assert_eq!(s[1].port, 9200);
        assert_eq!(s[1].permission, 0);
        assert!(!s[1].tls_enabled);
        // No parseable rows → empty (panel falls back to text).
        assert!(parse_agent_servers("Configured servers:\n  (none)").is_empty());
    }

    #[test]
    fn id_and_cmd_type_guards() {
        assert!(guard_id("a1b2-c3d4_e5.f6:7").is_ok());
        assert!(guard_id("../etc/passwd").is_err());
        assert!(guard_id("a b").is_err());
        assert!(guard_cmd_type("SERVICE_RESTART").is_ok());
        assert!(guard_cmd_type("service_restart").is_err());
        assert!(guard_cmd_type("RM -RF").is_err());
    }
}
