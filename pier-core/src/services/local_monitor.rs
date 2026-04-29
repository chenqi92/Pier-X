//! Local-host probe via the `sysinfo` crate — replaces the
//! per-OS shell-out path that used to spawn PowerShell on Windows
//! (`Get-CimInstance` + `Get-Process`), `vm_stat` / `sysctl` / `df`
//! on macOS, and `df` / `lsblk` on Linux.
//!
//! Why this replaced the shell-out approach:
//!
//!   * The Windows path launched `powershell.exe` per probe (~300 –
//!     800 ms cold, lighting up a CPU core during WMI + Get-Process).
//!     Stacked with the 5 s polling cadence, every input burst into
//!     a local terminal had a coin-flip chance of colliding with a
//!     spike, which the user perceived as the terminal freezing.
//!   * The macOS path made four sequential subprocess spawns per
//!     fast tick. Cheap individually, but death-by-a-thousand-cuts
//!     on the IPC dispatcher.
//!   * Splitting four cargo-dep-free implementations (one per OS)
//!     also meant CPU%, network rate, and process tables had subtly
//!     different shapes per-OS — surfacing as missing columns in
//!     the panel.
//!
//! `sysinfo` reads the equivalent kernel interfaces directly (Win32
//! NtQuery* APIs, Apple `libproc`, `/proc/*` on Linux) without
//! forking. A single in-process `System` handle survives across
//! probes so per-process CPU% / network rate deltas accumulate over
//! the polling cadence and don't need a synthetic 200 ms sleep on
//! every call.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, Networks, Pid, ProcessRefreshKind,
    ProcessesToUpdate, RefreshKind, System,
};

use super::server_monitor::{DiskEntry, ProcessRow, ServerSnapshot};

/// Per-process refresh kind pre-built once and reused — sysinfo's
/// builder allocates on each call, and we hit this hot path twice
/// per probe (once for top-by-CPU, once for top-by-mem).
fn process_refresh() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_exe(sysinfo::UpdateKind::OnlyIfNotSet)
}

/// Long-lived state used to compute deltas (CPU per-process
/// percentage, network rate). Held behind a `Mutex` because Tauri
/// might dispatch overlapping local probes from rapid panel mounts;
/// the mutex collapses them into one refresh per call.
struct ProbeState {
    sys: System,
    networks: Networks,
    /// Last network sample so we can compute rate without forcing
    /// a sleep inside the probe call. `None` until the first
    /// refresh writes a baseline. Sub-second deltas with an
    /// elapsed near zero produce a meaningless rate, so we skip
    /// reporting those (UI displays "—" until the second probe).
    last_net_total_rx: u64,
    last_net_total_tx: u64,
    last_net_at: Option<Instant>,
}

impl ProbeState {
    fn new() -> Self {
        Self {
            sys: System::new_with_specifics(
                RefreshKind::nothing()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything())
                    .with_processes(process_refresh()),
            ),
            networks: Networks::new_with_refreshed_list(),
            last_net_total_rx: 0,
            last_net_total_tx: 0,
            last_net_at: None,
        }
    }
}

fn state() -> &'static Mutex<ProbeState> {
    static STATE: OnceLock<Mutex<ProbeState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(ProbeState::new()))
}

