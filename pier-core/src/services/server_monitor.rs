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
    /// Root filesystem total in human-readable form.
    pub disk_total: String,
    /// Root filesystem used.
    pub disk_used: String,
    /// Root filesystem available.
    pub disk_avail: String,
    /// Root filesystem use percentage (0-100). -1 if unavailable.
    pub disk_use_pct: f64,
    /// CPU usage percentage (0-100) since boot, from /proc/stat.
    /// -1 if unavailable (macOS, containers without /proc).
    pub cpu_pct: f64,
}

/// Run a combined probe and return a single snapshot.
/// Internally chains `uptime && free -m && df -h /` via
/// the session's `exec_command`. Parsing failures for
/// individual sections are silently swallowed and default
/// to -1 / empty, so a partial result is better than
/// an error.
pub async fn probe(session: &SshSession) -> Result<ServerSnapshot> {
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
    let cmd = "LC_ALL=C; export LC_ALL; \
               echo '---UPTIME---'; (uptime 2>/dev/null || true); \
               echo '---FREE---'; (free -m 2>/dev/null || vm_stat 2>/dev/null || true); \
               echo '---DF---'; (df -hP / 2>/dev/null || df -h / 2>/dev/null || true); \
               echo '---CPUSTAT---'; (head -1 /proc/stat 2>/dev/null || true)";
    let (exit, stdout) = session.exec_command(cmd).await?;
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
    if let Some(s) = sections.get("CPUSTAT") {
        parse_cpustat(s, &mut snap);
    }

    Ok(snap)
}

/// Blocking wrapper for [`probe`].
pub fn probe_blocking(session: &SshSession) -> Result<ServerSnapshot> {
    crate::ssh::runtime::shared().block_on(probe(session))
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
fn parse_df(text: &str, snap: &mut ServerSnapshot) {
    // df occasionally wraps a long device path onto its own line and
    // pushes the size/use/mount columns to the next one. Coalesce
    // any line that has fewer than 5 whitespace-separated tokens
    // with the following line so we still see a complete row.
    let mut joined: Vec<String> = Vec::new();
    let mut buffer = String::new();
    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !buffer.is_empty() {
            buffer.push(' ');
        }
        buffer.push_str(trimmed);
        if buffer.split_whitespace().count() >= 5 {
            joined.push(std::mem::take(&mut buffer));
        }
    }
    if !buffer.is_empty() {
        joined.push(buffer);
    }

    for line in &joined {
        // Skip the header row whichever case it's in.
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("filesystem") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            continue;
        }
        // Find the percent-bearing column rather than assuming the
        // 5th token — `df -hP` and `df -h` both put it before the
        // mount point, but POSIX has it as the 5th and BSD as the
        // 4th. Scanning makes both layouts work.
        let pct_idx = parts.iter().position(|p| p.ends_with('%'));
        let Some(pct_idx) = pct_idx else { continue };
        if pct_idx < 3 {
            continue;
        }
        snap.disk_total = parts[pct_idx - 3].to_string();
        snap.disk_used = parts[pct_idx - 2].to_string();
        snap.disk_avail = parts[pct_idx - 1].to_string();
        let pct_str = parts[pct_idx].trim_end_matches('%');
        snap.disk_use_pct = pct_str.parse().unwrap_or(-1.0);
        break; // Only care about root (first data line).
    }
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
        assert_eq!(snap.disk_total, "50G");
        assert_eq!(snap.disk_used, "23G");
        assert_eq!(snap.disk_avail, "25G");
        assert!((snap.disk_use_pct - 48.0).abs() < 0.1);
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
    fn parse_df_handles_wrapped_filesystem_column() {
        // Long device paths (LVM / encrypted volumes) make df wrap
        // the first column onto its own line. The coalescing logic
        // joins it with the size/use/mount row that follows.
        let mut snap = ServerSnapshot {
            disk_use_pct: -1.0,
            ..Default::default()
        };
        let text = "Filesystem                                       Size  Used Avail Use% Mounted on
/dev/mapper/long--volume--name--that--wraps
                                                  100G   42G   58G  43% /";
        parse_df(text, &mut snap);
        assert_eq!(snap.disk_total, "100G");
        assert_eq!(snap.disk_used, "42G");
        assert_eq!(snap.disk_avail, "58G");
        assert!((snap.disk_use_pct - 43.0).abs() < 0.1);
    }
}
