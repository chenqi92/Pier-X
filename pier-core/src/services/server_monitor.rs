//! Remote server resource monitor (M7b).
//!
//! One-shot probes that run `uptime`, `free -m`, `df -h /`,
//! and read `/proc/stat` over SSH exec, then parse the output
//! into structured [`ServerSnapshot`]. The UI side polls this
//! periodically (typically every 5 s) from a QTimer.
//!
//! ## Cross-distro compat
//!
//! All four commands are available on every mainstream Linux
//! distro. macOS SSH targets get partial coverage: `uptime`
//! and `df` work; `free` doesn't exist (we fall back to
//! `vm_stat`); `/proc/stat` doesn't exist (CPU is left at
//! -1). The parser is lenient — missing fields default to
//! `-1.0` meaning "not available", so the UI can render a
//! "—" placeholder instead of a zero.
//!
//! ## Why one-shot, not streaming?
//!
//! A streaming `top` or `vmstat` produces a continuous line
//! feed that has to be buffered, parsed incrementally, and
//! rate-limited in the UI. For a monitoring dashboard where
//! the user glances at it every few seconds, a simple "run
//! three commands, parse, return" probe is dramatically
//! simpler and still gives sub-second latency per poll.

use serde::{Deserialize, Serialize};

use crate::ssh::error::{Result, SshError};
use crate::ssh::SshSession;

/// A single point-in-time resource snapshot.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerSnapshot {
    /// System uptime string, e.g. `"up 5 days, 3:42"`.
    pub uptime: String,
    /// 1-minute load average. -1 if unavailable.
    pub load_1: f64,
    /// 5-minute load average. -1 if unavailable.
    pub load_5: f64,
    /// 15-minute load average. -1 if unavailable.
    pub load_15: f64,
    /// Total physical RAM in MB. -1 if unavailable.
    pub mem_total_mb: f64,
    /// Used RAM in MB. -1 if unavailable.
    pub mem_used_mb: f64,
    /// Free RAM in MB. -1 if unavailable.
    pub mem_free_mb: f64,
    /// Total swap in MB. -1 if unavailable.
    pub swap_total_mb: f64,
    /// Used swap in MB. -1 if unavailable.
    pub swap_used_mb: f64,
    /// Aggregate disk total across every entry in [`disks`] (i.e. every
    /// mount that survived the pseudo/container filter), in human-readable
    /// form. So a host with `/` (913G) and `/mnt` (1.8T) reads "2.7T"
    /// here, not just the root size.
    pub disk_total: String,
    /// Aggregate used across every entry in [`disks`].
    pub disk_used: String,
    /// Aggregate available across every entry in [`disks`].
    pub disk_avail: String,
    /// Aggregate used percentage = Σused / Σtotal · 100 (0-100). -1 when
    /// no usable mount was found.
    pub disk_use_pct: f64,
    /// Per-filesystem breakdown from `df -hPT`. Populated with every
    /// mounted disk that isn't pseudo (tmpfs / devtmpfs / overlay) or
    /// docker-managed (`/var/lib/docker/*`, `/snap/*`). The root `/`
    /// entry is always first when present.
    pub disks: Vec<DiskEntry>,
    /// Whole-disk topology from `lsblk -P -b -o NAME,KNAME,PKNAME,TYPE,
    /// SIZE,ROTA,TRAN,MODEL,FSTYPE,MOUNTPOINT`. Includes physical disks
    /// even when they have no filesystem yet, plus the
    /// part/crypt/lvm/raid descendants needed to render the storage
    /// tree. Empty on hosts without `lsblk` (BusyBox, macOS) — the UI
    /// should hide its block-device section in that case.
    pub block_devices: Vec<BlockDeviceEntry>,
    /// CPU usage percentage (0-100) since boot, from /proc/stat.
    /// -1 if unavailable (macOS, containers without /proc).
    pub cpu_pct: f64,
    /// Logical CPU count from `nproc` / `/proc/cpuinfo`. 0 if
    /// unavailable.
    pub cpu_count: u32,
    /// Total running processes from `ps -e | wc -l` (minus header).
    /// 0 if unavailable.
    pub proc_count: u32,
    /// OS / kernel description, e.g. `"Ubuntu 24.04.1 · 5.15.0-139"`.
    /// Empty if unavailable.
    pub os_label: String,
    /// Bytes-per-second received across all interfaces, computed as
    /// the delta between two `/proc/net/dev` samples taken roughly
    /// 1 second apart by the probe. -1 when unavailable (no
    /// `/proc/net/dev`, or first sample so no rate yet).
    pub net_rx_bps: f64,
    /// Bytes-per-second transmitted across all interfaces.
    pub net_tx_bps: f64,
    /// Top processes by CPU%. Up to 8 entries, sorted descending.
    pub top_processes: Vec<ProcessRow>,
    /// Top processes by memory%. Up to 8 entries, sorted descending.
    /// A separate probe slice rather than a client-side resort of
    /// `top_processes` because high-memory workloads (Java heaps,
    /// databases, browser tabs) frequently sit at ~0% CPU — the
    /// top-by-CPU set won't contain them, so no client-side
    /// reshuffle can surface the real memory hogs.
    pub top_processes_mem: Vec<ProcessRow>,
}

/// One mounted filesystem as reported by `df -hPT`. Sizes stay as
/// their human-readable strings (same form the gauges use); use
/// [`use_pct`](Self::use_pct) for numeric comparisons.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiskEntry {
    /// Device or filesystem source (e.g. `/dev/nvme0n1p2`, `tmpfs`).
    pub filesystem: String,
    /// FS type reported by `df -T` (e.g. `ext4`, `xfs`).
    pub fs_type: String,
    /// Total size (human-readable, e.g. `"50G"`).
    pub total: String,
    /// Used space (human-readable).
    pub used: String,
    /// Available space (human-readable).
    pub avail: String,
    /// Used percentage 0-100. -1 if unparseable.
    pub use_pct: f64,
    /// Mount point (e.g. `/`, `/home`).
    pub mountpoint: String,
}

