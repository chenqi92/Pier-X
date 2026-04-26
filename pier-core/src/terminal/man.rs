//! Man-page summary parser for the smart-mode help popover.
//!
//! Smart Mode's `Ctrl+Shift+M` shortcut grabs the SYNOPSIS /
//! DESCRIPTION / OPTIONS sections of a command's man page so the
//! user can skim usage without leaving the terminal. We deliberately
//! parse the rendered output of `man -P cat <cmd>` rather than the
//! groff source — the pager-rendered form is what the user sees on
//! the command line, and it's available on every Unix system Pier-X
//! ships on without a new dependency.
//!
//! Behaviour summary:
//!
//! * spawn `man -P cat <cmd>` with a 1-second timeout. Anything
//!   longer almost always means the man invocation is hung waiting
//!   for input or an external pager — we kill the child and fall
//!   through to the `--help` path.
//! * if `man` returns nothing (or fails to spawn at all), try
//!   `<cmd> --help`. Most modern CLIs follow GNU conventions and
//!   output a usable summary on stderr+stdout.
//! * Windows skips `man` entirely and goes straight to `--help`.
//! * results are cached in a process-wide TTL/LRU map so a user who
//!   pops the popover for the same command twice in a session
//!   doesn't re-spawn anything.
//!
//! The parser intentionally errs on the side of returning whatever
//! it could extract: a command with no SYNOPSIS section but a
//! useful `--help` blurb still produces a `ManSynopsis` with an
//! empty synopsis and the help text in `description`. The popover
//! handles each section independently.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

/// One option entry parsed out of a man page's OPTIONS section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManOption {
    /// Flag(s) as they appear in the man page, e.g. `-l`, `--help`,
    /// or `-a, --all`. Verbatim from the source line so the popover
    /// can render them with their canonical formatting.
    pub flag: String,
    /// One-line summary of what the flag does. Multi-line man
    /// descriptions are joined into a single space-separated
    /// paragraph for display in a single popover row.
    pub summary: String,
}

/// Result of [`man_synopsis`]. Any of the three fields can be empty
/// (e.g. a command with no OPTIONS section). The caller decides
/// whether an empty synopsis is worth showing — we don't gate on it
/// so a `--help`-only fallback still produces a usable result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManSynopsis {
    /// Extracted SYNOPSIS section, joined into a single line.
    pub synopsis: String,
    /// First paragraph of the DESCRIPTION section.
    pub description: String,
    /// Parsed flag rows from the OPTIONS section.
    pub options: Vec<ManOption>,
    /// `"man"` / `"help"` / `""`. The popover shows this as a tiny
    /// muted hint so the user knows whether they're looking at a
    /// real man page or a `--help` fallback.
    pub source: String,
}

/// Errors `man_synopsis` can return. Most callers treat them all
/// the same way (close the popover, no message) so we keep the
/// enum lean.
#[derive(Debug, thiserror::Error)]
pub enum ManError {
    /// Command name is empty or contains shell metacharacters that
    /// make it unsafe to spawn.
    #[error("command name is empty or invalid")]
    InvalidName,
    /// Command lookup found no usable text from `man` or `--help`.
    /// This includes `man: no entry for X` followed by a `--help`
    /// that returned a non-zero exit status with empty stdout.
    #[error("no man page or --help output available for {0}")]
    NotFound(String),
    /// Underlying I/O error (process spawn, pipe read, etc.).
    #[error("man lookup I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Look up `cmd`'s man page summary. Cached for [`CACHE_TTL`] across
/// the lifetime of the process; safe to call freely from the UI on
/// each Ctrl+Shift+M without worrying about extra spawns.
pub fn man_synopsis(cmd: &str) -> Result<ManSynopsis, ManError> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return Err(ManError::InvalidName);
    }
    // Defensive: avoid passing anything that could itself be parsed
    // as a flag or shell metachar by `man`. We intentionally don't
    // try to escape — the popover only ever deals with plain
    // command names from the shell lexer.
    if trimmed.contains([' ', '\t', '|', ';', '&', '<', '>', '`', '$', '\n']) {
        return Err(ManError::InvalidName);
    }

    if let Some(cached) = cache_get(trimmed) {
        return Ok(cached);
    }

    let result = lookup_uncached(trimmed)?;
    cache_put(trimmed.to_string(), result.clone());
    Ok(result)
}

