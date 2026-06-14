//! Code search over an SSH session (M8).
//!
//! Runs the user's preferred grep tool — `rg` (ripgrep) when
//! installed, falling back to `git grep` when the cwd is inside a
//! git repo, and erroring out otherwise — against the active
//! terminal cwd. The shell pipeline is built once and shipped
//! through `exec_command`; output is parsed line-by-line into
//! structured [`SearchHit`]s the frontend can list and click.
//!
//! Why this lives in pier-core (and not as raw Tauri commands):
//! the shell-quoting + parsing is dialect-sensitive enough that
//! we want unit tests around it, and the module is UI-agnostic.

use serde::Serialize;

use super::docker::shell_quote;
use crate::ssh::{SshError, SshSession};

/// One match from a code-search run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// Path relative to `cwd` (paths from `rg` start with `./`,
    /// stripped before reaching this struct).
    pub file: String,
    /// 1-based line number.
    pub line: u32,
    /// 1-based column. `0` when the engine did not provide one.
    pub column: u32,
    /// The matching line, with terminal escapes already stripped.
    /// Truncated to a soft limit (rg's `--max-columns=400`); git
    /// grep does its own truncation.
    pub text: String,
}

/// What to search for. Selected by the panel's mode switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// Grep file *contents* (rg → git grep → grep).
    Content,
    /// Find files by *name* (fd / fdfind → rg --files → find).
    FileName,
    /// Locate an executable on `$PATH` (`command -v` + a PATH scan).
    /// cwd-independent, so it works even on a host with no grep tools.
    Command,
}

impl SearchMode {
    /// Parse the wire string the frontend sends. Unknown / empty
    /// falls back to `Content` (the historical default).
    pub fn from_wire(s: &str) -> Self {
        match s {
            "filename" | "file-name" | "file" => SearchMode::FileName,
            "command" | "cmd" | "which" => SearchMode::Command,
            _ => SearchMode::Content,
        }
    }
}

/// Which tool actually ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchEngine {
    /// `rg` — preferred content engine when present on PATH.
    Rg,
    /// `git grep` — content fallback when the cwd is inside a git
    /// working tree and `rg` was not on PATH.
    GitGrep,
    /// Plain `grep -rIn` — last-resort content engine. Lets content
    /// search work on a host without rg that isn't a git repo
    /// (slower; the frontend nudges toward installing ripgrep).
    Grep,
    /// `fd` / `fdfind` — preferred filename engine (respects
    /// `.gitignore`, fast). `fdfind` is the Debian/Ubuntu binary name.
    Fd,
    /// `rg --files | grep` — filename fallback when fd is absent but
    /// rg is present.
    RgFiles,
    /// `find … | grep` — last-resort filename engine.
    Find,
    /// `command -v` + `$PATH` scan — locates an executable. Only runs
    /// in `Command` mode.
    Command,
    /// No usable engine for the requested mode and the cwd is not a
    /// git repo. Frontend renders an "install ripgrep / fd" CTA.
    None,
    /// `cd` into the requested directory failed (path missing,
    /// not readable, etc.). No engine ran.
    CwdMissing,
}

/// Wire-friendly result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOutput {
    /// Resolved working directory after `cd`. This is especially
    /// important when the input cwd was empty and the backend fell
    /// back to `$HOME`; relative hits can still be opened through SFTP.
    pub cwd: String,
    /// Tool that produced `hits` (or the reason there are no hits).
    pub engine: SearchEngine,
    /// Up to `max_hits` matches.
    pub hits: Vec<SearchHit>,
    /// `true` when the engine produced more rows than `max_hits` —
    /// the UI surfaces a "refine your query" hint.
    pub truncated: bool,
    /// Exit code of the engine. `0` = matches, `1` = no matches
    /// (rg / git-grep convention). Anything else surfaces to the
    /// UI as an error banner.
    pub exit_code: i32,
}

/// Inputs to a single search. Construct in the Tauri layer from
/// the panel's form state.
#[derive(Debug, Clone)]
pub struct SearchOpts {
    /// What kind of search to run (content / filename / command).
    pub mode: SearchMode,
    /// Working directory to start the search in. Empty falls back
    /// to `$HOME` server-side. Ignored in `Command` mode.
    pub cwd: String,
    /// Pattern to search for. Treated as literal text by default;
    /// set `regex` to interpret as a regex.
    pub query: String,
    /// `-i` on both engines.
    pub case_insensitive: bool,
    /// When `false`, both engines run with `-F` (fixed strings).
    pub regex: bool,
    /// `-w` on both engines.
    pub whole_word: bool,
    /// Optional file glob. Mapped to ripgrep's `-g` and, for
    /// `git grep`, to a trailing pathspec.
    pub glob: String,
    /// Hard cap on hits returned to the UI. Soft floor of 1 (we
    /// always send at least one row when there's a match);
    /// soft ceiling of 5000 to keep the response budget tight.
    pub max_hits: usize,
}