/// One row from `lsblk -P -b`. Sizes are bytes (the `-b` flag) so we
/// can keep them numeric and let the UI format. The parent-key column
/// (`pkname`) lets the frontend rebuild the disk → part → crypt → lv
/// tree without having to re-run lsblk in tree mode.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BlockDeviceEntry {
    /// Device basename, e.g. `nvme0n1`, `nvme0n1p2`, `dm-0`.
    pub name: String,
    /// Internal kernel name — usually identical to `name` but stable
    /// across udev rename rules; used as the primary key for the tree.
    pub kname: String,
    /// Parent kernel name (`""` for root physical disks). Children
    /// reference this to find their parent in the tree.
    pub pkname: String,
    /// `disk` / `part` / `lvm` / `crypt` / `raid1` / `loop` etc.
    pub dev_type: String,
    /// Size in bytes (from `lsblk -b`). 0 when the column was empty.
    pub size_bytes: u64,
    /// Rotational media — `true` for spinning HDDs, `false` for SSDs
    /// and any device whose rota flag is unset (e.g. virtio).
    pub rota: bool,
    /// Transport bus, e.g. `sata`, `nvme`, `virtio`, `usb`. Empty for
    /// device-mapper layers (lvm/crypt) which don't have a bus.
    pub tran: String,
    /// Vendor-reported model string, e.g. `"Samsung SSD 980 PRO 1TB"`.
    /// Empty when unknown.
    pub model: String,
    /// Filesystem type if the device holds one directly, otherwise "".
    pub fs_type: String,
    /// Mount point of this exact node (only the leaf in a stacked
    /// topology gets one), or "" if not mounted.
    pub mountpoint: String,
}

/// One row in the "top processes" table. All fields are best-effort —
/// `ps`'s output column widths vary by distro and `comm` may be
/// truncated; the parser keeps whatever the remote shell prints.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessRow {
    /// PID as printed by `ps` (kept as a string because some shells
    /// zero-pad or right-align the column).
    pub pid: String,
    /// Command name (`comm`). May be truncated by the remote `ps`.
    pub command: String,
    /// CPU usage percentage column, verbatim from `ps` (e.g. `"3.4"`).
    pub cpu_pct: String,
    /// Memory usage percentage column, verbatim from `ps` (e.g. `"1.2"`).
    pub mem_pct: String,
    /// Elapsed wall-clock runtime column (`etime`), e.g. `"01:23:45"`.
    pub elapsed: String,
    /// Full command line (argv joined by spaces). Empty when the
    /// remote `ps` couldn't supply it or the source didn't capture
    /// it. Surfaced in the UI as a hover tooltip / detail expand.
    #[serde(default)]
    pub cmd_line: String,
}

/// Run a combined probe and return a single snapshot.
/// Internally chains `uptime && free -m && df -h /` via
/// the session's `exec_command`. Parsing failures for
/// individual sections are silently swallowed and default
/// to -1 / empty, so a partial result is better than
/// an error.
///
/// `include_disks` controls whether the heavy disk sections (`df` and
/// `lsblk`) are included. Pass `false` for the fast 5 s tier (CPU /
/// memory / network / processes only); pass `true` for the slow 30 s
/// tier or for one-off "refresh now" clicks.
pub async fn probe(session: &SshSession, include_disks: bool) -> Result<ServerSnapshot> {
    let mut throwaway: Option<NetSample> = None;
    probe_with_baseline(session, &mut throwaway, include_disks).await
}

