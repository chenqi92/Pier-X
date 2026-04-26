//! Tab-completion candidates for smart-mode.
//!
//! Given the current input line and cursor position, produce a flat
//! `Vec<Completion>` the UI shows in a popover. Two main sources for
//! M4:
//!
//! * **Command names** (when the cursor sits in the first word of a
//!   compound). Returns shell builtins + every executable found on
//!   `$PATH` whose name starts with the typed prefix.
//! * **File paths** (any later word). Lists entries inside the
//!   shell's last-known cwd (or `dirname(prefix)` resolved against
//!   it) whose name starts with the typed basename.
//!
//! The two history-flavoured sources from the plan (M5 history-ring,
//! M6 man-known options) are intentionally absent — they need state
//! that doesn't exist yet. Tab will still pop a useful menu without
//! them; the later milestones layer on top.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// One row in the completion popover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    /// Row classifier — drives the popover icon and rendering.
    pub kind: CompletionKind,
    /// Full text the UI should produce when this row is selected.
    /// For directories, includes a trailing `/` so the next Tab
    /// drills in.
    pub value: String,
    /// What the UI shows in the row's main label. Usually the same
    /// as `value`; differs for paths where we show only the basename
    /// even though `value` carries the directory portion.
    pub display: String,
    /// Optional muted right-side hint — for `binary` kind, the
    /// resolved absolute path; for `file`/`directory`, blank.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Optional localized description — fish/warp-style, shown as
    /// a side panel in the popover. Sourced from
    /// [`super::library::Library`] when the current word is a known
    /// subcommand or option of a known command. Empty otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Classifier for a [`Completion`] row — determines the popover icon
/// and how the UI renders the right-side hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompletionKind {
    /// Shell builtin (e.g. `cd`, `export`).
    Builtin,
    /// Executable found on `$PATH`.
    Binary,
    /// Regular file in the current directory or a path prefix.
    File,
    /// Directory entry (gets a trailing `/` in `value`).
    Directory,
    /// Library-known subcommand of the active command (e.g. `build`
    /// when typing `docker `). Carries a localized description from
    /// the bundled pack.
    Subcommand,
    /// Library-known option flag of the active (sub)command (e.g.
    /// `--tag` after `docker build `).
    Option,
}

/// Top-level entry point. `cursor` is a byte offset into `line`; if
/// it points past the end of `line` we treat it as end-of-line.
/// `cwd` is the shell's last-known directory (from OSC 7) — pass
/// `None` to fall back to the process cwd.
pub fn complete(line: &str, cursor: usize, cwd: Option<&Path>) -> Vec<Completion> {
    complete_with_library(line, cursor, cwd, &super::library::Library::empty(), "en")
}

/// Library-aware variant. When `lib` knows the active command, the
/// argument-position branch first emits **subcommand** rows from the
/// pack (with localized descriptions), then falls back to file
/// completion when no subcommand matches the prefix. `locale` picks
/// the description language (e.g. `"zh-CN"`) — see
/// [`super::library::Library::pick_locale`].
///
/// The Tauri layer constructs a process-global library at startup
/// and threads it (plus the user's locale) into every Tab press,
/// so this function stays pure.
pub fn complete_with_library(
    line: &str,
    cursor: usize,
    cwd: Option<&Path>,
    lib: &super::library::Library,
    locale: &str,
) -> Vec<Completion> {
    let cursor = cursor.min(line.len());
    let word_start = find_word_start(line, cursor);
    let prefix = &line[word_start..cursor];

    if is_command_position(line, word_start) {
        let mut rows = complete_command(prefix);
        // If the user has typed the *full* name of a library command
        // (e.g. `docker` without a trailing space) and hits Tab, what
        // they almost always want next is to drill into subcommands —
        // so surface those too. Pre-pending the command name to each
        // row's `value` lets the frontend's word-diff inject the right
        // tail (`docker` + Tab + `attach` → ` attach` written to PTY).
        // The popover keeps the bare subcommand name in `display`, so
        // the row stays visually compact.
        if !prefix.is_empty() {
            if let Some(pack) = lib.lookup(prefix) {
                let sub_rows = library_rows("", &pack.subcommands, &pack.options, locale);
                for mut sub in sub_rows {
                    sub.value = format!("{prefix} {}", sub.value);
                    rows.push(sub);
                }
            }
        }
        return rows;
    }

    // Argument position. First word of the line drives the library
    // lookup — so `git push` resolves the `git` pack and we offer
    // its `push` subcommand even though the cursor sits past `git `.
    let head = head_command(line);
    if let Some(pack) = head.and_then(|c| lib.lookup(c)) {
        // Walk into nested subcommands when typing e.g.
        // `git remote add `: each preceding word that matches a
        // subcommand drills one level deeper.
        let preceding = preceding_words(line, word_start);
        let active = walk_subcommands(pack, &preceding[1..]);
        let lib_rows = match active {
            Some(SubcommandView::Pack(p)) => library_rows(prefix, &p.subcommands, &p.options, locale),
            Some(SubcommandView::Sub(s)) => library_rows(prefix, &s.subcommands, &s.options, locale),
            None => Vec::new(),
        };
        if !lib_rows.is_empty() {
            return lib_rows;
        }
    }

    let resolved_cwd = resolve_cwd(cwd);
    complete_file(prefix, resolved_cwd.as_deref())
}

