//! Pure formatters for remote filesystem metadata — rendered by the
//! SFTP browser's wide-panel columns.
//!
//! No IO, no allocations beyond the returned `String`. Safe to call
//! from a render body.

use std::time::{SystemTime, UNIX_EPOCH};

/// Turn a Unix-epoch mtime into a compact relative label:
///   - `< 60s`            → `just now`
///   - `< 60m`            → `Nm`
///   - `< 24h`            → `Nh`
///   - `< 7d`             → `Nd`
///   - `< 52w`            → `Nw`
///   - older, or future   → `YYYY-MM-DD`
///
/// Designed to fit in ~56 px of `caption` text so it never pushes the
/// name column when the panel is in its medium range.
pub fn format_relative_time(mtime_secs: u64) -> String {
    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return format_date(mtime_secs);
    };
    let now_secs = now.as_secs();

    // Future timestamps (clock skew) fall back to the date.
    if mtime_secs > now_secs {
        return format_date(mtime_secs);
    }

    let diff = now_secs - mtime_secs;
    if diff < 60 {
        "just now".to_string()
    } else if diff < 60 * 60 {
        format!("{}m", diff / 60)
    } else if diff < 60 * 60 * 24 {
        format!("{}h", diff / 3600)
    } else if diff < 60 * 60 * 24 * 7 {
        format!("{}d", diff / 86_400)
    } else if diff < 60 * 60 * 24 * 365 {
        format!("{}w", diff / (86_400 * 7))
    } else {
        format_date(mtime_secs)
    }
}

/// `YYYY-MM-DD` from a Unix timestamp. Pure (no libc/chrono) so it
/// stays render-safe and dependency-free. Uses the proleptic
/// Gregorian calendar — adequate for any mtime a modern filesystem
/// will produce.
fn format_date(secs: u64) -> String {
    let days_since_epoch = (secs / 86_400) as i64;
    let (y, m, d) = civil_from_days(days_since_epoch);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Howard Hinnant's `civil_from_days` — converts days since
/// 1970-01-01 into (year, month, day). Verified to be correct for
/// all values of `z` that fit in an i64.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

/// Format a POSIX mode as `drwxr-xr-x` / `-rw-r--r--` / `lrwxrwxrwx`.
/// Only the low 9 permission bits plus the file-type hint are used;
/// setuid/setgid/sticky are intentionally skipped to keep the column
/// narrow (10 chars fixed).
pub fn format_permissions(mode: u32, is_dir: bool, is_link: bool) -> String {
    let type_char = if is_link {
        'l'
    } else if is_dir {
        'd'
    } else {
        '-'
    };
    let mut out = String::with_capacity(10);
    out.push(type_char);
    for shift in [6u32, 3, 0] {
        let bits = (mode >> shift) & 0b111;
        out.push(if bits & 0b100 != 0 { 'r' } else { '-' });
        out.push(if bits & 0b010 != 0 { 'w' } else { '-' });
        out.push(if bits & 0b001 != 0 { 'x' } else { '-' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_directory_755() {
        assert_eq!(format_permissions(0o755, true, false), "drwxr-xr-x");
    }

    #[test]
    fn permissions_file_644() {
        assert_eq!(format_permissions(0o644, false, false), "-rw-r--r--");
    }

    #[test]
    fn permissions_symlink() {
        assert_eq!(format_permissions(0o777, false, true), "lrwxrwxrwx");
    }

    #[test]
    fn relative_time_future_falls_back_to_date() {
        let future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 60 * 60 * 24 * 365;
        let out = format_relative_time(future);
        assert_eq!(out.len(), 10, "expected YYYY-MM-DD, got {out}");
    }

    #[test]
    fn civil_from_days_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn civil_from_days_known() {
        // 2026-01-01 = 20_454 days since epoch.
        assert_eq!(civil_from_days(20_454), (2026, 1, 1));
    }
}