/// Like [`probe`] but threads a `/proc/net/dev` baseline through the
/// caller. On entry, `*baseline` (if set) is the previous sample we
/// took for this target; the probe diffs against it to produce
/// `snap.net_rx_bps` / `snap.net_tx_bps`. On exit, `*baseline` is
/// updated to the most recent sample so the next call computes a
/// rate over the elapsed interval. The first call (with `None`)
/// leaves the rate fields at `-1`; subsequent calls fill them in.
///
/// `include_disks` mirrors [`probe`]: skip the disk segments when the
/// caller is on its frequent (5 s) cadence so we don't burn SSH /
/// remote CPU re-running `df` and `lsblk` for data that barely moves.
pub async fn probe_with_baseline(
    session: &SshSession,
    baseline: &mut Option<NetSample>,
    include_disks: bool,
) -> Result<ServerSnapshot> {
    // Chain commands with a separator line we can split on. Each step
    // is wrapped in `( … || true )` so a missing tool (no `free` on
    // BusyBox, no `/proc/stat` on macOS, an unmounted `/`) doesn't
    // short-circuit the rest of the chain — the section just stays
    // empty and the parser falls back to its `-1` defaults. Without
    // this the user can see a partial-data snapshot (CPU but no mem,
    // for example) when one earlier command had a hiccup.
    //
    // `LC_ALL=C` forces predictable English output regardless of the
    // remote's locale (e.g. a server set to zh_CN.UTF-8 would print
    // `内存:` instead of `Mem:` and the parser would skip the row).
    //
    // The full set of sections we now collect:
    //   UPTIME    — `uptime` for system uptime + load averages
    //   FREE      — `free -m` (or `vm_stat`) for memory + swap
    //   DF        — `df -hPT` for per-mount usage (only with include_disks)
    //   LSBLK     — `lsblk -P -b ...` for disk topology  (only with include_disks)
    //   CPUSTAT   — first line of `/proc/stat` for CPU%
    //   NPROC     — logical CPU count
    //   PROCS     — total process count (`ps -e | wc -l` minus header)
    //   OSREL     — distro / kernel id from `/etc/os-release` + `uname`
    //   NETDEV    — `cat /proc/net/dev` for network throughput
    //   TOPPROC   — `ps -eo pid,comm,pcpu,pmem,etime --sort=-pcpu` head
    //
    // Disk sections are skipped on the fast (5 s) cadence — the gauges
    // for CPU/memory/network are the part that needs a frequent
    // refresh; `df` and `lsblk` re-runs on a slower cycle (30 s) and
    // the frontend caches the previous disk view in between so the
    // table doesn't go blank.
    let disk_segments = if include_disks {
        "echo '---DF---'; (df -hPT 2>/dev/null || df -hP / 2>/dev/null || df -h / 2>/dev/null || true); \
         echo '---LSBLK---'; (lsblk -P -b -o NAME,KNAME,PKNAME,TYPE,SIZE,ROTA,TRAN,MODEL,FSTYPE,MOUNTPOINT 2>/dev/null || true); "
    } else {
        ""
    };
    let cmd = format!(
        "LC_ALL=C; export LC_ALL; \
         echo '---UPTIME---'; (uptime 2>/dev/null || true); \
         echo '---FREE---'; (free -m 2>/dev/null || vm_stat 2>/dev/null || true); \
         {disk_segments}\
         echo '---CPUSTAT---'; (head -1 /proc/stat 2>/dev/null || true); \
         echo '---NPROC---'; (nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo 2>/dev/null || true); \
         echo '---PROCS---'; (ps -eo pid 2>/dev/null | wc -l 2>/dev/null || true); \
         echo '---OSREL---'; (cat /etc/os-release 2>/dev/null; uname -sr 2>/dev/null || true); \
         echo '---NETDEV---'; (cat /proc/net/dev 2>/dev/null || true); \
         echo '---TOPPROC---'; (ps -eo pid,comm,pcpu,pmem,etime --sort=-pcpu --no-headers 2>/dev/null | head -8 || true); \
         echo '---TOPPROCM---'; (ps -eo pid,comm,pcpu,pmem,etime --sort=-pmem --no-headers 2>/dev/null | head -8 || true)"
    );
    let (exit, stdout) = session.exec_command(&cmd).await?;
    if exit != 0 && stdout.is_empty() {
        return Err(SshError::InvalidConfig(format!(
            "monitor probe exited {exit} with empty output"
        )));
    }
    // Some shells / wrappers (`free` aliased to a colorized version,
    // motd hooks that splice ANSI into the channel) inject escape
    // sequences. Strip them up front so the line-prefix matchers
    // below don't miss a `\x1b[1mMem:` styled row.
    let stdout = strip_ansi(&stdout);

    let mut snap = ServerSnapshot {
        load_1: -1.0,
        load_5: -1.0,
        load_15: -1.0,
        mem_total_mb: -1.0,
        mem_used_mb: -1.0,
        mem_free_mb: -1.0,
        swap_total_mb: -1.0,
        swap_used_mb: -1.0,
        disk_use_pct: -1.0,
        cpu_pct: -1.0,
        net_rx_bps: -1.0,
        net_tx_bps: -1.0,
        ..Default::default()
    };

    // Split into sections by our sentinel lines.
    let sections = split_sections(&stdout);

    if let Some(s) = sections.get("UPTIME") {
        parse_uptime(s, &mut snap);
    }
    if let Some(s) = sections.get("FREE") {
        parse_free(s, &mut snap);
    }
    if let Some(s) = sections.get("DF") {
        parse_df(s, &mut snap);
    }
    if let Some(s) = sections.get("LSBLK") {
        snap.block_devices = parse_lsblk(s);
    }
    if let Some(s) = sections.get("CPUSTAT") {
        parse_cpustat(s, &mut snap);
    }
    if let Some(s) = sections.get("NPROC") {
        snap.cpu_count = s.lines().next().unwrap_or("").trim().parse().unwrap_or(0);
    }
    if let Some(s) = sections.get("PROCS") {
        // `ps -eo pid | wc -l` includes the column header → subtract 1.
        let raw: u32 = s.lines().next().unwrap_or("").trim().parse().unwrap_or(0);
        snap.proc_count = raw.saturating_sub(1);
    }
    if let Some(s) = sections.get("OSREL") {
        snap.os_label = parse_os_label(s);
    }
    if let Some(s) = sections.get("NETDEV") {
        if let Some(now) = parse_netdev_totals(s) {
            if let Some(prev) = *baseline {
                let dt = now.captured_at.saturating_sub(prev.captured_at) as f64 / 1000.0;
                if dt > 0.05 {
                    snap.net_rx_bps = (now.rx_bytes.saturating_sub(prev.rx_bytes)) as f64 / dt;
                    snap.net_tx_bps = (now.tx_bytes.saturating_sub(prev.tx_bytes)) as f64 / dt;
                }
            }
            // Update the caller's baseline so the next probe can
            // compute a rate against this sample.
            *baseline = Some(now);
        }
    }
    if let Some(s) = sections.get("TOPPROC") {
        snap.top_processes = parse_top_processes(s);
    }
    if let Some(s) = sections.get("TOPPROCM") {
        snap.top_processes_mem = parse_top_processes(s);
    }

    // Post-parse sanity check: if every rich field is still at its
    // sentinel default, the remote host responded but nothing in the
    // output matched. Most common cause is a BusyBox / embedded
    // build (Synology DSM, OpenWRT) that ships neither `free` nor
    // `/proc/stat` in the expected shapes.
    //
    // Two levels of reporting:
    //   * non-sensitive summary (always on) — counts / booleans
    //     only, useful for "did the probe parse anything at all".
    //   * raw output excerpt (verbose-diagnostics gate) — the first
    //     800 chars of the remote stdout. Contains hostnames,
    //     `/etc/os-release`, process command names, df mountpoints.
    //     Nothing password-grade (the probe doesn't read
    //     `/etc/shadow` or env vars), but still user-identifying, so
    //     it's gated behind `set_verbose_diagnostics` and stays off
    //     by default.
    let degraded = snap.cpu_pct < 0.0
        && snap.mem_total_mb < 0.0
        && snap.disk_use_pct < 0.0
        && snap.cpu_count == 0
        && snap.proc_count == 0
        && snap.os_label.is_empty()
        && snap.top_processes.is_empty()
        && snap.top_processes_mem.is_empty();
    if degraded {
        crate::logging::write_event(
            "WARN",
            "monitor.parse",
            &format!(
                "all fields empty after parse (exit={}, stdout_bytes={}); \
                 enable verbose diagnostics in settings to include the raw excerpt",
                exit,
                stdout.len(),
            ),
        );
        // Verbose path: opt-in, tagged so it's obvious in the file.
        let excerpt: String = stdout.chars().take(800).collect();
        crate::logging::write_event_verbose(
            "WARN",
            "monitor.parse",
            &format!(
                "probe stdout excerpt (exit={}, stdout_bytes={}, first 800 chars): {}",
                exit,
                stdout.len(),
                excerpt
            ),
        );
    } else {
        // Non-sensitive partial-degradation log — just the list of
        // missing-field names, no remote output, always safe to
        // record. Helps a user looking at an all-dashes sub-gauge
        // find a reason (`cpu_pct=-1` → `/proc/stat` was missing).
        let mut missing: Vec<&str> = Vec::new();
        if snap.cpu_pct < 0.0 {
            missing.push("cpu_pct");
        }
        if snap.mem_total_mb < 0.0 {
            missing.push("mem");
        }
        if snap.disk_use_pct < 0.0 {
            missing.push("disk");
        }
        if snap.cpu_count == 0 {
            missing.push("cpu_count");
        }
        if snap.proc_count == 0 {
            missing.push("proc_count");
        }
        if snap.os_label.is_empty() {
            missing.push("os_label");
        }
        if snap.top_processes.is_empty() {
            missing.push("top_processes");
        }
        if !missing.is_empty() {
            crate::logging::write_event(
                "DEBUG",
                "monitor.parse",
                &format!("partial probe: missing [{}]", missing.join(", ")),
            );
        }
    }

    Ok(snap)
}

/// Blocking wrapper for [`probe`].
pub fn probe_blocking(session: &SshSession, include_disks: bool) -> Result<ServerSnapshot> {
    crate::ssh::runtime::shared().block_on(probe(session, include_disks))
}

/// Blocking wrapper for [`probe_with_baseline`].
pub fn probe_with_baseline_blocking(
    session: &SshSession,
    baseline: &mut Option<NetSample>,
    include_disks: bool,
) -> Result<ServerSnapshot> {
    crate::ssh::runtime::shared().block_on(probe_with_baseline(session, baseline, include_disks))
}