/// Async sibling of [`search_blocking`].
pub async fn search(session: &SshSession, opts: SearchOpts) -> Result<SearchOutput, SshError> {
    let cmd = build_command(&opts);
    let (exit_code, stdout) = session.exec_command(&cmd).await?;
    Ok(parse_output(&stdout, opts.max_hits, exit_code))
}

/// Run a code search and return the parsed result. Use this from
/// sync Tauri command bodies — it spins the shared runtime
/// internally.
pub fn search_blocking(session: &SshSession, opts: SearchOpts) -> Result<SearchOutput, SshError> {
    crate::ssh::runtime::shared().block_on(search(session, opts))
}

fn build_command(opts: &SearchOpts) -> String {
    match opts.mode {
        SearchMode::Content => build_content_command(opts),
        SearchMode::FileName => build_filename_command(opts),
        SearchMode::Command => build_command_lookup(opts),
    }
}

/// `cd`-target expression. Default to `$HOME` server-side when no
/// cwd was probed yet — `cd ""` is a no-op in most shells, but the
/// explicit `$HOME` makes the fallback obvious.
fn cwd_expr(opts: &SearchOpts) -> String {
    if opts.cwd.trim().is_empty() {
        "\"$HOME\"".to_string()
    } else {
        shell_quote(opts.cwd.trim())
    }
}

/// `head -n` cap, +1 so the parser can detect truncation
/// unambiguously (cap rows = exact, +1 row = truncated).
fn head_cap(opts: &SearchOpts) -> usize {
    opts.max_hits.max(1).min(5000) + 1
}

/// Content search: grep file contents. rg → git grep → plain grep.
fn build_content_command(opts: &SearchOpts) -> String {
    // Common flags. `-F` (literal) is the safe default; users opt
    // into regex via the panel's Regex toggle. All three engines
    // accept `-i` / `-w` / `-F`.
    let mut common = String::new();
    if opts.case_insensitive {
        common.push_str(" -i");
    }
    if opts.whole_word {
        common.push_str(" -w");
    }
    if !opts.regex {
        common.push_str(" -F");
    }

    let pattern = shell_quote(&opts.query);
    let glob = opts.glob.trim();
    let rg_glob = if glob.is_empty() {
        String::new()
    } else {
        format!(" -g {}", shell_quote(glob))
    };
    let git_pathspec = if glob.is_empty() {
        String::new()
    } else {
        format!(" -- {}", shell_quote(glob))
    };
    // `--include` is honored by both GNU and BSD grep.
    let grep_include = if glob.is_empty() {
        String::new()
    } else {
        format!(" --include={}", shell_quote(glob))
    };

    format!(
        "cd {cwd} 2>/dev/null || {{ echo 'ENGINE:CWD_MISSING'; exit 3; }}\n\
         printf 'CWD:%s\\n' \"$PWD\"\n\
         if command -v rg >/dev/null 2>&1; then\n\
         \x20\x20echo 'ENGINE:rg'\n\
         \x20\x20rg --no-heading --color=never -n --column --max-columns=400{common}{rg_glob} -e {pat} . 2>/dev/null | head -n {head_cap}\n\
         elif git rev-parse --git-dir >/dev/null 2>&1; then\n\
         \x20\x20echo 'ENGINE:git-grep'\n\
         \x20\x20git grep -n --column -I{common} -e {pat}{git_pathspec} 2>/dev/null | head -n {head_cap}\n\
         else\n\
         \x20\x20echo 'ENGINE:grep'\n\
         \x20\x20grep -rIn --color=never{common}{grep_include} -e {pat} . 2>/dev/null | head -n {head_cap}\n\
         fi\n",
        cwd = cwd_expr(opts),
        common = common,
        rg_glob = rg_glob,
        pat = pattern,
        git_pathspec = git_pathspec,
        grep_include = grep_include,
        head_cap = head_cap(opts),
    )
}

