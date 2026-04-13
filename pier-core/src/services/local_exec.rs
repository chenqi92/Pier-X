//! Local command execution — run commands on the local machine.
//!
//! Used by right-panel tools when no SSH session is available.
//! Docker, monitoring, and log viewing can all work locally.

use std::process::Command;

use serde::{Deserialize, Serialize};

/// Run a local command and return (exit_code, stdout).
pub fn exec(cmd: &str) -> Result<(i32, String), String> {
    let shell = if cfg!(target_os = "windows") {
        "cmd"
    } else {
        "sh"
    };
    let flag = if cfg!(target_os = "windows") {
        "/C"
    } else {
        "-c"
    };

    let output = Command::new(shell)
        .arg(flag)
        .arg(cmd)
        .output()
        .map_err(|e| format!("failed to run '{cmd}': {e}"))?;

    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok((code, stdout))
}

/// Local Docker: list containers.
pub fn docker_list_containers(all: bool) -> Result<String, String> {
    let cmd = if all {
        "docker ps --all --no-trunc --format '{{json .}}'"
    } else {
        "docker ps --no-trunc --format '{{json .}}'"
    };
    let (code, stdout) = exec(cmd)?;
    if code != 0 {
        return Err(format!("docker ps exited {code}"));
    }
    Ok(stdout)
}

/// Local Docker: list images.
pub fn docker_list_images() -> Result<String, String> {
    let (code, stdout) = exec("docker images --format '{{json .}}'")?;
    if code != 0 {
        return Err(format!("docker images exited {code}"));
    }
    Ok(stdout)
}

/// Local Docker: list volumes.
pub fn docker_list_volumes() -> Result<String, String> {
    let (code, stdout) = exec("docker volume ls --format '{{json .}}'")?;
    if code != 0 {
        return Err(format!("docker volume ls exited {code}"));
    }
    Ok(stdout)
}

/// Local Docker: list networks.
pub fn docker_list_networks() -> Result<String, String> {
    let (code, stdout) = exec("docker network ls --format '{{json .}}'")?;
    if code != 0 {
        return Err(format!("docker network ls exited {code}"));
    }
    Ok(stdout)
}

/// Local Docker: run `docker <args...>` and return
/// `(exit_code, stdout)` without forcing a success path.
pub fn docker_exec(args: &[String]) -> Result<(i32, String), String> {
    let tail = crate::services::docker::join_shell_args(args.iter().map(String::as_str));
    let cmd = if tail.is_empty() {
        String::from("docker")
    } else {
        format!("docker {tail}")
    };
    exec(&cmd)
}

/// Local Docker: simple action (start/stop/restart/rm).
pub fn docker_action(verb: &str, id: &str, force: bool) -> Result<(), String> {
    if !crate::services::docker::is_safe_id(id) {
        return Err(format!("unsafe id: {id}"));
    }
    let cmd = if force {
        format!("docker {verb} --force {id}")
    } else {
        format!("docker {verb} {id}")
    };
    let (code, stdout) = exec(&cmd)?;
    if code != 0 {
        return Err(format!(
            "docker {verb} exited {code}: {}",
            stdout.lines().next().unwrap_or("")
        ));
    }
    Ok(())
}

/// Local Docker: inspect container.
pub fn docker_inspect(id: &str) -> Result<String, String> {
    if !crate::services::docker::is_safe_id(id) {
        return Err(format!("unsafe id: {id}"));
    }
    let (code, stdout) = exec(&format!("docker inspect --type container {id}"))?;
    if code != 0 {
        return Err(format!("docker inspect exited {code}"));
    }
    Ok(stdout)
}

/// Local system metrics.
#[allow(missing_docs)]
#[derive(Serialize, Deserialize)]
pub struct LocalMetrics {
    pub hostname: String,
    pub os: String,
    pub uptime: String,
    pub cpu_percent: f64,
    pub memory_total_mb: u64,
    pub memory_used_mb: u64,
    pub load_1: f64,
    pub load_5: f64,
    pub load_15: f64,
}

