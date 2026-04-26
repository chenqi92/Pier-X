//! Extract pack data from `<cmd> --help` output.
//!
//! Generic last-resort parser. Recognises the layouts most
//! Rust/Go/Python CLI frameworks emit:
//!
//!   * `Usage: <cmd> [OPTIONS] <COMMAND>` lines
//!   * `Commands:` / `SUBCOMMANDS:` sections (clap, cobra)
//!   * `Options:` / `Flags:` sections (clap, argparse, cobra)
//!
//! Each section is line-oriented:
//!
//! ```text
//! Commands:
//!   build    Build an image from a Dockerfile
//!   push     Upload an image to a registry
//! ```
//!
//! ```text
//! Options:
//!   -t, --tag <TAG>    Tag the image
//!   -q, --quiet        Suppress non-error messages
//! ```
//!
//! We split each line on the first run of two-or-more spaces:
//! everything before is the name/flag, everything after is the
//! description. Lines that don't fit the pattern are skipped, not
//! errors.

use std::process::Command;

use crate::schema::{CommandPack, OptionEntry, SubcommandEntry};
use crate::timeout::{run_with_timeout, DEFAULT_TIMEOUT};

pub fn extract(cmd: &str) -> Result<CommandPack, String> {
    let body = run_help(cmd)?;
    let mut pack = CommandPack::default();
    pack.command = cmd.to_string();
    parse_into(&body, &mut pack);
    if pack.subcommands.is_empty() && pack.options.is_empty() {
        return Err(format!("`{cmd} --help` had no recognised entries"));
    }
    Ok(pack)
}

fn run_help(cmd: &str) -> Result<String, String> {
    // Try `--help` first, then `-h` (some old Unix tools).
    for arg in ["--help", "-h"] {
        let mut command = Command::new(cmd);
        command.arg(arg);
        let out = match run_with_timeout(command, DEFAULT_TIMEOUT) {
            Ok(o) => o,
            Err(e) => return Err(format!("spawn `{cmd} {arg}`: {e}")),
        };
        // Some tools emit help to stderr (BSD ls) — combine.
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !stdout.is_empty() {
            return Ok(stdout.into_owned());
        }
        if !stderr.is_empty() {
            return Ok(stderr.into_owned());
        }
    }
    Err(format!("`{cmd}` produced no help output on either flag"))
}

fn parse_into(body: &str, pack: &mut CommandPack) {
    let mut section = Section::None;
    for raw in body.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(next) = section_header(line) {
            section = next;
            continue;
        }
        match section {
            Section::None | Section::Other => {}
            Section::Commands => {
                if let Some(entry) = parse_two_column(line) {
                    if !entry.name.starts_with('-') {
                        pack.subcommands.push(SubcommandEntry::with_en(
                            entry.name,
                            entry.desc,
                        ));
                    }
                }
            }
            Section::Options => {
                if let Some(entry) = parse_two_column(line) {
                    if entry.name.starts_with('-') {
                        pack.options.push(OptionEntry::with_en(
                            normalise_flag(&entry.name),
                            entry.desc,
                        ));
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Section {
    None,
    Commands,
    Options,
    /// Unknown / skipped section — `EXAMPLES`, `ENVIRONMENT`, etc.
    Other,
}

fn section_header(line: &str) -> Option<Section> {
    let trimmed = line.trim();
    let lower = trimmed.to_ascii_lowercase();
    let lower = lower.trim_end_matches(':');
    match lower {
        "commands" | "subcommands" | "available commands" | "common commands"
        | "main porcelain commands" => Some(Section::Commands),
        "options" | "flags" | "global flags" | "global options" => Some(Section::Options),
        // Anything that *looks* like a header (single line, ends
        // with `:`, ALL CAPS, or starts at column 0 after blank
        // lines) but isn't ours → flip to Other so we don't keep
        // appending to the previous section.
        h if (h.is_empty() || h.contains(' '))
            && line.ends_with(':')
            && !line.starts_with(' ') =>
        {
            Some(Section::Other)
        }
        _ => None,
    }
}

struct TwoCol {
    name: String,
    desc: String,
}

/// Split a body line into `(name, description)` by the first run
/// of ≥2 spaces. Returns `None` for lines that don't match.
fn parse_two_column(line: &str) -> Option<TwoCol> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    // Find the first run of 2+ spaces.
    let mut i = 0;
    let bytes = trimmed.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b' ' && i + 1 < bytes.len() && bytes[i + 1] == b' ' {
            let name = trimmed[..i].trim();
            let rest = trimmed[i..].trim_start();
            if name.is_empty() || rest.is_empty() {
                return None;
            }
            return Some(TwoCol {
                name: name.to_string(),
                desc: rest.to_string(),
            });
        }
        i += 1;
    }
    None
}

/// Normalise `-t, --tag <TAG>` → `-t, --tag` (drop the value
/// placeholder so the popover row stays clean).
///
/// Algorithm: walk tokens left-to-right. A token is "part of the
/// flag list" if it starts with `-` or is exactly `,`. The first
/// token that's neither (typically a placeholder `<TAG>` /
/// `[VALUE]` / a bare lowercase word) terminates the list.
fn normalise_flag(raw: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for tok in raw.split_whitespace() {
        if tok.starts_with('-') || tok == "," || tok == ",," {
            kept.push(tok);
        } else if tok.starts_with(",-") {
            // `-q,--quiet` collapses into one token in some help
            // outputs. Keep it.
            kept.push(tok);
        } else {
            break;
        }
    }
    let joined = kept.join(" ");
    // Collapse `"-t , --tag"` → `"-t, --tag"`. Also collapse a
    // hanging `=` from `--config=<FILE>` style.
    let cleaned = joined.replace(" , ", ", ");
    cleaned.trim_end_matches(',').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clap_style_commands_and_options() {
        let body = "\
Usage: tool [OPTIONS] <COMMAND>

Commands:
  build    Build the project
  test     Run tests

Options:
  -v, --verbose    Enable verbose output
  -q, --quiet      Suppress output
";
        let mut pack = CommandPack::default();
        parse_into(body, &mut pack);
        assert_eq!(pack.subcommands.len(), 2);
        assert_eq!(pack.subcommands[0].name, "build");
        assert!(pack.options.iter().any(|o| o.flag == "-v, --verbose"));
    }

    #[test]
    fn normalise_flag_drops_value_placeholder() {
        assert_eq!(normalise_flag("-t, --tag <TAG>"), "-t, --tag");
        // `--config=<FILE>` is one token (no whitespace) so the
        // splitter keeps it intact. Future iteration could trim
        // the `=<...>` tail; for now we keep it as-is.
        assert_eq!(normalise_flag("--config=<FILE>"), "--config=<FILE>");
        assert_eq!(normalise_flag("-q"), "-q");
    }

    #[test]
    fn parse_two_column_requires_two_spaces() {
        let one = parse_two_column("  build single-space description");
        // Single space between name and description means we treat
        // the whole thing as a name — better to skip than to lose
        // half the description.
        assert!(one.is_none() || one.as_ref().unwrap().desc.is_empty());
        let two = parse_two_column("  build    real description").unwrap();
        assert_eq!(two.name, "build");
        assert_eq!(two.desc, "real description");
    }
}
