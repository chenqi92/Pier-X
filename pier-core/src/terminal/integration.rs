//! Shell integration snippets shipped with Pier-X.
//!
//! These are tiny rc scripts that pier-ui-gpui uploads to a remote
//! host over SFTP the first time a russh shell channel opens, then
//! launches the remote shell with `bash --rcfile <path>` so the
//! script is sourced for this interactive session.
//!
//! Each script's only responsibility right now is to emit OSC 7
//! (`file://HOST/PATH`) before every prompt so the Pier-X left file
//! panel can follow `cd` on the remote side in real time. Future
//! revisions will add OSC 133 command-boundary markers (M2.8) and
//! nested-`ssh` hijacking (M3).

/// Bash integration. Source this with `bash --rcfile <path> -i`.
/// Chain-sources the user's own `~/.bashrc` first so their prompt,
/// aliases, and PATH are preserved; then appends our OSC 7 hook to
/// `PROMPT_COMMAND`.
pub const BASH_INTEGRATION: &str = include_str!("integration/integration.bash");

/// PowerShell integration. Source with
/// `pwsh -NoLogo -NoExit -Command ". <path>"`. Same OSC 7 / OSC 133
/// semantics as the bash variant, adapted to PowerShell's `prompt`
/// function + `$LASTEXITCODE` conventions.
pub const POWERSHELL_INTEGRATION: &str = include_str!("integration/integration.ps1");

/// cmd.exe fallback — the absolute minimum when a Windows OpenSSH
/// host has neither bash nor pwsh. Only emits OSC 7 via the
/// `PROMPT` env var (cmd has no prompt hook akin to
/// `PROMPT_COMMAND`, so OSC 133 command boundaries aren't
/// reportable here).
pub const CMD_INTEGRATION: &str = include_str!("integration/integration.cmd");

/// Relative path (under `$HOME`) where `BASH_INTEGRATION` should land
/// on the remote side. Keeps the upload + launch sides in lock-step
/// so there's only one string to get right.
pub const REMOTE_INTEGRATION_DIR: &str = ".pier-x";
/// Remote path for the uploaded bash rc.
pub const REMOTE_INTEGRATION_BASH_PATH: &str = ".pier-x/integration.sh";
/// Remote path for the uploaded PowerShell rc.
pub const REMOTE_INTEGRATION_POWERSHELL_PATH: &str = ".pier-x/integration.ps1";

/// Command to pass to `open_exec_channel` after the script has been
/// uploaded. Uses `~` so it resolves against whatever `$HOME` is on
/// the remote — SFTP's relative paths land in the same place.
pub const BASH_LAUNCH_COMMAND: &str = "bash --rcfile ~/.pier-x/integration.sh -i";

/// PowerShell launch command. `-NoLogo` suppresses the startup
/// banner, `-NoExit` keeps the shell interactive after the
/// dot-source, and `$HOME\.pier-x\integration.ps1` resolves against
/// the user profile on Windows OpenSSH (SFTP's `~` lands in the
/// same place).
pub const POWERSHELL_LAUNCH_COMMAND: &str =
    "pwsh -NoLogo -NoExit -Command \". $HOME/.pier-x/integration.ps1\"";

/// Remote path for the uploaded cmd.exe rc.
pub const REMOTE_INTEGRATION_CMD_PATH: &str = ".pier-x/integration.cmd";

/// cmd.exe launch. `/K` keeps the interpreter running after
/// executing the batch. `%USERPROFILE%` resolves to the home dir
/// on Windows, matching where SFTP landed the rc.
pub const CMD_LAUNCH_COMMAND: &str = "cmd.exe /K call %USERPROFILE%\\.pier-x\\integration.cmd";

// ── Local-side install / uninstall (M4 opt-in) ────────────────

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use directories::UserDirs;

fn home_dir() -> Option<PathBuf> {
    UserDirs::new().map(|u| u.home_dir().to_path_buf())
}