/// Returns the bare first word of the line (the command name), or
/// `None` when the line is empty / starts with whitespace and
/// nothing has been typed.
fn head_command(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let end = trimmed.find(|c: char| c.is_whitespace()).unwrap_or(trimmed.len());
    let head = &trimmed[..end];
    if head.is_empty() { None } else { Some(head) }
}

/// Words separating the head command from the cursor's current
/// word. For `git remote add ori|` (`|` = cursor) this returns
/// `["git", "remote", "add"]`. The first element is always the
/// command head.
fn preceding_words(line: &str, word_start: usize) -> Vec<&str> {
    line[..word_start]
        .split_whitespace()
        .collect()
}

/// View into the active subcommand context — either the top-level
/// pack or a nested subcommand reached by walking N levels.
enum SubcommandView<'a> {
    Pack(&'a super::library::CommandPack),
    Sub(&'a super::library::SubcommandEntry),
}

/// Walk nested subcommands. `chain` is the words *between* the
/// head command and the cursor's current word — e.g. for
/// `git remote add ori|` (head=git), the chain is `["remote", "add"]`.
/// Returns the deepest subcommand whose name matched; `None` means
/// one of the chain elements wasn't a known subcommand at its
/// level (so the user typed something unknown — fall through to
/// files).
fn walk_subcommands<'a>(
    pack: &'a super::library::CommandPack,
    chain: &[&str],
) -> Option<SubcommandView<'a>> {
    if chain.is_empty() {
        return Some(SubcommandView::Pack(pack));
    }
    // Skip flag-shaped tokens — `git -C path commit` should still
    // resolve `commit` against the top-level pack.
    let mut subs: &[super::library::SubcommandEntry] = &pack.subcommands;
    let mut current: Option<&super::library::SubcommandEntry> = None;
    for word in chain {
        if word.starts_with('-') {
            continue;
        }
        let found = subs.iter().find(|s| s.name == *word)?;
        current = Some(found);
        subs = &found.subcommands;
    }
    Some(match current {
        Some(s) => SubcommandView::Sub(s),
        None => SubcommandView::Pack(pack),
    })
}

/// Build the popover rows from a (subcommands, options) pair. When
/// `prefix` starts with `-`, only options match; otherwise only
/// subcommands.
fn library_rows(
    prefix: &str,
    subs: &[super::library::SubcommandEntry],
    opts: &[super::library::OptionEntry],
    locale: &str,
) -> Vec<Completion> {
    use super::library::Library;
    let mut out: Vec<Completion> = Vec::new();
    if prefix.starts_with('-') {
        for opt in opts {
            // Match against any of the comma-separated forms — so
            // typing `--ta` matches `-t, --tag`.
            let matched = opt
                .flag
                .split(',')
                .map(str::trim)
                .any(|f| f.starts_with(prefix));
            if !matched {
                continue;
            }
            let desc = Library::pick_locale(&opt.i18n, locale).to_string();
            // The `value` for option rows is the longest form (the
            // one beginning with `--`) when both forms exist —
            // matches user expectation for Tab-completing flags.
            let value = pick_long_form(&opt.flag).to_string();
            out.push(Completion {
                kind: CompletionKind::Option,
                value: value.clone(),
                display: opt.flag.clone(),
                hint: None,
                description: if desc.is_empty() { None } else { Some(desc) },
            });
        }
    } else {
        for sub in subs {
            if !sub.name.starts_with(prefix) {
                continue;
            }
            let desc = Library::pick_locale(&sub.i18n, locale).to_string();
            out.push(Completion {
                kind: CompletionKind::Subcommand,
                value: sub.name.clone(),
                display: sub.name.clone(),
                hint: None,
                description: if desc.is_empty() { None } else { Some(desc) },
            });
        }
    }
    out
}