/// One `/proc/net/dev` sample — total rx/tx bytes summed across all
/// non-loopback interfaces, plus the local timestamp the sample was
/// captured at. Two samples a few seconds apart let us derive a
/// per-second rate.
#[derive(Clone, Copy, Debug)]
pub struct NetSample {
    /// Total bytes received across all non-loopback interfaces.
    pub rx_bytes: u64,
    /// Total bytes transmitted across all non-loopback interfaces.
    pub tx_bytes: u64,
    /// Local clock (ms since UNIX epoch) the sample was taken at.
    pub captured_at: u64,
}

/// Sum the `rx_bytes` (column 1) and `tx_bytes` (column 9) across
/// every interface in `/proc/net/dev`, skipping `lo` (loopback) and
/// any header rows. Returns `None` if no data lines parse — the
/// caller leaves the network gauge at "unavailable".
fn parse_netdev_totals(text: &str) -> Option<NetSample> {
    let mut rx: u64 = 0;
    let mut tx: u64 = 0;
    let mut any = false;
    for line in text.lines() {
        let trimmed = line.trim();
        // Header rows have a `|` in them — skip.
        if trimmed.is_empty() || trimmed.contains('|') {
            continue;
        }
        // Interface name is the column ending in `:`.
        let Some(colon) = trimmed.find(':') else {
            continue;
        };
        let iface = trimmed[..colon].trim();
        // Skip loopback — local-only traffic isn't useful for the
        // "is this server seeing real network activity?" question.
        if iface == "lo" {
            continue;
        }
        let nums: Vec<u64> = trimmed[colon + 1..]
            .split_whitespace()
            .filter_map(|t| t.parse::<u64>().ok())
            .collect();
        // Columns: rx_bytes rx_packets rx_errs ... tx_bytes tx_packets ...
        if nums.len() >= 9 {
            rx = rx.saturating_add(nums[0]);
            tx = tx.saturating_add(nums[8]);
            any = true;
        }
    }
    if !any {
        return None;
    }
    let captured_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Some(NetSample {
        rx_bytes: rx,
        tx_bytes: tx,
        captured_at,
    })
}

/// Build the OS / kernel label shown in the panel header. Combines
/// `/etc/os-release`'s `PRETTY_NAME` with `uname -sr`. Falls back to
/// whichever piece is available.
fn parse_os_label(text: &str) -> String {
    let mut pretty = String::new();
    let mut kernel = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("PRETTY_NAME=") {
            pretty = rest.trim_matches('"').to_string();
        } else if !trimmed.is_empty() && !trimmed.contains('=') && kernel.is_empty() {
            // `uname -sr` output: "Linux 5.15.0-139-generic"
            kernel = trimmed.to_string();
        }
    }
    match (pretty.is_empty(), kernel.is_empty()) {
        (false, false) => format!("{pretty} · {kernel}"),
        (false, true) => pretty,
        (true, false) => kernel,
        (true, true) => String::new(),
    }
}

/// Parse the `ps -eo pid,comm,pcpu,pmem,etime --sort=-pcpu` output
/// into structured rows. Lines are space-separated with `comm` (the
/// 2nd column) potentially containing spaces if the kernel padded it
/// — we treat it as everything between the PID and the next numeric
/// token (CPU%).
fn parse_top_processes(text: &str) -> Vec<ProcessRow> {
    let mut out: Vec<ProcessRow> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        if tokens.len() < 5 {
            continue;
        }
        // Layout: PID COMMAND CPU% MEM% ETIME — at least 5 tokens,
        // with COMMAND being everything between PID and CPU%. CPU%
        // is the third-from-last token, MEM% second-from-last,
        // ETIME last.
        let pid = tokens[0].to_string();
        let elapsed = tokens[tokens.len() - 1].to_string();
        let mem_pct = tokens[tokens.len() - 2].to_string();
        let cpu_pct = tokens[tokens.len() - 3].to_string();
        let command = tokens[1..tokens.len() - 3].join(" ");
        out.push(ProcessRow {
            pid,
            command,
            cpu_pct,
            mem_pct,
            elapsed,
            // The legacy `ps -o pid,comm,...` format used here doesn't
            // carry `args` — adding it would change the column count
            // and break this parser. Cmdline stays empty for SSH
            // probes; local sysinfo path fills it in.
            cmd_line: String::new(),
        });
    }
    out
}

/// Split the combined stdout into named sections.
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

/// Parse `uptime` output. Example:
/// ` 14:23:07 up 5 days,  3:42,  2 users,  load average: 0.12, 0.34, 0.56`
fn parse_uptime(text: &str, snap: &mut ServerSnapshot) {
    let line = text.lines().last().unwrap_or("").trim();
    // Extract "up ... users" part for uptime string.
    if let Some(up_idx) = line.find("up ") {
        let rest = &line[up_idx..];
        if let Some(comma_idx) = rest.find("user") {
            let up_part = &rest[..comma_idx];
            // Trim trailing ", N " (user count prefix).
            snap.uptime = up_part
                .trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ' || c == ',')
                .trim()
                .to_string();
        }
    }
    // Load averages.
    if let Some(la_idx) = line.find("load average:") {
        let la_str = &line[la_idx + "load average:".len()..];
        let parts: Vec<&str> = la_str.split(',').collect();
        if parts.len() >= 3 {
            snap.load_1 = parts[0].trim().parse().unwrap_or(-1.0);
            snap.load_5 = parts[1].trim().parse().unwrap_or(-1.0);
            snap.load_15 = parts[2].trim().parse().unwrap_or(-1.0);
        }
    }
}

/// Parse `free -m` output. Example:
/// ```text
///               total        used        free      shared  buff/cache   available
/// Mem:          15923        4123        8234         456        3565       11343
/// Swap:          2047           0        2047
/// ```
fn parse_free(text: &str, snap: &mut ServerSnapshot) {
    for line in text.lines() {
        // Strip whitespace AND any leading non-letter junk a wrapper
        // might inject (control bytes already removed by `strip_ansi`,
        // but a stray BOM or `>` from a prompt could still slip in).
        let trimmed = line
            .trim_start_matches(|c: char| !c.is_ascii_alphabetic())
            .trim();
        // Case-insensitive prefix check against the row label, with
        // an optional `:` — covers `Mem:`, `mem `, `Memory:` etc.
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("mem") {
            let nums = extract_numbers(trimmed);
            if nums.len() >= 3 {
                snap.mem_total_mb = nums[0];
                snap.mem_used_mb = nums[1];
                snap.mem_free_mb = nums[2];
            }
        } else if lower.starts_with("swap") {
            let nums = extract_numbers(trimmed);
            if nums.len() >= 2 {
                snap.swap_total_mb = nums[0];
                snap.swap_used_mb = nums[1];
            }
        }
    }
}