/// Collect a single snapshot of the local host. Mirrors the shape
/// of [`super::server_monitor::probe`] so the same `ServerSnapshot`
/// → `ServerSnapshotView` mapping in the Tauri layer covers both
/// remote and local paths. `include_disks` controls the `df` /
/// `lsblk` equivalent — kept off on the fast tier to mirror the
/// SSH probe's bandwidth tier separation.
pub fn collect_snapshot(include_disks: bool) -> ServerSnapshot {
    let mut state = match state().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };

    // CPU + memory are always cheap. Process refresh is the bulk of
    // the work but `sysinfo` already throttles its own kernel calls
    // — back-to-back refreshes within `minimum_cpu_update_interval`
    // (200 ms) are no-ops.
    state
        .sys
        .refresh_cpu_specifics(CpuRefreshKind::everything());
    state
        .sys
        .refresh_memory_specifics(MemoryRefreshKind::everything());
    state
        .sys
        .refresh_processes_specifics(ProcessesToUpdate::All, true, process_refresh());

    // Memory — bytes -> MB. `total_memory()` is bytes in sysinfo
    // 0.30+ (older versions returned KB).
    let mem_total_bytes = state.sys.total_memory();
    let mem_used_bytes = state.sys.used_memory();
    let mem_free_bytes = mem_total_bytes.saturating_sub(mem_used_bytes);
    let mem_total_mb = mem_total_bytes as f64 / (1024.0 * 1024.0);
    let mem_used_mb = mem_used_bytes as f64 / (1024.0 * 1024.0);
    let mem_free_mb = mem_free_bytes as f64 / (1024.0 * 1024.0);
    let swap_total_mb = state.sys.total_swap() as f64 / (1024.0 * 1024.0);
    let swap_used_mb = state.sys.used_swap() as f64 / (1024.0 * 1024.0);

    // CPU% — averaged across logical cores. `global_cpu_usage()`
    // returns the post-refresh aggregate from the previous interval.
    // First-ever call returns 0; that's a single-tick artefact and
    // self-corrects on the second call.
    let cpu_pct = state.sys.global_cpu_usage() as f64;
    let cpu_count = state.sys.cpus().len() as u32;

    // Load average — sysinfo provides on Unix; Windows always reports
    // -1 here. We convert NaN/negative to -1 so the UI consistently
    // shows the placeholder.
    let load = sysinfo::System::load_average();
    let load_or_neg = |v: f64| if v.is_finite() { v } else { -1.0 };
    let (load_1, load_5, load_15) = if cfg!(windows) {
        (-1.0, -1.0, -1.0)
    } else {
        (load_or_neg(load.one), load_or_neg(load.five), load_or_neg(load.fifteen))
    };

    // Uptime — sysinfo returns seconds since boot. Format the same
    // way the SSH probe does: `D-HH:MM:SS` or `HH:MM:SS`.
    let uptime_secs = sysinfo::System::uptime();
    let uptime = format_elapsed(uptime_secs);

    // OS / kernel label — same shape as SSH probe.
    let os_label = build_os_label();

    // Process count
    let proc_count = state.sys.processes().len() as u32;

    // Top processes — split into CPU-sorted and memory-sorted. We
    // collect once, sort twice — cheap relative to the refresh.
    let mut all_procs: Vec<(Pid, &sysinfo::Process)> = state
        .sys
        .processes()
        .iter()
        .filter(|(_, p)| p.thread_kind().is_none())
        .map(|(pid, p)| (*pid, p))
        .collect();
    let total_mem_for_pct = if mem_total_bytes > 0 { mem_total_bytes as f64 } else { 1.0 };
    let to_row = |(pid, p): (Pid, &sysinfo::Process)| -> ProcessRow {
        let mem_pct = (p.memory() as f64 / total_mem_for_pct) * 100.0;
        // Join argv with single spaces. We don't try to shell-escape
        // — this is purely for display and the user sees something
        // close to what they typed at the shell.
        let cmd_line = p
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        ProcessRow {
            pid: pid.as_u32().to_string(),
            command: process_display_name(p),
            cpu_pct: format!("{:.1}", p.cpu_usage()),
            mem_pct: format!("{:.1}", mem_pct),
            elapsed: format_elapsed(p.run_time()),
            cmd_line,
        }
    };
    all_procs.sort_unstable_by(|a, b| {
        b.1.cpu_usage()
            .partial_cmp(&a.1.cpu_usage())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top_processes: Vec<ProcessRow> = all_procs.iter().take(8).copied().map(to_row).collect();
    all_procs.sort_unstable_by_key(|(_, p)| std::cmp::Reverse(p.memory()));
    let top_processes_mem: Vec<ProcessRow> =
        all_procs.iter().take(8).copied().map(to_row).collect();

    // Network rate — sysinfo accumulates totals; we diff vs the last
    // call to get bytes/second. First call returns -1 / -1 to match
    // the "warming up" UI placeholder.
    state.networks.refresh(true);
    let mut total_rx: u64 = 0;
    let mut total_tx: u64 = 0;
    for (_, data) in state.networks.iter() {
        total_rx = total_rx.saturating_add(data.total_received());
        total_tx = total_tx.saturating_add(data.total_transmitted());
    }
    let now = Instant::now();
    let (net_rx_bps, net_tx_bps) = match state.last_net_at {
        Some(prev_at) => {
            let dt = now.duration_since(prev_at).as_secs_f64();
            if dt > 0.05 {
                let drx = total_rx.saturating_sub(state.last_net_total_rx) as f64;
                let dtx = total_tx.saturating_sub(state.last_net_total_tx) as f64;
                (drx / dt, dtx / dt)
            } else {
                (-1.0, -1.0)
            }
        }
        None => (-1.0, -1.0),
    };
    state.last_net_total_rx = total_rx;
    state.last_net_total_tx = total_tx;
    state.last_net_at = Some(now);

    // Disk — only on the full tier. `Disks::new_with_refreshed_list`
    // walks every mounted FS each call; we still gate on the flag so
    // the fast tier stays cheap.
    let (disk_total, disk_used, disk_avail, disk_use_pct, disks) = if include_disks {
        let disks_handle = Disks::new_with_refreshed_list();
        let mut total_bytes: u64 = 0;
        let mut avail_bytes: u64 = 0;
        let mut entries: Vec<DiskEntry> = Vec::with_capacity(disks_handle.len());
        for d in disks_handle.iter() {
            let total = d.total_space();
            let avail = d.available_space();
            if total == 0 {
                continue;
            }
            let used = total.saturating_sub(avail);
            total_bytes = total_bytes.saturating_add(total);
            avail_bytes = avail_bytes.saturating_add(avail);
            let mountpoint = d.mount_point().to_string_lossy().into_owned();
            entries.push(DiskEntry {
                filesystem: d.name().to_string_lossy().into_owned(),
                fs_type: d.file_system().to_string_lossy().into_owned(),
                total: humanize_bytes(total),
                used: humanize_bytes(used),
                avail: humanize_bytes(avail),
                use_pct: ((used as f64) / (total as f64)) * 100.0,
                mountpoint,
            });
        }
        // Stable order: root mount first (matches the SSH `df`
        // parser's behaviour), then alphabetical.
        entries.sort_by(|a, b| {
            let root_a = a.mountpoint == "/" || a.mountpoint.eq_ignore_ascii_case("c:\\");
            let root_b = b.mountpoint == "/" || b.mountpoint.eq_ignore_ascii_case("c:\\");
            match (root_a, root_b) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.mountpoint.cmp(&b.mountpoint),
            }
        });
        let used_bytes = total_bytes.saturating_sub(avail_bytes);
        let pct = if total_bytes > 0 {
            ((used_bytes as f64) / (total_bytes as f64)) * 100.0
        } else {
            -1.0
        };
        (
            humanize_bytes(total_bytes),
            humanize_bytes(used_bytes),
            humanize_bytes(avail_bytes),
            pct,
            entries,
        )
    } else {
        (String::new(), String::new(), String::new(), -1.0, Vec::new())
    };

    ServerSnapshot {
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
        disks,
        // sysinfo doesn't surface the lsblk-style topology. The SSH
        // path still parses it for Linux remotes; locally we leave
        // empty and the BLOCK DEVICES section auto-hides on the
        // frontend. Keeping shape parity with the SSH ServerSnapshot.
        block_devices: Vec::new(),
        cpu_pct,
        cpu_count,
        proc_count,
        os_label,
        net_rx_bps,
        net_tx_bps,
        top_processes,
        top_processes_mem,
    }
}

