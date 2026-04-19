//! "Where is this terminal sitting right now?" — the single piece of
//! state every right/left panel consumes to decide whether it should
//! show the local filesystem / local git repo, or the remote one at
//! the other end of an SSH session.
//!
//! Populated by `TerminalPanel` off two signals the emulator already
//! produces:
//!
//!  * `take_ssh_detected()` — fires when the emulator sees the user
//!    type (or auto-receive) an `ssh user@host` command in the active
//!    line. The UI layer promotes that one-shot event into the
//!    persistent `Remote { .. }` state below.
//!  * `take_ssh_exit_detected()` — fires on `exit` / `logout` in a
//!    remote shell. Pops back to `Local`.
//!  * `current_dir()` — OSC 7 reports the shell's cwd. In a local
//!    shell it's a local path; after an `ssh`, it's the remote path
//!    (if the remote shell also advertises OSC 7 — we only surface
//!    this when we know we're remote).
//!
//! The type is kept intentionally lightweight — no SSH handle, no
//! SFTP client. The handle lives inside `SshSessionState` and is
//! attached separately to the owning `TerminalPanel`; panels that
//! need filesystem access look it up through `PierApp::active_session`.
//!
//! Depth tracking is shaped for M3 (nested ssh) — the counter is
//! already part of the wire format so M3 doesn't need another
//! migration. For M2 it stays at 0 / 1.

use std::fmt;

/// Remote identity bundle extracted from the emulator's SSH detector.
///
/// Equality is "same host + user + port" so that idempotent re-detection
/// while the user is already inside that session doesn't flap the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTarget {
    pub host: String,
    pub user: String,
    pub port: u16,
}

impl RemoteTarget {
    pub fn new(host: impl Into<String>, user: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            user: user.into(),
            port,
        }
    }

    /// `user@host` with the port suffix only when it isn't 22 — matches
    /// the label style already used in `format_ssh_target`.
    pub fn short_label(&self) -> String {
        if self.port == 22 {
            if self.user.is_empty() {
                self.host.clone()
            } else {
                format!("{}@{}", self.user, self.host)
            }
        } else if self.user.is_empty() {
            format!("{}:{}", self.host, self.port)
        } else {
            format!("{}@{}:{}", self.user, self.host, self.port)
        }
    }
}

impl fmt::Display for RemoteTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.short_label())
    }
}

/// Where a terminal is currently running. Consumed by the left file
/// panel, the right SFTP / Git panels, and the status bar.
///
/// `Local` is the default any time we don't have positive evidence of
/// a remote session. `Remote { cwd: None, .. }` means "we know we're
/// over ssh but the remote shell hasn't told us its cwd yet" — panels
/// should show a connecting / waiting state, not a stale local path.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ShellLocation {
    #[default]
    Local,
    Remote {
        target: RemoteTarget,
        /// OSC 7 cwd from the remote shell, if any.
        cwd: Option<String>,
        /// Nested-ssh depth. `1` for a single `ssh`, `2` for ssh-in-ssh.
        /// Reserved for M3; M2 always produces `1`.
        depth: u8,
    },
}

impl ShellLocation {
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote { .. })
    }

    pub fn remote_target(&self) -> Option<&RemoteTarget> {
        match self {
            Self::Remote { target, .. } => Some(target),
            Self::Local => None,
        }
    }

    pub fn remote_cwd(&self) -> Option<&str> {
        match self {
            Self::Remote { cwd, .. } => cwd.as_deref(),
            Self::Local => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_label_drops_default_port() {
        let t = RemoteTarget::new("box.local", "root", 22);
        assert_eq!(t.short_label(), "root@box.local");
    }

    #[test]
    fn short_label_includes_non_default_port() {
        let t = RemoteTarget::new("box.local", "root", 2222);
        assert_eq!(t.short_label(), "root@box.local:2222");
    }

    #[test]
    fn short_label_without_user() {
        let t = RemoteTarget::new("box.local", "", 22);
        assert_eq!(t.short_label(), "box.local");
    }

    #[test]
    fn location_default_is_local() {
        assert_eq!(ShellLocation::default(), ShellLocation::Local);
        assert!(!ShellLocation::Local.is_remote());
    }

    #[test]
    fn remote_accessors() {
        let loc = ShellLocation::Remote {
            target: RemoteTarget::new("h", "u", 22),
            cwd: Some("/root".into()),
            depth: 1,
        };
        assert!(loc.is_remote());
        assert_eq!(loc.remote_target().unwrap().host, "h");
        assert_eq!(loc.remote_cwd(), Some("/root"));
    }
}