/// Strip ANSI CSI escape sequences (`\x1b[…<letter>`) from a string.
/// Used before parsing so a colorized `free` / `df` / motd wrapper
/// doesn't slide a `\x1b[1m` past our line-prefix matchers. Keeps
/// other bytes (including UTF-8) untouched.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // Skip CSI: ESC [ <params> <final letter>
            i += 2;
            while i < bytes.len() {
                let b = bytes[i];
                i += 1;
                if (b'@'..=b'~').contains(&b) {
                    break;
                }
            }
            continue;
        }
        // Push the next byte as a UTF-8-safe slice — `text` is &str
        // so we know byte boundaries align with char boundaries.
        let ch_end = next_char_boundary(text, i);
        out.push_str(&text[i..ch_end]);
        i = ch_end;
    }
    out
}

fn next_char_boundary(text: &str, start: usize) -> usize {
    let mut end = start + 1;
    while !text.is_char_boundary(end) && end < text.len() {
        end += 1;
    }
    end.min(text.len())
}

/// Parse `df -h /` output. Example:
/// ```text
/// Filesystem      Size  Used Avail Use% Mounted on
/// /dev/sda1        50G   23G   25G  48% /
/// ```
pub fn parse_df(text: &str, snap: &mut ServerSnapshot) {
    // df occasionally wraps a long device path onto its own line and
    // pushes the size/use/mount columns to the next one. Coalesce
    // any line that has fewer than 5 whitespace-separated tokens
    // with the following line so we still see a complete row.
    let mut joined: Vec<String> = Vec::new();
    let mut buffer = String::new();
    // Minimum token count: five columns for `df -hP` (fs, size, used,
    // avail, use%, mount = 6 — but with row-wrap we only coalesce up
    // to at least 5 so the "mount point" token is also visible).
    // `df -hPT` has an extra type column making it seven.
    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !buffer.is_empty() {
            buffer.push(' ');
        }
        buffer.push_str(trimmed);
        if buffer.split_whitespace().count() >= 6 {
            joined.push(std::mem::take(&mut buffer));
        }
    }
    if !buffer.is_empty() {
        joined.push(buffer);
    }

    // Pseudo / container-managed filesystems we intentionally hide
    // from the per-disk list — the user asked for a "df -h real
    // disks" view without Docker's overlay volumes muddying the
    // space read.
    fn is_ignorable_fs_type(t: &str) -> bool {
        matches!(
            t.to_ascii_lowercase().as_str(),
            "tmpfs"
                | "devtmpfs"
                | "squashfs"
                | "overlay"
                | "overlay2"
                | "aufs"
                | "proc"
                | "sysfs"
                | "cgroup"
                | "cgroup2"
                | "devpts"
                | "mqueue"
                | "pstore"
                | "securityfs"
                | "fusectl"
                | "debugfs"
                | "tracefs"
                | "nsfs"
                | "binfmt_misc"
                | "autofs"
                | "ramfs"
                | "fuse.lxcfs"
                | "none"
        )
    }
    fn is_ignorable_mount(m: &str) -> bool {
        m.starts_with("/var/lib/docker")
            || m.starts_with("/var/lib/containers")
            || m.starts_with("/var/lib/containerd")
            || m.starts_with("/run")
            || m.starts_with("/snap")
            || m.starts_with("/sys")
            || m.starts_with("/proc")
            || m.starts_with("/dev")
            || m.starts_with("/boot/efi")
            || m.starts_with("/private/var/vm")
    }

    for line in &joined {
        // Skip the header row whichever case it's in.
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("filesystem") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            continue;
        }
        // Find the percent-bearing column rather than assuming a
        // fixed index — `df -hP`, `df -h`, and `df -hPT` differ by
        // whether a type column is present, and POSIX puts the
        // percent at 5 while BSD puts it at 4. Scanning works for
        // all layouts.
        let pct_idx = parts.iter().position(|p| p.ends_with('%'));
        let Some(pct_idx) = pct_idx else { continue };
        if pct_idx < 3 {
            continue;
        }
        let filesystem = parts[0].to_string();
        // When `df -hPT` is used, the second column is the FS type;
        // without -T the size column is in slot 1 and we don't know
        // the type. Detect by checking whether parts[1] parses as a
        // df-style size (ends in digit or size-suffix letter) — if
        // it looks like a size, there's no type column.
        let has_type_col = {
            let candidate = parts.get(1).copied().unwrap_or("");
            !(candidate
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(true))
        };
        let fs_type = if has_type_col {
            parts[1].to_string()
        } else {
            String::new()
        };
        let total = parts[pct_idx - 3].to_string();
        let used = parts[pct_idx - 2].to_string();
        let avail = parts[pct_idx - 1].to_string();
        let pct_str = parts[pct_idx].trim_end_matches('%');
        let use_pct = pct_str.parse().unwrap_or(-1.0);
        let mountpoint = parts.get(pct_idx + 1).copied().unwrap_or("").to_string();

        let skip = is_ignorable_fs_type(&fs_type)
            || is_ignorable_mount(&mountpoint)
            || mountpoint.is_empty();
        if !skip {
            snap.disks.push(DiskEntry {
                filesystem,
                fs_type,
                total,
                used,
                avail,
                use_pct,
                mountpoint,
            });
        }
    }
    // The legacy "track the root FS into snap.disk_*" branch used to live
    // in the loop; we now aggregate across every kept mount further down.

    // Legacy fallback: if the remote only returned a single-FS `df -h /`
    // row (pct_idx handling above skipped it for missing column count),
    // try the old five-column parse one more time so we still surface a
    // disk reading. Hits when the remote's `df` collapses output to one
    // line with no header.
    if snap.disks.is_empty() {
        for line in &joined {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("filesystem") {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 5 {
                continue;
            }
            let pct_idx = parts.iter().position(|p| p.ends_with('%'));
            let Some(pct_idx) = pct_idx else { continue };
            if pct_idx < 3 {
                continue;
            }
            snap.disk_total = parts[pct_idx - 3].to_string();
            snap.disk_used = parts[pct_idx - 2].to_string();
            snap.disk_avail = parts[pct_idx - 1].to_string();
            snap.disk_use_pct = parts[pct_idx].trim_end_matches('%').parse().unwrap_or(-1.0);
            break;
        }
    }

    // Root first in the per-disk list, then the rest by mountpoint
    // for a predictable readout.
    snap.disks
        .sort_by(|a, b| match (a.mountpoint == "/", b.mountpoint == "/") {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.mountpoint.cmp(&b.mountpoint),
        });

    // Top-level "总计" / "DISK" gauge is the **sum across every mount we
    // kept** — root + /mnt + any other real disks. Previously this only
    // tracked `/`, so a host with a 913G root and a 1.8T `/mnt` data
    // disk would show "913G total" and the user had no way to know the
    // bigger one even existed in the gauge. Aggregating gives an
    // honest "how much storage is attached to this machine" signal.
    if !snap.disks.is_empty() {
        let mut total_b: u64 = 0;
        let mut used_b: u64 = 0;
        let mut avail_b: u64 = 0;
        let mut any = false;
        for d in &snap.disks {
            if let (Some(t), Some(u), Some(a)) = (
                parse_df_size(&d.total),
                parse_df_size(&d.used),
                parse_df_size(&d.avail),
            ) {
                total_b = total_b.saturating_add(t);
                used_b = used_b.saturating_add(u);
                avail_b = avail_b.saturating_add(a);
                any = true;
            }
        }
        if any && total_b > 0 {
            snap.disk_total = format_df_size(total_b);
            snap.disk_used = format_df_size(used_b);
            snap.disk_avail = format_df_size(avail_b);
            snap.disk_use_pct =
                ((used_b as f64 / total_b as f64) * 100.0 * 10.0).round() / 10.0;
        }
    }
}