/// Marker that brackets the section we add to the user's shell rc
/// file (`~/.bashrc` / `~/.zshrc`). Keeps uninstall surgical — we
/// only strip our own block without touching anything around it.
pub const LOCAL_MARKER_BEGIN: &str = "# Pier-X BEGIN — shell integration (managed)";
pub const LOCAL_MARKER_END: &str = "# Pier-X END";

/// Absolute path where the local-side **bash** integration script
/// should live. Mirrors the remote location so `ssh` hijacking can
/// say `~/.pier-x/integration.sh` on both sides without conditionals.
pub fn local_integration_script_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".pier-x").join("integration.sh"))
}

/// Write **both** the bash and PowerShell rcs to `~/.pier-x/`. We do
/// this unconditionally on any install — platform-independent — so
/// the nested-`ssh` hijacker in either rc can always find the
/// counterpart script to upload when jumping to a target of the
/// opposite shell family. The cost is a few kilobytes on disk; the
/// benefit is cross-shell-family nested tracking.
fn write_both_local_rcs() -> io::Result<()> {
    let Some(home) = home_dir() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "$HOME is unavailable",
        ));
    };
    let base = home.join(".pier-x");
    fs::create_dir_all(&base)?;
    fs::write(base.join("integration.sh"), BASH_INTEGRATION)?;
    fs::write(base.join("integration.ps1"), POWERSHELL_INTEGRATION)?;
    Ok(())
}

/// Delete both local rcs. Used from either uninstall path so
/// opting out really cleans up.
fn remove_both_local_rcs() {
    let Some(home) = home_dir() else { return };
    let base = home.join(".pier-x");
    let _ = fs::remove_file(base.join("integration.sh"));
    let _ = fs::remove_file(base.join("integration.ps1"));
}

/// Rc files we try to augment, in priority order. The first existing
/// file wins — we don't touch multiple rcs to avoid double-sourcing.
pub fn candidate_local_rc_files() -> Vec<PathBuf> {
    let home = match home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };
    vec![
        home.join(".bashrc"),
        home.join(".zshrc"),
        home.join(".profile"),
    ]
}

/// Write the shipped `BASH_INTEGRATION` script to
/// `~/.pier-x/integration.sh` and ensure **one** of the user's shell
/// rc files sources it through our BEGIN/END marker block.
///
/// Idempotent: calling twice leaves the user's rc in the same state.
/// Returns the rc file that was modified (or already contained the
/// block) so callers can surface it to the user.
pub fn install_local_bash_integration() -> io::Result<PathBuf> {
    // Lay down both flavours so the bash rc's nested-`ssh` hijacker
    // can upload `.ps1` when jumping to a Windows target (and vice
    // versa).
    write_both_local_rcs()?;
    let script_path = local_integration_script_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "$HOME is unavailable"))?;

    // Pick the first rc that exists, else fall back to `.bashrc`
    // (creating it if the user has no shell rc at all).
    let candidates = candidate_local_rc_files();
    let rc_path = candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| {
            candidates
                .first()
                .cloned()
                .unwrap_or_else(|| PathBuf::from(".bashrc"))
        });

    let existing = read_or_empty(&rc_path)?;
    if existing.contains(LOCAL_MARKER_BEGIN) {
        // Already installed — refresh the script file above but
        // don't double-append the source-block.
        return Ok(rc_path);
    }

    let script_lit = script_path.to_string_lossy();
    let block = format!(
        "\n{begin}\nif [ -f \"{path}\" ]; then . \"{path}\"; fi\n{end}\n",
        begin = LOCAL_MARKER_BEGIN,
        path = script_lit,
        end = LOCAL_MARKER_END,
    );

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&rc_path)?;
    file.write_all(block.as_bytes())?;
    Ok(rc_path)
}

/// Remove the marker block from every rc we can find and delete
/// both `~/.pier-x/integration.sh` and `~/.pier-x/integration.ps1`.
/// Idempotent: silently no-ops anything that isn't present.
pub fn uninstall_local_bash_integration() -> io::Result<()> {
    for rc_path in candidate_local_rc_files() {
        if !rc_path.exists() {
            continue;
        }
        let existing = read_or_empty(&rc_path)?;
        if !existing.contains(LOCAL_MARKER_BEGIN) {
            continue;
        }
        let cleaned = strip_marker_block(&existing);
        fs::write(&rc_path, cleaned)?;
    }
    remove_both_local_rcs();
    Ok(())
}

