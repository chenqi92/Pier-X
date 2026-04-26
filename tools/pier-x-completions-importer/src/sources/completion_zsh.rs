//! Extract pack data from `<cmd> completion zsh` output.
//!
//! Modern CLIs (kubectl, docker, helm, gh, fluxctl, npm, yarn,
//! pnpm, cargo, rustup, doctl, oc, kustomize, k0s, kind, …) ship a
//! `completion <shell>` subcommand that emits a fully-structured
//! shell script for completion. The zsh form is most useful to us
//! because it uses the `_arguments` / `_describe` / `compdef`
//! pattern with **descriptions inline**, often verbatim copies of
//! `--help` text.
//!
//! ## What we recognise
//!
//! Two common shapes show up in the wild. Both come from
//! `compgen` / `cobra`-style generators:
//!
//! ### Shape A — `_describe` blocks
//!
//! ```sh
//! local -a commands
//! commands=(
//!     "build:Build an image from a Dockerfile"
//!     "push:Upload an image to a registry"
//! )
//! _describe -t commands "command" commands
//! ```
//!
//! ### Shape B — inline `_arguments`
//!
//! ```sh
//! _arguments \
//!   '--debug[Enable debug mode]' \
//!   '(-h --help)'{-h,--help}'[Help for docker]' \
//!   '*::: :->args'
//! ```
//!
//! Both are line-oriented enough that a regex-driven scanner gets
//! us most of the data. Anything we can't parse cleanly is
//! skipped; the resulting pack just has fewer rows but stays
//! valid.

use std::process::Command;

use crate::schema::{CommandPack, OptionEntry, SubcommandEntry};
use crate::timeout::{run_with_timeout, DEFAULT_TIMEOUT, SKIP_COMPLETION_ZSH};

pub fn extract(cmd: &str) -> Result<CommandPack, String> {
    if SKIP_COMPLETION_ZSH.contains(&cmd) {
        return Err(format!(
            "skipping `{cmd} completion zsh` — known to drop into interactive mode"
        ));
    }
    let mut command = Command::new(cmd);
    command.args(["completion", "zsh"]);
    let out = run_with_timeout(command, DEFAULT_TIMEOUT)
        .map_err(|e| format!("spawn `{cmd} completion zsh`: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`{cmd} completion zsh` exited {}",
            out.status.code().unwrap_or(-1)
        ));
    }
    let body = String::from_utf8_lossy(&out.stdout);
    let mut pack = CommandPack::default();
    pack.command = cmd.to_string();
    pack.subcommands = parse_describe_blocks(&body);
    pack.options = parse_argument_flags(&body);
    if pack.subcommands.is_empty() && pack.options.is_empty() {
        return Err(format!("zsh script for `{cmd}` had no recognised entries"));
    }
    Ok(pack)
}

/// Parse `commands=( "name:desc" "name:desc" )` style blocks.
/// Tolerant of extra whitespace, single + double quotes, and
/// `\:` escapes inside the description.
fn parse_describe_blocks(body: &str) -> Vec<SubcommandEntry> {
    let mut out: Vec<SubcommandEntry> = Vec::new();
    let mut in_block = false;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.starts_with("commands=(") || line.contains("=(") && line.ends_with('(') {
            in_block = true;
            continue;
        }
        if in_block && line.starts_with(')') {
            in_block = false;
            continue;
        }
        if !in_block {
            // Some generators emit a one-liner: _describe args
            // alongside a literal `'name:desc' 'name:desc'` list.
            // Catch those by scanning for repeated `'<name>:`.
            for entry in scan_quoted_pairs(line) {
                if seen.insert(entry.name.clone()) {
                    out.push(entry);
                }
            }
            continue;
        }
        if let Some(entry) = parse_quoted_entry(line) {
            if seen.insert(entry.name.clone()) {
                out.push(entry);
            }
        }
    }
    out
}

/// Parse one `"name:description"` line. Returns `None` for
/// comments / non-matching syntax.
fn parse_quoted_entry(line: &str) -> Option<SubcommandEntry> {
    let unquoted = strip_outer_quotes(line.trim().trim_end_matches(','))?;
    // Split on the first un-escaped colon. zsh escapes inner
    // colons as `\:`; we honour that.
    let (name, desc) = split_unescaped_colon(unquoted)?;
    let name = name.trim().to_string();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    Some(SubcommandEntry::with_en(name, desc.trim()))
}

fn scan_quoted_pairs(line: &str) -> Vec<SubcommandEntry> {
    // Greedy scan for `'name:desc'` or `"name:desc"`.
    let mut out = Vec::new();
    let mut chars = line.char_indices().peekable();
    while let Some(&(i, c)) = chars.peek() {
        if c == '\'' || c == '"' {
            let quote = c;
            chars.next();
            let start = i + c.len_utf8();
            let mut end = start;
            while let Some(&(j, nc)) = chars.peek() {
                end = j;
                if nc == quote {
                    chars.next();
                    break;
                }
                chars.next();
            }
            if end > start {
                let candidate = &line[start..end];
                if let Some((name, desc)) = split_unescaped_colon(candidate) {
                    let n = name.trim();
                    if !n.is_empty() && !n.contains(char::is_whitespace) {
                        out.push(SubcommandEntry::with_en(n, desc.trim()));
                    }
                }
            }
        } else {
            chars.next();
        }
    }
    out
}