/// Parse a df-style size string like `"913G"`, `"1.8T"`, `"212M"`,
/// `"0"` into bytes. Returns `None` if the input doesn't match the
/// `<number>[KMGTP][i?B?]` shape — the aggregation step uses that to
/// quietly skip mounts the remote `df` reported in an unexpected form
/// rather than poisoning the total. Uses 1024-power units to match
/// `df -h`'s default.
fn parse_df_size(s: &str) -> Option<u64> {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return None;
    }
    // Strip a trailing `B` / `iB` if present (some BSD `df`s emit that).
    let stripped = trimmed.trim_end_matches('B').trim_end_matches('i');
    let (num_part, suffix) = match stripped.chars().last() {
        Some(c) if c.is_ascii_alphabetic() => stripped.split_at(stripped.len() - 1),
        _ => (stripped, ""),
    };
    let num: f64 = num_part.parse().ok()?;
    let mult: f64 = match suffix.to_ascii_uppercase().as_str() {
        "" => 1.0,
        "K" => 1024.0,
        "M" => 1024.0 * 1024.0,
        "G" => 1024.0 * 1024.0 * 1024.0,
        "T" => 1024.0_f64.powi(4),
        "P" => 1024.0_f64.powi(5),
        "E" => 1024.0_f64.powi(6),
        _ => return None,
    };
    Some((num * mult) as u64)
}

/// Inverse of [`parse_df_size`] — picks the largest unit that keeps the
/// value ≥ 1 and renders one decimal place (matching `df -h`'s style).
/// `0 → "0B"`, `1234 → "1.2K"`, `2_750_000_000_000 → "2.5T"`.
fn format_df_size(bytes: u64) -> String {
    if bytes == 0 {
        return "0B".to_string();
    }
    // 1024-power scales matching `df -h`. Spelled out as literals
    // because `f64::powi` isn't const-callable.
    const UNITS: &[(&str, f64)] = &[
        ("E", 1_152_921_504_606_846_976.0), // 1024^6
        ("P", 1_125_899_906_842_624.0),     // 1024^5
        ("T", 1_099_511_627_776.0),         // 1024^4
        ("G", 1_073_741_824.0),             // 1024^3
        ("M", 1_048_576.0),                 // 1024^2
        ("K", 1024.0),
    ];
    let b = bytes as f64;
    for (label, scale) in UNITS {
        if b >= *scale {
            let v = b / scale;
            // df -h drops the decimal when the value rounds to ≥ 10.
            if v >= 10.0 {
                return format!("{:.0}{}", v, label);
            }
            return format!("{:.1}{}", v, label);
        }
    }
    format!("{}B", bytes)
}

/// Parse `lsblk -P -b` (key="value" pairs, one device per line) into
/// [`BlockDeviceEntry`] rows. Returns an empty Vec when the section is
/// blank — that's the BusyBox/macOS path where lsblk isn't installed.
///
/// We deliberately don't try to build a tree here; the frontend handles
/// the disk → part → crypt → lv layout because the rendering choices
/// (collapsing, indent style) belong to the UI. The parser just keeps
/// `pkname` so the UI can stitch the children to their parent.
pub fn parse_lsblk(text: &str) -> Vec<BlockDeviceEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut entry = BlockDeviceEntry::default();
        // `-P` rows look like: NAME="sda" KNAME="sda" PKNAME="" TYPE="disk" ...
        // A naive split on whitespace would shred quoted MODEL strings
        // ("Samsung SSD 980 PRO 1TB"), so we walk the line by hand.
        let bytes = trimmed.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // Skip leading whitespace.
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            // Read the KEY up to the `=`.
            let key_start = i;
            while i < bytes.len() && bytes[i] != b'=' {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            let key = &trimmed[key_start..i];
            i += 1; // skip `=`
            // Read the value: either a quoted string or a bare token.
            let value = if i < bytes.len() && bytes[i] == b'"' {
                i += 1;
                let v_start = i;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += 1;
                }
                let v = &trimmed[v_start..i];
                if i < bytes.len() {
                    i += 1; // skip closing `"`
                }
                v
            } else {
                let v_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                &trimmed[v_start..i]
            };
            match key {
                "NAME" => entry.name = value.to_string(),
                "KNAME" => entry.kname = value.to_string(),
                "PKNAME" => entry.pkname = value.to_string(),
                "TYPE" => entry.dev_type = value.to_string(),
                "SIZE" => entry.size_bytes = value.parse().unwrap_or(0),
                "ROTA" => entry.rota = value == "1",
                "TRAN" => entry.tran = value.to_string(),
                "MODEL" => entry.model = value.trim().to_string(),
                "FSTYPE" => entry.fs_type = value.to_string(),
                "MOUNTPOINT" => entry.mountpoint = value.to_string(),
                _ => {}
            }
        }
        if entry.kname.is_empty() {
            // Without a stable identifier we can't anchor children to
            // this row, so drop the malformed line rather than emit a
            // ghost node.
            continue;
        }
        out.push(entry);
    }
    out
}

/// Parse `/proc/stat` first line to get overall CPU usage %.
/// Format: `cpu  user nice system idle iowait irq softirq steal guest guest_nice`
/// We compute `(total - idle) / total * 100`.
fn parse_cpustat(text: &str, snap: &mut ServerSnapshot) {
    let line = text.lines().next().unwrap_or("").trim();
    if !line.starts_with("cpu ") {
        return;
    }
    let nums = extract_numbers(line);
    if nums.len() >= 4 {
        let total: f64 = nums.iter().sum();
        let idle = nums[3]; // 4th field is idle
        if total > 0.0 {
            snap.cpu_pct = ((total - idle) / total * 100.0 * 10.0).round() / 10.0;
        }
    }
}