const TIMEOUT: Duration = Duration::from_millis(1000);
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const CACHE_CAPACITY: usize = 100;

fn lookup_uncached(cmd: &str) -> Result<ManSynopsis, ManError> {
    // On Unix, try `man -P cat <cmd>` first. The `-P cat` flag tells
    // man to skip its pager and dump straight to stdout — without it
    // we'd be stuck waiting on `less` to exit.
    #[cfg(unix)]
    {
        let mut man_cmd = Command::new("man");
        man_cmd.args(["-P", "cat", cmd]);
        // LANG=C reduces translated section headers ("BESCHREIBUNG"
        // → "DESCRIPTION") which would otherwise break the parser.
        man_cmd.env("LANG", "C");
        man_cmd.env("LC_ALL", "C");
        if let Ok(output) = run_with_timeout(man_cmd, TIMEOUT) {
            if output.status.success() && !output.stdout.is_empty() {
                let text = strip_overstriking(&output.stdout);
                let parsed = parse_sections(&text, "man");
                if !parsed.is_empty_meaningful() {
                    return Ok(parsed);
                }
            }
        }
    }

    // `--help` fallback. Many GNU tools print to stdout; some BSD-
    // flavoured ones go to stderr. We capture both and concat.
    let mut help_cmd = Command::new(cmd);
    help_cmd.arg("--help");
    if let Ok(output) = run_with_timeout(help_cmd, TIMEOUT) {
        // Don't gate on exit status — many CLIs exit non-zero on
        // --help (e.g. `tar --help` returns 0 but `find --help`
        // returns 1 in some BSD versions). What matters is whether
        // we got usable text.
        let mut combined = output.stdout.clone();
        combined.extend_from_slice(&output.stderr);
        if !combined.is_empty() {
            let text = String::from_utf8_lossy(&combined).into_owned();
            let parsed = parse_sections(&text, "help");
            if !parsed.is_empty_meaningful() {
                return Ok(parsed);
            }
        }
    }

    Err(ManError::NotFound(cmd.to_string()))
}

