//! Smart-mode shell init injector.
//!
//! When the UI requests a "smart" terminal session (fish-style autosuggest /
//! syntax highlighting / Tab popover / man-page assistant), pier-core needs
//! a way to know where each prompt ends and the user's input begins. We use
//! the OSC 133 prompt-sentinel convention (also used by VS Code's terminal,
//! iTerm2 shell integration, Wave, Warp): the shell wraps its PS1 with
//!
//!   \e]133;A\a    start of prompt
//!   \e]133;B\a    end of prompt — user input starts here
//!   \e]133;C\a    user pressed Enter, command begins
//!   \e]133;D\a    command finished
//!
//! The emulator (`emulator::osc_dispatch`) records the most recent A/B
//! positions so the UI can overlay the smart layer at the right cell.
//!
//! This module's job is to spawn the user's shell in a way that injects
//! those wrappers around the **user's existing PS1**, without replacing
//! it. The user's git/branch/colour prompt stays as configured; we just
//! prepend / append two OSC sequences.
//!
//! Strategy per shell:
//!  * **bash** — `--rcfile <tmp>` whose script first sources the user's
//!    own startup chain (`/etc/profile`, `~/.bash_profile`, `~/.bashrc`),
//!    then re-exports `PS1` wrapped with OSC 133 A/B in `\001..\002`
//!    readline non-printing markers so prompt width math stays correct.
//!  * **zsh** — set `ZDOTDIR=<tmpdir>` containing a `.zshrc` that sources
//!    the user's real startup files (we expose the original ZDOTDIR via
//!    `PIERX_REAL_ZDOTDIR`) and wraps `PROMPT` with `%{...%}` literal
//!    escapes.
//!  * other shells (fish, pwsh, dash, …) — return an unrecognised init;
//!    callers fall back to the plain spawn path and smart mode silently
//!    stays off.
//!
//! Temp files MUST outlive the shell process. `SmartShellInit` owns the
//! tempdir and removes it on Drop. The PTY layer keeps the init alive
//! for the duration of the session.

use std::io::Write;
use std::path::PathBuf;

/// Init directives produced for one smart-mode shell launch.
///
/// The PTY layer takes the resulting `args` and `env` and forwards them
/// to the child process. The struct must be kept alive for the PTY's
/// lifetime — its `Drop` removes the temp directory underneath.
pub struct SmartShellInit {
    /// Extra command-line arguments to use when spawning the shell.
    /// For bash: `["--rcfile", "<tmp>", "-i"]`.
    /// Replaces — does not append to — the caller's default args, since
    /// `-l` is incompatible with `--rcfile`.
    pub args: Vec<String>,
    /// Extra environment overrides to set in the child process before
    /// `execvp`. For zsh: `[("ZDOTDIR", "<tmpdir>"),
    /// ("PIERX_REAL_ZDOTDIR", "<orig>")]`.
    pub env: Vec<(String, String)>,
    /// Whether the shell is recognised as smart-mode-capable. If false,
    /// the caller should fall through to the plain spawn path; the init
    /// has empty `args` / `env` and no temp files.
    pub recognised: bool,
    _tmp_dir: Option<TempDir>,
}

impl SmartShellInit {
    fn empty() -> Self {
        Self {
            args: Vec::new(),
            env: Vec::new(),
            recognised: false,
            _tmp_dir: None,
        }
    }
}

/// Decide what flavour of shell `shell` is and return the init it needs.
///
/// `shell` is the program path that `Pty::spawn_shell` would have used
/// (`/bin/bash`, `/usr/bin/zsh`, `pwsh.exe`, …). Matching is by the
/// final path component, case-insensitive, with any `.exe` suffix
/// stripped.
pub fn inject_init(shell: &str) -> SmartShellInit {
    let leaf_owned = shell
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    let leaf = leaf_owned
        .strip_suffix(".exe")
        .unwrap_or(leaf_owned.as_str());

    match leaf {
        "bash" => bash_init().unwrap_or_else(|_| SmartShellInit::empty()),
        "zsh" => zsh_init().unwrap_or_else(|_| SmartShellInit::empty()),
        // fish has its own line editor — autosuggest/highlight already.
        // pwsh, cmd, dash: no smart support yet.
        _ => SmartShellInit::empty(),
    }
}