/// Extract all numeric tokens from a line.
fn extract_numbers(line: &str) -> Vec<f64> {
    line.split_whitespace()
        .filter_map(|tok| tok.parse::<f64>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_uptime_extracts_load_averages() {
        let mut snap = ServerSnapshot {
            load_1: -1.0,
            load_5: -1.0,
            load_15: -1.0,
            ..Default::default()
        };
        parse_uptime(
            " 14:23:07 up 5 days,  3:42,  2 users,  load average: 0.12, 0.34, 0.56",
            &mut snap,
        );
        assert!((snap.load_1 - 0.12).abs() < 0.001);
        assert!((snap.load_5 - 0.34).abs() < 0.001);
        assert!((snap.load_15 - 0.56).abs() < 0.001);
        assert!(snap.uptime.contains("5 days"));
    }

    #[test]
    fn parse_free_extracts_mem_and_swap() {
        let mut snap = ServerSnapshot {
            mem_total_mb: -1.0,
            ..Default::default()
        };
        let text = "\
              total        used        free      shared  buff/cache   available
Mem:          15923        4123        8234         456        3565       11343
Swap:          2047           0        2047";
        parse_free(text, &mut snap);
        assert!((snap.mem_total_mb - 15923.0).abs() < 0.1);
        assert!((snap.mem_used_mb - 4123.0).abs() < 0.1);
        assert!((snap.swap_total_mb - 2047.0).abs() < 0.1);
        assert!((snap.swap_used_mb - 0.0).abs() < 0.1);
    }

    #[test]
    fn parse_df_extracts_root_usage() {
        let mut snap = ServerSnapshot {
            disk_use_pct: -1.0,
            ..Default::default()
        };
        let text = "\
Filesystem      Size  Used Avail Use% Mounted on
/dev/sda1        50G   23G   25G  48% /";
        parse_df(text, &mut snap);
        // Single-mount case: aggregation reduces to that one mount.
        // Total/used/avail are reformatted from the byte sums (50 GiB =
        // 50.0G), and use% is recomputed from used/total rather than
        // taken verbatim from the `df` column.
        assert_eq!(snap.disk_total, "50G");
        assert_eq!(snap.disk_used, "23G");
        assert_eq!(snap.disk_avail, "25G");
        // 23 / 50 * 100 = 46.0 (the 48% from df includes reserved blocks
        // we don't see in Used/Avail).
        assert!((snap.disk_use_pct - 46.0).abs() < 0.5);
        assert_eq!(snap.disks.len(), 1);
        assert_eq!(snap.disks[0].mountpoint, "/");
    }

    #[test]
    fn parse_df_aggregates_multiple_mounts() {
        // Reproducing the 192.168.0.3 layout: 913G root + 1.8T /mnt
        // data disk. Pre-fix the panel showed "总计 913G" — root only.
        // Post-fix the gauge sums both.
        let mut snap = ServerSnapshot {
            disk_use_pct: -1.0,
            ..Default::default()
        };
        let text = "\
Filesystem     Type   Size  Used Avail Use% Mounted on
/dev/sda1      ext4   913G   59G  809G   7% /
/dev/sda2      ext4   2.0G  212M  1.6G  12% /boot
/dev/sdb1      xfs    1.8T   31G  1.7T   2% /mnt";
        parse_df(text, &mut snap);
        assert_eq!(snap.disks.len(), 3);
        // Σtotal ≈ 913G + 2.0G + 1.8T ≈ 2.7T
        assert!(
            snap.disk_total.ends_with('T'),
            "expected aggregate to roll over into T, got {}",
            snap.disk_total
        );
        // Σused ≈ 59 + 0.2 + 31 = ~90G
        assert!(
            snap.disk_used.ends_with('G'),
            "expected aggregate used in G, got {}",
            snap.disk_used
        );
        // Aggregate percentage should be small — most of the storage is
        // unused on /mnt — ≈ 90 / 2750 * 100 ≈ 3.3%.
        assert!(
            snap.disk_use_pct > 0.0 && snap.disk_use_pct < 10.0,
            "aggregate use% should be a few percent, got {}",
            snap.disk_use_pct
        );
    }

    #[test]
    fn parse_df_size_round_trips_common_units() {
        // Round-tripping `df -h` output through bytes and back gives us
        // the same shape (within one decimal place of precision).
        for s in ["0", "212M", "1.6G", "913G", "1.8T", "2.7T"] {
            let bytes = parse_df_size(s).unwrap_or_else(|| panic!("parse failed for {}", s));
            let back = format_df_size(bytes);
            // Allow either same string or one with a slight rounding
            // difference (e.g. "913G" -> "913G", "1.8T" might come back
            // as "1.8T" or "1.8T").
            let trimmed = back.trim_end_matches('B');
            assert!(
                trimmed.starts_with(s.trim_end_matches('B').trim_start_matches('0'))
                    || (s == "0" && back == "0B"),
                "round trip for {} produced {}",
                s,
                back
            );
        }
    }

    #[test]
    fn format_df_size_drops_decimal_above_ten() {
        assert_eq!(format_df_size(0), "0B");
        // 11 GiB → "11G" not "11.0G"
        let eleven_gib = 11u64 * 1024 * 1024 * 1024;
        assert_eq!(format_df_size(eleven_gib), "11G");
        // 1.5 GiB → "1.5G"
        let one_and_half_gib = 1024u64 * 1024 * 1024 * 3 / 2;
        assert_eq!(format_df_size(one_and_half_gib), "1.5G");
    }

    #[test]
    fn parse_lsblk_handles_quoted_model_and_tree() {
        // Real lsblk output: nvme physical disk → partition → encrypted
        // root → LVM volume group → logical volume mounted at /home.
        let text = "\
NAME=\"nvme0n1\" KNAME=\"nvme0n1\" PKNAME=\"\" TYPE=\"disk\" SIZE=\"1024209543168\" ROTA=\"0\" TRAN=\"nvme\" MODEL=\"Samsung SSD 980 PRO 1TB\" FSTYPE=\"\" MOUNTPOINT=\"\"
NAME=\"nvme0n1p1\" KNAME=\"nvme0n1p1\" PKNAME=\"nvme0n1\" TYPE=\"part\" SIZE=\"536870912\" ROTA=\"0\" TRAN=\"\" MODEL=\"\" FSTYPE=\"vfat\" MOUNTPOINT=\"/boot/efi\"
NAME=\"nvme0n1p2\" KNAME=\"nvme0n1p2\" PKNAME=\"nvme0n1\" TYPE=\"part\" SIZE=\"1023672672256\" ROTA=\"0\" TRAN=\"\" MODEL=\"\" FSTYPE=\"crypto_LUKS\" MOUNTPOINT=\"\"
NAME=\"cryptroot\" KNAME=\"dm-0\" PKNAME=\"nvme0n1p2\" TYPE=\"crypt\" SIZE=\"1023668477952\" ROTA=\"0\" TRAN=\"\" MODEL=\"\" FSTYPE=\"LVM2_member\" MOUNTPOINT=\"\"
NAME=\"vg-home\" KNAME=\"dm-1\" PKNAME=\"dm-0\" TYPE=\"lvm\" SIZE=\"858993459200\" ROTA=\"0\" TRAN=\"\" MODEL=\"\" FSTYPE=\"ext4\" MOUNTPOINT=\"/home\"";
        let entries = parse_lsblk(text);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].name, "nvme0n1");
        assert_eq!(entries[0].tran, "nvme");
        assert!(!entries[0].rota); // SSD
        assert_eq!(entries[0].model, "Samsung SSD 980 PRO 1TB"); // quoted, contains spaces
        assert_eq!(entries[0].pkname, "");
        assert_eq!(entries[1].pkname, "nvme0n1");
        assert_eq!(entries[1].mountpoint, "/boot/efi");
        assert_eq!(entries[3].dev_type, "crypt");
        assert_eq!(entries[3].pkname, "nvme0n1p2");
        assert_eq!(entries[4].dev_type, "lvm");
        assert_eq!(entries[4].mountpoint, "/home");
    }

    #[test]
    fn parse_lsblk_tolerates_empty_input() {
        // BusyBox / macOS: no lsblk → segment is blank → empty Vec.
        assert!(parse_lsblk("").is_empty());
        assert!(parse_lsblk("   \n\n").is_empty());
    }

    #[test]
    fn parse_cpustat_computes_usage_pct() {
        let mut snap = ServerSnapshot {
            cpu_pct: -1.0,
            ..Default::default()
        };
        // user=100 nice=0 system=50 idle=850 → (1000-850)/1000*100 = 15%
        parse_cpustat("cpu  100 0 50 850 0 0 0 0 0 0", &mut snap);
        assert!((snap.cpu_pct - 15.0).abs() < 0.1);
    }

    #[test]
    fn split_sections_handles_multi_section() {
        let text = "---A---\nline1\nline2\n---B---\nline3\n";
        let sections = split_sections(text);
        assert_eq!(sections.get("A").unwrap(), "line1\nline2");
        assert_eq!(sections.get("B").unwrap(), "line3");
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let snap = ServerSnapshot {
            uptime: "up 2 days".into(),
            load_1: 0.5,
            load_5: 1.2,
            load_15: 0.8,
            mem_total_mb: 16000.0,
            mem_used_mb: 8000.0,
            mem_free_mb: 8000.0,
            swap_total_mb: 2048.0,
            swap_used_mb: 100.0,
            disk_total: "100G".into(),
            disk_used: "40G".into(),
            disk_avail: "55G".into(),
            disk_use_pct: 42.0,
            cpu_pct: 23.5,
            ..Default::default()
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: ServerSnapshot = serde_json::from_str(&json).unwrap();
        assert!((back.cpu_pct - 23.5).abs() < 0.01);
        assert_eq!(back.disk_total, "100G");
    }

    #[test]
    fn parse_uptime_tolerates_no_load_average() {
        let mut snap = ServerSnapshot {
            load_1: -1.0,
            ..Default::default()
        };
        parse_uptime("14:23:07 up 5 days", &mut snap);
        assert!((snap.load_1 - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn parse_free_tolerates_empty_input() {
        let mut snap = ServerSnapshot {
            mem_total_mb: -1.0,
            ..Default::default()
        };
        parse_free("", &mut snap);
        assert!((snap.mem_total_mb - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn parse_free_tolerates_leading_ansi_strip_and_lowercase() {
        // After `strip_ansi` runs the row label can land in any case
        // and may carry whitespace from a wrapped column header.
        // The lenient prefix match should still pick it up.
        let mut snap = ServerSnapshot {
            mem_total_mb: -1.0,
            ..Default::default()
        };
        let text = "              total        used        free      shared  buff/cache   available
mem:           7841        2031        3092         512        2718        5417
swap:          2047           0        2047";
        parse_free(text, &mut snap);
        assert!((snap.mem_total_mb - 7841.0).abs() < 0.1);
        assert!((snap.mem_used_mb - 2031.0).abs() < 0.1);
        assert!((snap.swap_total_mb - 2047.0).abs() < 0.1);
    }

    #[test]
    fn strip_ansi_removes_csi_sequences() {
        let s = "\x1b[1mMem:\x1b[0m  100 50 50";
        let stripped = strip_ansi(s);
        assert_eq!(stripped, "Mem:  100 50 50");
    }

    #[test]
    fn parse_netdev_skips_loopback_and_sums_other_interfaces() {
        let text = "Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 1234567   12345    0    0    0     0          0         0  1234567   12345    0    0    0     0       0          0
  eth0: 100000     400    0    0    0     0          0        10   50000     200    0    0    0     0       0          0
  eth1: 200000     500    0    0    0     0          0         5   80000     300    0    0    0     0       0          0";
        let sample = parse_netdev_totals(text).expect("expected a sample");
        assert_eq!(sample.rx_bytes, 300_000); // eth0 + eth1, lo skipped
        assert_eq!(sample.tx_bytes, 130_000);
    }

    #[test]
    fn parse_top_processes_extracts_columns() {
        let text = "  1234 systemd        12.4  0.5 5-12:34:56
   789 nginx           3.2  1.1   3:42
   42  postgres        0.0  4.7 14-00:00:00";
        let rows = parse_top_processes(text);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].pid, "1234");
        assert_eq!(rows[0].command, "systemd");
        assert_eq!(rows[0].cpu_pct, "12.4");
        assert_eq!(rows[0].mem_pct, "0.5");
        assert_eq!(rows[0].elapsed, "5-12:34:56");
        assert_eq!(rows[1].command, "nginx");
        assert_eq!(rows[2].pid, "42");
    }

    #[test]
    fn parse_os_label_combines_pretty_name_and_kernel() {
        let text =
            "PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nVERSION=\"24.04.1\"\nLinux 5.15.0-139-generic";
        let label = parse_os_label(text);
        assert!(label.contains("Ubuntu 24.04.1 LTS"));
        assert!(label.contains("5.15.0-139-generic"));
    }

    #[test]
    fn parse_df_handles_wrapped_filesystem_column() {
        // Long device paths (LVM / encrypted volumes) make df wrap
        // the first column onto its own line. The coalescing logic
        // joins it with the size/use/mount row that follows.
        let mut snap = ServerSnapshot {
            disk_use_pct: -1.0,
            ..Default::default()
        };
        let text =
            "Filesystem                                       Size  Used Avail Use% Mounted on
/dev/mapper/long--volume--name--that--wraps
                                                  100G   42G   58G  43% /";
        parse_df(text, &mut snap);
        assert_eq!(snap.disk_total, "100G");
        assert_eq!(snap.disk_used, "42G");
        assert_eq!(snap.disk_avail, "58G");
        // Aggregation recomputes pct from used/total — 42/100 = 42.0,
        // not the verbatim 43% from df (which factors in reserved
        // blocks the human-readable Used/Avail columns hide).
        assert!((snap.disk_use_pct - 42.0).abs() < 0.5);
    }
}
