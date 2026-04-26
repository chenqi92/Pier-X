//! Extract pack data from `man <cmd>` output.
//!
//! Man pages are roff-formatted; `man -P cat <cmd>` renders them
//! to plain text. We then split into sections by `^[A-Z][A-Z ]+$`
//! headers (NAME / SYNOPSIS / DESCRIPTION / OPTIONS / COMMANDS /
//! …) and parse the OPTIONS / COMMANDS sections only.
//!
//! ## Format we expect
//!
//! ```text
//! OPTIONS
//!        -t, --tag tag
//!               Name and optionally a tag in the 'name:tag' format.
//!
//!        -q, --quiet
//!               Suppress non-error messages.
//! ```
//!
//! Indentation drives the parse: a flag line starts at column 8
//! (one tab indent), the description follows on lines indented
//! deeper. We collapse the description into a single line.
//!
//! ## Subcommand sections
//!
//! Some pages have a `COMMANDS` or `SUBCOMMANDS` section with the
//! same indented shape; we treat unmarked names there as
//! subcommands.

use std::process::Command;

use crate::schema::{CommandPack, OptionEntry, SubcommandEntry};

pub fn extract(cmd: &str) -> Result<CommandPack, String> {
    let out = Command::new("man")
        .args(["-P", "cat", cmd])
        .output()
        .map_err(|e| format!("spawn `man -P cat {cmd}`: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`man {cmd}` exited {}",
            out.status.code().unwrap_or(-1)
        ));
    }
    let body = strip_backspace_overstrike(&String::from_utf8_lossy(&out.stdout));
    let mut pack = CommandPack::default();
    pack.command = cmd.to_string();
    parse_into(&body, &mut pack);
    if pack.subcommands.is_empty() && pack.options.is_empty() {
        return Err(format!("man `{cmd}` had no recognised entries"));
    }
    Ok(pack)
}

/// Old-style man pages render bold/underline as `X\bX` /
/// `_\bX` overstrike pairs. The final text we want is just the
/// printable character; `man -P cat` doesn't always strip it.
fn strip_backspace_overstrike(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_char: Option<char> = None;
    for c in s.chars() {
        if c == '\u{08}' {
            // Backspace — drop the previous char from output and
            // forget it; next char will be the "real" one.
            if !out.is_empty() {
                out.pop();
            }
            prev_char = None;
            continue;
        }
        out.push(c);
        prev_char = Some(c);
    }
    let _ = prev_char;
    out
}

/// Walk the body, switching parsing state on section headers.
fn parse_into(body: &str, pack: &mut CommandPack) {
    let mut state = State::None;
    let mut current: Option<Pending> = None;
    for raw in body.lines() {
        if let Some(next) = section_header(raw) {
            flush(&mut current, &mut state.clone(), pack);
            state = next;
            continue;
        }
        match state {
            State::None | State::Other => {}
            State::Options | State::Commands => {
                let leading = leading_spaces(raw);
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    flush(&mut current, &mut state.clone(), pack);
                    continue;
                }
                // Heuristic: a row starting at column 0..=10 is a
                // new entry. Deeper indentation continues the
                // description of the previous entry.
                if leading <= 10 {
                    flush(&mut current, &mut state.clone(), pack);
                    current = Some(Pending {
                        head: trimmed.to_string(),
                        body: String::new(),
                    });
                } else if let Some(p) = current.as_mut() {
                    if !p.body.is_empty() {
                        p.body.push(' ');
                    }
                    p.body.push_str(trimmed);
                }
            }
        }
    }
    flush(&mut current, &mut state, pack);
}

#[derive(Clone, Copy)]
enum State {
    None,
    Options,
    Commands,
    Other,
}

struct Pending {
    head: String,
    body: String,
}

fn section_header(raw: &str) -> Option<State> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains(' ') {
        // Section headers are a single ALL-CAPS word at column 0
        // for nroff output. Multi-word "GLOBAL OPTIONS" is also
        // possible — handled below.
    }
    if !raw.starts_with(|c: char| c.is_ascii_uppercase()) {
        return None;
    }
    let body = raw.trim();
    if body.is_empty() {
        return None;
    }
    // All-caps section header (allow spaces).
    if !body.chars().all(|c| c.is_ascii_uppercase() || c == ' ') {
        return None;
    }
    let lower = body.to_ascii_lowercase();
    Some(match lower.as_str() {
        "options" | "global options" | "general options" | "common options" => State::Options,
        "commands" | "subcommands" | "main porcelain commands" => State::Commands,
        _ => State::Other,
    })
}

fn flush(current: &mut Option<Pending>, state: &mut State, pack: &mut CommandPack) {
    let Some(pending) = current.take() else {
        return;
    };
    match state {
        State::Options => {
            // The first whitespace splits the flag list from any
            // value placeholder ("-t, --tag tag"). Drop the
            // placeholder.
            let head = pending.head.clone();
            let mut head_parts = head.splitn(2, |c: char| c == ' ' || c == '\t');
            let raw_flag = head_parts.next().unwrap_or("");
            // Look for `, --long` continuation on the same line.
            let flag = if let Some(rest) = head_parts.next() {
                if rest.starts_with(',') || rest.starts_with("--") {
                    // Take everything up to the next placeholder
                    // word (lowercase letter starting a token after
                    // a space).
                    let combined = format!("{raw_flag} {}", rest);
                    let cleaned: String = combined
                        .split_whitespace()
                        .filter(|tok| tok.starts_with('-') || *tok == ",")
                        .collect::<Vec<_>>()
                        .join(" ");
                    cleaned.replace(" ,", ",")
                } else {
                    raw_flag.to_string()
                }
            } else {
                raw_flag.to_string()
            };
            if !flag.is_empty() && flag.starts_with('-') {
                pack.options.push(OptionEntry::with_en(flag, pending.body.trim()));
            }
        }
        State::Commands => {
            let name = pending
                .head
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() && !name.starts_with('-') {
                pack.subcommands
                    .push(SubcommandEntry::with_en(name, pending.body.trim()));
            }
        }
        _ => {}
    }
}

fn leading_spaces(line: &str) -> usize {
    let mut n = 0;
    for c in line.chars() {
        if c == ' ' {
            n += 1;
        } else if c == '\t' {
            // Tab → 8 spaces, matching what `man` does.
            n += 8;
        } else {
            break;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_options_section_with_continuation_lines() {
        let body = "\
NAME
       cmd - do thing

OPTIONS
       -t, --tag tag
              Name and optionally a tag in the 'name:tag' format.

       -q, --quiet
              Suppress non-error messages.
";
        let mut pack = CommandPack::default();
        parse_into(body, &mut pack);
        assert!(pack
            .options
            .iter()
            .any(|o| o.flag.starts_with("-t") && o.i18n.get("en").unwrap().contains("tag")));
        assert!(pack
            .options
            .iter()
            .any(|o| o.flag.starts_with("-q") && o.i18n.get("en").unwrap().contains("Suppress")));
    }

    #[test]
    fn strips_overstrike_pairs() {
        let raw = "b\u{08}bo\u{08}ol\u{08}ld\u{08}d";
        assert_eq!(strip_backspace_overstrike(raw), "bold");
    }
}