/// Filename search: match the query against file names.
/// fd → fdfind (Debian/Ubuntu binary) → `rg --files | grep` → `find | grep`.
/// `--type f` keeps results openable in the SFTP editor.
fn build_filename_command(opts: &SearchOpts) -> String {
    let pattern = shell_quote(&opts.query);

    // fd: regex by default; `--fixed-strings` when the Regex toggle
    // is off. fd has no whole-word notion, so it's ignored here.
    let mut fd_flags = String::from("--color=never --type f");
    if opts.case_insensitive {
        fd_flags.push_str(" --ignore-case");
    } else {
        fd_flags.push_str(" --case-sensitive");
    }
    if !opts.regex {
        fd_flags.push_str(" --fixed-strings");
    }

    // grep flags for the rg-files / find fallbacks (matching against
    // the whole path). `-E` for regex, `-F` for literal.
    let mut gflags = String::new();
    if opts.case_insensitive {
        gflags.push_str(" -i");
    }
    if opts.regex {
        gflags.push_str(" -E");
    } else {
        gflags.push_str(" -F");
    }

    format!(
        "cd {cwd} 2>/dev/null || {{ echo 'ENGINE:CWD_MISSING'; exit 3; }}\n\
         printf 'CWD:%s\\n' \"$PWD\"\n\
         if command -v fd >/dev/null 2>&1; then\n\
         \x20\x20echo 'ENGINE:fd'\n\
         \x20\x20fd {fd_flags} -- {pat} . 2>/dev/null | head -n {head_cap}\n\
         elif command -v fdfind >/dev/null 2>&1; then\n\
         \x20\x20echo 'ENGINE:fd'\n\
         \x20\x20fdfind {fd_flags} -- {pat} . 2>/dev/null | head -n {head_cap}\n\
         elif command -v rg >/dev/null 2>&1; then\n\
         \x20\x20echo 'ENGINE:rg-files'\n\
         \x20\x20rg --files 2>/dev/null | grep{gflags} -e {pat} 2>/dev/null | head -n {head_cap}\n\
         else\n\
         \x20\x20echo 'ENGINE:find'\n\
         \x20\x20find . -type f 2>/dev/null | grep{gflags} -e {pat} 2>/dev/null | head -n {head_cap}\n\
         fi\n",
        cwd = cwd_expr(opts),
        fd_flags = fd_flags,
        gflags = gflags,
        pat = pattern,
        head_cap = head_cap(opts),
    )
}

/// Command lookup: locate an executable on `$PATH`. cwd-independent
/// and dependency-free — `command -v` is a POSIX shell builtin, and
/// the PATH scan catches shadowed copies (e.g. python3, python3.11)
/// without relying on `which` (dropped by recent Debian) or `whereis`
/// (absent / differently-shaped on macOS).
fn build_command_lookup(opts: &SearchOpts) -> String {
    let pattern = shell_quote(&opts.query);
    // `pat` assignment keeps the substring literal while the
    // surrounding `*` still glob-expand in the PATH scan.
    format!(
        "printf 'CWD:\\n'\n\
         echo 'ENGINE:command'\n\
         pat={pat}\n\
         {{ command -v -- \"$pat\" 2>/dev/null\n\
         \x20\x20IFS=:\n\
         \x20\x20for d in $PATH; do\n\
         \x20\x20\x20\x20[ -d \"$d\" ] || continue\n\
         \x20\x20\x20\x20for f in \"$d/\"*\"$pat\"*; do\n\
         \x20\x20\x20\x20\x20\x20[ -f \"$f\" ] && [ -x \"$f\" ] && printf '%s\\n' \"$f\"\n\
         \x20\x20\x20\x20done\n\
         \x20\x20done\n\
         }} 2>/dev/null | head -n {head_cap}\n",
        pat = pattern,
        head_cap = head_cap(opts),
    )
}

/// How an engine's result rows are shaped.
enum RowKind {
    /// `file:line:col:text` — rg, git grep.
    FourField,
    /// `file:line:text` — plain grep (no column).
    ThreeField,
    /// A bare (relative) path — fd, rg --files, find.
    Path,
    /// A bare absolute path, de-duplicated — command lookup.
    CommandPath,
    /// No rows expected (none / cwd-missing).
    None,
}