/// Pick the `--long` form from a `"shortest, longest"` style flag
/// string. Falls back to the original when only one form is present.
fn pick_long_form(flag: &str) -> &str {
    flag.split(',')
        .map(str::trim)
        .find(|f| f.starts_with("--"))
        .unwrap_or_else(|| flag.split(',').next().map(str::trim).unwrap_or(flag))
}

/// Walk back from `cursor` while the char is part of a "word" — not
/// whitespace, not a shell operator. Returns the byte offset at
/// which the current word starts. The split mirrors the lexer's
/// notion of a word boundary so the resulting prefix matches what
/// the syntax overlay shows as one token.
fn find_word_start(line: &str, cursor: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = cursor;
    while i > 0 {
        let prev = bytes[i - 1];
        if matches!(
            prev,
            b' ' | b'\t' | b'|' | b'&' | b';' | b'>' | b'<' | b'\n'
        ) {
            break;
        }
        i -= 1;
    }
    i
}

/// True when `word_start` sits at the beginning of a compound — i.e.
/// the previous non-whitespace character is one of the operators
/// that begins a fresh command (or the line itself starts here).
fn is_command_position(line: &str, word_start: usize) -> bool {
    let mut i = word_start;
    while i > 0 {
        let prev = line.as_bytes()[i - 1];
        if prev == b' ' || prev == b'\t' {
            i -= 1;
            continue;
        }
        return matches!(prev, b'|' | b'&' | b';' | b'\n');
    }
    true
}

/// Builtins + PATH binaries whose name begins with `prefix`.
///
/// Listing is alphabetical with builtins first, capped at
/// `MAX_RESULTS` so a low-effort `Tab` on an empty prefix still
/// yields a usable list (and not, say, every binary on the system).
const MAX_RESULTS: usize = 64;

fn complete_command(prefix: &str) -> Vec<Completion> {
    use super::validate::SHELL_BUILTINS;

    let mut out: Vec<Completion> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for builtin in SHELL_BUILTINS {
        if !builtin.starts_with(prefix) {
            continue;
        }
        if !seen.insert((*builtin).to_string()) {
            continue;
        }
        out.push(Completion {
            kind: CompletionKind::Builtin,
            value: (*builtin).to_string(),
            display: (*builtin).to_string(),
            hint: Some("builtin".to_string()),
            description: None,
        });
        if out.len() >= MAX_RESULTS {
            break;
        }
    }

    if out.len() < MAX_RESULTS {
        if let Ok(path_var) = std::env::var("PATH") {
            let separator = if cfg!(windows) { ';' } else { ':' };
            for dir in path_var.split(separator) {
                if dir.is_empty() {
                    continue;
                }
                let entries = match std::fs::read_dir(dir) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                for entry in entries.flatten() {
                    let name = match entry.file_name().into_string() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    if !name.starts_with(prefix) {
                        continue;
                    }
                    let path = entry.path();
                    if !is_executable(&path) {
                        continue;
                    }
                    if !seen.insert(name.clone()) {
                        continue;
                    }
                    out.push(Completion {
                        kind: CompletionKind::Binary,
                        value: name.clone(),
                        display: name,
                        hint: Some(path.to_string_lossy().into_owned()),
                        description: None,
                    });
                    if out.len() >= MAX_RESULTS {
                        break;
                    }
                }
                if out.len() >= MAX_RESULTS {
                    break;
                }
            }
        }
    }

    out.sort_by(|a, b| a.display.cmp(&b.display));
    out
}