/// A self-cleaning temp directory under `$TMPDIR/pier-x-smart-<pid>-<n>`.
///
/// We avoid pulling in the `tempfile` crate — the directory contents are
/// short-lived (one shell session) and a manual best-effort
/// `remove_dir_all` on drop is enough.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> std::io::Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("pier-x-smart-{pid}-{n}-{nanos}"));
        std::fs::create_dir_all(&dir)?;
        Ok(Self { path: dir })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn bash_init() -> std::io::Result<SmartShellInit> {
    let dir = TempDir::new()?;
    let rc_path = dir.path.join("pier-rcfile");
    // `\001` / `\002` are RL_PROMPT_START_IGNORE / END_IGNORE — readline
    // counts the bytes between them as zero-width when computing prompt
    // length. `\[` / `\]` would work too but are translated by PS1
    // expansion, not all configurations expand them inside variable
    // substitution; the raw bytes are unambiguous.
    let script = "\
# pier-x smart-mode bash init — sourced via --rcfile.
case $- in
  *i*) ;;
  *) return ;;
esac

[ -f /etc/profile ] && . /etc/profile
[ -f \"$HOME/.bash_profile\" ] && . \"$HOME/.bash_profile\"
[ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"

if [ -z \"${PIERX_PROMPT_WRAPPED:-}\" ]; then
  PIERX_OSC_A=$'\\001\\e]133;A\\a\\002'
  PIERX_OSC_B=$'\\001\\e]133;B\\a\\002'
  PS1=\"${PIERX_OSC_A}${PS1}${PIERX_OSC_B}\"
  unset PIERX_OSC_A PIERX_OSC_B
  export PIERX_PROMPT_WRAPPED=1
fi

# OSC 7 — report cwd to the host every prompt redraw so the smart
# completer can list files relative to where the user actually `cd`-ed
# (matches what VTE's vte.sh and wezterm's shell-integration.sh do).
__pierx_osc7() {
  printf '\\033]7;file://%s%s\\033\\\\' \"${HOSTNAME:-localhost}\" \"$PWD\"
}
case \"${PROMPT_COMMAND:-}\" in
  *__pierx_osc7*) ;;
  '') PROMPT_COMMAND='__pierx_osc7' ;;
  *) PROMPT_COMMAND='__pierx_osc7;'\"$PROMPT_COMMAND\" ;;
esac
__pierx_osc7
";
    let mut f = std::fs::File::create(&rc_path)?;
    f.write_all(script.as_bytes())?;
    drop(f);

    Ok(SmartShellInit {
        args: vec![
            "--rcfile".to_string(),
            rc_path.to_string_lossy().into_owned(),
            "-i".to_string(),
        ],
        env: Vec::new(),
        recognised: true,
        _tmp_dir: Some(dir),
    })
}