/// Spawn `cmd` and return its `Output`, killing the child if it
/// hasn't exited inside `timeout`. We poll `try_wait` on a 25ms
/// cadence — short enough that timeout slop stays under the user's
/// perceptual threshold, slow enough that the polling loop costs
/// effectively nothing.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<std::process::Output, ManError> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().map_err(ManError::from);
        }
        if start.elapsed() > timeout {
            // Best effort — if kill fails the OS will reap the
            // zombie when the process eventually exits. We don't
            // wait on it to avoid blocking the popover request.
            let _ = child.kill();
            return Err(ManError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "man lookup exceeded 1s deadline",
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Strip groff overstriking (`X\bX` for bold, `_\bX` for underline)
/// out of `bytes`. The terminal-rendering form of man pages emits
/// these sequences on every styled character; without removing them
/// the section headers come out as `S\bSY\bYN\bNO\bOP\bPS\bSI\bIS\bS`.
fn strip_overstriking(bytes: &[u8]) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // Pattern: keep byte at i+2 when bytes[i+1] == 0x08 (BS).
        // The first byte is either the same as the third (bold) or
        // an underscore (underline) — either way, the third byte is
        // the visible glyph.
        if i + 2 < bytes.len() && bytes[i + 1] == 0x08 {
            out.push(bytes[i + 2]);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl ManSynopsis {
    fn empty(source: impl Into<String>) -> Self {
        Self {
            synopsis: String::new(),
            description: String::new(),
            options: Vec::new(),
            source: source.into(),
        }
    }
    /// True when at least one of the three sections has content
    /// the popover would actually render. Used to decide whether
    /// to fall back to `--help` when the man invocation succeeded
    /// but produced an empty result.
    fn is_empty_meaningful(&self) -> bool {
        self.synopsis.is_empty() && self.description.is_empty() && self.options.is_empty()
    }
}

/// Parse the rendered text of a man page (or `--help` blurb) into
/// our three-section summary. Section headers are detected as fully
/// uppercase words at the very start of a line; this is the
/// convention every man page on Unix uses.
fn parse_sections(text: &str, source: &str) -> ManSynopsis {
    let mut out = ManSynopsis::empty(source);

    // Split into sections keyed by their heading. Headers are flush
    // left, ALL CAPS, no leading whitespace. Body lines below them
    // are typically indented with 7 spaces (man's default).
    let mut current_header: Option<String> = None;
    let mut buffer: Vec<&str> = Vec::new();
    let mut sections: Vec<(String, String)> = Vec::new();

    for line in text.lines() {
        if is_section_header(line) {
            if let Some(h) = current_header.take() {
                sections.push((h, buffer.join("\n")));
                buffer.clear();
            }
            current_header = Some(line.trim().to_string());
        } else {
            buffer.push(line);
        }
    }
    if let Some(h) = current_header.take() {
        sections.push((h, buffer.join("\n")));
    }

    // Pluck the headers we care about. The `--help` path doesn't
    // typically have section headers — handled below.
    for (name, body) in &sections {
        let upper = name.to_ascii_uppercase();
        let body_clean = dedent_and_collapse(body);
        if upper == "SYNOPSIS" && out.synopsis.is_empty() {
            out.synopsis = body_clean;
        } else if (upper == "DESCRIPTION" || upper == "USAGE") && out.description.is_empty() {
            out.description = body_clean;
        } else if upper == "OPTIONS" && out.options.is_empty() {
            out.options = parse_options(body);
        }
    }

    // `--help` path — no section headers. Treat the whole text as
    // a description and try to extract option lines from it. The
    // synopsis is best-effort: many `--help` outputs start with a
    // `Usage:` / `usage:` line.
    if sections.is_empty() {
        let mut lines = text.lines();
        if let Some(first) = lines.next() {
            let trimmed = first.trim_start();
            if trimmed.to_ascii_lowercase().starts_with("usage:") {
                out.synopsis = trimmed
                    .trim_start_matches("Usage:")
                    .trim_start_matches("usage:")
                    .trim()
                    .to_string();
            }
        }
        out.description = dedent_and_collapse(text);
        out.options = parse_options(text);
    }

    out
}

/// Section headers in a rendered man page are ALL-CAPS (with optional
/// digits and spaces) words flush against the left margin.
fn is_section_header(line: &str) -> bool {
    if line.is_empty() || line.starts_with(' ') || line.starts_with('\t') {
        return false;
    }
    let trimmed = line.trim_end();
    if trimmed.is_empty() {
        return false;
    }
    // Heuristic: at least one alphabetic char, all alphabetic chars
    // uppercase, only `[A-Z 0-9 ]` overall. Matches "SYNOPSIS",
    // "EXIT STATUS", "SEE ALSO", "DESCRIPTION 1", but NOT typical
    // body text.
    let mut has_alpha = false;
    for c in trimmed.chars() {
        if c.is_ascii_alphabetic() {
            has_alpha = true;
            if !c.is_ascii_uppercase() {
                return false;
            }
        } else if !(c.is_ascii_digit() || c == ' ' || c == '-') {
            return false;
        }
    }
    has_alpha
}

/// Trim a fixed indent (man-page bodies use 7 spaces by default),
/// collapse runs of blank lines, and join into a single string with
/// preserved paragraph breaks.
fn dedent_and_collapse(body: &str) -> String {
    let mut out = String::new();
    let mut last_blank = false;
    for line in body.lines() {
        let stripped = line.trim_start();
        if stripped.is_empty() {
            if !last_blank && !out.is_empty() {
                out.push('\n');
            }
            last_blank = true;
        } else {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push(' ');
            }
            if last_blank && !out.is_empty() {
                out.push('\n');
            }
            out.push_str(stripped);
            last_blank = false;
        }
    }
    out.trim().to_string()
}

/// Walk `body` looking for option-style lines and pair each with
/// the indented description that follows. We accept any leading
/// indent so this works on both man bodies (7-space indent) and
/// `--help` outputs (often 2-space).
fn parse_options(body: &str) -> Vec<ManOption> {
    let mut out: Vec<ManOption> = Vec::new();
    let lines: Vec<&str> = body.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some((indent, rest)) = leading_indent(line) {
            if rest.starts_with('-') {
                if let Some((flag, inline_summary)) = split_option_line(rest) {
                    let mut summary = inline_summary.trim().to_string();
                    // Description on a deeper-indented continuation
                    // line — common on man pages where the flag and
                    // the description go on separate lines.
                    let mut j = i + 1;
                    while j < lines.len() {
                        let next = lines[j];
                        if let Some((next_indent, next_rest)) = leading_indent(next) {
                            if next_indent > indent && !next_rest.starts_with('-') {
                                if !summary.is_empty() {
                                    summary.push(' ');
                                }
                                summary.push_str(next_rest.trim());
                                j += 1;
                                continue;
                            }
                        } else if next.trim().is_empty() {
                            // Single blank line is part of the
                            // description in many man pages.
                            j += 1;
                            continue;
                        }
                        break;
                    }
                    if !flag.is_empty() {
                        out.push(ManOption {
                            flag,
                            summary: summary.trim().to_string(),
                        });
                    }
                    i = j;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Return `(indent_count, rest_of_line)` where `indent_count` is the
/// number of leading whitespace columns. `None` for empty lines.
fn leading_indent(line: &str) -> Option<(usize, &str)> {
    if line.trim().is_empty() {
        return None;
    }
    let indent = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
    Some((indent, &line[indent..]))
}

/// Split an option header line like `-a, --all  do not list .` into
/// `("-a, --all", "do not list .")`. The flag/summary boundary is
/// the first run of two-or-more whitespace characters; that's the
/// universal man / `--help` convention. Returns `None` if the line
/// doesn't look like an option (e.g. starts with `--` followed by
/// something weird, or has no recognisable flag).
fn split_option_line(rest: &str) -> Option<(String, String)> {
    let bytes = rest.as_bytes();
    // Scan to the first whitespace run >= 2 chars.
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b' ' || bytes[i] == b'\t' {
            // Count run length
            let mut j = i;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            if j - i >= 2 || j == bytes.len() {
                let flag = rest[..i].to_string();
                let desc = rest[j..].to_string();
                if flag.starts_with('-') {
                    return Some((flag, desc));
                }
                return None;
            }
            i = j;
            continue;
        }
        i += 1;
    }
    // No whitespace found — entire line is the flag, no summary
    if rest.starts_with('-') {
        Some((rest.to_string(), String::new()))
    } else {
        None
    }
}

// ── Cache ──────────────────────────────────────────────────────────

struct CacheEntry {
    written_at: Instant,
    value: ManSynopsis,
}

fn cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_get(key: &str) -> Option<ManSynopsis> {
    let guard = cache().lock().ok()?;
    let entry = guard.get(key)?;
    if entry.written_at.elapsed() > CACHE_TTL {
        return None;
    }
    Some(entry.value.clone())
}

fn cache_put(key: String, value: ManSynopsis) {
    let mut guard = match cache().lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if guard.len() >= CACHE_CAPACITY {
        // Evict the single oldest entry. Cheaper than maintaining a
        // separate access-order list given our 100-entry cap; the
        // O(n) min walk is microseconds.
        if let Some(oldest_key) = guard
            .iter()
            .min_by_key(|(_, e)| e.written_at)
            .map(|(k, _)| k.clone())
        {
            guard.remove(&oldest_key);
        }
    }
    guard.insert(
        key,
        CacheEntry {
            written_at: Instant::now(),
            value,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_name_rejected() {
        assert!(matches!(man_synopsis(""), Err(ManError::InvalidName)));
        assert!(matches!(man_synopsis("   "), Err(ManError::InvalidName)));
        assert!(matches!(man_synopsis("ls; rm"), Err(ManError::InvalidName)));
        assert!(matches!(
            man_synopsis("foo bar"),
            Err(ManError::InvalidName)
        ));
        assert!(matches!(man_synopsis("a$b"), Err(ManError::InvalidName)));
    }

    #[test]
    fn strip_overstriking_collapses_man_styling() {
        // `S\bSY\bY` -> `SY` (bold rendering of `SY`)
        let input: Vec<u8> = vec![b'S', 0x08, b'S', b'Y', 0x08, b'Y', b' ', b'_', 0x08, b'X'];
        let out = strip_overstriking(&input);
        assert_eq!(out, "SY X");
    }

    #[test]
    fn detects_section_headers() {
        assert!(is_section_header("SYNOPSIS"));
        assert!(is_section_header("EXIT STATUS"));
        assert!(is_section_header("SEE ALSO"));
        assert!(!is_section_header("       -a")); // indented body
        assert!(!is_section_header("Usage:"));
        assert!(!is_section_header(""));
    }

    #[test]
    fn parse_options_handles_two_line_format() {
        let body = "       -a\n              Show all files.\n       -l, --long\n              Use long format.\n";
        let opts = parse_options(body);
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].flag, "-a");
        assert_eq!(opts[0].summary, "Show all files.");
        assert_eq!(opts[1].flag, "-l, --long");
        assert_eq!(opts[1].summary, "Use long format.");
    }

    #[test]
    fn parse_options_handles_inline_format() {
        // `--help` style: flag and description on same line
        let body = "  -a, --all   do not ignore entries starting with .\n  -h          help text\n";
        let opts = parse_options(body);
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].flag, "-a, --all");
        assert!(opts[0].summary.contains("do not ignore"));
        assert_eq!(opts[1].flag, "-h");
        assert_eq!(opts[1].summary, "help text");
    }

    #[test]
    fn parse_sections_extracts_synopsis_and_description() {
        let text = "\
NAME
       ls - list directory contents

SYNOPSIS
       ls [OPTION]... [FILE]...

DESCRIPTION
       List information about the FILEs.

       Sort entries alphabetically.

OPTIONS
       -a
              Do not ignore entries starting with .
";
        let parsed = parse_sections(text, "man");
        assert!(parsed.synopsis.contains("ls [OPTION]"));
        assert!(parsed.description.contains("List information"));
        assert_eq!(parsed.source, "man");
        assert_eq!(parsed.options.len(), 1);
        assert_eq!(parsed.options[0].flag, "-a");
    }

    #[test]
    fn parse_sections_handles_help_only_with_usage() {
        let text = "Usage: tool [options] file\n\n  -v   Verbose\n  -h   Help\n";
        let parsed = parse_sections(text, "help");
        assert_eq!(parsed.synopsis, "tool [options] file");
        assert_eq!(parsed.options.len(), 2);
        assert_eq!(parsed.source, "help");
    }

    #[test]
    fn cache_round_trip() {
        let key = format!("___test_marker_{}", std::process::id());
        let value = ManSynopsis {
            synopsis: "x".to_string(),
            description: "y".to_string(),
            options: vec![],
            source: "test".to_string(),
        };
        cache_put(key.clone(), value.clone());
        let got = cache_get(&key).expect("cache should hold our entry");
        assert_eq!(got, value);
    }
}