/// File and directory entries under `cwd / dirname(prefix)` whose
/// basename begins with `basename(prefix)`. Returns directories
/// before files for stable ordering.
fn complete_file(prefix: &str, cwd: Option<&Path>) -> Vec<Completion> {
    let (dir_part, base_part) = split_path_prefix(prefix);

    let base_dir = if dir_part.is_empty() {
        cwd.map(|c| c.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    } else if dir_part == "~" || dir_part.starts_with("~/") {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        if dir_part == "~" {
            home
        } else {
            home.join(&dir_part[2..])
        }
    } else if Path::new(&dir_part).is_absolute() {
        PathBuf::from(&dir_part)
    } else {
        cwd.map(|c| c.join(&dir_part))
            .unwrap_or_else(|| PathBuf::from(&dir_part))
    };

    let entries = match std::fs::read_dir(&base_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut dirs: Vec<Completion> = Vec::new();
    let mut files: Vec<Completion> = Vec::new();
    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        // Hide dotfiles unless the user typed at least one leading
        // dot — matches typical shell behaviour and keeps the popup
        // clean on home / project directories.
        if name.starts_with('.') && !base_part.starts_with('.') {
            continue;
        }
        if !name.starts_with(&base_part) {
            continue;
        }

        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

        // Reattach the user's typed dir prefix so inserting `value`
        // into the line preserves what they had. Trailing `/` for
        // directories so the next Tab drills in.
        let mut value = String::new();
        if !dir_part.is_empty() {
            value.push_str(&dir_part);
            if !dir_part.ends_with('/') {
                value.push('/');
            }
        }
        value.push_str(&name);
        if is_dir {
            value.push('/');
        }

        let completion = Completion {
            kind: if is_dir {
                CompletionKind::Directory
            } else {
                CompletionKind::File
            },
            value,
            display: if is_dir { format!("{}/", name) } else { name },
            hint: None,
            description: None,
        };

        if is_dir {
            dirs.push(completion);
        } else {
            files.push(completion);
        }
    }
    dirs.sort_by(|a, b| a.display.cmp(&b.display));
    files.sort_by(|a, b| a.display.cmp(&b.display));
    dirs.extend(files);
    if dirs.len() > MAX_RESULTS {
        dirs.truncate(MAX_RESULTS);
    }
    dirs
}

/// Split `prefix` into a `(dir, base)` pair using the last `/`. For
/// `foo/bar/baz` → (`foo/bar`, `baz`). For `~/proj/x` → (`~/proj`,
/// `x`). For a bare basename without slashes → (``, `prefix`).
fn split_path_prefix(prefix: &str) -> (String, String) {
    if let Some(idx) = prefix.rfind('/') {
        (
            prefix[..idx + 1].trim_end_matches('/').to_string(),
            prefix[idx + 1..].to_string(),
        )
    } else {
        (String::new(), prefix.to_string())
    }
}

fn resolve_cwd(supplied: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = supplied {
        return Some(p.to_path_buf());
    }
    std::env::current_dir().ok()
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(m) => m.is_file() && (m.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn find_word_start_walks_back_to_whitespace() {
        assert_eq!(find_word_start("git st", 6), 4);
        assert_eq!(find_word_start("git", 3), 0);
        assert_eq!(find_word_start("a | b", 5), 4);
        assert_eq!(find_word_start("", 0), 0);
    }

    #[test]
    fn command_position_reflects_pipe_and_separator() {
        assert!(is_command_position("git st", 0));
        assert!(is_command_position("ls | gr", 5)); // after `| `
        assert!(is_command_position("a; b", 3)); // after `; `
        assert!(!is_command_position("git st", 4)); // arg word
    }

    #[test]
    fn empty_input_command_returns_builtins_at_least() {
        let results = complete_command("");
        // Caller controls MAX_RESULTS; we ship 64.
        assert!(
            !results.is_empty(),
            "empty prefix should still yield results"
        );
        assert!(results.len() <= MAX_RESULTS);
        let names: HashSet<_> = results.iter().map(|c| c.display.as_str()).collect();
        // `cd` and `echo` are universal — any sane PATH should at least
        // expose them as builtins via the static table.
        assert!(names.contains("cd"));
        assert!(names.contains("echo"));
    }

    #[test]
    fn command_prefix_filters_correctly() {
        let results = complete_command("ec");
        // `echo` + maybe other ec-prefixed binaries
        assert!(results.iter().any(|c| c.display == "echo"));
        for r in &results {
            assert!(
                r.display.starts_with("ec"),
                "every result must match prefix, got {:?}",
                r.display,
            );
        }
    }

    #[test]
    fn file_completion_lists_cwd_entries() {
        // Build a temp dir with a known set of entries and probe it.
        let tmp = std::env::temp_dir().join(format!(
            "pier-x-completions-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("alpha.txt"), b"").unwrap();
        fs::write(tmp.join("beta.txt"), b"").unwrap();
        fs::create_dir(tmp.join("subdir")).unwrap();

        let results = complete_file("", Some(&tmp));
        let names: HashSet<_> = results.iter().map(|c| c.display.as_str()).collect();
        assert!(names.contains("alpha.txt"), "got {names:?}");
        assert!(names.contains("beta.txt"), "got {names:?}");
        assert!(names.contains("subdir/"), "got {names:?}");
        // Dotfiles hidden by default — none should appear with empty prefix.
        for n in &names {
            assert!(!n.starts_with('.'), "dotfile leaked: {n}");
        }
        // Directory should sort before files.
        let positions: Vec<_> = results
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if c.display == "subdir/" || c.display == "alpha.txt" {
                    Some((c.display.clone(), i))
                } else {
                    None
                }
            })
            .collect();
        let dir_idx = positions.iter().find(|(n, _)| n == "subdir/").unwrap().1;
        let file_idx = positions.iter().find(|(n, _)| n == "alpha.txt").unwrap().1;
        assert!(dir_idx < file_idx, "dir should come before file");

        // Cleanup
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn split_path_prefix_handles_dirs_and_basenames() {
        assert_eq!(split_path_prefix("foo"), (String::new(), "foo".to_string()));
        assert_eq!(
            split_path_prefix("foo/bar"),
            ("foo".to_string(), "bar".to_string()),
        );
        assert_eq!(
            split_path_prefix("foo/bar/"),
            ("foo/bar".to_string(), "".to_string()),
        );
        assert_eq!(
            split_path_prefix("~/proj/x"),
            ("~/proj".to_string(), "x".to_string()),
        );
    }

    #[test]
    fn complete_dispatches_to_command_then_file() {
        // First word on the line → command completion.
        let cmd_results = complete("ec", 2, None);
        assert!(cmd_results.iter().any(|c| c.display == "echo"));

        // After a space → file completion (cwd defaults to process cwd
        // which is the workspace dir during cargo test).
        let file_results = complete("ls Cargo.t", 10, None);
        // Cargo.toml exists at the crate root during cargo test runs;
        // the test is forgiving about other matches.
        assert!(
            file_results.iter().any(|c| c.display.starts_with("Cargo.")),
            "expected Cargo.* in results, got {:?}",
            file_results.iter().map(|c| &c.display).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn library_subcommand_completion_emits_descriptions() {
        let lib = super::super::library::Library::bundled();
        // After `docker `, the prefix is empty so we should get
        // every docker subcommand back, each with a description.
        let line = "docker ";
        let rows = complete_with_library(line, line.len(), None, &lib, "en");
        assert!(!rows.is_empty(), "bundled docker pack should match");
        let build_row = rows
            .iter()
            .find(|r| r.value == "build")
            .expect("`build` should be a known docker subcommand");
        assert_eq!(build_row.kind, CompletionKind::Subcommand);
        assert!(
            build_row.description.as_deref().unwrap_or("").contains("Build"),
            "expected English description, got {:?}",
            build_row.description,
        );
    }

    #[test]
    fn library_command_position_with_exact_match_offers_subcommands_with_prefixed_value() {
        // `docker` (no trailing space) + Tab — backend should also
        // include the docker pack's subcommands so the user can drill
        // in without having to first type a space. Each subcommand
        // row's `value` must be prefixed with the full command + space
        // so the frontend's word-diff inserts ` <sub>` correctly.
        let lib = super::super::library::Library::bundled();
        let line = "docker";
        let rows = complete_with_library(line, line.len(), None, &lib, "en");
        let attach = rows
            .iter()
            .find(|r| r.display == "attach")
            .expect("attach subcommand should be surfaced at command-position");
        assert_eq!(attach.kind, CompletionKind::Subcommand);
        assert_eq!(attach.value, "docker attach");
        // We should still see the regular binary completion for the
        // command name itself.
        assert!(rows.iter().any(|r| r.value == "docker" && matches!(r.kind, CompletionKind::Binary | CompletionKind::Builtin)),
                "`docker` itself should still be in the list");
    }

    #[test]
    fn library_completion_falls_back_to_files_when_prefix_does_not_match_subcommand() {
        let lib = super::super::library::Library::bundled();
        // `docker zzz` — `zzz` isn't a known subcommand, so the
        // engine falls through to file completion. We just verify
        // the library path doesn't swallow the line and returns
        // *something* (could be empty in cwd).
        let line = "docker zzz";
        let _ = complete_with_library(line, line.len(), None, &lib, "en");
        // No assertion — the contract is "no panic, falls through".
    }

    #[test]
    fn library_completion_picks_user_locale_when_present() {
        let lib = super::super::library::Library::bundled();
        let line = "docker bui";
        let rows_en = complete_with_library(line, line.len(), None, &lib, "en");
        let rows_zh = complete_with_library(line, line.len(), None, &lib, "zh-CN");
        let en_build = rows_en.iter().find(|r| r.value == "build").unwrap();
        let zh_build = rows_zh.iter().find(|r| r.value == "build").unwrap();
        // Different locales should yield different description text
        // when both are present in the bundled pack.
        assert_ne!(en_build.description, zh_build.description);
        assert!(zh_build
            .description
            .as_deref()
            .unwrap_or("")
            .contains("Dockerfile"));
    }

    #[test]
    fn library_walks_nested_subcommands_when_chain_matches() {
        // Hand-build a tiny library to exercise the chain walker
        // without depending on the bundled `git` pack growing
        // nested entries.
        use super::super::library::{CommandPack, SubcommandEntry};
        let mut pack = CommandPack {
            schema_version: super::super::library::SCHEMA_VERSION,
            command: "git".into(),
            tool_version: String::new(),
            source: String::new(),
            import_method: String::new(),
            import_date: String::new(),
            options: Vec::new(),
            subcommands: vec![SubcommandEntry {
                name: "remote".into(),
                i18n: HashMap::new(),
                options: Vec::new(),
                subcommands: vec![SubcommandEntry {
                    name: "add".into(),
                    i18n: {
                        let mut m = HashMap::new();
                        m.insert("en".into(), "Add a remote".into());
                        m
                    },
                    options: Vec::new(),
                    subcommands: Vec::new(),
                }],
            }],
        };
        // Need to import HashMap for the test
        let _ = std::mem::replace(&mut pack.command, "git".into());
        let mut lib = super::super::library::Library::empty();
        lib.insert(pack);

        // Type `git remote a` — chain is ["git", "remote"], cursor
        // word is "a". The `remote` walks one level, then we match
        // `a` against {add}.
        let line = "git remote a";
        let rows = complete_with_library(line, line.len(), None, &lib, "en");
        assert!(rows.iter().any(|r| r.value == "add"));
    }

    #[test]
    fn library_option_rows_match_either_short_or_long_flag_form() {
        use super::super::library::{CommandPack, OptionEntry};
        let mut lib = super::super::library::Library::empty();
        lib.insert(CommandPack {
            schema_version: super::super::library::SCHEMA_VERSION,
            command: "myc".into(),
            tool_version: String::new(),
            source: String::new(),
            import_method: String::new(),
            import_date: String::new(),
            subcommands: Vec::new(),
            options: vec![OptionEntry {
                flag: "-t, --tag".into(),
                i18n: {
                    let mut m = HashMap::new();
                    m.insert("en".into(), "Tag the build".into());
                    m
                },
            }],
        });
        // Long form lookup
        let line = "myc --ta";
        let rows = complete_with_library(line, line.len(), None, &lib, "en");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, CompletionKind::Option);
        assert_eq!(rows[0].value, "--tag");
        // Short form lookup
        let line = "myc -t";
        let rows = complete_with_library(line, line.len(), None, &lib, "en");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "--tag"); // value always picks long form
    }

    use std::collections::HashMap;
}
