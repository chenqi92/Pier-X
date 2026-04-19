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

/// Relative path (under `$HOME`) where `BASH_INTEGRATION` should land
/// on the remote side. Keeps the upload + launch sides in lock-step
/// so there's only one string to get right.
pub const REMOTE_INTEGRATION_DIR: &str = ".pier-x";
pub const REMOTE_INTEGRATION_BASH_PATH: &str = ".pier-x/integration.sh";

/// Command to pass to `open_exec_channel` after the script has been
/// uploaded. Uses `~` so it resolves against whatever `$HOME` is on
/// the remote — SFTP's relative paths land in the same place.
pub const BASH_LAUNCH_COMMAND: &str = "bash --rcfile ~/.pier-x/integration.sh -i";

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

/// Absolute path where the local-side integration script should live.
/// Mirrors the remote location so `ssh` hijacking can say `~/.pier-x/…`
/// on both sides without conditionals.
pub fn local_integration_script_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".pier-x").join("integration.sh"))
}

/// Rc files we try to augment, in priority order. The first existing
/// file wins — we don't touch multiple rcs to avoid double-sourcing.
pub fn candidate_local_rc_files() -> Vec<PathBuf> {
    let home = match home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };
    vec![home.join(".bashrc"), home.join(".zshrc"), home.join(".profile")]
}

/// Write the shipped `BASH_INTEGRATION` script to
/// `~/.pier-x/integration.sh` and ensure **one** of the user's shell
/// rc files sources it through our BEGIN/END marker block.
///
/// Idempotent: calling twice leaves the user's rc in the same state.
/// Returns the rc file that was modified (or already contained the
/// block) so callers can surface it to the user.
pub fn install_local_bash_integration() -> io::Result<PathBuf> {
    let script_path = local_integration_script_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "$HOME is unavailable"))?;
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&script_path, BASH_INTEGRATION)?;

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
/// `~/.pier-x/integration.sh`. Idempotent: silently no-ops anything
/// that isn't present.
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
    if let Some(script_path) = local_integration_script_path() {
        let _ = fs::remove_file(&script_path);
    }
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