/// True if the marker block is present in any of the candidate rc
/// files — used by Settings UI to render the toggle's current state
/// without trusting the persisted flag alone (user might have edited
/// their rc by hand).
pub fn is_local_bash_integration_installed() -> bool {
    candidate_local_rc_files().iter().any(|rc| {
        read_or_empty(rc)
            .map(|c| c.contains(LOCAL_MARKER_BEGIN))
            .unwrap_or(false)
    })
}

// ── Windows-side install / uninstall (PowerShell profiles) ────

/// Absolute path for the local PowerShell rc. Mirrors the bash
/// version — one canonical script file that the profile sources.
pub fn local_powershell_script_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".pier-x").join("integration.ps1"))
}

/// PowerShell profile paths we try to augment. Both are written so
/// the user gets integration regardless of whether they run `pwsh`
/// (PowerShell 7+) or `powershell` (Windows PowerShell 5.x).
pub fn candidate_powershell_profiles() -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let docs = home.join("Documents");
    vec![
        docs.join("PowerShell")
            .join("Microsoft.PowerShell_profile.ps1"),
        docs.join("WindowsPowerShell")
            .join("Microsoft.PowerShell_profile.ps1"),
    ]
}

/// Write the shipped `POWERSHELL_INTEGRATION` script to
/// `~/.pier-x/integration.ps1` and add a BEGIN/END marker block to
/// every candidate PowerShell profile that exists (or create the
/// first one if none exist). Returns the paths that ended up
/// carrying the block.
///
/// Idempotent — re-running silently refreshes the script content
/// without appending the block twice.
pub fn install_local_powershell_integration() -> io::Result<Vec<PathBuf>> {
    // Both flavours land so the .ps1's nested-`ssh` hijacker can
    // upload `.sh` when jumping to a POSIX target.
    write_both_local_rcs()?;
    let script_path = local_powershell_script_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "$HOME is unavailable"))?;

    let profiles = candidate_powershell_profiles();
    if profiles.is_empty() {
        return Ok(Vec::new());
    }
    // If no profile exists yet, create the first candidate so the
    // user still gets the integration the next time they open a
    // PowerShell tab.
    let anything_exists = profiles.iter().any(|p| p.exists());
    let targets: Vec<PathBuf> = if anything_exists {
        profiles.iter().filter(|p| p.exists()).cloned().collect()
    } else {
        vec![profiles[0].clone()]
    };

    let script_lit = script_path.to_string_lossy();
    // PowerShell uses `.` as the dot-source operator and `&` for
    // command invocation. Quoting with single quotes protects
    // against paths containing spaces (`C:\Users\My Name\…`).
    let block = format!(
        "\n{begin}\nif (Test-Path '{path}') {{ . '{path}' }}\n{end}\n",
        begin = LOCAL_MARKER_BEGIN,
        path = script_lit,
        end = LOCAL_MARKER_END,
    );

    let mut touched = Vec::new();
    for profile in &targets {
        if let Some(parent) = profile.parent() {
            fs::create_dir_all(parent)?;
        }
        let existing = read_or_empty(profile)?;
        if existing.contains(LOCAL_MARKER_BEGIN) {
            touched.push(profile.clone());
            continue;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(profile)?;
        file.write_all(block.as_bytes())?;
        touched.push(profile.clone());
    }
    Ok(touched)
}

/// Remove the BEGIN/END marker block from every candidate
/// PowerShell profile and delete both rcs under `~/.pier-x/`.
pub fn uninstall_local_powershell_integration() -> io::Result<()> {
    for profile in candidate_powershell_profiles() {
        if !profile.exists() {
            continue;
        }
        let existing = read_or_empty(&profile)?;
        if !existing.contains(LOCAL_MARKER_BEGIN) {
            continue;
        }
        let cleaned = strip_marker_block(&existing);
        fs::write(&profile, cleaned)?;
    }
    remove_both_local_rcs();
    Ok(())
}