fn parse_output(stdout: &str, max_hits: usize, exit_code: i32) -> SearchOutput {
    let mut lines = stdout.lines();
    let first = lines.next().unwrap_or("").trim_end_matches('\r').trim();
    let (cwd, header) = if let Some(cwd) = first.strip_prefix("CWD:") {
        (
            cwd.to_string(),
            lines.next().unwrap_or("").trim_end_matches('\r').trim(),
        )
    } else {
        (String::new(), first)
    };
    let engine = match header {
        "ENGINE:rg" => SearchEngine::Rg,
        "ENGINE:git-grep" => SearchEngine::GitGrep,
        "ENGINE:grep" => SearchEngine::Grep,
        "ENGINE:fd" => SearchEngine::Fd,
        "ENGINE:rg-files" => SearchEngine::RgFiles,
        "ENGINE:find" => SearchEngine::Find,
        "ENGINE:command" => SearchEngine::Command,
        "ENGINE:none" => SearchEngine::None,
        "ENGINE:CWD_MISSING" => SearchEngine::CwdMissing,
        _ => SearchEngine::None,
    };

    let row_kind = match engine {
        SearchEngine::Rg | SearchEngine::GitGrep => RowKind::FourField,
        SearchEngine::Grep => RowKind::ThreeField,
        SearchEngine::Fd | SearchEngine::RgFiles | SearchEngine::Find => RowKind::Path,
        SearchEngine::Command => RowKind::CommandPath,
        SearchEngine::None | SearchEngine::CwdMissing => RowKind::None,
    };

    let mut hits: Vec<SearchHit> = Vec::new();
    let mut truncated = false;
    let cap = max_hits.max(1).min(5000);
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    if !matches!(row_kind, RowKind::None) {
        for raw in lines {
            let trimmed = raw.trim_end_matches('\r');
            if trimmed.is_empty() {
                continue;
            }
            if hits.len() >= cap {
                truncated = true;
                break;
            }
            let hit = match row_kind {
                RowKind::FourField => parse_hit(trimmed),
                RowKind::ThreeField => parse_hit_3(trimmed),
                RowKind::Path => parse_path_row(trimmed),
                RowKind::CommandPath => parse_command_row(trimmed, &mut seen),
                RowKind::None => None,
            };
            if let Some(hit) = hit {
                hits.push(hit);
            }
        }
    }

    SearchOutput {
        cwd,
        engine,
        hits,
        truncated,
        exit_code,
    }
}

/// Parse one `path:line:col:text` row. Paths with embedded `:`
/// would be misparsed; keep that on the radar but don't pre-build
/// for it — source code paths in practice don't carry `:`.
fn parse_hit(row: &str) -> Option<SearchHit> {
    // Skip past a leading `./` that rg adds for cwd-rooted matches.
    let row = row.strip_prefix("./").unwrap_or(row);

    let (file, rest) = row.split_once(':')?;
    let (line_s, rest) = rest.split_once(':')?;
    let (col_s, text) = rest.split_once(':')?;
    let line: u32 = line_s.parse().ok()?;
    let column: u32 = col_s.parse().unwrap_or(0);
    if line == 0 || file.is_empty() {
        return None;
    }
    Some(SearchHit {
        file: file.to_string(),
        line,
        column,
        text: text.to_string(),
    })
}

/// Parse one `path:line:text` row (plain grep — no column).
fn parse_hit_3(row: &str) -> Option<SearchHit> {
    let row = row.strip_prefix("./").unwrap_or(row);
    let (file, rest) = row.split_once(':')?;
    let (line_s, text) = rest.split_once(':')?;
    let line: u32 = line_s.parse().ok()?;
    if line == 0 || file.is_empty() {
        return None;
    }
    Some(SearchHit {
        file: file.to_string(),
        line,
        column: 0,
        text: text.to_string(),
    })
}

/// Parse a bare-path row (filename engines). The whole line is the
/// path; there is no line / column / matched text.
fn parse_path_row(row: &str) -> Option<SearchHit> {
    let path = row.strip_prefix("./").unwrap_or(row).trim();
    if path.is_empty() {
        return None;
    }
    Some(SearchHit {
        file: path.to_string(),
        line: 0,
        column: 0,
        text: String::new(),
    })
}