fn strip_outer_quotes(s: &str) -> Option<&str> {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        Some(&s[1..s.len() - 1])
    } else {
        None
    }
}

/// Split on the first colon that isn't escaped with a backslash.
fn split_unescaped_colon(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' && (i == 0 || bytes[i - 1] != b'\\') {
            return Some((&s[..i], &s[i + 1..]));
        }
        i += 1;
    }
    None
}

/// Pull `--flag[Description]` style entries out of `_arguments`
/// blocks. We don't try to track which subcommand each flag
/// belongs to — `pack.options` carries the union; per-subcommand
/// scoping is a follow-up.
fn parse_argument_flags(body: &str) -> Vec<OptionEntry> {
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for raw in body.lines() {
        let line = raw.trim();
        // Every variation we care about contains a `[` after the
        // flag name and before the description. Skip lines without
        // the marker.
        if !line.contains('[') {
            continue;
        }
        for entry in scan_argument_line(line) {
            if seen.insert(entry.flag.clone()) {
                out.push(entry);
            }
        }
    }
    out
}

fn scan_argument_line(line: &str) -> Vec<OptionEntry> {
    let mut out = Vec::new();
    // The pattern we hunt for: `--flag` / `-f` / `(-h --help)`
    // followed by `[Description]`.
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Find the next `[` — that's the description opener.
        let bracket = match line[i..].find('[') {
            Some(n) => i + n,
            None => break,
        };
        let close = match line[bracket..].find(']') {
            Some(n) => bracket + n,
            None => break,
        };
        let desc = &line[bracket + 1..close];
        // Walk backward from `bracket` to recover the flag(s).
        let flag = recover_flag_before(&line[..bracket]);
        if let Some(flag) = flag {
            // Filter out clearly non-flag tokens (descriptions
            // sometimes contain `[default: foo]` markers we don't
            // want to harvest as options).
            if flag.starts_with('-') {
                out.push(OptionEntry::with_en(
                    normalise_flag(flag),
                    desc.replace('\\', "").trim(),
                ));
            }
        }
        i = close + 1;
    }
    out
}

/// Walk back from the `[` of `--foo[Desc]` to find the flag
/// token. Stops at whitespace, single quote, equals, or comma —
/// whichever delimiter appeared first. The returned slice has any
/// surrounding quotes / equals trimmed off so it's directly usable
/// as a flag token.
fn recover_flag_before(prefix: &str) -> Option<&str> {
    let trimmed = prefix.trim_end_matches(['"', '\'', '=']);
    // Special cases: `(-h --help)` → take the longest form.
    if let Some(open) = trimmed.rfind('(') {
        let group = &trimmed[open + 1..];
        let group = group.trim_end_matches(')');
        return group
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| s.starts_with('-'))
            .max_by_key(|s| s.len());
    }
    // Otherwise the last whitespace-bounded token, stripped of
    // any leading quote/comma so `'--quiet` becomes `--quiet`.
    trimmed
        .rsplit(|c: char| c.is_whitespace() || c == ',')
        .next()
        .map(|s| s.trim_start_matches(|c: char| c == '\'' || c == '"' || c == '='))
        .map(str::trim)
}

/// Strip surrounding braces from `{-h,--help}` and produce the
/// canonical "shortest-form first, longest-form last,
/// comma-separated" presentation.
fn normalise_flag(raw: &str) -> String {
    let inner = raw.trim_matches(|c: char| c == '{' || c == '}' || c == ',' || c.is_whitespace());
    if inner.contains(',') {
        let mut parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        parts.sort_by_key(|s| s.len());
        parts.join(", ")
    } else {
        inner.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_describe_block_with_simple_pairs() {
        let body = r#"
local -a commands
commands=(
    "build:Build an image from a Dockerfile"
    "push:Upload an image to a registry"
)
_describe -t commands "command" commands
"#;
        let subs = parse_describe_blocks(body);
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].name, "build");
        assert_eq!(subs[0].i18n.get("en").unwrap(), "Build an image from a Dockerfile");
    }

    #[test]
    fn parses_argument_flag_with_description() {
        let body = "_arguments '--quiet[Suppress non-error messages]' '--debug[Enable debug mode]'";
        let opts = parse_argument_flags(body);
        let names: Vec<&str> = opts.iter().map(|o| o.flag.as_str()).collect();
        assert!(names.contains(&"--quiet"));
        assert!(names.contains(&"--debug"));
    }

    #[test]
    fn split_unescaped_colon_handles_escaped_colons_in_description() {
        let (name, desc) = split_unescaped_colon("foo:bar\\:baz").unwrap();
        assert_eq!(name, "foo");
        assert_eq!(desc, "bar\\:baz");
    }

    #[test]
    fn normalise_flag_orders_short_then_long() {
        assert_eq!(normalise_flag("{-h,--help}"), "-h, --help");
        assert_eq!(normalise_flag("--help"), "--help");
    }
}