/// True if the marker block is present in any candidate PowerShell
/// profile. Matches `is_local_bash_integration_installed` shape.
pub fn is_local_powershell_integration_installed() -> bool {
    candidate_powershell_profiles().iter().any(|p| {
        read_or_empty(p)
            .map(|c| c.contains(LOCAL_MARKER_BEGIN))
            .unwrap_or(false)
    })
}

// ── Unified local-integration entry points ────────────────────

/// Platform-aware installer. Routes to bash on Unix and PowerShell
/// on Windows so the Settings toggle doesn't need to know which one
/// we're on. `Ok(())` doesn't guarantee *any* rc was modified —
/// `candidate_local_rc_files` / `candidate_powershell_profiles` may
/// both be empty on unsupported platforms; the caller checks
/// `is_local_integration_installed` to confirm.
pub fn install_local_integration() -> io::Result<()> {
    #[cfg(windows)]
    {
        install_local_powershell_integration().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        install_local_bash_integration().map(|_| ())
    }
}

/// Platform-aware uninstaller. Mirror of `install_local_integration`.
pub fn uninstall_local_integration() -> io::Result<()> {
    #[cfg(windows)]
    {
        uninstall_local_powershell_integration()
    }
    #[cfg(not(windows))]
    {
        uninstall_local_bash_integration()
    }
}

/// Platform-aware check. Used by Settings UI to reflect the on-disk
/// truth without relying on the persisted flag alone.
pub fn is_local_integration_installed() -> bool {
    #[cfg(windows)]
    {
        is_local_powershell_integration_installed()
    }
    #[cfg(not(windows))]
    {
        is_local_bash_integration_installed()
    }
}

fn read_or_empty(path: &Path) -> io::Result<String> {
    match fs::File::open(path) {
        Ok(mut f) => {
            let mut s = String::new();
            f.read_to_string(&mut s)?;
            Ok(s)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err),
    }
}

fn strip_marker_block(content: &str) -> String {
    // Strip every BEGIN..END (inclusive) block. Preserves everything
    // else byte-for-byte so the user's other rc edits stay intact.
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(begin) = rest.find(LOCAL_MARKER_BEGIN) {
        out.push_str(&rest[..begin]);
        // Include the preceding newline in the strip if we have one,
        // so removing doesn't leave an orphan blank line.
        if out.ends_with('\n') && out.len() >= 2 && out.as_bytes()[out.len() - 2] == b'\n' {
            out.pop();
        }
        let after_begin = &rest[begin..];
        match after_begin.find(LOCAL_MARKER_END) {
            Some(end_rel) => {
                let end_abs = end_rel + LOCAL_MARKER_END.len();
                // Include one trailing newline if present.
                let mut skip = end_abs;
                if after_begin[skip..].starts_with('\n') {
                    skip += 1;
                }
                rest = &after_begin[skip..];
            }
            None => {
                // Unterminated block — drop everything after BEGIN.
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_removes_our_block() {
        let input = format!(
            "# user line 1\n{begin}\n. /path/to/integration.sh\n{end}\n# user line 2\n",
            begin = LOCAL_MARKER_BEGIN,
            end = LOCAL_MARKER_END,
        );
        let stripped = strip_marker_block(&input);
        assert!(!stripped.contains(LOCAL_MARKER_BEGIN));
        assert!(!stripped.contains(LOCAL_MARKER_END));
        assert!(stripped.contains("user line 1"));
        assert!(stripped.contains("user line 2"));
    }

    #[test]
    fn strip_is_noop_when_absent() {
        let input = "# user file\nexport PATH=/usr/local/bin:$PATH\n";
        assert_eq!(strip_marker_block(input), input);
    }

    #[test]
    fn strip_handles_unterminated_block() {
        let input = format!("keep me\n{}\nunterminated tail\n", LOCAL_MARKER_BEGIN);
        let stripped = strip_marker_block(&input);
        assert!(stripped.contains("keep me"));
        assert!(!stripped.contains(LOCAL_MARKER_BEGIN));
    }
}