fn process_display_name(p: &sysinfo::Process) -> String {
    // Prefer the executable basename — it's what users recognise
    // (`chrome.exe`, `node`) — falling back to the kernel-reported
    // `name()` (which is sometimes truncated to 15 chars on Linux).
    if let Some(exe) = p.exe() {
        if let Some(stem) = exe.file_name() {
            return stem.to_string_lossy().into_owned();
        }
    }
    p.name().to_string_lossy().into_owned()
}

fn build_os_label() -> String {
    let name = sysinfo::System::name().unwrap_or_default();
    let version = sysinfo::System::os_version().unwrap_or_default();
    let kernel = sysinfo::System::kernel_version().unwrap_or_default();
    match (name.as_str(), version.as_str(), kernel.as_str()) {
        ("", "", "") => String::new(),
        (n, v, "") if !n.is_empty() && !v.is_empty() => format!("{n} {v}"),
        (n, "", k) if !n.is_empty() && !k.is_empty() => format!("{n} · {k}"),
        (n, v, k) if !n.is_empty() && !v.is_empty() && !k.is_empty() => {
            format!("{n} {v} · {k}")
        }
        (n, _, _) if !n.is_empty() => n.to_string(),
        (_, v, _) if !v.is_empty() => v.to_string(),
        (_, _, k) if !k.is_empty() => k.to_string(),
        _ => String::new(),
    }
}

