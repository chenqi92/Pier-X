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
    fn case_insensitive_match_handles_uppercase_paths() {
        // Windows PATH entries can have mixed case; the matcher should
        // still recognise BASH.EXE as bash.
        let init = inject_init("C:\\Program Files\\Git\\bin\\BASH.EXE");
        assert!(init.recognised, "uppercase BASH.EXE should match bash");
    }
}