/// Parse a command-lookup row. Keep only absolute paths (drops
/// `command -v` output for shell builtins / keywords) and skip
/// duplicates surfaced by both `command -v` and the PATH scan.
fn parse_command_row(row: &str, seen: &mut std::collections::HashSet<String>) -> Option<SearchHit> {
    let path = row.trim();
    if !path.starts_with('/') || !seen.insert(path.to_string()) {
        return None;
    }
    Some(SearchHit {
        file: path.to_string(),
        line: 0,
        column: 0,
        text: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_handles_rg_rows_strips_dot_slash() {
        let stdout = "ENGINE:rg\n\
                      ./src/lib.rs:12:7:fn main() {\n\
                      ./README.md:3:1:# Title\n";
        let out = parse_output(stdout, 100, 0);
        assert_eq!(out.engine, SearchEngine::Rg);
        assert!(!out.truncated);
        assert_eq!(out.hits.len(), 2);
        assert_eq!(out.hits[0].file, "src/lib.rs");
        assert_eq!(out.hits[0].line, 12);
        assert_eq!(out.hits[0].column, 7);
        assert_eq!(out.hits[0].text, "fn main() {");
    }

    #[test]
    fn parse_captures_resolved_cwd() {
        let stdout = "CWD:/home/alice/project\n\
                      ENGINE:rg\n\
                      ./src/lib.rs:2:1:needle\n";
        let out = parse_output(stdout, 100, 0);
        assert_eq!(out.cwd, "/home/alice/project");
        assert_eq!(out.engine, SearchEngine::Rg);
        assert_eq!(out.hits.len(), 1);
    }

    #[test]
    fn parse_handles_git_grep_rows() {
        let stdout = "ENGINE:git-grep\n\
                      pier-core/src/main.rs:42:5:    println!(\"hi\");\n";
        let out = parse_output(stdout, 100, 0);
        assert_eq!(out.engine, SearchEngine::GitGrep);
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].file, "pier-core/src/main.rs");
        assert_eq!(out.hits[0].text, "    println!(\"hi\");");
    }

    #[test]
    fn parse_marks_truncation_when_overflow() {
        let mut s = String::from("ENGINE:rg\n");
        for i in 0..6 {
            s.push_str(&format!("a/b.rs:{}:1:line {}\n", i + 1, i));
        }
        let out = parse_output(&s, 5, 0);
        assert!(out.truncated);
        assert_eq!(out.hits.len(), 5);
    }

    #[test]
    fn parse_engine_none_passes_through() {
        let out = parse_output("ENGINE:none\n", 100, 0);
        assert_eq!(out.engine, SearchEngine::None);
        assert!(out.hits.is_empty());
    }

    #[test]
    fn parse_cwd_missing_reported() {
        let out = parse_output("ENGINE:CWD_MISSING\n", 100, 3);
        assert_eq!(out.engine, SearchEngine::CwdMissing);
    }

    #[test]
    fn parse_skips_malformed_rows() {
        let stdout = "ENGINE:rg\n\
                      not-a-hit-line\n\
                      a:notanumber:0:text\n\
                      a/b.rs:5:1:ok\n";
        let out = parse_output(stdout, 100, 0);
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].file, "a/b.rs");
    }

    #[test]
    fn build_command_sets_literal_by_default() {
        let cmd = build_command(&SearchOpts {
            mode: SearchMode::Content,
            cwd: "/var/www".into(),
            query: "TODO".into(),
            case_insensitive: false,
            regex: false,
            whole_word: false,
            glob: String::new(),
            max_hits: 200,
        });
        // shell_quote leaves shell-safe tokens unquoted.
        assert!(cmd.contains("cd /var/www"), "{cmd}");
        assert!(cmd.contains(" -F -e TODO "), "{cmd}");
        assert!(cmd.contains("head -n 201"), "{cmd}");
    }

    #[test]
    fn build_command_threads_flags() {
        let cmd = build_command(&SearchOpts {
            mode: SearchMode::Content,
            cwd: "".into(),
            query: "needle".into(),
            case_insensitive: true,
            regex: true,
            whole_word: true,
            glob: String::new(),
            max_hits: 0, // floor → 1
        });
        assert!(cmd.contains("cd \"$HOME\""), "{cmd}");
        // -i -w but no -F since regex=true.
        assert!(cmd.contains(" -i -w -e needle "), "{cmd}");
        assert!(!cmd.contains(" -F "));
        assert!(cmd.contains("head -n 2"), "{cmd}");
    }

    #[test]
    fn build_command_quotes_pattern_with_special_chars() {
        let cmd = build_command(&SearchOpts {
            mode: SearchMode::Content,
            cwd: "/tmp".into(),
            query: "it's a $needle".into(),
            case_insensitive: false,
            regex: false,
            whole_word: false,
            glob: String::new(),
            max_hits: 100,
        });
        // shell_quote wraps in '...' and escapes embedded '.
        assert!(cmd.contains("'it'\\''s a $needle'"), "{cmd}");
    }

    #[test]
    fn build_command_threads_glob_to_engines() {
        let cmd = build_command(&SearchOpts {
            mode: SearchMode::Content,
            cwd: "/srv/app".into(),
            query: "TODO".into(),
            case_insensitive: false,
            regex: false,
            whole_word: false,
            glob: "src/**/*.ts".into(),
            max_hits: 100,
        });
        assert!(cmd.contains(" -g 'src/**/*.ts' -e TODO "), "{cmd}");
        assert!(
            cmd.contains("git grep -n --column -I -F -e TODO -- 'src/**/*.ts'"),
            "{cmd}"
        );
    }

    #[test]
    fn content_command_has_grep_fallback() {
        let cmd = build_content_command(&SearchOpts {
            mode: SearchMode::Content,
            cwd: "/srv".into(),
            query: "needle".into(),
            case_insensitive: true,
            regex: false,
            whole_word: false,
            glob: String::new(),
            max_hits: 100,
        });
        assert!(cmd.contains("ENGINE:grep"), "{cmd}");
        assert!(
            cmd.contains("grep -rIn --color=never -i -F -e needle ."),
            "{cmd}"
        );
    }

    #[test]
    fn parse_grep_three_field_rows() {
        let stdout = "CWD:/srv\nENGINE:grep\n./a/b.rs:7:let x = 1;\n";
        let out = parse_output(stdout, 100, 0);
        assert_eq!(out.engine, SearchEngine::Grep);
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].file, "a/b.rs");
        assert_eq!(out.hits[0].line, 7);
        assert_eq!(out.hits[0].column, 0);
        assert_eq!(out.hits[0].text, "let x = 1;");
    }

    #[test]
    fn filename_command_probes_fd_then_fdfind() {
        let cmd = build_filename_command(&SearchOpts {
            mode: SearchMode::FileName,
            cwd: "/srv/app".into(),
            query: "nginx".into(),
            case_insensitive: true,
            regex: false,
            whole_word: false,
            glob: String::new(),
            max_hits: 100,
        });
        assert!(cmd.contains("command -v fd "), "{cmd}");
        assert!(cmd.contains("command -v fdfind "), "{cmd}");
        assert!(
            cmd.contains("--type f --ignore-case --fixed-strings -- nginx ."),
            "{cmd}"
        );
        assert!(cmd.contains("rg --files"), "{cmd}");
        assert!(cmd.contains("find . -type f"), "{cmd}");
    }

    #[test]
    fn filename_rows_parse_as_paths() {
        let stdout = "CWD:/srv\nENGINE:fd\n./conf/nginx.conf\nbin/run.sh\n";
        let out = parse_output(stdout, 100, 0);
        assert_eq!(out.engine, SearchEngine::Fd);
        assert_eq!(out.hits.len(), 2);
        assert_eq!(out.hits[0].file, "conf/nginx.conf");
        assert_eq!(out.hits[0].line, 0);
        assert_eq!(out.hits[0].text, "");
        assert_eq!(out.hits[1].file, "bin/run.sh");
    }

    #[test]
    fn command_lookup_is_cwd_independent() {
        let cmd = build_command_lookup(&SearchOpts {
            mode: SearchMode::Command,
            cwd: "/ignored".into(),
            query: "python3".into(),
            case_insensitive: false,
            regex: false,
            whole_word: false,
            glob: String::new(),
            max_hits: 100,
        });
        assert!(!cmd.contains("cd "), "command mode must not cd: {cmd}");
        assert!(cmd.contains("command -v -- \"$pat\""), "{cmd}");
        assert!(cmd.contains("pat=python3"), "{cmd}");
    }

    #[test]
    fn command_rows_keep_absolute_paths_and_dedupe() {
        let stdout = "CWD:\nENGINE:command\n\
                      /usr/bin/python3\n\
                      python3: shell builtin\n\
                      /usr/bin/python3\n\
                      /usr/local/bin/python3.11\n";
        let out = parse_output(stdout, 100, 0);
        assert_eq!(out.engine, SearchEngine::Command);
        assert_eq!(out.hits.len(), 2);
        assert_eq!(out.hits[0].file, "/usr/bin/python3");
        assert_eq!(out.hits[1].file, "/usr/local/bin/python3.11");
    }
}