/// Get local system metrics.
pub fn system_metrics() -> Result<LocalMetrics, String> {
    let hostname = exec("hostname")
        .map(|(_, s)| s.trim().to_string())
        .unwrap_or_default();

    let os = if cfg!(target_os = "macos") {
        exec("sw_vers -productVersion")
            .map(|(_, s)| format!("macOS {}", s.trim()))
            .unwrap_or_else(|_| "macOS".into())
    } else if cfg!(target_os = "linux") {
        exec("uname -r")
            .map(|(_, s)| format!("Linux {}", s.trim()))
            .unwrap_or_else(|_| "Linux".into())
    } else {
        "Unknown".into()
    };

    let uptime = exec("uptime")
        .map(|(_, s)| s.trim().to_string())
        .unwrap_or_default();

    // Load averages
    let (load_1, load_5, load_15) = if cfg!(target_os = "macos") {
        exec("sysctl -n vm.loadavg")
            .map(|(_, s)| {
                let parts: Vec<f64> = s
                    .trim()
                    .trim_matches(|c| c == '{' || c == '}')
                    .split_whitespace()
                    .filter_map(|v| v.parse().ok())
                    .collect();
                (
                    parts.get(0).copied().unwrap_or(0.0),
                    parts.get(1).copied().unwrap_or(0.0),
                    parts.get(2).copied().unwrap_or(0.0),
                )
            })
            .unwrap_or((0.0, 0.0, 0.0))
    } else {
        exec("cat /proc/loadavg")
            .map(|(_, s)| {
                let parts: Vec<f64> = s
                    .split_whitespace()
                    .take(3)
                    .filter_map(|v| v.parse().ok())
                    .collect();
                (
                    parts.get(0).copied().unwrap_or(0.0),
                    parts.get(1).copied().unwrap_or(0.0),
                    parts.get(2).copied().unwrap_or(0.0),
                )
            })
            .unwrap_or((0.0, 0.0, 0.0))
    };

    // Memory
    let (mem_total, mem_used) = if cfg!(target_os = "macos") {
        let total = exec("sysctl -n hw.memsize")
            .map(|(_, s)| s.trim().parse::<u64>().unwrap_or(0) / 1024 / 1024)
            .unwrap_or(0);
        // vm_stat gives pages; page size is typically 16384 on ARM, 4096 on Intel
        let page_size = exec("sysctl -n vm.pagesize")
            .map(|(_, s)| s.trim().parse::<u64>().unwrap_or(16384))
            .unwrap_or(16384);
        let pages_active = exec("vm_stat")
            .map(|(_, s)| {
                let mut active = 0u64;
                for line in s.lines() {
                    if line.contains("Pages active") || line.contains("Pages wired") {
                        if let Some(v) = line.split(':').nth(1) {
                            active += v.trim().trim_end_matches('.').parse::<u64>().unwrap_or(0);
                        }
                    }
                }
                active
            })
            .unwrap_or(0);
        (total, pages_active * page_size / 1024 / 1024)
    } else {
        // Linux: /proc/meminfo
        exec("cat /proc/meminfo")
            .map(|(_, s)| {
                let mut total = 0u64;
                let mut available = 0u64;
                for line in s.lines() {
                    if line.starts_with("MemTotal:") {
                        total = line
                            .split_whitespace()
                            .nth(1)
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0)
                            / 1024;
                    } else if line.starts_with("MemAvailable:") {
                        available = line
                            .split_whitespace()
                            .nth(1)
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0)
                            / 1024;
                    }
                }
                (total, total.saturating_sub(available))
            })
            .unwrap_or((0, 0))
    };

    Ok(LocalMetrics {
        hostname,
        os,
        uptime,
        cpu_percent: load_1 * 100.0 / num_cpus().max(1) as f64,
        memory_total_mb: mem_total,
        memory_used_mb: mem_used,
        load_1,
        load_5,
        load_15,
    })
}

fn num_cpus() -> usize {
    exec("nproc")
        .or_else(|_| exec("sysctl -n hw.ncpu"))
        .map(|(_, s)| s.trim().parse().unwrap_or(1))
        .unwrap_or(1)
}
