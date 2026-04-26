//! File-backed logger used by both `pier-core` and the Tauri runtime.
//!
//! The goal is pragmatic post-mortem debugging: the file is truncated on
//! every app startup so it never grows unbounded, and every line carries a
//! UTC timestamp, a level tag, and a short source label so a user can paste
//! the log verbatim into a bug report.
//!
//! The logger also installs itself as the global `log` crate implementation,
//! so every existing `log::info!()` / `log::debug!()` call across `pier-core`
//! starts flowing into the same file — callers don't have to retrofit any
//! code to get observability.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static STATE: OnceLock<LoggerState> = OnceLock::new();
static GLOBAL_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Opt-in switch for "verbose diagnostic" records that may contain
/// remote-machine output — /etc/os-release lines, process names from
/// `ps`, df mountpoint paths, shell banners. These aren't secrets but
/// they ARE user-machine-identifying, so by default they stay out of
/// the log. A developer or user reporting a bug can flip this on via
/// [`set_verbose_diagnostics`] (wired to a Settings toggle) to
/// capture the extra context, but the default is silent.
///
/// Intentionally a separate gate from the `log` crate's level filter:
/// the `log::` levels are about "how noisy is each component", this
/// is about "is the user opted into having their remote-machine
/// output written to disk".
static VERBOSE_DIAGNOSTICS: AtomicBool = AtomicBool::new(false);

/// Shared state held behind a single `OnceLock`. The file handle is held
/// inside a `Mutex` because every write needs exclusive access to advance
/// the cursor, and contention here is negligible — this is a debug log,
/// not a hot path.
struct LoggerState {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

/// Initialise the file logger and install it as the global `log`
/// implementation. Truncates `path` on entry so each run starts with a
/// fresh log. Safe to call multiple times; only the first call takes
/// effect.
pub fn init(path: PathBuf) -> std::io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;

    let _ = STATE.set(LoggerState {
        path: path.clone(),
        file: Mutex::new(Some(file)),
    });

    // Best-effort install into the `log` crate facade. Ignored if another
    // logger already claimed the slot (e.g. a test harness running
    // env_logger) — we still keep our own direct `write_line` API.
    //
    // `Info` is deliberate: russh emits a DEBUG record per SSH packet,
    // and while a terminal is active that's thousands of records per
    // second. Each one would walk the `log::set_logger` hook, hit our
    // file mutex, and block on a synchronous disk write — the UI
    // thread stalls visibly during interactive prompts (a `yes/no`
    // host-key confirmation drives enough channel traffic to freeze
    // the app). `Info` is quiet enough that russh / tokio / mio don't
    // contribute, while pier-core's own `log::info!` / `log::warn!` /
    // `log::error!` calls still flow through. When you need a noisy
    // dependency back, use `enabled()` below to whitelist specific
    // targets instead of raising the max level globally.
    if GLOBAL_INSTALLED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        let _ = log::set_logger(&GlobalBridge);
        log::set_max_level(log::LevelFilter::Info);
    }

    write_line(
        "INFO",
        "logger",
        &format!("log initialised at {}", path.display()),
    );
    Ok(path)
}

/// Absolute path to the active log file, or `None` if [`init`] has not
/// been called yet.
pub fn log_file_path() -> Option<PathBuf> {
    STATE.get().map(|s| s.path.clone())
}

