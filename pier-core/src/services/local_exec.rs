//! Local command execution — run commands on the local machine.
//!
//! Used by right-panel tools when no SSH session is available.
//! Docker, monitoring, and log viewing can all work locally.

use std::process::Command;

use crate::services::server_monitor::ServerSnapshot;
use crate::process_util::configure_background_command;

/// Local system metrics reuse the same schema as the remote
/// SSH monitor so the Qt layer does not need a second parsing path.
pub type LocalMetrics = ServerSnapshot;

struct ProcessOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn finish_command(mut command: Command, description: &str) -> Result<ProcessOutput, String> {
    configure_background_command(&mut command);

    let output = command
        .output()
        .map_err(|e| format!("failed to run {description}: {e}"))?;

    Ok(ProcessOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn run_program(program: &str, args: &[&str]) -> Result<ProcessOutput, String> {
    let mut command = Command::new(program);
    command.args(args);
    finish_command(command, &format!("`{program}`"))
}

fn shell_output(cmd: &str) -> Result<ProcessOutput, String> {
    let command = if cfg!(target_os = "windows") {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(cmd);
        command
    } else {
        let mut command = Command::new("sh");
        command.arg("-c").arg(cmd);
        command
    };
    finish_command(command, &format!("shell command `{cmd}`"))
}

fn first_diagnostic_line(output: &ProcessOutput) -> String {
    output
        .stderr
        .lines()
        .chain(output.stdout.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string()
}

fn combine_streams(output: ProcessOutput) -> String {
    match (output.stdout.trim().is_empty(), output.stderr.trim().is_empty()) {
        (false, true) => output.stdout,
        (true, false) => output.stderr,
        (false, false) => format!("{}\n{}", output.stdout.trim_end(), output.stderr.trim_end()),
        (true, true) => String::new(),
    }
}

fn successful_stdout(output: ProcessOutput, label: &str) -> Result<String, String> {
    if output.code != 0 {
        return Err(format!(
            "{label} exited {}: {}",
            output.code,
            first_diagnostic_line(&output)
        ));
    }
    Ok(output.stdout)
}

fn docker_output(args: &[&str]) -> Result<ProcessOutput, String> {
    run_program("docker", args)
}

/// Run a local command and return `(exit_code, stdout_or_stderr)`.
pub fn exec(cmd: &str) -> Result<(i32, String), String> {
    let output = shell_output(cmd)?;
    let primary = if output.stdout.trim().is_empty() {
        output.stderr.clone()
    } else {
        output.stdout.clone()
    };
    Ok((output.code, primary))
}

/// Local Docker: list containers.
pub fn docker_list_containers(all: bool) -> Result<String, String> {
    if all {
        successful_stdout(
            docker_output(&["ps", "--all", "--no-trunc", "--format", "{{json .}}"])?,
            "docker ps",
        )
    } else {
        successful_stdout(
            docker_output(&["ps", "--no-trunc", "--format", "{{json .}}"])?,
            "docker ps",
        )
    }
}

/// Local Docker: list images.
pub fn docker_list_images() -> Result<String, String> {
    successful_stdout(
        docker_output(&["images", "--format", "{{json .}}"])?,
        "docker images",
    )
}

/// Local Docker: list volumes.
pub fn docker_list_volumes() -> Result<String, String> {
    successful_stdout(
        docker_output(&["volume", "ls", "--format", "{{json .}}"])?,
        "docker volume ls",
    )
}

/// Local Docker: list networks.
pub fn docker_list_networks() -> Result<String, String> {
    successful_stdout(
        docker_output(&["network", "ls", "--format", "{{json .}}"])?,
        "docker network ls",
    )
}

/// Local Docker: run `docker <args...>` and return
/// `(exit_code, stdout+stderr)` without forcing a success path.
pub fn docker_exec(args: &[String]) -> Result<(i32, String), String> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = docker_output(&refs)?;
    Ok((output.code, combine_streams(output)))
}

/// Local Docker: simple action (start/stop/restart/rm).
pub fn docker_action(verb: &str, id: &str, force: bool) -> Result<(), String> {
    if !crate::services::docker::is_safe_id(id) {
        return Err(format!("unsafe id: {id}"));
    }

    let mut args = vec![verb];
    if force {
        args.push("--force");
    }
    args.push(id);

    successful_stdout(docker_output(&args)?, &format!("docker {verb}")).map(|_| ())
}

/// Local Docker: inspect container.
pub fn docker_inspect(id: &str) -> Result<String, String> {
    if !crate::services::docker::is_safe_id(id) {
        return Err(format!("unsafe id: {id}"));
    }
    successful_stdout(
        docker_output(&["inspect", "--type", "container", id])?,
        "docker inspect",
    )
}

/// Get local system metrics.
pub fn system_metrics() -> Result<LocalMetrics, String> {
    #[cfg(target_os = "windows")]
    {
        return system_metrics_windows();
    }

    #[cfg(not(target_os = "windows"))]
    {
        system_metrics_unix()
    }
}

#[cfg(target_os = "windows")]
fn system_metrics_windows() -> Result<LocalMetrics, String> {
    // Probe everything in one hidden PowerShell run so switching to the
    // monitor panel does not flash a half-dozen transient console windows.
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
function Format-Size([double]$bytes) {
    if ($bytes -ge 1TB) { return ('{0:0.0} TB' -f ($bytes / 1TB)) }
    if ($bytes -ge 1GB) { return ('{0:0.0} GB' -f ($bytes / 1GB)) }
    if ($bytes -ge 1MB) { return ('{0:0} MB' -f ($bytes / 1MB)) }
    return ('{0:0} KB' -f ($bytes / 1KB))
}
function Format-Uptime([TimeSpan]$span) {
    $parts = New-Object System.Collections.Generic.List[string]
    if ($span.Days -gt 0) {
        $parts.Add(('{0} day{1}' -f $span.Days, $(if ($span.Days -ne 1) { 's' } else { '' })))
    }
    if ($span.Hours -gt 0) {
        $parts.Add(('{0} hour{1}' -f $span.Hours, $(if ($span.Hours -ne 1) { 's' } else { '' })))
    }
    if ($span.Minutes -gt 0 -and $parts.Count -lt 2) {
        $parts.Add(('{0} min' -f $span.Minutes))
    }
    if ($parts.Count -eq 0) {
        $parts.Add('0 min')
    }
    return 'up ' + ($parts -join ', ')
}

$os = Get-CimInstance Win32_OperatingSystem
$disk = Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' | Sort-Object Size -Descending | Select-Object -First 1
$pageFiles = @(Get-CimInstance Win32_PageFileUsage -ErrorAction SilentlyContinue)

$cpuPct = -1.0
try {
    $cpuPct = [double][math]::Round((Get-Counter '\Processor(_Total)\% Processor Time').CounterSamples[0].CookedValue, 1)
} catch {
    $cpuPct = -1.0
}

$memTotalMb = [double][math]::Round($os.TotalVisibleMemorySize / 1024, 1)
$memFreeMb = [double][math]::Round($os.FreePhysicalMemory / 1024, 1)
$memUsedMb = [double][math]::Round([math]::Max(0, $memTotalMb - $memFreeMb), 1)

$swapTotalMb = -1.0
$swapUsedMb = -1.0
if ($pageFiles.Count -gt 0) {
    $swapTotalMb = [double][math]::Round((($pageFiles | Measure-Object -Property AllocatedBaseSize -Sum).Sum), 1)
    $swapUsedMb = [double][math]::Round((($pageFiles | Measure-Object -Property CurrentUsage -Sum).Sum), 1)
}

$diskTotalBytes = if ($disk) { [double]$disk.Size } else { 0.0 }
$diskFreeBytes = if ($disk) { [double]$disk.FreeSpace } else { 0.0 }
$diskUsedBytes = [double][math]::Max(0, $diskTotalBytes - $diskFreeBytes)
$diskUsePct = if ($diskTotalBytes -gt 0) {
    [double][math]::Round(($diskUsedBytes / $diskTotalBytes) * 100, 1)
} else {
    -1.0
}

$snapshot = @{
    uptime = (Format-Uptime ((Get-Date) - $os.LastBootUpTime))
    load_1 = -1.0
    load_5 = -1.0
    load_15 = -1.0
    mem_total_mb = $memTotalMb
    mem_used_mb = $memUsedMb
    mem_free_mb = $memFreeMb
    swap_total_mb = $swapTotalMb
    swap_used_mb = $swapUsedMb
    disk_total = if ($diskTotalBytes -gt 0) { Format-Size $diskTotalBytes } else { '' }
    disk_used = if ($diskTotalBytes -gt 0) { Format-Size $diskUsedBytes } else { '' }
    disk_avail = if ($diskTotalBytes -gt 0) { Format-Size $diskFreeBytes } else { '' }
    disk_use_pct = $diskUsePct
    cpu_pct = $cpuPct
}

$snapshot | ConvertTo-Json -Compress
"#;

    let output = run_program(
        "powershell.exe",
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            SCRIPT,
        ],
    )?;

    if output.code != 0 {
        return Err(format!(
            "local monitor probe exited {}: {}",
            output.code,
            first_diagnostic_line(&output)
        ));
    }

    serde_json::from_str(output.stdout.trim()).map_err(|e| {
        format!(
            "failed to parse local monitor probe JSON: {e}; raw={}",
            output.stdout.trim()
        )
    })
}

#[cfg(not(target_os = "windows"))]
fn system_metrics_unix() -> Result<LocalMetrics, String> {
    let uptime_output = shell_output("uptime")?;
    let uptime_line = uptime_output
        .stdout
        .lines()
        .last()
        .unwrap_or("")
        .trim()
        .to_string();
    let (uptime, load_1, load_5, load_15) = parse_uptime_line(&uptime_line);

    let (mem_total_mb, mem_used_mb, mem_free_mb, swap_total_mb, swap_used_mb) =
        if cfg!(target_os = "macos") {
            let total_mb = shell_output("sysctl -n hw.memsize")
                .ok()
                .and_then(|o| o.stdout.trim().parse::<f64>().ok())
                .map(|bytes| (bytes / 1024.0 / 1024.0).round())
                .unwrap_or(-1.0);
            let page_size = shell_output("sysctl -n vm.pagesize")
                .ok()
                .and_then(|o| o.stdout.trim().parse::<f64>().ok())
                .unwrap_or(16384.0);
            let active_pages = shell_output("vm_stat")
                .ok()
                .map(|o| {
                    let mut active = 0.0;
                    for line in o.stdout.lines() {
                        if line.contains("Pages active") || line.contains("Pages wired") {
                            if let Some(value) = line.split(':').nth(1) {
                                active += value
                                    .trim()
                                    .trim_end_matches('.')
                                    .parse::<f64>()
                                    .unwrap_or(0.0);
                            }
                        }
                    }
                    active
                })
                .unwrap_or(0.0);
            let used_mb = ((active_pages * page_size) / 1024.0 / 1024.0).round();
            let (swap_total, swap_used) = shell_output("sysctl -n vm.swapusage")
                .ok()
                .map(|o| parse_macos_swapusage(&o.stdout))
                .unwrap_or((-1.0, -1.0));
            (
                total_mb,
                used_mb,
                if total_mb >= 0.0 && used_mb >= 0.0 {
                    (total_mb - used_mb).max(0.0)
                } else {
                    -1.0
                },
                swap_total,
                swap_used,
            )
        } else {
            shell_output("free -m")
                .ok()
                .map(|o| parse_linux_free(&o.stdout))
                .unwrap_or((-1.0, -1.0, -1.0, -1.0, -1.0))
        };

    let (disk_total, disk_used, disk_avail, disk_use_pct) = shell_output("df -h /")
        .ok()
        .map(|o| parse_df_line(&o.stdout))
        .unwrap_or_else(|| (String::new(), String::new(), String::new(), -1.0));

    let cpu_pct = if load_1 >= 0.0 {
        ((load_1 * 100.0) / num_cpus().max(1) as f64 * 10.0).round() / 10.0
    } else {
        -1.0
    };

    Ok(LocalMetrics {
        uptime,
        load_1,
        load_5,
        load_15,
        mem_total_mb,
        mem_used_mb,
        mem_free_mb,
        swap_total_mb,
        swap_used_mb,
        disk_total,
        disk_used,
        disk_avail,
        disk_use_pct,
        cpu_pct,
    })
}

#[cfg(not(target_os = "windows"))]
fn parse_uptime_line(line: &str) -> (String, f64, f64, f64) {
    let mut uptime = line.to_string();
    let mut load_1 = -1.0;
    let mut load_5 = -1.0;
    let mut load_15 = -1.0;

    if let Some(up_idx) = line.find("up ") {
        let rest = &line[up_idx..];
        if let Some(user_idx) = rest.find("user") {
            uptime = rest[..user_idx]
                .trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ' || c == ',')
                .trim()
                .to_string();
        } else {
            uptime = rest.trim().to_string();
        }
    }

    if let Some(load_idx) = line.find("load average:") {
        let parts: Vec<f64> = line[load_idx + "load average:".len()..]
            .split(',')
            .filter_map(|v| v.trim().parse::<f64>().ok())
            .collect();
        load_1 = parts.first().copied().unwrap_or(-1.0);
        load_5 = parts.get(1).copied().unwrap_or(-1.0);
        load_15 = parts.get(2).copied().unwrap_or(-1.0);
    }

    (uptime, load_1, load_5, load_15)
}

#[cfg(not(target_os = "windows"))]
fn parse_linux_free(stdout: &str) -> (f64, f64, f64, f64, f64) {
    let mut mem_total = -1.0;
    let mut mem_used = -1.0;
    let mut mem_free = -1.0;
    let mut swap_total = -1.0;
    let mut swap_used = -1.0;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Mem:") {
            let nums: Vec<f64> = trimmed
                .split_whitespace()
                .filter_map(|part| part.parse::<f64>().ok())
                .collect();
            if nums.len() >= 3 {
                mem_total = nums[0];
                mem_used = nums[1];
                mem_free = nums[2];
            }
        } else if trimmed.starts_with("Swap:") {
            let nums: Vec<f64> = trimmed
                .split_whitespace()
                .filter_map(|part| part.parse::<f64>().ok())
                .collect();
            if nums.len() >= 2 {
                swap_total = nums[0];
                swap_used = nums[1];
            }
        }
    }

    (mem_total, mem_used, mem_free, swap_total, swap_used)
}

