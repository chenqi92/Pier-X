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

/// Which tool actually ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchEngine {
    /// `rg` — preferred when present on PATH.
    Rg,
    /// `git grep` — only when the cwd is inside a git working tree
    /// and `rg` was not on PATH.
    GitGrep,
    /// Neither tool is available on the remote and the cwd is not
    /// a git repo. Frontend renders an "install ripgrep" CTA.
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
    /// Working directory to start the search in. Empty falls back
    /// to `$HOME` server-side.
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
    // Default to $HOME server-side when no cwd was probed yet.
    // Empty-string would `cd ""` which most shells treat as "no
    // change", but it's an explicit reminder that the panel
    // hasn't latched a real cwd onto the tab.
    let cwd_expr = if opts.cwd.trim().is_empty() {
        "\"$HOME\"".to_string()
    } else {
        shell_quote(opts.cwd.trim())
    };

    // Common flags. `-F` (literal) is the safe default; users opt
    // into regex via the panel's Regex toggle.
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
    let cap = opts.max_hits.max(1).min(5000);
    // Request +1 row so the parser can detect truncation without
    // ambiguity (cap rows = exactly cap, +1 row = truncated).
    let head_cap = cap + 1;

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
         \x20\x20echo 'ENGINE:none'\n\
         fi\n",
        cwd = cwd_expr,
        common = common,
        rg_glob = rg_glob,
        pat = pattern,
        git_pathspec = git_pathspec,
        head_cap = head_cap,
    )
}

fn parse_output(stdout: &str, max_hits: usize, exit_code: i32) -> SearchOutput {
    let mut lines = stdout.lines();
    let first = lines.next().unwrap_or("").trim();
    let (cwd, header) = if let Some(cwd) = first.strip_prefix("CWD:") {
        (cwd.to_string(), lines.next().unwrap_or("").trim())
    } else {
        (String::new(), first)
    };
    let engine = match header {
        "ENGINE:rg" => SearchEngine::Rg,
        "ENGINE:git-grep" => SearchEngine::GitGrep,
        "ENGINE:none" => SearchEngine::None,
        "ENGINE:CWD_MISSING" => SearchEngine::CwdMissing,
        _ => SearchEngine::None,
    };

    let mut hits: Vec<SearchHit> = Vec::new();
    let mut truncated = false;
    let cap = max_hits.max(1).min(5000);

    if matches!(engine, SearchEngine::Rg | SearchEngine::GitGrep) {
        for raw in lines {
            let trimmed = raw.trim_end_matches('\r');
            if trimmed.is_empty() {
                continue;
            }
            if hits.len() >= cap {
                truncated = true;
                break;
            }
            if let Some(hit) = parse_hit(trimmed) {
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
}