/// Append a single line to the log file. No-op if the logger has not
/// been initialised. Errors writing are silently dropped — a logger
/// that crashes the app on a disk issue is worse than losing a line.
pub fn write_line(level: &str, source: &str, message: &str) {
    let Some(state) = STATE.get() else { return };
    let ts = format_timestamp(SystemTime::now());
    let line = format!(
        "{ts} [{lvl:<5}] [{src}] {msg}\n",
        ts = ts,
        lvl = level,
        src = source,
        msg = sanitize_for_line(message),
    );
    if let Ok(mut guard) = state.file.lock() {
        if let Some(file) = guard.as_mut() {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

/// Convenience wrapper for structured records coming in from the
/// frontend via the `log_write` Tauri command.
pub fn write_event(level: &str, source: &str, message: &str) {
    write_line(level, source, message);
}

/// Toggle the verbose-diagnostic gate. See [`VERBOSE_DIAGNOSTICS`]
/// for what this gates. Wired from a Settings UI toggle; callers
/// that emit sensitive excerpts should go through
/// [`write_event_verbose`] so flipping this off is a real privacy
/// knob, not just a cosmetic filter.
pub fn set_verbose_diagnostics(enabled: bool) {
    VERBOSE_DIAGNOSTICS.store(enabled, Ordering::Release);
    write_line(
        "INFO",
        "logger",
        &format!(
            "verbose diagnostics {}",
            if enabled { "enabled" } else { "disabled" }
        ),
    );
}

/// Whether the verbose-diagnostic gate is currently open.
pub fn verbose_diagnostics_enabled() -> bool {
    VERBOSE_DIAGNOSTICS.load(Ordering::Acquire)
}

/// Like [`write_event`] but only writes when
/// [`set_verbose_diagnostics`] has been flipped on. Use this for any
/// log line that would contain remote-machine output — hostnames,
/// `/etc/os-release`, `ps` process lists, df mount paths, raw
/// command output. Keeps the default log free of user-identifiable
/// telemetry while still letting a user turn it on for bug reports.
///
/// The `level` is prefixed with `verbose.` so it's obvious in the
/// file which records are gated.
pub fn write_event_verbose(level: &str, source: &str, message: &str) {
    if !verbose_diagnostics_enabled() {
        return;
    }
    write_line(&format!("{level}+"), source, message);
}

/// Strip newlines / control characters so a single log event stays on
/// one physical line. Long messages are kept intact — if the user wants
/// to paste a traceback they get the whole thing, just re-flowed.
fn sanitize_for_line(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    for ch in message.chars() {
        match ch {
            '\n' | '\r' => out.push_str(" | "),
            '\t' => out.push_str("  "),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// UTC timestamp formatted as `YYYY-MM-DDTHH:MM:SS.mmmZ`, produced
/// without pulling in `chrono` / `time`. Good enough for debug logs —
/// not a calendar library.
fn format_timestamp(now: SystemTime) -> String {
    let dur = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs() as i64;
    let ms = dur.subsec_millis();
    let (year, month, day, hour, minute, second) = civil_from_unix(secs);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hour, minute, second, ms
    )
}

/// Convert seconds-since-1970 (UTC) into a civil calendar tuple.
/// Uses Howard Hinnant's `civil_from_days` algorithm — handles dates
/// well beyond any conceivable Pier-X install lifetime.
fn civil_from_unix(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let (days, time_of_day) = {
        let days = secs.div_euclid(86_400);
        let rem = secs.rem_euclid(86_400) as u32;
        (days, rem)
    };
    let hour = time_of_day / 3600;
    let minute = (time_of_day % 3600) / 60;
    let second = time_of_day % 60;

    // Days since 1970-01-01 → civil date.
    let z = days + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = (z - era * 146_097) as u32; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { (y + 1) as i32 } else { y as i32 };
    (year, m, d, hour, minute, second)
}

/// `log` crate adapter — forwards records into [`write_line`] so every
/// existing `log::info!()` / `log::warn!()` / `log::error!()` call in
/// the codebase ends up in the file without touching its call sites.
///
/// Transitive crates that log at extremely high cardinality are
/// downgraded here instead of at the global max-level knob. russh in
/// particular emits one record per SSH packet under its default
/// config; even at `Info` level that's dozens of records per second
/// during an interactive prompt. We let them through only at `Warn`
/// and above so a real error still surfaces but normal packet traffic
/// doesn't saturate the file mutex.
struct GlobalBridge;

fn is_noisy_target(target: &str) -> bool {
    // Prefix match is enough — these crates all namespace their log
    // targets off their own crate name. Added russh_sftp separately
    // because it ships its own target; same treatment needed.
    const NOISY: &[&str] = &[
        "russh",
        "russh_sftp",
        "russh_keys",
        "russh_cryptovec",
        "mio",
        "tokio",
        "tokio_util",
        "rustls",
        "hyper",
        "h2",
        "reqwest",
    ];
    NOISY
        .iter()
        .any(|p| target == *p || target.starts_with(&format!("{}::", p)))
}

impl log::Log for GlobalBridge {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        if is_noisy_target(metadata.target()) {
            return metadata.level() <= log::Level::Warn;
        }
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            log::Level::Error => "ERROR",
            log::Level::Warn => "WARN",
            log::Level::Info => "INFO",
            log::Level::Debug => "DEBUG",
            log::Level::Trace => "TRACE",
        };
        let source = record.target();
        let message = record.args().to_string();
        write_line(level, source, &message);
    }

    fn flush(&self) {
        if let Some(state) = STATE.get() {
            if let Ok(mut guard) = state.file.lock() {
                if let Some(file) = guard.as_mut() {
                    let _ = file.flush();
                }
            }
        }
    }
}

/// Convenience alias so `_ = logging::init_under(dir, "pier-x.log")`
/// reads naturally from the Tauri setup hook. Returns the absolute
/// path of the log file on success.
pub fn init_under<P: AsRef<Path>>(dir: P, file_name: &str) -> std::io::Result<PathBuf> {
    let mut path = dir.as_ref().to_path_buf();
    path.push(file_name);
    init(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_unix_matches_known_dates() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        let (y, m, d, h, mi, s) = civil_from_unix(1_704_067_200);
        assert_eq!((y, m, d, h, mi, s), (2024, 1, 1, 0, 0, 0));
        // 1970-01-01 epoch
        let (y, m, d, _, _, _) = civil_from_unix(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn sanitize_collapses_newlines_into_delimiter() {
        let out = sanitize_for_line("line1\nline2\r\nline3");
        assert_eq!(out, "line1 | line2 |  | line3");
    }

    #[test]
    fn format_timestamp_is_iso8601_shape() {
        let s = format_timestamp(SystemTime::UNIX_EPOCH);
        assert!(s.starts_with("1970-01-01T00:00:00."));
        assert!(s.ends_with('Z'));
    }
}