#[cfg(not(target_os = "windows"))]
fn parse_macos_swapusage(stdout: &str) -> (f64, f64) {
    let mut total = -1.0;
    let mut used = -1.0;
    for token in stdout.split_whitespace() {
        if let Some(value) = token.strip_prefix("total") {
            total = parse_macos_gigabytes(value);
        } else if let Some(value) = token.strip_prefix("used") {
            used = parse_macos_gigabytes(value);
        }
    }
    (total, used)
}

#[cfg(not(target_os = "windows"))]
fn parse_macos_gigabytes(token: &str) -> f64 {
    token
        .trim_start_matches('=')
        .trim_end_matches('G')
        .trim_end_matches('M')
        .parse::<f64>()
        .map(|value| {
            if token.ends_with('G') {
                value * 1024.0
            } else {
                value
            }
        })
        .unwrap_or(-1.0)
}

#[cfg(not(target_os = "windows"))]
fn parse_df_line(stdout: &str) -> (String, String, String, f64) {
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("Filesystem") {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 5 {
            return (
                parts[1].to_string(),
                parts[2].to_string(),
                parts[3].to_string(),
                parts[4]
                    .trim_end_matches('%')
                    .parse::<f64>()
                    .unwrap_or(-1.0),
            );
        }
    }
    (String::new(), String::new(), String::new(), -1.0)
}

#[cfg(not(target_os = "windows"))]
fn num_cpus() -> usize {
    exec("nproc")
        .or_else(|_| exec("sysctl -n hw.ncpu"))
        .map(|(_, s)| s.trim().parse().unwrap_or(1))
        .unwrap_or(1)
}