/// Format a duration in seconds as either `D-HH:MM:SS` (≥ 1 day)
/// or `HH:MM:SS`. Matches the format the SSH `ps -eo etime` parser
/// emits so process tables read identically across local / remote.
fn format_elapsed(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    if days > 0 {
        format!("{days}-{h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

/// Compact human-readable byte count — uses base-1024 units to match
/// the `df -h` output that SSH probes return, so a "free of" string
/// like `100G` reads the same in either path.
fn humanize_bytes(b: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;
    if b >= TB {
        format!("{:.1}T", b as f64 / TB as f64)
    } else if b >= GB {
        format!("{:.1}G", b as f64 / GB as f64)
    } else if b >= MB {
        format!("{:.1}M", b as f64 / MB as f64)
    } else if b >= KB {
        format!("{:.1}K", b as f64 / KB as f64)
    } else {
        format!("{b}B")
    }
}

/// Send a termination signal to a local process by PID. `force=false`
/// requests graceful termination (SIGTERM on Unix, WM_CLOSE-equivalent
/// via `TerminateProcess(255)` on Windows — sysinfo wraps both); the
/// process is given a chance to clean up. `force=true` is the
/// equivalent of SIGKILL — immediate, no signal handler can stop it.
///
/// We deliberately route through sysinfo's `Signal::Term` /
/// `Signal::Kill` rather than a per-OS shell-out: this keeps the
/// implementation cross-platform and avoids spawning yet another
/// child process to do the kill (which would defeat the
/// no-subprocess principle the rest of this module follows).
pub fn kill_local_process(pid: u32, force: bool) -> Result<(), String> {
    let mut sys = match state().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    // Refresh just enough to find the target — full process refresh
    // would be overkill for a one-off lookup.
    sys.sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        ProcessRefreshKind::nothing(),
    );
    let Some(proc) = sys.sys.process(Pid::from_u32(pid)) else {
        return Err(format!("no such pid: {pid}"));
    };
    let signal = if force {
        sysinfo::Signal::Kill
    } else {
        sysinfo::Signal::Term
    };
    match proc.kill_with(signal) {
        Some(true) => Ok(()),
        Some(false) => Err(format!(
            "kill failed (insufficient permissions or already exited): pid {pid}"
        )),
        None => {
            // Signal not supported on this OS — fall back to the
            // platform-default which on Windows is TerminateProcess.
            if proc.kill() {
                Ok(())
            } else {
                Err(format!("kill failed: pid {pid}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_populates_basics() {
        // First call produces -1 net rates and 0 cpu (no prior delta);
        // we just want to confirm the path doesn't panic and the
        // self-reported counts look sane on the test host.
        let snap = collect_snapshot(false);
        assert!(snap.cpu_count > 0, "should report at least one logical CPU");
        assert!(
            snap.mem_total_mb > 0.0,
            "host with sysinfo support should always report memory"
        );
        assert!(
            !snap.uptime.is_empty(),
            "uptime should be formatted as HH:MM:SS or D-HH:MM:SS"
        );
        assert!(
            snap.proc_count > 0,
            "the test runner is itself a process — count must be ≥ 1"
        );
    }

    #[test]
    fn second_snapshot_has_network_rate() {
        // First snapshot primes the network baseline; second yields a
        // real (possibly zero) bytes/sec delta.
        let _ = collect_snapshot(false);
        std::thread::sleep(std::time::Duration::from_millis(60));
        let snap = collect_snapshot(false);
        assert!(snap.net_rx_bps >= 0.0);
        assert!(snap.net_tx_bps >= 0.0);
    }

    #[test]
    fn full_tier_includes_disks() {
        let snap = collect_snapshot(true);
        assert!(
            !snap.disks.is_empty(),
            "any host running this test has at least one mounted disk"
        );
        assert!(!snap.disk_total.is_empty());
        assert!(snap.disk_use_pct >= 0.0 && snap.disk_use_pct <= 100.0);
    }

    #[test]
    fn humanize_bytes_uses_binary_units() {
        assert_eq!(humanize_bytes(0), "0B");
        assert_eq!(humanize_bytes(512), "512B");
        assert_eq!(humanize_bytes(1024), "1.0K");
        assert_eq!(humanize_bytes(1024 * 1024), "1.0M");
        assert_eq!(humanize_bytes(1024 * 1024 * 1024), "1.0G");
    }

    #[test]
    fn format_elapsed_matches_ps_etime_shape() {
        assert_eq!(format_elapsed(0), "00:00:00");
        assert_eq!(format_elapsed(59), "00:00:59");
        assert_eq!(format_elapsed(3661), "01:01:01");
        assert_eq!(format_elapsed(86_400 + 3600 + 60 + 1), "1-01:01:01");
    }
}