fn zsh_init() -> std::io::Result<SmartShellInit> {
    let dir = TempDir::new()?;
    let zshrc_path = dir.path.join(".zshrc");
    // `%{ ... %}` are zsh prompt-internal "zero-width" markers (analogous
    // to bash's \[ / \]). Inside them, the `$'\e]...\a'` ANSI-C escape
    // emits real ESC and BEL bytes the emulator parses as OSC 133.
    let script = "\
# pier-x smart-mode zsh init — sourced via ZDOTDIR.
[[ -o interactive ]] || return

pierx_real_zdotdir=\"${PIERX_REAL_ZDOTDIR:-$HOME}\"
[[ -f /etc/zshenv ]] && source /etc/zshenv
[[ -f /etc/zshrc ]] && source /etc/zshrc
[[ -f \"$pierx_real_zdotdir/.zshenv\" ]] && source \"$pierx_real_zdotdir/.zshenv\"
[[ -f \"$pierx_real_zdotdir/.zprofile\" ]] && source \"$pierx_real_zdotdir/.zprofile\"
[[ -f \"$pierx_real_zdotdir/.zshrc\" ]] && source \"$pierx_real_zdotdir/.zshrc\"
[[ -f \"$pierx_real_zdotdir/.zlogin\" ]] && source \"$pierx_real_zdotdir/.zlogin\"

if [[ -z \"${PIERX_PROMPT_WRAPPED:-}\" ]]; then
  PROMPT=$'%{\\e]133;A\\a%}'\"$PROMPT\"$'%{\\e]133;B\\a%}'
  export PIERX_PROMPT_WRAPPED=1
fi

# OSC 7 — see bash_init for rationale. precmd_functions is zsh's
# native pre-prompt hook; we keep it idempotent so re-sourcing zshrc
# (oh-my-zsh `omz reload`) doesn't stack duplicates.
__pierx_osc7() {
  printf '\\e]7;file://%s%s\\e\\\\' \"${HOST:-localhost}\" \"$PWD\"
}
typeset -ga precmd_functions
if [[ -z \"${precmd_functions[(r)__pierx_osc7]:-}\" ]]; then
  precmd_functions+=(__pierx_osc7)
fi
__pierx_osc7
unset pierx_real_zdotdir
";
    std::fs::write(&zshrc_path, script)?;

    let real_zdotdir = std::env::var_os("ZDOTDIR")
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| h.to_string_lossy().into_owned())
                .unwrap_or_default()
        });

    Ok(SmartShellInit {
        args: vec!["-i".to_string()],
        env: vec![
            (
                "ZDOTDIR".to_string(),
                dir.path.to_string_lossy().into_owned(),
            ),
            ("PIERX_REAL_ZDOTDIR".to_string(), real_zdotdir),
        ],
        recognised: true,
        _tmp_dir: Some(dir),
    })
}

/// PowerShell prompt-hook script. Returned as raw text so the
/// caller can inject it via `-NoExit -Command "<script>"` when
/// spawning pwsh on Windows.
///
/// Wraps the user's existing `prompt` function and prepends two
/// OSC sequences on every redraw:
///
/// * **OSC 9;9;\<path\>** — Microsoft's Windows-Terminal-native
///   "current working directory" channel. Honoured by Windows
///   Terminal, ConEmu, Tabby; we parse it in `emulator.rs` so
///   Pier-X tabs pick it up cheaply.
/// * **OSC 7;file://host/path** — XTerm-style URI form. Provides
///   compatibility with VS Code's terminal, iTerm2, the wider
///   Linux/macOS ecosystem, and our own existing OSC 7 parser.
///
/// Reference: Microsoft's "Tutorial: New tab, same directory"
/// guide for Windows Terminal walks through exactly this pattern,
/// and pwsh-on-Windows users already follow it in their `$PROFILE`.
/// We just inject it for free so smart-mode tabs work out of the
/// box without modifying the user's profile.
pub fn pwsh_init_script() -> &'static str {
    // Single-quoted PowerShell here-string: `$` is literal so we
    // don't have to escape every variable. The wrapper keeps a
    // reference to the *previous* prompt so a user-customised
    // `$profile` prompt continues to work — this is the same
    // pattern Microsoft documents for Windows Terminal.
    r#"
if (-not (Get-Variable -Name __pierxPromptInstalled -Scope Global -ErrorAction SilentlyContinue)) {
    $global:__pierxPromptInstalled = $true
    $global:__pierxOldPrompt = $function:prompt
    function global:prompt {
        $loc = $executionContext.SessionState.Path.CurrentLocation
        $esc = [char]27
        $bel = [char]7
        $host_ = [System.Net.Dns]::GetHostName()
        # OSC 9;9 — Windows Terminal native cwd channel. Quote the
        # path so paths with spaces survive.
        [Console]::Write("$esc]9;9;`"$loc`"$esc\")
        if ($loc.Provider.Name -eq 'FileSystem') {
            $forward = $loc.ProviderPath -replace '\\','/'
            # OSC 7 — file:// URI form for cross-tool compatibility.
            [Console]::Write("$esc]7;file://$host_$forward$esc\")
        }
        & $global:__pierxOldPrompt
    }
}
"#
}

/// One-shot init line to write into a freshly-spawned remote SSH
/// shell so it starts emitting OSC 7 (cwd) and OSC 133 (prompt
/// sentinels) on every prompt.
///
/// This is the SSH-equivalent of `bash_init` / `zsh_init`. We can't
/// inject `--rcfile` over SSH (the channel runs the user's login
/// shell directly), so we send a single semicolon-joined line via
/// stdin after the channel comes up. The line is shell-detecting:
/// it sets up a hook for whichever of bash/zsh is hosting it, and
/// is a silent no-op under fish/sh/dash.
///
/// Behaviour:
///
/// * **bash** — appends a `__pierx_osc7` function to `PROMPT_COMMAND`
///   (preserving any existing value), and wraps `PS1` with OSC 133.
/// * **zsh** — adds the function to `precmd_functions` and wraps
///   `PROMPT` with `%{...%}`-marked OSC 133.
/// * **fish / sh / dash** — the conditional fails silently; no harm
///   done.
///
/// Reference: this mirrors VTE's `vte.sh` and wezterm's
/// `shell-integration.sh` patterns. We send it as one line so it
/// pollutes history with at most one entry (and a leading space
/// keeps it out of history under any shell with `HISTCONTROL`
/// containing `ignorespace` — Ubuntu/Fedora default).
pub fn remote_init_payload() -> Vec<u8> {
    // Build the line carefully — every newline would split the line
    // and bash would execute halves separately. Use `;` as the only
    // statement separator. Trailing `\n` actually executes the line.
    //
    // We emit a leading SPACE so `HISTCONTROL=ignorespace` (default
    // in most modern distros) drops it from history.
    const SCRIPT: &str = concat!(
        " ",
        // bash branch
        "if [ -n \"${BASH_VERSION:-}\" ]; then ",
            "__pierx_osc7() { printf '\\033]7;file://%s%s\\033\\\\' \"${HOSTNAME:-localhost}\" \"$PWD\"; }; ",
            "case \"${PROMPT_COMMAND:-}\" in *__pierx_osc7*) ;; ",
                "'') PROMPT_COMMAND='__pierx_osc7' ;; ",
                "*) PROMPT_COMMAND='__pierx_osc7;'\"$PROMPT_COMMAND\" ;; ",
            "esac; ",
            "if [ -z \"${PIERX_PROMPT_WRAPPED:-}\" ]; then ",
                "PS1=$'\\001\\033]133;A\\007\\002'\"$PS1\"$'\\001\\033]133;B\\007\\002'; ",
                "export PIERX_PROMPT_WRAPPED=1; ",
            "fi; ",
            "__pierx_osc7; ",
        // zsh branch
        "elif [ -n \"${ZSH_VERSION:-}\" ]; then ",
            "__pierx_osc7() { printf '\\e]7;file://%s%s\\e\\\\' \"${HOST:-localhost}\" \"$PWD\"; }; ",
            "typeset -ga precmd_functions; ",
            "if [[ -z \"${precmd_functions[(r)__pierx_osc7]:-}\" ]]; then precmd_functions+=(__pierx_osc7); fi; ",
            "if [[ -z \"${PIERX_PROMPT_WRAPPED:-}\" ]]; then ",
                "PROMPT=$'%{\\e]133;A\\a%}'\"$PROMPT\"$'%{\\e]133;B\\a%}'; ",
                "export PIERX_PROMPT_WRAPPED=1; ",
            "fi; ",
            "__pierx_osc7; ",
        "fi; ",
        // Wipe the visible echo of this very line. The PTY's terminal
        // driver echoes everything we write to its master back as
        // printable input (the remote shell hasn't reached its
        // readline loop yet, so input is in canonical+echo mode), so
        // without this the user sees ~10 wrapped lines of the init
        // script before bash silently runs it. `printf '\033[H\033[2J'`
        // clears the visible screen after the line executes — the
        // banner / prompt that follows lands on a clean canvas. We
        // intentionally don't `\033[3J` (erase scrollback): the user
        // can still scroll up to see the SSH login banner if needed.
        "printf '\\033[H\\033[2J'; true",
        "\n",
    );
    SCRIPT.as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_shell_yields_unrecognised_init() {
        let init = inject_init("/usr/bin/dash");
        assert!(!init.recognised);
        assert!(init.args.is_empty());
        assert!(init.env.is_empty());
    }

    #[test]
    fn fish_is_unrecognised_so_caller_skips_smart_mode() {
        let init = inject_init("/usr/local/bin/fish");
        assert!(!init.recognised);
    }

    #[test]
    fn powershell_returns_unrecognised_for_now() {
        assert!(!inject_init("pwsh").recognised);
        assert!(!inject_init("powershell.exe").recognised);
        assert!(!inject_init("PowerShell.exe").recognised);
    }

    #[test]
    fn bash_init_writes_rcfile_with_osc133_markers() {
        let init = inject_init("/bin/bash");
        assert!(init.recognised);
        assert_eq!(init.args.first().map(String::as_str), Some("--rcfile"));
        let path = init.args.get(1).expect("rcfile path argument");
        let body = std::fs::read_to_string(path).expect("rcfile readable");
        assert!(body.contains("133;A"), "missing OSC 133;A in:\n{body}");
        assert!(body.contains("133;B"), "missing OSC 133;B in:\n{body}");
        // Readline non-printing markers must be present so prompt width
        // math stays correct.
        assert!(body.contains("\\001"));
        assert!(body.contains("\\002"));
        assert!(init.env.is_empty());
    }

    #[test]
    fn zsh_init_sets_zdotdir_and_real_zdotdir() {
        let init = inject_init("/usr/bin/zsh");
        assert!(init.recognised);
        let env: std::collections::HashMap<_, _> = init.env.iter().cloned().collect();
        let zdotdir = env.get("ZDOTDIR").expect("ZDOTDIR set");
        assert!(env.contains_key("PIERX_REAL_ZDOTDIR"));
        let zshrc = std::path::Path::new(zdotdir).join(".zshrc");
        let body = std::fs::read_to_string(&zshrc).expect("zshrc readable");
        assert!(body.contains("133;A"));
        assert!(body.contains("133;B"));
        assert!(body.contains("PROMPT"));
        assert!(body.contains("PIERX_PROMPT_WRAPPED"));
    }

    #[test]
    fn temp_dir_is_removed_on_drop() {
        let parent: PathBuf = {
            let init = inject_init("/bin/bash");
            assert!(init.recognised);
            let p: PathBuf = init.args[1].clone().into();
            assert!(p.exists());
            p.parent()
                .map(PathBuf::from)
                .expect("rcfile parent must exist")
        };
        // init has dropped here — the temp dir under it should be gone.
        assert!(
            !parent.exists(),
            "temp dir {:?} survived SmartShellInit drop",
            parent
        );
    }

    #[test]
    fn bash_init_emits_osc7_for_cwd_tracking() {
        let init = inject_init("/bin/bash");
        let body = std::fs::read_to_string(&init.args[1]).unwrap();
        assert!(body.contains("__pierx_osc7"), "missing OSC 7 hook");
        // Path is reported as a file:// URI with $HOSTNAME + $PWD.
        assert!(body.contains("]7;file://"));
        // And it must register itself into PROMPT_COMMAND idempotently.
        assert!(body.contains("PROMPT_COMMAND"));
    }

    #[test]
    fn zsh_init_emits_osc7_via_precmd_functions() {
        let init = inject_init("/usr/bin/zsh");
        let zdotdir = init
            .env
            .iter()
            .find(|(k, _)| k == "ZDOTDIR")
            .map(|(_, v)| v.clone())
            .unwrap();
        let body = std::fs::read_to_string(std::path::Path::new(&zdotdir).join(".zshrc")).unwrap();
        assert!(body.contains("__pierx_osc7"));
        assert!(body.contains("precmd_functions"));
        assert!(body.contains("]7;file://"));
    }

    #[test]
    fn pwsh_init_script_carries_osc9_and_osc7() {
        let s = pwsh_init_script();
        assert!(s.contains("]9;9;"), "missing OSC 9;9 (Windows Terminal cwd)");
        assert!(s.contains("]7;file://"), "missing OSC 7 (cross-tool cwd)");
        assert!(s.contains("__pierxOldPrompt"), "must preserve user prompt");
        assert!(
            s.contains("__pierxPromptInstalled"),
            "must guard against double-install on profile reload",
        );
    }

    #[test]
    fn remote_init_payload_has_no_embedded_newlines_until_terminator() {
        let bytes = remote_init_payload();
        let s = std::str::from_utf8(&bytes).expect("payload must be valid UTF-8");
        // A bare newline mid-payload would split the line and bash
        // would execute halves separately — we want one atomic
        // command sent to the remote shell.
        let body = s.trim_end_matches('\n');
        assert!(
            !body.contains('\n'),
            "remote payload contained a mid-stream newline:\n{body}",
        );
        // The line must end with exactly one newline (the Enter that
        // submits it to the remote shell).
        assert!(s.ends_with('\n'));
    }

    #[test]
    fn remote_init_payload_handles_both_bash_and_zsh() {
        let s = String::from_utf8(remote_init_payload()).unwrap();
        assert!(s.contains("BASH_VERSION"), "must check for bash");
        assert!(s.contains("ZSH_VERSION"), "must check for zsh");
        assert!(s.contains("__pierx_osc7"), "must define the hook");
        // Leading space → HISTCONTROL=ignorespace drops it from history.
        assert!(s.starts_with(' '), "leading space for history-skip");
    }

    #[test]
    fn case_insensitive_match_handles_uppercase_paths() {
        // Windows PATH entries can have mixed case; the matcher should
        // still recognise BASH.EXE as bash.
        let init = inject_init("C:\\Program Files\\Git\\bin\\BASH.EXE");
        assert!(init.recognised, "uppercase BASH.EXE should match bash");
    }
}
